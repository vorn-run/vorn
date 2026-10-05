//! One session's grid clients (Terminal State Protocol §8 and §11): the
//! render clock, the per-attachment credit window, synchronized output, and
//! the requests a client makes of the terminal.
//!
//! - **Render clock.** After a batch of records the hub runs one render
//!   update if a frame interval (8 ms) has passed since the last; otherwise
//!   it says when one is [`Hub::due`], and its owner calls [`Hub::tick`]
//!   then. Nothing is rendered while no attachment is visible.
//! - **Credits.** Each attachment has two frames in flight at most. A full
//!   window leaves it behind; its next ack sends one delta covering
//!   everything since. Per attachment the hub keeps what the client holds
//!   and a count, never a queue, so a slow client costs constant memory and
//!   never slows the session or another client.
//! - **Synchronized output.** While DEC mode 2026 is set, frames are held for
//!   up to 150 ms so a redraw is never shown half done.
//! - **Hidden attachments** get events and no frames; showing one sends one
//!   delta or snapshot.
//!
//! The hub does no I/O: it is handed the terminal and the time, and answers
//! with messages for connections and bytes for the program ([`HubOut`]).

use std::time::{Duration, Instant};

use libghostty_vt::terminal::Mode;
use vorn_screen::Emulator;
use vorn_term_proto::msg::{
    Attach, Attached, CopyFormat, EventId, EventKind, Fidelity, GridPoint, GridResume, InputEvent,
    Resume, ResyncReason, SelectKind, ServerMsg,
};
use vorn_term_proto::screen::Row;
use vorn_term_proto::Cursor;

use crate::grid::{Grid, Held};
use crate::input::{Encoded, InputEncoder};
use crate::query::{self, Frame, Needle};

/// One attachment: a connection and the id vornd gave the attachment on it.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct Peer {
    pub conn: u64,
    pub sid: u32,
}

/// A grid client's request for one session, decoded and routed by vornd.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum GridIn {
    Attach {
        peer: Peer,
        attach: Attach,
    },
    Detach {
        peer: Peer,
    },
    /// Every attachment on a connection that closed.
    Gone {
        conn: u64,
    },
    SetVisible {
        peer: Peer,
        visible: bool,
    },
    Ack {
        peer: Peer,
        rev: u64,
    },
    Input {
        peer: Peer,
        input_seq: u64,
        event: InputEvent,
    },
    FetchHistory {
        peer: Peer,
        req: u32,
        sb_epoch: u32,
        from_line: u64,
        count: u16,
    },
    SelectAt {
        peer: Peer,
        req: u32,
        at: GridPoint,
        kind: SelectKind,
    },
    Copy {
        peer: Peer,
        req: u32,
        from: GridPoint,
        to: GridPoint,
        rect: bool,
        format: CopyFormat,
    },
    Search {
        peer: Peer,
        req: u32,
        query: String,
        regex: bool,
        case: bool,
        from_line: Option<u64>,
    },
}

/// What the hub wants done.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum HubOut {
    /// A message for one connection.
    Send { conn: u64, msg: ServerMsg },
    /// Bytes for the program, in order with everything else written to it.
    /// `ack` names the input event, for its `InputAck` once written.
    Write {
        bytes: Vec<u8>,
        ack: Option<(Peer, u64)>,
    },
}

/// What the hub reads of its session for each call.
#[derive(Clone, Copy)]
pub struct Ctx<'a> {
    pub em: &'a Emulator,
    pub session: &'a str,
    /// After the last record applied: what a frame cut now is stamped with.
    pub resume: Cursor,
    /// Replay has reached the head. Until then nothing is rendered.
    pub live: bool,
    pub fidelity: Fidelity,
}

impl std::fmt::Debug for Ctx<'_> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Ctx")
            .field("session", &self.session)
            .field("resume", &self.resume)
            .field("live", &self.live)
            .finish()
    }
}

/// The timings and window a hub runs with (TP §8, §11).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct HubConfig {
    /// The render clock's interval.
    pub frame: Duration,
    /// How long frames wait for synchronized output to end.
    pub sync_hold: Duration,
    /// Frames in flight per attachment.
    pub credits: u8,
}

impl Default for HubConfig {
    fn default() -> Self {
        HubConfig {
            frame: Duration::from_millis(8),
            sync_hold: Duration::from_millis(150),
            credits: 2,
        }
    }
}

/// What precedes the next snapshot to an attachment.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Note {
    Resync(ResyncReason),
}

#[derive(Debug)]
struct Attachment {
    peer: Peer,
    visible: bool,
    /// What the client holds, once it was sent a frame (or resumed one).
    held: Option<Held>,
    in_flight: u8,
    history_tail: u16,
    note: Option<Note>,
}

/// One session's grid clients and its grid, built on the first attach.
#[derive(Debug, Default)]
pub struct Hub {
    cfg: HubConfig,
    grid: Option<Grid>,
    atts: Vec<Attachment>,
    input: Option<InputEncoder>,
    last_render: Option<Instant>,
    /// Records applied since the last render update.
    pending: bool,
    /// When synchronized output began, while it lasts.
    sync_since: Option<Instant>,
    /// Whether the session was live at the last call: nothing renders
    /// before, so no frame is due either.
    live: bool,
    /// When the records waiting began to wait.
    pending_at: Option<Instant>,
}

impl Hub {
    pub fn new(cfg: HubConfig) -> Hub {
        Hub {
            cfg,
            ..Hub::default()
        }
    }

    /// Whether any client is attached.
    pub fn attached(&self) -> bool {
        !self.atts.is_empty()
    }

    /// The grid, once a client attached.
    pub fn grid(&self) -> Option<&Grid> {
        self.grid.as_ref()
    }

    /// Records were applied: render if a frame is due.
    pub fn records(&mut self, ctx: &Ctx<'_>, now: Instant, out: &mut Vec<HubOut>) {
        self.live = ctx.live;
        if !self.pending {
            self.pending = true;
            self.pending_at = Some(now);
        }
        self.tick(ctx, now, out);
    }

    /// When the next render update is due, if one is waiting.
    pub fn due(&self) -> Option<Instant> {
        // While replaying nothing renders: a frame due now would have the
        // owner wake for it at once, again and again, until replay ends.
        if !self.live || !self.pending || !self.atts.iter().any(|a| a.visible) {
            return None;
        }
        let mut at = self.last_render.map(|t| t + self.cfg.frame);
        if let Some(since) = self.sync_since {
            let end = since + self.cfg.sync_hold;
            at = Some(at.map_or(end, |a| a.max(end)));
        }
        // Never rendered: due since the records came.
        at.or(self.pending_at)
    }

    /// Renders if a frame is due by `now`.
    pub fn tick(&mut self, ctx: &Ctx<'_>, now: Instant, out: &mut Vec<HubOut>) {
        self.live = ctx.live;
        if self.due().is_some_and(|d| d <= now) {
            self.render(ctx, now, out);
        }
    }

    /// A side effect, for every attachment, visible or not.
    pub fn event(&mut self, id: EventId, kind: EventKind, out: &mut Vec<HubOut>) {
        for a in &self.atts {
            out.push(HubOut::Send {
                conn: a.peer.conn,
                msg: ServerMsg::Event {
                    sid: a.peer.sid,
                    id,
                    kind: kind.clone(),
                },
            });
        }
    }

    /// The terminal is about to be replaced by an exact rebuild: settle
    /// line numbers against it while they can still be counted.
    pub fn before_swap(&mut self, em: &Emulator) {
        if let Some(g) = &mut self.grid {
            g.lines.before_swap(em.terminal());
        }
    }

    /// The terminal was read by someone else (a checkpoint cut reads the
    /// render state) or replaced by an exact rebuild: carry on numbering on
    /// `em` and re-encode every row at the next update.
    pub fn after_swap(&mut self, em: &Emulator) {
        if let Some(g) = &mut self.grid {
            g.lines.after_swap(em.terminal());
            g.distrust_dirty();
        }
    }

    /// The terminal was built again from scratch (output was lost): a new
    /// `state_gen`, and every attachment resyncs.
    pub fn rebuilt(&mut self) {
        if self.grid.take().is_some() {
            for a in &mut self.atts {
                if a.held.is_some() {
                    a.note = Some(Note::Resync(ResyncReason::Gap));
                }
            }
        }
        self.pending = true;
    }

    /// One client request, answered into `out`.
    pub fn handle(&mut self, msg: GridIn, ctx: &Ctx<'_>, now: Instant, out: &mut Vec<HubOut>) {
        self.live = ctx.live;
        match msg {
            GridIn::Attach { peer, attach } => self.attach(peer, &attach, ctx, now, out),
            GridIn::Detach { peer } => self.atts.retain(|a| a.peer != peer),
            GridIn::Gone { conn } => self.atts.retain(|a| a.peer.conn != conn),
            GridIn::SetVisible { peer, visible } => {
                if let Some(a) = self.att(peer) {
                    a.visible = visible;
                }
                if visible {
                    self.catch_up(ctx, now, out);
                }
            }
            GridIn::Ack { peer, .. } => {
                if let Some(a) = self.att(peer) {
                    a.in_flight = a.in_flight.saturating_sub(1);
                }
                self.catch_up(ctx, now, out);
            }
            GridIn::Input {
                peer,
                input_seq,
                event,
            } => self.input(peer, input_seq, &event, ctx, out),
            GridIn::FetchHistory {
                peer,
                req,
                sb_epoch,
                from_line,
                count,
            } => self.fetch(peer, req, sb_epoch, from_line, count, ctx, out),
            GridIn::SelectAt {
                peer,
                req,
                at,
                kind,
            } => {
                let range = self
                    .frame(ctx)
                    .and_then(|f| query::select_at(ctx.em.terminal(), &f, &at, kind));
                out.push(send(
                    peer,
                    ServerMsg::Selection {
                        sid: peer.sid,
                        req,
                        range,
                    },
                ));
            }
            GridIn::Copy {
                peer,
                req,
                from,
                to,
                rect,
                format,
            } => {
                let text = self
                    .frame(ctx)
                    .and_then(|f| query::copy(ctx.em.terminal(), &f, &from, &to, rect, format))
                    .unwrap_or_default();
                out.push(send(
                    peer,
                    ServerMsg::Copied {
                        sid: peer.sid,
                        req,
                        text,
                    },
                ));
            }
            GridIn::Search {
                peer,
                req,
                query: q,
                regex,
                case,
                from_line,
            } => {
                let hits = match (self.frame(ctx), Needle::new(&q, regex, case)) {
                    (Some(f), Some(n)) => query::search(ctx.em.terminal(), &f, &n, from_line),
                    _ => Vec::new(),
                };
                // Streamed in pieces a client can draw as they come.
                let mut chunks = hits.chunks(256).peekable();
                if chunks.peek().is_none() {
                    out.push(send(
                        peer,
                        ServerMsg::SearchHits {
                            sid: peer.sid,
                            req,
                            hits: Vec::new(),
                            done: true,
                        },
                    ));
                }
                while let Some(c) = chunks.next() {
                    out.push(send(
                        peer,
                        ServerMsg::SearchHits {
                            sid: peer.sid,
                            req,
                            hits: c.to_vec(),
                            done: chunks.peek().is_none(),
                        },
                    ));
                }
            }
        }
    }

    fn att(&mut self, peer: Peer) -> Option<&mut Attachment> {
        self.atts.iter_mut().find(|a| a.peer == peer)
    }

    fn attach(
        &mut self,
        peer: Peer,
        req: &Attach,
        ctx: &Ctx<'_>,
        now: Instant,
        out: &mut Vec<HubOut>,
    ) {
        self.atts.retain(|a| a.peer != peer);
        out.push(send(
            peer,
            ServerMsg::Attached(Attached {
                sid: peer.sid,
                session: ctx.session.to_owned(),
                epoch: ctx.resume.epoch,
                owner: false,
                fidelity: ctx.fidelity,
            }),
        ));
        let mut att = Attachment {
            peer,
            visible: req.visible,
            held: None,
            in_flight: 0,
            history_tail: req.history_tail.min(query::MAX_FETCH),
            note: None,
        };
        if let Some(Resume::Grid(r)) = req.resume {
            let (held, note) = self.resume(&r);
            att.held = held;
            att.note = note.map(Note::Resync);
        }
        self.atts.push(att);
        self.catch_up(ctx, now, out);
    }

    /// What a resuming client holds as far as this hub can use it, and why
    /// it gets a snapshot when it cannot (TP §12).
    fn resume(&self, r: &GridResume) -> (Option<Held>, Option<ResyncReason>) {
        let held = Held {
            state_gen: r.state_gen,
            table_gen: r.table_gen,
            rev: r.rev,
            style_mark: r.style_mark,
            link_mark: r.link_mark,
        };
        match &self.grid {
            Some(g) if g.rendered() && r.state_gen == g.state_gen() => {
                if r.table_gen != g.table_gen() {
                    // ResetTables, then a snapshot.
                    (Some(held), None)
                } else if g.delta(&held).is_some() {
                    (Some(held), None)
                } else {
                    (None, Some(ResyncReason::NotRetained))
                }
            }
            _ => (None, Some(ResyncReason::Restarted)),
        }
    }

    /// Renders now if records are waiting and nothing holds frames, then
    /// sends what attachments with credit lack: an attach, an ack and a
    /// pane shown all want the newest state without waiting for the clock.
    fn catch_up(&mut self, ctx: &Ctx<'_>, now: Instant, out: &mut Vec<HubOut>) {
        let never = self.grid.as_ref().is_none_or(|g| !g.rendered());
        let clock = self.last_render.is_none_or(|t| now >= t + self.cfg.frame);
        if self.pending && (never || clock) {
            self.render(ctx, now, out);
        } else {
            self.pump(ctx, out);
        }
    }

    fn render(&mut self, ctx: &Ctx<'_>, now: Instant, out: &mut Vec<HubOut>) {
        if !ctx.live {
            return;
        }
        let t = ctx.em.terminal();
        if t.mode(Mode::SYNC_OUTPUT).unwrap_or(false) {
            let since = *self.sync_since.get_or_insert(now);
            if now.duration_since(since) < self.cfg.sync_hold {
                // What clients hold is still a whole frame; new attachments
                // get it too.
                self.pump(ctx, out);
                return;
            }
        } else {
            self.sync_since = None;
        }
        if self.update(ctx, now) {
            self.pump(ctx, out);
        }
    }

    /// The program ended and the session is about to leave: every visible
    /// attachment gets the last frame now, whatever the clock, a held
    /// synchronized update or its credits say, since no later one comes.
    pub fn finish(&mut self, ctx: &Ctx<'_>, now: Instant, out: &mut Vec<HubOut>) {
        if !ctx.live || !self.atts.iter().any(|a| a.visible) {
            return;
        }
        if self.pending && !self.update(ctx, now) {
            return;
        }
        self.pump_with(ctx, out, u8::MAX);
    }

    /// One render update; false when none could be made.
    fn update(&mut self, ctx: &Ctx<'_>, now: Instant) -> bool {
        if self.grid.is_none() {
            match Grid::new() {
                Ok(g) => self.grid = Some(g),
                Err(_) => return false,
            }
        }
        let Some(g) = &mut self.grid else {
            return false;
        };
        self.last_render = Some(now);
        if g.update(ctx.em, ctx.resume).is_err() {
            // A read from Ghostty failed part way: trust nothing, and try
            // again at the next frame.
            g.distrust_dirty();
            return false;
        }
        self.pending = false;
        true
    }

    /// Sends each visible attachment with credit what it lacks.
    fn pump(&mut self, ctx: &Ctx<'_>, out: &mut Vec<HubOut>) {
        self.pump_with(ctx, out, self.cfg.credits);
    }

    fn pump_with(&mut self, ctx: &Ctx<'_>, out: &mut Vec<HubOut>, credits: u8) {
        let Some(g) = &mut self.grid else { return };
        if !g.rendered() {
            return;
        }
        for a in &mut self.atts {
            if !a.visible || a.in_flight >= credits {
                continue;
            }
            let sid = a.peer.sid;
            let msg = match a.held {
                Some(h)
                    if h.state_gen == g.state_gen()
                        && h.table_gen == g.table_gen()
                        && h.rev == g.rev()
                        && a.note.is_none() =>
                {
                    continue
                }
                Some(h) if a.note.is_none() => g.delta(&h).map(|d| ServerMsg::Delta {
                    sid,
                    delta: Box::new(d),
                }),
                _ => None,
            };
            let msg = match msg {
                Some(m) => m,
                None => {
                    if let Some(Note::Resync(reason)) = a.note.take() {
                        out.push(send(a.peer, ServerMsg::Resync { sid, reason }));
                    }
                    if let Some(h) = a.held {
                        if h.state_gen == g.state_gen() && h.table_gen != g.table_gen() {
                            out.push(send(
                                a.peer,
                                ServerMsg::ResetTables {
                                    sid,
                                    table_gen: g.table_gen(),
                                },
                            ));
                        }
                    }
                    let history = tail(g, ctx, a.history_tail);
                    ServerMsg::Snapshot {
                        sid,
                        snap: Box::new(g.snapshot(history)),
                    }
                }
            };
            a.held = Some(g.held());
            a.in_flight += 1;
            out.push(send(a.peer, msg));
        }
    }

    fn input(
        &mut self,
        peer: Peer,
        input_seq: u64,
        event: &InputEvent,
        ctx: &Ctx<'_>,
        out: &mut Vec<HubOut>,
    ) {
        if self.input.is_none() {
            self.input = InputEncoder::new().ok();
        }
        let Some(enc) = &mut self.input else { return };
        match enc.encode(ctx.em.terminal(), event) {
            Ok(Encoded::Bytes(bytes)) => out.push(HubOut::Write {
                bytes,
                ack: Some((peer, input_seq)),
            }),
            Ok(Encoded::Confirm) => out.push(send(
                peer,
                ServerMsg::PasteConfirm {
                    sid: peer.sid,
                    input_seq,
                },
            )),
            // Nothing to write is acknowledged at once: it is done.
            Ok(Encoded::Nothing) | Err(_) => out.push(send(
                peer,
                ServerMsg::InputAck {
                    sid: peer.sid,
                    input_seq,
                },
            )),
        }
    }

    #[allow(clippy::too_many_arguments)]
    fn fetch(
        &mut self,
        peer: Peer,
        req: u32,
        sb_epoch: u32,
        from_line: u64,
        count: u16,
        ctx: &Ctx<'_>,
        out: &mut Vec<HubOut>,
    ) {
        let frame = self.frame(ctx);
        let Some(g) = &mut self.grid else {
            out.push(send(peer, empty_history(peer, req, sb_epoch, from_line, 0)));
            return;
        };
        let Some(a) = self.atts.iter_mut().find(|a| a.peer == peer) else {
            return;
        };
        // Rows are encoded against the tables the client holds a prefix of;
        // one holding another generation is about to get a snapshot.
        let usable = a
            .held
            .is_some_and(|h| h.state_gen == g.state_gen() && h.table_gen == g.table_gen());
        let reply = match frame {
            Some(f) if usable && f.sb_epoch == sb_epoch => {
                let rows = query::history(ctx.em.terminal(), &f, &mut g.tables, from_line, count);
                let mut held = a.held.unwrap_or_else(|| g.held());
                let styles = g.tables.styles()[held.style_mark as usize..].to_vec();
                let links = g.tables.links()[held.link_mark as usize..].to_vec();
                held.style_mark = g.tables.styles().len() as u32;
                held.link_mark = g.tables.links().len() as u32;
                a.held = Some(held);
                ServerMsg::History {
                    sid: peer.sid,
                    req,
                    sb_epoch,
                    from_line,
                    rows,
                    oldest_line: g.lines().oldest_line(),
                    styles,
                    links,
                }
            }
            _ => empty_history(
                peer,
                req,
                g.lines().sb_epoch(),
                from_line,
                g.lines().oldest_line(),
            ),
        };
        out.push(send(peer, reply));
    }

    /// Where lines are on the terminal now, with what scrolled since the
    /// last frame counted (and reported by the next one).
    fn frame(&mut self, ctx: &Ctx<'_>) -> Option<Frame> {
        let g = self.grid.as_mut()?;
        g.settle(ctx.em);
        Some(Frame::of(ctx.em.terminal(), g.lines()))
    }
}

fn send(peer: Peer, msg: ServerMsg) -> HubOut {
    HubOut::Send {
        conn: peer.conn,
        msg,
    }
}

fn empty_history(
    peer: Peer,
    req: u32,
    sb_epoch: u32,
    from_line: u64,
    oldest_line: u64,
) -> ServerMsg {
    ServerMsg::History {
        sid: peer.sid,
        req,
        sb_epoch,
        from_line,
        rows: Vec::new(),
        oldest_line,
        styles: Vec::new(),
        links: Vec::new(),
    }
}

/// The `n` history lines above the grid's last frame, for a snapshot. Lines
/// are counted on the terminal as it is now, which may have moved on since
/// that frame; the lines are absolute, so the rows are the same ones. None
/// when the epoch changed since, which the next frame reports.
fn tail(g: &mut Grid, ctx: &Ctx<'_>, n: u16) -> Vec<Row> {
    if n == 0 {
        return Vec::new();
    }
    let t = ctx.em.terminal();
    g.settle(ctx.em);
    let f = Frame::of(t, g.lines());
    if f.sb_epoch != g.term().sb_epoch {
        return Vec::new();
    }
    let top = g.term().top_line;
    let from = top.saturating_sub(u64::from(n));
    query::history(t, &f, &mut g.tables, from, n.min((top - from) as u16))
}

#[cfg(test)]
mod tests {
    use super::*;
    use vorn_term_proto::msg::{AttachMode, Size};

    fn ctx(em: &Emulator, live: bool) -> Ctx<'_> {
        Ctx {
            em,
            session: "s",
            resume: Cursor::default(),
            live,
            fidelity: Fidelity::Exact,
        }
    }

    /// While a session replays, a visible attachment with records waiting
    /// has no frame due: one due at once would spin its worker until
    /// replay ends. Once live, the frame is due.
    #[test]
    fn nothing_is_due_while_replaying() {
        let mut em = Emulator::new(20, 4).unwrap();
        let mut hub = Hub::new(HubConfig::default());
        let now = Instant::now();
        let mut out = Vec::new();
        let attach = Attach {
            session: "s".into(),
            mode: AttachMode::Grid,
            view: Size::default(),
            visible: true,
            resume: None,
            history_tail: 0,
        };
        let peer = Peer { conn: 1, sid: 1 };
        hub.handle(
            GridIn::Attach { peer, attach },
            &ctx(&em, false),
            now,
            &mut out,
        );
        em.feed(b"replayed", &mut Vec::new());
        hub.records(&ctx(&em, false), now, &mut out);
        assert_eq!(hub.due(), None);
        assert!(!out.iter().any(|o| matches!(
            o,
            HubOut::Send {
                msg: ServerMsg::Snapshot { .. },
                ..
            }
        )));
        hub.records(&ctx(&em, true), now, &mut out);
        assert!(out.iter().any(|o| matches!(
            o,
            HubOut::Send {
                msg: ServerMsg::Snapshot { .. },
                ..
            }
        )));
    }
}

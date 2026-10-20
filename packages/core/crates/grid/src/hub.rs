//! One session's grid clients (Terminal State Protocol §8 and §11): the
//! render clock, the per-attachment credit window, synchronized output, and
//! the requests a client makes of the terminal.
//!
//! - **Render clock.** After a batch of records the hub runs one render
//!   update if a frame interval (8 ms) has passed since the last; otherwise
//!   it says when one is [`Hub::due`], and its owner calls [`Hub::tick`]
//!   then. Nothing is rendered while no attachment is visible.
//! - **Input-driven frames.** Output that follows input written to the
//!   program within [`HubConfig::typed`] is the program answering a person:
//!   up to [`HubConfig::typed_frames`] frames are cut as soon as it comes,
//!   without waiting for the clock. Output nobody typed for, and whatever
//!   follows past those frames, keeps the clock's pace.
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

use vorn_screen::Emulator;
use vorn_term_proto::msg::{
    Attach, Attached, CopyFormat, EventId, EventKind, Fidelity, GridPoint, GridResume, InputEvent,
    Resume, ResyncReason, SelectKind, ServerMsg,
};
use vorn_term_proto::screen::{Image, Row};
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
        /// The connection agreed on images: its frames carry placements, and it gets the
        /// pixels they show.
        images: bool,
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
    /// How long after input is written its program's output skips the clock.
    pub typed: Duration,
    /// Frames each input may cut off the clock.
    pub typed_frames: u8,
}

impl Default for HubConfig {
    fn default() -> Self {
        HubConfig {
            frame: Duration::from_millis(8),
            sync_hold: Duration::from_millis(150),
            credits: 2,
            typed: Duration::from_millis(100),
            typed_frames: 4,
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
    /// Whether frames carry placements (see [`GridIn::Attach`]).
    images: bool,
    /// The images, by id and generation, the client was sent and still shows, sorted.
    sent: Vec<(u32, u64)>,
}

/// Before a frame that places images, sends `a` the pixels it lacks; a client that did not
/// agree on images gets the frame without placements.
fn images_for(a: &mut Attachment, em: &Emulator, msg: &mut ServerMsg, out: &mut Vec<HubOut>) {
    let (placements, snapshot) = match msg {
        ServerMsg::Snapshot { snap, .. } => (&mut snap.placements, true),
        ServerMsg::Delta { delta, .. } => (&mut delta.placements, false),
        _ => return,
    };
    if !a.images {
        *placements = None;
        return;
    }
    let Some(p) = placements else {
        if snapshot {
            a.sent.clear();
        }
        return;
    };
    let mut shown: Vec<(u32, u64)> = p.images().collect();
    shown.sort_unstable();
    shown.dedup();
    // An image whose pixels changed since the update is left out; the next frame places it.
    shown.retain(|&(id, gen)| {
        if a.sent.binary_search(&(id, gen)).is_ok() {
            return true;
        }
        let Some(pic) = em.picture(id, gen) else {
            return false;
        };
        let image = Image {
            id,
            gen,
            width: pic.width,
            height: pic.height,
            rgba: pic.rgba,
        };
        out.push(send(
            a.peer,
            ServerMsg::Image {
                sid: a.peer.sid,
                image: Box::new(image),
            },
        ));
        true
    });
    a.sent = shown;
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
    /// The synchronized output hold under way and when it began, while it lasts.
    sync: Option<(u64, Instant)>,
    /// Whether the session was live at the last call: nothing renders
    /// before, so no frame is due either.
    live: bool,
    /// When the records waiting began to wait.
    pending_at: Option<Instant>,
    /// When input was last written to the program.
    typed_at: Option<Instant>,
    /// Frames the last input may still cut off the clock.
    typed_left: u8,
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
        self.note_hold(ctx.em, now);
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
        let mut at = if self.answering() {
            None
        } else {
            self.last_render.map(|t| t + self.cfg.frame)
        };
        if let Some((_, since)) = self.sync {
            let end = since + self.cfg.sync_hold;
            at = Some(at.map_or(end, |a| a.max(end)));
        }
        // Never rendered, or answering input: due since the records came.
        at.or(self.pending_at)
    }

    /// Whether the records waiting answer recent input and that input may
    /// still cut a frame off the clock.
    fn answering(&self) -> bool {
        self.typed_left > 0
            && self
                .typed_at
                .zip(self.pending_at)
                .is_some_and(|(typed, came)| came.saturating_duration_since(typed) < self.cfg.typed)
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
            GridIn::Attach {
                peer,
                attach,
                images,
            } => self.attach(peer, &attach, images, ctx, now, out),
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
            } => self.input(peer, input_seq, &event, ctx, now, out),
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
        images: bool,
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
            images,
            sent: Vec::new(),
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
        let clock = self.answering() || self.last_render.is_none_or(|t| now >= t + self.cfg.frame);
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
        self.note_hold(ctx.em, now);
        if let Some((_, since)) = self.sync {
            if now.duration_since(since) < self.cfg.sync_hold {
                // What clients hold is still a whole frame; new attachments
                // get it too.
                self.pump(ctx, out);
                return;
            }
        }
        if self.update(ctx, now) {
            self.pump(ctx, out);
        }
    }

    /// Follows synchronized output: a hold that ended frees the next frame at once, and one
    /// begun since the last look waits its own `sync_hold`, even when the previous ended and
    /// this one started between two looks.
    fn note_hold(&mut self, em: &Emulator, now: Instant) {
        self.sync = em.render_hold().map(|n| match self.sync {
            Some((m, since)) if m == n => (n, since),
            _ => (n, now),
        });
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
        // Spent by every attempt, so failing ones cannot keep the frame due at once.
        if self.answering() {
            self.typed_left -= 1;
        }
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
            self.typed_left = 0;
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
            let mut msg = msg;
            images_for(a, ctx.em, &mut msg, out);
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
        now: Instant,
        out: &mut Vec<HubOut>,
    ) {
        if self.input.is_none() {
            self.input = InputEncoder::new().ok();
        }
        let Some(enc) = &mut self.input else { return };
        match enc.encode(ctx.em.terminal(), event) {
            Ok(Encoded::Bytes(bytes)) => {
                self.typed_at = Some(now);
                self.typed_left = self.cfg.typed_frames;
                out.push(HubOut::Write {
                    bytes,
                    ack: Some((peer, input_seq)),
                });
            }
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
            GridIn::Attach {
                peer,
                attach,
                images: false,
            },
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

    /// A hold begun after another ended between two looks waits its own
    /// `sync_hold`, and the hold ending sends the frame at the clock.
    #[test]
    fn each_synchronized_update_waits_from_its_own_start() {
        let mut em = Emulator::new(20, 4).unwrap();
        let mut hub = Hub::new(HubConfig::default());
        let hold = HubConfig::default().sync_hold;
        let t0 = Instant::now();
        let ms = |n| t0 + Duration::from_millis(n);
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
            GridIn::Attach {
                peer,
                attach,
                images: false,
            },
            &ctx(&em, true),
            t0,
            &mut out,
        );
        let feed = |em: &mut Emulator, bytes: &[u8]| em.feed(bytes, &mut Vec::new());

        feed(&mut em, b"\x1b[?2026hone");
        hub.records(&ctx(&em, true), ms(10), &mut out);
        assert_eq!(hub.due(), Some(ms(10) + hold));

        feed(&mut em, b"\x1b[?2026l\x1b[?2026htw");
        hub.records(&ctx(&em, true), ms(100), &mut out);
        assert_eq!(hub.due(), Some(ms(100) + hold));
        hub.tick(&ctx(&em, true), ms(10) + hold, &mut out);
        assert_eq!(
            hub.due(),
            Some(ms(100) + hold),
            "the second hold is not cut short"
        );

        feed(&mut em, b"o\x1b[?2026l");
        out.clear();
        hub.records(&ctx(&em, true), ms(120), &mut out);
        assert_eq!(hub.due(), None, "released at once");
        assert!(out.iter().any(|o| matches!(o, HubOut::Send { .. })));
    }

    fn frames(out: &[HubOut]) -> usize {
        out.iter()
            .filter(|o| {
                matches!(
                    o,
                    HubOut::Send {
                        msg: ServerMsg::Delta { .. } | ServerMsg::Snapshot { .. },
                        ..
                    }
                )
            })
            .count()
    }

    /// A hub with one visible attachment that has a first frame, and
    /// credit enough that only the clock decides when frames go.
    fn shown(em: &Emulator, t0: Instant) -> (Hub, Peer) {
        let mut hub = Hub::new(HubConfig {
            credits: u8::MAX,
            ..HubConfig::default()
        });
        let attach = Attach {
            session: "s".into(),
            mode: AttachMode::Grid,
            view: Size::default(),
            visible: true,
            resume: None,
            history_tail: 0,
        };
        let peer = Peer { conn: 1, sid: 1 };
        let mut out = Vec::new();
        hub.handle(
            GridIn::Attach {
                peer,
                attach,
                images: false,
            },
            &ctx(em, true),
            t0,
            &mut out,
        );
        hub.records(&ctx(em, true), t0, &mut out);
        assert_eq!(frames(&out), 1);
        (hub, peer)
    }

    /// The messages one connection was sent.
    fn sent(out: &[HubOut], to: u64) -> Vec<&ServerMsg> {
        out.iter()
            .filter_map(|o| match o {
                HubOut::Send { conn, msg } if *conn == to => Some(msg),
                _ => None,
            })
            .collect()
    }

    /// An image's pixels go once to a connection that agreed on images, before the frame
    /// placing it; frames that move it go without them; one that did not agree gets frames
    /// with no placements.
    #[test]
    fn images_go_once_before_the_frame_that_places_them() {
        let mut em = Emulator::with_scrollback(20, 4, 1 << 16).unwrap();
        em.resize_px(20, 4, 200, 80, &mut Vec::new()).unwrap();
        em.set_images(true);
        let mut hub = Hub::new(HubConfig {
            credits: u8::MAX,
            ..HubConfig::default()
        });
        let t0 = Instant::now();
        let mut out = Vec::new();
        for (conn, images) in [(1, true), (2, false)] {
            let attach = Attach {
                session: "s".into(),
                mode: AttachMode::Grid,
                view: Size::default(),
                visible: true,
                resume: None,
                history_tail: 0,
            };
            let peer = Peer { conn, sid: 1 };
            let input = GridIn::Attach {
                peer,
                attach,
                images,
            };
            hub.handle(input, &ctx(&em, true), t0, &mut out);
        }
        // A 1×1 RGBA image, put at the cursor.
        em.feed(b"\x1b_Ga=T,f=32,s=1,v=1,i=5,q=2;AQIDBA==\x1b\\", &mut Vec::new());
        out.clear();
        hub.records(&ctx(&em, true), t0, &mut out);
        let one = sent(&out, 1);
        let ServerMsg::Image { image, .. } = one[0] else {
            panic!("{one:?}");
        };
        assert_eq!((image.id, image.width, image.rgba.as_slice()), (5, 1, &[1, 2, 3, 4][..]));
        let ServerMsg::Snapshot { snap, .. } = one[1] else {
            panic!("{one:?}");
        };
        let p = snap.placements.as_ref().unwrap();
        assert_eq!((p.cell_w, p.cell_h, p.list[0].line), (10, 20, 0));
        let two = sent(&out, 2);
        assert_eq!(two.len(), 1);
        assert!(matches!(two[0], ServerMsg::Snapshot { snap, .. } if snap.placements.is_none()));

        em.feed(b"\r\n\n\n\n", &mut Vec::new());
        out.clear();
        hub.records(&ctx(&em, true), t0 + Duration::from_secs(1), &mut out);
        let one = sent(&out, 1);
        assert_eq!(one.len(), 1, "no pixels again");
        let ServerMsg::Delta { delta, .. } = one[0] else {
            panic!("{one:?}");
        };
        assert_eq!(delta.scrolled, 1);
        assert!(delta.placements.is_none(), "its absolute line stays put as it scrolls");

        em.feed(b"\x1b_Ga=d,d=I,i=5,q=2\x1b\\", &mut Vec::new());
        out.clear();
        hub.records(&ctx(&em, true), t0 + Duration::from_secs(2), &mut out);
        let ServerMsg::Delta { delta, .. } = sent(&out, 1)[0] else {
            panic!();
        };
        assert_eq!(delta.placements.as_ref().map(|p| p.is_empty()), Some(true));
    }

    fn typed(hub: &mut Hub, em: &Emulator, peer: Peer, at: Instant) {
        let mut out = Vec::new();
        let input = GridIn::Input {
            peer,
            input_seq: 1,
            event: InputEvent::Raw {
                bytes: b"x".to_vec(),
            },
        };
        hub.handle(input, &ctx(em, true), at, &mut out);
        assert!(out.iter().any(|o| matches!(o, HubOut::Write { .. })));
    }

    /// Output that answers input is framed as it comes, even just after a
    /// frame, for a few frames; then the clock paces it again.
    #[test]
    fn output_answering_input_skips_the_clock() {
        let mut em = Emulator::new(20, 4).unwrap();
        let t0 = Instant::now();
        let ms = |n| t0 + Duration::from_millis(n);
        let (mut hub, peer) = shown(&em, t0);
        let cfg = HubConfig::default();

        typed(&mut hub, &em, peer, ms(1));
        for n in 0..u64::from(cfg.typed_frames) {
            em.feed(b"a", &mut Vec::new());
            let mut out = Vec::new();
            hub.records(&ctx(&em, true), ms(2 + n), &mut out);
            assert_eq!(frames(&out), 1, "answer {n} framed at once");
            assert_eq!(hub.due(), None);
        }
        em.feed(b"b", &mut Vec::new());
        let mut out = Vec::new();
        let last = ms(1 + u64::from(cfg.typed_frames));
        hub.records(&ctx(&em, true), last + Duration::from_millis(1), &mut out);
        assert_eq!(frames(&out), 0, "past the input's frames the clock paces");
        assert_eq!(hub.due(), Some(last + cfg.frame));
    }

    /// Output with no input before it, or long after the input, keeps the
    /// clock's pace.
    #[test]
    fn output_nobody_typed_for_keeps_the_clock() {
        let mut em = Emulator::new(20, 4).unwrap();
        let t0 = Instant::now();
        let ms = |n| t0 + Duration::from_millis(n);
        let (mut hub, peer) = shown(&em, t0);
        let cfg = HubConfig::default();

        em.feed(b"a", &mut Vec::new());
        let mut out = Vec::new();
        hub.records(&ctx(&em, true), ms(1), &mut out);
        assert_eq!(frames(&out), 0);
        assert_eq!(hub.due(), Some(t0 + cfg.frame));
        hub.tick(&ctx(&em, true), t0 + cfg.frame, &mut out);
        assert_eq!(frames(&out), 1);

        let late = t0 + cfg.frame + cfg.typed;
        typed(&mut hub, &em, peer, t0 + cfg.frame);
        em.feed(b"b", &mut Vec::new());
        let mut out = Vec::new();
        hub.records(&ctx(&em, true), late, &mut out);
        assert_eq!(frames(&out), 1, "the clock had come round anyway");
        em.feed(b"c", &mut Vec::new());
        let mut out = Vec::new();
        hub.records(&ctx(&em, true), late + Duration::from_millis(1), &mut out);
        assert_eq!(frames(&out), 0, "too long after the input to answer it");
        assert_eq!(hub.due(), Some(late + cfg.frame));
    }
}

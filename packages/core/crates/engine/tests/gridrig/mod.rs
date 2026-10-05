//! A session engine with headless grid clients attached, with no sockets
//! and a clock the test moves: what the grid protocol tests drive.
//!
//! Every message a session sends a client is framed and decoded as on the
//! socket, so the codec is exercised with the frames. The clients are
//! `vorn-grid-client`, which links no terminal; the reference they are
//! compared with is read from the session's terminal cell by cell, through
//! grid references, independently of the encoder that made the frames.

#![allow(dead_code)]

use std::collections::BTreeMap;
use std::sync::Arc;
use std::time::{Duration, Instant};

use libghostty_vt::fmt::{Format, Formatter, FormatterOptions};
use libghostty_vt::screen::{CellContentTag, CellWide};
use libghostty_vt::terminal::{Point, PointCoordinate};
use vorn_engine::{Config, GridIn, HubOut, Input, Out, Peer, Session};
use vorn_grid::tables::style_def;
use vorn_grid_client::{Client, Got};
use vorn_screen::Emulator;
use vorn_term_mirror::Mirror;
use vorn_term_proto::msg::{Attach, AttachMode, GridResume, Resume, ServerMsg, Size};
use vorn_term_proto::row;
use vorn_term_proto::screen::{Color, Screen, StyleDef};
use vorn_term_proto::{Cursor, Entry, Record};

/// A history limit that keeps a few thousand lines of an 80-column screen.
pub const SCROLLBACK: usize = 2 << 20;

/// A config that cuts no checkpoints: a cut carries the whole scrollback,
/// which the tests with a large one would spend their time on.
pub fn config(scrollback: usize) -> Arc<Config> {
    cutting(scrollback, u64::MAX)
}

/// A config that also cuts a checkpoint every `bytes` of output, so frames
/// are cut across terminal swaps.
pub fn cutting(scrollback: usize, bytes: u64) -> Arc<Config> {
    Arc::new(Config {
        scrollback,
        cadence: vorn_engine::Cadence {
            bytes,
            ..vorn_engine::Cadence::default()
        },
        build: "test".into(),
        ..Config::default()
    })
}

/// Bytes a client received in frames and in everything else.
#[derive(Debug, Default, Clone, Copy)]
pub struct Traffic {
    pub frames: u64,
    pub frame_bytes: u64,
}

/// Bytes written to the program, with the input event they answer.
pub type Written = (Vec<u8>, Option<(Peer, u64)>);

/// Called with the session, the client's connection and attachment, and the
/// client, after each frame it applied.
pub type OnFrame = Box<dyn FnMut(&Session, u64, u32, &Client)>;

pub struct Rig {
    pub s: Session,
    pub now: Instant,
    pub clients: BTreeMap<u64, Client>,
    /// Frames each connection got and has not acknowledged, oldest first.
    pub unacked: BTreeMap<u64, Vec<(u32, u64)>>,
    /// Every message each connection got, in order.
    pub got: BTreeMap<u64, Vec<Got>>,
    /// Bytes written to the program, with the input event they answer.
    pub writes: Vec<Written>,
    pub traffic: BTreeMap<u64, Traffic>,
    /// Called with the client's connection and its pane after each frame.
    pub on_frame: Option<OnFrame>,
}

impl Rig {
    pub fn new(size: (u16, u16), scrollback: usize) -> Rig {
        Rig::with_config(size, config(scrollback))
    }

    pub fn with_config(size: (u16, u16), cfg: Arc<Config>) -> Rig {
        let s = Session::fresh("s", cfg, size, Cursor::start(0)).unwrap();
        Rig::with(s)
    }

    pub fn with(s: Session) -> Rig {
        Rig {
            s,
            now: Instant::now(),
            clients: BTreeMap::new(),
            unacked: BTreeMap::new(),
            got: BTreeMap::new(),
            writes: Vec::new(),
            traffic: BTreeMap::new(),
            on_frame: None,
        }
    }

    pub fn em(&self) -> &Emulator {
        self.s.emulator().expect("a terminal")
    }

    pub fn client(&mut self, conn: u64) -> &mut Client {
        self.clients
            .entry(conn)
            .or_insert_with(|| Client::hello("test"))
    }

    pub fn mirror(&self, conn: u64, sid: u32) -> &Mirror {
        self.clients[&conn]
            .pane(sid)
            .and_then(|p| p.mirror())
            .expect("a mirror")
    }

    pub fn attach(&mut self, conn: u64, sid: u32, visible: bool, resume: Option<GridResume>) {
        self.attach_with(conn, sid, visible, resume, 0);
    }

    pub fn attach_with(
        &mut self,
        conn: u64,
        sid: u32,
        visible: bool,
        resume: Option<GridResume>,
        history_tail: u16,
    ) {
        self.client(conn);
        self.unacked.entry(conn).or_default().clear();
        self.grid(GridIn::Attach {
            peer: Peer { conn, sid },
            attach: Attach {
                session: "s".into(),
                mode: AttachMode::Grid,
                view: Size {
                    cols: 80,
                    rows: 24,
                    px_w: 0,
                    px_h: 0,
                },
                visible,
                resume: resume.map(Resume::Grid),
                history_tail,
            },
        });
    }

    pub fn detach(&mut self, conn: u64, sid: u32) {
        self.grid(GridIn::Detach {
            peer: Peer { conn, sid },
        });
        // The client keeps its mirror, as an app keeps the last frame on
        // screen, to resume from.
        self.unacked.entry(conn).or_default().clear();
    }

    /// A grid request, as vornd routes it.
    pub fn grid(&mut self, m: GridIn) {
        let mut out = Vec::new();
        self.s.input(Input::Grid(m), self.now, &mut out);
        self.deliver(out);
    }

    /// Records, applied as one batch at the current time.
    pub fn feed(&mut self, entries: &[Entry]) {
        let mut out = Vec::new();
        self.s.apply_all(entries, self.now, &mut out);
        self.deliver(out);
    }

    /// Moves the clock and cuts a frame if one fell due.
    pub fn advance(&mut self, d: Duration) {
        self.now += d;
        let mut out = Vec::new();
        self.s.frame(self.now, &mut out);
        self.deliver(out);
    }

    pub fn deliver(&mut self, out: Vec<Out>) {
        for o in out {
            let Out::Grid(o) = o else { continue };
            match o {
                HubOut::Send { conn, msg } => self.deliver_to(conn, &msg),
                HubOut::Write { bytes, ack } => self.writes.push((bytes, ack)),
            }
        }
    }

    fn deliver_to(&mut self, conn: u64, msg: &ServerMsg) {
        let mut bytes = Vec::new();
        msg.encode(&mut bytes);
        let client = self.client(conn);
        let got = client.receive(&bytes).expect("a frame vornd sent decodes");
        for g in got {
            if let Got::Frame { sid, rev, .. } = &g {
                let t = self.traffic.entry(conn).or_default();
                t.frames += 1;
                t.frame_bytes += bytes.len() as u64;
                self.unacked.entry(conn).or_default().push((*sid, *rev));
                if let Some(f) = &mut self.on_frame {
                    f(&self.s, conn, *sid, &self.clients[&conn]);
                }
            }
            if let Got::Refused { sid, why } = &g {
                panic!("conn {conn} sid {sid} refused a frame: {why:?}");
            }
            self.got.entry(conn).or_default().push(g);
        }
    }

    /// Acknowledges every frame connection `conn` holds.
    pub fn ack(&mut self, conn: u64) {
        let pending = std::mem::take(self.unacked.entry(conn).or_default());
        for (sid, rev) in pending {
            self.grid(GridIn::Ack {
                peer: Peer { conn, sid },
                rev,
            });
        }
    }

    pub fn ack_all(&mut self) {
        let conns: Vec<u64> = self.unacked.keys().copied().collect();
        for c in conns {
            self.ack(c);
        }
    }

    /// Runs the clock, acknowledging everything, until no frame is due or
    /// in flight: every client then shows the terminal as it is now.
    pub fn settle(&mut self) {
        for _ in 0..1000 {
            self.advance(Duration::from_millis(20));
            let busy = self.unacked.values().any(|v| !v.is_empty());
            self.ack_all();
            if !busy && self.s.due().is_none() {
                return;
            }
        }
        panic!("the grid never settled");
    }
}

/// One column of a screen, as both sides compare it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Col {
    Cell {
        text: String,
        wide: bool,
        style: StyleDef,
        link: Option<String>,
    },
    /// The second column of a wide character.
    Spacer,
}

fn blank() -> Col {
    Col::Cell {
        text: String::new(),
        wide: false,
        style: StyleDef::default(),
        link: None,
    }
}

/// An empty cell and a space look alike; the encoding keeps the
/// difference only before the end of a row.
fn norm(text: &str) -> &str {
    if text == " " {
        ""
    } else {
        text
    }
}

/// The terminal's viewport, read cell by cell through grid references,
/// with the one accepted difference applied ([`spacerless_wide`]).
pub fn terminal_cols(em: &Emulator) -> Vec<Vec<Col>> {
    let mut rows = raw_terminal_cols(em);
    for r in &mut rows {
        spacerless_wide(r);
    }
    rows
}

/// Accepted difference, by name: the row encoding implies a spacer after a
/// wide cell, so a wide cell Ghostty left without its spacer (at the last
/// column, or after a resize) arrives narrow, and a spacer with no wide
/// cell before it arrives as an empty cell. Every cell keeps its column.
pub fn spacerless_wide(row: &mut [Col]) {
    for x in 0..row.len() {
        let next_is_spacer = matches!(row.get(x + 1), Some(Col::Spacer));
        match &mut row[x] {
            Col::Cell { wide, .. } if *wide && !next_is_spacer => *wide = false,
            Col::Spacer => {
                let after_wide = x > 0 && matches!(&row[x - 1], Col::Cell { wide: true, .. });
                if !after_wide {
                    row[x] = blank();
                }
            }
            _ => {}
        }
    }
}

fn raw_terminal_cols(em: &Emulator) -> Vec<Vec<Col>> {
    let t = em.terminal();
    let (cols, rows) = (t.cols().unwrap(), t.rows().unwrap());
    let mut chars = vec!['\0'; 64];
    let mut uri = vec![0u8; 4096];
    (0..rows)
        .map(|y| {
            (0..cols)
                .map(|x| {
                    let at = Point::Viewport(PointCoordinate { x, y: u32::from(y) });
                    let g = t.grid_ref(at).unwrap();
                    let cell = g.cell().unwrap();
                    let wide = cell.wide().unwrap();
                    if wide == CellWide::SpacerTail {
                        return Col::Spacer;
                    }
                    let mut text = String::new();
                    if wide != CellWide::SpacerHead {
                        let n = loop {
                            match g.graphemes(&mut chars) {
                                Ok(n) => break n,
                                Err(libghostty_vt::Error::OutOfSpace { required }) => {
                                    chars.resize(required, '\0')
                                }
                                Err(e) => panic!("{e:?}"),
                            }
                        };
                        text.extend(&chars[..n]);
                    }
                    let bg = match cell.content_tag().unwrap() {
                        CellContentTag::BgColorPalette => {
                            Some(Color::Palette(cell.bg_color_palette().unwrap().0))
                        }
                        CellContentTag::BgColorRgb => {
                            let c = cell.bg_color_rgb().unwrap();
                            Some(Color::Rgb(c.r, c.g, c.b))
                        }
                        _ => None,
                    };
                    let mut style = if cell.has_styling().unwrap() || bg.is_some() {
                        style_def(&g.style().unwrap())
                    } else {
                        StyleDef::default()
                    };
                    if let Some(bg) = bg {
                        style.bg = bg;
                    }
                    let link = cell.has_hyperlink().unwrap().then(|| {
                        let n = g.hyperlink_uri(&mut uri).unwrap();
                        String::from_utf8(uri[..n].to_vec()).unwrap()
                    });
                    let link = link.filter(|l| !l.is_empty());
                    Col::Cell {
                        text: norm(&text).to_owned(),
                        wide: wide == CellWide::Wide,
                        style,
                        link,
                    }
                })
                .collect()
        })
        .collect()
}

/// A mirror's viewport, column by column, with its styles and links
/// looked up in its own tables.
pub fn mirror_cols(m: &Mirror) -> Vec<Vec<Col>> {
    let cols = usize::from(m.term().cols);
    m.rows()
        .iter()
        .map(|r| {
            let mut out = Vec::with_capacity(cols);
            for run in row::decode(&r.cells).expect("rows decode") {
                let mut style = m
                    .style(run.style)
                    .expect("a style the mirror holds")
                    .clone();
                style.id = 0;
                let link = (run.link != 0).then(|| {
                    m.link(run.link)
                        .expect("a link the mirror holds")
                        .uri
                        .clone()
                });
                for c in run.cells {
                    out.push(Col::Cell {
                        text: norm(&c.text).to_owned(),
                        wide: c.wide,
                        style: style.clone(),
                        link: link.clone(),
                    });
                    if c.wide {
                        out.push(Col::Spacer);
                    }
                }
            }
            out.resize(cols, blank());
            out
        })
        .collect()
}

/// TP-T2's comparison: the mirror shows the terminal cell for cell (text,
/// style, width, link), with the same size, screen, cursor and title, and
/// its text is the formatter's plain output of the screen.
pub fn assert_same(em: &Emulator, m: &Mirror, what: &str) {
    let t = em.terminal();
    let term = m.term();
    assert_eq!(
        (term.cols, term.rows),
        (t.cols().unwrap(), t.rows().unwrap()),
        "{what}: size"
    );
    let screen = match t.active_screen().unwrap() {
        libghostty_vt::screen::Screen::Primary => Screen::Primary,
        libghostty_vt::screen::Screen::Alternate => Screen::Alternate,
    };
    assert_eq!(term.screen, screen, "{what}: screen");
    let want = terminal_cols(em);
    let got = mirror_cols(m);
    for (y, (w, g)) in want.iter().zip(&got).enumerate() {
        if w != g {
            let x = w.iter().zip(g).position(|(a, b)| a != b).unwrap_or(0);
            let show = |r: &[Col]| -> String {
                r.iter()
                    .map(|c| match c {
                        Col::Spacer => "|".to_owned(),
                        Col::Cell {
                            text,
                            wide,
                            style,
                            link,
                        } => format!(
                            "{}{}{}{}",
                            if text.is_empty() { "." } else { text },
                            if *wide { "W" } else { "" },
                            if *style != StyleDef::default() {
                                "*"
                            } else {
                                ""
                            },
                            if link.is_some() { "@" } else { "" }
                        ),
                    })
                    .collect()
            };
            panic!(
                "{what}: row {y} differs from column {x}\n terminal: {:?}\n   mirror: {:?}\n terminal row: {}\n   mirror row: {}",
                &w[x..(x + 2).min(w.len())],
                &g[x..(x + 2).min(g.len())],
                show(w),
                show(g),
            );
        }
    }
    assert_eq!(want.len(), got.len(), "{what}: rows");
    assert_eq!(term.title, em.title(), "{what}: title");
    let cur_x = t.cursor_x().unwrap();
    let cur_y = t.cursor_y().unwrap();
    if term.cursor.visible {
        assert_eq!(
            (term.cursor.x, term.cursor.y),
            (cur_x, cur_y),
            "{what}: cursor"
        );
    }
    assert_plain(em, m, what);
}

/// The mirror's rows as text are the last rows of the formatter's plain
/// output (which holds history above them and drops trailing blank rows).
pub fn assert_plain(em: &Emulator, m: &Mirror, what: &str) {
    let opts = FormatterOptions::new().with_format(Format::Plain);
    let plain = Formatter::new(em.terminal(), opts)
        .and_then(|mut f| f.format_alloc(None))
        .map(|b| String::from_utf8_lossy(&b).into_owned())
        .unwrap();
    let formatted: Vec<&str> = plain.split('\n').map(|l| l.trim_end_matches(' ')).collect();
    let mut mine = m.text();
    while mine.last().is_some_and(String::is_empty) {
        mine.pop();
    }
    if mine.is_empty() {
        return;
    }
    let mut formatted: Vec<&str> = formatted;
    while formatted.last().is_some_and(|l| l.is_empty()) {
        formatted.pop();
    }
    assert!(
        formatted.len() >= mine.len()
            && formatted[formatted.len() - mine.len()..]
                .iter()
                .zip(&mine)
                .all(|(a, b)| a == b),
        "{what}: plain text differs\n formatter tail: {:?}\n mirror: {:?}",
        &formatted[formatted.len().saturating_sub(mine.len())..],
        mine
    );
}

/// The data bytes of `entries`.
pub fn data_len(entries: &[Entry]) -> u64 {
    entries
        .iter()
        .map(|e| match &e.rec {
            Record::Data { bytes, .. } => bytes.len() as u64,
            _ => 0,
        })
        .sum()
}

//! A session's terminal: libghostty-vt fed one write per flush, its labels and counters, and its effects.

use std::cell::{Cell, RefCell};
use std::collections::HashMap;
use std::rc::Rc;

use libghostty_vt::terminal::{
    ConformanceLevel, DeviceAttributeFeature, DeviceAttributes, DeviceType,
    PrimaryDeviceAttributes, SecondaryDeviceAttributes, SizeReportSize, Terminal,
    TertiaryDeviceAttributes,
};

use crate::scan::Scan;
use crate::{clip_units, dimension, is_plausible_path, percent_decode, Result};

/// The longest unfinished sequence a checkpoint carries; inside a longer one none is cut.
pub(crate) const MAX_CONTINUATION: usize = 1 << 20;

/// What the terminal asks its host to do.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Effect {
    /// Bytes answering a query, for the program's input; only a live terminal sends them.
    Reply(Vec<u8>),
    Bell,
    /// A clipboard write, decoded; empty contents clear.
    Clipboard {
        location: ClipboardTarget,
        contents: Vec<(String, String)>,
    },
    /// An OSC 9 or OSC 777 desktop notification.
    Notify {
        title: String,
        body: String,
    },
    /// The cwd an OSC 5522 moved to, once per move.
    Cwd(String),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ClipboardTarget {
    Standard,
    Selection,
    Primary,
}

/// Bits of [`Emulator::changed`].
const TITLE: u8 = 1;
const PWD: u8 = 2;

/// Answers a device attributes query as Ghostty itself does.
const PRIMARY_DA: PrimaryDeviceAttributes = PrimaryDeviceAttributes::new(
    ConformanceLevel::VT220,
    &[DeviceAttributeFeature::ANSI_COLOR],
);

/// What [`Emulator::parsed_bytes`], [`Emulator::joiners_printed`] and
/// [`Emulator::history_clears`] stand at: kept beside a checkpoint by a host
/// that drops a terminal and rebuilds it later.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct Counters {
    parsed: u64,
    joiners: u64,
    clears: u64,
}

#[derive(Debug)]
pub struct Emulator {
    pub(crate) term: Terminal<'static, 'static>,
    effects: Rc<RefCell<Vec<Effect>>>,
    size: Rc<Cell<(u16, u16)>>,
    pub(crate) cols: u16,
    pub(crate) rows: u16,
    /// Title and cwd as the session record keeps them (see [`crate::Screen`]).
    pub(crate) title: String,
    pub(crate) cwd: String,
    pub(crate) scan: Scan,
    /// Which of Ghostty's title and pwd the last write changed.
    changed: Rc<Cell<u8>>,
    /// Codepoints the last feed printed that may have joined a cell.
    joins: Vec<u32>,
    /// Whether Ghostty gives each codepoint asked about no width, so it joins the cell before.
    zero_width: HashMap<u32, bool>,
    /// Output bytes handed to Ghostty by [`Emulator::feed`].
    pub(crate) parsed: u64,
    /// Feeds that printed a zero-width codepoint (see [`Emulator::joiners_printed`]).
    pub(crate) joiners: u64,
}

impl Emulator {
    /// A terminal that keeps no history, as a session engine runs it.
    pub fn new(cols: u32, rows: u32) -> Result<Self> {
        Self::with_scrollback(cols, rows, 0)
    }

    /// A terminal keeping `max_scrollback` bytes of history pages, which checkpoints carry.
    pub fn with_scrollback(cols: u32, rows: u32, max_scrollback: usize) -> Result<Self> {
        let mut term = crate::new_terminal(cols, rows, max_scrollback)?;
        // Tracked from the first byte, so a checkpoint can be cut anywhere.
        term.set_continuation_max_bytes(MAX_CONTINUATION)?;
        Self::wrap(term)
    }

    /// Wraps a new or decoded terminal with the callbacks that turn what it raises into effects.
    pub(crate) fn wrap(mut term: Terminal<'static, 'static>) -> Result<Self> {
        let (cols, rows) = (term.cols()?, term.rows()?);
        let effects = Rc::new(RefCell::new(Vec::new()));
        let size = Rc::new(Cell::new((cols, rows)));
        let sink = Rc::clone(&effects);
        term.on_pty_write(move |_, bytes| sink.borrow_mut().push(Effect::Reply(bytes.to_vec())))?;
        let sink = Rc::clone(&effects);
        term.on_bell(move |_| sink.borrow_mut().push(Effect::Bell))?;
        let sink = Rc::clone(&effects);
        term.on_clipboard_write(move |_, write| {
            use libghostty_vt::terminal::ClipboardLocation as L;
            let location = match write.location() {
                L::Selection => ClipboardTarget::Selection,
                L::Primary => ClipboardTarget::Primary,
                _ => ClipboardTarget::Standard,
            };
            let contents = write
                .contents()
                .map(|c| {
                    (
                        c.mime.to_owned(),
                        String::from_utf8_lossy(c.data).into_owned(),
                    )
                })
                .collect();
            sink.borrow_mut()
                .push(Effect::Clipboard { location, contents });
            // The host carries the write out; the program hears it was done.
            write.reply(Ok(()), false);
        })?;
        let sink = Rc::clone(&effects);
        term.on_desktop_notification(move |_, n| {
            sink.borrow_mut().push(Effect::Notify {
                title: String::from_utf8_lossy(n.title()).into_owned(),
                body: String::from_utf8_lossy(n.body()).into_owned(),
            })
        })?;
        term.on_device_attributes(|_| {
            Some(DeviceAttributes {
                primary: PRIMARY_DA,
                secondary: SecondaryDeviceAttributes {
                    device_type: DeviceType::VT220,
                    firmware_version: 1,
                    rom_cartridge: 0,
                },
                tertiary: TertiaryDeviceAttributes { unit_id: 0 },
            })
        })?;
        let dims = Rc::clone(&size);
        term.on_size(move |_| {
            let (columns, rows) = dims.get();
            Some(SizeReportSize {
                rows,
                columns,
                cell_width: 1,
                cell_height: 1,
            })
        })?;
        term.on_xtversion(|_| Some("vorn"))?;
        let changed = Rc::new(Cell::new(0));
        let flag = Rc::clone(&changed);
        term.on_title_changed(move |_| flag.set(flag.get() | TITLE))?;
        let flag = Rc::clone(&changed);
        term.on_pwd_changed(move |_| flag.set(flag.get() | PWD))?;
        Ok(Self {
            term,
            effects,
            size,
            cols,
            rows,
            title: String::new(),
            cwd: String::new(),
            scan: Scan::default(),
            changed,
            joins: Vec::new(),
            zero_width: HashMap::new(),
            parsed: 0,
            joiners: 0,
        })
    }

    pub fn cols(&self) -> u16 {
        self.cols
    }

    pub fn rows(&self) -> u16 {
        self.rows
    }

    /// Ghostty's history limit this terminal keeps, in bytes.
    pub fn scrollback_limit(&self) -> usize {
        self.term
            .scrollback_max_bytes()
            .ok()
            .flatten()
            .unwrap_or(usize::MAX)
    }

    /// The terminal itself, read only, for comparing states.
    pub fn terminal(&self) -> &Terminal<'static, 'static> {
        &self.term
    }

    /// The last OSC 0/2 title, as the session record keeps it.
    pub fn title(&self) -> &str {
        &self.title
    }

    /// The last cwd from OSC 7 or OSC 5522.
    pub fn cwd(&self) -> &str {
        &self.cwd
    }

    /// Output bytes [`Emulator::feed`] passed to Ghostty, across cuts: the one-parse counter (TP-T1).
    pub fn parsed_bytes(&self) -> u64 {
        self.parsed
    }

    /// Feeds that printed a joining codepoint, whose row Ghostty does not mark dirty outside mode 2027.
    pub fn joiners_printed(&self) -> u64 {
        self.joiners
    }

    /// Scrollback clears (ED 3) and resets (RIS): history line numbers start again when this moves.
    pub fn history_clears(&self) -> u64 {
        self.scan.clears
    }

    /// The `(fg, bg)` defaults OSC 10 and 11 queries are answered with; `None` unsets them.
    pub fn set_default_colors(&mut self, colors: Option<([u8; 3], [u8; 3])>) -> Result<()> {
        let rgb = |c: [u8; 3]| libghostty_vt::style::RgbColor {
            r: c[0],
            g: c[1],
            b: c[2],
        };
        self.term
            .set_default_fg_color(colors.map(|(fg, _)| rgb(fg)))?;
        self.term
            .set_default_bg_color(colors.map(|(_, bg)| rgb(bg)))?;
        Ok(())
    }

    /// No sequence or UTF-8 character open: where Ghostty's VT formatter describes the whole state.
    pub fn at_ground(&self) -> bool {
        self.term.is_vt_ground().unwrap_or(false)
    }

    /// Feeds one flush of output, appending what it asks of the host to `out`.
    pub fn feed(&mut self, bytes: &[u8], out: &mut Vec<Effect>) {
        self.parsed += bytes.len() as u64;
        self.term.vt_write(bytes);
        let mut joins = std::mem::take(&mut self.joins);
        let (cwd, effects) = (&mut self.cwd, &self.effects);
        self.scan.feed(
            bytes,
            |payload| {
                if let Some(moved) = private_cwd(cwd, payload) {
                    effects.borrow_mut().push(Effect::Cwd(moved));
                }
            },
            &mut joins,
        );
        // After the OSC 5522 reports, so one in the same feed is not hidden.
        let changed = self.changed.replace(0);
        if changed & TITLE != 0 {
            if let Ok(title) = self.term.title() {
                self.title = clip_units(title);
            }
        }
        if changed & PWD != 0 {
            if let Some(path) = self.term.pwd().ok().and_then(url_path) {
                self.cwd = path;
            }
        }
        if joins.drain(..).any(|cp| self.zero_width(cp)) {
            self.joiners += 1;
        }
        self.joins = joins;
        out.append(&mut self.effects.borrow_mut());
    }

    /// Resizes, appending any in-band size report to `out`.
    pub fn resize(&mut self, cols: u32, rows: u32, out: &mut Vec<Effect>) -> Result<()> {
        let (c, r) = (dimension(cols)?, dimension(rows)?);
        if (c, r) == (self.cols, self.rows) {
            return Ok(());
        }
        self.term.resize(c, r, 1, 1)?;
        self.cols = c;
        self.rows = r;
        self.size.set((c, r));
        out.append(&mut self.effects.borrow_mut());
        Ok(())
    }

    /// Whether Ghostty gives `cp` no width, asked of its own tables once.
    fn zero_width(&mut self, cp: u32) -> bool {
        *self
            .zero_width
            .entry(cp)
            .or_insert_with(|| probe_zero_width(cp).unwrap_or(true))
    }

    /// The counters that follow the output, which a checkpoint does not carry.
    pub fn counters(&self) -> Counters {
        Counters {
            parsed: self.parsed,
            joiners: self.joiners,
            clears: self.scan.clears,
        }
    }

    /// Takes up `c`, as a terminal rebuilt in place of the one they were read from.
    pub fn set_counters(&mut self, c: Counters) {
        self.parsed = c.parsed;
        self.joiners = c.joiners;
        self.scan.clears = c.clears;
    }

    /// Takes the counters and caches that follow the output from the terminal this replaces.
    pub(crate) fn carry_counters(&mut self, from: &mut Emulator) {
        self.parsed = from.parsed;
        self.joiners = from.joiners;
        self.scan.clears = from.scan.clears;
        self.zero_width = std::mem::take(&mut from.zero_width);
    }
}

/// An OSC 5522 report: the cwd it moved to, once per move.
fn private_cwd(cwd: &mut String, payload: &[u8]) -> Option<String> {
    let rest = payload.strip_prefix(b"cwd;")?;
    let next = clip_units(&String::from_utf8_lossy(rest));
    if next.is_empty() || !is_plausible_path(&next) || next == *cwd {
        return None;
    }
    cwd.clone_from(&next);
    Some(next)
}

/// The path of an OSC 7 URL (or a bare path), percent-decoded.
fn url_path(raw: &str) -> Option<String> {
    let path = match raw.strip_prefix("file://") {
        Some(rest) => rest.find('/').map_or("", |at| &rest[at..]),
        None => raw,
    };
    (!path.is_empty()).then(|| clip_units(&percent_decode(path).unwrap_or_else(|| path.to_owned())))
}

/// Prints `cp` after a narrow character: the cursor stays in column 1 only if it joined it.
fn probe_zero_width(cp: u32) -> Option<bool> {
    let ch = char::from_u32(cp)?;
    let mut t = Terminal::new(16, 1).ok()?;
    let mut bytes = b"a".to_vec();
    let mut buf = [0; 4];
    bytes.extend_from_slice(ch.encode_utf8(&mut buf).as_bytes());
    t.vt_write(&bytes);
    Some(t.cursor_x().ok()? == 1)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// ED 3, DECSED 3 and RIS each count as a scrollback clear; ED 2 does not.
    #[test]
    fn scrollback_clears_are_counted() {
        let mut em = Emulator::with_scrollback(10, 3, 1 << 16).unwrap();
        let mut fx = Vec::new();
        for (bytes, want) in [
            (&b"a\r\nb\r\nc\r\nd\r\n"[..], 0),
            (b"\x1b[?3J", 1),
            (b"\x1b[2J", 1),
            (b"\x1b[3J", 2),
            (b"\x1bc", 3),
        ] {
            em.feed(bytes, &mut fx);
            assert_eq!(em.history_clears(), want, "{bytes:?}");
        }
    }

    #[test]
    fn joiners_are_counted_per_feed_by_ghosttys_widths() {
        let mut em = Emulator::new(20, 3).unwrap();
        let mut fx = Vec::new();
        em.feed("é ─ ✓ 日本".as_bytes(), &mut fx);
        assert_eq!(em.joiners_printed(), 0);
        em.feed("e\u{301}".as_bytes(), &mut fx);
        assert_eq!(em.joiners_printed(), 1);
        // Greek takes a cell of its own, though the filter lets it through.
        em.feed("αβγ".as_bytes(), &mut fx);
        assert_eq!(em.joiners_printed(), 1);
        em.feed("👍\u{1f3fd} a\u{200d}".as_bytes(), &mut fx);
        assert_eq!(em.joiners_printed(), 2);
    }

    #[test]
    fn effects_come_out_in_order_with_labels_last() {
        let mut em = Emulator::new(20, 3).unwrap();
        let mut fx = Vec::new();
        em.feed(
            b"\x1b]5522;cwd;/srv\x07\x07\x1b]9;done\x07\x1b]52;c;aGk=\x07\x1b]2;t\x07",
            &mut fx,
        );
        assert_eq!(
            fx,
            [
                Effect::Bell,
                Effect::Notify {
                    title: String::new(),
                    body: "done".into()
                },
                Effect::Clipboard {
                    location: ClipboardTarget::Standard,
                    contents: vec![("text/plain".into(), "hi".into())]
                },
                Effect::Cwd("/srv".into()),
            ]
        );
        assert_eq!((em.title(), em.cwd()), ("t", "/srv"));
    }
}

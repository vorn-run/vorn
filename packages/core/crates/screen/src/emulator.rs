//! A session's terminal: libghostty-vt fed one write per flush, its labels and counters, and its effects.

use std::cell::{Cell, RefCell};
use std::rc::Rc;

use libghostty_vt::style::RgbColor;
use libghostty_vt::terminal::{
    ColorScheme, ConformanceLevel, DeviceAttributeFeature, DeviceAttributes, DeviceType, Mode,
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
    /// An OSC 9;4 progress report: a percentage from 0 to 100 when given.
    Progress {
        state: ProgressState,
        percent: Option<u8>,
    },
}

/// What an OSC 9;4 progress report shows.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ProgressState {
    Remove,
    Set,
    Error,
    Indeterminate,
    Pause,
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

/// A theme's colours, as RGB: what OSC 4 and 10 to 12 queries are answered with.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Theme {
    pub fg: [u8; 3],
    pub bg: [u8; 3],
    /// `None` draws the cursor in the cell's colours.
    pub cursor: Option<[u8; 3]>,
    /// The sixteen ANSI colours; the other 240 palette entries keep Ghostty's.
    pub ansi: Option<[[u8; 3]; 16]>,
}

impl Theme {
    /// Foreground and background only.
    pub const fn new(fg: [u8; 3], bg: [u8; 3]) -> Self {
        Theme {
            fg,
            bg,
            cursor: None,
            ansi: None,
        }
    }
}

#[derive(Debug)]
pub struct Emulator {
    pub(crate) term: Terminal<'static, 'static>,
    effects: Rc<RefCell<Vec<Effect>>>,
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
    /// Output bytes handed to Ghostty by [`Emulator::feed`].
    pub(crate) parsed: u64,
    /// Feeds that printed a zero-width codepoint (see [`Emulator::joiners_printed`]).
    pub(crate) joiners: u64,
    /// Synchronized output holds started, counted by Ghostty's render hold callback.
    holds: Rc<Cell<u64>>,
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
        // Read from the terminal, so a report raised by a resize carries the new size.
        term.on_size(|t| {
            let (columns, rows) = (t.cols().ok()?, t.rows().ok()?);
            let (cell_width, cell_height) = cell_px(t)?;
            Some(SizeReportSize {
                rows,
                columns,
                cell_width,
                cell_height,
            })
        })?;
        term.on_xtversion(|_| Some("vorn"))?;
        // CSI ? 996 n: dark when the theme's background is darker than its text; unanswered
        // without a theme.
        term.on_color_scheme(|t| {
            let fg = t.default_fg_color().ok()??;
            let bg = t.default_bg_color().ok()??;
            Some(if luma(bg) < luma(fg) {
                ColorScheme::Dark
            } else {
                ColorScheme::Light
            })
        })?;
        let sink = Rc::clone(&effects);
        term.on_progress_report(move |_, report| {
            use libghostty_vt::terminal::ProgressState as P;
            let state = match report.state() {
                Ok(P::Remove) => ProgressState::Remove,
                Ok(P::Set) => ProgressState::Set,
                Ok(P::Error) => ProgressState::Error,
                Ok(P::Indeterminate) => ProgressState::Indeterminate,
                Ok(P::Pause) => ProgressState::Pause,
                _ => return,
            };
            let percent = report.progress().map(|p| p.min(100));
            sink.borrow_mut().push(Effect::Progress { state, percent });
        })?;
        let changed = Rc::new(Cell::new(0));
        let flag = Rc::clone(&changed);
        term.on_title_changed(move |_| flag.set(flag.get() | TITLE))?;
        let flag = Rc::clone(&changed);
        term.on_pwd_changed(move |_| flag.set(flag.get() | PWD))?;
        let holds = Rc::new(Cell::new(0));
        let count = Rc::clone(&holds);
        term.on_render_hold(move |_, held| {
            if held {
                count.set(count.get() + 1);
            }
        })?;
        Ok(Self {
            term,
            effects,
            cols,
            rows,
            title: String::new(),
            cwd: String::new(),
            scan: Scan::default(),
            changed,
            joins: Vec::new(),
            parsed: 0,
            joiners: 0,
            holds,
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

    /// While synchronized output (mode 2026) holds the screen, which hold this is: holds are
    /// counted as they start, so a renderer that polls can tell a new hold begun since it last
    /// looked from the one it already waited on.
    pub fn render_hold(&self) -> Option<u64> {
        // The mode says whether a hold is on: a rebuilt terminal restores it without calling back.
        self.term
            .mode(Mode::SYNC_OUTPUT)
            .unwrap_or(false)
            .then(|| self.holds.get())
    }

    /// Scrollback clears (ED 3) and resets (RIS): history line numbers start again when this moves.
    pub fn history_clears(&self) -> u64 {
        self.scan.clears
    }

    /// The theme colour queries are answered with and renderers draw defaults in; `None` unsets it.
    pub fn set_theme(&mut self, theme: Option<&Theme>) -> Result<()> {
        let rgb = |c: [u8; 3]| RgbColor {
            r: c[0],
            g: c[1],
            b: c[2],
        };
        self.term.set_default_fg_color(theme.map(|t| rgb(t.fg)))?;
        self.term.set_default_bg_color(theme.map(|t| rgb(t.bg)))?;
        self.term
            .set_default_cursor_color(theme.and_then(|t| t.cursor).map(rgb))?;
        // Ghostty's own palette first, so the entries past the theme's are its own.
        self.term.set_default_color_palette(None)?;
        if let Some(ansi) = theme.and_then(|t| t.ansi) {
            let mut palette = self.term.default_color_palette()?;
            for (slot, c) in palette.0.iter_mut().zip(ansi) {
                *slot = rgb(c);
            }
            self.term.set_default_color_palette(Some(palette))?;
        }
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
        if joins.drain(..).any(zero_width) {
            self.joiners += 1;
        }
        self.joins = joins;
        out.append(&mut self.effects.borrow_mut());
    }

    /// Resizes with cells of unknown pixel size, appending any in-band size report to `out`.
    pub fn resize(&mut self, cols: u32, rows: u32, out: &mut Vec<Effect>) -> Result<()> {
        self.resize_px(cols, rows, 0, 0, out)
    }

    /// Resizes to a window of `px_w`×`px_h` pixels, as the PTY is told (0 when unknown), so size
    /// reports and pixel mouse reports carry the real cell size.
    pub fn resize_px(
        &mut self,
        cols: u32,
        rows: u32,
        px_w: u32,
        px_h: u32,
        out: &mut Vec<Effect>,
    ) -> Result<()> {
        let (c, r) = (dimension(cols)?, dimension(rows)?);
        // An unknown size keeps one pixel a cell, so the encoders still count in cells.
        let cell = ((px_w / cols).max(1), (px_h / rows).max(1));
        if (c, r) == (self.cols, self.rows) && Some(cell) == cell_px(&self.term) {
            return Ok(());
        }
        self.term.resize(c, r, cell.0, cell.1)?;
        self.cols = c;
        self.rows = r;
        out.append(&mut self.effects.borrow_mut());
        Ok(())
    }

    /// The pixel size of one cell, one pixel when the window's size is unknown.
    pub fn cell_px(&self) -> (u32, u32) {
        cell_px(&self.term).unwrap_or((1, 1))
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

    /// Takes the counters that follow the output from the terminal this replaces.
    pub(crate) fn carry_counters(&mut self, from: &Emulator) {
        self.set_counters(from.counters());
        self.holds.set(from.holds.get());
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

/// Relative luminance, enough to tell a light colour from a dark one.
fn luma(c: RgbColor) -> u32 {
    2126 * u32::from(c.r) + 7152 * u32::from(c.g) + 722 * u32::from(c.b)
}

/// The cell size the last resize gave `t`, one pixel when it was not told one.
fn cell_px(t: &Terminal<'_, '_>) -> Option<(u32, u32)> {
    let (cols, rows) = (u32::from(t.cols().ok()?), u32::from(t.rows().ok()?));
    let w = t.width_px().ok()?.checked_div(cols)?.max(1);
    let h = t.height_px().ok()?.checked_div(rows)?.max(1);
    Some((w, h))
}

/// Whether Ghostty's print gives `cp` no width, so it joins the cell before: the same table it prints with.
fn zero_width(cp: u32) -> bool {
    char::from_u32(cp).is_none_or(|c| libghostty_vt::unicode::codepoint_width(c) == 0)
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

    fn replies(fx: Vec<Effect>) -> Vec<String> {
        fx.into_iter()
            .filter_map(|f| match f {
                Effect::Reply(b) => Some(String::from_utf8(b).unwrap()),
                _ => None,
            })
            .collect()
    }

    #[test]
    fn size_reports_carry_the_cell_pixels_and_the_new_size() {
        let mut em = Emulator::new(80, 24).unwrap();
        let mut fx = Vec::new();
        em.feed(b"\x1b[14t\x1b[16t", &mut fx);
        assert_eq!(
            replies(std::mem::take(&mut fx)),
            ["\x1b[4;24;80t", "\x1b[6;1;1t"]
        );
        em.feed(b"\x1b[?2048h", &mut fx);
        fx.clear();
        em.resize_px(100, 30, 1000, 600, &mut fx).unwrap();
        assert_eq!(em.cell_px(), (10, 20));
        assert_eq!(
            replies(std::mem::take(&mut fx)),
            ["\x1b[48;30;100;600;1000t"]
        );
        em.feed(b"\x1b[14t\x1b[16t", &mut fx);
        assert_eq!(
            replies(std::mem::take(&mut fx)),
            ["\x1b[4;600;1000t", "\x1b[6;20;10t"]
        );
        // A new pixel size alone is a resize too; the same one is not.
        em.resize_px(100, 30, 1200, 600, &mut fx).unwrap();
        assert_eq!(em.cell_px(), (12, 20));
        assert_eq!(fx.len(), 1);
        em.resize_px(100, 30, 1200, 600, &mut fx).unwrap();
        assert_eq!(fx.len(), 1);
    }

    /// The width table agrees with what printing after a narrow character does.
    #[test]
    fn zero_width_is_what_ghosttys_print_joins() {
        let mut t = Terminal::new(16, 1).unwrap();
        let joined = |t: &mut Terminal<'static, 'static>, c: char| {
            let mut bytes = b"\x1b[Ha".to_vec();
            bytes.extend_from_slice(c.encode_utf8(&mut [0; 4]).as_bytes());
            t.vt_write(&bytes);
            t.cursor_x().unwrap() == 1
        };
        let sample = (0x300..0x3000).chain((0x3000..0x11_0000).step_by(7));
        let mut zero = 0;
        for c in sample
            .filter(|&cp| crate::scan::may_join(cp))
            .filter_map(char::from_u32)
        {
            assert_eq!(
                zero_width(u32::from(c)),
                joined(&mut t, c),
                "U+{:04X}",
                u32::from(c)
            );
            zero += usize::from(zero_width(u32::from(c)));
        }
        assert!(zero > 100, "{zero}");
        assert!(zero_width(0xd800), "not a character");
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

    #[test]
    fn render_holds_are_numbered_and_survive_a_cut() {
        let mut em = Emulator::with_scrollback(20, 4, 1 << 16).unwrap();
        let mut fx = Vec::new();
        assert_eq!(em.render_hold(), None);
        em.feed(b"\x1b[?2026ha\x1b[?2026h", &mut fx);
        assert_eq!(em.render_hold(), Some(1));
        em.feed(b"\x1b[?2026l\x1b[?2026hb", &mut fx);
        assert_eq!(em.render_hold(), Some(2));
        em.checkpoint().unwrap();
        assert_eq!(em.render_hold(), Some(2));
        em.feed(b"\x1b[?2026l", &mut fx);
        assert_eq!(em.render_hold(), None);
        em.feed(b"\x1b[?2026h", &mut fx);
        assert_eq!(em.render_hold(), Some(3));
    }

    #[test]
    fn progress_reports_are_effects_and_the_theme_answers_the_scheme_query() {
        let mut em = Emulator::new(20, 3).unwrap();
        let mut fx = Vec::new();
        em.feed(b"\x1b]9;4;1;42\x07\x1b]9;4;3\x07\x1b]9;4;0\x07", &mut fx);
        let progress = |state, percent| Effect::Progress { state, percent };
        assert_eq!(
            fx,
            [
                progress(ProgressState::Set, Some(42)),
                progress(ProgressState::Indeterminate, None),
                progress(ProgressState::Remove, None),
            ]
        );
        fx.clear();
        em.feed(b"\x1b[?996n", &mut fx);
        assert!(fx.is_empty(), "no theme, no answer: {fx:?}");
        em.set_theme(Some(&Theme::new([0xd4; 3], [0x14; 3])))
            .unwrap();
        em.feed(b"\x1b[?996n", &mut fx);
        em.set_theme(Some(&Theme::new([0x14; 3], [0xfa; 3])))
            .unwrap();
        em.feed(b"\x1b[?996n", &mut fx);
        assert_eq!(replies(fx), ["\x1b[?997;1n", "\x1b[?997;2n"]);
    }
}

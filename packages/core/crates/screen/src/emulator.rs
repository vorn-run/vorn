//! A session's terminal: libghostty-vt fed one write per flush, its labels and counters, and its effects.

use std::cell::{Cell, RefCell};
use std::rc::Rc;

use libghostty_vt::fmt::Format;
use libghostty_vt::selection::{FormatOptions, Selection};
use libghostty_vt::style::RgbColor;
use libghostty_vt::terminal::{
    ColorScheme, ConformanceLevel, DeviceAttributeFeature, DeviceAttributes, DeviceType, Mode,
    Point, PointCoordinate, PrimaryDeviceAttributes, SecondaryDeviceAttributes, SizeReportSize,
    Terminal, TertiaryDeviceAttributes,
};

use crate::scan::{Osc, Scan};
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
    /// A shell's OSC 133 prompt or command mark.
    Command(Mark),
}

/// Where a shell is in running a command, as its OSC 133 marks say.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Mark {
    /// A new prompt (A).
    Prompt,
    /// A command began running (C), with its text when the shell told it.
    Start { command: Option<String> },
    /// The command finished (D), with its exit code and the duration the shell measured.
    End {
        exit_code: Option<i32>,
        duration_ms: Option<u64>,
    },
}

/// What OSC 5522 said about the next command mark.
#[derive(Debug, Default)]
pub(crate) struct Pending {
    command: Option<String>,
    duration_ms: Option<u64>,
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

/// What [`Emulator::parsed_bytes`] and [`Emulator::history_clears`] stand at: kept beside a checkpoint by a host
/// that drops a terminal and rebuilds it later.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct Counters {
    parsed: u64,
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
    /// The command text and duration OSC 5522 gave for the next mark.
    pending: Pending,
    /// Which of Ghostty's title and pwd the last write changed.
    changed: Rc<Cell<u8>>,
    /// Output bytes handed to Ghostty by [`Emulator::feed`].
    pub(crate) parsed: u64,
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
            pending: Pending::default(),
            changed,
            parsed: 0,
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

    /// The cursor's row and column on the screen, and whether it shows.
    pub fn cursor(&self) -> (u16, u16, bool) {
        (
            self.term.cursor_y().unwrap_or(0),
            self.term.cursor_x().unwrap_or(0),
            self.term.is_cursor_visible().unwrap_or(false),
        )
    }

    /// Screen rows `from..=to` as plain text, one string per row with its
    /// trailing blanks dropped, into `out`, which is cleared first.
    pub fn rows_text(&self, from: u16, to: u16, out: &mut Vec<String>) {
        out.clear();
        let to = to.min(self.rows.saturating_sub(1));
        if from > to {
            return;
        }
        let at = |x: u16, y: u16| {
            self.term
                .grid_ref(Point::Active(PointCoordinate { x, y: u32::from(y) }))
                .ok()
        };
        let text = at(0, from)
            .zip(at(self.cols.saturating_sub(1), to))
            .and_then(|(a, b)| {
                let sel = Selection::new(a, b, false);
                let opts = FormatOptions::new()
                    .with_emit_format(Format::Plain)
                    .with_unwrap(false)
                    .with_trim(true)
                    .with_selection(&sel);
                self.term.format_selection_alloc(None, opts).ok().flatten()
            });
        if let Some(text) = text {
            out.extend(
                String::from_utf8_lossy(&text)
                    .split('\n')
                    .map(|r| r.trim_end().to_owned()),
            );
        }
        out.resize(usize::from(to - from) + 1, String::new());
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
        let (cwd, effects, pending) = (&mut self.cwd, &self.effects, &mut self.pending);
        self.scan.feed(bytes, |kind, payload| match kind {
            Osc::Private => {
                if let Some(moved) = private_cwd(cwd, payload) {
                    effects.borrow_mut().push(Effect::Cwd(moved));
                } else {
                    pending.read(payload);
                }
            }
            Osc::Mark => {
                if let Some(mark) = pending.mark(payload) {
                    effects.borrow_mut().push(Effect::Command(mark));
                }
            }
        });
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
            clears: self.scan.clears,
        }
    }

    /// Takes up `c`, as a terminal rebuilt in place of the one they were read from.
    pub fn set_counters(&mut self, c: Counters) {
        self.parsed = c.parsed;
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

impl Pending {
    /// Takes an OSC 5522 `cmd;<base64>` or `dur;<ms>` report.
    fn read(&mut self, payload: &[u8]) {
        if let Some(b64) = payload.strip_prefix(b"cmd;") {
            self.command = base64_decode(b64)
                .map(|raw| clip_units(String::from_utf8_lossy(&raw).trim()))
                .filter(|c| !c.is_empty());
        } else if let Some(ms) = payload.strip_prefix(b"dur;") {
            self.duration_ms = std::str::from_utf8(ms).ok().and_then(|m| m.parse().ok());
        }
    }

    /// An OSC 133 payload as a mark; the command text and duration go to the mark that uses them.
    fn mark(&mut self, payload: &[u8]) -> Option<Mark> {
        let text = String::from_utf8_lossy(payload);
        let mut parts = text.split(';');
        match parts.next()? {
            "A" => Some(Mark::Prompt),
            "C" => {
                let given = parts
                    .find_map(|p| p.strip_prefix("cmdline_url="))
                    .map(|c| percent_decode(c).unwrap_or_else(|| c.to_owned()));
                let command = given.or(self.command.take());
                self.command = None;
                Some(Mark::Start {
                    command: command.map(|c| clip_units(c.trim())).filter(|c| !c.is_empty()),
                })
            }
            "D" => Some(Mark::End {
                exit_code: parts.next().and_then(|c| c.trim().parse().ok()),
                duration_ms: self.duration_ms.take(),
            }),
            _ => None,
        }
    }
}

/// Standard or URL-safe base64, padded or not; `None` when it is not base64.
fn base64_decode(s: &[u8]) -> Option<Vec<u8>> {
    let value = |c: u8| match c {
        b'A'..=b'Z' => Some(c - b'A'),
        b'a'..=b'z' => Some(c - b'a' + 26),
        b'0'..=b'9' => Some(c - b'0' + 52),
        b'+' | b'-' => Some(62),
        b'/' | b'_' => Some(63),
        _ => None,
    };
    let mut out = Vec::with_capacity(s.len() * 3 / 4);
    let (mut acc, mut bits) = (0u32, 0u32);
    for &c in s.iter().take_while(|&&c| c != b'=') {
        acc = ((acc << 6) | u32::from(value(c)?)) & 0xffff;
        bits += 6;
        if bits >= 8 {
            bits -= 8;
            out.push((acc >> bits) as u8);
        }
    }
    Some(out)
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rows_text_keeps_one_string_per_screen_row() {
        let mut em = Emulator::with_scrollback(12, 6, 1 << 16).unwrap();
        let mut fx = Vec::new();
        em.feed(b"old\r\n\r\n\r\n\r\n\r\n\r\n", &mut fx);
        em.feed(
            b"\x1b[2J\x1b[Htop  \x1b[3;3Hmid\x1b[6;1H> abcdefghi\x1b[?25l",
            &mut fx,
        );
        let mut rows = Vec::new();
        em.rows_text(0, 5, &mut rows);
        assert_eq!(rows, ["top", "", "  mid", "", "", "> abcdefghi"]);
        em.rows_text(2, 99, &mut rows);
        assert_eq!(rows, ["  mid", "", "", "> abcdefghi"]);
        em.rows_text(4, 3, &mut rows);
        assert!(rows.is_empty());
        assert_eq!(em.cursor(), (5, 11, false));
    }

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
    fn shell_marks_become_command_effects() {
        let mut em = Emulator::new(20, 3).unwrap();
        let mut fx = Vec::new();
        // "git status" in base64, as the shell integration reports it before C.
        em.feed(
            b"\x1b]133;A\x07$ \x1b]5522;cmd;Z2l0IHN0YXR1cw==\x07\x1b]133;C\x07out\r\n\x1b]5522;dur;1500\x07\x1b]133;D;2\x07",
            &mut fx,
        );
        em.feed(
            b"\x1b]133;C;cmdline_url=echo%20hi\x07\x1b]133;D\x07\x1b]133;C\x07",
            &mut fx,
        );
        let marks: Vec<Mark> = fx
            .into_iter()
            .filter_map(|e| match e {
                Effect::Command(m) => Some(m),
                _ => None,
            })
            .collect();
        assert_eq!(
            marks,
            [
                Mark::Prompt,
                Mark::Start {
                    command: Some("git status".into())
                },
                Mark::End {
                    exit_code: Some(2),
                    duration_ms: Some(1500)
                },
                Mark::Start {
                    command: Some("echo hi".into())
                },
                Mark::End {
                    exit_code: None,
                    duration_ms: None
                },
                Mark::Start { command: None },
            ]
        );
        assert_eq!(base64_decode(b"aGk"), Some(b"hi".to_vec()));
        assert_eq!(base64_decode(b"a*"), None);
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

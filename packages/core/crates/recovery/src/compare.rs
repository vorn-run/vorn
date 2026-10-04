//! The comparator: whether two terminals are in the same state, by the
//! recovery contract's equivalence, and which parts differ when they are not.
//!
//! Two terminals are equal when all of these match, one [`Check`] each: the
//! dimensions; the formatter's VT output of the screen and its retained
//! scrollback; the cursor (position, pending wrap, visibility, SGR pen and
//! shape); every DEC private and ANSI mode Ghostty knows; the kitty keyboard
//! flag stacks; which screen is active; the other screen and the saved
//! cursors; the scrolling region; the title; the working directory.
//!
//! [`TermState::capture`] reads all of it into plain data that compares with
//! `==` and travels between processes (serde). Some of it Ghostty only gives
//! up by being driven (the kitty stack is read by popping it, the inactive
//! screen by switching to it, a saved cursor by restoring it), so capture
//! consumes the [`Screen`] it reads.
//!
//! Each difference is meant to show under one check only: the screen content
//! is formatted without modes, cursor or region, so a missed mode is a
//! Modes difference and not also a Screen one.

use std::collections::BTreeMap;
use std::fmt;

use libghostty_vt::fmt::{Format, Formatter, FormatterOptions};
use libghostty_vt::render::RenderState;
use libghostty_vt::screen::Screen as Which;
use libghostty_vt::terminal::{Mode, ModeKind};
use serde::{Deserialize, Serialize};
use vorn_screen::Screen;

use crate::log::Size;
use crate::Error;

/// One part of the equivalence.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
pub enum Check {
    Dimensions,
    /// The active screen's content and retained scrollback, as VT.
    Screen,
    Cursor,
    Modes,
    KittyKeyboard,
    ActiveScreen,
    /// The inactive screen's content and both screens' saved cursors.
    SavedScreen,
    ScrollingRegion,
    Title,
    Cwd,
}

impl Check {
    pub const ALL: [Check; 10] = [
        Check::Dimensions,
        Check::Screen,
        Check::Cursor,
        Check::Modes,
        Check::KittyKeyboard,
        Check::ActiveScreen,
        Check::SavedScreen,
        Check::ScrollingRegion,
        Check::Title,
        Check::Cwd,
    ];
}

/// A screen's content: the formatter's VT for every row it keeps, history
/// first, with no modes, cursor or region in it. The formatter drops trailing
/// blank rows, so the history length goes beside it to pin where the rows sit.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Content {
    pub vt: String,
    pub scrollback_rows: usize,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CursorState {
    pub x: u16,
    pub y: u16,
    pub pending_wrap: bool,
    pub visible: bool,
    /// The formatter's VT for what the next printed cell takes from the
    /// cursor: SGR, hyperlink, protection and character sets.
    pub pen: String,
    /// DECSCUSR's shape. Blinking is a mode (12) and compares there.
    pub shape: String,
}

/// What DECRC brings back on one screen.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SavedCursor {
    pub x: u16,
    pub y: u16,
    pub pending_wrap: bool,
    pub pen: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Saved {
    /// The screen that is not active.
    pub inactive: Content,
    /// The active screen's saved cursor.
    pub active_cursor: SavedCursor,
    /// The inactive screen's saved cursor.
    pub inactive_cursor: SavedCursor,
}

/// Ghostty keeps one ring of eight flag sets per screen; each is listed from
/// the top of the stack down, all eight, since a push of zero is a level too.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct KittyStacks {
    pub active: Vec<u8>,
    pub inactive: Vec<u8>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum ActiveScreen {
    Primary,
    Alternate,
}

/// A terminal's state, one field per [`Check`].
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct TermState {
    pub size: Size,
    pub screen: Content,
    pub cursor: CursorState,
    /// Every mode Ghostty's API names, by name (`?2004`, `4`), set or not.
    pub modes: BTreeMap<String, bool>,
    pub kitty: KittyStacks,
    pub active_screen: ActiveScreen,
    pub saved: Saved,
    /// DECSTBM and DECSLRM as the formatter writes them; empty when the
    /// region is the whole screen.
    pub scrolling_region: String,
    pub title: String,
    pub cwd: String,
}

/// Every mode constant libghostty-vt's `Mode` defines, ANSI and DEC.
const MODES: [Mode; 40] = [
    Mode::KAM,
    Mode::INSERT,
    Mode::SRM,
    Mode::LINEFEED,
    Mode::DECCKM,
    Mode::_132_COLUMN,
    Mode::SLOW_SCROLL,
    Mode::REVERSE_COLORS,
    Mode::ORIGIN,
    Mode::WRAPAROUND,
    Mode::AUTOREPEAT,
    Mode::X10_MOUSE,
    Mode::CURSOR_BLINKING,
    Mode::CURSOR_VISIBLE,
    Mode::ENABLE_MODE3,
    Mode::REVERSE_WRAP,
    Mode::ALT_SCREEN_LEGACY,
    Mode::KEYPAD_KEYS,
    Mode::LEFT_RIGHT_MARGIN,
    Mode::NORMAL_MOUSE,
    Mode::BUTTON_MOUSE,
    Mode::ANY_MOUSE,
    Mode::FOCUS_EVENT,
    Mode::UTF8_MOUSE,
    Mode::SGR_MOUSE,
    Mode::ALT_SCROLL,
    Mode::URXVT_MOUSE,
    Mode::SGR_PIXELS_MOUSE,
    Mode::NUMLOCK_KEYPAD,
    Mode::ALT_ESC_PREFIX,
    Mode::ALT_SENDS_ESC,
    Mode::REVERSE_WRAP_EXT,
    Mode::ALT_SCREEN,
    Mode::SAVE_CURSOR,
    Mode::ALT_SCREEN_SAVE,
    Mode::BRACKETED_PASTE,
    Mode::SYNC_OUTPUT,
    Mode::GRAPHEME_CLUSTER,
    Mode::COLOR_SCHEME_REPORT,
    Mode::IN_BAND_RESIZE,
];

/// The kitty flag ring's depth in Ghostty (`kitty/key.zig`, `FlagStack`).
const KITTY_DEPTH: usize = 8;

fn mode_name(m: Mode) -> String {
    match m.kind() {
        ModeKind::Ansi => m.value().to_string(),
        _ => format!("?{}", m.value()),
    }
}

/// The formatter's VT output with only the given extras.
fn format(screen: &Screen, opts: FormatterOptions<'_, '_>) -> Result<String, Error> {
    let mut f = Formatter::new(screen.terminal(), opts.with_format(Format::Vt))?;
    let bytes = f.format_alloc(None)?;
    Ok(String::from_utf8_lossy(&bytes).into_owned())
}

/// What `opts` adds after the bare content: the extras' own sequences.
fn extras(screen: &Screen, content: &str, opts: FormatterOptions<'_, '_>) -> Result<String, Error> {
    let all = format(screen, opts)?;
    Ok(match all.strip_prefix(content) {
        Some(rest) => rest.to_owned(),
        // Extras that change the content itself would land here; keep the
        // whole output so the difference still shows.
        None => all,
    })
}

fn content(screen: &Screen) -> Result<Content, Error> {
    Ok(Content {
        vt: format(screen, FormatterOptions::new())?,
        scrollback_rows: screen.terminal().scrollback_rows()?,
    })
}

fn pen(screen: &Screen, content: &str) -> Result<String, Error> {
    extras(
        screen,
        content,
        FormatterOptions::new()
            .with_style(true)
            .with_hyperlink(true)
            .with_protection(true)
            .with_charsets(true),
    )
}

/// Reads the whole ring by popping it: the top, then each level below.
fn kitty_stack(screen: &mut Screen) -> Result<Vec<u8>, Error> {
    let mut stack = Vec::with_capacity(KITTY_DEPTH);
    for _ in 0..KITTY_DEPTH {
        stack.push(screen.terminal().kitty_keyboard_flags()?.bits());
        screen.feed(b"\x1b[<u");
    }
    Ok(stack)
}

/// DECRC, then reads what it restored.
fn saved_cursor(screen: &mut Screen) -> Result<SavedCursor, Error> {
    screen.feed(b"\x1b8");
    let t = screen.terminal();
    let (x, y, pending_wrap) = (t.cursor_x()?, t.cursor_y()?, t.is_cursor_pending_wrap()?);
    let content = format(screen, FormatterOptions::new())?;
    Ok(SavedCursor {
        x,
        y,
        pending_wrap,
        pen: pen(screen, &content)?,
    })
}

impl TermState {
    /// Reads every check's state out of `screen`. Takes the screen by value:
    /// reading the kitty stacks, the inactive screen and the saved cursors
    /// drives the terminal (pops, a screen switch, DECRC), and what is left
    /// afterwards is not the state that was captured.
    pub fn capture(mut screen: Screen) -> Result<TermState, Error> {
        let t = screen.terminal();
        let size = Size::new(t.cols()?, t.rows()?);
        let mut modes = BTreeMap::new();
        for m in MODES {
            modes.insert(mode_name(m), t.mode(m)?);
        }
        let active_screen = match t.active_screen()? {
            Which::Primary => ActiveScreen::Primary,
            Which::Alternate => ActiveScreen::Alternate,
        };
        let shape = {
            let mut rs = RenderState::new()?;
            let snap = rs.update(t)?;
            format!("{:?}", snap.cursor_visual_style()?)
        };
        let screen_content = content(&screen)?;
        let t = screen.terminal();
        let cursor = CursorState {
            x: t.cursor_x()?,
            y: t.cursor_y()?,
            pending_wrap: t.is_cursor_pending_wrap()?,
            visible: t.is_cursor_visible()?,
            pen: pen(&screen, &screen_content.vt)?,
            shape,
        };
        let scrolling_region = extras(
            &screen,
            &screen_content.vt,
            FormatterOptions::new().with_scrolling_region(true),
        )?;
        let title = screen.title().to_owned();
        let cwd = screen.cwd().to_owned();

        // From here on the probes change the terminal.
        let active_kitty = kitty_stack(&mut screen)?;
        let active_cursor = saved_cursor(&mut screen)?;
        // Mode 47 switches screens without clearing either; it copies the
        // cursor across, which nothing reads after this.
        screen.feed(match active_screen {
            ActiveScreen::Primary => b"\x1b[?47h",
            ActiveScreen::Alternate => b"\x1b[?47l",
        });
        let inactive = content(&screen)?;
        let inactive_kitty = kitty_stack(&mut screen)?;
        let inactive_cursor = saved_cursor(&mut screen)?;

        Ok(TermState {
            size,
            screen: screen_content,
            cursor,
            modes,
            kitty: KittyStacks {
                active: active_kitty,
                inactive: inactive_kitty,
            },
            active_screen,
            saved: Saved {
                inactive,
                active_cursor,
                inactive_cursor,
            },
            scrolling_region,
            title,
            cwd,
        })
    }
}

/// One check that failed, with both sides rendered for a reader.
#[derive(Clone, PartialEq, Eq)]
pub struct Diff {
    pub check: Check,
    pub detail: String,
}

/// Every check that failed. Its `Debug` is its `Display`, so a test's
/// `unwrap()` prints a readable diff.
#[derive(Clone, PartialEq, Eq)]
pub struct Mismatch {
    pub diffs: Vec<Diff>,
}

impl Mismatch {
    /// The failing checks, in [`Check`] order.
    pub fn checks(&self) -> Vec<Check> {
        self.diffs.iter().map(|d| d.check).collect()
    }
}

impl fmt::Display for Mismatch {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        writeln!(f, "terminals differ in {:?}", self.checks())?;
        for d in &self.diffs {
            writeln!(f, "- {:?}: {}", d.check, d.detail)?;
        }
        Ok(())
    }
}

impl fmt::Debug for Mismatch {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        fmt::Display::fmt(self, f)
    }
}

impl std::error::Error for Mismatch {}

/// Compares two states by every check; `left` is usually the terminal that
/// never died, `right` the recovered one.
pub fn compare(left: &TermState, right: &TermState) -> Result<(), Mismatch> {
    let mut diffs = Vec::new();
    let mut push = |check, detail: Option<String>| {
        if let Some(detail) = detail {
            diffs.push(Diff { check, detail });
        }
    };
    push(Check::Dimensions, plain(&left.size, &right.size));
    push(Check::Screen, content_diff(&left.screen, &right.screen));
    push(Check::Cursor, plain(&left.cursor, &right.cursor));
    push(Check::Modes, modes_diff(&left.modes, &right.modes));
    push(Check::KittyKeyboard, plain(&left.kitty, &right.kitty));
    push(
        Check::ActiveScreen,
        plain(&left.active_screen, &right.active_screen),
    );
    push(Check::SavedScreen, saved_diff(&left.saved, &right.saved));
    push(
        Check::ScrollingRegion,
        plain(&left.scrolling_region, &right.scrolling_region),
    );
    push(Check::Title, plain(&left.title, &right.title));
    push(Check::Cwd, plain(&left.cwd, &right.cwd));
    if diffs.is_empty() {
        Ok(())
    } else {
        Err(Mismatch { diffs })
    }
}

fn saved_diff(l: &Saved, r: &Saved) -> Option<String> {
    let parts: Vec<String> = [
        content_diff(&l.inactive, &r.inactive).map(|d| format!("inactive screen: {d}")),
        plain(&l.active_cursor, &r.active_cursor).map(|d| format!("saved cursor: {d}")),
        plain(&l.inactive_cursor, &r.inactive_cursor)
            .map(|d| format!("inactive screen's saved cursor: {d}")),
    ]
    .into_iter()
    .flatten()
    .collect();
    (!parts.is_empty()).then(|| parts.join("; "))
}

fn plain<T: PartialEq + fmt::Debug>(l: &T, r: &T) -> Option<String> {
    (l != r).then(|| format!("{l:?} != {r:?}"))
}

fn modes_diff(l: &BTreeMap<String, bool>, r: &BTreeMap<String, bool>) -> Option<String> {
    let differ: Vec<String> = l
        .iter()
        .filter(|(k, v)| r.get(*k) != Some(*v))
        .map(|(k, v)| format!("{k}: {v} != {:?}", r.get(k)))
        .collect();
    (!differ.is_empty()).then(|| differ.join(", "))
}

fn content_diff(l: &Content, r: &Content) -> Option<String> {
    if l == r {
        return None;
    }
    let mut out = String::new();
    if l.scrollback_rows != r.scrollback_rows {
        out.push_str(&format!(
            "scrollback rows {} != {}; ",
            l.scrollback_rows, r.scrollback_rows
        ));
    }
    if l.vt != r.vt {
        out.push_str(&text_diff(&l.vt, &r.vt));
    }
    Some(out)
}

/// Where two long strings first differ, with some context on each side.
pub fn text_diff(l: &str, r: &str) -> String {
    const CONTEXT: usize = 40;
    let at = l
        .char_indices()
        .zip(r.chars())
        .find(|((_, a), b)| a != b)
        .map(|((i, _), _)| i)
        .unwrap_or_else(|| l.len().min(r.len()));
    let from = floor_char(l, at.saturating_sub(CONTEXT));
    let show = |s: &str| {
        let start = floor_char(s, from);
        let end = floor_char(s, (at + CONTEXT).min(s.len()));
        format!("{:?}", &s[start..end.max(start)])
    };
    format!(
        "first difference at byte {at} (lengths {} and {}): {} != {}",
        l.len(),
        r.len(),
        show(l),
        show(r)
    )
}

fn floor_char(s: &str, mut i: usize) -> usize {
    i = i.min(s.len());
    while !s.is_char_boundary(i) {
        i -= 1;
    }
    i
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn text_diff_points_at_the_first_difference() {
        let d = text_diff("hello world", "hello there");
        assert!(d.contains("byte 6"), "{d}");
        let d = text_diff("日本語", "日本");
        assert!(d.contains("byte 6"), "{d}");
    }

    #[test]
    fn a_fresh_terminal_has_an_empty_kitty_ring_and_no_region() {
        let s = TermState::capture(Screen::new(10, 4).unwrap()).unwrap();
        assert_eq!(s.kitty.active, vec![0; KITTY_DEPTH]);
        assert_eq!(s.scrolling_region, "");
        assert_eq!(s.active_screen, ActiveScreen::Primary);
        assert_eq!(s.modes.len(), MODES.len());
        assert!(s.modes["?7"], "autowrap is on by default");
    }

    #[test]
    fn capture_reads_the_whole_kitty_ring_and_both_screens() {
        let mut s = Screen::new(10, 4).unwrap();
        s.feed(b"\x1b[>1u\x1b[>0u\x1b[>5u\x1b[?1049hALT\x1b[>3u\x1b[2;3r");
        let st = TermState::capture(s).unwrap();
        assert_eq!(st.active_screen, ActiveScreen::Alternate);
        assert_eq!(st.kitty.active[..2], [3, 0]);
        assert_eq!(st.kitty.inactive[..4], [5, 0, 1, 0]);
        assert_eq!(st.scrolling_region, "\x1b[2;3r");
        assert!(st.screen.vt.contains("ALT"));
        // 1049 saved the primary's cursor; the probe restored it there.
        assert_eq!(
            (st.saved.inactive_cursor.x, st.saved.inactive_cursor.y),
            (0, 0)
        );
    }
}

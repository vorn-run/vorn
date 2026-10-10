//! The text a field edits, apart from how it is drawn: the string, the
//! caret and selection (byte offsets on grapheme boundaries), the IME
//! composition, and what each key does to them. It knows nothing about
//! pixels; moving up or down a line is answered by the [`Ui`](crate::Ui),
//! which has the shaped text.

use std::ops::Range;

use unicode_segmentation::UnicodeSegmentation;

use crate::input::{mods, Input};

/// Where cut, copy and paste go. Windows use the system clipboard; tests
/// and headless runs keep it in memory.
pub trait Clipboard {
    fn get(&mut self) -> Option<String>;
    fn set(&mut self, text: &str);
}

/// A clipboard that lives as long as the [`Ui`](crate::Ui).
#[derive(Debug, Default)]
pub struct MemoryClipboard(pub Option<String>);

impl Clipboard for MemoryClipboard {
    fn get(&mut self) -> Option<String> {
        self.0.clone()
    }
    fn set(&mut self, text: &str) {
        self.0 = Some(text.to_owned());
    }
}

/// What a key did to a field.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Outcome {
    /// Not an editing key: the app or the focus order may use it.
    Unhandled,
    /// The caret or selection moved, or the text was copied.
    Moved,
    Changed,
    /// Enter in a single-line field, or without Shift in a composer.
    Submit,
    /// Up (-1) or down (1) a visual line; the caller knows where that is.
    Vertical {
        dir: i32,
        extend: bool,
    },
}

#[derive(Debug, Clone, Default, PartialEq)]
pub struct TextEdit {
    text: String,
    caret: usize,
    anchor: usize,
    preedit: String,
    multiline: bool,
    /// Where an up/down run started, physical pixels, so the caret keeps
    /// its column across short lines.
    pub(crate) goal_x: Option<f32>,
    /// How far a multiline field is scrolled, logical pixels.
    pub(crate) scroll_y: f32,
}

impl TextEdit {
    pub fn new(text: &str, multiline: bool) -> TextEdit {
        let mut e = TextEdit {
            multiline,
            ..TextEdit::default()
        };
        e.set_text(text);
        e
    }

    pub fn text(&self) -> &str {
        &self.text
    }

    pub fn is_multiline(&self) -> bool {
        self.multiline
    }

    /// Follows the field's look, for edits made before their field was
    /// first painted.
    pub(crate) fn set_multiline(&mut self, multiline: bool) {
        if self.multiline != multiline {
            self.multiline = multiline;
            let t = std::mem::take(&mut self.text);
            self.text = self.clean(&t);
            self.caret = self.caret.min(self.text.len());
            self.anchor = self.anchor.min(self.text.len());
        }
    }

    /// Replaces the text and puts the caret at its end.
    pub fn set_text(&mut self, text: &str) {
        self.text = self.clean(text);
        self.caret = self.text.len();
        self.anchor = self.caret;
        self.preedit.clear();
        self.goal_x = None;
    }

    pub fn caret(&self) -> usize {
        self.caret
    }

    pub fn anchor(&self) -> usize {
        self.anchor
    }

    /// The selected bytes, in order (empty when there is only a caret).
    pub fn selection(&self) -> Range<usize> {
        self.caret.min(self.anchor)..self.caret.max(self.anchor)
    }

    pub fn selected_text(&self) -> &str {
        &self.text[self.selection()]
    }

    /// The IME composition in progress, drawn at the caret.
    pub fn preedit(&self) -> &str {
        &self.preedit
    }

    /// The text as drawn, with the composition spliced in at the caret, and
    /// where the composition sits in it.
    pub fn display(&self) -> (String, Range<usize>) {
        if self.preedit.is_empty() {
            return (self.text.clone(), self.caret..self.caret);
        }
        let mut s = String::with_capacity(self.text.len() + self.preedit.len());
        s.push_str(&self.text[..self.caret]);
        s.push_str(&self.preedit);
        s.push_str(&self.text[self.caret..]);
        (s, self.caret..self.caret + self.preedit.len())
    }

    /// Selects `anchor..caret`, snapped back to grapheme boundaries.
    pub fn set_selection(&mut self, anchor: usize, caret: usize) {
        self.anchor = self.snap(anchor);
        self.caret = self.snap(caret);
        self.goal_x = None;
    }

    /// Moves the caret to `i`, keeping the anchor when `extend`.
    pub fn move_to(&mut self, i: usize, extend: bool) {
        self.caret = self.snap(i);
        if !extend {
            self.anchor = self.caret;
        }
    }

    pub fn select_all(&mut self) {
        self.anchor = 0;
        self.caret = self.text.len();
    }

    /// Selects the word around byte `i` (a double click).
    pub fn select_word(&mut self, i: usize) {
        let i = self.snap(i);
        for (start, w) in self.text.split_word_bound_indices() {
            let end = start + w.len();
            if i >= start && (i < end || end == self.text.len()) {
                self.anchor = start;
                self.caret = end;
                return;
            }
        }
    }

    /// Types `s` over the selection.
    pub fn insert(&mut self, s: &str) {
        let s = self.clean(s);
        let r = self.selection();
        self.text.replace_range(r.clone(), &s);
        self.caret = r.start + s.len();
        self.anchor = self.caret;
        self.goal_x = None;
    }

    /// What a key or IME event does to the field. `mac` picks the
    /// platform's shortcuts: Cmd for clipboard and line moves, Option for
    /// words; elsewhere Ctrl for both, Home and End for lines.
    pub fn key(&mut self, input: &Input, mac: bool, clip: &mut dyn Clipboard) -> Outcome {
        let (code, m, typed) = match input {
            Input::Preedit(s) => {
                if !s.is_empty() && self.caret != self.anchor {
                    self.insert("");
                }
                self.preedit.clone_from(s);
                return Outcome::Changed;
            }
            Input::Commit(s) => {
                self.preedit.clear();
                self.insert(s);
                return Outcome::Changed;
            }
            Input::Key { code, mods, text } => (code.as_str(), *mods, text.as_deref()),
        };
        // Keys go to the IME while it composes; it reports the result.
        if !self.preedit.is_empty() {
            return Outcome::Moved;
        }
        let shift = m & mods::SHIFT != 0;
        // Windows reports AltGr as Ctrl+Alt; it types, it is not a chord.
        let altgr = !mac && m & mods::CTRL != 0 && m & mods::ALT != 0;
        let primary = !altgr && m & if mac { mods::SUPER } else { mods::CTRL } != 0;
        let word = !altgr && m & if mac { mods::ALT } else { mods::CTRL } != 0;
        let line = mac && m & mods::SUPER != 0;
        if primary && !shift {
            match code {
                "KeyA" => {
                    self.select_all();
                    return Outcome::Moved;
                }
                "KeyC" | "KeyX" => {
                    if self.caret == self.anchor {
                        return Outcome::Moved;
                    }
                    clip.set(self.selected_text());
                    if code == "KeyC" {
                        return Outcome::Moved;
                    }
                    self.insert("");
                    return Outcome::Changed;
                }
                "KeyV" => {
                    let Some(s) = clip.get() else {
                        return Outcome::Moved;
                    };
                    self.insert(&s);
                    return Outcome::Changed;
                }
                _ => {}
            }
        }
        if code != "ArrowUp" && code != "ArrowDown" {
            self.goal_x = None;
        }
        match code {
            "ArrowLeft" | "ArrowRight" => {
                let fwd = code == "ArrowRight";
                let to = if line {
                    if fwd {
                        self.line_end(self.caret)
                    } else {
                        self.line_start(self.caret)
                    }
                } else if word {
                    self.word_from(self.caret, fwd)
                } else if !shift && self.caret != self.anchor {
                    // Collapsing a selection lands on its edge.
                    let r = self.selection();
                    if fwd {
                        r.end
                    } else {
                        r.start
                    }
                } else {
                    self.grapheme_from(self.caret, fwd)
                };
                self.move_to(to, shift);
                Outcome::Moved
            }
            "ArrowUp" | "ArrowDown" => {
                let dir = if code == "ArrowUp" { -1 } else { 1 };
                if line || !self.multiline {
                    if !self.multiline && !mac {
                        return Outcome::Unhandled;
                    }
                    self.move_to(if dir < 0 { 0 } else { self.text.len() }, shift);
                    return Outcome::Moved;
                }
                Outcome::Vertical { dir, extend: shift }
            }
            "Home" | "End" => {
                let to = match (code == "End", primary) {
                    (false, true) => 0,
                    (true, true) => self.text.len(),
                    (false, false) => self.line_start(self.caret),
                    (true, false) => self.line_end(self.caret),
                };
                self.move_to(to, shift);
                Outcome::Moved
            }
            "Backspace" | "Delete" => {
                if self.caret == self.anchor {
                    let fwd = code == "Delete";
                    let to = match (line, word) {
                        (true, _) if fwd => self.line_end(self.caret),
                        (true, _) => self.line_start(self.caret),
                        (false, true) => self.word_from(self.caret, fwd),
                        (false, false) => self.grapheme_from(self.caret, fwd),
                    };
                    self.anchor = to;
                }
                if self.caret == self.anchor {
                    return Outcome::Moved;
                }
                self.insert("");
                Outcome::Changed
            }
            "Enter" | "NumpadEnter" => {
                if self.multiline && shift {
                    self.insert("\n");
                    Outcome::Changed
                } else {
                    Outcome::Submit
                }
            }
            _ => match typed {
                Some(t)
                    if !primary
                        && !(m & mods::CTRL != 0 && m & mods::ALT == 0)
                        && !t.is_empty()
                        && !t.chars().any(char::is_control) =>
                {
                    self.insert(t);
                    Outcome::Changed
                }
                _ => Outcome::Unhandled,
            },
        }
    }

    /// The paste of a single-line field loses its newlines, as a browser's
    /// `<input>` does.
    fn clean(&self, s: &str) -> String {
        if self.multiline {
            s.replace("\r\n", "\n")
        } else {
            s.replace(['\r', '\n'], "")
        }
    }

    fn snap(&self, i: usize) -> usize {
        let mut i = i.min(self.text.len());
        while !self.text.is_char_boundary(i) {
            i -= 1;
        }
        i
    }

    fn grapheme_from(&self, i: usize, fwd: bool) -> usize {
        if fwd {
            self.text[i..]
                .graphemes(true)
                .next()
                .map_or(i, |g| i + g.len())
        } else {
            self.text[..i]
                .grapheme_indices(true)
                .next_back()
                .map_or(0, |(s, _)| s)
        }
    }

    /// The next word end (forward) or previous word start (back), skipping
    /// spaces and punctuation between, like a browser's word move.
    fn word_from(&self, i: usize, fwd: bool) -> usize {
        let is_word = |w: &str| w.chars().any(char::is_alphanumeric);
        if fwd {
            self.text[i..]
                .split_word_bound_indices()
                .find(|(_, w)| is_word(w))
                .map_or(self.text.len(), |(s, w)| i + s + w.len())
        } else {
            self.text[..i]
                .split_word_bound_indices()
                .rev()
                .find(|(_, w)| is_word(w))
                .map_or(0, |(s, _)| s)
        }
    }

    fn line_start(&self, i: usize) -> usize {
        self.text[..i].rfind('\n').map_or(0, |n| n + 1)
    }

    fn line_end(&self, i: usize) -> usize {
        self.text[i..].find('\n').map_or(self.text.len(), |n| i + n)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn key(code: &str, m: u16) -> Input {
        Input::Key {
            code: code.into(),
            mods: m,
            text: None,
        }
    }

    fn typed(e: &mut TextEdit, s: &str) {
        let mut c = MemoryClipboard::default();
        for ch in s.chars() {
            e.key(&Input::char(ch), false, &mut c);
        }
    }

    #[test]
    fn typing_replaces_the_selection() {
        let mut e = TextEdit::new("hello world", false);
        e.set_selection(0, 5);
        typed(&mut e, "bye");
        assert_eq!(e.text(), "bye world");
        assert_eq!((e.caret(), e.anchor()), (3, 3));
    }

    #[test]
    fn backspace_and_arrows_step_over_whole_graphemes() {
        // e + combining acute, and a flag (two regional indicators).
        let mut e = TextEdit::new("ae\u{301}🇯🇵", false);
        let mut c = MemoryClipboard::default();
        e.key(&key("Backspace", 0), false, &mut c);
        assert_eq!(e.text(), "ae\u{301}");
        e.key(&key("ArrowLeft", 0), false, &mut c);
        assert_eq!(e.caret(), 1);
        e.key(&key("Delete", 0), false, &mut c);
        assert_eq!(e.text(), "a");
    }

    #[test]
    fn word_moves_follow_the_platform() {
        let mut c = MemoryClipboard::default();
        let mut e = TextEdit::new("git commit -m fix", false);
        e.key(&key("ArrowLeft", mods::ALT), true, &mut c);
        assert_eq!(e.caret(), 14, "Option+Left on mac");
        e.key(&key("ArrowLeft", mods::CTRL), false, &mut c);
        assert_eq!(e.caret(), 12, "Ctrl+Left elsewhere");
        e.key(&key("Backspace", mods::ALT), true, &mut c);
        assert_eq!(e.text(), "git m fix", "Option+Backspace skips the dash");
        e.key(&key("ArrowRight", mods::SUPER | mods::SHIFT), true, &mut c);
        assert_eq!(e.selected_text(), "m fix");
    }

    #[test]
    fn clipboard_shortcuts_cut_and_paste() {
        let mut c = MemoryClipboard::default();
        let mut e = TextEdit::new("abc", false);
        e.key(&key("KeyA", mods::CTRL), false, &mut c);
        assert_eq!(
            e.key(&key("KeyX", mods::CTRL), false, &mut c),
            Outcome::Changed
        );
        assert_eq!((e.text(), c.0.as_deref()), ("", Some("abc")));
        e.key(&key("KeyV", mods::SUPER), true, &mut c);
        e.key(&key("KeyV", mods::SUPER), true, &mut c);
        assert_eq!(e.text(), "abcabc");
        // Ctrl+V on a mac is not paste, and types nothing.
        assert_eq!(
            e.key(&key("KeyV", mods::CTRL), true, &mut c),
            Outcome::Unhandled
        );
    }

    #[test]
    fn enter_submits_and_shift_enter_breaks_in_a_composer() {
        let mut c = MemoryClipboard::default();
        let mut e = TextEdit::new("a", true);
        assert_eq!(e.key(&key("Enter", 0), false, &mut c), Outcome::Submit);
        assert_eq!(
            e.key(&key("Enter", mods::SHIFT), false, &mut c),
            Outcome::Changed
        );
        assert_eq!(e.text(), "a\n");
        assert_eq!(
            e.key(&key("ArrowUp", 0), false, &mut c),
            Outcome::Vertical {
                dir: -1,
                extend: false
            }
        );
        let mut one = TextEdit::new("x", false);
        assert_eq!(
            one.key(&key("Enter", mods::SHIFT), false, &mut c),
            Outcome::Submit
        );
        one.set_selection(0, 0);
        one.insert("a\nb");
        assert_eq!(one.text(), "abx", "single-line fields drop newlines");
    }

    #[test]
    fn composition_shows_at_the_caret_until_commit() {
        let mut c = MemoryClipboard::default();
        let mut e = TextEdit::new("$ ", false);
        e.key(&Input::Preedit("にほん".into()), false, &mut c);
        assert_eq!(e.display(), ("$ にほん".into(), 2..11));
        assert_eq!(e.text(), "$ ");
        // Keys during composition belong to the IME.
        e.key(&key("Backspace", 0), false, &mut c);
        assert_eq!(e.text(), "$ ");
        e.key(&Input::Commit("日本語".into()), false, &mut c);
        assert_eq!((e.text(), e.preedit()), ("$ 日本語", ""));
        assert_eq!(e.caret(), e.text().len());
    }

    #[test]
    fn home_end_and_lines_in_a_multiline_field() {
        let mut c = MemoryClipboard::default();
        let mut e = TextEdit::new("one\ntwo", true);
        e.key(&key("Home", 0), false, &mut c);
        assert_eq!(e.caret(), 4);
        e.key(&key("End", mods::SHIFT), false, &mut c);
        assert_eq!(e.selected_text(), "two");
        e.key(&key("Home", mods::CTRL), false, &mut c);
        assert_eq!(e.caret(), 0);
        e.select_word(5);
        assert_eq!(e.selected_text(), "two");
    }

    #[test]
    fn control_chords_type_nothing_but_altgr_does() {
        let mut c = MemoryClipboard::default();
        let mut e = TextEdit::new("", false);
        let ctrl_k = Input::Key {
            code: "KeyK".into(),
            mods: mods::CTRL,
            text: Some("k".into()),
        };
        assert_eq!(e.key(&ctrl_k, false, &mut c), Outcome::Unhandled);
        let altgr = Input::Key {
            code: "KeyQ".into(),
            mods: mods::CTRL | mods::ALT,
            text: Some("@".into()),
        };
        assert_eq!(e.key(&altgr, false, &mut c), Outcome::Changed);
        assert_eq!(e.text(), "@");
    }
}

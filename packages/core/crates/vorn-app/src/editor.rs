//! An open file's buffer (`FilePanel`): the text on disk and the text being
//! edited, whether it can be edited, the find bar, saving with its moved-file
//! guard, and drafts (`editor-drafts.ts`) that outlive the app.

use std::collections::{BTreeMap, HashMap};
use std::path::{Path, PathBuf};
use web_time::Instant;

use serde::{Deserialize, Serialize};
use serde_json::Value;

/// Past this many lines colour costs more than it gives (`MAX_HIGHLIGHT_LINES`).
pub const MAX_HIGHLIGHT_LINES: usize = 2000;
/// How long typing has to stop before the text is coloured again.
pub const HIGHLIGHT_SETTLE_MS: u128 = 150;
/// How long typing has to stop before the draft is written (`DRAFT_SETTLE_MS`).
pub const DRAFT_SETTLE_MS: u128 = 400;
/// Shared by the gutter and the text (`EDIT_LINE_HEIGHT`).
pub const LINE_HEIGHT: f32 = 21.0;

/// What the editor's controls do, by buffer key.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub enum EditAct {
    ToggleFind(String),
    FindStep(String, i32),
    CloseFind(String),
    Save(String),
    /// Saves over a file that moved on disk.
    SaveAnyway(String),
    Discard(String),
    /// Throws the edit away for what is on disk.
    TakeDisk(String),
    KeepEditing(String),
    /// The discard question's answers.
    ConfirmDiscard,
    CancelDiscard,
}

/// What a file was when read: its size and modification time (`FileStamp`).
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Stamp {
    pub size: f64,
    pub mtime_ms: f64,
}

impl Stamp {
    pub fn from_value(v: &Value) -> Option<Stamp> {
        Some(Stamp {
            size: v.get("size")?.as_f64()?,
            mtime_ms: v.get("mtimeMs")?.as_f64()?,
        })
    }
}

/// `hasMoved`: no base never armed the guard; an unknown current stamp
/// counts as moved.
pub fn has_moved(base: Option<Stamp>, current: Option<Stamp>) -> bool {
    match (base, current) {
        (None, _) => false,
        (Some(_), None) => true,
        (Some(b), Some(c)) => b != c,
    }
}

/// `isTruncatedRead`: a capped read, which must not be saved over the file.
pub fn is_truncated(text: &str) -> bool {
    let Some(rest) = text
        .rfind("\n\n--- truncated")
        .map(|i| &text[i + "\n\n--- truncated".len()..])
    else {
        return false;
    };
    if rest == " ---" {
        return true;
    }
    rest.strip_prefix(" (")
        .and_then(|r| r.strip_suffix(" bytes total) ---"))
        .is_some_and(|n| !n.is_empty() && n.bytes().all(|b| b.is_ascii_digit()))
}

/// Why a file is shown but cannot be typed in (`ReadOnlyReason`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ReadOnly {
    Binary,
    Unreadable,
    Truncated,
}

/// The find bar: its query and which match is current.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Find {
    pub query: String,
    pub index: usize,
}

/// One find hit, as byte offsets into the whole text, with its line.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Match {
    pub line: usize,
    pub range: (usize, usize),
}

/// `computeMatches`: every case-insensitive, non-overlapping hit, line by
/// line, in reading order.
pub fn matches(text: &str, query: &str) -> Vec<Match> {
    if query.is_empty() {
        return Vec::new();
    }
    let q = query.to_lowercase();
    let mut out = Vec::new();
    let mut at = 0;
    for (n, line) in text.split('\n').enumerate() {
        // Byte offsets need a lowercase of the same length; others fold ASCII only.
        let lower = line.to_lowercase();
        let (hay, needle) = if lower.len() == line.len() {
            (lower, q.clone())
        } else {
            (line.to_ascii_lowercase(), query.to_ascii_lowercase())
        };
        let mut from = 0;
        while let Some(i) = hay[from..].find(&needle) {
            let s = from + i;
            out.push(Match {
                line: n,
                range: (at + s, at + s + needle.len()),
            });
            from = s + needle.len().max(1);
            if from > hay.len() {
                break;
            }
        }
        at += line.len() + 1;
    }
    out
}

/// One open file.
#[derive(Debug, Clone)]
pub struct Buffer {
    pub path: String,
    /// What is on disk, once read.
    pub content: Option<String>,
    pub loading: bool,
    pub read_only: Option<ReadOnly>,
    /// The text being edited, once taken from the file or its draft.
    pub draft: String,
    pub ready: bool,
    /// What the file was when the edit started.
    pub base: Option<Stamp>,
    /// The file moved on disk under the edit.
    pub conflict: bool,
    pub saving: bool,
    pub error: Option<String>,
    pub find: Option<Find>,
    /// When the text last changed, for the colour and draft debounces.
    pub typed: Option<Instant>,
    /// The text last coloured, and its colours.
    pub coloured: Option<(String, crate::highlight::Spans)>,
    /// The draft last written, to write each change once.
    pub drafted: Option<String>,
}

impl Buffer {
    pub fn new(path: &str) -> Buffer {
        Buffer {
            path: path.to_owned(),
            content: None,
            loading: true,
            read_only: None,
            draft: String::new(),
            ready: false,
            base: None,
            conflict: false,
            saving: false,
            error: None,
            find: None,
            typed: None,
            coloured: None,
            drafted: None,
        }
    }

    /// Typing goes in.
    pub fn editable(&self) -> bool {
        self.read_only.is_none() && self.content.is_some() && !self.loading
    }

    /// `dirty`: edits not on disk.
    pub fn dirty(&self) -> bool {
        self.ready && self.editable() && Some(self.draft.as_str()) != self.content.as_deref()
    }

    /// The file read: editable unless binary, unreadable or capped.
    pub fn loaded(&mut self, content: Option<String>, stamp_known: bool) {
        self.loading = false;
        match content {
            Some(text) => {
                self.read_only = is_truncated(&text).then_some(ReadOnly::Truncated);
                self.content = Some(text);
            }
            None => {
                self.read_only = Some(if stamp_known {
                    ReadOnly::Binary
                } else {
                    ReadOnly::Unreadable
                });
            }
        }
    }

    /// Takes the file, or the draft kept over it, into the buffer. Returns
    /// whether the draft was used (its base must then be checked).
    pub fn take(&mut self, kept: Option<&Draft>) -> bool {
        let Some(content) = self.content.clone().filter(|_| self.editable()) else {
            return false;
        };
        self.ready = true;
        match kept.filter(|d| d.text != content) {
            Some(d) => {
                self.base = d.base;
                self.draft = d.text.clone();
                self.drafted = Some(d.text.clone());
                true
            }
            None => {
                self.draft = content;
                false
            }
        }
    }

    /// The text saved: it is what is on disk now.
    pub fn saved(&mut self, text: String) {
        if is_truncated(&text) {
            self.read_only = Some(ReadOnly::Truncated);
        }
        self.content = Some(text);
        self.conflict = false;
        self.base = None;
        self.saving = false;
        self.drafted = None;
    }

    /// The text to draw: the edit once ready, else what was read.
    pub fn text(&self) -> &str {
        if self.ready {
            &self.draft
        } else {
            self.content.as_deref().unwrap_or("")
        }
    }

    /// The find hits in what is shown.
    pub fn matches(&self) -> Vec<Match> {
        self.find
            .as_ref()
            .map_or_else(Vec::new, |f| matches(self.text(), &f.query))
    }
}

/// An edit never saved, kept so a quit does not throw it away (`EditorDraft`).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Draft {
    pub file_path: String,
    pub text: String,
    pub base: Option<Stamp>,
    #[serde(default)]
    pub saved_at: f64,
}

/// Drafts by pane, kept in a file beside the view; none keeps them in
/// memory only.
#[derive(Debug, Default)]
pub struct Drafts {
    file: Option<PathBuf>,
    all: BTreeMap<String, Draft>,
}

impl Drafts {
    pub fn load(file: Option<PathBuf>) -> Drafts {
        let all = file
            .as_deref()
            .and_then(crate::keep::read)
            .and_then(|s| serde_json::from_str(&s).ok())
            .unwrap_or_default();
        Drafts { file, all }
    }

    fn save(&self) {
        let Some(f) = &self.file else { return };
        // A full disk costs the draft's survival, not the edit on screen.
        if let Ok(s) = serde_json::to_string(&self.all) {
            let _ = crate::keep::write(f, &s);
        }
    }

    /// `readDraft`: the draft this pane held, if still about `path`.
    pub fn read(&self, key: &str, path: &str) -> Option<&Draft> {
        self.all.get(key).filter(|d| d.file_path == path)
    }

    pub fn write(&mut self, key: &str, path: &str, text: &str, base: Option<Stamp>) {
        self.all.insert(
            key.to_owned(),
            Draft {
                file_path: path.to_owned(),
                text: text.to_owned(),
                base,
                saved_at: crate::state::now_ms(),
            },
        );
        self.save();
    }

    pub fn forget(&mut self, key: &str) {
        if self.all.remove(key).is_some() {
            self.save();
        }
    }

    /// `pruneDrafts`: drafts for panes that are gone.
    pub fn prune(&mut self, live: impl Fn(&str) -> bool) {
        let n = self.all.len();
        self.all.retain(|k, _| live(k));
        if self.all.len() != n {
            self.save();
        }
    }
}

/// Where drafts are kept for `data_dir`'s vornd.
pub fn drafts_file(data_dir: &Path) -> PathBuf {
    data_dir.join("desktop").join("drafts.json")
}

/// Every open buffer, by tab or card key, and their drafts.
#[derive(Debug, Default)]
pub struct Editors {
    pub buffers: HashMap<String, Buffer>,
    pub drafts: Drafts,
    /// A discard waiting for its answer.
    pub confirm: Option<crate::act::Act>,
}

impl Editors {
    pub fn dirty(&self, key: &str) -> bool {
        self.buffers.get(key).is_some_and(Buffer::dirty)
    }
}

/// `relPath`: `path` under `cwd`, or whole.
pub fn rel_path<'a>(cwd: &str, path: &'a str) -> &'a str {
    if !cwd.is_empty() {
        if let Some(rest) = path.strip_prefix(cwd) {
            let rest = rest.trim_start_matches(['/', '\\']);
            if !rest.is_empty() {
                return rest;
            }
            return path.rsplit(['/', '\\']).next().unwrap_or(path);
        }
    }
    path
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn capped_reads_are_known_by_their_marker() {
        assert!(is_truncated("a\n\n--- truncated ---"));
        assert!(is_truncated("a\n\n--- truncated (12345 bytes total) ---"));
        assert!(!is_truncated("a\n\n--- truncated (x bytes total) ---"));
        assert!(!is_truncated("--- truncated ---"));
        assert!(!is_truncated("plain"));
    }

    #[test]
    fn find_is_case_blind_and_never_overlaps() {
        let m = matches("aAa\nxaa", "aa");
        assert_eq!(
            m,
            vec![
                Match {
                    line: 0,
                    range: (0, 2)
                },
                Match {
                    line: 1,
                    range: (5, 7)
                },
            ]
        );
        assert!(matches("abc", "").is_empty());
        let m = matches("İx x", "x");
        assert_eq!(m.len(), 2);
        assert!(m.iter().all(|h| "İx x".is_char_boundary(h.range.0)));
    }

    #[test]
    fn the_guard_arms_only_with_a_base() {
        let s = Stamp {
            size: 1.0,
            mtime_ms: 2.0,
        };
        assert!(!has_moved(None, None));
        assert!(has_moved(Some(s), None));
        assert!(!has_moved(Some(s), Some(s)));
        assert!(has_moved(Some(s), Some(Stamp { size: 2.0, ..s })));
    }

    #[test]
    fn a_buffer_is_dirty_only_when_its_edit_differs() {
        let mut b = Buffer::new("/f");
        b.loaded(Some("one".into()), true);
        assert!(!b.take(None));
        assert!(!b.dirty());
        b.draft.push('!');
        assert!(b.dirty());
        b.saved("one!".into());
        assert!(!b.dirty());

        let mut capped = Buffer::new("/g");
        capped.loaded(Some("x\n\n--- truncated ---".into()), true);
        assert_eq!(capped.read_only, Some(ReadOnly::Truncated));
        assert!(!capped.editable() && !capped.take(None));

        let mut binary = Buffer::new("/h");
        binary.loaded(None, true);
        assert_eq!(binary.read_only, Some(ReadOnly::Binary));
        let mut gone = Buffer::new("/i");
        gone.loaded(None, false);
        assert_eq!(gone.read_only, Some(ReadOnly::Unreadable));
    }

    #[test]
    fn a_draft_that_differs_is_restored_with_its_base() {
        let mut b = Buffer::new("/f");
        b.loaded(Some("disk".into()), true);
        let base = Some(Stamp {
            size: 4.0,
            mtime_ms: 1.0,
        });
        let d = Draft {
            file_path: "/f".into(),
            text: "mine".into(),
            base,
            saved_at: 0.0,
        };
        assert!(b.take(Some(&d)));
        assert_eq!(b.draft, "mine");
        assert_eq!(b.base, base);
        assert!(b.dirty());
    }

    #[test]
    fn drafts_survive_a_reload_and_are_keyed_by_pane() {
        let dir = std::env::temp_dir().join(format!("vorn-drafts-{}", std::process::id()));
        let file = drafts_file(&dir);
        let mut d = Drafts::load(Some(file.clone()));
        d.write("tab", "/f", "text", None);
        let back = Drafts::load(Some(file.clone()));
        assert_eq!(
            back.read("tab", "/f").map(|d| d.text.as_str()),
            Some("text")
        );
        assert!(back.read("tab", "/other").is_none());
        let mut back = back;
        back.prune(|_| false);
        assert!(Drafts::load(Some(file)).read("tab", "/f").is_none());
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn paths_read_relative_to_the_session() {
        assert_eq!(rel_path("/p", "/p/src/a.ts"), "src/a.ts");
        assert_eq!(rel_path("/p", "/q/a.ts"), "/q/a.ts");
        assert_eq!(rel_path("", "/q/a.ts"), "/q/a.ts");
    }
}

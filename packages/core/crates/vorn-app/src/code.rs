//! A file's text being edited (the element tree's `code`): what the client
//! keeps of each one it draws. The extension that placed it owns the file
//! (reading, drafts, saving and its guard); the client owns the edit while
//! it is typed: the text, its colours and the find bar, because they change
//! with every key and a round trip per key would put the extension between
//! a key and its letter.

use std::collections::HashMap;
use web_time::Instant;

use crate::points::unscope;

/// Past this many lines colour costs more than it gives (`MAX_HIGHLIGHT_LINES`).
pub const MAX_HIGHLIGHT_LINES: usize = 2000;
/// How long typing has to stop before the text is coloured again.
pub const HIGHLIGHT_SETTLE_MS: u128 = 150;
/// How long typing has to stop before the edit is told to the extension, which keeps it as a draft.
pub const DRAFT_SETTLE_MS: u128 = 400;
/// Shared by the gutter and the text (`EDIT_LINE_HEIGHT`).
pub const LINE_HEIGHT: f32 = 21.0;

/// What a code's controls do, by instance.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub enum CodeAct {
    ToggleFind(String),
    FindStep(String, i32),
    CloseFind(String),
    Save(String),
    /// One of the extension's answers to a moved file or an edit: `discard`, `force`, `disk`, `keep`.
    Tell(String, &'static str),
}

/// A code drawn in tree `tree` as node `node`: its instance key.
pub fn instance(tree: &str, node: &str) -> String {
    format!("{tree}|{node}")
}

/// The tree and node of instance `key`; a node may name a file, `|` and all.
pub fn parts(key: &str) -> (&str, &str) {
    key.split_once('|').unwrap_or((key, ""))
}

/// The session a code's tree draws for, when it is a pane's.
pub fn scope(key: &str) -> Option<&str> {
    unscope(parts(key).0).1
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

/// Every case-insensitive, non-overlapping hit, line by line, in reading order.
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

/// `path` under `cwd`, or whole.
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

/// What the client keeps of one code.
#[derive(Debug, Clone, Default)]
pub struct View {
    /// The `rev` last taken into the edit.
    pub rev: Option<u64>,
    /// The edit: what every window drawing this code shows.
    pub text: String,
    pub find: Option<Find>,
    /// When the text last changed, for the colour and draft debounces.
    pub typed: Option<Instant>,
    /// The text last coloured, and its colours.
    pub coloured: Option<(String, crate::highlight::Spans)>,
    /// The text last told to the extension, and whether it differed from the file then.
    pub told: Option<(String, bool)>,
}

/// Every code drawn, by instance.
#[derive(Debug, Default)]
pub struct Codes {
    pub views: HashMap<String, View>,
}

/// What telling the extension about an edit should say now: `Some(text)`
/// once its dirtiness flipped, or once typing settled on a text not yet told.
pub fn to_tell(view: &View, text: &str, on_disk: &str, now: Instant) -> Option<String> {
    let dirty = text != on_disk;
    let told_dirty = view.told.as_ref().is_some_and(|(_, d)| *d);
    if dirty != told_dirty && view.told.as_ref().is_none_or(|(t, _)| t != text) {
        return Some(text.to_owned());
    }
    let settled = view
        .typed
        .is_none_or(|t| now.duration_since(t).as_millis() >= DRAFT_SETTLE_MS);
    let told = view.told.as_ref().is_some_and(|(t, _)| t == text);
    (settled && dirty && !told).then(|| text.to_owned())
}

/// What an edit still settling would tell, said now: before anything that
/// changes which file the code shows, so no keystroke is left behind.
pub fn unsaid(view: &View, text: &str, on_disk: &str) -> Option<String> {
    let told_dirty = view.told.as_ref().is_some_and(|(_, d)| *d);
    let told = view.told.as_ref().is_some_and(|(t, _)| t == text);
    (!told && (text != on_disk || told_dirty)).then(|| text.to_owned())
}

#[cfg(test)]
mod tests {
    use super::*;

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
    fn paths_read_relative_to_the_session() {
        assert_eq!(rel_path("/p", "/p/src/a.ts"), "src/a.ts");
        assert_eq!(rel_path("/p", "/q/a.ts"), "/q/a.ts");
        assert_eq!(rel_path("", "/q/a.ts"), "/q/a.ts");
    }

    #[test]
    fn an_edit_is_told_when_it_starts_and_stops_differing_and_once_it_settles() {
        let t0 = Instant::now();
        let mut v = View {
            typed: Some(t0),
            ..View::default()
        };
        assert_eq!(to_tell(&v, "disk", "disk", t0), None);
        assert_eq!(to_tell(&v, "disk!", "disk", t0).as_deref(), Some("disk!"));
        v.told = Some(("disk!".into(), true));
        // Still typing: nothing until it settles.
        assert_eq!(to_tell(&v, "disk!!", "disk", t0), None);
        let later = t0 + std::time::Duration::from_millis(450);
        assert_eq!(
            to_tell(&v, "disk!!", "disk", later).as_deref(),
            Some("disk!!")
        );
        v.told = Some(("disk!!".into(), true));
        assert_eq!(to_tell(&v, "disk!!", "disk", later), None);
        // Typed back to the file: told at once.
        assert_eq!(to_tell(&v, "disk", "disk", t0).as_deref(), Some("disk"));
    }

    #[test]
    fn an_instance_names_its_tree_and_node() {
        let k = instance("vorn.files/panes/files#s1", "code:/p/a|b.ts");
        assert_eq!(parts(&k), ("vorn.files/panes/files#s1", "code:/p/a|b.ts"));
        assert_eq!(scope(&k), Some("s1"));
    }

    #[test]
    fn an_edit_still_settling_is_said_at_once_when_asked() {
        let mut v = View {
            typed: Some(Instant::now()),
            ..View::default()
        };
        assert_eq!(unsaid(&v, "disk", "disk"), None);
        assert_eq!(unsaid(&v, "disk!", "disk").as_deref(), Some("disk!"));
        v.told = Some(("disk!".into(), true));
        assert_eq!(unsaid(&v, "disk!", "disk"), None);
        assert_eq!(unsaid(&v, "disk!!", "disk").as_deref(), Some("disk!!"));
        assert_eq!(unsaid(&v, "disk", "disk").as_deref(), Some("disk"));
    }
}

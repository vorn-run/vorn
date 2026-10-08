//! Link handlers: which of a session's handlers want the text a person clicked.

use serde::Serialize;

use crate::activation::{shown_on, Subject};
use crate::js;
use crate::pack::InstalledPack;

/// Longest clicked text matched or passed on, in UTF-16 units.
pub const MAX_CLICKED_TEXT: usize = 2048;

/// A handler that wants the clicked text.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct LinkMatch {
    pub extension_id: String,
    pub extension_name: String,
    pub handler_id: String,
    pub title: String,
}

/// The handlers shown on `subject` whose pattern matches somewhere in `text`.
pub fn match_links(packs: &[InstalledPack], subject: &Subject<'_>, text: &str) -> Vec<LinkMatch> {
    let clicked = js::slice16(text, MAX_CLICKED_TEXT);
    if clicked.is_empty() {
        return Vec::new();
    }
    let mut found = Vec::new();
    for pack in packs {
        let shown = shown_on(pack, subject);
        if !shown.active {
            continue;
        }
        let handlers = pack
            .contributions()
            .map(|c| c.link_handlers())
            .unwrap_or_default();
        for handler in handlers {
            if !shown.link_handlers.contains(&handler.base.id) {
                continue;
            }
            // Checked when the manifest was read; one that no longer compiles matches nothing.
            let Ok(pattern) = regress::Regex::new(&handler.pattern) else {
                continue;
            };
            if js::test(&pattern, clicked) {
                found.push(LinkMatch {
                    extension_id: pack.id.clone(),
                    extension_name: pack.name.clone(),
                    handler_id: handler.base.id.clone(),
                    title: handler.base.title.clone(),
                });
            }
        }
    }
    found
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::pack::{tests::install, PackStore};
    use serde_json::json;
    use std::path::Path;

    #[test]
    fn offers_the_handlers_whose_pattern_matches() {
        let root = tempfile::tempdir().unwrap();
        install(
            root.path(),
            "jira",
            "1.0.0",
            &json!({ "id": "jira", "name": "Jira", "kind": "extension", "contributes": { "linkHandlers": [
                { "id": "issue", "title": "Open issue", "pattern": "\\b[A-Z]+-\\d+\\b" },
                { "id": "codex", "pattern": "x", "when": { "agent": ["codex"] } }
            ] } }),
        );
        let packs = PackStore::new(root.path()).extensions();
        let subject = Subject {
            worktree: Path::new("/"),
            agent: "claude",
            platform: "linux",
            remote_host: None,
        };
        assert_eq!(
            match_links(&packs, &subject, "see PROJ-12 x"),
            vec![LinkMatch {
                extension_id: "jira".into(),
                extension_name: "Jira".into(),
                handler_id: "issue".into(),
                title: "Open issue".into()
            }]
        );
        assert!(match_links(&packs, &subject, "nothing").is_empty());
        assert!(match_links(&packs, &subject, "").is_empty());
        let far = format!("{}PROJ-1", "a ".repeat(1100));
        assert!(match_links(&packs, &subject, &far).is_empty());
    }
}

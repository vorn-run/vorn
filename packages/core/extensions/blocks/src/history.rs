//! The commands typed into shells, newest first, kept in the extension's
//! storage: searched as a person types, and the ghost of a past command
//! that carries on from what is typed.

use vorn_extension_sdk::serde_json;
use vorn_extension_sdk::{Error, Host, Value};

/// Inputs kept, most recent first.
const MAX_ENTRIES: usize = 1000;
/// History rows a search offers.
pub const MAX_HISTORY: usize = 8;
/// Where they are kept.
pub const KEY: &str = "history";

/// One submitted input, as the desktop has always written it.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Entry {
    pub text: String,
    pub kind: String,
    pub timestamp: u64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub project_path: Option<String>,
}

/// Every input kept, newest first.
#[derive(Debug, Default, Clone, PartialEq)]
pub struct History {
    entries: Vec<Entry>,
}

/// Whether `query`'s characters appear in `text` in order.
fn subsequence(query: &str, text: &str) -> bool {
    let mut q = query.chars().peekable();
    for c in text.chars() {
        if q.peek() == Some(&c) {
            q.next();
        }
    }
    q.peek().is_none()
}

impl History {
    /// What storage holds; none read as empty.
    pub fn load(host: &mut dyn Host) -> Result<History, Error> {
        let entries = host
            .get(KEY)?
            .and_then(|v| serde_json::from_value(v).ok())
            .unwrap_or_default();
        Ok(History { entries })
    }

    fn save(&self, host: &mut dyn Host) -> Result<(), Error> {
        let v =
            serde_json::to_value(&self.entries).map_err(|e| Error::new("failed", e.to_string()))?;
        host.set(KEY, &v)
    }

    fn shell(&self) -> impl Iterator<Item = &Entry> {
        self.entries.iter().filter(|e| e.kind == "shell")
    }

    /// `text` to the front, once; kept at once.
    pub fn record(
        &mut self,
        host: &mut dyn Host,
        text: &str,
        project: Option<&str>,
        now_ms: u64,
    ) -> Result<(), Error> {
        let text = text.trim();
        if text.is_empty() {
            return Ok(());
        }
        self.entries
            .retain(|e| !(e.text == text && e.kind == "shell"));
        self.entries.insert(
            0,
            Entry {
                text: text.to_owned(),
                kind: "shell".to_owned(),
                timestamp: now_ms,
                project_path: project.map(str::to_owned),
            },
        );
        self.entries.truncate(MAX_ENTRIES);
        self.save(host)
    }

    /// Entries from elsewhere taken in behind those kept, each text once.
    pub fn import(&mut self, host: &mut dyn Host, more: Vec<Entry>) -> Result<usize, Error> {
        let before = self.entries.len();
        for e in more {
            if !self
                .entries
                .iter()
                .any(|k| k.text == e.text && k.kind == e.kind)
            {
                self.entries.push(e);
            }
        }
        self.entries.sort_by_key(|e| std::cmp::Reverse(e.timestamp));
        self.entries.truncate(MAX_ENTRIES);
        self.save(host)?;
        Ok(self.entries.len().saturating_sub(before))
    }

    /// Prefix over substring over subsequence, this project's and recent
    /// ones first; the most recent for no query.
    pub fn search(&self, query: &str, project: Option<&str>) -> Vec<&Entry> {
        let q = query.trim().to_lowercase();
        let here = |e: &Entry| e.project_path.as_deref() == project;
        if q.is_empty() {
            let mut all: Vec<&Entry> = self.shell().collect();
            all.sort_by(|a, b| here(b).cmp(&here(a)).then(b.timestamp.cmp(&a.timestamp)));
            all.truncate(MAX_HISTORY);
            return all;
        }
        let mut scored: Vec<(i64, &Entry)> = Vec::new();
        for (i, e) in self.shell().enumerate() {
            let text = e.text.to_lowercase();
            if text == q {
                continue;
            }
            let mut score = if text.starts_with(&q) {
                3000
            } else if text.contains(&q) {
                2000
            } else if subsequence(&q, &text) {
                1000
            } else {
                continue;
            };
            if here(e) {
                score += 500;
            }
            scored.push((score - i as i64, e));
        }
        scored.sort_by_key(|s| std::cmp::Reverse(s.0));
        scored
            .into_iter()
            .take(MAX_HISTORY)
            .map(|(_, e)| e)
            .collect()
    }

    /// The latest input that carries on from `input`.
    pub fn ghost(&self, input: &str, project: Option<&str>) -> Option<&str> {
        if input.is_empty() || input.contains('\n') {
            return None;
        }
        let mut fallback = None;
        for e in self.shell() {
            if e.text.len() <= input.len() || !e.text.starts_with(input) {
                continue;
            }
            if e.project_path.as_deref() == project {
                return Some(&e.text);
            }
            fallback.get_or_insert(e.text.as_str());
        }
        fallback
    }
}

/// `Entry`s read from JSON, the ones that do not read left out.
pub fn entries(v: &Value) -> Vec<Entry> {
    v.as_array()
        .into_iter()
        .flatten()
        .filter_map(|e| serde_json::from_value(e.clone()).ok())
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use vorn_extension_sdk::testing::FakeHost;

    #[test]
    fn history_ranks_prefix_then_substring_then_subsequence() {
        let mut host = FakeHost::default();
        let mut h = History::default();
        h.record(&mut host, "git status", Some("/a"), 1).unwrap();
        h.record(&mut host, "echo git", Some("/b"), 2).unwrap();
        h.record(&mut host, "grep -i t", Some("/a"), 3).unwrap();
        h.record(&mut host, "git status", Some("/a"), 4).unwrap();
        let found: Vec<_> = h
            .search("git", Some("/a"))
            .into_iter()
            .map(|e| e.text.as_str())
            .collect();
        assert_eq!(found, vec!["git status", "echo git", "grep -i t"]);
        assert!(h.search("git status", Some("/a")).is_empty());
        assert_eq!(h.search("", Some("/b"))[0].text, "echo git");
        assert_eq!(h.ghost("git s", Some("/z")), Some("git status"));
        assert_eq!(h.ghost("git status", None), None);
    }

    #[test]
    fn history_survives_its_instance_and_imports_once() {
        let mut host = FakeHost::default();
        History::default()
            .record(&mut host, "ls -la", None, 5)
            .unwrap();
        let mut back = History::load(&mut host).unwrap();
        assert_eq!(back.search("", None)[0].text, "ls -la");
        let old = entries(&vorn_extension_sdk::json!([
            {"text": "ls -la", "kind": "shell", "timestamp": 1},
            {"text": "make", "kind": "shell", "timestamp": 2, "projectPath": "/p"},
            {"nope": true}
        ]));
        assert_eq!(back.import(&mut host, old).unwrap(), 1);
        let texts: Vec<_> = History::load(&mut host)
            .unwrap()
            .search("", None)
            .into_iter()
            .map(|e| e.text.clone())
            .collect();
        assert_eq!(texts, ["ls -la", "make"]);
    }
}

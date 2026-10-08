//! What a session's agent has spent, read from the conversation it writes.
//!
//! Only Claude Code publishes one where it can be found: under
//! `~/.claude/projects/<its cwd with every other character a dash>/<id>.jsonl`.

use std::fs::File;
use std::io::{Read, Seek, SeekFrom};
use std::path::{Path, PathBuf};

use serde::Serialize;
use serde_json::Value;

/// Read from the end, so a long conversation costs no more than its last turns.
const TAIL_BYTES: u64 = 256 * 1024;

/// The newest turn's context and how much of it came from the cache; empty
/// when the agent publishes nothing.
#[derive(Debug, Clone, Default, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Usage {
    #[serde(
        skip_serializing_if = "Option::is_none",
        serialize_with = "crate::js::serialize_some_number"
    )]
    pub context_tokens: Option<f64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub cache_hit_rate: Option<f64>,
}

/// The session a reading is for.
#[derive(Debug, Clone, Copy)]
pub struct Conversation<'a> {
    pub agent_session_id: Option<&'a str>,
    pub worktree: Option<&'a str>,
    pub project: &'a str,
}

/// Each UTF-16 unit that is not an ASCII letter or digit becomes a dash.
fn transcript_dir(cwd: &str) -> String {
    cwd.chars()
        .flat_map(|c| {
            let dashes = if c.is_ascii_alphanumeric() {
                0
            } else {
                c.len_utf16()
            };
            let kept = (dashes == 0).then_some(c);
            kept.into_iter().chain(std::iter::repeat_n('-', dashes))
        })
        .collect()
}

fn transcript(of: Conversation<'_>, home: &Path) -> Option<PathBuf> {
    let id = of.agent_session_id?;
    let named = !id.is_empty()
        && id
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || matches!(b, b'.' | b'_' | b'-'));
    if !named {
        return None;
    }
    [of.worktree, Some(of.project)]
        .into_iter()
        .flatten()
        .filter(|cwd| !cwd.is_empty())
        .map(|cwd| {
            home.join(".claude")
                .join("projects")
                .join(transcript_dir(cwd))
                .join(format!("{id}.jsonl"))
        })
        .find(|path| path.exists())
}

fn read_tail(path: &Path) -> std::io::Result<String> {
    let mut file = File::open(path)?;
    let size = file.metadata()?.len();
    let length = size.min(TAIL_BYTES);
    file.seek(SeekFrom::Start(size - length))?;
    let mut buf = Vec::new();
    file.take(length).read_to_end(&mut buf)?;
    Ok(String::from_utf8_lossy(&buf).into_owned())
}

fn last_usage(contents: &str) -> Option<serde_json::Map<String, Value>> {
    contents.split('\n').rev().find_map(|line| {
        let parsed: Value = serde_json::from_str(line.trim()).ok()?;
        parsed.get("message")?.get("usage")?.as_object().cloned()
    })
}

/// The newest turn's usage in `of`'s conversation under `home`.
pub fn usage_for(of: Conversation<'_>, home: &Path) -> Usage {
    let Some(usage) = transcript(of, home)
        .and_then(|path| read_tail(&path).ok())
        .and_then(|text| last_usage(&text))
    else {
        return Usage::default();
    };
    let count = |name: &str| usage.get(name).and_then(Value::as_f64).unwrap_or(0.0);
    let cache_read = count("cache_read_input_tokens");
    let context = count("input_tokens") + count("cache_creation_input_tokens") + cache_read;
    if context == 0.0 {
        return Usage::default();
    }
    Usage {
        context_tokens: Some(context),
        cache_hit_rate: Some(cache_read / context),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;

    fn write(home: &Path, cwd: &str, id: &str, lines: &[&str]) {
        let dir = home.join(".claude/projects").join(transcript_dir(cwd));
        fs::create_dir_all(&dir).unwrap();
        fs::write(dir.join(format!("{id}.jsonl")), lines.join("\n")).unwrap();
    }

    fn of<'a>(id: &'a str, worktree: Option<&'a str>) -> Conversation<'a> {
        Conversation {
            agent_session_id: Some(id),
            worktree,
            project: "/p",
        }
    }

    #[test]
    fn counts_context_and_cache_from_the_newest_turn() {
        let home = tempfile::tempdir().unwrap();
        write(
            home.path(),
            "/p",
            "abc",
            &[
                r#"{"message":{"usage":{"input_tokens":1}}}"#,
                r#"{"message":{"usage":{"input_tokens":10,"cache_creation_input_tokens":30,"cache_read_input_tokens":60}}}"#,
                r#"{"message":{"role":"user"}}"#,
                r#"{"half"#,
            ],
        );
        let u = usage_for(of("abc", None), home.path());
        assert_eq!(u.context_tokens, Some(100.0));
        assert_eq!(u.cache_hit_rate, Some(0.6));
        assert_eq!(serde_json::to_value(&u).unwrap()["contextTokens"], 100.0);
    }

    #[test]
    fn reads_the_worktree_before_the_project() {
        let home = tempfile::tempdir().unwrap();
        write(
            home.path(),
            "/p",
            "a",
            &[r#"{"message":{"usage":{"input_tokens":1}}}"#],
        );
        write(
            home.path(),
            "/w",
            "a",
            &[r#"{"message":{"usage":{"input_tokens":2}}}"#],
        );
        assert_eq!(
            usage_for(of("a", Some("/w")), home.path()).context_tokens,
            Some(2.0)
        );
    }

    #[test]
    fn says_nothing_without_a_usable_conversation() {
        let home = tempfile::tempdir().unwrap();
        write(
            home.path(),
            "/p",
            "z",
            &[r#"{"message":{"usage":{"input_tokens":0}}}"#],
        );
        assert_eq!(usage_for(of("z", None), home.path()), Usage::default());
        assert_eq!(usage_for(of("../z", None), home.path()), Usage::default());
        assert_eq!(
            usage_for(of("missing", None), home.path()),
            Usage::default()
        );
        let none = Conversation {
            agent_session_id: None,
            worktree: None,
            project: "/p",
        };
        assert_eq!(usage_for(none, home.path()), Usage::default());
    }

    #[test]
    fn names_the_directory_as_claude_code_does() {
        assert_eq!(transcript_dir("/Users/a.b/x😀"), "-Users-a-b-x--");
    }
}

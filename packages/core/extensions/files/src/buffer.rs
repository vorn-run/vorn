//! An open file: what is on disk, whether it can be edited, the edit kept
//! as a draft until saved, and the guard that keeps a save from writing over
//! a file that changed on disk under the edit.

use serde::{Deserialize, Serialize};
use vorn_extension_sdk::tree::CodeState;
use vorn_extension_sdk::{json, serde_json, Error, Host, Value};

/// What a file was when read: its size and modification time.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Stamp {
    pub size: f64,
    pub mtime_ms: f64,
}

impl Stamp {
    pub fn read(v: &Value) -> Option<Stamp> {
        Some(Stamp {
            size: v.get("size")?.as_f64()?,
            mtime_ms: v.get("mtimeMs")?.as_f64()?,
        })
    }
}

/// Whether the file moved since it was read: no base never armed the guard;
/// an unknown current stamp counts as moved.
pub fn has_moved(base: Option<Stamp>, current: Option<Stamp>) -> bool {
    match (base, current) {
        (None, _) => false,
        (Some(_), None) => true,
        (Some(b), Some(c)) => b != c,
    }
}

/// A capped read, which must not be saved over the file.
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

/// An edit never saved, kept so a quit does not throw it away.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Draft {
    pub file_path: String,
    pub text: String,
    pub base: Option<Stamp>,
    #[serde(default)]
    pub saved_at: f64,
}

fn draft_key(key: &str) -> String {
    format!("draft/{key}")
}

fn rev_key(key: &str) -> String {
    format!("rev/{key}")
}

/// The draft kept for buffer `key`, if any.
pub fn read_draft(host: &mut dyn Host, key: &str) -> Result<Option<Draft>, Error> {
    Ok(host
        .get(&draft_key(key))?
        .and_then(|v| serde_json::from_value(v).ok()))
}

pub fn forget_draft(host: &mut dyn Host, key: &str) -> Result<(), Error> {
    host.set(&draft_key(key), &Value::Null)?;
    host.set(&rev_key(key), &Value::Null)
}

/// `path`'s stamp now, if it can be read.
pub fn stamp(host: &mut dyn Host, path: &str) -> Option<Stamp> {
    host.call("files.stat", json!({ "path": path }))
        .ok()
        .as_ref()
        .and_then(Stamp::read)
}

/// One open file.
#[derive(Debug, Clone, PartialEq)]
pub struct Buffer {
    pub path: String,
    /// What is on disk, once read.
    pub content: Option<String>,
    pub state: CodeState,
    /// The edit, while it differs from `content`.
    pub draft: Option<String>,
    /// What the file was when the edit started.
    pub base: Option<Stamp>,
    /// The file moved on disk under the edit.
    pub conflict: bool,
    pub error: Option<String>,
    /// Bumped whenever the client must take the text afresh; kept, so a
    /// new instance does not throw away an edit the client holds.
    pub rev: u64,
}

impl Buffer {
    /// `path` read through Vorn into buffer `key`, with the draft kept for it.
    pub fn load(host: &mut dyn Host, key: &str, path: &str, now_ms: u64) -> Result<Buffer, Error> {
        let read = host
            .call("files.read", json!({ "path": path }))
            .ok()
            .and_then(|v| v.as_str().map(str::to_owned));
        let mut b = Buffer {
            path: path.to_owned(),
            content: None,
            state: CodeState::Loading,
            draft: None,
            base: None,
            conflict: false,
            error: None,
            rev: 0,
        };
        let now = stamp(host, path);
        match read {
            Some(text) => {
                b.state = if is_truncated(&text) {
                    CodeState::Read
                } else {
                    CodeState::Edit
                };
                b.content = Some(text);
            }
            None => {
                b.state = if now.is_some() {
                    CodeState::Binary
                } else {
                    CodeState::Unreadable
                };
            }
        }
        let kept = read_draft(host, key)?.filter(|d| d.file_path == path);
        let editable = b.state == CodeState::Edit;
        match kept.filter(|d| editable && Some(&d.text) != b.content.as_ref()) {
            Some(d) => {
                b.base = d.base;
                b.conflict = has_moved(d.base, now);
                b.draft = Some(d.text);
            }
            None => {
                host.set(&draft_key(key), &Value::Null)?;
                b.base = now.filter(|_| editable);
            }
        }
        b.rev = match host.get(&rev_key(key))?.and_then(|v| v.as_u64()) {
            Some(r) => r,
            None => {
                host.set(&rev_key(key), &json!(now_ms))?;
                now_ms
            }
        };
        Ok(b)
    }

    pub fn editable(&self) -> bool {
        self.state == CodeState::Edit
    }

    pub fn dirty(&self) -> bool {
        self.editable() && self.draft.is_some()
    }

    /// The client's text: kept as a draft while it differs from the file.
    pub fn edited(
        &mut self,
        host: &mut dyn Host,
        key: &str,
        text: String,
        now_ms: u64,
    ) -> Result<(), Error> {
        if !self.editable() {
            return Ok(());
        }
        if Some(&text) == self.content.as_ref() {
            self.draft = None;
            return host.set(&draft_key(key), &Value::Null);
        }
        let d = Draft {
            file_path: self.path.clone(),
            text: text.clone(),
            base: self.base,
            saved_at: now_ms as f64,
        };
        self.draft = Some(text);
        let v = serde_json::to_value(&d).map_err(|e| Error::new("failed", e.to_string()))?;
        host.set(&draft_key(key), &v)
    }

    /// The client must take the text afresh.
    fn renew(&mut self, host: &mut dyn Host, key: &str, now_ms: u64) -> Result<(), Error> {
        self.rev = now_ms.max(self.rev + 1);
        host.set(&rev_key(key), &json!(self.rev))
    }

    /// Writes `text`, unless the file moved since the edit started and not `force`.
    pub fn save(
        &mut self,
        host: &mut dyn Host,
        key: &str,
        text: String,
        force: bool,
    ) -> Result<(), Error> {
        if !self.editable() {
            return Ok(());
        }
        self.error = None;
        if !force && has_moved(self.base, stamp(host, &self.path)) {
            self.conflict = true;
            self.draft = Some(text);
            return Ok(());
        }
        let wrote = host.call("files.write", json!({ "path": self.path, "content": text }));
        let failed = match &wrote {
            Ok(v) if v.get("success").and_then(Value::as_bool) == Some(true) => None,
            Ok(v) => Some(
                v.get("error")
                    .and_then(Value::as_str)
                    .unwrap_or("Failed to save")
                    .to_owned(),
            ),
            Err(e) => Some(e.message.clone()),
        };
        if let Some(e) = failed {
            self.error = Some(e);
            self.draft = Some(text);
            return Ok(());
        }
        if is_truncated(&text) {
            self.state = CodeState::Read;
        }
        self.content = Some(text);
        self.draft = None;
        self.conflict = false;
        self.base = stamp(host, &self.path);
        host.set(&draft_key(key), &Value::Null)
    }

    /// Throws the edit away for what was read.
    pub fn discard(&mut self, host: &mut dyn Host, key: &str, now_ms: u64) -> Result<(), Error> {
        self.draft = None;
        self.error = None;
        self.conflict = false;
        host.set(&draft_key(key), &Value::Null)?;
        self.base = stamp(host, &self.path).filter(|_| self.editable());
        self.renew(host, key, now_ms)
    }

    /// Throws the edit away for what is on disk now.
    pub fn take_disk(&mut self, host: &mut dyn Host, key: &str, now_ms: u64) -> Result<(), Error> {
        let read = host
            .call("files.read", json!({ "path": self.path }))
            .ok()
            .and_then(|v| v.as_str().map(str::to_owned));
        let Some(text) = read else {
            self.error = Some("The file could not be read.".to_owned());
            return Ok(());
        };
        self.content = Some(text);
        self.discard(host, key, now_ms)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::cell::RefCell;
    use std::rc::Rc;
    use vorn_extension_sdk::testing::FakeHost;

    #[test]
    fn capped_reads_are_known_by_their_marker() {
        assert!(is_truncated("a\n\n--- truncated ---"));
        assert!(is_truncated("a\n\n--- truncated (12345 bytes total) ---"));
        assert!(!is_truncated("a\n\n--- truncated (x bytes total) ---"));
        assert!(!is_truncated("--- truncated ---"));
        assert!(!is_truncated("plain"));
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

    /// A file's text and modification time, which tests change under the edit.
    type File = Rc<RefCell<(Option<String>, f64)>>;

    /// A file on a fake disk.
    fn disk(text: &str) -> (FakeHost, File) {
        let file = Rc::new(RefCell::new((Some(text.to_owned()), 1.0)));
        let (r, s, w) = (file.clone(), file.clone(), file.clone());
        let host = FakeHost::default()
            .answer("files.read", move |_| Ok(json!(r.borrow().0)))
            .answer("files.stat", move |_| {
                let f = s.borrow();
                Ok(f.0
                    .as_ref()
                    .map_or(Value::Null, |t| json!({"size": t.len(), "mtimeMs": f.1})))
            })
            .answer("files.write", move |a| {
                let mut f = w.borrow_mut();
                f.0 = a["content"].as_str().map(str::to_owned);
                f.1 += 1.0;
                Ok(json!({"success": true}))
            });
        (host, file)
    }

    #[test]
    fn an_edit_is_kept_as_a_draft_and_saved_unless_the_file_moved() {
        let (mut host, file) = disk("one");
        let mut b = Buffer::load(&mut host, "k", "/f", 10).unwrap();
        assert_eq!((b.state, b.rev, b.dirty()), (CodeState::Edit, 10, false));
        b.edited(&mut host, "k", "one!".into(), 11).unwrap();
        assert!(b.dirty());
        // A new instance finds the draft and the same rev.
        let again = Buffer::load(&mut host, "k", "/f", 99).unwrap();
        assert_eq!((again.draft.as_deref(), again.rev), (Some("one!"), 10));
        b.save(&mut host, "k", "one!".into(), false).unwrap();
        assert_eq!(file.borrow().0.as_deref(), Some("one!"));
        assert!(!b.dirty());
        // Someone else writes the file: the next save stops and says so.
        b.edited(&mut host, "k", "mine".into(), 12).unwrap();
        file.borrow_mut().1 += 5.0;
        b.save(&mut host, "k", "mine".into(), false).unwrap();
        assert!(b.conflict);
        assert_eq!(file.borrow().0.as_deref(), Some("one!"));
        b.save(&mut host, "k", "mine".into(), true).unwrap();
        assert_eq!(file.borrow().0.as_deref(), Some("mine"));
        assert!(!b.conflict);
    }

    #[test]
    fn a_draft_over_a_moved_file_says_so_and_discard_takes_the_disk() {
        let (mut host, file) = disk("disk");
        let mut b = Buffer::load(&mut host, "k", "/f", 1).unwrap();
        b.edited(&mut host, "k", "mine".into(), 2).unwrap();
        file.borrow_mut().0 = Some("theirs".into());
        file.borrow_mut().1 = 9.0;
        let mut b = Buffer::load(&mut host, "k", "/f", 3).unwrap();
        assert!(b.conflict && b.dirty());
        b.take_disk(&mut host, "k", 4).unwrap();
        assert_eq!(b.content.as_deref(), Some("theirs"));
        assert!(!b.dirty() && !b.conflict);
        assert!(b.rev > 1);
    }

    #[test]
    fn binary_gone_and_capped_files_are_not_editable() {
        let (mut host, file) = disk("x\n\n--- truncated ---");
        assert_eq!(
            Buffer::load(&mut host, "a", "/f", 1).unwrap().state,
            CodeState::Read
        );
        file.borrow_mut().0 = None;
        assert_eq!(
            Buffer::load(&mut host, "b", "/f", 1).unwrap().state,
            CodeState::Unreadable
        );
        let mut host = FakeHost::default()
            .answer("files.read", |_| Ok(Value::Null))
            .answer("files.stat", |_| Ok(json!({"size": 4, "mtimeMs": 1})));
        assert_eq!(
            Buffer::load(&mut host, "c", "/f", 1).unwrap().state,
            CodeState::Binary
        );
    }
}

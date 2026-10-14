//! Program status reports (OSC 7501) for what `vorn` waits on with
//! `--wait`, so the terminal it runs in shows the run's progress.

use std::collections::HashMap;
use std::time::Duration;

use data_encoding::BASE64;
use serde_json::Value;

use crate::js::field;

/// How often a run waited on is looked at.
pub const POLL: Duration = Duration::from_secs(1);

/// The decoded message limit of a report.
const MAX_MSG: usize = 2048;

/// One record of a report.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Record {
    /// `""` for the run itself, else a step under it.
    pub id: String,
    /// `idle`, `working`, `blocked`, `done`, `error` or `clear`.
    pub state: &'static str,
    /// For `blocked`: `permission`, `question` or `auth`.
    pub kind: Option<&'static str>,
    pub progress: Option<u8>,
    pub msg: String,
}

impl Record {
    pub fn new(id: &str, state: &'static str, msg: &str) -> Record {
        Record {
            id: id.to_owned(),
            state,
            kind: None,
            progress: None,
            msg: msg.to_owned(),
        }
    }

    /// The escape sequence that reports it.
    pub fn encode(&self) -> String {
        let mut out = format!("\x1b]7501;state={}", self.state);
        if let Some(kind) = self.kind {
            out.push_str(&format!(":kind={kind}"));
        }
        if let Some(p) = self.progress {
            out.push_str(&format!(":progress={}", p.min(100)));
        }
        let id = record_id(&self.id);
        if !id.is_empty() {
            out.push_str(&format!(":id={id}"));
        }
        out.push_str(":app=vorn");
        let msg = printable(&self.msg);
        if !msg.is_empty() {
            out.push_str(&format!(":msg={}", BASE64.encode(msg.as_bytes())));
        }
        out.push_str("\x1b\\");
        out
    }
}

/// An id the protocol takes: segments of letters, digits, `-`, `_` and `.`.
fn record_id(id: &str) -> String {
    id.split('/')
        .map(|seg| {
            seg.chars()
                .map(|c| {
                    if c.is_ascii_alphanumeric() || matches!(c, '-' | '_' | '.') {
                        c
                    } else {
                        '-'
                    }
                })
                .collect::<String>()
        })
        .collect::<Vec<_>>()
        .join("/")
}

/// `text` without control characters, cut to the message limit on a character boundary.
fn printable(text: &str) -> String {
    let mut out = String::new();
    for c in text.chars().filter(|c| !c.is_control()) {
        if out.len() + c.len_utf8() > MAX_MSG {
            break;
        }
        out.push(c);
    }
    out
}

/// What a workflow run reports: the run, then each step that runs or waits.
/// `labels` names the steps by node id.
pub fn run_records(run: &Value, labels: &HashMap<String, String>, name: &str) -> Vec<Record> {
    let states: &[Value] = field(run, "nodeStates")
        .and_then(Value::as_array)
        .map_or(&[], Vec::as_slice);
    let node = |s: &Value| {
        field(s, "nodeId")
            .and_then(Value::as_str)
            .unwrap_or("")
            .to_owned()
    };
    let label = |s: &Value| {
        let id = node(s);
        labels.get(&id).cloned().unwrap_or(id)
    };
    let counted: Vec<&Value> = states.iter().filter(|s| status(s) != "skipped").collect();
    let finished = counted
        .iter()
        .filter(|s| matches!(status(s), "success" | "error"))
        .count();
    let progress = (!counted.is_empty()).then(|| (finished * 100 / counted.len()) as u8);

    let mut root = match field(run, "status").and_then(Value::as_str) {
        Some("success") => Record::new("", "done", name),
        Some("error") => Record::new("", "error", name),
        Some("cancelled") => Record::new("", "idle", name),
        _ => {
            let waiting = states.iter().find(|s| status(s) == "waiting");
            let running = states.iter().find(|s| status(s) == "running");
            match (waiting, running) {
                (Some(w), _) => Record {
                    kind: Some(wait_kind(w)),
                    ..Record::new("", "blocked", &format!("{name}: {}", label(w)))
                },
                (None, Some(r)) => Record::new("", "working", &format!("{name}: {}", label(r))),
                (None, None) => Record::new("", "working", name),
            }
        }
    };
    root.progress = progress;

    let mut out = vec![root];
    for s in states {
        let step = match status(s) {
            "running" => Record::new(&node(s), "working", &label(s)),
            "waiting" => Record {
                kind: Some(wait_kind(s)),
                ..Record::new(&node(s), "blocked", &label(s))
            },
            _ => Record::new(&node(s), "clear", ""),
        };
        out.push(step);
    }
    out
}

fn status(state: &Value) -> &str {
    field(state, "status").and_then(Value::as_str).unwrap_or("")
}

/// Why a step waits: a sign-in, else an approval asked of a person.
fn wait_kind(state: &Value) -> &'static str {
    if field(state, "waitingFor").and_then(Value::as_str) == Some("signIn") {
        "auth"
    } else {
        "question"
    }
}

/// What a headless session reports: working while it runs, then how it ended.
pub fn headless_record(session: &Value, name: &str) -> Record {
    if field(session, "status").and_then(Value::as_str) != Some("exited") {
        return Record::new("", "working", name);
    }
    match field(session, "exitCode").and_then(Value::as_i64) {
        Some(0) | None => Record::new("", "done", name),
        Some(code) => Record::new("", "error", &format!("{name}: exited with {code}")),
    }
}

/// Reports only what changed since `last`, keyed by record id.
#[derive(Debug, Default)]
pub struct Reporter {
    last: HashMap<String, Record>,
}

impl Reporter {
    /// The sequences to write for `records`.
    pub fn changes(&mut self, records: Vec<Record>) -> String {
        let mut out = String::new();
        for r in records {
            let unchanged = self.last.get(&r.id) == Some(&r);
            let fresh_clear = r.state == "clear" && !self.last.contains_key(&r.id);
            if unchanged || fresh_clear {
                continue;
            }
            out.push_str(&r.encode());
            if r.state == "clear" {
                self.last.remove(&r.id);
            } else {
                self.last.insert(r.id.clone(), r);
            }
        }
        out
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn a_record_encodes_as_the_protocol_reads_it() {
        let mut r = Record::new("deploy/build 1", "blocked", "Sign in\x07 to GitHub");
        r.kind = Some("auth");
        r.progress = Some(140);
        assert_eq!(
            r.encode(),
            format!(
                "\x1b]7501;state=blocked:kind=auth:progress=100:id=deploy/build-1:app=vorn:msg={}\x1b\\",
                BASE64.encode(b"Sign in to GitHub")
            )
        );
        assert_eq!(
            Record::new("", "done", "").encode(),
            "\x1b]7501;state=done:app=vorn\x1b\\"
        );
        assert_eq!(printable(&"é".repeat(2000)).len(), 2048);
    }

    #[test]
    fn a_run_reports_itself_and_its_steps_and_only_what_changed() {
        let labels = HashMap::from([
            ("a".to_owned(), "Build".to_owned()),
            ("b".to_owned(), "Approve".to_owned()),
        ]);
        let mut rep = Reporter::default();
        let run = json!({ "status": "running", "nodeStates": [
            { "nodeId": "a", "status": "running" },
            { "nodeId": "b", "status": "pending" },
            { "nodeId": "c", "status": "skipped" },
        ]});
        let out = rep.changes(run_records(&run, &labels, "Ship"));
        assert!(
            out.starts_with("\x1b]7501;state=working:progress=0:app=vorn:msg="),
            "{out:?}"
        );
        assert!(out.contains("state=working:id=a:app=vorn"));
        assert!(!out.contains("id=b") && !out.contains("id=c"));
        assert_eq!(rep.changes(run_records(&run, &labels, "Ship")), "");

        let run = json!({ "status": "running", "nodeStates": [
            { "nodeId": "a", "status": "success" },
            { "nodeId": "b", "status": "waiting", "waitingFor": "signIn" },
        ]});
        let out = rep.changes(run_records(&run, &labels, "Ship"));
        assert!(
            out.contains("state=blocked:kind=auth:progress=50:app=vorn"),
            "{out:?}"
        );
        assert!(out.contains("state=clear:id=a:app=vorn"));
        assert!(out.contains("state=blocked:kind=auth:id=b"));

        let run = json!({ "status": "success", "nodeStates": [
            { "nodeId": "a", "status": "success" },
            { "nodeId": "b", "status": "success" },
        ]});
        let out = rep.changes(run_records(&run, &labels, "Ship"));
        assert!(out.contains("state=done:progress=100:app=vorn"), "{out:?}");
        assert!(out.contains("state=clear:id=b"));
    }

    #[test]
    fn a_headless_session_reports_how_it_ended() {
        assert_eq!(
            headless_record(&json!({ "status": "running" }), "fix").state,
            "working"
        );
        assert_eq!(
            headless_record(&json!({ "status": "exited", "exitCode": 0 }), "fix").state,
            "done"
        );
        let failed = headless_record(&json!({ "status": "exited", "exitCode": 2 }), "fix");
        assert_eq!(
            (failed.state, failed.msg.as_str()),
            ("error", "fix: exited with 2")
        );
    }
}

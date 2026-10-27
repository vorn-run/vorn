//! What a session's program reports of itself (OSC 7501), as the card and
//! the sidebar word it: the root record's line and progress, and each child
//! record as a step under it.

use vorn_protocol::{ProgramRecord, ProgramStatus};

use super::Session;

/// The report a session shows.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Report {
    /// What the program is doing or waits for, in a few words.
    pub line: String,
    /// 0–100, when it says.
    pub progress: Option<u8>,
    /// It finished and nobody has looked since.
    pub unseen: bool,
    pub steps: Vec<Step>,
}

/// A child record, shown as a sub-row.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Step {
    /// How deep under the root: 0 for "a", 1 for "a/b".
    pub depth: usize,
    pub line: String,
    /// The status word its dot is drawn from (`running`, `waiting`, ...).
    pub status: &'static str,
    pub progress: Option<u8>,
}

impl Session {
    /// What its program reports, when it reports anything worth a line.
    pub fn report(&self) -> Option<Report> {
        let p: &ProgramStatus = self.info.program_status.as_ref()?;
        let root = p.records.iter().find(|r| r.id.is_empty());
        let steps: Vec<Step> = p
            .records
            .iter()
            .filter(|r| !r.id.is_empty())
            .map(|r| Step {
                depth: r.id.matches('/').count(),
                line: step_line(r),
                status: status_word(r.state.as_str()),
                progress: percent(r),
            })
            .collect();
        let progress = root.and_then(percent);
        let mut line = root.map(root_line).unwrap_or_default();
        if line.is_empty() && progress.is_some() {
            line = "Working".to_owned();
        }
        if line.is_empty() && steps.is_empty() {
            return None;
        }
        Some(Report {
            line,
            progress,
            unseen: p.unseen == Some(true),
            steps,
        })
    }
}

fn text(s: &Option<String>) -> &str {
    s.as_deref().map(str::trim).unwrap_or("")
}

fn percent(r: &ProgramRecord) -> Option<u8> {
    r.progress.map(|p| p.clamp(0.0, 100.0).round() as u8)
}

/// What a blocked record waits for.
fn waits_for(r: &ProgramRecord) -> &'static str {
    match r.kind.as_deref().map(String::as_str) {
        Some("permission") => "Waiting for permission",
        Some("question") => "Waiting for an answer",
        Some("auth") => "Waiting for sign-in",
        _ => "Waiting for input",
    }
}

/// The root record's line: its state when that is news, then its message.
fn root_line(r: &ProgramRecord) -> String {
    let lead = match r.state.as_str() {
        "blocked" => Some(waits_for(r)),
        "done" => Some("Finished"),
        "error" => Some("Failed"),
        _ => None,
    };
    let msg = match text(&r.msg) {
        "" => text(&r.title),
        m => m,
    };
    match (lead, msg) {
        (Some(l), "") => l.to_owned(),
        (Some(l), m) => format!("{l} \u{b7} {m}"),
        (None, m) => m.to_owned(),
    }
}

fn step_line(r: &ProgramRecord) -> String {
    let name = match text(&r.title) {
        "" => r.id.rsplit('/').next().unwrap_or(&r.id),
        t => t,
    };
    let msg = match r.state.as_str() {
        "blocked" => waits_for(r),
        _ => text(&r.msg),
    };
    if msg.is_empty() {
        name.to_owned()
    } else {
        format!("{name} \u{b7} {msg}")
    }
}

/// The session status a record's state reads as.
fn status_word(state: &str) -> &'static str {
    match state {
        "working" => "running",
        "blocked" => "waiting",
        "error" => "error",
        _ => "idle",
    }
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::*;

    fn session(program: serde_json::Value) -> Session {
        let info = serde_json::from_value(json!({
            "id": "a", "agentType": "acme", "projectName": "p", "projectPath": "/p",
            "status": "waiting", "createdAt": 1.0, "pid": 1.0, "programStatus": program,
        }))
        .unwrap();
        Session {
            info,
            exited: None,
            cause: None,
            ordinal: 0,
        }
    }

    #[test]
    fn the_root_says_what_it_waits_for_and_children_are_steps() {
        let s = session(json!({ "native": true, "records": [
            { "id": "", "state": "blocked", "kind": "auth", "progress": 40, "msg": "Deploying" },
            { "id": "build", "state": "working", "progress": 60, "title": "Build" },
            { "id": "build/test", "state": "blocked", "kind": "question" },
        ]}));
        let r = s.report().unwrap();
        assert_eq!(r.line, "Waiting for sign-in \u{b7} Deploying");
        assert_eq!(r.progress, Some(40));
        assert_eq!(
            r.steps,
            [
                Step {
                    depth: 0,
                    line: "Build".into(),
                    status: "running",
                    progress: Some(60)
                },
                Step {
                    depth: 1,
                    line: "test \u{b7} Waiting for an answer".into(),
                    status: "waiting",
                    progress: None,
                },
            ]
        );
    }

    #[test]
    fn a_finished_report_reads_finished_until_seen() {
        let s = session(json!({ "native": false, "unseen": true, "records": [
            { "id": "", "state": "done", "msg": "Built" },
        ]}));
        let r = s.report().unwrap();
        assert_eq!((r.line.as_str(), r.unseen), ("Finished \u{b7} Built", true));
        assert_eq!(
            session(json!({ "native": true, "records": [] })).report(),
            None
        );
        let quiet =
            session(json!({ "native": true, "records": [{ "id": "", "state": "working" }] }));
        assert_eq!(quiet.report(), None);
    }
}

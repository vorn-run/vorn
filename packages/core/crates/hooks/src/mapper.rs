//! Which terminal a hook event is about, and the status it means
//! (`hookStatusMapper`).
//!
//! An event names its terminal exactly when Vorn started the agent (its
//! terminal id), or by the conversation a terminal was started on; failing
//! both, a link made before; failing that, the newest terminal in the
//! event's directory that no conversation claims yet. A link found exactly
//! overrules one the directory guessed. Each link is the hook session the
//! terminal's record should then carry ([`Resolved::link`]).

use std::collections::HashMap;
use std::path::{Path, PathBuf};

use crate::event::Event;

/// A live terminal, as far as linking needs it.
#[derive(Clone, Debug, PartialEq)]
pub struct Terminal {
    pub id: String,
    /// The conversation it was started on (`agentSessionId`).
    pub agent_session: Option<String>,
    /// The conversation its hooks are linked to (`hookSessionId`).
    pub hook_session: Option<String>,
    pub created_at: f64,
    /// Its worktree, else its project, as [`normalize`] reads it.
    pub path: PathBuf,
}

/// The terminal an event is about.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Resolved {
    pub terminal: String,
    /// The hook session to set on the terminal's record, when it changed.
    pub link: Option<String>,
}

/// An agent's status as a hook event reports it.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Status {
    Running,
    Waiting,
    Idle,
    Error,
}

impl Status {
    pub fn as_str(self) -> &'static str {
        match self {
            Status::Running => "running",
            Status::Waiting => "waiting",
            Status::Idle => "idle",
            Status::Error => "error",
        }
    }
}

/// The status an event's name means; `None` for one that means none.
pub fn status_of(name: &str) -> Option<Status> {
    Some(match name {
        "SessionStart" | "PreToolUse" | "PostToolUse" => Status::Running,
        "PostToolUseFailure" => Status::Error,
        "Notification" | "PermissionRequest" => Status::Waiting,
        "Stop" | "SessionEnd" => Status::Idle,
        _ => return None,
    })
}

/// A directory as terminals are matched by it (`normalizePath`): no trailing
/// separator, and the real path where there is one.
pub fn normalize(dir: &str) -> PathBuf {
    let trimmed = dir.trim_end_matches(['/', '\\']);
    let kept = if trimmed.is_empty() && !dir.is_empty() {
        &dir[..1]
    } else {
        trimmed
    };
    let path = Path::new(kept);
    std::fs::canonicalize(path).unwrap_or_else(|_| path.to_path_buf())
}

/// The links from conversations to terminals, made as events arrive.
#[derive(Debug, Default)]
pub struct Mapper {
    links: HashMap<String, String>,
}

impl Mapper {
    pub fn linked(&self, session: &str) -> Option<&str> {
        self.links.get(session).map(String::as_str)
    }

    /// Links a conversation Vorn made up itself, as for a Copilot terminal.
    pub fn force_link(&mut self, session: &str, terminal: &str) {
        self.links.insert(session.to_owned(), terminal.to_owned());
    }

    pub fn forget(&mut self, session: &str) {
        self.links.remove(session);
    }

    pub fn clear(&mut self) {
        self.links.clear();
    }

    /// The terminal `event` is about among `terminals`, linking it.
    pub fn resolve(&mut self, event: &Event, terminals: &[Terminal]) -> Option<Resolved> {
        let session = event.session();
        if let Some(exact) = exact(event, terminals) {
            if self.linked(session) == Some(exact.id.as_str()) {
                return Some(Resolved {
                    terminal: exact.id.clone(),
                    link: None,
                });
            }
            // Whatever else claimed this terminal was a guess, or a conversation it left.
            self.links.retain(|_, t| t != &exact.id);
            self.links.insert(session.to_owned(), exact.id.clone());
            tracing::info!(session, terminal = %exact.id, "linked a conversation to its terminal");
            let link = (exact.hook_session.as_deref() != Some(session)).then(|| session.to_owned());
            return Some(Resolved {
                terminal: exact.id.clone(),
                link,
            });
        }
        if let Some(terminal) = self.linked(session) {
            return Some(Resolved {
                terminal: terminal.to_owned(),
                link: None,
            });
        }
        let wanted = normalize(event.cwd());
        let mut unlinked: Vec<&Terminal> = terminals
            .iter()
            .filter(|t| t.hook_session.is_none())
            .filter(|t| !self.links.values().any(|l| l == &t.id))
            .filter(|t| t.path == wanted)
            .collect();
        unlinked.sort_by(|a, b| b.created_at.total_cmp(&a.created_at));
        let Some(newest) = unlinked.first() else {
            tracing::info!(
                session,
                cwd = event.cwd(),
                "no unlinked terminal for a conversation"
            );
            return None;
        };
        if unlinked.len() > 1 {
            tracing::warn!(
                session,
                terminal = %newest.id,
                sharing = unlinked.len(),
                "a conversation names no terminal and several share its folder; guessed the newest"
            );
        }
        self.links.insert(session.to_owned(), newest.id.clone());
        Some(Resolved {
            terminal: newest.id.clone(),
            link: Some(session.to_owned()),
        })
    }

    /// The terminal `event` is about and the status it means; a session's
    /// end forgets its link.
    pub fn map(&mut self, event: &Event, terminals: &[Terminal]) -> Option<(Resolved, Status)> {
        let resolved = self.resolve(event, terminals)?;
        let status = status_of(event.name())?;
        if event.name() == "SessionEnd" {
            self.forget(event.session());
        }
        Some((resolved, status))
    }
}

/// A live terminal the event names exactly: by its id, then by the
/// conversation it was started on (the newest such).
fn exact<'a>(event: &Event, terminals: &'a [Terminal]) -> Option<&'a Terminal> {
    if let Some(found) = event
        .terminal()
        .and_then(|id| terminals.iter().find(|t| t.id == id))
    {
        return Some(found);
    }
    terminals
        .iter()
        .filter(|t| t.agent_session.as_deref() == Some(event.session()))
        .max_by(|a, b| a.created_at.total_cmp(&b.created_at))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn event(name: &str, session: &str, cwd: &str, terminal: Option<&str>) -> Event {
        let mut v =
            serde_json::json!({ "hook_event_name": name, "session_id": session, "cwd": cwd });
        if let Some(t) = terminal {
            v["vorn_terminal_id"] = t.into();
        }
        Event::parse(v.to_string().as_bytes(), None).unwrap()
    }

    fn terminal(id: &str, path: &str, created_at: f64) -> Terminal {
        Terminal {
            id: id.into(),
            agent_session: None,
            hook_session: None,
            created_at,
            path: normalize(path),
        }
    }

    #[test]
    fn links_by_terminal_then_conversation_then_folder() {
        let mut m = Mapper::default();
        let mut started = terminal("b", "/nowhere/p", 2.0);
        started.agent_session = Some("conv".into());
        let terminals = vec![
            terminal("a", "/nowhere/p", 1.0),
            started,
            terminal("c", "/nowhere/p/", 3.0),
        ];

        let named = m
            .resolve(&event("PreToolUse", "s1", "/x", Some("a")), &terminals)
            .unwrap();
        assert_eq!(
            named,
            Resolved {
                terminal: "a".into(),
                link: Some("s1".into())
            }
        );
        let again = m
            .resolve(&event("Stop", "s1", "/x", Some("a")), &terminals)
            .unwrap();
        assert_eq!(again.link, None);

        let by_conv = m
            .resolve(&event("Stop", "conv", "/x", None), &terminals)
            .unwrap();
        assert_eq!(by_conv.terminal, "b");

        // The newest unlinked terminal in the folder; trailing separators do not matter.
        let guessed = m
            .resolve(&event("Stop", "s2", "/nowhere/p", None), &terminals)
            .unwrap();
        assert_eq!(
            guessed,
            Resolved {
                terminal: "c".into(),
                link: Some("s2".into())
            }
        );
        assert!(m
            .resolve(&event("Stop", "s3", "/nowhere/p", None), &terminals)
            .is_none());
        // An exact identity overrules the guess.
        let exact = m
            .resolve(&event("Stop", "s4", "/", Some("c")), &terminals)
            .unwrap();
        assert_eq!(exact.terminal, "c");
        assert_eq!(m.linked("s2"), None);
    }

    #[test]
    fn maps_each_event_to_a_status_and_forgets_an_ended_session() {
        let mut m = Mapper::default();
        m.force_link("copilot-t", "t");
        let terminals = vec![terminal("t", "/q", 1.0)];
        let status = |m: &mut Mapper, name| {
            m.map(&event(name, "copilot-t", "", None), &terminals)
                .map(|(_, s)| s)
        };
        assert_eq!(status(&mut m, "SessionStart"), Some(Status::Running));
        assert_eq!(status(&mut m, "PostToolUseFailure"), Some(Status::Error));
        assert_eq!(status(&mut m, "PermissionRequest"), Some(Status::Waiting));
        assert_eq!(status(&mut m, "Whatever"), None);
        assert_eq!(status(&mut m, "SessionEnd"), Some(Status::Idle));
        assert_eq!(m.linked("copilot-t"), None);
        assert_eq!(Status::Waiting.as_str(), "waiting");
    }
}

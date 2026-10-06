//! What Vorn reads about the coding agents it runs, without starting one.
//!
//! The server answers three questions about the agents before any session
//! exists: which of them are installed ([`detect`]), which conversations each
//! has had in a project ([`history`]), and which models each offers
//! ([`models`]). All three read the agents' own files or ask their CLIs, and
//! none of them touches a session, so a host can answer them on any thread.
//! Each answers exactly as the server's TypeScript of the same purpose does
//! (`agent-detector`, `agent-history`, `agent-model-catalog`); where a module
//! departs from it on input no agent writes, its docs say so.
//!
//! Everything here blocks the thread it runs on: file reads, SQLite queries,
//! and for the model catalog a child process of up to fifteen seconds.

pub mod detect;
pub mod history;
mod js;
pub mod models;
pub mod paths;
mod probe;

pub use probe::{probe, Probe, ProbeContext, Step};

/// The coding agents Vorn launches. A shell is not one: it has no history,
/// models or install check.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum Agent {
    Claude,
    Copilot,
    Codex,
    OpenCode,
    Gemini,
}

impl Agent {
    /// Every agent, in the order the server checks which are installed.
    pub const ALL: [Agent; 5] = [
        Agent::Claude,
        Agent::Copilot,
        Agent::Codex,
        Agent::OpenCode,
        Agent::Gemini,
    ];

    /// The agent's `AiAgentType` name.
    pub fn id(self) -> &'static str {
        match self {
            Agent::Claude => "claude",
            Agent::Copilot => "copilot",
            Agent::Codex => "codex",
            Agent::OpenCode => "opencode",
            Agent::Gemini => "gemini",
        }
    }

    /// The agent an `AiAgentType` names; `None` for `shell` or anything else.
    pub fn from_id(id: &str) -> Option<Agent> {
        Agent::ALL.into_iter().find(|a| a.id() == id)
    }

    /// Whether a past session can be resumed by its own id
    /// (`supportsExactSessionResume`).
    pub fn resumes_exactly(self) -> bool {
        self != Agent::Gemini
    }

    /// What one unit of a session's activity is called
    /// (`getRecentSessionActivityLabel`).
    pub fn activity_label(self) -> &'static str {
        match self {
            Agent::Claude | Agent::Codex => "entry",
            Agent::Copilot => "turn",
            Agent::Gemini => "prompt",
            Agent::OpenCode => "message",
        }
    }

    /// Whether a model can be chosen for it (`supportsModelSelection`).
    pub fn selects_models(self) -> bool {
        self != Agent::Gemini
    }

    /// The command the app launches it with when none is configured
    /// (`DEFAULT_AGENT_COMMANDS`).
    pub fn default_command(self) -> AgentCommand {
        let headless: &[&str] = match self {
            Agent::Claude => &["--dangerously-skip-permissions"],
            Agent::Copilot => &["--allow-all"],
            Agent::Codex => &["-a", "never"],
            Agent::OpenCode => &[],
            Agent::Gemini => &["-y"],
        };
        AgentCommand {
            command: self.id().to_owned(),
            args: Vec::new(),
            headless_args: (self != Agent::OpenCode)
                .then(|| headless.iter().map(|s| (*s).to_owned()).collect()),
            fallback_command: None,
            fallback_args: None,
        }
    }
}

/// How an agent is launched, as configured (`AgentCommandConfig`).
///
/// Whole, though the reads here use only some of it: the model catalog keeps
/// one list per configuration, and a change to any part of it is a new one.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub struct AgentCommand {
    pub command: String,
    pub args: Vec<String>,
    pub headless_args: Option<Vec<String>>,
    pub fallback_command: Option<String>,
    pub fallback_args: Option<Vec<String>>,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn names_each_agent_as_the_app_does() {
        for agent in Agent::ALL {
            assert_eq!(Agent::from_id(agent.id()), Some(agent));
        }
        assert_eq!(Agent::from_id("shell"), None);
        assert_eq!(Agent::from_id("Claude"), None);
    }

    #[test]
    fn opencode_alone_has_no_headless_arguments_by_default() {
        assert_eq!(Agent::OpenCode.default_command().headless_args, None);
        assert_eq!(
            Agent::Codex.default_command().headless_args,
            Some(vec!["-a".to_owned(), "never".to_owned()])
        );
    }
}

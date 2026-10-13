//! Each agent's screen rules: what its screen shows when it is working,
//! waiting for input or idle, as one TOML file per agent in `status/`.
//!
//! Adding or fixing an agent is an edit to its file plus fixtures under
//! `tests/fixtures/status/<agent>/` that show the screens it reads. Where no
//! rule holds, the generic output patterns decide (errors come only from
//! them), and hooks, where an agent has them, decide over both.

use std::sync::OnceLock;

use vorn_analysis::rules::Rules;

use crate::Agent;

/// The shipped rule file's text for `agent`.
pub fn source(agent: Agent) -> &'static str {
    match agent {
        Agent::Claude => include_str!("../status/claude.toml"),
        Agent::Copilot => include_str!("../status/copilot.toml"),
        Agent::Codex => include_str!("../status/codex.toml"),
        Agent::OpenCode => include_str!("../status/opencode.toml"),
        Agent::Gemini => include_str!("../status/gemini.toml"),
    }
}

impl Agent {
    /// The rules this agent's status is read from its screen with.
    pub fn screen_rules(self) -> &'static Rules {
        static RULES: OnceLock<Vec<Rules>> = OnceLock::new();
        let all = RULES.get_or_init(|| {
            Agent::ALL
                .iter()
                .map(|a| {
                    Rules::parse(source(*a))
                        .unwrap_or_else(|e| panic!("status/{}.toml: {e}", a.id()))
                })
                .collect()
        });
        let i = Agent::ALL.iter().position(|a| *a == self).expect("in ALL");
        &all[i]
    }
}

/// The screen rules for an `AiAgentType` name, or `None` for a shell or an
/// agent Vorn does not know.
pub fn rules(agent_type: &str) -> Option<&'static Rules> {
    Agent::from_id(agent_type).map(Agent::screen_rules)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_agent_has_a_rule_file_naming_it() {
        for agent in Agent::ALL {
            assert_eq!(agent.screen_rules().agent(), agent.id());
            assert!(std::ptr::eq(
                rules(agent.id()).unwrap(),
                agent.screen_rules()
            ));
        }
        assert!(rules("shell").is_none());
        assert!(rules("").is_none());
    }
}

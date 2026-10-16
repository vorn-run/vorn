//! What Vorn reads about the coding agents it runs, without starting one.
//!
//! Vorn knows no agent by name. Each agent CLI comes from an adapter: the
//! `[agent]` section of an extension's manifest ([`spec`]), read into an
//! [`Agent`]. Everything here works from that data: which agents are
//! installed ([`detect`]), which conversations each has had in a project
//! ([`history`]), which models each offers ([`models`]), and how each is
//! launched, and a shell with it ([`launch`]), which starts nothing.
//!
//! Each answers exactly as the server's TypeScript of the same purpose did
//! (`agent-detector`, `agent-history`, `agent-model-catalog`, `agent-launch`)
//! for the agents it knew, checked by the corpora recorded from it; where a
//! module departs from it on input no agent writes, its docs say so.
//!
//! Everything here blocks the thread it runs on: file reads, SQLite queries,
//! and for the model catalog a child process of up to fifteen seconds.

pub mod detect;
pub mod history;
mod js;
pub mod launch;
pub mod models;
pub mod paths;
mod probe;
pub mod spec;
pub mod status;

use std::collections::HashMap;
use std::fmt;
use std::hash::{Hash, Hasher};
use std::sync::{Arc, Mutex, OnceLock, PoisonError, RwLock};

use vorn_analysis::rules::Rules;

pub use probe::{probe, Probe, ProbeContext, Step};
pub use spec::{AgentSpec, SpecError};

/// One agent CLI Vorn can run, as its adapter describes it. Cheap to clone;
/// two are the same agent when their ids are.
#[derive(Clone)]
pub struct Agent(Arc<Inner>);

struct Inner {
    spec: AgentSpec,
    rules: Option<&'static Rules>,
}

impl Agent {
    /// The agent `spec` describes, once [`AgentSpec::check`] passed. From
    /// then on its id is one Vorn knows ([`is_known`]) and the variables it
    /// sets in its sessions are stripped from every process Vorn starts.
    pub fn new(spec: AgentSpec) -> Result<Agent, SpecError> {
        spec.check()?;
        let rules = (!spec.status.is_empty())
            .then(|| status::intern(&spec.id, &spec.status))
            .transpose()
            .map_err(|e| SpecError {
                field: "agent.status".to_owned(),
                message: e.to_string(),
            })?;
        if let Some(env) = &spec.env {
            launch::env::add_markers(&env.strip, &env.strip_prefixes);
        }
        know(&spec.id);
        Ok(Agent(Arc::new(Inner { spec, rules })))
    }

    /// Its `AiAgentType` name.
    pub fn id(&self) -> &str {
        &self.0.spec.id
    }

    pub fn spec(&self) -> &AgentSpec {
        &self.0.spec
    }

    /// Whether a past session can be resumed by its own id
    /// (`supportsExactSessionResume`).
    pub fn resumes_exactly(&self) -> bool {
        self.0.spec.resume.is_some()
    }

    /// Whether a fresh session can be pinned to an id chosen beforehand, to
    /// resume it by later (`supportsSessionIdPinning`).
    pub fn pins_session_ids(&self) -> bool {
        self.0
            .spec
            .resume
            .as_ref()
            .is_some_and(|r| !r.pin.is_empty())
    }

    /// What one unit of a session's activity is called
    /// (`getRecentSessionActivityLabel`).
    pub fn activity_label(&self) -> &str {
        self.0
            .spec
            .history
            .as_ref()
            .map_or("session", |h| h.activity.as_str())
    }

    /// Whether a model can be chosen for it (`supportsModelSelection`).
    pub fn selects_models(&self) -> bool {
        self.0.spec.models.is_some()
    }

    /// The rules its status is read from its screen with, if it has any.
    pub fn screen_rules(&self) -> Option<&'static Rules> {
        self.0.rules
    }

    /// The command it is launched with when none is configured
    /// (`DEFAULT_AGENT_COMMANDS`).
    pub fn default_command(&self) -> AgentCommand {
        let launch = &self.0.spec.launch;
        AgentCommand {
            command: launch.command.clone(),
            args: launch.args.clone(),
            headless_args: launch.headless_args.clone(),
        }
    }
}

impl PartialEq for Agent {
    fn eq(&self, other: &Agent) -> bool {
        self.id() == other.id()
    }
}

impl Eq for Agent {}

impl Hash for Agent {
    fn hash<H: Hasher>(&self, state: &mut H) {
        self.id().hash(state);
    }
}

impl fmt::Debug for Agent {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "Agent({})", self.id())
    }
}

/// Every agent id an adapter read in this process has named, on or off.
static KNOWN: RwLock<Vec<String>> = RwLock::new(Vec::new());

/// Records that an adapter names agent `id`, as reading its manifest does.
pub fn know(id: &str) {
    let mut known = KNOWN.write().unwrap_or_else(PoisonError::into_inner);
    if !known.iter().any(|k| k == id) {
        known.push(id.to_owned());
    }
}

/// Whether an adapter Vorn read names agent `id`, switched on or not: a
/// manifest's activation rule naming any other is one this Vorn cannot match.
pub fn is_known(id: &str) -> bool {
    KNOWN
        .read()
        .unwrap_or_else(PoisonError::into_inner)
        .iter()
        .any(|k| k == id)
}

/// The agents a host runs, in the order the launcher lists them. A shell is
/// not one: it has no history, models or install check.
#[derive(Clone, Debug, Default)]
pub struct Agents {
    list: Vec<Agent>,
}

impl Agents {
    /// `list` in its order; a later agent with an id already taken is left
    /// out, so the first adapter to claim an id keeps it.
    pub fn new(list: impl IntoIterator<Item = Agent>) -> Agents {
        let mut kept: Vec<Agent> = Vec::new();
        for agent in list {
            if !kept.contains(&agent) {
                kept.push(agent);
            }
        }
        Agents { list: kept }
    }

    /// The agent an `AiAgentType` names; `None` for `shell` or one no
    /// adapter that is on describes.
    pub fn get(&self, id: &str) -> Option<&Agent> {
        self.list.iter().find(|a| a.id() == id)
    }

    pub fn iter(&self) -> std::slice::Iter<'_, Agent> {
        self.list.iter()
    }

    pub fn is_empty(&self) -> bool {
        self.list.is_empty()
    }

    pub fn len(&self) -> usize {
        self.list.len()
    }

    /// The agent a session starts when none is chosen: the first, or none,
    /// when the launcher opens a shell.
    pub fn first(&self) -> Option<&Agent> {
        self.list.first()
    }
}

impl<'a> IntoIterator for &'a Agents {
    type Item = &'a Agent;
    type IntoIter = std::slice::Iter<'a, Agent>;

    fn into_iter(self) -> Self::IntoIter {
        self.list.iter()
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
}

/// The agent a manifest's text describes, read once per process: the same
/// text gives the same agent. `None` for a manifest with no `[agent]`.
pub fn from_manifest_text(text: &str) -> Result<Option<Agent>, SpecError> {
    static SEEN: OnceLock<Mutex<HashMap<String, Agent>>> = OnceLock::new();
    let seen = SEEN.get_or_init(Mutex::default);
    if let Some(agent) = seen.lock().unwrap_or_else(|e| e.into_inner()).get(text) {
        return Ok(Some(agent.clone()));
    }
    let Some(spec) = AgentSpec::from_manifest(text)? else {
        return Ok(None);
    };
    let agent = Agent::new(spec)?;
    seen.lock()
        .unwrap_or_else(|e| e.into_inner())
        .insert(text.to_owned(), agent.clone());
    Ok(Some(agent))
}

#[cfg(test)]
pub(crate) mod testing {
    //! Agents for this crate's own tests, written as an adapter would write them.

    use super::*;

    /// The agent the `[agent]` TOML in `section` describes.
    pub fn agent(section: &str) -> Agent {
        from_manifest_text(section)
            .expect("the test adapter reads")
            .expect("the test adapter has an [agent] section")
    }

    /// The smallest agent: a command and nothing else.
    pub fn bare(id: &str) -> Agent {
        agent(&format!(
            "[agent]\nid = \"{id}\"\nname = \"{id}\"\n[agent.launch]\ncommand = \"{id}\"\n"
        ))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn keeps_the_first_agent_claiming_an_id() {
        let a = testing::bare("one");
        let b = testing::bare("two");
        let agents = Agents::new([a.clone(), b, testing::bare("one")]);
        assert_eq!(agents.len(), 2);
        assert_eq!(agents.get("one"), Some(&a));
        assert!(agents.get("shell").is_none());
        assert!(is_known("one") && !is_known("never-read"));
        assert_eq!(agents.first(), Some(&a));
        assert!(Agents::default().first().is_none());
    }

    #[test]
    fn reads_what_the_spec_offers() {
        let a = testing::bare("plain");
        assert!(!a.resumes_exactly() && !a.pins_session_ids() && !a.selects_models());
        assert_eq!(a.activity_label(), "session");
        assert!(a.screen_rules().is_none());
        assert_eq!(a.default_command().headless_args, None);
        let b = testing::agent(
            "[agent]\nid = \"rich\"\nname = \"Rich\"\n[agent.launch]\ncommand = \"rich\"\nheadless_args = [\"-y\"]\n\
             [agent.resume]\nargs = [\"--resume\", \"{id}\"]\npin = [\"--id\", \"{id}\"]\n\
             [agent.models]\n[agent.history]\nactivity = \"turn\"\n[agent.history.jsonl]\n\
             file = \"~/h\"\nsession = \"s\"\nproject = \"p\"\ntime = \"t\"\ntitle = \"d\"\n\
             [[agent.status]]\nstatus = \"idle\"\nany = [\"> \"]\n",
        );
        assert!(b.resumes_exactly() && b.pins_session_ids() && b.selects_models());
        assert_eq!(b.activity_label(), "turn");
        assert_eq!(b.screen_rules().map(Rules::agent), Some("rich"));
        assert_eq!(
            b.default_command().headless_args,
            Some(vec!["-y".to_owned()])
        );
    }
}

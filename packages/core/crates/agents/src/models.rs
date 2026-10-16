//! The models an agent offers, asked of its own CLI.
//!
//! [`parse_choices`] reads each CLI's answer into one shape, and [`Catalog`]
//! keeps one list per agent, project and command configuration for five
//! minutes: a request inside that answers from the list, a later one answers
//! with the old list marked stale while a fresh one is asked for, and an
//! explicit refresh waits for the fresh one. The asking itself is
//! [`crate::probe`], which a host runs with the command it resolved.

use std::collections::HashMap;
use std::fmt;
use std::path::PathBuf;
use std::sync::{Arc, Condvar, Mutex, MutexGuard};
use std::time::Duration;

use serde_json::{json, Map, Value};

use crate::spec::{at, ChoiceMap, Keep};
use crate::{js, Agent, AgentCommand};

/// How long a list answers without being asked for again.
pub const CACHE_TTL: Duration = Duration::from_secs(5 * 60);

/// One model, as the model picker shows it (`AgentModelChoice`).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ModelChoice {
    pub id: String,
    pub label: String,
    pub description: Option<String>,
}

impl ModelChoice {
    pub fn to_json(&self) -> Value {
        let mut out = Map::new();
        out.insert("id".into(), Value::String(self.id.clone()));
        out.insert("label".into(), Value::String(self.label.clone()));
        if let Some(d) = &self.description {
            out.insert("description".into(), Value::String(d.clone()));
        }
        Value::Object(out)
    }
}

/// Why an agent's models could not be listed. Each message is what the
/// picker shows.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum DiscoveryError {
    /// The command is a wrapper with its own flags, which a model cannot ride.
    Wrapper,
    /// The command is on no directory of PATH.
    NotInstalled(String),
    CouldNotStart,
    ConnectionClosed,
    TimedOut,
    TooMuchOutput,
    /// The CLI answered something this does not read.
    Unsupported,
    /// The CLI ended without answering.
    Failed,
    /// An answer that is not a model list at all.
    InvalidCatalog,
}

impl fmt::Display for DiscoveryError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            DiscoveryError::Wrapper => f.write_str(
                "A model needs a command that is one executable. Move the wrapper and its flags into agent arguments, or use the configured default.",
            ),
            DiscoveryError::NotInstalled(command) => {
                write!(f, "{command} is not installed on this machine.")
            }
            DiscoveryError::CouldNotStart => {
                f.write_str("Could not start the configured agent. Check its executable.")
            }
            DiscoveryError::ConnectionClosed => f.write_str("Agent discovery connection closed."),
            DiscoveryError::TimedOut => {
                f.write_str("Model discovery timed out. Refresh to retry or type a model id.")
            }
            DiscoveryError::TooMuchOutput => f.write_str("Model catalog exceeded the output limit."),
            DiscoveryError::Unsupported => f.write_str(
                "The agent returned an unsupported model response. Update the CLI or type a model id.",
            ),
            DiscoveryError::Failed => f.write_str(
                "Agent model discovery failed. Check the CLI installation and sign-in, then refresh.",
            ),
            DiscoveryError::InvalidCatalog => f.write_str("Invalid model catalog."),
        }
    }
}

impl std::error::Error for DiscoveryError {}

/// The first of `paths` that is set and not null.
fn first_set<'a>(entry: &'a Value, paths: &[String]) -> Option<&'a Value> {
    paths
        .iter()
        .find_map(|p| at(entry, p).filter(|v| !v.is_null()))
}

/// `template` read against `entry`: a field's text, or `{field}`s filled
/// in; `None` unless every field it names is non-empty text.
fn text_of(entry: &Value, template: &str) -> Option<String> {
    if !template.contains('{') {
        return at(entry, template)
            .and_then(Value::as_str)
            .filter(|s| !s.is_empty())
            .map(str::to_owned);
    }
    let mut out = String::new();
    let mut rest = template;
    while let Some(open) = rest.find('{') {
        let close = open + rest[open..].find('}')?;
        out.push_str(&rest[..open]);
        let value = at(entry, &rest[open + 1..close])
            .and_then(Value::as_str)
            .filter(|s| !s.is_empty())?;
        out.push_str(value);
        rest = &rest[close + 1..];
    }
    out.push_str(rest);
    Some(out)
}

/// Whether `entry` is one the CLI lists but does not offer.
fn skipped(entry: &Value, map: &ChoiceMap) -> bool {
    map.skip.iter().any(|rule| {
        let found = at(entry, &rule.path);
        rule.is
            .as_ref()
            .is_some_and(|is| found.is_some_and(|v| same(v, is)))
            || rule
                .is_not
                .as_ref()
                .is_some_and(|not| found.is_some_and(|v| !same(v, not)))
    })
}

/// Equal as JSON, numbers by value.
pub(crate) fn same(a: &Value, b: &Value) -> bool {
    match (a.as_f64(), b.as_f64()) {
        (Some(x), Some(y)) => x == y,
        _ => a == b,
    }
}

/// A CLI's listed entries, normalised to one shape: entries it does not
/// offer dropped, a repeated id keeping its first place and its last value.
pub fn parse_choices(map: &ChoiceMap, value: &Value) -> Result<Vec<ModelChoice>, DiscoveryError> {
    let Value::Array(items) = value else {
        return Err(DiscoveryError::InvalidCatalog);
    };
    let mut choices: Vec<ModelChoice> = Vec::new();
    let mut seen: HashMap<String, usize> = HashMap::new();
    for item in items {
        if !item.is_object() || skipped(item, map) {
            continue;
        }
        let Some(id) = first_set(item, &map.id)
            .and_then(Value::as_str)
            .filter(|s| !s.is_empty())
        else {
            continue;
        };
        let label = first_set(item, &map.label)
            .and_then(Value::as_str)
            .unwrap_or(id);
        let description = map.description.iter().find_map(|t| text_of(item, t));
        let choice = ModelChoice {
            id: id.to_owned(),
            label: label.to_owned(),
            description,
        };
        match seen.get(id) {
            Some(&i) => choices[i] = choice,
            None => {
                seen.insert(id.to_owned(), choices.len());
                choices.push(choice);
            }
        }
    }
    Ok(choices)
}

/// A listing that prints one model id per line among other text: each
/// line matching `pattern`, once, as its own label.
pub fn line_choices(pattern: &regex::Regex, text: &str) -> Vec<ModelChoice> {
    let mut seen = std::collections::HashSet::new();
    text.split('\n')
        .map(|line| js::trim(line.strip_suffix('\r').unwrap_or(line)))
        .filter(|line| pattern.is_match(line))
        .filter(|id| seen.insert(*id))
        .map(|id| ModelChoice {
            id: id.to_owned(),
            label: id.to_owned(),
            description: None,
        })
        .collect()
}

/// Only the configured arguments that decide which models a CLI can see;
/// a prompt or a mode would start work.
pub fn discovery_arguments(keep: &Keep, configured: &[String]) -> Vec<String> {
    let mut args = Vec::new();
    let mut i = 0;
    while i < configured.len() {
        let arg = &configured[i];
        let next = configured.get(i + 1).filter(|n| !n.is_empty());
        if let (true, Some(next)) = (keep.paired.contains(arg), next) {
            args.push(arg.clone());
            args.push(next.clone());
            i += 1;
        } else if keep
            .paired
            .iter()
            .any(|flag| arg.starts_with(&format!("{flag}=")))
            || keep.flags.contains(arg)
        {
            args.push(arg.clone());
        }
        i += 1;
    }
    args
}

/// A model rides the arguments, so the command must be one executable. A
/// command of several words is still one when it is the path of a file,
/// spaces and all.
pub fn check_model_command(command: &str) -> Result<(), DiscoveryError> {
    crate::launch::model::check_model_command(command, true).map_err(|_| DiscoveryError::Wrapper)
}

/// What the picker asked for (`AgentModelRequest`), read by the host.
#[derive(Clone, Debug)]
pub struct ModelRequest {
    /// `None` for an agent type that has no models to choose: a shell, or a
    /// name no adapter that is on describes.
    pub agent: Option<Agent>,
    /// The project, absolute; empty when none was given.
    pub project_path: String,
    /// The project is on a remote host, whose CLIs are not asked.
    pub remote: bool,
    pub refresh: bool,
}

/// The catalog's answer (`AgentModelCatalog`).
#[derive(Clone, Debug, PartialEq)]
pub enum CatalogAnswer {
    Ready {
        choices: Vec<ModelChoice>,
        fetched_at: u64,
    },
    /// The last list, while or after a fresh one is asked for; `error` is
    /// why the last asking failed, if it did.
    Stale {
        choices: Vec<ModelChoice>,
        fetched_at: u64,
        error: Option<String>,
    },
    Unavailable {
        error: String,
    },
}

impl CatalogAnswer {
    fn unavailable(error: &str) -> CatalogAnswer {
        CatalogAnswer::Unavailable {
            error: error.to_owned(),
        }
    }

    /// The object the server sends, in its key order.
    pub fn to_json(&self) -> Value {
        let list = |c: &[ModelChoice]| Value::Array(c.iter().map(ModelChoice::to_json).collect());
        match self {
            CatalogAnswer::Ready {
                choices,
                fetched_at,
            } => json!({ "choices": list(choices), "status": "ready", "fetchedAt": fetched_at }),
            CatalogAnswer::Stale {
                choices,
                fetched_at,
                error,
            } => {
                let mut out =
                    json!({ "choices": list(choices), "status": "stale", "fetchedAt": fetched_at });
                if let Some(error) = error {
                    out["error"] = Value::String(error.clone());
                }
                out
            }
            CatalogAnswer::Unavailable { error } => {
                json!({ "choices": [], "status": "unavailable", "error": error })
            }
        }
    }
}

/// One list per agent, project directory and command configuration.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
struct Key {
    /// The agent's id.
    agent: String,
    project: PathBuf,
    command: AgentCommand,
}

#[derive(Debug, Default)]
struct State {
    choices: Option<Vec<ModelChoice>>,
    fetched_at: u64,
    error: Option<String>,
    asking: bool,
    /// How many askings have finished, and what the last one answered.
    finished: u64,
    answer: Option<CatalogAnswer>,
}

#[derive(Debug, Default)]
struct Entry {
    state: Mutex<State>,
    settled: Condvar,
}

impl Entry {
    fn lock(&self) -> MutexGuard<'_, State> {
        self.state.lock().unwrap_or_else(|e| e.into_inner())
    }
}

/// The lists, kept while the host runs.
#[derive(Debug, Default)]
pub struct Catalog {
    entries: Mutex<HashMap<Key, Arc<Entry>>>,
}

impl Catalog {
    /// Answers `request` for an agent configured as `command`. When a list
    /// has to be asked for, `discover` asks on a thread of its own; this waits
    /// for it only when there is no list to answer with meanwhile or the
    /// request is a refresh. `now` is the time in milliseconds since the
    /// epoch.
    pub fn list<D>(
        &self,
        request: &ModelRequest,
        command: &AgentCommand,
        now: impl Fn() -> u64 + Send + 'static,
        discover: D,
    ) -> CatalogAnswer
    where
        D: FnOnce() -> Result<Vec<ModelChoice>, DiscoveryError> + Send + 'static,
    {
        let Some(agent) = request.agent.clone().filter(Agent::selects_models) else {
            return CatalogAnswer::unavailable("Model selection is unavailable for this agent.");
        };
        if request.remote {
            return CatalogAnswer::unavailable(
                "Models on a remote host cannot be listed; use the default or type a model id.",
            );
        }
        if request.project_path.is_empty() || request.project_path.contains("{{") {
            return CatalogAnswer::unavailable(
                "Choose a project to list models, or type a model id.",
            );
        }
        let key = Key {
            agent: agent.id().to_owned(),
            project: PathBuf::from(crate::paths::normalize_lexically(&request.project_path)),
            command: command.clone(),
        };
        let entry = Arc::clone(
            self.entries
                .lock()
                .unwrap_or_else(|e| e.into_inner())
                .entry(key)
                .or_default(),
        );
        let mut state = entry.lock();
        let fresh = state.choices.is_some()
            && now().saturating_sub(state.fetched_at) < CACHE_TTL.as_millis() as u64;
        if !request.refresh && fresh {
            return CatalogAnswer::Ready {
                choices: state.choices.clone().unwrap_or_default(),
                fetched_at: state.fetched_at,
            };
        }
        let waiting_for = state.finished;
        if !state.asking {
            state.asking = true;
            let entry = Arc::clone(&entry);
            std::thread::spawn(move || {
                let found = discover();
                let mut state = entry.lock();
                let answer = match found {
                    Ok(choices) => {
                        state.choices = Some(choices.clone());
                        state.fetched_at = now();
                        state.error = None;
                        CatalogAnswer::Ready {
                            choices,
                            fetched_at: state.fetched_at,
                        }
                    }
                    Err(err) => {
                        let error = err.to_string();
                        state.error = Some(error.clone());
                        match &state.choices {
                            None => CatalogAnswer::Unavailable { error },
                            Some(choices) => CatalogAnswer::Stale {
                                choices: choices.clone(),
                                fetched_at: state.fetched_at,
                                error: Some(error),
                            },
                        }
                    }
                };
                state.asking = false;
                state.finished += 1;
                state.answer = Some(answer);
                drop(state);
                entry.settled.notify_all();
            });
        }
        if !request.refresh {
            if let Some(choices) = &state.choices {
                return CatalogAnswer::Stale {
                    choices: choices.clone(),
                    fetched_at: state.fetched_at,
                    error: state.error.clone(),
                };
            }
        }
        while state.finished == waiting_for {
            state = entry.settled.wait(state).unwrap_or_else(|e| e.into_inner());
        }
        state
            .answer
            .clone()
            .unwrap_or_else(|| CatalogAnswer::unavailable("Could not list models."))
    }
}

#[cfg(test)]
mod tests {
    use std::sync::atomic::{AtomicU64, AtomicUsize, Ordering};

    use super::*;

    fn choice(id: &str, label: &str, description: Option<&str>) -> ModelChoice {
        ModelChoice {
            id: id.into(),
            label: label.into(),
            description: description.map(Into::into),
        }
    }

    fn map(toml_text: &str) -> ChoiceMap {
        toml::from_str(toml_text).unwrap()
    }

    /// The fields and skips each shape of answer the shipped adapters read.
    const HIDDEN: &str = "skip = [{ path = \"hidden\", is = true }, { path = \"policy.state\", is = \"disabled\" }]\n";

    #[test]
    fn normalizes_each_shape_of_answer_and_drops_unusable_entries() {
        assert_eq!(
            parse_choices(
                &map(&format!("id = [\"value\"]\n{HIDDEN}")),
                &json!([{ "value": "opus[1m]", "resolvedModel": "versioned", "displayName": "Opus" }])
            ),
            Ok(vec![choice("opus[1m]", "Opus", None)])
        );
        assert_eq!(
            parse_choices(
                &map(&format!("id = [\"model\", \"id\"]\n{HIDDEN}")),
                &json!([
                    { "id": "internal", "model": "cli-id", "displayName": "Model" },
                    { "id": "fallback", "model": null },
                    { "id": "hidden", "hidden": true },
                    { "id": "off", "policy": { "state": "disabled" } },
                    { "id": "", "name": "empty" },
                    "not an object",
                    { "id": "cli-id", "name": "Again", "description": "" }
                ])
            ),
            Ok(vec![
                choice("cli-id", "Again", None),
                choice("fallback", "fallback", None)
            ])
        );
        let usage =
            map(&format!(
            "id = [\"modelId\"]\ndescription = [\"{{_meta.usage}} usage\", \"description\"]\n{}",
            HIDDEN.replace("]\n", ", { path = \"_meta.enablement\", is_not = \"enabled\" }]\n")
        ));
        assert_eq!(
            parse_choices(
                &usage,
                &json!([
                    { "modelId": "auto", "name": "Auto", "description": "Let it pick" },
                    { "modelId": "opus", "name": "Opus",
                      "_meta": { "usage": "15x", "enablement": "enabled" } },
                    { "modelId": "blocked", "_meta": { "enablement": "policy_disabled" } },
                    { "modelId": "nulled", "_meta": { "enablement": null } }
                ])
            ),
            Ok(vec![
                choice("auto", "Auto", Some("Let it pick")),
                choice("opus", "Opus", Some("15x usage"))
            ])
        );
        let provider_model = regex::Regex::new(r"^[^\s/]+/\S+$").unwrap();
        assert_eq!(
            line_choices(
                &provider_model,
                "provider/model\r\nprovider/model\nother/model\nnoise\n a/b c\n/x\n"
            ),
            vec![
                choice("provider/model", "provider/model", None),
                choice("other/model", "other/model", None)
            ]
        );
        assert_eq!(
            parse_choices(&ChoiceMap::default(), &json!({})),
            Err(DiscoveryError::InvalidCatalog)
        );
    }

    #[test]
    fn keeps_only_the_arguments_that_select_models() {
        let configured: Vec<String> = [
            "--profile",
            "work",
            "-c",
            "model=x",
            "--oss",
            "--config=a=b",
            "-m",
            "gpt",
            "-p",
        ]
        .map(String::from)
        .to_vec();
        let keep = Keep {
            paired: ["-c", "--config", "-p", "--profile", "--local-provider"]
                .map(String::from)
                .to_vec(),
            flags: vec!["--oss".to_owned()],
        };
        assert_eq!(
            discovery_arguments(&keep, &configured),
            [
                "--profile",
                "work",
                "-c",
                "model=x",
                "--oss",
                "--config=a=b"
            ]
        );
        assert!(discovery_arguments(&Keep::default(), &configured).is_empty());
    }

    #[test]
    fn a_model_needs_one_executable() {
        assert_eq!(check_model_command("agent"), Ok(()));
        assert_eq!(check_model_command("'/opt/my tools/agent'"), Ok(()));
        for wrapper in [
            "npx -y agent",
            "env A=1 agent",
            "agent; rm",
            "$(which agent)",
            "\"unterminated",
            "trailing\\",
            "",
        ] {
            assert_eq!(
                check_model_command(wrapper),
                Err(DiscoveryError::Wrapper),
                "{wrapper}"
            );
        }
        let dir = tempfile::tempdir().unwrap();
        let spaced = dir.path().join("my agent");
        std::fs::write(&spaced, "").unwrap();
        assert_eq!(check_model_command(spaced.to_str().unwrap()), Ok(()));
    }

    fn listing() -> Agent {
        crate::testing::agent(
            "[agent]\nid = \"lister\"\nname = \"L\"\n[agent.launch]\ncommand = \"lister\"\n[agent.models]\n",
        )
    }

    fn request(agent: Agent) -> ModelRequest {
        ModelRequest {
            agent: Some(agent),
            project_path: "/project".into(),
            remote: false,
            refresh: false,
        }
    }

    #[test]
    fn answers_unavailable_without_asking_what_cannot_be_asked() {
        let catalog = Catalog::default();
        let command = listing().default_command();
        let never = || -> Result<Vec<ModelChoice>, DiscoveryError> { panic!("asked") };
        let mut r = request(crate::testing::bare("no-models"));
        assert!(matches!(
            catalog.list(&r, &command, || 0, never),
            CatalogAnswer::Unavailable { .. }
        ));
        r = request(listing());
        r.remote = true;
        assert!(matches!(
            catalog.list(&r, &command, || 0, never),
            CatalogAnswer::Unavailable { .. }
        ));
        r = request(listing());
        r.project_path = "{{context.path}}".into();
        assert_eq!(
            catalog.list(&r, &command, || 0, never).to_json(),
            json!({ "choices": [], "status": "unavailable",
                    "error": "Choose a project to list models, or type a model id." })
        );
    }

    #[test]
    fn caches_then_refreshes_and_keeps_the_last_list_on_failure() {
        let catalog = Catalog::default();
        let command = listing().default_command();
        let asked = Arc::new(AtomicUsize::new(0));
        let clock = Arc::new(AtomicU64::new(1_000));
        let now = {
            let clock = Arc::clone(&clock);
            move || clock.load(Ordering::SeqCst)
        };
        let ok = |asked: &Arc<AtomicUsize>| {
            let asked = Arc::clone(asked);
            move || {
                asked.fetch_add(1, Ordering::SeqCst);
                Ok(vec![choice("m", "Model", None)])
            }
        };
        let first = catalog.list(&request(listing()), &command, now.clone(), ok(&asked));
        assert_eq!(
            first,
            CatalogAnswer::Ready {
                choices: vec![choice("m", "Model", None)],
                fetched_at: 1_000
            }
        );
        catalog.list(&request(listing()), &command, now.clone(), ok(&asked));
        assert_eq!(asked.load(Ordering::SeqCst), 1);

        // Another project or configuration is another list.
        let mut other = request(listing());
        other.project_path = "/project/../another".into();
        catalog.list(&other, &command, now.clone(), ok(&asked));
        let mut profiled = command.clone();
        profiled.args = vec!["--profile".into(), "work".into()];
        catalog.list(&request(listing()), &profiled, now.clone(), ok(&asked));
        assert_eq!(asked.load(Ordering::SeqCst), 3);

        // Past the TTL a failure answers the old list as stale.
        clock.store(1_000 + 300_001, Ordering::SeqCst);
        let failing = || Err(DiscoveryError::Failed);
        let mut refresh = request(listing());
        refresh.refresh = true;
        let failed = catalog.list(&refresh, &command, now.clone(), failing);
        assert_eq!(
            failed,
            CatalogAnswer::Stale {
                choices: vec![choice("m", "Model", None)],
                fetched_at: 1_000,
                error: Some(DiscoveryError::Failed.to_string())
            }
        );
        // Not a refresh: the stale list at once, with the last error, and a
        // fresh one asked for behind it.
        let stale = catalog.list(&request(listing()), &command, now.clone(), ok(&asked));
        assert!(matches!(stale, CatalogAnswer::Stale { error: Some(_), .. }));
        let mut tries = 0;
        while catalog.list(&request(listing()), &command, now.clone(), failing)
            != (CatalogAnswer::Ready {
                choices: vec![choice("m", "Model", None)],
                fetched_at: 301_001,
            })
        {
            tries += 1;
            assert!(
                tries < 500,
                "the refresh behind the stale answer never landed"
            );
            std::thread::sleep(Duration::from_millis(2));
        }
    }

    #[test]
    fn a_failure_with_no_list_is_unavailable_with_its_reason() {
        let catalog = Catalog::default();
        let answer = catalog.list(
            &request(listing()),
            &listing().default_command(),
            || 5,
            || Err(DiscoveryError::NotInstalled("lister".into())),
        );
        assert_eq!(
            answer.to_json(),
            json!({ "choices": [], "status": "unavailable",
                    "error": "lister is not installed on this machine." })
        );
    }

    #[test]
    fn requests_at_once_share_one_asking() {
        let catalog = Arc::new(Catalog::default());
        let asked = Arc::new(AtomicUsize::new(0));
        let threads: Vec<_> = (0..6)
            .map(|_| {
                let (catalog, asked) = (Arc::clone(&catalog), Arc::clone(&asked));
                std::thread::spawn(move || {
                    catalog.list(
                        &request(listing()),
                        &listing().default_command(),
                        || 7,
                        move || {
                            asked.fetch_add(1, Ordering::SeqCst);
                            std::thread::sleep(Duration::from_millis(30));
                            Ok(Vec::new())
                        },
                    )
                })
            })
            .collect();
        for t in threads {
            assert_eq!(
                t.join().unwrap(),
                CatalogAnswer::Ready {
                    choices: Vec::new(),
                    fetched_at: 7
                }
            );
        }
        assert_eq!(asked.load(Ordering::SeqCst), 1);
    }
}

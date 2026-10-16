//! An agent adapter: the `[agent]` section of a `vorn:extension` manifest,
//! which tells Vorn everything it needs to run one agent CLI as data.
//!
//! The section says how the CLI is found and launched, how a past
//! conversation is resumed or a fresh one pinned to an id, which models it
//! offers and how to ask it for them, where it keeps past conversations,
//! what its screen shows in each state, and how its hooks are installed.
//! Nothing here starts a process or reads a file; [`crate::launch`],
//! [`crate::models`], [`crate::history`] and the hook installers read a spec
//! and do that.
//!
//! Templates in strings: `{id}` is a conversation id, `{prompt}` a first
//! prompt, `{cwd}` the directory a model listing runs in. Paths start with
//! `~` (the home directory), `{data}` (`$XDG_DATA_HOME`, `~/.local/share`,
//! or `%LOCALAPPDATA%` on Windows) or `{env:NAME|fallback}` (a variable, or
//! the fallback when it is unset or empty).

use std::fmt;
use std::path::{Path, PathBuf};

use serde::Deserialize;
use serde_json::{Map, Value};
use vorn_analysis::rules::{RuleSpec, Rules};

/// An `[agent]` section as written.
#[derive(Clone, Debug, PartialEq, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct AgentSpec {
    /// The agent type sessions carry (`AiAgentType`): lowercase letters,
    /// digits and dashes. `shell` is taken.
    pub id: String,
    /// What the launcher and Settings call it, as `Acme Code`.
    pub name: String,
    /// What menus, the palette and the task board call it; `name` when absent.
    #[serde(default)]
    pub label: Option<String>,
    /// The shortest name, as a workflow step's; `label` when absent.
    #[serde(default)]
    pub short_name: Option<String>,
    /// One line for Settings.
    #[serde(default)]
    pub description: Option<String>,
    /// The mark, an SVG file in the package.
    #[serde(default)]
    pub icon: Option<String>,
    /// One character standing for the mark where only text is drawn.
    #[serde(default)]
    pub mark: Option<String>,
    /// The card's accent, `#rrggbb`, and its tint behind the mark, `#rrggbbaa`.
    #[serde(default)]
    pub color: Option<String>,
    #[serde(default)]
    pub tint: Option<String>,
    /// Where to read about it, and the command that installs it.
    #[serde(default)]
    pub url: Option<String>,
    #[serde(default)]
    pub install: Option<String>,
    pub launch: Launch,
    #[serde(default)]
    pub resume: Option<Resume>,
    #[serde(default)]
    pub headless: Option<Headless>,
    #[serde(default)]
    pub models: Option<Models>,
    #[serde(default)]
    pub history: Option<History>,
    /// Its screen rules, first match wins (`[[agent.status]]`).
    #[serde(default)]
    pub status: Vec<RuleSpec>,
    #[serde(default)]
    pub hooks: Option<Hooks>,
    #[serde(default)]
    pub env: Option<EnvMarkers>,
    #[serde(default)]
    pub usage: Option<Usage>,
    #[serde(default)]
    pub mcp: Option<McpSetup>,
}

/// How it is started.
#[derive(Clone, Debug, PartialEq, Eq, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Launch {
    /// The program, looked for on PATH.
    pub command: String,
    #[serde(default)]
    pub args: Vec<String>,
    /// The arguments a headless run uses in place of `args`; none when absent.
    #[serde(default)]
    pub headless_args: Option<Vec<String>>,
    /// How a first prompt rides the interactive line, as `["-i", "{prompt}"]`.
    #[serde(default = "positional_prompt")]
    pub prompt: Vec<String>,
}

fn positional_prompt() -> Vec<String> {
    vec![PROMPT.to_owned()]
}

/// Where words that resume a conversation go on the line.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum At {
    /// After everything configured.
    #[default]
    End,
    /// As the first argument, before everything configured: a subcommand.
    Start,
}

/// How a past conversation is continued. Absent, the agent cannot be sent
/// back to one by id: resuming lets it choose.
#[derive(Clone, Debug, PartialEq, Eq, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Resume {
    /// What resumes conversation `{id}`, as `["--resume", "{id}"]`.
    pub args: Vec<String>,
    #[serde(default)]
    pub at: At,
    /// What starts a fresh conversation under an id chosen beforehand, as
    /// `["--session-id", "{id}"]`; absent when the CLI takes none.
    #[serde(default)]
    pub pin: Vec<String>,
    /// The words a configured command may already carry that pick a
    /// conversation, removed where they can be proven before Vorn adds its own.
    #[serde(default)]
    pub selectors: Selectors,
    /// Reads the conversation a fresh session took from the agent's own
    /// database, for an agent that cannot be told one.
    #[serde(default)]
    pub capture: Option<Capture>,
}

/// Flags and a subcommand that select a conversation on a configured line.
#[derive(Clone, Debug, Default, PartialEq, Eq, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Selectors {
    /// `--flag value` and `--flag=value`.
    #[serde(default)]
    pub with_value: Vec<String>,
    /// Flags without a value; whatever follows is not theirs.
    #[serde(default)]
    pub bare: Vec<String>,
    /// A subcommand in the first argument position, with the id or `--last`
    /// after it.
    #[serde(default)]
    pub subcommand: Option<String>,
}

/// One query for the newest conversation in a directory.
#[derive(Clone, Debug, PartialEq, Eq, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Capture {
    /// The SQLite file, a path template.
    pub db: String,
    /// Answers one row whose first column is the id; `?1` is the directory,
    /// lowercase with forward slashes and no trailing one.
    pub query: String,
}

/// How it runs headless: spawned directly, the prompt on its stdin.
#[derive(Clone, Debug, Default, PartialEq, Eq, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Headless {
    /// What ends the arguments when there is a prompt, which goes to stdin.
    #[serde(default)]
    pub prompt: Vec<String>,
    /// What ends them when there is none, so it does not wait on stdin.
    #[serde(default)]
    pub no_prompt: Vec<String>,
    /// What resumes `{id}`, added after the arguments.
    #[serde(default)]
    pub resume: Vec<String>,
    /// What ends the arguments in place of the prompt's when resuming
    /// `{id}`; its prompt, or an empty one, goes to stdin.
    #[serde(default)]
    pub resume_tail: Vec<String>,
    /// Its package's service reads what a headless run prints, line by line
    /// (`agent.stream`), into the conversation it took and hook events.
    #[serde(default)]
    pub stream: bool,
}

/// The model a session runs. Absent, no model can be chosen.
#[derive(Clone, Debug, PartialEq, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Models {
    /// The flag the chosen model is passed with.
    #[serde(default = "model_flag")]
    pub flag: String,
    /// Flags taking a model, removed from the configured arguments first:
    /// `--flag value`, `--flag=value`, and `-xvalue` for a one-letter one.
    #[serde(default)]
    pub selectors: Vec<String>,
    /// A config override that selects the model, as `-c model=x`.
    #[serde(default)]
    pub config: Option<ConfigSelector>,
    /// Ids are `provider/model`; the picker groups them by provider.
    #[serde(default)]
    pub group: bool,
    /// How the CLI is asked for its models; absent, only a typed id.
    #[serde(default)]
    pub list: Option<ModelList>,
}

fn model_flag() -> String {
    "--model".to_owned()
}

/// A config flag whose `key=` value is a model.
#[derive(Clone, Debug, PartialEq, Eq, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ConfigSelector {
    pub flags: Vec<String>,
    pub key: String,
}

/// Asking the CLI for its models without starting a conversation.
#[derive(Clone, Debug, PartialEq, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ModelList {
    /// Added after the configured arguments that are kept.
    pub args: Vec<String>,
    /// The configured arguments that decide which models it sees; any
    /// other would start work.
    #[serde(default)]
    pub keep: Keep,
    /// A JSON-lines exchange over stdin and stdout, in order.
    #[serde(default)]
    pub steps: Vec<Step>,
    /// With no steps: stdin is closed at once and, when it exits with 0,
    /// each line of stdout matching this pattern is a model id.
    #[serde(default)]
    pub lines: Option<String>,
    /// How one entry of the answer reads as a model.
    #[serde(default)]
    pub choice: ChoiceMap,
}

/// Configured arguments kept for a listing.
#[derive(Clone, Debug, Default, PartialEq, Eq, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Keep {
    /// Flags kept with the value after them, or joined by `=`.
    #[serde(default)]
    pub paired: Vec<String>,
    /// Flags kept alone.
    #[serde(default)]
    pub flags: Vec<String>,
}

/// One turn of a listing exchange: messages to send, then the line to wait
/// for. Paths are dotted, as `result.models`.
#[derive(Clone, Debug, PartialEq, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Step {
    /// Written one per line; `"{cwd}"` is the directory it runs in.
    #[serde(default)]
    pub send: Vec<Value>,
    /// The fields of the line answering this step; `"{id}"` is the `id` of
    /// the last message sent.
    pub reply: Map<String, Value>,
    /// A field that fails the listing when it holds anything.
    #[serde(default)]
    pub fail: Option<String>,
    /// Fields that fail the listing unless they hold these values.
    #[serde(default)]
    pub require: Map<String, Value>,
    /// The model list in the reply; the exchange ends with the step that has one.
    #[serde(default)]
    pub models: Option<String>,
    /// The next page's cursor in the reply: the last message is sent again
    /// with its `id` one higher and the cursor at `cursor`.
    #[serde(default)]
    pub next: Option<String>,
    #[serde(default)]
    pub cursor: Option<String>,
}

/// Which fields of a listed entry are its id, label and description, and
/// which entries are not offered.
#[derive(Clone, Debug, PartialEq, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ChoiceMap {
    /// The first of these that is set is the id.
    #[serde(default = "id_paths")]
    pub id: Vec<String>,
    #[serde(default = "label_paths")]
    pub label: Vec<String>,
    /// The first that reads as text; an entry may be a template, as
    /// `"{_meta.usage} usage"`, which reads only when each field it names does.
    #[serde(default = "description_paths")]
    pub description: Vec<String>,
    #[serde(default)]
    pub skip: Vec<Skip>,
}

impl Default for ChoiceMap {
    fn default() -> ChoiceMap {
        ChoiceMap {
            id: id_paths(),
            label: label_paths(),
            description: description_paths(),
            skip: Vec::new(),
        }
    }
}

fn id_paths() -> Vec<String> {
    vec!["id".to_owned()]
}

fn label_paths() -> Vec<String> {
    vec!["displayName".to_owned(), "name".to_owned()]
}

fn description_paths() -> Vec<String> {
    vec!["description".to_owned()]
}

/// An entry not offered: one whose field is `is`, or whose field is there
/// and is anything but `is_not`.
#[derive(Clone, Debug, PartialEq, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Skip {
    pub path: String,
    #[serde(default)]
    pub is: Option<Value>,
    #[serde(default)]
    pub is_not: Option<Value>,
}

/// Where its past conversations are, for Recent sessions and resuming.
#[derive(Clone, Debug, PartialEq, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct History {
    /// What one unit of a conversation is called, as `turn` or `prompt`.
    pub activity: String,
    /// The longest title listed, in UTF-16 units; uncut when absent.
    #[serde(default)]
    pub title_limit: Option<usize>,
    /// Orders sessions of different agents at the same moment, lower
    /// first; the launcher's order among equal ranks.
    #[serde(default)]
    pub rank: u32,
    #[serde(default)]
    pub jsonl: Option<JsonlHistory>,
    #[serde(default)]
    pub sqlite: Option<SqliteHistory>,
    #[serde(default)]
    pub json_files: Option<JsonFilesHistory>,
}

/// One JSON object per line, one line per prompt, folded by conversation.
#[derive(Clone, Debug, PartialEq, Eq, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct JsonlHistory {
    pub file: String,
    pub session: String,
    pub project: String,
    /// Milliseconds since the epoch.
    pub time: String,
    /// The first line's is the title.
    pub title: String,
}

/// A table in an SQLite file.
#[derive(Clone, Debug, PartialEq, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SqliteHistory {
    pub db: String,
    /// The rows, newest first; `{scope}` is where the project's directories
    /// are matched, `1` when every project is listed.
    pub query: String,
    /// The column `{scope}` matches.
    pub scope: String,
    pub id: String,
    pub project: String,
    /// The first of these that is set is the title.
    pub title: Vec<String>,
    pub time: String,
    pub unit: TimeUnit,
    /// The activity count: a column, or lines of a JSON-lines file naming the conversation.
    #[serde(default)]
    pub count: Option<String>,
    #[serde(default)]
    pub count_lines: Option<CountLines>,
}

/// How a time is recorded.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum TimeUnit {
    Seconds,
    Milliseconds,
    /// ISO 8601 text.
    Date,
}

/// The activity of a conversation counted as the lines of a file naming it.
#[derive(Clone, Debug, PartialEq, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CountLines {
    pub file: String,
    pub key: String,
    /// The count of a conversation no line names.
    pub missing: f64,
}

/// One JSON file per conversation, in a directory per project.
#[derive(Clone, Debug, PartialEq, Eq, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct JsonFilesHistory {
    /// Directories each holding a marker file whose text is the project's path.
    #[serde(default)]
    pub markers: Option<Markers>,
    /// A JSON file mapping each project's path to a name.
    #[serde(default)]
    pub map: Option<ProjectMap>,
    /// Which files are conversations: `prefix*suffix`.
    pub files: String,
    pub id: String,
    pub time: String,
    pub messages: String,
    /// A message the person wrote: this field holds this value.
    pub user: [String; 2],
    /// Where a message's text is, however it nests.
    pub text: String,
}

#[derive(Clone, Debug, PartialEq, Eq, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Markers {
    pub dir: String,
    pub file: String,
    /// The conversations' directory inside each.
    pub sessions: String,
}

#[derive(Clone, Debug, PartialEq, Eq, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ProjectMap {
    pub file: String,
    pub key: String,
    /// Where a project's conversations are; `{name}` is its name.
    pub sessions: String,
}

/// The hooks that tell Vorn what the agent does.
#[derive(Clone, Debug, PartialEq, Eq, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Hooks {
    #[serde(default)]
    pub settings: Option<SettingsHooks>,
    #[serde(default)]
    pub file: Option<FileHooks>,
}

/// HTTP hooks merged into the agent's JSON settings while Vorn holds the
/// hook registration, beside the person's own.
#[derive(Clone, Debug, PartialEq, Eq, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SettingsHooks {
    pub file: String,
    pub events: Vec<String>,
    #[serde(default)]
    pub own_status: Option<OwnStatus>,
}

/// From a version on, the CLI reports its own status (OSC 7501) and needs
/// only these hooks answered.
#[derive(Clone, Debug, PartialEq, Eq, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct OwnStatus {
    pub since: String,
    pub events: Vec<String>,
}

/// A hooks file of Vorn's own, written when a session of the agent starts:
/// each event runs a script that posts it as the HTTP hooks do.
#[derive(Clone, Debug, PartialEq, Eq, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct FileHooks {
    pub path: String,
    /// The conversation id its events post; `{terminal}` is the session's.
    pub session: String,
    /// The agent's event names, each with Vorn's name for it.
    pub events: Vec<[String; 2]>,
}

/// What the CLI sets to describe the session it runs in, which no process
/// Vorn starts inherits.
#[derive(Clone, Debug, Default, PartialEq, Eq, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct EnvMarkers {
    #[serde(default)]
    pub strip: Vec<String>,
    #[serde(default)]
    pub strip_prefixes: Vec<String>,
}

/// Where the context a conversation has used is recorded.
#[derive(Clone, Debug, PartialEq, Eq, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Usage {
    /// `{cwd}` is the directory with each character but letters and digits
    /// a dash, `{id}` the conversation.
    pub transcript: String,
    /// The newest line with an object here is read.
    pub at: String,
    /// Summed, the context; `cached` is the part read from the cache.
    pub context: Vec<String>,
    pub cached: String,
}

/// How the agent is connected to Vorn's tools, shown in Settings.
#[derive(Clone, Debug, PartialEq, Eq, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct McpSetup {
    pub commands: Vec<String>,
    /// Typed inside the agent rather than in a shell.
    #[serde(default)]
    pub in_agent: bool,
    #[serde(default)]
    pub note: Option<String>,
}

/// The placeholder for a conversation id.
pub const ID: &str = "{id}";
/// The placeholder for a first prompt.
pub const PROMPT: &str = "{prompt}";

/// A spec that does not hold together, and where.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SpecError {
    /// The dotted field, as `agent.models.list.lines`.
    pub field: String,
    pub message: String,
}

impl fmt::Display for SpecError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}: {}", self.field, self.message)
    }
}

impl std::error::Error for SpecError {}

fn error(field: &str, message: impl Into<String>) -> SpecError {
    SpecError {
        field: format!("agent.{field}"),
        message: message.into(),
    }
}

/// Whether `id` may name an agent: lowercase ASCII letters, digits and
/// dashes, starting with a letter.
pub fn valid_id(id: &str) -> bool {
    let b = id.as_bytes();
    !b.is_empty()
        && b.len() <= 32
        && b[0].is_ascii_lowercase()
        && b.iter()
            .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || *c == b'-')
}

/// Whether `words` holds `placeholder` as one whole word.
fn has_word(words: &[String], placeholder: &str) -> bool {
    words.iter().any(|w| w == placeholder)
}

impl AgentSpec {
    /// Reads an `[agent]` section, already parsed as TOML.
    pub fn from_toml(section: toml::Value) -> Result<AgentSpec, SpecError> {
        let spec: AgentSpec = section.try_into().map_err(|e: toml::de::Error| SpecError {
            field: "agent".to_owned(),
            message: e.message().trim_end().to_owned(),
        })?;
        spec.check()?;
        Ok(spec)
    }

    /// The `[agent]` section of a whole manifest's text, or `None` when it
    /// has none.
    pub fn from_manifest(text: &str) -> Result<Option<AgentSpec>, SpecError> {
        let mut table: toml::Table = toml::from_str(text).map_err(|e| SpecError {
            field: String::new(),
            message: e.message().trim_end().to_owned(),
        })?;
        table.remove("agent").map(AgentSpec::from_toml).transpose()
    }

    /// What the parser cannot see: ids, placeholders, patterns and rules
    /// that must hold together.
    pub fn check(&self) -> Result<(), SpecError> {
        if !valid_id(&self.id) || self.id == "shell" {
            return Err(error(
                "id",
                "lowercase letters, digits and dashes, starting with a letter; not `shell`",
            ));
        }
        if self.name.trim().is_empty() {
            return Err(error("name", "is empty"));
        }
        if self.launch.command.trim().is_empty() {
            return Err(error("launch.command", "is empty"));
        }
        if !has_word(&self.launch.prompt, PROMPT) {
            return Err(error("launch.prompt", "names no {prompt}"));
        }
        if let Some(mark) = &self.mark {
            if mark.chars().count() != 1 {
                return Err(error("mark", "is one character"));
            }
        }
        for (field, value, len) in [("color", &self.color, 7), ("tint", &self.tint, 9)] {
            if let Some(v) = value {
                let hex = v.strip_prefix('#').unwrap_or("");
                if v.len() != len || !hex.bytes().all(|b| b.is_ascii_hexdigit()) {
                    return Err(error(field, format!("is #{} hex digits", len - 1)));
                }
            }
        }
        if let Some(r) = &self.resume {
            if !has_word(&r.args, ID) {
                return Err(error("resume.args", "names no {id}"));
            }
            if !r.pin.is_empty() && !has_word(&r.pin, ID) {
                return Err(error("resume.pin", "names no {id}"));
            }
        }
        if let Some(h) = &self.headless {
            for (field, words) in [
                ("headless.resume", &h.resume),
                ("headless.resume_tail", &h.resume_tail),
            ] {
                if !words.is_empty() && !has_word(words, ID) {
                    return Err(error(field, "names no {id}"));
                }
            }
        }
        for (field, template, json) in self.paths() {
            if !rooted(template) || template.split(['/', '\\']).any(|seg| seg == "..") {
                return Err(error(
                    field,
                    "starts at `~`, `{data}`, `{env:NAME|fallback}` or `/`, and has no `..`",
                ));
            }
            if json && !template.ends_with(".json") {
                return Err(error(field, "names a .json file"));
            }
        }
        if let Some(m) = &self.models {
            if !m.flag.starts_with('-') {
                return Err(error("models.flag", "is a flag"));
            }
            if let Some(list) = &m.list {
                match (&list.lines, list.steps.is_empty()) {
                    (Some(_), false) => {
                        return Err(error("models.list", "has both `lines` and `steps`"))
                    }
                    (None, true) => return Err(error("models.list", "needs `lines` or `steps`")),
                    (Some(p), true) => {
                        regex::Regex::new(p)
                            .map_err(|e| error("models.list.lines", e.to_string()))?;
                    }
                    (None, false) => {
                        if list
                            .steps
                            .iter()
                            .flat_map(|s| &s.send)
                            .any(|m| !m.is_object())
                        {
                            return Err(error("models.list.steps", "sends JSON objects"));
                        }
                        if !list.steps.iter().any(|s| s.models.is_some()) {
                            return Err(error("models.list.steps", "no step reads `models`"));
                        }
                        for (i, s) in list.steps.iter().enumerate() {
                            if s.next.is_some() != s.cursor.is_some() {
                                return Err(error(
                                    &format!("models.list.steps.{i}"),
                                    "`next` and `cursor` go together",
                                ));
                            }
                        }
                    }
                }
            }
        }
        if let Some(h) = &self.history {
            let kinds = [
                h.jsonl.is_some(),
                h.sqlite.is_some(),
                h.json_files.is_some(),
            ];
            if kinds.iter().filter(|k| **k).count() != 1 {
                return Err(error(
                    "history",
                    "needs exactly one of `jsonl`, `sqlite` and `json_files`",
                ));
            }
            if let Some(s) = &h.sqlite {
                if !s.query.contains("{scope}") {
                    return Err(error("history.sqlite.query", "names no {scope}"));
                }
                if s.title.is_empty() {
                    return Err(error("history.sqlite.title", "is empty"));
                }
            }
            if let Some(j) = &h.json_files {
                if j.markers.is_none() && j.map.is_none() {
                    return Err(error("history.json_files", "needs `markers` or `map`"));
                }
                if j.files.matches('*').count() != 1 {
                    return Err(error("history.json_files.files", "is `prefix*suffix`"));
                }
            }
        }
        if !self.status.is_empty() {
            Rules::new(&self.id, &self.status).map_err(|e| error("status", e.to_string()))?;
        }
        if let Some(h) = &self.hooks {
            if h.settings.is_none() && h.file.is_none() {
                return Err(error("hooks", "needs `settings` or `file`"));
            }
            if let Some(f) = &h.file {
                if !f.session.contains("{terminal}") {
                    return Err(error("hooks.file.session", "names no {terminal}"));
                }
            }
        }
        Ok(())
    }

    /// What menus call it.
    pub fn label(&self) -> &str {
        self.label.as_deref().unwrap_or(&self.name)
    }

    /// Every path template, its field, and whether it is a JSON file Vorn writes.
    fn paths(&self) -> Vec<(&'static str, &str, bool)> {
        let mut out = Vec::new();
        if let Some(c) = self.resume.as_ref().and_then(|r| r.capture.as_ref()) {
            out.push(("resume.capture.db", c.db.as_str(), false));
        }
        if let Some(h) = &self.history {
            if let Some(j) = &h.jsonl {
                out.push(("history.jsonl.file", j.file.as_str(), false));
            }
            if let Some(q) = &h.sqlite {
                out.push(("history.sqlite.db", q.db.as_str(), false));
                if let Some(c) = &q.count_lines {
                    out.push(("history.sqlite.count_lines.file", c.file.as_str(), false));
                }
            }
            if let Some(f) = &h.json_files {
                if let Some(m) = &f.markers {
                    out.push(("history.json_files.markers.dir", m.dir.as_str(), false));
                }
                if let Some(m) = &f.map {
                    out.push(("history.json_files.map.file", m.file.as_str(), false));
                    out.push((
                        "history.json_files.map.sessions",
                        m.sessions.as_str(),
                        false,
                    ));
                }
            }
        }
        if let Some(h) = &self.hooks {
            if let Some(s) = &h.settings {
                out.push(("hooks.settings.file", s.file.as_str(), true));
            }
            if let Some(f) = &h.file {
                out.push(("hooks.file.path", f.path.as_str(), true));
            }
        }
        if let Some(u) = &self.usage {
            out.push(("usage.transcript", u.transcript.as_str(), false));
        }
        out
    }

    /// The name a workflow step uses.
    pub fn short_name(&self) -> &str {
        self.short_name.as_deref().unwrap_or(self.label())
    }
}

/// The directories path templates are read against.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Places {
    pub home: PathBuf,
    pub data: PathBuf,
}

impl Places {
    /// As Vorn finds them: `HOME` (`USERPROFILE` on Windows), and
    /// `$XDG_DATA_HOME` or `~/.local/share` (`%LOCALAPPDATA%` or
    /// `~\AppData\Local` on Windows). `None` without a home directory.
    pub fn from_env() -> Option<Places> {
        let home = PathBuf::from(var(if cfg!(windows) { "USERPROFILE" } else { "HOME" })?);
        let data = if cfg!(windows) {
            var("LOCALAPPDATA").map_or_else(|| home.join("AppData").join("Local"), PathBuf::from)
        } else {
            var("XDG_DATA_HOME").map_or_else(|| home.join(".local").join("share"), PathBuf::from)
        };
        Some(Places { home, data })
    }

    /// `template` as a path: its first segment read as a place, the rest
    /// joined on with this host's separator.
    pub fn path(&self, template: &str) -> PathBuf {
        self.path_with(template, &var)
    }

    /// [`Places::path`] with the variables `env` answers.
    pub fn path_with(&self, template: &str, env: &dyn Fn(&str) -> Option<String>) -> PathBuf {
        let (first, rest) = split_first(template);
        let mut out = self.root(first, env);
        for seg in rest.split('/').filter(|s| !s.is_empty()) {
            out.push(seg);
        }
        out
    }

    fn root(&self, first: &str, env: &dyn Fn(&str) -> Option<String>) -> PathBuf {
        if first == "~" {
            return self.home.clone();
        }
        if first == "{data}" {
            return self.data.clone();
        }
        if let Some(inner) = first
            .strip_prefix("{env:")
            .and_then(|r| r.strip_suffix('}'))
        {
            let (name, fallback) = inner.split_once('|').unwrap_or((inner, ""));
            return match env(name).filter(|v| !v.is_empty()) {
                Some(v) => PathBuf::from(v),
                None => self.path_with(fallback, env),
            };
        }
        PathBuf::from(first)
    }
}

/// The first segment of a template (a `{...}` place may hold slashes) and the rest.
fn split_first(template: &str) -> (&str, &str) {
    if template.starts_with('{') {
        if let Some(end) = template.find('}') {
            let (first, rest) = template.split_at(end + 1);
            return (first, rest);
        }
    }
    match template.find('/') {
        Some(at) => (&template[..at], &template[at..]),
        None => (template, ""),
    }
}

fn var(name: &str) -> Option<String> {
    std::env::var(name).ok().filter(|v| !v.is_empty())
}

/// The value at a dotted `path` in `value`, through objects only.
pub fn at<'a>(value: &'a Value, path: &str) -> Option<&'a Value> {
    path.split('.')
        .try_fold(value, |v, key| v.as_object().and_then(|o| o.get(key)))
}

/// Whether `path` is the root of a place template, so a check can refuse
/// one that is relative.
pub fn rooted(template: &str) -> bool {
    let (first, _) = split_first(template);
    first == "~"
        || first == "{data}"
        || first.starts_with("{env:")
        || Path::new(first).is_absolute()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn places() -> Places {
        Places {
            home: PathBuf::from("/h"),
            data: PathBuf::from("/d"),
        }
    }

    #[test]
    fn reads_places_in_paths() {
        let p = places();
        let none = &|_: &str| None;
        assert_eq!(
            p.path_with("~/.x/y.db", none),
            Path::new("/h").join(".x").join("y.db")
        );
        assert_eq!(
            p.path_with("{data}/a/b", none),
            Path::new("/d").join("a").join("b")
        );
        assert_eq!(
            p.path_with("{env:X_HOME|~/.x}/hooks/v.json", none),
            Path::new("/h").join(".x").join("hooks").join("v.json")
        );
        let set = &|n: &str| (n == "X_HOME").then(|| "/c".to_owned());
        assert_eq!(
            p.path_with("{env:X_HOME|~/.x}/hooks/v.json", set),
            Path::new("/c").join("hooks").join("v.json")
        );
        assert!(rooted("~/a") && rooted("{data}/b") && !rooted("rel/a"));
    }

    #[test]
    fn checks_what_the_parser_cannot() {
        let base = r#"
            [agent]
            id = "demo"
            name = "Demo"
            [agent.launch]
            command = "demo"
        "#;
        let spec = AgentSpec::from_manifest(base).unwrap().unwrap();
        assert_eq!(spec.launch.prompt, ["{prompt}"]);
        assert_eq!(spec.short_name(), "Demo");
        assert!(AgentSpec::from_manifest("id = 'x'").unwrap().is_none());
        let bad = |extra: &str| {
            AgentSpec::from_manifest(&format!("{base}\n{extra}"))
                .unwrap_err()
                .to_string()
        };
        assert!(bad("[agent.resume]\nargs = ['--resume']").contains("agent.resume.args"));
        assert!(bad("[agent.history]\nactivity = 'turn'").contains("exactly one"));
        assert!(bad("[[agent.status]]\nstatus = 'idle'\nany = ['(']").contains("agent.status"));
        assert!(bad("[agent.models.list]\nargs = []").contains("`lines` or `steps`"));
        assert!(bad("unknown = 1").contains("unknown"));
        let hooks = |file: &str| {
            bad(&format!(
                "[agent.hooks.settings]\nfile = '{file}'\nevents = []"
            ))
        };
        assert!(hooks("~/.zshrc").contains("names a .json file"));
        assert!(hooks("~/../x/settings.json").contains("no `..`"));
        assert!(hooks("rel/settings.json").contains("no `..`"));
        assert!(bad("[agent.history]\nactivity = 'a'\n[agent.history.jsonl]\nfile = '../h'\nsession = 's'\nproject = 'p'\ntime = 't'\ntitle = 'd'").contains("history.jsonl.file"));
        assert!(bad("[agent.models.list]\nargs = []\n[[agent.models.list.steps]]\nsend = ['x']\nreply = {}\nmodels = 'm'").contains("JSON objects"));
        let shell = base.replace("\"demo\"\n            name", "\"shell\"\n            name");
        assert!(AgentSpec::from_manifest(&shell).is_err());
    }

    #[test]
    fn reads_dotted_paths_through_objects() {
        let v = serde_json::json!({ "a": { "b": [1], "c": { "d": "x" } } });
        assert_eq!(at(&v, "a.c.d"), Some(&Value::from("x")));
        assert_eq!(at(&v, "a.b.0"), None);
        assert_eq!(at(&v, "z"), None);
    }
}

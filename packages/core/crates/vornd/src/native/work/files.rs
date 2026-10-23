//! Workflows as files ([`vorn_workflow_file`]): each project's
//! `.vorn/workflows/*.toml`, read into the stored workflows and watched, so
//! editing a file changes its workflow; a workflow from a file written back
//! to it when the editor changes it, keeping the file's comments; a workflow
//! made in Vorn saved to a project's folder in one call; and the workflows
//! installed extensions ship (`[[contributes.workflows]]`), added once.
//!
//! A project's file runs nothing until the user turns it on: trust is kept
//! per file as the hash of the content turned on, so a file edited outside
//! Vorn is off again until turned on again.
//!
//! Which stored workflow came from which file is kept in
//! `vornd/workflow-files.json` beside the database, so a file removed while
//! vornd was down still takes its workflow with it.

use std::collections::{BTreeMap, BTreeSet, HashMap};
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex, MutexGuard};
use std::time::{Duration, SystemTime};

use serde_json::{json, Map, Value};
use tracing::{info, warn};
use vorn_workflow_file::{Bindings, FileError, RUNTIME_FIELDS};

use vorn_work::model::NodeKind;
use vorn_workflow::Registry;

use crate::atomic;

use super::host::VorndHost;
use super::Work;

/// How often the projects' workflow folders are looked at.
const LOOK: Duration = Duration::from_secs(1);

/// Where in a project its workflows are.
pub const FOLDER: &str = ".vorn/workflows";

/// What a file was when last read or written: when it changed and how long it was.
type Stamp = (SystemTime, u64);

#[derive(Debug, Default)]
struct State {
    /// The stored workflows that came from a file, by id.
    by_id: BTreeMap<String, PathBuf>,
    stamps: HashMap<PathBuf, Stamp>,
    /// Files that do not read, and why: what `workflow:files` shows.
    errors: BTreeMap<PathBuf, Vec<String>>,
    /// Extension workflows added once, as `extension/id`.
    seeded: BTreeSet<String>,
    /// Each file's workflow as last read or written: what an edit of the
    /// file and an edit in the editor made apart both started from.
    base: HashMap<PathBuf, Value>,
    /// Each file's content hash as the user turned it on.
    trusted: BTreeMap<PathBuf, String>,
    /// Each file's content hash as last read or written.
    hashes: BTreeMap<PathBuf, String>,
    /// Each file's workflow as checked, for the user to see what it runs
    /// before turning it on.
    runs: HashMap<PathBuf, Value>,
}

impl State {
    /// Whether `file` is as the user turned it on.
    fn trusts(&self, file: &Path) -> bool {
        matches!((self.trusted.get(file), self.hashes.get(file)), (Some(t), Some(h)) if t == h)
    }

    /// `file` does not read, for `errors`: nothing of it shows or turns on
    /// until it reads again.
    fn refuse(&mut self, file: &Path, at: Option<Stamp>, errors: Vec<String>) {
        if let Some(at) = at {
            self.stamps.insert(file.to_owned(), at);
        }
        self.hashes.remove(file);
        self.runs.remove(file);
        self.errors.insert(file.to_owned(), errors);
    }

    fn forget(&mut self, file: &Path) {
        self.stamps.remove(file);
        self.errors.remove(file);
        self.base.remove(file);
        self.trusted.remove(file);
        self.hashes.remove(file);
        self.runs.remove(file);
    }
}

/// The workflow files vornd knows.
#[derive(Debug)]
pub struct Files {
    state: Mutex<State>,
    /// `vornd/workflow-files.json`.
    record: PathBuf,
}

/// What is kept between runs of vornd.
#[derive(serde::Serialize, serde::Deserialize, Default)]
struct Kept {
    #[serde(default)]
    files: BTreeMap<String, PathBuf>,
    #[serde(default)]
    seeded: BTreeSet<String>,
    #[serde(default)]
    trusted: BTreeMap<PathBuf, String>,
    #[serde(default)]
    hashes: BTreeMap<PathBuf, String>,
}

/// A file's content hash, as trust is kept.
fn digest(text: &str) -> String {
    use sha2::{Digest, Sha256};
    Sha256::digest(text.as_bytes())
        .iter()
        .map(|b| format!("{b:02x}"))
        .collect()
}

/// `wf` with `enabled` as `from` has it: an untrusted file's own setting,
/// never the off the store holds for it.
fn enabled_as(mut wf: Value, from: &Value) -> Value {
    if let Some(map) = wf.as_object_mut() {
        match from.get("enabled") {
            Some(v) => {
                map.insert("enabled".into(), v.clone());
            }
            None => {
                map.shift_remove("enabled");
            }
        }
    }
    wf
}

/// A built-in node type's settings: those listed for the user, then those
/// that change only how a step is named or timed. A file holding any other
/// setting on a step of the type does not read.
struct Kind {
    id: &'static str,
    shown: &'static [&'static str],
    quiet: &'static [&'static str],
}

/// Every node type the runner knows but extensions' and triggers' (which
/// list every setting they hold), with every setting it reads.
const KINDS: &[Kind] = &[
    Kind {
        id: "condition",
        shown: &[],
        quiet: &["variable", "operator", "value"],
    },
    Kind {
        id: "approval",
        shown: &[],
        quiet: &["message", "timeoutMs", "view", "feedback", "edit"],
    },
    Kind {
        id: "loop",
        shown: &[],
        quiet: &[
            "nodeType",
            "mode",
            "items",
            "bodyNodeIds",
            "maxIterations",
            "until",
            "parallel",
            "concurrency",
        ],
    },
    Kind {
        id: "launchAgent",
        shown: &[
            "agentType",
            "prompt",
            "args",
            "model",
            "projectName",
            "projectPath",
            "useWorktree",
            "worktreeMode",
            "worktreeFromStepSlug",
            "existingWorktreePath",
            "branch",
            "taskId",
            "taskFromQueue",
            "headless",
            "outputSchema",
        ],
        quiet: &["displayName", "promptDelayMs", "timeoutMs"],
    },
    Kind {
        id: "script",
        shown: &[
            "scriptType",
            "scriptContent",
            "args",
            "cwd",
            "projectName",
            "projectPath",
            "secretsFrom",
        ],
        quiet: &["runId"],
    },
    Kind {
        id: "createTaskFromItem",
        shown: &["project", "initialStatus"],
        quiet: &["nodeType"],
    },
    Kind {
        id: "callConnectorAction",
        shown: &["action", "connectionId", "args"],
        quiet: &["nodeType", "actionLabel", "connectorId"],
    },
    Kind {
        id: "httpRequest",
        shown: &["method", "url", "headers", "body", "profileConnectionId"],
        quiet: &["nodeType"],
    },
];

/// A setting's value as the list shows it: text as it is, the rest as JSON.
fn shown_value(v: &Value) -> String {
    match v {
        Value::String(s) => s.clone(),
        other => other.to_string(),
    }
}

/// The remote host an agent step in project `name` runs on, from `projects`
/// as `dbListProjects` lists them.
fn remote_line(name: &str, projects: &[Value]) -> String {
    let project = (!name.is_empty() && !name.contains("{{"))
        .then(|| {
            projects
                .iter()
                .find(|p| p.get("name").and_then(Value::as_str) == Some(name))
        })
        .flatten();
    match project {
        Some(p) => match vorn_workflows::agent::remote_host_of(p) {
            Some(host) => format!("  remote host: {host}"),
            None => "  remote host: none, this computer".to_owned(),
        },
        None => "  remote host: its project's, when it runs".to_owned(),
    }
}

/// What a workflow does when it runs, one line per node and one per setting
/// that decides what it runs or sends, for the user to read before turning
/// it on; an agent step also names the remote host its project runs on, from
/// `projects`. Refuses a step of a built-in type with a setting the type
/// does not read. A step runs as `types` resolves it, so one of a type no
/// extension declares is listed and checked as the fallback it runs as.
pub fn runs(
    wf: &Value,
    projects: &[Value],
    types: &Registry<VorndHost>,
) -> Result<Vec<String>, String> {
    let mut out = Vec::new();
    for node in wf
        .get("nodes")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
    {
        let raw = node.get("type").and_then(Value::as_str).unwrap_or("");
        let kind = match NodeKind::parse(raw).step() {
            Some(step) => types.runs_as(step).unwrap_or_else(|| step.to_owned()),
            None => raw.to_owned(),
        };
        let kind = kind.as_str();
        let label = node
            .get("label")
            .and_then(Value::as_str)
            .map(|l| l.split_whitespace().collect::<Vec<_>>().join(" "))
            .filter(|l| !l.is_empty())
            .or_else(|| node.get("id").and_then(Value::as_str).map(str::to_owned))
            .unwrap_or_else(|| kind.to_owned());
        let config = match node.get("config") {
            None | Some(Value::Null) => None,
            Some(Value::Object(c)) => Some(c),
            Some(_) => return Err(format!("{label}: its settings are not a table")),
        };
        let settings = || config.into_iter().flatten().filter(|(_, v)| !v.is_null());
        if kind == "trigger" {
            let on = config
                .and_then(|c| c.get("triggerType"))
                .and_then(Value::as_str)
                .unwrap_or("manual");
            out.push(format!("Trigger: {on}"));
            for (key, v) in settings().filter(|(k, _)| *k != "triggerType") {
                out.push(format!("  {key}: {}", shown_value(v)));
            }
            continue;
        }
        let Some(known) = KINDS.iter().find(|k| k.id == kind) else {
            // An extension's step: the action is its type, and every setting an input.
            out.push(format!("{label} ({kind})"));
            for (key, v) in settings() {
                out.push(format!("  {key}: {}", shown_value(v)));
            }
            continue;
        };
        if let Some(key) = config
            .into_iter()
            .flat_map(|c| c.keys())
            .find(|k| !known.shown.contains(&k.as_str()) && !known.quiet.contains(&k.as_str()))
        {
            return Err(format!("{label}: {key} is not a setting of a {kind} step"));
        }
        if known.shown.is_empty() {
            continue;
        }
        out.push(format!("{label} ({kind})"));
        for key in known.shown {
            if let Some(v) = config.and_then(|c| c.get(*key)).filter(|v| !v.is_null()) {
                out.push(format!("  {key}: {}", shown_value(v)));
            }
        }
        if kind == "launchAgent" {
            let project = config
                .and_then(|c| c.get("projectName"))
                .and_then(Value::as_str)
                .unwrap_or_default();
            out.push(remote_line(project, projects));
        }
    }
    Ok(out)
}

fn stamp(path: &Path) -> Option<Stamp> {
    let meta = std::fs::metadata(path).ok()?;
    Some((meta.modified().ok()?, meta.len()))
}

/// The id a file's workflow has when the file names none: its project and name.
pub fn derived_id(project: &str, path: &Path) -> String {
    let stem = path
        .file_stem()
        .map(|s| s.to_string_lossy().into_owned())
        .unwrap_or_default();
    format!("file:{project}/{stem}")
}

/// A stored workflow as its file says it: without when it last ran.
fn as_file(mut wf: Value) -> Value {
    if let Some(map) = wf.as_object_mut() {
        for k in RUNTIME_FIELDS {
            map.shift_remove(k);
        }
    }
    vorn_work::js_numbers(wf)
}

impl Files {
    pub fn new(data_dir: &Path) -> Files {
        let record = data_dir.join("vornd").join("workflow-files.json");
        let kept: Kept = std::fs::read(&record)
            .ok()
            .and_then(|b| serde_json::from_slice(&b).ok())
            .unwrap_or_default();
        Files {
            state: Mutex::new(State {
                by_id: kept.files,
                seeded: kept.seeded,
                trusted: kept.trusted,
                hashes: kept.hashes,
                ..State::default()
            }),
            record,
        }
    }

    fn lock(&self) -> MutexGuard<'_, State> {
        self.state.lock().unwrap_or_else(|e| e.into_inner())
    }

    fn keep(&self, state: &State) {
        let kept = Kept {
            files: state.by_id.clone(),
            seeded: state.seeded.clone(),
            trusted: state.trusted.clone(),
            hashes: state.hashes.clone(),
        };
        let bytes = serde_json::to_vec_pretty(&kept).unwrap_or_default();
        if let Err(err) = atomic::replace(&self.record, &bytes) {
            warn!(%err, "could not keep which workflows came from files");
        }
    }

    /// The file workflow `id` came from.
    pub fn file_of(&self, id: &str) -> Option<PathBuf> {
        self.lock().by_id.get(id).cloned()
    }

    /// Why workflow `id` may not run: it came from a file not turned on.
    pub fn refuses(&self, id: &str) -> Option<String> {
        let s = self.lock();
        let file = s.by_id.get(id)?;
        (!s.trusts(file)).then(|| {
            format!(
                "{} is off until you turn it on from its row in Workflows",
                file.display()
            )
        })
    }

    /// Takes back each stored workflow of a file not turned on whose id is
    /// in a namespace `owns` says an extension holds now, and lets every
    /// file that did not read be read again: the extensions changed, so what
    /// an id may be did too. The ids taken back, to delete.
    fn take_owned(&self, owns: impl Fn(&str) -> bool) -> Vec<String> {
        let mut s = self.lock();
        let taken: Vec<(String, PathBuf)> = s
            .by_id
            .iter()
            .filter(|(id, file)| id.contains(':') && !s.trusts(file) && owns(id))
            .map(|(id, file)| (id.clone(), file.clone()))
            .collect();
        for (id, file) in &taken {
            s.by_id.remove(id);
            s.forget(file);
        }
        let refused: Vec<PathBuf> = s.errors.keys().cloned().collect();
        for file in refused {
            s.stamps.remove(&file);
        }
        if !taken.is_empty() {
            self.keep(&s);
        }
        taken.into_iter().map(|(id, _)| id).collect()
    }

    /// Turns on workflow `id`'s file as the user saw it: `hash` is the
    /// content listed to them, and a file that changed since is refused.
    pub fn trust(&self, id: &str, hash: &str) -> Result<(), String> {
        let mut s = self.lock();
        let Some(file) = s.by_id.get(id).cloned() else {
            return Err(format!("Workflow {id} is not from a project's file"));
        };
        if s.hashes.get(&file).map(String::as_str) != Some(hash) {
            return Err(format!(
                "{} changed since it was shown; look at it again before turning it on",
                file.display()
            ));
        }
        s.trusted.insert(file, hash.to_owned());
        self.keep(&s);
        Ok(())
    }

    /// Every file, whether it is on and what it runs, and what is wrong
    /// with the ones that do not read; agent steps' hosts from `projects`.
    pub fn listed(&self, projects: &[Value], types: &Registry<VorndHost>) -> Value {
        let s = self.lock();
        let files: Vec<Value> = s
            .by_id
            .iter()
            .map(|(id, path)| {
                json!({
                    "workflowId": id,
                    "path": path,
                    "trusted": s.trusts(path),
                    "hash": s.hashes.get(path),
                    "runs": s.runs.get(path).map(|wf| runs(wf, projects, types).unwrap_or_default()).unwrap_or_default(),
                })
            })
            .collect();
        let errors: Vec<Value> = s
            .errors
            .iter()
            .map(|(path, errors)| json!({ "path": path, "errors": errors }))
            .collect();
        json!({ "files": files, "errors": errors })
    }
}

/// The connections here, for a file's names: each one's id and name.
async fn connections(work: &Work) -> Vec<(String, String)> {
    work.db
        .run(|s| s.call("dbListSourceConnections", json!([null])).ok())
        .await
        .flatten()
        .and_then(|v| v.as_array().cloned())
        .unwrap_or_default()
        .iter()
        .filter_map(|c| {
            Some((
                c.get("id")?.as_str()?.to_owned(),
                c.get("name")?.as_str()?.to_owned(),
            ))
        })
        .collect()
}

/// Every project, as `dbListProjects` lists it.
async fn project_rows(work: &Work) -> Vec<Value> {
    work.db
        .run(|s| s.call("dbListProjects", json!([])).ok())
        .await
        .flatten()
        .and_then(|v| v.as_array().cloned())
        .unwrap_or_default()
}

/// Each project's name and path.
async fn projects(work: &Work) -> Vec<(String, String)> {
    project_rows(work)
        .await
        .iter()
        .filter_map(|p| {
            Some((
                p.get("name")?.as_str()?.to_owned(),
                p.get("path")?.as_str()?.to_owned(),
            ))
        })
        .collect()
}

/// The `.toml` files in a project's workflow folder.
fn listed_files(project_path: &str) -> Vec<PathBuf> {
    let dir = Path::new(project_path).join(FOLDER);
    let Ok(entries) = std::fs::read_dir(dir) else {
        return Vec::new();
    };
    let mut out: Vec<PathBuf> = entries
        .flatten()
        .map(|e| e.path())
        .filter(|p| {
            p.extension()
                .is_some_and(|e| e == vorn_workflow_file::EXTENSION)
                && !atomic::is_temp(p)
                && p.is_file()
        })
        .collect();
    out.sort();
    out
}

/// A write that syncs to disk, run off the async runtime.
async fn off_runtime(
    write: impl FnOnce() -> std::io::Result<()> + Send + 'static,
) -> std::io::Result<()> {
    tokio::task::spawn_blocking(write)
        .await
        .unwrap_or_else(|e| Err(std::io::Error::other(e)))
}

fn shown(path: &Path, errors: &[FileError]) -> Vec<String> {
    errors
        .iter()
        .map(|e| format!("{}:{e}", path.display()))
        .collect()
}

impl Work {
    /// Watches the projects' workflow folders until vornd stops.
    pub(crate) async fn watch_files(self: Arc<Self>) {
        let mut every = tokio::time::interval(LOOK);
        every.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
        loop {
            every.tick().await;
            self.look_at_files().await;
        }
    }

    /// Reads every workflow file that changed since the last look, and
    /// takes away the workflows whose files are gone.
    pub(crate) async fn look_at_files(&self) {
        let projects = projects(self).await;
        let mut seen: Vec<(String, String, PathBuf)> = Vec::new();
        for (name, path) in &projects {
            for file in listed_files(path) {
                seen.push((name.clone(), path.clone(), file));
            }
        }
        let changed: Vec<&(String, String, PathBuf)> = {
            let s = self.files.lock();
            seen.iter()
                .filter(|(_, _, f)| s.stamps.get(f) != stamp(f).as_ref())
                .collect()
        };
        let mut any = false;
        for (name, path, file) in changed {
            any |= self.read_file(name, path, file).await;
        }
        let gone: Vec<(String, PathBuf)> = {
            let s = self.files.lock();
            s.by_id
                .iter()
                .filter(|(_, p)| !p.is_file())
                .map(|(id, p)| (id.clone(), p.clone()))
                .collect()
        };
        for (id, path) in gone {
            info!(file = %path.display(), workflow = %id, "a workflow's file is gone; so is the workflow");
            {
                let mut s = self.files.lock();
                s.by_id.remove(&id);
                s.forget(&path);
                self.files.keep(&s);
            }
            let w = id.clone();
            self.db
                .run(move |s| {
                    let _ = s.call("dbDeleteWorkflow", json!([w]));
                })
                .await;
            any = true;
        }
        if any {
            self.workflows_changed();
        }
    }

    /// Takes back the stored workflows of files not turned on whose ids an
    /// extension now owns, as `read_file` would refuse them today.
    pub(crate) async fn recheck_files(&self) {
        let taken = self
            .files
            .take_owned(|id| self.extension_owning(id).is_some());
        if taken.is_empty() {
            return;
        }
        for id in &taken {
            info!(workflow = %id, "a project's workflow is in an extension's namespace; it is taken back");
        }
        self.db
            .run(move |s| {
                for id in taken {
                    let _ = s.call("dbDeleteWorkflow", json!([id]));
                }
            })
            .await;
        self.workflows_changed();
    }

    /// The extension whose namespace workflow `id` is in (`<extension id>:`,
    /// in any case): Vorn's own, one installed, or a built-in.
    fn extension_owning(&self, id: &str) -> Option<String> {
        let (namespace, _) = id.split_once(':')?;
        let namespace = namespace.to_ascii_lowercase();
        if namespace.starts_with(vorn_extensions::pack::SHIPPED_NAMESPACE) {
            return Some(namespace);
        }
        let native = self.native.upgrade()?;
        let ext = native.extensions()?;
        let installed = ext.router().store().ids();
        let owned = ext.builtins().has(&namespace)
            || installed.iter().any(|i| i.eq_ignore_ascii_case(&namespace));
        owned.then_some(namespace)
    }

    /// Reads `file` of project `name` at `path` into its stored workflow;
    /// whether the workflow changed.
    async fn read_file(&self, name: &str, path: &str, file: &Path) -> bool {
        let at = stamp(file);
        let Ok(text) = std::fs::read_to_string(file) else {
            return false;
        };
        let hash = digest(&text);
        // `was`: the stored workflow holds its file's setting, not an off of Vorn's.
        let (trusted, was) = {
            let s = self.files.lock();
            (s.trusted.get(file) == Some(&hash), s.trusts(file))
        };
        let conns = connections(self).await;
        let bindings = Bindings {
            connections: &conns,
            project: Some((name, path)),
        };
        let read = match vorn_workflow_file::read(&text, &bindings) {
            Ok(r) => r,
            Err(errors) => {
                let lines = shown(file, &errors);
                warn!(file = %file.display(), errors = %lines.join("; "), "a workflow file does not read");
                let mut s = self.files.lock();
                s.refuse(file, at, lines);
                self.files.keep(&s);
                return false;
            }
        };
        for w in read.warnings.iter().chain(&read.hints) {
            warn!(file = %file.display(), "{}", w);
        }
        let mut wf = read.workflow;
        let id = match wf.get("id").and_then(Value::as_str) {
            Some(id) => id.to_owned(),
            None => {
                let id = derived_id(name, file);
                wf["id"] = json!(id);
                id
            }
        };
        // One Vorn saved itself, as written, keeps its id.
        if let Some(owner) = self.extension_owning(&id).filter(|_| !trusted) {
            let mut s = self.files.lock();
            s.refuse(
                file,
                at,
                vec![format!(
                    "{}: its id {id} is in the namespace of the extension {owner}, which only it adds workflows to",
                    file.display(),
                )],
            );
            self.files.keep(&s);
            return false;
        }
        let (taken, tied) = {
            let s = self.files.lock();
            let other = s
                .by_id
                .get(&id)
                .filter(|p| p.as_path() != file && p.is_file())
                .map(|p| p.display().to_string());
            (other, s.by_id.contains_key(&id))
        };
        // A file never takes over a workflow made in the app.
        let taken = match taken {
            Some(other) => Some(other),
            None if !tied && id != derived_id(name, file) => {
                let w = id.clone();
                let stored = self
                    .db
                    .run(move |s| s.call("dbGetWorkflow", json!([w])).ok())
                    .await
                    .flatten()
                    .is_some_and(|v| v.is_object());
                stored.then(|| "a workflow made in the app".to_owned())
            }
            None => None,
        };
        if let Some(other) = taken {
            let mut s = self.files.lock();
            s.refuse(
                file,
                at,
                vec![format!(
                    "{}: its id {id} is the workflow of {other} already",
                    file.display(),
                )],
            );
            self.files.keep(&s);
            return false;
        }
        // An edit in the editor not yet in the file meets the file's edit.
        let base = self.files.lock().base.get(file).cloned();
        let theirs = vorn_work::js_numbers(wf);
        let met = match base {
            Some(base) => {
                let w = id.clone();
                let ours = self
                    .db
                    .run(move |s| s.call("dbGetWorkflow", json!([w])).ok())
                    .await
                    .flatten()
                    .filter(Value::is_object)
                    .map(as_file)
                    .map(|ours| {
                        if trusted && was {
                            ours
                        } else {
                            enabled_as(ours, &base)
                        }
                    });
                match ours {
                    Some(ours) if ours != base => vorn_workflow_file::meet(&base, &ours, &theirs),
                    _ => theirs.clone(),
                }
            }
            None => theirs.clone(),
        };
        if let Err(why) = runs(&met, &[], self.engine.registry()) {
            warn!(file = %file.display(), %why, "a workflow file does not read");
            let mut s = self.files.lock();
            s.refuse(file, at, vec![format!("{}: {why}", file.display())]);
            self.files.keep(&s);
            return false;
        }
        // Known as this file's before it is stored, so it is held off from the first moment.
        {
            let mut s = self.files.lock();
            s.by_id.insert(id.clone(), file.to_owned());
            s.hashes.insert(file.to_owned(), hash.clone());
            s.runs.insert(file.to_owned(), met.clone());
            s.errors.remove(file);
            self.files.keep(&s);
        }
        let mut stored = met.clone();
        if !trusted {
            stored["enabled"] = json!(false);
        }
        let w = id.clone();
        let (changed, row) = self
            .db
            .run(move |s| {
                let changed = upsert(s, &stored).unwrap_or(false);
                (changed, s.call("dbGetWorkflow", json!([w])).ok())
            })
            .await
            .unwrap_or((false, None));
        let (mut at, mut hash) = (at, hash);
        if met != theirs {
            let text = vorn_workflow_file::update(&text, &met, &bindings);
            let (f, t) = (file.to_owned(), text.clone());
            if off_runtime(move || atomic::replace(&f, t.as_bytes()))
                .await
                .is_ok()
            {
                at = stamp(file);
                hash = digest(&text);
            }
        }
        // The base is the row as stored, defaults and all, so a plain edit of the file meets nothing.
        let base = row
            .filter(Value::is_object)
            .map(as_file)
            .map(|b| if trusted { b } else { enabled_as(b, &met) })
            .unwrap_or(met);
        let renamed: Vec<String> = {
            let mut s = self.files.lock();
            let renamed: Vec<String> = s
                .by_id
                .iter()
                .filter(|(other, p)| p.as_path() == file && **other != id)
                .map(|(other, _)| other.clone())
                .collect();
            for other in &renamed {
                s.by_id.remove(other);
            }
            s.base.insert(file.to_owned(), base);
            if trusted {
                s.trusted.insert(file.to_owned(), hash.clone());
            }
            s.hashes.insert(file.to_owned(), hash);
            if let Some(at) = at {
                s.stamps.insert(file.to_owned(), at);
            }
            s.errors.remove(file);
            self.files.keep(&s);
            renamed
        };
        // The file now names another id: the workflow it named before goes.
        for other in &renamed {
            let w = other.clone();
            self.db
                .run(move |s| {
                    let _ = s.call("dbDeleteWorkflow", json!([w]));
                })
                .await;
        }
        if changed {
            info!(file = %file.display(), workflow = %id, "a workflow was read from its file");
        }
        changed || !renamed.is_empty()
    }

    /// Writes workflow `id` back to its file, when it came from one,
    /// keeping the file's comments and layout where nothing changed.
    pub(crate) async fn write_back(&self, id: &str) {
        let Some(file) = self.files.file_of(id) else {
            return;
        };
        let w = id.to_owned();
        let Some(row) = self
            .db
            .run(move |s| s.call("dbGetWorkflow", json!([w])).ok())
            .await
            .flatten()
            .filter(Value::is_object)
        else {
            return;
        };
        let project = projects(self)
            .await
            .into_iter()
            .find(|(_, p)| file.starts_with(Path::new(p).join(FOLDER)));
        let conns = connections(self).await;
        let bindings = Bindings {
            connections: &conns,
            project: project.as_ref().map(|(n, p)| (n.as_str(), p.as_str())),
        };
        let old = std::fs::read_to_string(&file).unwrap_or_default();
        // The file changed since it was last read: both edits meet.
        let (known, base, trusted) = {
            let s = self.files.lock();
            let trusted = s.trusts(&file) && s.hashes.get(&file) == Some(&digest(&old));
            (
                s.stamps.get(&file).copied(),
                s.base.get(&file).cloned(),
                trusted,
            )
        };
        // Off in the store only: the file keeps its own setting, and one that does not read is left alone.
        let ours = match (&base, trusted) {
            (_, true) => as_file(row),
            (Some(base), false) => enabled_as(as_file(row), base),
            (None, false) => match vorn_workflow_file::read(&old, &bindings) {
                Ok(file) => enabled_as(as_file(row), &file.workflow),
                Err(_) => return,
            },
        };
        let mut next = ours.clone();
        if known != stamp(&file) {
            if let (Some(base), Ok(theirs)) = (base, vorn_workflow_file::read(&old, &bindings)) {
                let mut theirs = vorn_work::js_numbers(theirs.workflow);
                if theirs.get("id").is_none() {
                    theirs["id"] = json!(id);
                }
                next = vorn_workflow_file::meet(&base, &ours, &theirs);
                if next != ours {
                    let mut stored = next.clone();
                    if !trusted {
                        stored["enabled"] = json!(false);
                    }
                    self.db.run(move |s| upsert(s, &stored)).await;
                    self.workflows_changed();
                }
            }
        }
        let text = vorn_workflow_file::update(&old, &next, &bindings);
        if text != old {
            let (f, t) = (file.clone(), text.clone());
            if let Err(err) = off_runtime(move || atomic::replace(&f, t.as_bytes())).await {
                warn!(%err, file = %file.display(), "could not write a workflow back to its file");
                return;
            }
        }
        let hash = digest(&text);
        let mut s = self.files.lock();
        match runs(&next, &[], self.engine.registry()) {
            Ok(_) => {
                if let Some(at) = stamp(&file) {
                    s.stamps.insert(file.clone(), at);
                }
                if trusted {
                    s.trusted.insert(file.clone(), hash.clone());
                }
                s.hashes.insert(file.clone(), hash);
                s.runs.insert(file.clone(), next.clone());
            }
            Err(why) => {
                let shown = format!("{}: {why}", file.display());
                s.refuse(&file, stamp(&file), vec![shown]);
            }
        }
        s.base.insert(file, next);
        self.files.keep(&s);
    }

    /// `workflow:files`: [`Files::listed`] with each agent step's host as its project has it now.
    pub(crate) async fn files_listed(&self) -> Value {
        self.files
            .listed(&project_rows(self).await, self.engine.registry())
    }

    /// Removes the file of workflow `id`, as the workflow was deleted.
    pub(crate) fn forget_file(&self, id: &str) {
        let mut s = self.files.lock();
        let Some(file) = s.by_id.remove(id) else {
            return;
        };
        if let Err(err) = std::fs::remove_file(&file) {
            warn!(%err, file = %file.display(), "could not remove a deleted workflow's file");
        }
        s.forget(&file);
        self.files.keep(&s);
    }

    /// `workflow:saveToFile`: workflow `id` written to `.vorn/workflows/` of
    /// `project`, and from then on read from there and written back to it.
    pub(crate) async fn save_to_file(&self, id: &str, project: &str) -> Result<Value, String> {
        if let Some(file) = self.files.file_of(id) {
            return Ok(json!({ "path": file, "created": false }));
        }
        let (name, path) = projects(self)
            .await
            .into_iter()
            .find(|(n, _)| n == project)
            .ok_or_else(|| format!("No project named {project}"))?;
        let w = id.to_owned();
        let row = self
            .db
            .run(move |s| s.call("dbGetWorkflow", json!([w])).ok())
            .await
            .flatten()
            .filter(Value::is_object)
            .ok_or_else(|| format!("Workflow {id} not found"))?;
        let dir = Path::new(&path).join(FOLDER);
        let base = vorn_workflow_file::file_name(row["name"].as_str().unwrap_or("workflow"));
        let ext = format!(".{}", vorn_workflow_file::EXTENSION);
        let stem = base.trim_end_matches(&ext).to_owned();
        let conns = connections(self).await;
        let bindings = Bindings {
            connections: &conns,
            project: Some((&name, &path)),
        };
        let text = vorn_workflow_file::write(&as_file(row), &bindings);
        let mut file = dir.join(&base);
        // A link, even one to nothing, is taken too: it is never written through.
        for n in 2.. {
            let (f, t) = (file.clone(), text.clone());
            match off_runtime(move || atomic::create(&f, t.as_bytes())).await {
                Ok(()) => break,
                Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists => {
                    file = dir.join(format!("{stem}-{n}{ext}"));
                }
                Err(e) => return Err(format!("Could not write {}: {e}", file.display())),
            }
        }
        let read = vorn_workflow_file::read(&text, &bindings)
            .ok()
            .map(|r| vorn_work::js_numbers(r.workflow));
        let mut s = self.files.lock();
        s.by_id.insert(id.to_owned(), file.clone());
        // Made in Vorn, so it stays on as it is.
        let hash = digest(&text);
        s.trusted.insert(file.clone(), hash.clone());
        s.hashes.insert(file.clone(), hash);
        if let Some(at) = stamp(&file) {
            s.stamps.insert(file.clone(), at);
        }
        if let Some(read) = read {
            if runs(&read, &[], self.engine.registry()).is_ok() {
                s.runs.insert(file.clone(), read.clone());
            }
            s.base.insert(file.clone(), read);
        }
        self.files.keep(&s);
        info!(file = %file.display(), workflow = %id, "a workflow was saved to a file");
        Ok(json!({ "path": file, "created": true }))
    }

    /// Adds, once each, the workflows the extensions that are on ship, held
    /// to what each extension may seed. One from an extension that does not
    /// declare `process.run` arrives off, so it runs only once switched on.
    pub(crate) async fn seed_contributed(&self, contributed: Vec<(String, String, String)>) {
        let Some(native) = self.native.upgrade() else {
            return;
        };
        let Some(router) = native.extensions().map(|e| Arc::clone(e.router())) else {
            return;
        };
        let conns = connections(self).await;
        for (extension, id, text) in contributed {
            let key = format!("{extension}/{id}");
            if self.files.lock().seeded.contains(&key) {
                continue;
            }
            let bindings = Bindings {
                connections: &conns,
                project: None,
            };
            let mut wf = match vorn_workflow_file::read(&text, &bindings) {
                Ok(r) => r.workflow,
                Err(errors) => {
                    warn!(%extension, workflow = %id, errors = ?errors, "a shipped workflow does not read");
                    continue;
                }
            };
            // Namespaced by its extension, so removing the extension can find it.
            let stored_id = format!(
                "{extension}:{}",
                wf.get("id").and_then(Value::as_str).unwrap_or(&id)
            );
            wf["id"] = json!(stored_id);
            let Ok(pack) = router.pack(&extension) else {
                continue;
            };
            let caller = router.caller_of(&pack).await;
            let runs = |t: &str| crate::native::extensions::world::runs_commands(&native, t);
            if let Err(err) = vorn_extensions::gate::admit_seed(&caller, &wf, runs) {
                warn!(%extension, workflow = %stored_id, "a shipped workflow is not added: {}", err.message);
                continue;
            }
            if !caller.runs_commands {
                wf["enabled"] = json!(false);
            }
            let row = wf.clone();
            let added = self
                .db
                .run(move |s| {
                    let fresh = s.call("dbGetWorkflow", json!([row["id"]])).ok()?.is_null();
                    if fresh {
                        s.call("dbInsertWorkflow", json!([with_defaults(row)]))
                            .ok()?;
                    }
                    Some(fresh)
                })
                .await
                .flatten()
                .unwrap_or(false);
            let mut s = self.files.lock();
            s.seeded.insert(key);
            self.files.keep(&s);
            if added {
                info!(%extension, workflow = %stored_id, "a workflow an extension ships was added");
                self.workflows_changed();
            }
        }
    }
}

/// A workflow as the store takes it: the columns it cannot leave empty filled.
fn with_defaults(mut wf: Value) -> Value {
    let map = match wf.as_object_mut() {
        Some(m) => m,
        None => return wf,
    };
    for (k, v) in [
        ("name", json!("Workflow")),
        ("icon", json!("Zap")),
        ("iconColor", json!("#6366f1")),
        ("enabled", json!(true)),
        ("nodes", json!([])),
        ("edges", json!([])),
    ] {
        map.entry(k).or_insert(v);
    }
    wf
}

/// Writes `wf` to the store, adding it or changing what differs; whether
/// anything changed.
fn upsert(s: &mut vorn_store::Store, wf: &Value) -> Option<bool> {
    let id = wf.get("id")?.clone();
    let current = s.call("dbGetWorkflow", json!([id])).ok()?;
    if current.is_null() {
        s.call("dbInsertWorkflow", json!([with_defaults(wf.clone())]))
            .ok()?;
        return Some(true);
    }
    let current = vorn_work::js_numbers(current);
    let mut updates = Map::new();
    for key in [
        "name",
        "icon",
        "iconColor",
        "nodes",
        "edges",
        "enabled",
        "staggerDelayMs",
        "workspaceId",
    ] {
        if let Some(v) = wf.get(key) {
            if current.get(key) != Some(v) {
                updates.insert(key.to_owned(), v.clone());
            }
        }
    }
    if updates.is_empty() {
        return Some(false);
    }
    s.call("dbUpdateWorkflow", json!([id, updates])).ok()?;
    Some(true)
}

#[cfg(test)]
mod tests {
    use super::*;
    use vorn_workflow::NodeType;

    #[test]
    fn an_extension_takes_back_its_namespace_from_files_not_turned_on() {
        let dir = tempfile::tempdir().unwrap();
        let files = Files::new(dir.path());
        let at = (SystemTime::UNIX_EPOCH, 1);
        let [x, y, plain, bad] =
            ["x.toml", "y.toml", "p.toml", "bad.toml"].map(|f| dir.path().join(f));
        {
            let mut s = files.lock();
            for (id, file) in [("acme:x", &x), ("acme:y", &y), ("acme-plain", &plain)] {
                s.by_id.insert(id.into(), file.clone());
                s.hashes.insert(file.clone(), "h".into());
                s.stamps.insert(file.clone(), at);
            }
            s.trusted.insert(y.clone(), "h".into());
            s.refuse(&bad, Some(at), vec!["bad".into()]);
        }
        assert!(files.take_owned(|id| id.starts_with("other:")).is_empty());
        assert_eq!(files.take_owned(|id| id.starts_with("acme:")), ["acme:x"]);
        let s = files.lock();
        assert_eq!(s.by_id.keys().collect::<Vec<_>>(), ["acme-plain", "acme:y"]);
        assert!(!s.stamps.contains_key(&x) && !s.hashes.contains_key(&x));
        assert!(
            !s.stamps.contains_key(&bad),
            "a file that did not read is read again"
        );
        assert!(s.stamps.contains_key(&y) && s.stamps.contains_key(&plain));
    }

    /// The built-in steps and one extension's, with the agent step as the fallback, as vornd declares them.
    fn types() -> Registry<VorndHost> {
        let types = Registry::default();
        let builtin = KINDS
            .iter()
            .filter(|k| NodeKind::parse(k.id).step().is_some())
            .map(|k| (k.id, vorn_workflows::ID));
        for (id, provider) in builtin.chain([("some.ext/step", "some.ext")]) {
            let node = NodeType {
                id: id.to_owned(),
                provider: provider.to_owned(),
                provider_name: provider.to_owned(),
                doc: Value::Null,
            };
            types.add_node(node, None);
        }
        types.set_fallback(Some(vorn_workflows::FALLBACK));
        types
    }

    fn node(node: Value) -> Result<Vec<String>, String> {
        let projects = [
            json!({ "name": "far", "path": "/f", "hostIds": ["local", "box"] }),
            json!({ "name": "near", "path": "/n", "hostIds": ["local"] }),
        ];
        runs(&json!({ "nodes": [node] }), &projects, &types())
    }

    fn one(kind: &str, config: Value) -> Result<Vec<String>, String> {
        node(json!({ "id": "s", "type": kind, "label": "Do\nit", "config": config }))
    }

    #[test]
    fn lists_everything_a_step_runs_or_sends() {
        assert_eq!(
            one(
                "launchAgent",
                json!({ "agentType": "claude", "prompt": "fix it", "args": ["--x"],
                        "model": "m", "projectName": "far", "projectPath": "/f",
                        "useWorktree": true, "existingWorktreePath": "/f/w",
                        "displayName": "d", "timeoutMs": 5 })
            )
            .unwrap(),
            [
                "Do it (launchAgent)",
                "  agentType: claude",
                "  prompt: fix it",
                "  args: [\"--x\"]",
                "  model: m",
                "  projectName: far",
                "  projectPath: /f",
                "  useWorktree: true",
                "  existingWorktreePath: /f/w",
                "  remote host: box",
            ]
        );
        assert_eq!(
            one("launchAgent", json!({ "projectName": "near" })).unwrap()[2..],
            ["  remote host: none, this computer"]
        );
        assert_eq!(
            one(
                "launchAgent",
                json!({ "projectName": "{{context.projectName}}" })
            )
            .unwrap()[2..],
            ["  remote host: its project's, when it runs"]
        );
        assert_eq!(
            one(
                "script",
                json!({ "scriptType": "bash", "scriptContent": "a\nb", "args": ["1"],
                        "projectPath": "/p", "secretsFrom": "c1", "runId": "r" })
            )
            .unwrap()[2..],
            [
                "  scriptContent: a\nb",
                "  args: [\"1\"]",
                "  projectPath: /p",
                "  secretsFrom: c1",
            ]
        );
        assert_eq!(
            one(
                "httpRequest",
                json!({ "nodeType": "httpRequest", "method": "POST", "url": "https://x",
                        "headers": { "a": "b" }, "body": "{}", "profileConnectionId": "c" })
            )
            .unwrap()[1..],
            [
                "  method: POST",
                "  url: https://x",
                "  headers: {\"a\":\"b\"}",
                "  body: {}",
                "  profileConnectionId: c",
            ]
        );
        assert_eq!(
            one(
                "callConnectorAction",
                json!({ "connectionId": "c", "action": "send", "args": { "to": "x" },
                        "actionLabel": "Send" })
            )
            .unwrap()[1..],
            [
                "  action: send",
                "  connectionId: c",
                "  args: {\"to\":\"x\"}"
            ]
        );
        assert_eq!(
            one(
                "createTaskFromItem",
                json!({ "project": "fromConnection", "initialStatus": "todo" })
            )
            .unwrap()[1..],
            ["  project: fromConnection", "  initialStatus: todo"]
        );
        // An extension's step shows its type as the action and every input.
        assert_eq!(
            one("some.ext/step", json!({ "command": "make", "cwd": "/w" })).unwrap(),
            ["Do it (some.ext/step)", "  command: make", "  cwd: /w"]
        );
        assert_eq!(
            one(
                "trigger",
                json!({ "triggerType": "webhook", "method": "POST" })
            )
            .unwrap(),
            ["Trigger: webhook", "  method: POST"]
        );
        assert!(one("approval", json!({ "message": "ok?" }))
            .unwrap()
            .is_empty());
    }

    #[test]
    fn refuses_a_setting_its_step_does_not_read() {
        for (kind, config) in [
            ("launchAgent", json!({ "command": "x" })),
            ("launchAgent", json!({ "remoteHostId": "box" })),
            ("script", json!({ "prompt": "x" })),
            ("script", json!({ "env": { "A": "1" } })),
            ("httpRequest", json!({ "scriptContent": "x" })),
            ("callConnectorAction", json!({ "url": "x" })),
            ("condition", json!({ "url": "x" })),
            ("approval", json!({ "anything": 1 })),
            ("loop", json!({ "shell": "sh" })),
            ("agent", json!({ "command": "x" })),
        ] {
            let why = one(kind, config).unwrap_err();
            let runs_as = if kind == "agent" { "launchAgent" } else { kind };
            assert!(why.contains(&format!("of a {runs_as} step")), "{why}");
        }
        let untyped = node(json!({ "id": "s", "config": { "command": "x" } }));
        assert!(untyped.unwrap_err().contains("of a launchAgent step"));
        assert!(one("script", json!("echo")).is_err());
    }

    #[test]
    fn lists_a_step_of_a_type_nothing_declares_as_the_agent_it_runs_as() {
        let config = json!({ "prompt": "go", "projectName": "far" });
        let shown = [
            "Do it (launchAgent)",
            "  prompt: go",
            "  projectName: far",
            "  remote host: box",
        ];
        assert_eq!(one("agent", config.clone()).unwrap(), shown);
        let untyped = node(json!({ "id": "s", "label": "Do it", "config": config }));
        assert_eq!(untyped.unwrap(), shown);
    }

    #[test]
    fn reads_a_loop_that_runs_its_items_side_by_side() {
        let config = json!({ "mode": "forEach", "items": "a", "bodyNodeIds": ["b"],
                             "parallel": true, "concurrency": 2 });
        assert!(one("loop", config).unwrap().is_empty());
    }

    #[test]
    fn reads_the_workflows_vorn_ships() {
        for (_, _, text) in vorn_workflows::SEEDS {
            let bindings = Bindings {
                connections: &[],
                project: None,
            };
            let wf = vorn_workflow_file::read(text, &bindings).unwrap().workflow;
            runs(&vorn_work::js_numbers(wf), &[], &types()).unwrap();
        }
    }
}

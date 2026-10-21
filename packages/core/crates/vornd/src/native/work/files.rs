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
    /// What each file's workflow runs, for the user to see before turning it on.
    runs: HashMap<PathBuf, Vec<String>>,
}

impl State {
    /// Whether `file` is as the user turned it on.
    fn trusts(&self, file: &Path) -> bool {
        matches!((self.trusted.get(file), self.hashes.get(file)), (Some(t), Some(h)) if t == h)
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

/// One line per thing a workflow does when it runs: its trigger, and each
/// step with the command, script or prompt it runs.
pub fn runs(wf: &Value) -> Vec<String> {
    let first = |v: Option<&Value>| {
        v.and_then(Value::as_str)
            .map(str::trim)
            .filter(|t| !t.is_empty())
            .map(str::to_owned)
    };
    wf.get("nodes")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .map(|node| {
            let kind = node.get("type").and_then(Value::as_str).unwrap_or("step");
            let config = node.get("config").unwrap_or(&Value::Null);
            let label = first(node.get("label")).unwrap_or_else(|| kind.to_owned());
            if kind == "trigger" {
                let on = first(config.get("triggerType")).unwrap_or_else(|| "manual".into());
                return format!("Trigger: {on}");
            }
            let agent = first(config.get("agentType"));
            let what = first(config.get("scriptContent"))
                .or_else(|| first(config.get("command")))
                .or_else(|| first(config.get("prompt")));
            match (agent, what) {
                (Some(agent), Some(what)) => format!("{label} ({agent}): {what}"),
                (Some(agent), None) => format!("{label} ({agent})"),
                (None, Some(what)) => format!("{label}: {what}"),
                (None, None) => format!("{label} ({kind})"),
            }
        })
        .collect()
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
        let written = self
            .record
            .parent()
            .map_or(Ok(()), std::fs::create_dir_all)
            .and_then(|()| {
                std::fs::write(
                    &self.record,
                    serde_json::to_vec_pretty(&kept).unwrap_or_default(),
                )
            });
        if let Err(err) = written {
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

    /// Turns on workflow `id`'s file as it is now; whether it came from one.
    pub fn trust(&self, id: &str) -> bool {
        let mut s = self.lock();
        let Some(file) = s.by_id.get(id).cloned() else {
            return false;
        };
        let Some(hash) = s.hashes.get(&file).cloned() else {
            return false;
        };
        s.trusted.insert(file, hash);
        self.keep(&s);
        true
    }

    /// Every file, whether it is on and what it runs, and what is wrong
    /// with the ones that do not read.
    pub fn listed(&self) -> Value {
        let s = self.lock();
        let files: Vec<Value> = s
            .by_id
            .iter()
            .map(|(id, path)| {
                json!({
                    "workflowId": id,
                    "path": path,
                    "trusted": s.trusts(path),
                    "runs": s.runs.get(path).cloned().unwrap_or_default(),
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

/// Each project's name and path.
async fn projects(work: &Work) -> Vec<(String, String)> {
    work.db
        .run(|s| s.call("dbListProjects", json!([])).ok())
        .await
        .flatten()
        .and_then(|v| v.as_array().cloned())
        .unwrap_or_default()
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
                && p.is_file()
        })
        .collect();
    out.sort();
    out
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

    /// Reads `file` of project `name` at `path` into its stored workflow;
    /// whether the workflow changed.
    async fn read_file(&self, name: &str, path: &str, file: &Path) -> bool {
        let at = stamp(file);
        let Ok(text) = std::fs::read_to_string(file) else {
            return false;
        };
        let hash = digest(&text);
        let trusted = self.files.lock().trusted.get(file) == Some(&hash);
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
                if let Some(at) = at {
                    s.stamps.insert(file.to_owned(), at);
                }
                s.errors.insert(file.to_owned(), lines);
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
            s.errors.insert(
                file.to_owned(),
                vec![format!(
                    "{}: its id {id} is the workflow of {other} already",
                    file.display(),
                )],
            );
            if let Some(at) = at {
                s.stamps.insert(file.to_owned(), at);
            }
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
                    .map(|ours| if trusted { ours } else { enabled_as(ours, &base) });
                match ours {
                    Some(ours) if ours != base => vorn_workflow_file::meet(&base, &ours, &theirs),
                    _ => theirs.clone(),
                }
            }
            None => theirs.clone(),
        };
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
            if std::fs::write(file, &text).is_ok() {
                at = stamp(file);
                hash = digest(&text);
            }
        }
        let ran = runs(&met);
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
            s.by_id.insert(id.clone(), file.to_owned());
            s.base.insert(file.to_owned(), base);
            if trusted {
                s.trusted.insert(file.to_owned(), hash.clone());
            }
            s.hashes.insert(file.to_owned(), hash);
            s.runs.insert(file.to_owned(), ran);
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
            (s.stamps.get(&file).copied(), s.base.get(&file).cloned(), trusted)
        };
        let ours = match (&base, trusted) {
            (Some(base), false) => enabled_as(as_file(row), base),
            _ => as_file(row),
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
            if let Err(err) = std::fs::write(&file, &text) {
                warn!(%err, file = %file.display(), "could not write a workflow back to its file");
                return;
            }
        }
        let hash = digest(&text);
        let mut s = self.files.lock();
        if let Some(at) = stamp(&file) {
            s.stamps.insert(file.clone(), at);
        }
        if trusted {
            s.trusted.insert(file.clone(), hash.clone());
        }
        s.hashes.insert(file.clone(), hash);
        s.runs.insert(file.clone(), runs(&next));
        s.base.insert(file, next);
        self.files.keep(&s);
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
        let mut file = dir.join(&base);
        let mut n = 2;
        while file.exists() {
            file = dir.join(format!("{stem}-{n}{ext}"));
            n += 1;
        }
        let conns = connections(self).await;
        let bindings = Bindings {
            connections: &conns,
            project: Some((&name, &path)),
        };
        let text = vorn_workflow_file::write(&as_file(row), &bindings);
        std::fs::create_dir_all(&dir)
            .map_err(|e| format!("Could not make {}: {e}", dir.display()))?;
        std::fs::write(&file, &text)
            .map_err(|e| format!("Could not write {}: {e}", file.display()))?;
        let mut s = self.files.lock();
        s.by_id.insert(id.to_owned(), file.clone());
        // Made in Vorn, so it stays on as it is.
        let hash = digest(&text);
        s.trusted.insert(file.clone(), hash.clone());
        s.hashes.insert(file.clone(), hash);
        if let Some(at) = stamp(&file) {
            s.stamps.insert(file.clone(), at);
        }
        if let Ok(read) = vorn_workflow_file::read(
            &std::fs::read_to_string(&file).unwrap_or_default(),
            &bindings,
        ) {
            let read = vorn_work::js_numbers(read.workflow);
            s.runs.insert(file.clone(), runs(&read));
            s.base.insert(file.clone(), read);
        }
        self.files.keep(&s);
        info!(file = %file.display(), workflow = %id, "a workflow was saved to a file");
        Ok(json!({ "path": file, "created": true }))
    }

    /// Adds, once each, the workflows the extensions that are on ship.
    pub(crate) async fn seed_contributed(&self, contributed: Vec<(String, String, String)>) {
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

//! Saved layouts: `layout:list`, `get`, `open` and `save`.
//!
//! A layout is a file under `<data dir>/layouts/`, or one an extension
//! contributes (`[[contributes.layouts]]`). Opening starts its sessions through
//! the same calls the app makes (`terminal:create`, `shell:create`) and opens
//! its browser panes with `browser:openPane`, so a session started from a
//! layout is a session like any other.
//!
//! The tree maps onto the app's arrangement as its leaves in order: sessions
//! are started in tree order, which is the order the grid and tabs show them
//! in. Split directions and ratios are kept in the file but the server has no
//! per-card size to set, so they do not change what opens.

use std::path::{Path, PathBuf};
use std::sync::Arc;

use serde_json::{json, Value};
use vorn_agents::launch::{Platform, Quoting};
use vorn_agents::Agent;
use vorn_layout::{Direction, Launch, Layout, Leaf, Node, Pane, Session};

use super::config::Viewer;
use super::{Answer, Native};
use crate::registry::Registry;

pub const METHODS: &[&str] = &["layout:list", "layout:get", "layout:open", "layout:save"];

/// Where the person's layouts are kept; none before the database is set.
fn dir(native: &Native) -> Option<PathBuf> {
    native.db.get()?.parent().map(|d| d.join("layouts"))
}

fn agents() -> Vec<&'static str> {
    Agent::ALL.iter().map(|a| a.id()).collect()
}

/// A layout found, with where it came from.
#[derive(Debug, Clone)]
struct Found {
    name: String,
    /// `user`, or an extension's id.
    source: String,
    title: Option<String>,
    path: PathBuf,
}

fn found(native: &Native) -> Vec<Found> {
    let mut out = Vec::new();
    if let Some(dir) = dir(native) {
        let mut names: Vec<_> = std::fs::read_dir(&dir)
            .into_iter()
            .flatten()
            .flatten()
            .filter_map(|e| {
                let path = e.path();
                let name = path.file_stem()?.to_str()?.to_owned();
                (path.extension()? == "toml" && vorn_layout::valid_name(&name))
                    .then_some((name, path))
            })
            .collect();
        names.sort();
        out.extend(names.into_iter().map(|(name, path)| Found {
            name,
            source: "user".into(),
            title: None,
            path,
        }));
    }
    if let Some(host) = native.extensions.get() {
        out.extend(
            host.layout_files()
                .into_iter()
                .map(|(ext, name, title, path)| Found {
                    name,
                    source: ext,
                    title: Some(title),
                    path,
                }),
        );
    }
    out
}

/// The layout `name` (and `source`, when several offer it), or why not.
fn pick(native: &Native, params: &Value) -> Result<Found, String> {
    let name = params.get("name").and_then(Value::as_str).unwrap_or("");
    let source = params.get("source").and_then(Value::as_str);
    let all = found(native);
    let mut hits: Vec<&Found> = all
        .iter()
        .filter(|f| f.name == name && source.is_none_or(|s| s == f.source))
        .collect();
    // A person's own layout is the one a bare name means.
    if source.is_none() && hits.iter().any(|f| f.source == "user") {
        hits.retain(|f| f.source == "user");
    }
    match hits.as_slice() {
        [] => Err(format!("no layout named \"{name}\"")),
        [one] => Ok((*one).clone()),
        many => Err(format!(
            "\"{name}\" is offered by {}; say which with source",
            many.iter()
                .map(|f| f.source.as_str())
                .collect::<Vec<_>>()
                .join(", ")
        )),
    }
}

fn read(found: &Found) -> Result<(String, Layout), String> {
    let text = std::fs::read_to_string(&found.path)
        .map_err(|e| format!("could not read {}: {e}", found.path.display()))?;
    let file = found.path.display().to_string();
    let layout = vorn_layout::parse(&text, &agents()).map_err(|problems| {
        problems
            .iter()
            .map(|p| p.at(&file))
            .collect::<Vec<_>>()
            .join("\n")
    })?;
    Ok((text, layout))
}

pub async fn answer(native: &Arc<Native>, method: &str, params: Value, viewer: &Viewer) -> Answer {
    match method {
        "layout:list" => list(native),
        "layout:get" => {
            match pick(native, &params).and_then(|f| read(&f).map(|(text, _)| (f, text))) {
                Ok((f, text)) => {
                    Answer::Result(json!({ "name": f.name, "source": f.source, "toml": text }))
                }
                Err(e) => Answer::Error(e),
            }
        }
        "layout:open" => open(native, &params, viewer).await,
        "layout:save" => save(native, &params, viewer).await,
        _ => Answer::Unanswered,
    }
}

fn list(native: &Native) -> Answer {
    let rows = found(native)
        .into_iter()
        .map(|f| match read(&f) {
            Ok((_, l)) => json!({
                "name": f.name, "source": f.source, "title": f.title, "project": l.project,
                "sessions": l.leaves().iter().filter(|l| matches!(l, Leaf::Session(_))).count(),
            }),
            Err(e) => json!({ "name": f.name, "source": f.source, "title": f.title, "error": e }),
        })
        .collect();
    Answer::Result(Value::Array(rows))
}

/// A project's name and path from `wanted` (a name, or a path), against the configured projects.
async fn project(
    native: &Arc<Native>,
    wanted: &str,
    viewer: &Viewer,
) -> Result<(String, String, bool), String> {
    let config = match native
        .answer("config:load".into(), Value::Null, viewer)
        .await
    {
        Answer::Result(c) => c,
        _ => Value::Null,
    };
    let known: Vec<&Value> = config
        .get("projects")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .collect();
    let field = |p: &Value, k: &str| p.get(k).and_then(Value::as_str).unwrap_or("").to_owned();
    let is_path = wanted.contains(['/', '\\']) || wanted.starts_with('~');
    if is_path {
        if !Path::new(wanted).is_absolute() {
            return Err(format!("project path \"{wanted}\" is not absolute"));
        }
        let registered = known.iter().find(|p| field(p, "path") == wanted);
        let name = registered.map(|p| field(p, "name")).unwrap_or_else(|| {
            Path::new(wanted)
                .file_name()
                .map_or_else(|| wanted.to_owned(), |n| n.to_string_lossy().into_owned())
        });
        return Ok((name, wanted.to_owned(), registered.is_some()));
    }
    known
        .iter()
        .find(|p| field(p, "name") == wanted)
        .map(|p| (field(p, "name"), field(p, "path"), true))
        .ok_or_else(|| format!("no project named \"{wanted}\""))
}

fn resolve_cwd(cwd: Option<&str>, project: &str) -> String {
    match cwd {
        None | Some("") => project.to_owned(),
        Some(c) if Path::new(c).is_absolute() => c.to_owned(),
        Some(c) => Path::new(project).join(c).to_string_lossy().into_owned(),
    }
}

fn command_line(words: &[String]) -> String {
    let quoting = Quoting::local(Platform::HOST, "");
    words
        .iter()
        .map(|w| quoting.quote(w))
        .collect::<Vec<_>>()
        .join(" ")
}

async fn open(native: &Arc<Native>, params: &Value, viewer: &Viewer) -> Answer {
    let (found, layout) = match pick(native, params).and_then(|f| read(&f).map(|(_, l)| (f, l))) {
        Ok(ok) => ok,
        Err(e) => return Answer::Error(e),
    };
    let wanted = params
        .get("project")
        .and_then(Value::as_str)
        .filter(|p| !p.is_empty())
        .or(layout.project.as_deref())
        .or_else(|| {
            params
                .get("defaultProject")
                .and_then(Value::as_str)
                .filter(|p| !p.is_empty())
        });
    let Some(wanted) = wanted else {
        return Answer::Error(format!(
            "layout \"{}\" names no project; say which with project",
            found.name
        ));
    };
    let (project_name, project_path, _) = match project(native, wanted, viewer).await {
        Ok(p) => p,
        Err(e) => return Answer::Error(e),
    };
    let mut sessions = Vec::new();
    let mut panes = Vec::new();
    let mut errors: Vec<String> = Vec::new();
    // The session a pane belongs beside: the one started just before it.
    let mut last: Option<String> = None;
    for leaf in layout.leaves() {
        match leaf {
            Leaf::Session(s) => {
                let label = s.label.clone();
                let cwd = resolve_cwd(s.cwd.as_deref(), &project_path);
                let (method, ask) = match &s.launch {
                    Launch::Agent(agent) => {
                        let mut ask = json!({
                            "agentType": agent, "projectName": project_name, "projectPath": cwd,
                        });
                        if let Some(label) = &label {
                            ask["displayName"] = json!(label);
                        }
                        ("terminal:create", ask)
                    }
                    Launch::Shell | Launch::Command(_) => {
                        let mut ask = json!({ "cwd": cwd });
                        if let Launch::Command(words) = &s.launch {
                            ask["command"] = json!(command_line(words));
                        }
                        if let Some(label) = &label {
                            ask["displayName"] = json!(label);
                        }
                        ("shell:create", ask)
                    }
                };
                match native.answer(method.into(), ask, viewer).await {
                    Answer::Result(record) => {
                        let id = record
                            .get("id")
                            .and_then(Value::as_str)
                            .unwrap_or("")
                            .to_owned();
                        sessions.push(json!({
                            "id": id, "label": label,
                            "agent": record.get("agentType"), "cwd": cwd,
                        }));
                        last = Some(id);
                    }
                    Answer::Error(e) => errors.push(format!("{}: {e}", describe(s))),
                    _ => errors.push(format!("{}: not started", describe(s))),
                }
            }
            Leaf::Pane(p) => {
                panes.push(open_pane(native, p, last.as_deref(), viewer, &mut errors).await)
            }
        }
    }
    Answer::Result(json!({
        "layout": found.name, "source": found.source,
        "project": { "name": project_name, "path": project_path },
        "sessions": sessions, "panes": panes, "errors": errors,
    }))
}

fn describe(s: &Session) -> String {
    match &s.launch {
        Launch::Agent(a) => a.clone(),
        Launch::Shell => "shell".to_owned(),
        Launch::Command(c) => c.join(" "),
    }
}

async fn open_pane(
    native: &Arc<Native>,
    pane: &Pane,
    beside: Option<&str>,
    viewer: &Viewer,
    errors: &mut Vec<String>,
) -> Value {
    let Some(session) = beside else {
        let reason = "a pane opens beside a session, and none came before it";
        errors.push(format!("{}: {reason}", pane.kind));
        return json!({ "kind": pane.kind, "opened": false, "reason": reason });
    };
    let ask = json!({ "sessionId": session, "url": pane.url });
    match native.answer("browser:openPane".into(), ask, viewer).await {
        Answer::Result(_) | Answer::Void => {
            json!({ "kind": pane.kind, "session": session, "opened": true })
        }
        other => {
            let reason = match other {
                Answer::Error(e) => e,
                _ => "the app is not open".to_owned(),
            };
            errors.push(format!("{}: {reason}", pane.kind));
            json!({ "kind": pane.kind, "session": session, "opened": false, "reason": reason })
        }
    }
}

/// `layout:save {name, project, overwrite?}`: the project's open sessions, in order.
async fn save(native: &Arc<Native>, params: &Value, viewer: &Viewer) -> Answer {
    let name = params.get("name").and_then(Value::as_str).unwrap_or("");
    if !vorn_layout::valid_name(name) {
        return Answer::Error("a layout's name is letters, digits, `.`, `_` and `-`".to_owned());
    }
    let Some(wanted) = params
        .get("project")
        .and_then(Value::as_str)
        .filter(|p| !p.is_empty())
    else {
        return Answer::Error("layout:save needs project".to_owned());
    };
    let (project_name, project_path, registered) = match project(native, wanted, viewer).await {
        Ok(p) => p,
        Err(e) => return Answer::Error(e),
    };
    let Some(dir) = dir(native) else {
        return Answer::Error("vornd has no data directory yet".to_owned());
    };
    let file = dir.join(format!("{name}.toml"));
    if file.exists() && params.get("overwrite") != Some(&Value::Bool(true)) {
        return Answer::Error(format!(
            "layout \"{name}\" exists; pass overwrite to replace it"
        ));
    }
    let Some(open) = native.registry.get().and_then(|r| {
        r.read(|r: &Registry| {
            r.terminals()
                .into_iter()
                .filter(|t| Path::new(&t.project_path).starts_with(&project_path))
                .map(|t| {
                    (
                        t.agent_type.clone(),
                        t.display_name.clone(),
                        t.shell_cwd.clone(),
                    )
                })
                .collect::<Vec<_>>()
        })
    }) else {
        return Answer::Error("vornd has not read the sessions yet".to_owned());
    };
    if open.is_empty() {
        return Answer::Error(format!("no sessions are open in {project_name}"));
    }
    let leaves: Vec<Node> = open
        .into_iter()
        .map(|(agent, label, cwd)| {
            let launch = match Agent::from_id(&agent) {
                Some(a) => Launch::Agent(a.id().to_owned()),
                None => Launch::Shell,
            };
            // Under the project, a path relative to it keeps the layout movable.
            let cwd = cwd.filter(|c| c != &project_path).map(|c| {
                Path::new(&c)
                    .strip_prefix(&project_path)
                    .map_or(c.clone(), |r| r.to_string_lossy().into_owned())
            });
            Node::Leaf(Leaf::Session(Session { launch, cwd, label }))
        })
        .collect();
    let count = leaves.len();
    let root = if count == 1 {
        leaves
            .into_iter()
            .next()
            .unwrap_or_else(|| unreachable!("one leaf"))
    } else {
        Node::Split {
            direction: Direction::Horizontal,
            ratio: 1.0 / count as f64,
            children: leaves,
        }
    };
    let layout = Layout {
        name: Some(name.to_owned()),
        // A project Vorn does not know is kept by its path, which names it anywhere.
        project: Some(if registered {
            project_name
        } else {
            project_path.clone()
        }),
        root,
    };
    let (text, to) = (layout.to_toml(), file.clone());
    let wrote = tokio::task::spawn_blocking(move || {
        std::fs::create_dir_all(to.parent().unwrap_or(Path::new(".")))?;
        std::fs::write(&to, text)
    })
    .await;
    match wrote {
        Ok(Ok(())) => Answer::Result(json!({
            "name": name, "path": file.to_string_lossy(), "sessions": count,
        })),
        Ok(Err(e)) => Answer::Error(format!("could not write {}: {e}", file.display())),
        Err(e) => Answer::Error(e.to_string()),
    }
}

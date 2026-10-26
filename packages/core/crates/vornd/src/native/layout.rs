//! Saved layouts: `layout:list`, `get`, `open` and `save`.
//!
//! A layout is a file under `<data dir>/layouts/`, or one an extension
//! contributes (`[[contributes.layouts]]`). Opening starts its sessions through
//! the same calls the app makes (`terminal:create`, `shell:create`) and opens
//! its browser panes with `browser:openPane`, so a session started from a
//! layout is a session like any other.
//!
//! Sessions are started in tree order. The layout's view (`flexible`,
//! `grid` or `tabs`) is then set for the project's workspace: flexible lays
//! the cards out as the tree, each split's direction and ratio as written.
//! It is kept in the viewer's `gridLayout` and `layoutMode` settings (the
//! desktop app's for the CLI) and told to the apps (`layout:opened`).
//! Saving reads them back: the sessions in the grid's tree, in its view.

use std::path::{Path, PathBuf};
use std::sync::Arc;

use std::collections::HashMap;

use serde_json::{json, Map, Value};
use tracing::warn;
use vorn_agents::launch::{Platform, Quoting};
use vorn_layout::{Direction, Launch, Layout, Leaf, Node, Pane, Session, View};

use super::config::{with_store, Viewer};
use super::{Answer, Native};
use crate::registry::Registry;

pub const METHODS: &[&str] = &["layout:list", "layout:open", "layout:save"];

/// Where the person's layouts are kept; none before the database is set.
fn dir(native: &Native) -> Option<PathBuf> {
    native.db.get()?.parent().map(|d| d.join("layouts"))
}

/// The ids of the agents that are on, which a layout may name.
fn agents(native: &Native) -> Vec<String> {
    native
        .adapters()
        .agents()
        .iter()
        .map(|a| a.id().to_owned())
        .collect()
}

/// A layout found, with where it came from.
#[derive(Debug, Clone)]
struct Found {
    name: String,
    /// `user`, or an extension's id.
    source: String,
    title: Option<String>,
    path: PathBuf,
    /// An extension's layout: its folder and the file in it, read without
    /// following a link below the folder.
    packed: Option<(PathBuf, String)>,
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
            packed: None,
        }));
    }
    if let Some(host) = native.extensions.get() {
        out.extend(
            host.layout_files()
                .into_iter()
                .map(|(ext, name, title, folder, file)| Found {
                    name,
                    source: ext,
                    title: Some(title),
                    path: folder.join(&file),
                    packed: Some((folder, file)),
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

fn read(native: &Native, found: &Found) -> Result<(String, Layout), String> {
    let text = match &found.packed {
        Some((folder, file)) => vorn_extensions::reach::read_in(folder, file)
            .and_then(|b| String::from_utf8(b).map_err(std::io::Error::other)),
        None => std::fs::read_to_string(&found.path),
    }
    .map_err(|e| format!("could not read {}: {e}", found.path.display()))?;
    let file = found.path.display().to_string();
    let agents = agents(native);
    let ids: Vec<&str> = agents.iter().map(String::as_str).collect();
    let layout = vorn_layout::parse(&text, &ids).map_err(|problems| {
        problems
            .iter()
            .map(|p| p.at(&file))
            .collect::<Vec<_>>()
            .join("\n")
    })?;
    Ok((text, layout))
}

/// Runs `work` on a blocking thread: layouts are files, and so are the manifests that offer more.
async fn blocking<T: Send + 'static>(
    work: impl FnOnce() -> T + Send + 'static,
) -> Result<T, String> {
    tokio::task::spawn_blocking(work)
        .await
        .map_err(|e| e.to_string())
}

pub async fn answer(native: &Arc<Native>, method: &str, params: Value, viewer: &Viewer) -> Answer {
    match method {
        "layout:list" => {
            let n = Arc::clone(native);
            blocking(move || list(&n))
                .await
                .unwrap_or_else(Answer::Error)
        }
        "layout:open" => open(native, &params, viewer).await,
        "layout:save" => save(native, &params, viewer).await,
        _ => Answer::Unanswered,
    }
}

fn list(native: &Native) -> Answer {
    let rows = found(native)
        .into_iter()
        .map(|f| match read(native, &f) {
            Ok((_, l)) => json!({
                "name": f.name, "source": f.source, "title": f.title, "project": l.project,
                "sessions": l.leaves().iter().filter(|l| matches!(l, Leaf::Session(_))).count(),
            }),
            Err(e) => json!({ "name": f.name, "source": f.source, "title": f.title, "error": e }),
        })
        .collect();
    Answer::Result(Value::Array(rows))
}

/// A project found for a layout.
struct Project {
    name: String,
    path: String,
    /// Whether Vorn knows it, not just its path.
    registered: bool,
    /// The workspace its sessions show in.
    workspace: String,
}

/// A project's name and path from `wanted` (a name, or a path), against the configured projects.
async fn project(native: &Arc<Native>, wanted: &str, viewer: &Viewer) -> Result<Project, String> {
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
    let workspace = |p: Option<&&Value>| {
        p.map(|p| field(p, "workspaceId"))
            .filter(|w| !w.is_empty())
            .unwrap_or_else(|| "personal".to_owned())
    };
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
        return Ok(Project {
            name,
            path: wanted.to_owned(),
            registered: registered.is_some(),
            workspace: workspace(registered),
        });
    }
    known
        .iter()
        .find(|p| field(p, "name") == wanted)
        .map(|p| Project {
            name: field(p, "name"),
            path: field(p, "path"),
            registered: true,
            workspace: workspace(Some(p)),
        })
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
    let (n, ask) = (Arc::clone(native), params.clone());
    let read = blocking(move || pick(&n, &ask).and_then(|f| read(&n, &f).map(|(_, l)| (f, l))))
        .await
        .and_then(|r| r);
    let (found, layout) = match read {
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
    let Project {
        name: project_name,
        path: project_path,
        workspace,
        ..
    } = match project(native, wanted, viewer).await {
        Ok(p) => p,
        Err(e) => return Answer::Error(e),
    };
    // Each leaf's session, none for a pane or one that did not start.
    let mut started: Vec<Option<String>> = Vec::new();
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
                        started.push(Some(id.clone()));
                        last = Some(id);
                    }
                    Answer::Error(e) => {
                        started.push(None);
                        errors.push(format!("{}: {e}", describe(s)));
                    }
                    _ => {
                        started.push(None);
                        errors.push(format!("{}: not started", describe(s)));
                    }
                }
            }
            Leaf::Pane(p) => {
                started.push(None);
                panes.push(open_pane(native, p, last.as_deref(), viewer, &mut errors).await)
            }
        }
    }
    let view = layout.view();
    if let Some(view) = view {
        let tree = tree_json(&layout.root, &mut started.iter());
        let shown = Shown {
            workspace,
            view,
            columns: layout.columns,
            tree,
        };
        set_view(native, viewer, &shown).await;
    }
    Answer::Result(json!({
        "layout": found.name, "source": found.source,
        "project": { "name": project_name, "path": project_path },
        "view": view.map(View::word),
        "sessions": sessions, "panes": panes, "errors": errors,
    }))
}

/// The key the grid a layout sets is kept under: the viewer's own, the
/// desktop app's for the CLI and agents' tools.
fn grid_owner(viewer: &Viewer) -> String {
    viewer.key().unwrap_or_else(|| "desktop".to_owned())
}

/// How a workspace's grid shows its sessions.
struct Shown {
    workspace: String,
    view: View,
    columns: Option<u32>,
    /// The app's tree (`{"id"}` leaves, `{"split", "ratio", "children"}`).
    tree: Option<Value>,
}

/// The layout's tree with each leaf its started session: a pane or a session
/// that did not start gives its room to the rest.
fn tree_json<'a>(
    node: &Node,
    started: &mut impl Iterator<Item = &'a Option<String>>,
) -> Option<Value> {
    match node {
        Node::Leaf(_) => started.next()?.as_ref().map(|id| json!({ "id": id })),
        Node::Split {
            direction,
            ratio,
            children,
        } => {
            let kids: Vec<Option<Value>> = children.iter().map(|c| tree_json(c, started)).collect();
            let first = kids.first().is_some_and(Option::is_some);
            let mut kept: Vec<Value> = kids.into_iter().flatten().collect();
            if kept.len() < 2 {
                return kept.pop();
            }
            let mut split = json!({ "split": direction.word(), "children": kept });
            // Without its first child the ratio means nothing; the rest share equally.
            if first {
                split["ratio"] = json!(ratio);
            }
            Some(split)
        }
    }
}

/// Writes the grid `shown` sets into the viewer's settings and tells the apps.
async fn set_view(native: &Arc<Native>, viewer: &Viewer, shown: &Shown) {
    let (n, key) = (Arc::clone(native), grid_owner(viewer));
    let (ws, view, columns, tree) = (
        shown.workspace.clone(),
        shown.view,
        shown.columns,
        shown.tree.clone(),
    );
    let wrote = super::config::blocking("layout:open", move || {
        with_store(&n, |store| {
            let own = store.viewer_settings(&key)?;
            let mut grids = own
                .get("gridLayout")
                .and_then(Value::as_object)
                .cloned()
                .unwrap_or_default();
            let mut grid = grids
                .get(&ws)
                .and_then(Value::as_object)
                .cloned()
                .unwrap_or_default();
            match view {
                View::Flexible => {
                    grid.insert("view".into(), json!("flexible"));
                    match tree {
                        Some(tree) => grid.insert("tree".into(), tree),
                        None => grid.remove("tree"),
                    };
                }
                View::Grid => {
                    grid.insert("view".into(), json!("grid"));
                    grid.insert("columns".into(), json!(columns.unwrap_or(0)));
                }
                View::Tabs => {}
            }
            grids.insert(ws, Value::Object(grid));
            let mut put = Map::new();
            put.insert("gridLayout".into(), Value::Object(grids));
            let mode = if view == View::Tabs { "tabs" } else { "grid" };
            put.insert("layoutMode".into(), json!(mode));
            store.save_viewer_settings(&key, &put)
        })
    })
    .await;
    if let Err(err) = wrote {
        warn!(%err, "could not keep the layout's view");
    }
    native.broadcast(
        "layout:opened",
        json!({
            "workspace": shown.workspace,
            "view": shown.view.word(),
            "columns": shown.columns.unwrap_or(0),
            "tree": shown.tree,
        }),
    );
}

/// The view the viewer's grid shows `workspace` in, and its tree.
async fn get_view(native: &Arc<Native>, viewer: &Viewer, workspace: &str) -> Option<Shown> {
    let (n, key) = (Arc::clone(native), grid_owner(viewer));
    let own = super::config::blocking("layout:save", move || {
        with_store(&n, |store| store.viewer_settings(&key))
    })
    .await
    .ok()?;
    let grid = own.get("gridLayout").and_then(|g| g.get(workspace));
    let field = |k: &str| grid.and_then(|g| g.get(k));
    let tabs = own.get("layoutMode").and_then(Value::as_str) == Some("tabs");
    let flexible = field("view").and_then(Value::as_str) == Some("flexible");
    let columns = field("columns")
        .and_then(Value::as_u64)
        .and_then(|c| u32::try_from(c).ok())
        .filter(|c| (1..=vorn_layout::MAX_COLUMNS).contains(c));
    Some(Shown {
        workspace: workspace.to_owned(),
        view: if tabs {
            View::Tabs
        } else if flexible {
            View::Flexible
        } else {
            View::Grid
        },
        columns,
        tree: field("tree").cloned(),
    })
}

/// The app's tree as a layout's, each `{"id"}` the session `leaves` has for
/// it; one it lacks (another project's, a popped-out card) is left out. A
/// run of equal splits one way reads back as one split of several children.
fn layout_tree(v: &Value, leaves: &mut HashMap<String, Node>) -> Option<Node> {
    if let Some(id) = v.get("id").and_then(Value::as_str) {
        return leaves.remove(id);
    }
    let direction = match v.get("split")?.as_str()? {
        "horizontal" => Direction::Horizontal,
        "vertical" => Direction::Vertical,
        _ => return None,
    };
    let kids: Vec<Option<Node>> = v
        .get("children")?
        .as_array()?
        .iter()
        .map(|c| layout_tree(c, leaves))
        .collect();
    let first = kids.first().is_some_and(Option::is_some);
    let mut kept: Vec<Node> = kids.into_iter().flatten().collect();
    if kept.len() < 2 {
        return kept.pop();
    }
    let equal = 1.0 / kept.len() as f64;
    let ratio = v
        .get("ratio")
        .and_then(Value::as_f64)
        .filter(|r| first && *r > 0.0 && *r < 1.0)
        .unwrap_or(equal);
    if kept.len() == 2 {
        if let Node::Split {
            direction: d,
            ratio: r,
            children,
        } = &kept[1]
        {
            if *d == direction && (r - 1.0 / children.len() as f64).abs() < 1e-3 {
                let Some(Node::Split { children, .. }) = kept.pop() else {
                    unreachable!("matched above")
                };
                kept.extend(children);
            }
        }
    }
    Some(Node::Split {
        direction,
        ratio: (ratio * 10_000.0).round() / 10_000.0,
        children: kept,
    })
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

/// `layout:save {name, project, overwrite?}`: the project's open sessions in
/// the view the grid shows them in, as its tree when that is flexible.
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
    let Project {
        name: project_name,
        path: project_path,
        registered,
        workspace,
    } = match project(native, wanted, viewer).await {
        Ok(p) => p,
        Err(e) => return Answer::Error(e),
    };
    let Some(dir) = dir(native) else {
        return Answer::Error("vornd has no data directory yet".to_owned());
    };
    let file = dir.join(format!("{name}.toml"));
    let taken = {
        let file = file.clone();
        blocking(move || file.exists()).await.unwrap_or(false)
    };
    if taken && params.get("overwrite") != Some(&Value::Bool(true)) {
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
                        t.id.clone(),
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
    let leaves: Vec<(String, Node)> = open
        .into_iter()
        .map(|(id, agent, label, cwd)| {
            let launch = if vorn_agents::is_known(&agent) {
                Launch::Agent(agent)
            } else {
                Launch::Shell
            };
            // Under the project, a path relative to it keeps the layout movable.
            let cwd = cwd.filter(|c| c != &project_path).map(|c| {
                Path::new(&c)
                    .strip_prefix(&project_path)
                    .map_or(c.clone(), |r| r.to_string_lossy().into_owned())
            });
            (
                id,
                Node::Leaf(Leaf::Session(Session { launch, cwd, label })),
            )
        })
        .collect();
    let count = leaves.len();
    let shown = get_view(native, viewer, &workspace).await;
    let order: Vec<String> = leaves.iter().map(|(id, _)| id.clone()).collect();
    let mut by_id: HashMap<String, Node> = leaves.into_iter().collect();
    let tree = shown
        .as_ref()
        .filter(|s| s.view == View::Flexible)
        .and_then(|s| s.tree.as_ref())
        .and_then(|t| layout_tree(t, &mut by_id));
    // Sessions the tree does not hold (it was laid out elsewhere) follow it.
    let mut rest: Vec<Node> = order.iter().filter_map(|id| by_id.remove(id)).collect();
    let root = match tree {
        Some(tree) if rest.is_empty() => tree,
        Some(tree) => {
            rest.insert(0, tree);
            flat(rest)
        }
        None => flat(rest),
    };
    let view = shown.as_ref().map(|s| s.view);
    let layout = Layout {
        name: Some(name.to_owned()),
        // A project Vorn does not know is kept by its path, which names it anywhere.
        project: Some(if registered {
            project_name
        } else {
            project_path.clone()
        }),
        view,
        columns: shown
            .as_ref()
            .filter(|s| s.view == View::Grid)
            .and_then(|s| s.columns),
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
            "view": view.map(View::word),
        })),
        Ok(Err(e)) => Answer::Error(format!("could not write {}: {e}", file.display())),
        Err(e) => Answer::Error(e.to_string()),
    }
}

/// `nodes` side by side in equal shares; one node is itself.
fn flat(mut nodes: Vec<Node>) -> Node {
    if nodes.len() == 1 {
        return nodes.remove(0);
    }
    Node::Split {
        direction: Direction::Horizontal,
        ratio: 1.0 / nodes.len() as f64,
        children: nodes,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn shell(label: &str) -> Node {
        Node::Leaf(Leaf::Session(Session {
            launch: Launch::Shell,
            cwd: None,
            label: Some(label.to_owned()),
        }))
    }

    #[test]
    fn an_opened_tree_drops_what_did_not_start() {
        let root = Node::Split {
            direction: Direction::Horizontal,
            ratio: 0.7,
            children: vec![
                shell("a"),
                Node::Split {
                    direction: Direction::Vertical,
                    ratio: 0.5,
                    children: vec![shell("b"), shell("c")],
                },
            ],
        };
        let started = [Some("1".to_owned()), Some("2".to_owned()), None];
        let tree = tree_json(&root, &mut started.iter()).unwrap();
        assert_eq!(
            tree,
            json!({"split": "horizontal", "ratio": 0.7, "children": [{"id": "1"}, {"id": "2"}]})
        );
        let started = [None, Some("2".to_owned()), Some("3".to_owned())];
        let tree = tree_json(&root, &mut started.iter()).unwrap();
        assert_eq!(tree["split"], "vertical");
    }

    #[test]
    fn a_saved_tree_is_the_grids_with_equal_runs_joined() {
        // The app's binary tree: a | (b / c) at 0.7, then d and e equal below.
        let app = json!({"split": "vertical", "ratio": 0.6, "children": [
            {"split": "horizontal", "ratio": 0.7, "children": [
                {"id": "a"},
                {"split": "vertical", "ratio": 0.5, "children": [{"id": "b"}, {"id": "c"}]},
            ]},
            {"split": "horizontal", "ratio": 0.3333, "children": [
                {"id": "d"},
                {"split": "horizontal", "ratio": 0.5, "children": [{"id": "e"}, {"id": "f"}]},
            ]},
        ]});
        let mut leaves: HashMap<String, Node> = ["a", "b", "c", "d", "e"]
            .iter()
            .map(|id| ((*id).to_owned(), shell(id)))
            .collect();
        let node = layout_tree(&app, &mut leaves).unwrap();
        let Node::Split {
            ratio, children, ..
        } = &node
        else {
            panic!("{node:?}")
        };
        assert!((ratio - 0.6).abs() < 1e-9);
        assert!(matches!(&children[0], Node::Split { ratio, children, .. }
            if (*ratio - 0.7).abs() < 1e-9 && children.len() == 2));
        // "f" is not this project's: d and e split the row equally.
        assert!(matches!(&children[1], Node::Split { ratio, children, .. }
            if (*ratio - 0.3333).abs() < 1e-9 && children.len() == 2));
        assert!(leaves.is_empty());

        // A run of equal splits one way is one split of three.
        let run = json!({"split": "horizontal", "ratio": 0.3333, "children": [
            {"id": "a"},
            {"split": "horizontal", "ratio": 0.5, "children": [{"id": "b"}, {"id": "c"}]},
        ]});
        let mut leaves: HashMap<String, Node> = ["a", "b", "c"]
            .iter()
            .map(|id| ((*id).to_owned(), shell(id)))
            .collect();
        let node = layout_tree(&run, &mut leaves).unwrap();
        assert!(matches!(&node, Node::Split { children, .. } if children.len() == 3));
    }
}

//! Sessions: the list by project, its scope, starting one, and one session's page.

use std::collections::BTreeMap;

use serde_json::json;
use vorn_app::client::store::Session;
use vorn_components::status::Status;
use vornui::{El, Id, Ui};

use super::options::{self, SESSION_FIELDS, SESSION_GROUP, SESSION_ORDER, SESSION_STATUS};
use super::{
    badge, bar, choice, col, empty, link, listing, present, row, say, section, status, tone, Cx,
    Do, Scope,
};
use crate::model::{For, Link};
use crate::phone::Phone;
use crate::screens::{row_id, terminal};

const PROMPT: &str = "launch-prompt";

pub fn rename_id(sid: &str) -> Id {
    Id::new("rename").child(sid)
}

fn in_scope(s: &Session, scope: Option<&Scope>) -> bool {
    match scope {
        None => true,
        Some(Scope::Project(p)) => s.info.project_name == *p,
        Some(Scope::Branch(b)) => s.info.branch.as_deref() == Some(b.as_str()),
    }
}

/// Where a status's group goes: what waits on a person first, then what runs, then what broke.
fn attention(status: &str) -> u8 {
    match status {
        "waiting" => 0,
        "running" => 1,
        "error" => 2,
        _ => 3,
    }
}

fn status_label(status: &str) -> &'static str {
    SESSION_STATUS
        .options
        .iter()
        .find(|(k, _, _)| *k == status)
        .map_or("Idle", |(_, label, _)| label)
}

const MAIN_CHECKOUT: &str = "Main checkout";

/// What a session's worktree is called.
fn worktree_label(s: &Session) -> &str {
    s.info
        .worktree_name
        .as_deref()
        .or(s.info.branch.as_deref())
        .unwrap_or(MAIN_CHECKOUT)
}

/// When a session last printed something here, else when it started; epoch ms.
fn activity(cx: &Cx, s: &Session) -> f64 {
    cx.p.model
        .last_output
        .get(&s.info.id)
        .copied()
        .unwrap_or(s.info.created_at)
}

/// The sessions the options keep, sorted, under their groups' (key, label); one unlabelled group when ungrouped.
fn grouped<'a>(cx: &Cx, all: &'a [Session]) -> Vec<(String, String, Vec<&'a Session>)> {
    let v = cx.view();
    let scope = v.sessions_scope.as_ref();
    let want = v.pick(&SESSION_STATUS);
    let mut kept: Vec<&Session> = all
        .iter()
        .filter(|s| in_scope(s, scope) && (want == "all" || s.status() == want))
        .collect();
    if v.pick(&SESSION_ORDER) == "created" {
        kept.sort_by(|a, b| b.info.created_at.total_cmp(&a.info.created_at));
    } else {
        kept.sort_by(|a, b| activity(cx, b).total_cmp(&activity(cx, a)));
    }
    let by = v.pick(&SESSION_GROUP);
    if by == "none" {
        return if kept.is_empty() {
            vec![]
        } else {
            vec![(String::new(), String::new(), kept)]
        };
    }
    let mut groups: Vec<(String, String, Vec<&Session>)> = Vec::new();
    for s in kept {
        let (key, label) = match by {
            "status" => (s.status().to_owned(), status_label(s.status()).to_owned()),
            "project" => (s.info.project_name.clone(), s.info.project_name.clone()),
            _ => (
                format!(
                    "{}\u{0}{}",
                    s.info.project_name,
                    s.info.worktree_path.as_deref().unwrap_or("")
                ),
                worktree_label(s).to_owned(),
            ),
        };
        match groups.iter_mut().find(|g| g.0 == key) {
            Some(g) => g.2.push(s),
            None => groups.push((key, label, vec![s])),
        }
    }
    if by == "status" {
        groups.sort_by_key(|g| attention(&g.0));
    }
    groups
}

pub fn list(cx: &Cx) -> El {
    let m = &cx.p.model;
    let scope = cx.view().sessions_scope.as_ref();
    let mut items = vec![
        super::filter(
            Id::new("sessions-scope-open"),
            "Show sessions from",
            "sessions-scope".into(),
            scope.is_some(),
        ),
        options::button(
            "sessions-view-open",
            "sessions-view",
            cx.view().sessions_changed(),
        ),
    ];
    if cx.online() {
        items.push(link(
            Id::new("launch-open"),
            "Start a session",
            "launch".into(),
            vec![super::icon("plus"), say("Start a session", "body")],
        ));
    }
    let head = bar("Sessions", items);
    let all = &m.store.sessions;
    let body = if all.is_empty() {
        match &m.link {
            Link::Online if m.store.sessions_known => empty(
                "No agents running",
                "Start one and it appears here, with what it is doing and whether it is waiting on you. You can answer it from this phone.",
                vec![link(
                    Id::new("launch-empty"),
                    "Start a session",
                    "launch".into(),
                    vec![say("Start a session", "body")],
                )],
            ),
            Link::Online | Link::Connecting => empty("Loading sessions.", "", vec![]),
            Link::Offline(why) => empty("Nothing to show until the host answers.", why, vec![]),
            Link::Unpaired => empty("Not paired", "Pair the phone with a computer first.", vec![]),
        }
    } else {
        let groups = grouped(cx, all);
        let mut rows = Vec::new();
        if let Some(sc) = scope {
            rows.push(row(vec![
                say(format!("Showing {}", sc.name()), "secondary"),
                cx.button(
                    Id::new("sessions-scope-clear"),
                    "Show all",
                    "plain",
                    Do::SessionsScope(None),
                ),
            ]));
        }
        if groups.is_empty() {
            rows.push(say(
                format!("Nothing here. {} elsewhere.", all.len()),
                "secondary",
            ));
        }
        for (key, label, ss) in groups {
            let key = format!("sessions:{key}");
            let limit = cx.view().limit(&key);
            let mut kids: Vec<El> = ss.iter().take(limit).map(|s| session_row(cx, s)).collect();
            kids.extend(cx.more(&key, ss.len().min(limit), ss.len()));
            let by = cx.view().pick(&SESSION_GROUP);
            let title = if label.is_empty() && by == "project" {
                "Other".to_owned()
            } else {
                label
            };
            rows.push(match (by, ss.first()) {
                ("project" | "worktree", Some(s)) => super::marked_section(
                    &title,
                    super::project_mark(cx, &s.info.project_name),
                    kids,
                ),
                _ => section(&title, kids),
            });
        }
        listing(rows)
    };
    col(vec![head, body])
}

/// The facts under a session's name, as the options ask.
fn meta(cx: &Cx, s: &Session) -> String {
    let v = cx.view();
    let has = |f: &str| v.shows(&SESSION_FIELDS, f);
    let mut parts = Vec::new();
    if has("project")
        && s.info
            .display_name
            .as_deref()
            .is_some_and(|n| !n.trim().is_empty())
    {
        parts.push(s.info.project_name.clone());
    }
    if has("time") {
        let at = vorn_app::tasks::iso(s.info.created_at);
        parts.push(vorn_app::runs::relative(&at, cx.now));
    }
    let branch = s.info.branch.as_deref().filter(|b| !b.is_empty());
    if has("branch") {
        parts.extend(branch.map(str::to_owned));
    }
    if has("worktree") {
        let w = worktree_label(s);
        if Some(w) != branch {
            parts.push(w.to_owned());
        }
    }
    if has("agent") {
        parts.push(cx.p.model.store.agents.label(&s.info.agent_type).to_owned());
    }
    parts.join(" · ")
}

fn asks(cx: &Cx, sid: &str) -> bool {
    cx.p.model
        .permissions
        .iter()
        .any(|q| q["terminalId"] == sid)
}

fn session_row(cx: &Cx, s: &Session) -> El {
    let sid = s.info.id.as_str();
    let st = Status::parse(s.status());
    let title = s.title();
    let mut kids = vec![super::agent(&cx.p.model.store, &s.info.agent_type)];
    if cx.view().shows(&SESSION_FIELDS, "status") {
        kids.push(status(s.status(), tone(st)));
    }
    let meta = meta(cx, s);
    let mut lines = vec![say(title.clone(), "title")];
    if !meta.is_empty() {
        lines.push(say(meta, "secondary"));
    }
    kids.push(col(lines));
    kids.extend(asks(cx, sid).then(|| badge("Asks", "warn")));
    link(row_id(sid), &title, format!("session:{sid}"), kids)
}

/// Every project and branch sessions run in, with how many each has.
fn scopes(cx: &Cx) -> Vec<(Scope, usize)> {
    let mut projects: BTreeMap<String, usize> = BTreeMap::new();
    let mut branches: BTreeMap<String, usize> = BTreeMap::new();
    for s in &cx.p.model.store.sessions {
        *projects.entry(s.info.project_name.clone()).or_default() += 1;
        if let Some(b) = s.info.branch.as_ref().filter(|b| !b.is_empty()) {
            *branches.entry(b.clone()).or_default() += 1;
        }
    }
    for p in &cx.p.model.store.projects {
        projects.entry(p.name.clone()).or_default();
    }
    projects
        .into_iter()
        .filter(|(n, _)| !n.is_empty())
        .map(|(n, c)| (Scope::Project(n), c))
        .chain(branches.into_iter().map(|(n, c)| (Scope::Branch(n), c)))
        .collect()
}

pub fn scope(cx: &Cx) -> El {
    let search = Id::new("sessions-scope-find");
    let find = cx.ui.edit_text(search).trim().to_lowercase();
    let current = cx.view().sessions_scope.as_ref();
    let all = cx.p.model.store.sessions.len();
    let mut projects = Vec::new();
    let mut branches = Vec::new();
    for (sc, n) in scopes(cx) {
        if !find.is_empty() && !sc.name().to_lowercase().contains(&find) {
            continue;
        }
        let (into, key) = match &sc {
            Scope::Project(p) => (&mut projects, ("p", p.clone())),
            Scope::Branch(b) => (&mut branches, ("b", b.clone())),
        };
        let el = cx.option(
            Id::new("sessions-scope").child(&key),
            &format!("{} · {n}", sc.name()),
            current == Some(&sc),
            Do::SessionsScope(Some(sc.clone())),
        );
        into.push(match &sc {
            Scope::Project(p) => el.child(super::project_mark(cx, p)),
            Scope::Branch(_) => el.child(super::icon("git-branch")),
        });
    }
    let mut rows = vec![
        cx.field(search, "Find a project or branch", false),
        cx.option(
            Id::new("sessions-scope-all"),
            &format!("All sessions · {all}"),
            current.is_none(),
            Do::SessionsScope(None),
        ),
    ];
    if !projects.is_empty() {
        rows.push(section("Projects", projects));
    }
    if !branches.is_empty() {
        rows.push(section("Branches", branches));
    }
    col(vec![bar("Show sessions from", vec![]), listing(rows)])
}

const SHELL: &str = "shell";

fn launch_project(p: &Phone) -> Option<(String, String)> {
    let projects = &p.model.store.projects;
    let want = p.view.launch_project.as_deref();
    projects
        .iter()
        .find(|p| Some(p.name.as_str()) == want)
        .or_else(|| projects.first())
        .map(|p| (p.name.clone(), p.path.clone()))
}

fn launch_agent(p: &Phone) -> String {
    p.view
        .launch_agent
        .clone()
        .unwrap_or_else(|| p.model.store.default_agent().to_owned())
}

/// What the launcher says under its button; each line names something the app does.
const TIPS: &[&str] = &[
    "Create a worktree to give each agent an isolated working directory",
    "Leave the prompt empty to open the agent without a task",
    "A shell opens straight into the project directory, and takes no prompt",
    "Swipe a task on the board to archive it, or to move it along",
    "Open a run to see what each step was configured to do, and what it said",
    "A run can be stopped from its own screen while it is still going",
    "Approve a workflow waiting on you without getting to a desk",
];

pub fn launch(cx: &Cx) -> El {
    let project = launch_project(cx.p);
    let agent = launch_agent(cx.p);
    let shell = agent == SHELL;
    let projects =
        cx.p.model
            .store
            .projects
            .iter()
            .map(|p| {
                cx.option(
                    Id::new("launch-project").child(&p.name),
                    &p.name,
                    project.as_ref().is_some_and(|(n, _)| *n == p.name),
                    Do::LaunchProject(p.name.clone()),
                )
            })
            .collect();
    let agents =
        cx.p.model
            .store
            .agents
            .iter()
            .map(|a| (a.key.as_str(), a.name.as_str()))
            .chain([(SHELL, "Shell")])
            .map(|(k, n)| {
                cx.option(
                    Id::new("launch-agent").child(k),
                    n,
                    agent == k,
                    Do::LaunchAgent(k.to_owned()),
                )
                .child(super::agent(&cx.p.model.store, k))
            })
            .collect();
    let wt = cx.view().launch_worktree;
    let where_ = choice(
        "Where it works",
        vec![
            cx.option(
                Id::new("launch-where").child(0),
                "Main checkout",
                !wt,
                Do::LaunchWorktree(false),
            ),
            cx.option(
                Id::new("launch-where").child(1),
                "New worktree",
                wt,
                Do::LaunchWorktree(true),
            ),
        ],
    );
    let hint = if wt {
        "A separate checkout on its own branch"
    } else {
        "Shares the files you have open"
    };
    let prompt = Id::new(PROMPT);
    let typed = !cx.ui.edit_text(prompt).trim().is_empty();
    let busy = cx.p.model.busy.contains("launch");
    let label = match (busy, typed && !shell) {
        (true, _) => "Starting",
        (false, true) => "Start and send",
        (false, false) => "Start",
    };
    let mut rows = vec![section(
        "",
        vec![choice("Project", projects), choice("Agent", agents)],
    )];
    if !shell {
        rows.push(section("", vec![where_, say(hint, "secondary")]));
        rows.push(section(
            "Prompt",
            vec![cx.field(prompt, "What should it work on?", true)],
        ));
    } else {
        rows.push(say(
            "A shell opens straight into the project directory",
            "secondary",
        ));
    }
    if project.is_none() {
        rows.push(say("Add a project on the computer first.", "secondary"));
    }
    rows.extend(cx.error("launch"));
    if project.is_some() {
        rows.push(cx.button(Id::new("launch-start"), label, "primary", Do::Launch));
    }
    rows.push(say(TIPS[cx.view().tip % TIPS.len()], "secondary"));
    col(vec![bar("New session", vec![]), listing(rows)])
}

/// Starts what the launch page has picked; the session opens once vornd answers.
pub fn launch_now(p: &mut Phone, ui: &mut Ui) {
    if p.model.busy.contains("launch") {
        return;
    }
    let Some((name, path)) = launch_project(p) else {
        return;
    };
    let agent = launch_agent(p);
    let prompt = Id::new(PROMPT);
    let text = ui.edit_text(prompt).trim().to_owned();
    p.model.errors.remove("launch");
    p.model.busy.insert("launch".into());
    p.view.tip += 1;
    if agent == SHELL {
        p.model.call(For::Launch, "shell:create", json!(path));
        return;
    }
    let mut params = json!({ "agentType": agent, "projectName": name, "projectPath": path });
    if p.view.launch_worktree {
        params["useWorktree"] = json!(true);
    }
    if !text.is_empty() {
        params["initialPrompt"] = json!(text);
    }
    p.model.call(For::Launch, "terminal:create", params);
    ui.set_edit_text(prompt, "");
}

pub fn session(cx: &Cx, sid: &str) -> El {
    let s = cx.p.model.store.session(sid);
    let title = s.map_or_else(|| "Session".into(), Session::title);
    let st = s.map_or(Status::Idle, |s| Status::parse(s.status()));
    let menu = super::menu(vec![
        present(
            Id::new("rename-open").child(sid),
            "Rename",
            format!("rename:{sid}"),
            Some("pencil"),
        ),
        cx.icon_button(
            Id::new("font-smaller"),
            &format!("Smaller text, now {} point", cx.view().font),
            "minus",
            "",
            Do::Font(-1.0),
        ),
        cx.icon_button(
            Id::new("font-larger"),
            &format!("Larger text, now {} point", cx.view().font),
            "plus",
            "",
            Do::Font(1.0),
        ),
        cx.asking(
            Id::new("kill").child(sid),
            "End session",
            "End this session? The agent stops and its terminal closes.",
            Do::Kill(sid.to_owned()),
        ),
    ]);
    let mut out = vec![
        bar(title, vec![menu]),
        super::agent(
            &cx.p.model.store,
            s.map_or(SHELL, |s| s.info.agent_type.as_str()),
        ),
        status(s.map_or("idle", Session::status), tone(st)),
    ];
    for q in
        cx.p.model
            .permissions
            .iter()
            .filter(|q| q["terminalId"] == sid)
    {
        let request = q["requestId"].as_str().unwrap_or("").to_owned();
        let tool = q["toolName"].as_str().unwrap_or("a tool");
        let mut rows = vec![say(format!("Wants to use {tool}"), "title")];
        if let Some(d) = q["description"].as_str() {
            rows.push(say(d, "mono"));
        }
        rows.push(row(vec![
            cx.button(
                Id::new("allow").child(&request),
                "Allow",
                "primary",
                Do::Permission {
                    request: request.clone(),
                    allow: true,
                },
            ),
            cx.button(
                Id::new("deny").child(&request),
                "Deny",
                "danger",
                Do::Permission {
                    request: request.clone(),
                    allow: false,
                },
            ),
        ]));
        out.push(section("Permission", rows));
    }
    out.push(terminal(cx.p));
    col(out)
}

pub fn rename(cx: &Cx, sid: &str) -> El {
    col(vec![
        bar(
            "Rename",
            vec![cx.button(
                Id::new("rename-save").child(sid),
                "Save",
                "primary",
                Do::Rename(sid.to_owned()),
            )],
        ),
        listing(vec![cx.field(rename_id(sid), "Session name", false)]),
    ])
}

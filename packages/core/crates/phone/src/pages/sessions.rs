//! Sessions: the list by project, its scope, starting one, and one session's page.

use std::collections::BTreeMap;

use serde_json::json;
use vorn_app::client::store::Session;
use vorn_app::screen::menus::{agent_name, AGENTS};
use vorn_components::status::Status;
use vornui::native::Sem;
use vornui::{El, Id, Ui};

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

/// Anything waiting on a person first, then the newest.
fn order(a: &&Session, b: &&Session) -> std::cmp::Ordering {
    let waiting = |s: &Session| s.status() != "waiting";
    waiting(a)
        .cmp(&waiting(b))
        .then(b.info.created_at.total_cmp(&a.info.created_at))
}

pub fn list(cx: &Cx) -> El {
    let m = &cx.p.model;
    let scope = cx.view().sessions_scope.as_ref();
    let mut items = vec![present(
        Id::new("sessions-scope-open"),
        "Show sessions from",
        "sessions-scope".into(),
        Some(if scope.is_some() {
            "filter-on"
        } else {
            "filter"
        }),
    )];
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
        let shown: Vec<&Session> = all.iter().filter(|s| in_scope(s, scope)).collect();
        let mut groups: BTreeMap<&str, Vec<&Session>> = BTreeMap::new();
        for s in &shown {
            groups
                .entry(s.info.project_name.as_str())
                .or_default()
                .push(s);
        }
        let mut groups: Vec<(&str, Vec<&Session>)> = groups.into_iter().collect();
        let waits = |g: &[&Session]| !g.iter().any(|s| s.status() == "waiting");
        groups.sort_by_key(|g| waits(&g.1));
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
        if shown.is_empty() {
            rows.push(say(
                format!("Nothing here. {} elsewhere.", all.len()),
                "secondary",
            ));
        }
        for (project, mut ss) in groups {
            ss.sort_by(order);
            let title = if project.is_empty() { "Other" } else { project };
            rows.push(section(
                title,
                ss.into_iter().map(|s| session_row(cx, s)).collect(),
            ));
        }
        listing(rows)
    };
    col(vec![head, body])
}

fn meta(cx: &Cx, s: &Session) -> String {
    let mut parts = Vec::new();
    if let Some(b) = s.info.branch.as_deref().filter(|b| !b.is_empty()) {
        parts.push(b.to_owned());
    }
    parts.push(agent_name(&s.info.agent_type).to_owned());
    let at = vorn_app::tasks::iso(s.info.created_at);
    parts.push(vorn_app::runs::relative(&at, cx.now));
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
    link(
        row_id(sid),
        &title,
        format!("session:{sid}"),
        vec![
            status(s.status(), tone(st)),
            col(vec![
                say(title.clone(), "title"),
                say(meta(cx, s), "secondary"),
            ]),
        ]
        .into_iter()
        .chain(asks(cx, sid).then(|| badge("Asks", "warn")))
        .collect(),
    )
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
        into.push(cx.option(
            Id::new("sessions-scope").child(&key),
            &format!("{} · {n}", sc.name()),
            current == Some(&sc),
            Do::SessionsScope(Some(sc.clone())),
        ));
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
    let agents = AGENTS
        .iter()
        .map(|(k, n)| (*k, *n))
        .chain([(SHELL, "Shell")])
        .map(|(k, n)| {
            cx.option(
                Id::new("launch-agent").child(k),
                n,
                agent == k,
                Do::LaunchAgent(k.to_owned()),
            )
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
    let menu = vorn_more(vec![
        present(
            Id::new("rename-open").child(sid),
            "Rename",
            format!("rename:{sid}"),
            Some("pencil"),
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

fn vorn_more(items: Vec<El>) -> El {
    vornui::div().sem(Sem::More).children(items)
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

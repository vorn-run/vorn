//! Workflows: the list, runs and reviews, one workflow, one run, and answering a gate.

use serde_json::{json, Map, Value};
use vorn_app::gates;
use vorn_app::runs::{self, s, Line};
use vorn_app::workflows::{self as wf, Bucket};
use vornui::{El, Id, Ui};

use super::{
    badge, bar, choice, col, copy, empty, link, listing, present, row, run_tone, say, section,
    status, Cx, Do, Trigger, WfTab,
};
use crate::model::{Fetched, For, Link};
use crate::phone::Phone;

const RUNS: usize = 50;

fn def(p: &Phone, id: &str) -> Option<Value> {
    p.model
        .store
        .workflows
        .iter()
        .find(|w| w.id == id)
        .and_then(|w| serde_json::to_value(w).ok())
}

fn nodes(w: Option<&Value>) -> Value {
    w.and_then(|w| w.get("nodes")).cloned().unwrap_or(json!([]))
}

fn node_list(w: &Value) -> Vec<Value> {
    w["nodes"].as_array().cloned().unwrap_or_default()
}

fn trigger_kind(w: &Value) -> String {
    wf::trigger(&node_list(w))
        .and_then(|t| t.pointer("/config/triggerType"))
        .and_then(Value::as_str)
        .unwrap_or("manual")
        .to_owned()
}

fn started(r: &Value) -> &str {
    s(r, "startedAt").unwrap_or("")
}

/// The store's runs, newest first, that `keep` keeps.
fn runs_where(p: &Phone, keep: impl Fn(&Value) -> bool) -> Vec<Value> {
    let mut out: Vec<Value> = p
        .model
        .store
        .runs
        .values()
        .filter(|r| keep(r))
        .cloned()
        .collect();
    out.sort_by(|a, b| started(b).cmp(started(a)));
    out
}

fn waiting(p: &Phone) -> Vec<Value> {
    runs_where(p, |r| wf::bucket(r) == Bucket::Waiting)
}

/// One row per run, opening it.
pub fn run_rows(cx: &Cx, list: &[Value]) -> Vec<El> {
    list.iter()
        .map(|r| {
            let id = s(r, "runId").unwrap_or("");
            let w = def(cx.p, s(r, "workflowId").unwrap_or(""));
            let title = wf::describe(r, w.as_ref(), None).title;
            let st = runs::run_status(r);
            let line = wf::status_line(r, &nodes(w.as_ref()));
            link(
                Id::new("run-row").child(id),
                &title,
                format!("run:{id}"),
                vec![
                    status(runs::label(st), run_tone(st)),
                    col(vec![
                        say(title.clone(), "title"),
                        say(
                            format!("{line} · {}", runs::relative(started(r), cx.now)),
                            "secondary",
                        ),
                    ]),
                ],
            )
        })
        .collect()
}

fn offline(cx: &Cx, loading: &str) -> Option<El> {
    match &cx.p.model.link {
        Link::Online => None,
        Link::Connecting => Some(empty(loading, "", vec![])),
        Link::Offline(why) => Some(empty(
            "Nothing to show until the host answers.",
            why,
            vec![],
        )),
        Link::Unpaired => Some(empty(
            "Not paired",
            "Pair the phone with a computer first.",
            vec![],
        )),
    }
}

pub fn list(cx: &Cx) -> El {
    let tab = cx.view().wf_tab;
    let review = waiting(cx.p).len();
    let tabs = choice(
        "",
        [
            (WfTab::Workflows, "Workflows".to_owned()),
            (WfTab::Runs, "Runs".to_owned()),
            (WfTab::Review, format!("Review {review}")),
        ]
        .into_iter()
        .map(|(t, label)| {
            cx.option(
                Id::new("wf-tab").child(t as u8),
                &label,
                t == tab,
                Do::WfTab(t),
            )
        })
        .collect(),
    );
    let mut items = vec![];
    if tab == WfTab::Workflows {
        let on = cx.view().trigger != Trigger::All;
        items.push(super::filter(
            Id::new("wf-trigger-open"),
            "Show workflows",
            "workflows-trigger".into(),
            on,
        ));
    }
    let body = match tab {
        WfTab::Workflows => workflows(cx),
        WfTab::Runs => all_runs(cx),
        WfTab::Review => reviews(cx),
    };
    col(vec![bar("Workflows", items), tabs, body])
}

fn workflows(cx: &Cx) -> El {
    let all = &cx.p.model.store.workflows;
    if all.is_empty() {
        return offline(cx, "Loading workflows.").unwrap_or_else(|| {
            empty(
                "No workflows yet",
                "Workflows are built at the desk. Once one exists it appears here, and you can see what it does and when it last ran.",
                vec![],
            )
        });
    }
    let trigger = cx.view().trigger;
    let mut rows = Vec::new();
    for w in all {
        let Ok(v) = serde_json::to_value(w) else {
            continue;
        };
        let keep = match trigger {
            Trigger::All => true,
            Trigger::Manual => !wf::is_scheduled(&v),
            Trigger::Scheduled => wf::is_scheduled(&v),
        };
        if !keep {
            continue;
        }
        let (kind, _) = wf::trigger_info(&trigger_kind(&v));
        let mut meta = vec![kind.to_owned()];
        if !w.enabled {
            meta.push("Off".into());
        }
        if let Some(at) = &w.last_run_at {
            meta.push(format!("Ran {}", runs::relative(at, cx.now).to_lowercase()));
        }
        let name = if w.name.trim().is_empty() {
            "Untitled"
        } else {
            w.name.as_str()
        };
        let mut kids = vec![col(vec![
            say(name, "title"),
            say(meta.join(" · "), "secondary"),
        ])];
        if let Some(st) = w.last_run_status.as_deref() {
            kids.push(badge(runs::label(st), run_tone(st)));
        }
        rows.push(link(
            Id::new("wf-row").child(&w.id),
            name,
            format!("workflow:{}", w.id),
            kids,
        ));
    }
    if rows.is_empty() {
        rows.push(say("No workflows with that trigger.", "secondary"));
    }
    listing(rows)
}

fn all_runs(cx: &Cx) -> El {
    let fetched = cx.fetch("workflowRun:listAll", json!({ "limit": RUNS }));
    let shown = runs_where(cx.p, |_| true);
    if shown.is_empty() {
        if let Some(e) = offline(cx, "Loading runs.") {
            return e;
        }
        return match fetched {
            Fetched::Loading => empty("Loading runs.", "", vec![]),
            Fetched::Failed(e) => empty("Could not load runs", &e, vec![]),
            Fetched::Ready(_) => empty(
                "No runs yet",
                "Workflows run on a computer with the app open. Their runs and every step they took show up here.",
                vec![],
            ),
        };
    }
    let limit = cx.view().limit("runs");
    let mut rows = run_rows(cx, &shown[..shown.len().min(limit)]);
    rows.extend(cx.more("runs", shown.len().min(limit), shown.len()));
    listing(rows)
}

fn reviews(cx: &Cx) -> El {
    let list_ = waiting(cx.p);
    if list_.is_empty() {
        return empty(
            "Nothing is waiting for you",
            "When a workflow stops at an approval gate it appears here, and you can approve it without going back to the desk.",
            vec![],
        );
    }
    listing(run_rows(cx, &list_))
}

pub fn trigger(cx: &Cx) -> El {
    let now = cx.view().trigger;
    let rows = [
        (Trigger::All, "All workflows"),
        (Trigger::Manual, "Manual only"),
        (Trigger::Scheduled, "Scheduled only"),
    ]
    .into_iter()
    .map(|(t, label)| {
        cx.option(
            Id::new("wf-trigger").child(t as u8),
            label,
            t == now,
            Do::Trigger(t),
        )
    })
    .collect();
    col(vec![bar("Show workflows", vec![]), listing(rows)])
}

pub fn workflow(cx: &Cx, id: &str) -> El {
    let Some(w) = def(cx.p, id) else {
        return col(vec![
            bar("Workflow", vec![]),
            empty("This workflow is gone.", "", vec![]),
        ]);
    };
    let name = s(&w, "name")
        .filter(|n| !n.trim().is_empty())
        .unwrap_or("Untitled")
        .to_owned();
    let all = node_list(&w);
    let busy = cx.p.model.busy.contains(&format!("run:{id}"));
    let start = if busy {
        say("Starting", "secondary")
    } else if wf::needs_prompt(&all, false) {
        present(
            Id::new("wf-run").child(id),
            "Run · asks first",
            format!("workflow-run:{id}"),
            Some("play"),
        )
    } else {
        cx.button(
            Id::new("wf-run").child(id),
            "Run",
            "primary",
            Do::RunWorkflow(id.to_owned()),
        )
    };
    let (kind, hint) = wf::trigger_info(&trigger_kind(&w));
    let mut about = vec![row(vec![say("Starts", "body"), say(kind, "secondary")])];
    if !hint.is_empty() {
        about.push(say(hint, "secondary"));
    }
    if w["enabled"] == json!(false) {
        about.push(say("Turned off on the computer.", "secondary"));
    }
    let steps: Vec<El> = all
        .iter()
        .filter(|n| s(n, "type") != Some("trigger"))
        .map(|n| {
            let label = runs::node_label(Some(n), s(n, "id").unwrap_or(""));
            let sub = wf::subtitle(n);
            if sub.is_empty() {
                say(label, "body")
            } else {
                col(vec![say(label, "body"), say(sub, "secondary")])
            }
        })
        .collect();
    let mut rows = vec![section("", vec![start]), section("About", about)];
    rows.extend(cx.error(&format!("run:{id}")));
    if !steps.is_empty() {
        rows.push(section("Steps", steps));
    }
    let recent = match cx.fetch("workflowRun:list", json!({ "workflowId": id, "limit": 20 })) {
        Fetched::Loading => vec![say("Loading runs.", "secondary")],
        Fetched::Failed(e) => vec![say(e, "error")],
        Fetched::Ready(_) => {
            let mine = runs_where(cx.p, |r| s(r, "workflowId") == Some(id));
            if mine.is_empty() {
                vec![say("It has not run yet.", "secondary")]
            } else {
                run_rows(cx, &mine[..mine.len().min(20)])
            }
        }
    };
    rows.push(section("Runs", recent));
    col(vec![bar(name, vec![]), listing(rows)])
}

pub fn input_id(wf: &str, key: &str) -> Id {
    Id::new("wf-input").child((wf, key))
}

fn asked(p: &Phone, id: &str) -> Vec<Value> {
    def(p, id)
        .map(|w| wf::asked(&node_list(&w)))
        .unwrap_or_default()
}

pub fn input_defaults(p: &Phone, id: &str) -> Vec<(String, String)> {
    asked(p, id)
        .iter()
        .filter(|d| s(d, "type") != Some("boolean"))
        .map(|d| (s(d, "key").unwrap_or("").to_owned(), wf::input_default(d)))
        .collect()
}

fn flag(p: &Phone, id: &str, d: &Value) -> bool {
    let key = s(d, "key").unwrap_or("");
    p.view
        .flags
        .get(&(id.to_owned(), key.to_owned()))
        .copied()
        .unwrap_or_else(|| wf::input_default(d) == "true")
}

fn value(p: &Phone, ui: &Ui, id: &str, d: &Value) -> String {
    if s(d, "type") == Some("boolean") {
        flag(p, id, d).to_string()
    } else {
        ui.edit_text(input_id(id, s(d, "key").unwrap_or("")))
            .to_owned()
    }
}

pub fn form(cx: &Cx, id: &str) -> El {
    let name = def(cx.p, id)
        .and_then(|w| s(&w, "name").map(str::to_owned))
        .unwrap_or_else(|| "Run".into());
    let defs = asked(cx.p, id);
    let mut rows: Vec<El> = Vec::new();
    for d in &defs {
        let key = s(d, "key").unwrap_or("");
        let label = s(d, "label").filter(|l| !l.is_empty()).unwrap_or(key);
        let required = d["required"] == json!(true);
        let title = if required {
            format!("{label} *")
        } else {
            label.to_owned()
        };
        let field = match s(d, "type") {
            Some("boolean") => cx.toggle(
                input_id(id, key),
                label,
                flag(cx.p, id, d),
                Do::Flip(id.to_owned(), key.to_owned()),
            ),
            Some("textarea") => cx.field(input_id(id, key), "Anything the agent should know", true),
            _ => cx.field(input_id(id, key), "Tap to write", false),
        };
        let mut kids = vec![field];
        if let Some(h) = s(d, "description").filter(|h| !h.is_empty()) {
            kids.push(say(h, "secondary"));
        }
        rows.push(section(&title, kids));
    }
    let busy = cx.p.model.busy.contains(&format!("run:{id}"));
    let ready = wf::inputs_valid(&defs, |k| {
        defs.iter()
            .find(|d| s(d, "key") == Some(k))
            .map(|d| value(cx.p, cx.ui, id, d))
            .unwrap_or_default()
    });
    rows.extend(cx.error(&format!("run:{id}")));
    if !ready {
        rows.push(say("Fill in the starred fields to run it.", "secondary"));
    }
    let label = if busy { "Starting" } else { "Run" };
    let mut items = vec![];
    if ready && !busy {
        items.push(cx.button(
            Id::new("wf-start").child(id),
            label,
            "primary",
            Do::StartRun(id.to_owned()),
        ));
    }
    col(vec![bar(name, items), listing(rows)])
}

/// Runs `id` by hand with `inputs`; `back` leaves the form once it starts.
pub fn start(p: &mut Phone, id: &str, inputs: Value, back: bool) {
    let key = format!("run:{id}");
    if p.model.busy.contains(&key) {
        return;
    }
    p.model.errors.remove(&key);
    p.model.busy.insert(key);
    p.model.call(
        For::Run(id.to_owned(), back),
        "workflow:runManual",
        json!({ "workflowId": id, "inputs": inputs }),
    );
}

pub fn start_form(p: &mut Phone, ui: &mut Ui, id: &str) {
    let mut inputs = Map::new();
    for d in asked(p, id) {
        let raw = value(p, ui, id, &d);
        inputs.insert(
            s(&d, "key").unwrap_or("").to_owned(),
            wf::input_json(&d, &raw),
        );
    }
    for (key, _) in input_defaults(p, id) {
        p.view.seeded.remove(&input_id(id, &key));
    }
    start(p, id, Value::Object(inputs), true);
}

fn the_run<'a>(p: &'a Phone, id: &str) -> Option<&'a Value> {
    p.model.store.runs.get(id)
}

fn state<'a>(run: &'a Value, node: &str) -> Option<&'a Value> {
    runs::states(run).find(|n| s(n, "nodeId") == Some(node))
}

fn gate_config(p: &Phone, run: &Value, node: &str) -> Option<Value> {
    let w = def(p, s(run, "workflowId").unwrap_or(""))?;
    runs::node(&w["nodes"], node)
        .and_then(|n| n.get("config"))
        .cloned()
}

fn gate_key(run: &str, node: &str) -> String {
    format!("{run}|{node}")
}

/// What a gate asks about, as the reviewer reads it.
pub fn gate_text(p: &Phone, run: &str, node: &str) -> String {
    let Some(r) = the_run(p, run) else {
        return String::new();
    };
    let st = state(r, node).cloned().unwrap_or(Value::Null);
    if let Some(e) = p.view.edited.get(&(run.to_owned(), node.to_owned())) {
        return e.clone();
    }
    if let Some(t) = gates::editable(&st) {
        return t.to_owned();
    }
    s(&st, "message")
        .map(str::to_owned)
        .or_else(|| gate_config(p, r, node).and_then(|c| s(&c, "message").map(str::to_owned)))
        .unwrap_or_default()
}

pub fn edit_id(run: &str, node: &str) -> Id {
    Id::new("gate-edit").child((run, node))
}

fn comment_id(run: &str, node: &str) -> Id {
    Id::new("gate-comment").child((run, node))
}

/// The answers a waiting gate takes, in a section of its own.
fn gate(cx: &Cx, run_id: &str, r: &Value, st: &Value) -> El {
    let node = s(st, "nodeId").unwrap_or("");
    let w = def(cx.p, s(r, "workflowId").unwrap_or(""));
    let config = gate_config(cx.p, r, node);
    let label = runs::node_label(runs::node(&nodes(w.as_ref()), node), node);
    let key = gate_key(run_id, node);
    let busy = cx.p.model.busy.contains(&format!("gate:{key}"));
    let text = gate_text(cx.p, run_id, node);
    let mut rows = vec![if text.is_empty() {
        say("Waiting for approval.", "secondary")
    } else {
        say(text, "body")
    }];
    if let Some(a) = gates::last_asked(st) {
        rows.push(say(format!("You asked: {a}"), "secondary"));
    }
    if let Some(r) = gates::round_label(config.as_ref(), st) {
        rows.push(say(r, "secondary"));
    }
    if cx
        .p
        .view
        .edited
        .contains_key(&(run_id.to_owned(), node.to_owned()))
    {
        rows.push(say("Approving sends your edited text.", "secondary"));
    }
    rows.extend(cx.error(&format!("gate:{key}")));
    let answer = |decision: &'static str| Do::Gate {
        run: run_id.to_owned(),
        node: node.to_owned(),
        decision,
        back: false,
    };
    let gid = |what: &str| Id::new("gate").child((run_id, node, what));
    if busy {
        rows.push(say("Sending.", "secondary"));
    } else {
        rows.push(cx.button(gid("approve"), "Approve", "primary", answer("approve")));
        rows.push(present(
            gid("review"),
            "Open review",
            format!("review:{run_id}:{node}"),
            Some("file-text"),
        ));
        if gates::editable(st).is_some() {
            rows.push(present(
                gid("edit"),
                "Edit the text",
                format!("gate-edit:{run_id}:{node}"),
                Some("pencil"),
            ));
        }
        if gates::can_request_changes(config.as_ref(), st) {
            rows.push(present(
                gid("changes"),
                "Request changes",
                format!("gate-changes:{run_id}:{node}"),
                Some("message-square"),
            ));
        }
        rows.push(cx.asking(
            gid("reject"),
            "Reject",
            "Reject this run? It stops here.",
            answer("reject"),
        ));
    }
    section(&format!("Waiting at {label}"), rows)
}

pub fn run(cx: &Cx, id: &str) -> El {
    let Some(r) = the_run(cx.p, id) else {
        return col(vec![
            bar("Run", vec![]),
            empty(
                "This run is not loaded.",
                "Open it from a list again.",
                vec![],
            ),
        ]);
    };
    let w = def(cx.p, s(r, "workflowId").unwrap_or(""));
    let ns = nodes(w.as_ref());
    let d = wf::describe(r, w.as_ref(), None);
    let st = runs::run_status(r);
    let mut items = vec![];
    if runs::stoppable(r) {
        items.push(cx.asking(
            Id::new("run-stop").child(id),
            "Stop",
            "Stop this run? Steps still going are cancelled.",
            Do::StopRun(id.to_owned()),
        ));
    }
    let mut head = vec![
        row(vec![
            status(runs::label(st), run_tone(st)),
            say(wf::status_line(r, &ns), "secondary"),
        ]),
        say(
            format!(
                "Started {} · {}",
                runs::relative(started(r), cx.now).to_lowercase(),
                runs::duration(started(r), s(r, "completedAt"))
            ),
            "secondary",
        ),
    ];
    if let Some(sub) = &d.subtitle {
        head.push(say(sub.clone(), "secondary"));
    }
    let mut rows = vec![section("", head)];
    rows.extend(cx.error("quiet"));
    for g in runs::states(r).filter(|n| s(n, "status") == Some("waiting") && !wf::is_sign_in(n)) {
        rows.push(gate(cx, id, r, g));
    }
    let steps: Vec<El> = runs::steps(r, &ns, false)
        .into_iter()
        .map(|n| {
            let nid = s(n, "nodeId").unwrap_or("");
            let label = runs::node_label(runs::node(&ns, nid), nid);
            let nst = s(n, "status").unwrap_or("pending");
            let took = s(n, "startedAt")
                .map(|a| runs::duration(a, s(n, "completedAt")))
                .unwrap_or_default();
            link(
                Id::new("run-step").child((id, nid)),
                &label,
                format!("run-log:{id}:{nid}"),
                vec![
                    status(runs::label(nst), run_tone(nst)),
                    col(vec![say(label.clone(), "body"), say(took, "secondary")]),
                ],
            )
        })
        .collect();
    if !steps.is_empty() {
        rows.push(section("Steps", steps));
    }
    col(vec![bar(d.title, items), listing(rows)])
}

pub fn log(cx: &Cx, arg: &str) -> El {
    let (run_id, node) = arg.split_once(':').unwrap_or((arg, ""));
    let Some(r) = the_run(cx.p, run_id) else {
        return col(vec![
            bar("Log", vec![]),
            empty("This run is not loaded.", "", vec![]),
        ]);
    };
    let w = def(cx.p, s(r, "workflowId").unwrap_or(""));
    let ns = nodes(w.as_ref());
    let picked: Vec<&Value> = runs::steps(r, &ns, false)
        .into_iter()
        .filter(|n| node.is_empty() || s(n, "nodeId") == Some(node))
        .collect();
    let mut all = String::new();
    let mut rows = Vec::new();
    for n in picked {
        let nid = s(n, "nodeId").unwrap_or("");
        let label = runs::node_label(runs::node(&ns, nid), nid);
        let lines = runs::timeline(s(n, "logs"), s(n, "diagnostics"));
        let mut kids: Vec<El> = lines
            .iter()
            .map(|l| match l {
                Line::Agent(t) => say(t.clone(), "mono"),
                Line::Engine(t) => say(t.clone(), "secondary"),
            })
            .collect();
        if kids.is_empty() {
            kids.push(say(
                runs::empty_log(s(n, "status").unwrap_or("")),
                "secondary",
            ));
        }
        for l in &lines {
            let (Line::Agent(t) | Line::Engine(t)) = l;
            all.push_str(t);
            all.push('\n');
        }
        rows.push(section(&label, kids));
    }
    let title = if node.is_empty() {
        "Log".to_owned()
    } else {
        runs::node_label(runs::node(&ns, node), node)
    };
    let items = if all.is_empty() {
        vec![]
    } else {
        vec![copy(Id::new("log-copy").child(arg), "Copy", &all)]
    };
    col(vec![bar(title, items), listing(rows)])
}

fn split(arg: &str) -> (&str, &str) {
    arg.split_once(':').unwrap_or((arg, ""))
}

pub fn review(cx: &Cx, arg: &str) -> El {
    let (run_id, node) = split(arg);
    let st = the_run(cx.p, run_id).and_then(|r| state(r, node)).cloned();
    let Some(st) = st.filter(|n| s(n, "status") == Some("waiting")) else {
        return col(vec![
            bar("Review", vec![]),
            empty("This gate was answered.", "", vec![]),
        ]);
    };
    let config = the_run(cx.p, run_id).and_then(|r| gate_config(cx.p, r, node));
    let text = gate_text(cx.p, run_id, node);
    let answer = |decision: &'static str| Do::Gate {
        run: run_id.to_owned(),
        node: node.to_owned(),
        decision,
        back: true,
    };
    let gid = |what: &str| Id::new("review").child((run_id, node, what));
    let busy =
        cx.p.model
            .busy
            .contains(&format!("gate:{}", gate_key(run_id, node)));
    let mut rows = vec![section(
        "",
        vec![say(
            if text.is_empty() {
                "Waiting for approval.".to_owned()
            } else {
                text
            },
            "body",
        )],
    )];
    rows.extend(cx.error(&format!("gate:{}", gate_key(run_id, node))));
    if !busy {
        let mut acts = vec![cx.button(gid("approve"), "Approve", "primary", answer("approve"))];
        if gates::can_request_changes(config.as_ref(), &st) {
            acts.push(present(
                gid("changes"),
                "Request changes",
                format!("gate-changes:{run_id}:{node}"),
                Some("message-square"),
            ));
        }
        acts.push(cx.asking(
            gid("reject"),
            "Reject",
            "Reject this run? It stops here.",
            answer("reject"),
        ));
        rows.push(section("", acts));
    }
    col(vec![
        bar(
            "Review",
            vec![cx.button(gid("done"), "Done", "plain", Do::Go("back".into()))],
        ),
        listing(rows),
    ])
}

pub fn changes(cx: &Cx, arg: &str) -> El {
    let (run_id, node) = split(arg);
    let r = the_run(cx.p, run_id);
    let st = r
        .and_then(|r| state(r, node))
        .cloned()
        .unwrap_or(Value::Null);
    let config = r.and_then(|r| gate_config(cx.p, r, node));
    let from_id = config
        .as_ref()
        .and_then(|c| c.pointer("/feedback/from"))
        .and_then(Value::as_str)
        .unwrap_or("");
    let w = r.and_then(|r| def(cx.p, s(r, "workflowId").unwrap_or("")));
    let from = runs::node_label(runs::node(&nodes(w.as_ref()), from_id), from_id);
    let hint = gates::composer_hint(gates::Mode::Changes, &from, &st, config.as_ref());
    let mut rows = vec![
        cx.field(comment_id(run_id, node), "What should change", true),
        say(hint, "secondary"),
    ];
    rows.extend(cx.error(&format!("gate:{}", gate_key(run_id, node))));
    col(vec![
        bar(
            "Request changes",
            vec![cx.button(
                Id::new("changes-send").child((run_id, node)),
                "Send",
                "primary",
                Do::Gate {
                    run: run_id.to_owned(),
                    node: node.to_owned(),
                    decision: "changes",
                    back: true,
                },
            )],
        ),
        listing(rows),
    ])
}

pub fn edit(cx: &Cx, arg: &str) -> El {
    let (run_id, node) = split(arg);
    col(vec![
        bar(
            "Edit the text",
            vec![cx.button(
                Id::new("edit-save").child((run_id, node)),
                "Save",
                "primary",
                Do::SaveEdit(run_id.to_owned(), node.to_owned()),
            )],
        ),
        listing(vec![
            cx.field(edit_id(run_id, node), "The text to approve", true),
            say(
                "Approving sends this text in place of the original.",
                "secondary",
            ),
        ]),
    ])
}

/// Answers a gate; a comment comes from the changes page, an edit from the edit page.
pub fn answer(
    p: &mut Phone,
    ui: &mut Ui,
    run: &str,
    node: &str,
    decision: &'static str,
    back: bool,
) {
    let key = gate_key(run, node);
    let busy = format!("gate:{key}");
    if p.model.busy.contains(&busy) {
        return;
    }
    let mut params = json!({ "runId": run, "nodeId": node, "decision": decision });
    if decision == "changes" {
        let id = comment_id(run, node);
        let c = ui.edit_text(id).trim().to_owned();
        if c.is_empty() {
            p.model
                .errors
                .insert(busy, "Say what should change.".into());
            return;
        }
        params["comment"] = json!(c);
        ui.set_edit_text(id, "");
    }
    if decision == "approve" {
        if let Some(e) = p.view.edited.remove(&(run.to_owned(), node.to_owned())) {
            params["edited"] = json!(e);
        }
    }
    p.model.errors.remove(&busy);
    p.model.busy.insert(busy);
    p.model
        .call(For::Gate(key, back), "workflow:resolveGate", params);
}

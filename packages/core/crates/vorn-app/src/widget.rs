//! The floating widget (`Widget.tsx`, `widget.css`): every agent's status
//! and its waiting permission requests, in a small window that floats over
//! other apps. It runs as its own process (`vorn-app widget`) on its own
//! connection to vornd, and tells the app that started it what to do with
//! one line on stdout per request: `focus <terminal>`, `show`, `hide`.

use std::collections::HashMap;
use std::sync::mpsc::Receiver;

use serde_json::{json, Value};

use crate::client::rpc::{Event as RpcEvent, Rpc};
#[cfg(not(target_arch = "wasm32"))]
use crate::client::Endpoint;
use crate::components::{agent_icon, AgentMarks};
use crate::runs::s;
use crate::ui::theme::color as k;
use crate::ui::{div, icon, text, Action, El, Event, Id, Rgba, Role, Sense, Ui, WindowRequest};

/// The window's size in each mode, as the desktop sized it.
pub const FULL: (f32, f32) = (280.0, 400.0);
pub const COMPACT: (f32, f32) = (140.0, 36.0);

/// What the widget tells the app on stdout.
pub fn line(request: &Request) -> String {
    match request {
        Request::Focus(id) => format!("focus {id}"),
        Request::Show => "show".to_owned(),
        Request::Hide => "hide".to_owned(),
    }
}

/// A line from the widget, as the app reads it.
pub fn parse(line: &str) -> Option<Request> {
    match line.trim().split_once(' ') {
        Some(("focus", id)) if !id.is_empty() => Some(Request::Focus(id.to_owned())),
        None if line.trim() == "show" => Some(Request::Show),
        None if line.trim() == "hide" => Some(Request::Hide),
        _ => None,
    }
}

/// What the widget asks of the app.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Request {
    /// Bring the app forward on this terminal.
    Focus(String),
    Show,
    Hide,
}

/// One agent the widget lists (`WidgetAgentInfo`).
#[derive(Debug, Clone, PartialEq)]
pub struct Agent {
    pub id: String,
    pub agent_type: String,
    pub name: String,
    pub project: String,
    pub status: String,
}

impl Agent {
    pub fn from_value(v: &Value) -> Option<Agent> {
        let project = s(v, "projectName").unwrap_or("").to_owned();
        Some(Agent {
            id: s(v, "id")?.to_owned(),
            agent_type: s(v, "agentType").unwrap_or("shell").to_owned(),
            name: s(v, "displayName")
                .filter(|n| !n.is_empty())
                .unwrap_or(&project)
                .to_owned(),
            project,
            status: s(v, "status").unwrap_or("idle").to_owned(),
        })
    }
}

/// `labelSuggestion`: what a permission suggestion's button says.
pub fn label_suggestion(sg: &Value, tool: &str) -> String {
    match s(sg, "type") {
        Some("addRules") => {
            let first = sg.pointer("/rules/0");
            let name = first.and_then(|r| s(r, "toolName")).unwrap_or(tool);
            let rule = first.and_then(|r| s(r, "ruleContent")).unwrap_or("");
            if rule.contains("**") {
                let trimmed = rule.replace("/**", "");
                let dir = trimmed
                    .rsplit('/')
                    .next()
                    .filter(|d| !d.is_empty())
                    .unwrap_or(rule);
                format!("Allow {name} in {dir}/")
            } else if !rule.is_empty() {
                let short = if rule.chars().count() > 30 {
                    format!("{}\u{2026}", rule.chars().take(27).collect::<String>())
                } else {
                    rule.to_owned()
                };
                format!("Always allow `{short}`")
            } else {
                format!("Always allow {name}")
            }
        }
        Some("setMode") => match s(sg, "mode") {
            Some("acceptEdits") => "Auto-accept edits".to_owned(),
            Some("plan") => "Switch to plan mode".to_owned(),
            Some(m) => m.to_owned(),
            None => "Set mode".to_owned(),
        },
        other => s(sg, "label").or(other).unwrap_or("").to_owned(),
    }
}

/// The line under a request's tool: a file's name, a command, else what it says.
pub fn context(request: &Value) -> Option<(bool, String)> {
    let input = request.get("toolInput");
    if let Some(path) = input.and_then(|i| s(i, "file_path")) {
        let name = path
            .rsplit('/')
            .next()
            .filter(|n| !n.is_empty())
            .unwrap_or(path);
        return Some((true, name.to_owned()));
    }
    if let Some(cmd) = input.and_then(|i| s(i, "command")) {
        let shown = if cmd.chars().count() > 64 {
            format!("{}\u{2026}", cmd.chars().take(61).collect::<String>())
        } else {
            cmd.to_owned()
        };
        return Some((false, shown));
    }
    s(request, "description").map(|d| (false, d.to_owned()))
}

/// What a click in the widget does.
#[derive(Debug, Clone, PartialEq)]
enum WAct {
    Focus(String),
    Show,
    Hide,
    Compact(bool),
    Respond {
        id: String,
        allow: bool,
        suggestion: Option<Value>,
    },
    Pick {
        id: String,
        question: usize,
        label: String,
    },
    Submit(String),
}

/// The widget's window: its connection, what it lists, and its mode.
pub struct Widget {
    rpc: Rpc,
    events: Receiver<RpcEvent>,
    pub agents: Vec<Agent>,
    pub permissions: Vec<Value>,
    pub compact: bool,
    /// The chosen option of each question, by request and question.
    answers: HashMap<(String, usize), String>,
    marks: AgentMarks,
    acts: HashMap<Id, WAct>,
    /// Requests for the app, written by the caller.
    pub out: Vec<Request>,
}

impl Widget {
    /// Connects to vornd at `ep` and asks for the list.
    #[cfg(not(target_arch = "wasm32"))]
    pub fn connect(ep: &Endpoint, wake: crate::client::rpc::Wake) -> Result<Widget, String> {
        let (rpc, events) = Rpc::connect(ep, wake).map_err(|e| e.to_string())?;
        let topics = [
            "widget:status-update",
            "widget:permission-request",
            "widget:permission-cancelled",
        ];
        rpc.call(
            "subscribe:set",
            json!({ "topics": topics, "terminalBytes": false }),
        )
        .map_err(|e| e.to_string())?;
        let agents = rpc
            .call("terminal:listActive", Value::Null)
            .ok()
            .and_then(|v| {
                v.as_array()
                    .map(|a| a.iter().filter_map(Agent::from_value).collect())
            })
            .unwrap_or_default();
        Ok(Widget {
            rpc,
            events,
            agents,
            permissions: Vec::new(),
            compact: false,
            answers: HashMap::new(),
            marks: AgentMarks::default(),
            acts: HashMap::new(),
            out: Vec::new(),
        })
    }

    /// Folds in what vornd said since the last frame.
    pub fn pump(&mut self) {
        while let Ok(e) = self.events.try_recv() {
            let RpcEvent::Notification { method, params } = e else {
                continue;
            };
            self.notified(&method, &params);
        }
    }

    pub fn notified(&mut self, method: &str, params: &Value) {
        match method {
            "widget:status-update" => {
                if let Some(list) = params.as_array() {
                    self.agents = list.iter().filter_map(Agent::from_value).collect();
                }
            }
            "widget:permission-request" => {
                let id = s(params, "requestId");
                if !self.permissions.iter().any(|p| s(p, "requestId") == id) {
                    self.permissions.push(params.clone());
                }
            }
            "widget:permission-cancelled" => {
                let id = params.as_str();
                self.permissions.retain(|p| s(p, "requestId") != id);
            }
            _ => {}
        }
    }

    fn on(&mut self, id: Id, act: WAct) -> Id {
        self.acts.insert(id, act);
        id
    }

    fn respond(&mut self, id: &str, allow: bool, extra: Value) {
        let mut params = json!({ "requestId": id, "allow": allow });
        if let (Some(p), Some(e)) = (params.as_object_mut(), extra.as_object()) {
            p.extend(e.clone());
        }
        let _ = self.rpc.request("permission:resolve", params);
        self.permissions.retain(|p| s(p, "requestId") != Some(id));
    }

    pub fn click(&mut self, ui: &mut Ui, id: Id) {
        let Some(act) = self.acts.get(&id).cloned() else {
            return;
        };
        match act {
            WAct::Focus(t) => self.out.push(Request::Focus(t)),
            WAct::Show => self.out.push(Request::Show),
            WAct::Hide => self.out.push(Request::Hide),
            WAct::Compact(c) => {
                self.compact = c;
                let (w, h) = if c { COMPACT } else { FULL };
                ui.request(WindowRequest::Resize(w, h));
            }
            WAct::Respond {
                id,
                allow,
                suggestion,
            } => {
                let extra =
                    suggestion.map_or(json!({}), |sg| json!({ "updatedPermissions": [sg] }));
                self.respond(&id, allow, extra);
            }
            WAct::Pick {
                id,
                question,
                label,
            } => {
                self.answers.insert((id, question), label);
            }
            WAct::Submit(id) => {
                let Some(req) = self
                    .permissions
                    .iter()
                    .find(|p| s(p, "requestId") == Some(id.as_str()))
                else {
                    return;
                };
                let mut answers = serde_json::Map::new();
                for (i, _) in questions(req).iter().enumerate() {
                    if let Some(a) = self.answers.get(&(id.clone(), i)) {
                        answers.insert(i.to_string(), json!(a));
                    }
                }
                let mut input = req.get("toolInput").cloned().unwrap_or_else(|| json!({}));
                if let Some(o) = input.as_object_mut() {
                    o.insert("answers".into(), Value::Object(answers));
                }
                self.respond(&id, true, json!({ "updatedInput": input }));
            }
        }
    }

    pub fn view(&mut self, ui: &mut Ui) -> El {
        self.marks.load(ui);
        self.acts.clear();
        if self.compact {
            return self.compact_view(ui);
        }
        let running = self.agents.iter().filter(|a| a.status == "running").count();
        let total = self.agents.len();
        let mut left = div()
            .row()
            .items_center()
            .gap(8.0)
            .child(self.app_button(ui, "Vorn", 13.0));
        if total > 0 {
            left = left.child(
                text(format!("{running}/{total}"), 11.0, Rgba::white(0.4))
                    .line_height(16.0)
                    .silent(),
            );
        }
        let collapse = self.on(Id::new("w-collapse"), WAct::Compact(true));
        let hide = self.on(Id::new("w-hide"), WAct::Hide);
        let header = div()
            .row()
            .items_center()
            .justify_between()
            .px(12.0)
            .py(10.0)
            .border_bottom(1.0, Rgba::white(0.06))
            .id(Id::new("w-drag"))
            .sense(Sense::WINDOW_DRAG)
            .child(left)
            .child(
                div()
                    .row()
                    .items_center()
                    .gap(2.0)
                    .child(header_button(ui, collapse, "Collapse", "minus"))
                    .child(header_button(ui, hide, "Hide widget", "x")),
            );
        let mut body = div()
            .id(Id::new("w-body"))
            .col()
            .max_h(320.0)
            .scroll_y()
            .py(4.0);
        if self.agents.is_empty() {
            body = body.child(
                div()
                    .px(12.0)
                    .py(20.0)
                    .row()
                    .justify_center()
                    .child(text("No active agents", 12.0, Rgba::white(0.3)).line_height(16.0)),
            );
        }
        let agents = self.agents.clone();
        for a in &agents {
            body = body.child(self.agent_section(ui, a));
        }
        div()
            .col()
            .w_full()
            .child(container(14.0).child(header).child(body))
    }

    fn app_button(&mut self, ui: &Ui, label: &str, size: f32) -> El {
        let id = self.on(Id::new("w-app"), WAct::Show);
        let alpha = 1.0 - 0.2 * ui.hover(id);
        div()
            .id(id)
            .on(Action::Click)
            .ring(3.0)
            .rounded(3.0)
            .role(Role::Button, "Open Vorn")
            .child(
                text(label, size, k::BRONZO.alpha(alpha))
                    .weight(600)
                    .line_height(size * 1.3)
                    .silent(),
            )
    }

    fn agent_section(&mut self, ui: &Ui, a: &Agent) -> El {
        let id = self.on(
            Id::new("w-row").child(a.id.as_str()),
            WAct::Focus(a.id.clone()),
        );
        let hot = ui.hover(id);
        let row = div()
            .id(id)
            .on(Action::Click)
            .row()
            .items_center()
            .gap(10.0)
            .px(12.0)
            .py(8.0)
            .bg(Rgba::white(0.06 * hot))
            .role(Role::Button, format!("{} \u{2014} {}", a.project, a.status))
            .child(div().size(24.0, 24.0).center().shrink0().child(agent_icon(
                &self.marks,
                &a.agent_type,
                18.0,
            )))
            .child(
                div().grow().min_w0().child(
                    text(a.name.as_str(), 12.0, Rgba::white(0.9))
                        .weight(500)
                        .line_height(16.0)
                        .truncate(),
                ),
            )
            .child(status_dot(&a.status, 7.0));
        let mine: Vec<Value> = self
            .permissions
            .iter()
            .filter(|p| s(p, "terminalId") == Some(a.id.as_str()))
            .cloned()
            .collect();
        let mut col = div().col().child(row);
        if !mine.is_empty() {
            let mut list = div()
                .col()
                .gap(6.0)
                .px(8.0)
                .py(6.0)
                .border_bottom(1.0, Rgba::white(0.06));
            for p in &mine {
                list = list.child(self.permission_card(ui, p));
            }
            col = col.child(list);
        }
        col
    }

    fn permission_card(&mut self, ui: &Ui, p: &Value) -> El {
        let id = s(p, "requestId").unwrap_or("").to_owned();
        let tool = s(p, "toolName").unwrap_or("").to_owned();
        let qs = questions(p);
        if tool == "AskUserQuestion" && !qs.is_empty() {
            return self.question_card(ui, &id, &qs);
        }
        let mut card = card().child(div().row().mb(6.0).child(badge(&tool)));
        if let Some((file, ctx)) = context(p) {
            card = card.child(
                div()
                    .row()
                    .items_center()
                    .gap(5.0)
                    .mb(8.0)
                    .px(7.0)
                    .py(4.0)
                    .rounded(5.0)
                    .bg(Rgba::hexa(0, 0.25))
                    .child(
                        text(if file { "\u{25c8}" } else { "$" }, 10.0, Rgba::white(0.25))
                            .line_height(14.0)
                            .shrink0()
                            .silent(),
                    )
                    .child(
                        div().grow().min_w0().child(
                            text(ctx, 11.0, Rgba::white(0.75))
                                .mono()
                                .line_height(15.0)
                                .truncate(),
                        ),
                    ),
            );
        }
        let allow = self.on(
            Id::new("w-allow").child(id.as_str()),
            WAct::Respond {
                id: id.clone(),
                allow: true,
                suggestion: None,
            },
        );
        let deny = self.on(
            Id::new("w-deny").child(id.as_str()),
            WAct::Respond {
                id: id.clone(),
                allow: false,
                suggestion: None,
            },
        );
        card = card.child(
            div()
                .row()
                .gap(6.0)
                .child(action(ui, allow, "Allow", true, true))
                .child(action(ui, deny, "Deny", false, true)),
        );
        let suggestions: Vec<Value> = p
            .get("permissionSuggestions")
            .and_then(Value::as_array)
            .cloned()
            .unwrap_or_default();
        if !suggestions.is_empty() {
            let mut list = div().col().gap(4.0).mt(5.0);
            for (i, sg) in suggestions.into_iter().enumerate() {
                let label = label_suggestion(&sg, &tool);
                let sid = self.on(
                    Id::new("w-suggest").child(id.as_str()).child(i),
                    WAct::Respond {
                        id: id.clone(),
                        allow: true,
                        suggestion: Some(sg),
                    },
                );
                let hot = ui.hover(sid);
                list = list.child(
                    div()
                        .id(sid)
                        .on(Action::Click)
                        .w_full()
                        .px(8.0)
                        .py(4.0)
                        .rounded(5.0)
                        .border(1.0, Rgba::white(0.06).mix(Rgba::white(0.18), hot))
                        .bg(Rgba::white(0.08 * hot))
                        .role(Role::Button, &label)
                        .child(
                            text(
                                label.as_str(),
                                10.0,
                                Rgba::white(0.35).mix(Rgba::white(0.7), hot),
                            )
                            .weight(500)
                            .line_height(14.0)
                            .silent(),
                        ),
                );
            }
            card = card.child(list);
        }
        card
    }

    fn question_card(&mut self, ui: &Ui, id: &str, qs: &[Value]) -> El {
        let mut card = card().child(div().row().mb(6.0).child(badge("Question")));
        let mut all = true;
        for (qi, q) in qs.iter().enumerate() {
            let chosen = self.answers.get(&(id.to_owned(), qi)).cloned();
            all &= chosen.is_some();
            let mut block = div().col().mb(8.0);
            if let Some(h) = s(q, "header").filter(|h| !h.is_empty()) {
                block = block.child(
                    text(h.to_uppercase(), 10.0, Rgba::white(0.35))
                        .weight(600)
                        .tracking(0.04)
                        .line_height(14.0)
                        .mb(3.0),
                );
            }
            block = block.child(
                text(s(q, "question").unwrap_or(""), 11.0, Rgba::white(0.8))
                    .line_height(15.4)
                    .wrapping()
                    .mb(6.0),
            );
            let mut opts = div().col().gap(3.0);
            for o in q
                .get("options")
                .and_then(Value::as_array)
                .into_iter()
                .flatten()
            {
                let label = s(o, "label").unwrap_or("").to_owned();
                let on = chosen.as_deref() == Some(label.as_str());
                let oid = self.on(
                    Id::new("w-opt").child(id).child(qi).child(label.as_str()),
                    WAct::Pick {
                        id: id.to_owned(),
                        question: qi,
                        label: label.clone(),
                    },
                );
                let hot = ui.hover(oid);
                let (bg, border, fg) = if on {
                    (Rgba::white(0.12), Rgba::white(0.35), Rgba::white(0.9))
                } else {
                    (
                        Rgba::white(0.05 * hot),
                        Rgba::white(0.08),
                        Rgba::white(0.55).mix(Rgba::white(0.8), hot),
                    )
                };
                opts = opts.child(
                    div()
                        .id(oid)
                        .on(Action::Click)
                        .w_full()
                        .px(8.0)
                        .py(5.0)
                        .rounded(5.0)
                        .border(1.0, border)
                        .bg(bg)
                        .role(Role::Button, &label)
                        .selected(on)
                        .child(text(label.as_str(), 11.0, fg).line_height(15.0).silent()),
                );
            }
            card = card.child(block.child(opts));
        }
        let submit = self.on(Id::new("w-submit").child(id), WAct::Submit(id.to_owned()));
        let skip = self.on(
            Id::new("w-skip").child(id),
            WAct::Respond {
                id: id.to_owned(),
                allow: false,
                suggestion: None,
            },
        );
        card.child(
            div()
                .row()
                .gap(6.0)
                .child(action(ui, submit, "Submit", true, all))
                .child(action(ui, skip, "Skip", false, true)),
        )
    }

    fn compact_view(&mut self, ui: &mut Ui) -> El {
        let count = |st: &str| self.agents.iter().filter(|a| a.status == st).count();
        let (running, waiting, errored) = (count("running"), count("waiting"), count("error"));
        let mut dots = div().row().items_center().gap(6.0);
        for (n, st) in [
            (running, "running"),
            (waiting, "waiting"),
            (errored, "error"),
        ] {
            if n > 0 {
                dots = dots.child(
                    div()
                        .row()
                        .items_center()
                        .gap(3.0)
                        .child(status_dot(st, 6.0))
                        .child(text(n.to_string(), 10.0, Rgba::white(0.6)).line_height(14.0)),
                );
            }
        }
        if self.agents.is_empty() {
            dots = dots.child(text("0", 10.0, Rgba::white(0.3)).line_height(14.0));
        }
        let expand = self.on(Id::new("w-expand"), WAct::Compact(false));
        let hide = self.on(Id::new("w-hide"), WAct::Hide);
        let app = self.app_button(ui, "V_", 11.0);
        let pill = container(20.0)
            .id(Id::new("w-drag"))
            .sense(Sense::WINDOW_DRAG)
            .row()
            .items_center()
            .gap(8.0)
            .px(10.0)
            .py(6.0)
            .child(app)
            .child(dots)
            .child(div().grow())
            .child(
                div()
                    .row()
                    .items_center()
                    .gap(2.0)
                    .child(mode_button(ui, expand, "Expand panel", "panel-top"))
                    .child(mode_button(ui, hide, "Hide widget", "x")),
            );
        div().col().w_full().child(pill)
    }
}

/// The request's questions, for an `AskUserQuestion`.
fn questions(p: &Value) -> Vec<Value> {
    p.get("questions")
        .and_then(Value::as_array)
        .cloned()
        .unwrap_or_default()
}

/// `.widget-container`: the overlay at 96 %, a hairline, rounded.
fn container(radius: f32) -> El {
    div()
        .col()
        .w_full()
        .bg(k::SURFACE_OVERLAY.alpha(0.96))
        .border(1.0, Rgba::white(0.08))
        .rounded(radius)
        .clip()
}

/// `.permission-card`.
fn card() -> El {
    div()
        .col()
        .px(10.0)
        .py(8.0)
        .rounded(8.0)
        .bg(Rgba::white(0.04))
        .border(1.0, Rgba::white(0.1))
}

/// `.permission-tool-badge`.
fn badge(label: &str) -> El {
    div()
        .px(6.0)
        .py(1.0)
        .rounded(4.0)
        .bg(Rgba::white(0.08))
        .child(
            text(label, 10.0, Rgba::white(0.45))
                .weight(600)
                .mono()
                .line_height(14.0),
        )
}

/// `.status-dot`: only waiting takes the accent.
fn status_dot(status: &str, size: f32) -> El {
    let color = match status {
        "running" => k::INK,
        "waiting" => k::BRONZO,
        "error" => k::DANGER,
        _ => k::INK_GHOST,
    };
    div()
        .size(size, size)
        .rounded(size / 2.0)
        .shrink0()
        .bg(color)
}

/// `.permission-btn`: Allow neutral and raised, Deny quiet until hovered.
fn action(ui: &Ui, id: Id, label: &str, allow: bool, enabled: bool) -> El {
    let hot = if enabled { ui.hover(id) } else { 0.0 };
    let (bg, border, fg) = if allow {
        (
            Rgba::white(0.09).mix(Rgba::white(0.16), hot),
            Rgba::white(0.14).mix(Rgba::white(0.28), hot),
            Rgba::white(0.8).mix(k::INK, hot),
        )
    } else {
        (
            Rgba::white(0.04).mix(k::DANGER.alpha(0.14), hot),
            Rgba::white(0.08).mix(k::DANGER.alpha(0.3), hot),
            Rgba::white(0.4).mix(k::DANGER, hot),
        )
    };
    let mut b = div()
        .id(id)
        .flex(1.0)
        .row()
        .justify_center()
        .py(5.0)
        .rounded(5.0)
        .bg(bg)
        .border(1.0, border)
        .role(Role::Button, label)
        .child(text(label, 11.0, fg).weight(600).line_height(15.0).silent());
    if enabled {
        b = b.on(Action::Click).ring(5.0);
    } else {
        b = b.opacity(0.4).disabled(true);
    }
    b
}

fn header_button(ui: &Ui, id: Id, label: &str, glyph: &str) -> El {
    let hot = ui.hover(id);
    div()
        .id(id)
        .on(Action::Click)
        .ring(5.0)
        .size(22.0, 22.0)
        .center()
        .rounded(5.0)
        .bg(Rgba::white(0.08 * hot))
        .role(Role::Button, label)
        .child(glyph_el(
            glyph,
            12.0,
            Rgba::white(0.35).mix(Rgba::white(0.75), hot),
        ))
}

fn mode_button(ui: &Ui, id: Id, label: &str, glyph: &str) -> El {
    let hot = ui.hover(id);
    div()
        .id(id)
        .on(Action::Click)
        .ring(4.0)
        .size(20.0, 20.0)
        .center()
        .rounded(4.0)
        .bg(Rgba::white(0.1 * hot))
        .role(Role::Button, label)
        .child(glyph_el(
            glyph,
            10.0,
            Rgba::white(0.3).mix(Rgba::white(0.75), hot),
        ))
}

/// The widget's own small glyphs, drawn as the desktop drew them.
fn glyph_el(name: &str, size: f32, color: Rgba) -> El {
    let body = match name {
        "minus" => r#"<path d="M3 6H9" stroke-width="1.5" stroke-linecap="round"/>"#,
        "x" => r#"<path d="M3 3L9 9M9 3L3 9" stroke-width="1.5" stroke-linecap="round"/>"#,
        _ => {
            r#"<rect x="1.8" y="1.8" width="8.4" height="8.4" rx="1.4" stroke-width="1.4"/><path d="M1.8 4.2H10.2" stroke-width="1.4"/>"#
        }
    };
    icon(
        format!(
            r##"<svg xmlns="http://www.w3.org/2000/svg" width="12" height="12" viewBox="0 0 12 12" fill="none" stroke="#ffffff">{body}</svg>"##
        ),
        size,
        color,
    )
}

/// The widget as a window of its own.
pub struct WidgetWindow {
    pub widget: Widget,
}

impl crate::ui::WindowApp for WidgetWindow {
    fn view(&mut self, ui: &mut Ui) -> El {
        self.widget.pump();
        self.widget.view(ui)
    }

    fn event(&mut self, ui: &mut Ui, event: Event) {
        if let Event::Click(id) = event {
            self.widget.click(ui, id);
        }
        flush(&mut self.widget, ui);
    }

    fn background(&self) -> Rgba {
        Rgba::TRANSPARENT
    }
}

/// Writes the widget's requests to stdout; hiding ends the process.
fn flush(w: &mut Widget, ui: &mut Ui) {
    use std::io::Write;
    let mut out = std::io::stdout().lock();
    for r in std::mem::take(&mut w.out) {
        let _ = writeln!(out, "{}", line(&r));
        if r == Request::Hide {
            let _ = out.flush();
            ui.request(WindowRequest::Close);
        }
    }
    let _ = out.flush();
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn suggestions_read_as_the_widget_words_them() {
        let dir = json!({ "type": "addRules", "rules": [{ "toolName": "Edit", "ruleContent": "/repo/src/**" }] });
        assert_eq!(label_suggestion(&dir, "Bash"), "Allow Edit in src/");
        let cmd = json!({ "type": "addRules", "rules": [{ "ruleContent": "npm run test -- --watch=false --coverage" }] });
        assert_eq!(
            label_suggestion(&cmd, "Bash"),
            "Always allow `npm run test -- --watch=fal\u{2026}`"
        );
        assert_eq!(
            label_suggestion(&json!({ "type": "addRules" }), "Bash"),
            "Always allow Bash"
        );
        assert_eq!(
            label_suggestion(&json!({ "type": "setMode", "mode": "plan" }), "x"),
            "Switch to plan mode"
        );
        assert_eq!(
            label_suggestion(&json!({ "type": "other", "label": "L" }), "x"),
            "L"
        );
    }

    #[test]
    fn the_context_line_prefers_a_file_then_a_command() {
        let file = json!({ "toolInput": { "file_path": "/a/b/c.rs", "command": "ls" } });
        assert_eq!(context(&file), Some((true, "c.rs".into())));
        let long = "x".repeat(70);
        let cmd = json!({ "toolInput": { "command": long } });
        assert_eq!(context(&cmd).map(|(_, c)| c.chars().count()), Some(62));
        assert_eq!(
            context(&json!({ "description": "d" })),
            Some((false, "d".into()))
        );
        assert_eq!(context(&json!({})), None);
    }

    #[test]
    fn lines_to_the_app_round_trip() {
        for r in [Request::Focus("t 1".into()), Request::Show, Request::Hide] {
            assert_eq!(parse(&line(&r)), Some(r));
        }
        assert_eq!(parse("nonsense"), None);
    }

    #[test]
    fn agents_fall_back_to_their_project_name() {
        let a = Agent::from_value(
            &json!({ "id": "1", "agentType": "claude", "displayName": "",
                                            "projectName": "api", "status": "waiting" }),
        )
        .expect("an agent");
        assert_eq!((a.name.as_str(), a.status.as_str()), ("api", "waiting"));
    }
}

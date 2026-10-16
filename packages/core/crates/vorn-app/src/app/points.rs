//! Extensions' contributions against vornd: the list, the rows its
//! templates draw, what pressing their controls calls, and window points,
//! each a process of its own on a desktop and a floating panel elsewhere.

#[cfg(not(target_arch = "wasm32"))]
use std::io::{BufRead, BufReader};
use std::process::Child;
#[cfg(not(target_arch = "wasm32"))]
use std::process::Stdio;
use std::sync::mpsc::Receiver;
#[cfg(not(target_arch = "wasm32"))]
use std::sync::{mpsc::channel, Arc};

use serde_json::{json, Value};

use super::{App, Call};
use crate::client::RpcError;
use crate::points::{self, kind, Holder, PointAct, Press, Request};
use crate::runs::s;
use crate::ui::{Id, Ui, WindowRequest};

/// A contributions call on its way.
#[derive(Debug)]
pub(super) enum PointCall {
    List,
    /// The node types and trigger types workflows can use, read with the list.
    NodeTypes,
    Rows(String, String),
    /// A press or a choice; `holder` closes when the press asked it to.
    Called,
    Builtins,
    /// A sign-in action's window target, asked of the extension; the key it opens for.
    SignIn(String),
    /// A built-in switched; it goes back to `was` if vornd refuses.
    SetBuiltin {
        id: String,
        was: bool,
    },
}

/// A window point's process and the lines it writes.
pub(super) struct Running {
    child: Child,
    lines: Receiver<Request>,
}

impl Drop for Running {
    fn drop(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}

impl App {
    /// Window points are processes of their own only on a desktop with windows.
    fn own_windows(&self) -> bool {
        !cfg!(target_arch = "wasm32") && self.windowed
    }

    pub(super) fn run_point(&mut self, ui: &mut Ui, a: PointAct) {
        match a {
            PointAct::Press {
                extension,
                press,
                holder,
            } => {
                self.st.points.asking = None;
                if self.st.points.more.take().is_some() {
                    ui.close_menu();
                }
                self.press(ui, &extension, &press);
                if press.close {
                    match holder {
                        Some(Holder::Dialog) => self.st.points.dialog = None,
                        Some(Holder::Window(key)) => self.close_window(&key),
                        None => {}
                    }
                }
            }
            PointAct::Ask(id) => {
                if self.st.points.more.take().is_some() {
                    ui.close_menu();
                }
                self.st.points.asking = id;
            }
            // The kit's menu state too, so a press outside or Escape closes it.
            PointAct::More(id) => {
                self.st.points.more = id;
                match id {
                    Some(m) => ui.open_menu(Id(m), Id(m)),
                    None => ui.close_menu(),
                }
            }
            PointAct::Choose { key, args } => {
                let Some(p) = self.st.points.get(&key).cloned() else {
                    return;
                };
                // The Workflow editor's Run asks for a run's inputs and session here, as it always did.
                if p.extension == crate::workflows::EDITOR && p.call.as_deref() == Some("run") {
                    let args: Value = serde_json::from_str(&args).unwrap_or_default();
                    let text = |k: &str| args.get(k).and_then(Value::as_str).map(str::to_owned);
                    if let Some(id) = text("id") {
                        let source = text("sessionId");
                        self.run(
                            ui,
                            crate::act::Act::RunWorkflow { id, source },
                            Default::default(),
                        );
                    }
                    return;
                }
                if let Some(w) = &p.window {
                    // Whichever of its extension's windows is open hides.
                    let mine = format!("{}/{}/", p.extension, kind::WINDOWS);
                    let open: Vec<String> = (self.st.points.windows.iter())
                        .filter(|k| k.starts_with(&mine))
                        .cloned()
                        .collect();
                    if open.is_empty() {
                        self.open_window(&format!("{mine}{w}"));
                    }
                    for key in open {
                        self.close_window(&key);
                    }
                } else if let Some(params) = points::choose_params(&p, &args) {
                    self.call(Call::Point(PointCall::Called), "extension:call", params);
                }
            }
            PointAct::Answer { key, row, allow } => {
                let Some(p) = self.st.points.get(&key) else {
                    return;
                };
                let action = if allow {
                    p.call.clone()
                } else {
                    p.deny.clone()
                };
                if let Some(action) = action {
                    let params =
                        json!({ "id": p.extension, "action": action, "args": { "id": row } });
                    self.call(Call::Point(PointCall::Called), "extension:call", params);
                }
            }
            PointAct::Window { key, open } => {
                if open {
                    self.open_window(&key);
                } else {
                    self.close_window(&key);
                }
            }
            PointAct::CloseDialog => self.st.points.dialog = None,
            PointAct::Field {
                extension,
                call,
                row,
            } => {
                let id = ui.focus();
                let text = id.map(|id| ui.edit_text(id).to_owned()).unwrap_or_default();
                self.point_field(&extension, &call, row.as_deref(), &text);
            }
            PointAct::Settings(key) => {
                self.st.settings.ext_page = Some(key);
                self.st.settings.open = true;
            }
            PointAct::Builtin { id, on } => {
                let pts = &mut self.st.points;
                if pts.switching.is_some() {
                    return;
                }
                pts.set_builtin(&id, on);
                pts.switching = Some(id.clone());
                pts.switch_error = None;
                pts.off_note = (!on).then(|| id.clone());
                self.call(
                    Call::Point(PointCall::SetBuiltin {
                        id: id.clone(),
                        was: !on,
                    }),
                    "extension:setBuiltin",
                    json!({ "id": id, "on": on }),
                );
            }
        }
    }

    /// What a template's button or toggle asks for, in the order the template reads.
    fn press(&mut self, ui: &mut Ui, extension: &str, press: &Press) {
        // The board's menus and add buttons are the app's, as the board always was.
        if extension == crate::screen::tasks::board::TASKS {
            if let Some(act) = crate::tasks::board_press(press) {
                self.run_task(ui, act);
                return;
            }
        }
        if let Some(params) = points::call_params(extension, press) {
            self.call(Call::Point(PointCall::Called), "extension:call", params);
        }
        if let Some(url) = press.open.as_deref().filter(|u| u.starts_with("https://")) {
            super::tasks::open_url(url);
        }
        if let Some(id) = press.focus.as_deref() {
            if self.store.session(id).is_some() {
                self.st.view = crate::screen::MainView::Sessions;
                self.st.selected = Some(id.to_owned());
            }
        }
        if press.show || press.focus.is_some() {
            ui.request(WindowRequest::Focus);
        }
        if let Some(w) = &press.window {
            self.open_window(&format!("{extension}/{}/{w}", kind::WINDOWS));
        }
        // The extension says where the window signs in; the client holds the window.
        if let Some(key) = press.signin.as_deref().filter(|k| !k.is_empty()) {
            self.call(
                Call::Point(PointCall::SignIn(key.to_owned())),
                "extension:call",
                json!({ "id": extension, "action": "signin", "args": { "id": key } }),
            );
        }
    }

    /// A one-line field's Return, with what it holds.
    pub(super) fn point_field(
        &mut self,
        extension: &str,
        call: &str,
        row: Option<&str>,
        text: &str,
    ) {
        let mut args = json!({ "value": text });
        if let Some(id) = row {
            args["id"] = json!(id);
        }
        self.call(
            Call::Point(PointCall::Called),
            "extension:call",
            json!({ "id": extension, "action": call, "args": args }),
        );
    }

    pub(super) fn point_answered(&mut self, call: PointCall, answer: Result<Value, RpcError>) {
        match call {
            PointCall::List => match answer {
                Ok(Value::Array(list)) => self.st.points.listed(&list),
                Ok(_) => self.st.points.listed(&[]),
                // An older vornd has no contributions; ask no more.
                Err(_) => self.st.points.listed(&[]),
            },
            PointCall::NodeTypes => {
                self.st.points.node_types = match answer {
                    Ok(v) => v
                        .get("nodes")
                        .and_then(Value::as_array)
                        .cloned()
                        .unwrap_or_default(),
                    Err(_) => Vec::new(),
                };
            }
            PointCall::Rows(ext, coll) => {
                let rows = match &answer {
                    Ok(Value::Array(rows)) => Some(rows.as_slice()),
                    _ => None,
                };
                self.st.points.fill(&ext, &coll, rows);
            }
            PointCall::Called => match answer {
                Ok(v) => {
                    let (open, toast) = points::answered(&v);
                    if let Some(t) = toast {
                        self.st.toast(t, false);
                    }
                    if let Some(id) = open {
                        let ext = self.called_extension(&v);
                        self.open_dialog(ext.as_deref(), &id);
                    }
                }
                Err(e) => self.st.toast(e.to_string(), true),
            },
            PointCall::SignIn(key) => {
                let failed = match answer {
                    Ok(target) => self.open_signin(&key, &target, crate::signin::From::Page),
                    Err(e) => Some(e.to_string()),
                };
                if let Some(e) = failed {
                    self.st.toast(e, true);
                }
            }
            PointCall::SetBuiltin { id, was } => {
                let pts = &mut self.st.points;
                pts.switching = None;
                if let Err(e) = answer {
                    pts.set_builtin(&id, was);
                    pts.off_note = None;
                    pts.switch_error = Some((id, e.to_string()));
                }
            }
            PointCall::Builtins => match answer {
                Ok(Value::Array(list)) => {
                    self.st.points.builtins = list;
                    self.st.points.builtins_loaded = true;
                }
                Ok(_) => {}
                Err(e) => {
                    self.st.points.builtins_loaded = false;
                    self.st.toast(e.to_string(), true);
                }
            },
        }
    }

    /// The extension a dialog answer came from: its `extension`, else the one dialog of that id.
    fn called_extension(&self, answer: &Value) -> Option<String> {
        s(answer, "extension").map(str::to_owned)
    }

    fn open_dialog(&mut self, extension: Option<&str>, id: &str) {
        let found = self
            .st
            .points
            .at(kind::DIALOGS)
            .find(|p| p.id == id && extension.is_none_or(|e| p.extension == e))
            .map(points::Point::key);
        match found {
            Some(key) => self.st.points.dialog = Some(key),
            None => self.st.toast(format!("No dialog named {id}"), true),
        }
    }

    pub(super) fn points_notified(&mut self, method: &str, params: &Value) {
        match method {
            "extension:rowsChanged" => {
                let (Some(ext), Some(coll)) = (s(params, "extensionId"), s(params, "collection"))
                else {
                    return;
                };
                let ops = params["ops"].as_array().cloned().unwrap_or_default();
                let raised = self.st.points.changed(ext, coll, &ops);
                self.raised.extend(raised);
            }
            "extension:contributionsChanged" => {
                self.st.points.stale = true;
                self.st.points.builtins_loaded = false;
            }
            _ => {}
        }
    }

    /// Once a frame: the list when it is stale, the rows no one asked for,
    /// settings pages' calls on showing, and what window processes asked.
    pub(super) fn points_tick(&mut self, ui: &mut Ui) {
        let loading = |c: &Call| matches!(c, Call::Point(PointCall::List));
        if (!self.st.points.loaded || self.st.points.stale)
            && !self.calls.iter().any(|(c, _)| loading(c))
        {
            self.st.points.stale = false;
            self.call(
                Call::Point(PointCall::List),
                "extension:contributions",
                Value::Null,
            );
            self.call(
                Call::Point(PointCall::NodeTypes),
                "workflow:nodeTypes",
                Value::Null,
            );
        }
        for (ext, coll) in self.st.points.to_ask() {
            self.call(
                Call::Point(PointCall::Rows(ext.clone(), coll.clone())),
                "extension:rows",
                json!({ "id": ext, "collection": coll }),
            );
        }
        let palette = self.st.dialog == Some(crate::state::Dialog::Palette);
        if !self.st.points.builtins_loaded && (self.st.settings.open || palette) {
            self.st.points.builtins_loaded = true;
            self.call(
                Call::Point(PointCall::Builtins),
                "extension:builtins",
                Value::Null,
            );
        }
        self.st.points.floating = !self.own_windows();
        // A window over other apps comes up only while Vorn's is behind them.
        let raised = std::mem::take(&mut self.raised);
        if self.st.points.floating || !ui.window_focused() {
            for key in raised {
                self.open_window(&key);
            }
        }
        self.show_calls();
        self.window_tick(ui);
    }

    /// A settings page's `call` runs each time the page shows.
    fn show_calls(&mut self) {
        let page = self
            .st
            .settings
            .open
            .then(|| self.st.settings.ext_page.clone())
            .flatten();
        let showing: Vec<String> = page.into_iter().collect();
        let showing: Vec<String> = showing.into_iter().chain(self.open_panes()).collect();
        self.st.points.shown.retain(|k| showing.contains(k));
        for key in showing {
            if self.st.points.shown.contains(&key) {
                continue;
            }
            let Some(p) = self.st.points.get(&key).cloned() else {
                continue;
            };
            self.st.points.shown.insert(key);
            if let Some(params) = points::choose_params(&p, "{}") {
                self.call(Call::Point(PointCall::Called), "extension:call", params);
            }
        }
    }

    /// The template panes open on cards, by point key.
    fn open_panes(&self) -> Vec<String> {
        self.st
            .ext
            .panes
            .values()
            .filter_map(crate::extensions::view_key)
            .collect()
    }

    fn open_window(&mut self, key: &str) {
        let Some(p) = self.st.points.get(key) else {
            return;
        };
        // An extension shows one window at a time: one opens in place of another.
        let mine = format!("{}/{}/", p.extension, kind::WINDOWS);
        let others: Vec<String> = (self.st.points.windows.iter())
            .filter(|k| k.starts_with(&mine) && *k != key)
            .cloned()
            .collect();
        for k in others {
            self.close_window(&k);
        }
        self.st.points.windows.insert(key.to_owned());
        if self.own_windows() && !self.panels.contains_key(key) {
            if let Some(r) = self.start_panel(key) {
                self.panels.insert(key.to_owned(), r);
            }
        }
    }

    fn close_window(&mut self, key: &str) {
        self.st.points.windows.remove(key);
        self.panels.remove(key);
    }

    #[cfg(target_arch = "wasm32")]
    fn start_panel(&mut self, _key: &str) -> Option<Running> {
        None
    }

    #[cfg(not(target_arch = "wasm32"))]
    fn start_panel(&mut self, key: &str) -> Option<Running> {
        let exe = std::env::current_exe().ok()?;
        let dir = self.data_dir.clone()?;
        let mut cmd = vorn_spawn::command(exe);
        cmd.arg("panel")
            .arg("--point")
            .arg(key)
            .arg("--data-dir")
            .arg(dir)
            .stdin(Stdio::null())
            .stdout(Stdio::piped())
            .stderr(Stdio::null());
        let mut child = cmd.spawn().ok()?;
        let (tx, lines) = channel();
        if let Some(out) = child.stdout.take() {
            let wake = Arc::clone(&self.wake);
            std::thread::spawn(move || {
                for line in BufReader::new(out).lines().map_while(Result::ok) {
                    if let Some(r) = points::parse(&line) {
                        if tx.send(r).is_err() {
                            return;
                        }
                        wake();
                    }
                }
            });
        }
        Some(Running { child, lines })
    }

    /// What window processes asked, and which have gone.
    fn window_tick(&mut self, ui: &mut Ui) {
        if !self.own_windows() {
            return;
        }
        let mut asked = Vec::new();
        let mut gone = Vec::new();
        for (key, r) in &mut self.panels {
            asked.extend(r.lines.try_iter());
            if matches!(r.child.try_wait(), Ok(Some(_))) {
                // Its last lines (an `open` before it hid) may still be in the pipe.
                let late = std::time::Duration::from_millis(50);
                asked.extend(std::iter::from_fn(|| r.lines.recv_timeout(late).ok()));
                gone.push(key.clone());
            }
        }
        for key in gone {
            self.close_window(&key);
        }
        for r in asked {
            match r {
                Request::Focus(id) => {
                    ui.request(WindowRequest::Focus);
                    if self.store.session(&id).is_some() {
                        self.st.view = crate::screen::MainView::Sessions;
                        self.st.selected = Some(id);
                    }
                }
                Request::Show => ui.request(WindowRequest::Focus),
                Request::Hide => {}
                Request::Open(key) => self.open_window(&key),
            }
        }
    }
}

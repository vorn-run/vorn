//! A window point in a window of its own (`vorn-app panel --point KEY`):
//! its template on its own connection to vornd, floating over other apps.
//! It tells the app that started it what to do with one line on stdout
//! per request: `focus <terminal>`, `show`, `hide`.

use std::sync::mpsc::Receiver;

use serde_json::{json, Value};

use crate::act::{Act, Acts};
use crate::client::rpc::{Event as RpcEvent, Rpc};
use crate::client::Endpoint;
use crate::points::{self, line, Holder, PointAct, Points, Press, Request};
use crate::runs::s;
use crate::ui::theme::color as k;
use crate::ui::{div, text, El, Event, Id, Rgba, Role, Sense, Ui, WindowRequest};

/// A window's size when its point names none.
pub const SIZE: (f32, f32) = (280.0, 400.0);

/// The panel's window: its connection, the point it draws and that point's rows.
pub struct Panel {
    rpc: Rpc,
    events: Receiver<RpcEvent>,
    key: String,
    pub points: Points,
    acts: Acts,
    /// Requests for the app, written by the caller.
    pub out: Vec<Request>,
}

impl Panel {
    /// Connects to vornd at `ep` and loads point `key` with its rows.
    pub fn connect(
        ep: &Endpoint,
        key: &str,
        wake: crate::client::rpc::Wake,
    ) -> Result<Panel, String> {
        let (rpc, events) = Rpc::connect(ep, wake).map_err(|e| e.to_string())?;
        rpc.call(
            "subscribe:set",
            json!({ "topics": ["extension:*"], "terminalBytes": false }),
        )
        .map_err(|e| e.to_string())?;
        let mut p = Panel {
            rpc,
            events,
            key: key.to_owned(),
            points: Points::default(),
            acts: Acts::default(),
            out: Vec::new(),
        };
        p.reload();
        if p.points.get(key).is_none() {
            return Err(format!("no window point {key:?}"));
        }
        Ok(p)
    }

    /// The point's size, as its manifest gives it.
    pub fn size(&self) -> (f32, f32) {
        self.points
            .get(&self.key)
            .and_then(|p| p.size)
            .unwrap_or(SIZE)
    }

    fn reload(&mut self) {
        let list = self
            .rpc
            .call("extension:contributions", Value::Null)
            .ok()
            .and_then(|v| v.as_array().cloned())
            .unwrap_or_default();
        self.points.listed(&list);
        for (ext, coll) in self.points.to_ask() {
            if let Ok(Value::Array(rows)) = self
                .rpc
                .call("extension:rows", json!({ "id": ext, "collection": coll }))
            {
                self.points.fill(&ext, &coll, &rows);
            }
        }
    }

    /// Folds in what vornd said since the last frame.
    pub fn pump(&mut self) {
        let mut stale = false;
        while let Ok(e) = self.events.try_recv() {
            let RpcEvent::Notification { method, params } = e else {
                continue;
            };
            match method.as_str() {
                "extension:rowsChanged" => {
                    if let (Some(ext), Some(coll)) =
                        (s(&params, "extensionId"), s(&params, "collection"))
                    {
                        let ops = params["ops"].as_array().cloned().unwrap_or_default();
                        self.points.changed(ext, coll, &ops);
                    }
                }
                "extension:contributionsChanged" => stale = true,
                _ => {}
            }
        }
        if stale {
            self.reload();
            if self.points.get(&self.key).is_none() {
                self.out.push(Request::Hide);
            }
        }
    }

    fn press(&mut self, extension: &str, press: &Press) {
        if let Some(params) = points::call_params(extension, press) {
            let _ = self.rpc.request("extension:call", params);
        }
        if let Some(id) = &press.focus {
            self.out.push(Request::Focus(id.clone()));
        } else if press.show {
            self.out.push(Request::Show);
        }
        if press.close {
            self.out.push(Request::Hide);
        }
    }

    pub fn click(&mut self, id: Id) {
        let Some(Act::Point(a)) = self.acts.clicked(id) else {
            return;
        };
        match a {
            PointAct::Press {
                extension, press, ..
            } => {
                self.points.asking = None;
                self.press(&extension, &press);
            }
            PointAct::Ask(id) => self.points.asking = id,
            _ => {}
        }
    }

    pub fn submit(&mut self, ui: &Ui, id: Id) {
        let Some(Act::Point(PointAct::Field {
            extension,
            call,
            row,
        })) = self.acts.text_of(id)
        else {
            return;
        };
        let mut args = json!({ "value": ui.edit_text(id) });
        if let Some(r) = row {
            args["id"] = json!(r);
        }
        let _ = self.rpc.request(
            "extension:call",
            json!({ "id": extension, "action": call, "args": args }),
        );
    }

    pub fn view(&mut self, ui: &mut Ui) -> El {
        self.acts.clear();
        let Some(p) = self.points.get(&self.key) else {
            return div();
        };
        let body = crate::screen::points::draw(
            ui,
            &self.acts,
            &self.points,
            p,
            None,
            Some(Holder::Window(self.key.clone())),
        )
        .unwrap_or_else(div);
        let hide = Id::new("panel-hide");
        self.acts.bind(
            hide,
            Act::Point(PointAct::Window {
                key: self.key.clone(),
                open: false,
            }),
        );
        let header = div()
            .row()
            .items_center()
            .justify_between()
            .px(12.0)
            .py(8.0)
            .border_bottom(1.0, Rgba::white(0.06))
            .id(Id::new("panel-drag"))
            .sense(Sense::WINDOW_DRAG)
            .child(
                text(p.title.as_str(), 13.0, k::BRONZO)
                    .weight(600)
                    .line_height(18.0)
                    .silent(),
            )
            .child(crate::screen::points::close_button(ui, hide, "Hide"));
        div().col().w_full().child(
            div()
                .col()
                .w_full()
                .rounded(4.0)
                .bg(k::SURFACE_OVERLAY)
                .border(1.0, Rgba::white(0.08))
                .role(Role::Dialog, p.title.as_str())
                .child(header)
                .child(
                    div()
                        .id(Id::new("panel-body"))
                        .col()
                        .px(12.0)
                        .py(8.0)
                        .max_h(self.size().1 - 40.0)
                        .scroll_y()
                        .child(body),
                ),
        )
    }
}

/// The panel as a window of its own.
pub struct PanelWindow {
    pub panel: Panel,
}

impl crate::ui::WindowApp for PanelWindow {
    fn view(&mut self, ui: &mut Ui) -> El {
        self.panel.pump();
        let el = self.panel.view(ui);
        flush(&mut self.panel, ui);
        el
    }

    fn event(&mut self, ui: &mut Ui, event: Event) {
        match event {
            Event::Click(id) if id == Id::new("panel-hide") => self.panel.out.push(Request::Hide),
            Event::Click(id) => self.panel.click(id),
            Event::Submit(id) => self.panel.submit(ui, id),
            _ => {}
        }
        flush(&mut self.panel, ui);
    }

    fn background(&self) -> Rgba {
        Rgba::TRANSPARENT
    }
}

/// Writes the panel's requests to stdout; hiding ends the process.
fn flush(p: &mut Panel, ui: &mut Ui) {
    use std::io::Write;
    let mut out = std::io::stdout().lock();
    for r in std::mem::take(&mut p.out) {
        let _ = writeln!(out, "{}", line(&r));
        if r == Request::Hide {
            let _ = out.flush();
            ui.request(WindowRequest::Close);
        }
    }
    let _ = out.flush();
}

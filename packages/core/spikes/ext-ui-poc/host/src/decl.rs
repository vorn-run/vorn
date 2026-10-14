//! Model A: the client renders the extension's template and keyed rows with vornui, keeping selection, filter and confirm state itself.

use std::collections::{BTreeSet, HashMap};

use serde_json::{Map, Value};
use vornui::theme::{color, radius, rgb, text as size};
use vornui::widgets::{self, Tone};
use vornui::{div, text, Action, El, Event, Id, Rgba, Sense, Ui};

/// What activating an element does.
#[derive(Clone)]
enum Act {
    Toggle(usize),
    Run(Value),
}

pub struct View {
    template: Value,
    pub rows: Vec<Value>,
    state: Map<String, Value>,
    selected: BTreeSet<String>,
    anchor: Option<usize>,
    pub status: String,
    acts: HashMap<Id, Act>,
    list: Option<(Id, String)>,
    /// Calls the template asks for, drained by the host.
    pub calls: Vec<(String, Value)>,
}

fn id_of(s: &str) -> Id {
    Id::new(s)
}

impl View {
    pub fn new(template: &str) -> View {
        let template: Value = serde_json::from_str(template).unwrap_or(Value::Null);
        let state = template["state"].as_object().cloned().unwrap_or_default();
        View {
            template,
            rows: Vec::new(),
            state,
            selected: BTreeSet::new(),
            anchor: None,
            status: String::new(),
            acts: HashMap::new(),
            list: None,
            calls: Vec::new(),
        }
    }

    /// Replaces the rows, keeping the selection of rows still there.
    pub fn set_rows(&mut self, rows: Vec<Value>) {
        let live: BTreeSet<String> = rows
            .iter()
            .filter_map(|r| r["id"].as_str().map(str::to_owned))
            .collect();
        self.selected.retain(|id| live.contains(id));
        self.rows = rows;
    }

    /// Applies one row op: an upsert by `id`.
    pub fn upsert(&mut self, row: Value) {
        match self.rows.iter_mut().find(|r| r["id"] == row["id"]) {
            Some(r) => *r = row,
            None => self.rows.push(row),
        }
    }

    fn fill(&self, s: &str, row: Option<&Value>) -> String {
        let mut out = String::new();
        let mut rest = s;
        while let Some(a) = rest.find('{') {
            out.push_str(&rest[..a]);
            let Some(b) = rest[a..].find('}') else { break };
            out.push_str(&self.lookup(&rest[a + 1..a + b], row));
            rest = &rest[a + b + 1..];
        }
        out.push_str(rest);
        out
    }

    fn lookup(&self, path: &str, row: Option<&Value>) -> String {
        match path.split_once('.') {
            Some(("row", k)) => row.map(|r| scalar(&r[k])).unwrap_or_default(),
            Some(("rows", "count")) => self.rows.len().to_string(),
            Some(("rows", k)) => self
                .rows
                .iter()
                .filter(|r| r[k] == Value::Bool(true))
                .count()
                .to_string(),
            Some(("selected", "count")) => self.selected.len().to_string(),
            None if path == "status" => self.status.clone(),
            _ => scalar(self.state.get(path).unwrap_or(&Value::Null)),
        }
    }

    fn truthy(&self, path: &str) -> bool {
        let v = self.lookup(path, None);
        !(v.is_empty() || v == "0" || v == "false")
    }

    pub fn view(&mut self, ui: &Ui) -> El {
        self.acts.clear();
        let root = self.template["view"].clone();
        self.node(ui, &root, None)
    }

    fn node(&mut self, ui: &Ui, n: &Value, row: Option<&Value>) -> El {
        let mut el = match n["c"].as_str().unwrap_or("") {
            "column" | "row" => {
                let mut d = div();
                d = if n["c"] == "row" {
                    d.row().items_center()
                } else {
                    d.col()
                };
                if let Some(g) = n["gap"].as_f64() {
                    d = d.gap(g as f32);
                }
                if let Some(p) = n["pad"].as_f64() {
                    d = d.p(p as f32).w_full().h_full().bg(color::SURFACE_BASE);
                }
                if n["justify"] == "between" {
                    d = d.justify_between();
                }
                let kids: Vec<El> = n["children"]
                    .as_array()
                    .into_iter()
                    .flatten()
                    .map(|c| self.node(ui, c, row))
                    .collect();
                d.children(kids)
            }
            "text" => {
                let v = self.fill(n["value"].as_str().unwrap_or(""), row);
                match n["role"].as_str().unwrap_or("body") {
                    "title" => text(v, size::SM.0, color::INK).weight(600),
                    "secondary" => text(v, size::XS.0, color::INK_SECONDARY),
                    "mono" => text(v, size::XS.0, color::GRAY_300).mono().truncate(),
                    _ => text(v, size::XS.0, color::INK).truncate(),
                }
            }
            "field" => {
                let id = id_of(n["id"].as_str().unwrap_or("field"));
                let ph = n["placeholder"].as_str().unwrap_or("");
                if let Some(k) = n["model"].as_str() {
                    self.state
                        .insert(k.to_owned(), Value::String(ui.edit_text(id).to_owned()));
                }
                widgets::text_input(ui, id, ph, ph)
            }
            "list" => self.list(ui, n),
            "check" => {
                let on = row
                    .and_then(|r| r["id"].as_str())
                    .is_some_and(|id| self.selected.contains(id));
                div()
                    .size(14.0, 14.0)
                    .rounded(radius::SM)
                    .border(1.0, Rgba::white(if on { 0.6 } else { 0.25 }))
                    .bg(if on { color::BRONZO } else { Rgba::TRANSPARENT })
                    .glyph(if on { "☑" } else { "☐" })
            }
            "badge" => {
                let v = self.fill(n["value"].as_str().unwrap_or(""), row);
                let (fg, bg) = match n["tone"][&v].as_str().unwrap_or("") {
                    "good" => (color::GREEN_400, Rgba::hexa(rgb::GREEN_500, 0.15)),
                    "warn" => (color::YELLOW_500, Rgba::hexa(rgb::YELLOW_500, 0.15)),
                    "bad" => (color::RED_400, Rgba::hexa(rgb::RED_500, 0.15)),
                    "info" => (color::STATUS_BLUE, Rgba::white(0.08)),
                    _ => (color::GRAY_400, Rgba::white(0.06)),
                };
                widgets::badge(&v, fg, bg)
            }
            "confirm" => {
                let name = n["id"].as_str().unwrap_or("confirm");
                let (id, ok, cancel) = (
                    id_of(name),
                    id_of(&format!("{name}.ok")),
                    id_of(&format!("{name}.cancel")),
                );
                let label = self.fill(n["label"].as_str().unwrap_or(""), None);
                let enabled = n["enabled"].as_str().is_none_or(|p| self.truthy(p));
                let mut b = widgets::button(ui, id, &label);
                if enabled {
                    self.acts.insert(id, Act::Run(n["on"]["open"].clone()));
                } else {
                    b = b.disabled(true).opacity(0.4);
                }
                if n["open"].as_str().is_some_and(|p| self.truthy(p)) {
                    let msg = self.fill(n["message"].as_str().unwrap_or(""), None);
                    self.acts.insert(ok, Act::Run(n["on"]["ok"].clone()));
                    self.acts
                        .insert(cancel, Act::Run(n["on"]["cancel"].clone()));
                    let tone = if n["danger"] == true {
                        Tone::Danger
                    } else {
                        Tone::Neutral
                    };
                    b = widgets::confirm(
                        ui,
                        b,
                        &msg,
                        (ok, n["ok"].as_str().unwrap_or("OK")),
                        cancel,
                        tone,
                    );
                }
                b
            }
            _ => div(),
        };
        if let Some(w) = n["w"].as_f64() {
            el = el.w(w as f32);
        }
        if n["grow"] == true {
            el = el.grow();
        }
        el
    }

    fn visible(&self, ui: &Ui, n: &Value) -> Vec<usize> {
        let q = n["match"]["state"]
            .as_str()
            .map(|k| self.lookup(k, None))
            .unwrap_or_default()
            .to_lowercase();
        let _ = ui;
        let fields: Vec<&str> = n["match"]["fields"]
            .as_array()
            .into_iter()
            .flatten()
            .filter_map(Value::as_str)
            .collect();
        (0..self.rows.len())
            .filter(|&i| {
                q.is_empty()
                    || fields
                        .iter()
                        .any(|f| scalar(&self.rows[i][*f]).to_lowercase().contains(&q))
            })
            .collect()
    }

    fn list(&mut self, ui: &Ui, n: &Value) -> El {
        let name = n["id"].as_str().unwrap_or("list").to_owned();
        let lid = id_of(&name);
        self.list = Some((lid, n["select"]["when"].as_str().unwrap_or("").to_owned()));
        let item = n["item"].clone();
        let mut out = Vec::new();
        for i in self.visible(ui, n) {
            let row = self.rows[i].clone();
            let id = lid.child(i);
            let on = row["id"]
                .as_str()
                .is_some_and(|r| self.selected.contains(r));
            let ok = self.selectable(&row);
            self.acts.insert(id, Act::Toggle(i));
            let wash = if on {
                Rgba::white(0.06)
            } else {
                Rgba::TRANSPARENT.mix(Rgba::white(0.04), ui.hover(id))
            };
            let body = self.node(ui, &item, Some(&row));
            out.push(
                body.id(id)
                    .px(8.0)
                    .py(6.0)
                    .rounded(radius::MD)
                    .bg(wash)
                    .on(Action::Click)
                    .sense(Sense::MOVE)
                    .opacity(if ok { 1.0 } else { 0.55 }),
            );
        }
        widgets::list(lid, &name, out).grow().min_h(0.0)
    }

    fn selectable(&self, row: &Value) -> bool {
        match &self.list {
            Some((_, when)) if !when.is_empty() => row[when.as_str()] == Value::Bool(true),
            _ => true,
        }
    }

    fn run(&mut self, a: &Value) {
        if let Some(set) = a["set"].as_object() {
            for (k, v) in set {
                self.state.insert(k.clone(), v.clone());
            }
        }
        if let Some(m) = a["call"].as_str() {
            let args = match a["args"].as_str() {
                Some("selected") => Value::from(self.selected.iter().cloned().collect::<Vec<_>>()),
                _ => a["args"].clone(),
            };
            self.calls.push((m.to_owned(), args));
            self.selected.clear();
        }
    }

    pub fn event(&mut self, ui: &Ui, e: Event) {
        match e {
            Event::Click(id) => match self.acts.get(&id).cloned() {
                Some(Act::Toggle(i)) => {
                    let row = self.rows[i].clone();
                    if self.selectable(&row) {
                        let rid = scalar(&row["id"]);
                        if !self.selected.remove(&rid) {
                            self.selected.insert(rid);
                        }
                        self.anchor = Some(i);
                    }
                }
                Some(Act::Run(a)) => self.run(&a),
                None => {}
            },
            Event::Move { y, .. } => {
                let Some((lid, _)) = self.list else { return };
                let Some(a) = self.anchor else { return };
                let Some(b) = (0..self.rows.len()).find(|&i| {
                    ui.rect_of(lid.child(i))
                        .is_some_and(|r| y >= r.y && y < r.y + r.h)
                }) else {
                    return;
                };
                for i in a.min(b)..=a.max(b) {
                    let row = self.rows[i].clone();
                    if self.selectable(&row) {
                        self.selected.insert(scalar(&row["id"]));
                    }
                }
            }
            _ => {}
        }
    }
}

fn scalar(v: &Value) -> String {
    match v {
        Value::String(s) => s.clone(),
        Value::Null => String::new(),
        other => other.to_string(),
    }
}

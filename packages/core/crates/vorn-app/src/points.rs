//! What extensions contribute to the screens (`extension:contributions`):
//! each point with its checked template, the rows its templates draw, and
//! which dialog and windows are open. The pure half; the RPC glue is
//! `app::points` and the drawing `screen::points`.

use std::collections::{BTreeMap, BTreeSet, HashMap};

use serde_json::{json, Value};
use vorn_view::{Options, Row, Template};

pub use vorn_components::template::Press;

use crate::runs::s;

/// Where contributions show, as `extension:contributions` names them.
pub mod kind {
    pub const SIDEBAR: &str = "sidebar";
    pub const BADGES: &str = "badges";
    pub const CARD_FOOTERS: &str = "cardFooters";
    pub const PANES: &str = "panes";
    pub const COMMANDS: &str = "commands";
    pub const SETTINGS: &str = "settings";
    pub const DECORATIONS: &str = "decorations";
    pub const COMPOSER: &str = "composer";
    pub const LAUNCHERS: &str = "launchers";
    pub const WINDOWS: &str = "windows";
    pub const MENUS: &str = "menus";
    pub const DIALOGS: &str = "dialogs";
    pub const VIEWS: &str = "views";
}

/// One contribution and its template, or why the template does not check.
#[derive(Debug, Clone)]
pub struct Point {
    pub extension: String,
    pub extension_name: String,
    pub builtin: bool,
    pub kind: String,
    pub id: String,
    pub title: String,
    pub description: Option<String>,
    pub icon: Option<String>,
    pub key: Option<String>,
    pub call: Option<String>,
    pub menu: Option<String>,
    pub window: Option<String>,
    pub size: Option<(f32, f32)>,
    pub raise: Option<String>,
    /// `Ok(None)` for a point with no template (a command, a menu item).
    pub template: Result<Option<Template>, Vec<String>>,
}

impl Point {
    pub fn from_value(v: &Value) -> Option<Point> {
        let kind = s(v, "kind")?.to_owned();
        let errors: Vec<String> = v
            .get("errors")
            .and_then(Value::as_array)
            .map(|a| {
                a.iter()
                    .filter_map(Value::as_str)
                    .map(str::to_owned)
                    .collect()
            })
            .unwrap_or_default();
        let per_session = matches!(
            kind.as_str(),
            kind::BADGES | kind::CARD_FOOTERS | kind::DECORATIONS
        );
        let template = if !errors.is_empty() {
            Err(errors)
        } else {
            match v.get("view").filter(|t| !t.is_null()) {
                None => Ok(None),
                Some(t) => vorn_view::parse_value(
                    t,
                    Options {
                        session_row: per_session,
                    },
                )
                .map(Some)
                .map_err(|e| e.iter().map(ToString::to_string).collect()),
            }
        };
        let size = v
            .get("size")
            .and_then(Value::as_array)
            .and_then(|a| Some((a.first()?.as_f64()? as f32, a.get(1)?.as_f64()? as f32)));
        let text = |k: &str| s(v, k).map(str::to_owned);
        let extension = s(v, "extension")?.to_owned();
        Some(Point {
            extension_name: text("extensionName").unwrap_or_else(|| extension.clone()),
            extension,
            builtin: v.get("builtin").and_then(Value::as_bool) == Some(true),
            kind,
            id: s(v, "id")?.to_owned(),
            title: text("title").unwrap_or_default(),
            description: text("description"),
            icon: text("icon"),
            key: text("key"),
            call: text("call"),
            menu: text("menu"),
            window: text("window"),
            size,
            raise: text("raise"),
            template,
        })
    }

    /// `extension/kind/id`: the point's name in ids and error boxes.
    pub fn key(&self) -> String {
        format!("{}/{}/{}", self.extension, self.kind, self.id)
    }

    /// What its error box calls it: `Checks › Sidebar › checks`.
    pub fn place(&self) -> String {
        let title = if self.title.is_empty() {
            &self.id
        } else {
            &self.title
        };
        format!("{} › {}", self.extension_name, title)
    }

    /// The collections its template draws.
    pub fn collections(&self) -> Vec<String> {
        match &self.template {
            Ok(Some(t)) => t.collections.iter().map(|(n, _)| n.clone()).collect(),
            _ => Vec::new(),
        }
    }
}

/// A row operation as vornd tells it: `{op: upsert, row}`, `{op: delete, id}`, `{op: clear}`.
pub fn apply(rows: &mut Vec<Row>, op: &Value) {
    match s(op, "op") {
        Some("upsert") => {
            let Some(row) = op.get("row").and_then(Row::from_value) else {
                return;
            };
            match rows.iter_mut().find(|r| r.id == row.id) {
                Some(kept) => *kept = row,
                None => rows.push(row),
            }
        }
        Some("delete") => {
            let id = s(op, "id");
            rows.retain(|r| Some(r.id.as_str()) != id);
        }
        Some("clear") => rows.clear(),
        _ => {}
    }
}

/// What the screens' extension controls do.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub enum PointAct {
    /// A template's button or toggle; `holder` is the dialog or window it sits in.
    Press {
        extension: String,
        press: Press,
        holder: Option<Holder>,
    },
    /// Asks before button `id` runs, or stops asking.
    Ask(Option<u64>),
    /// Opens a template's more menu `id`, or closes it.
    More(Option<u64>),
    /// A one-line field's Return calls `call` with what it holds.
    Field {
        extension: String,
        call: String,
        row: Option<String>,
    },
    /// Shows the settings page of point `key`.
    Settings(String),
    /// Chooses a command, menu item or launcher; `args` is the menu's context.
    Choose {
        key: String,
        args: String,
    },
    /// Opens or closes a window point.
    Window {
        key: String,
        open: bool,
    },
    CloseDialog,
    /// Switches a built-in extension on or off.
    Builtin {
        id: String,
        on: bool,
    },
}

/// The dialog or window a template is drawn in.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub enum Holder {
    Dialog,
    Window(String),
}

/// Every contribution, the rows its templates draw, and what is open.
#[derive(Debug, Default)]
pub struct Points {
    pub all: Vec<Point>,
    /// Rows by extension, then collection.
    pub rows: HashMap<String, BTreeMap<String, Vec<Row>>>,
    pub loaded: bool,
    /// Something changed since the list was fetched.
    pub stale: bool,
    /// Collections asked for since the list was fetched.
    pub asked: BTreeSet<(String, String)>,
    /// Collections asked for and not yet answered, and whether a change came meanwhile.
    pub pending: BTreeMap<(String, String), bool>,
    /// The dialog showing, by point key.
    pub dialog: Option<String>,
    /// Windows open, by point key.
    pub windows: BTreeSet<String>,
    /// The button asking before it runs.
    pub asking: Option<u64>,
    /// The template more menu open.
    pub more: Option<u64>,
    /// Built-in extensions: `{id, name, description, on}`.
    pub builtins: Vec<Value>,
    pub builtins_loaded: bool,
    /// The built-in whose switch vornd has not answered yet.
    pub switching: Option<String>,
    /// The built-in that just went off, so its row says what that hid.
    pub off_note: Option<String>,
    /// A built-in whose switch was refused, and why.
    pub switch_error: Option<(String, String)>,
    /// Settings pages and panes whose `call` ran since they showed.
    pub shown: BTreeSet<String>,
    /// Window points draw inside the app, which cannot open windows of their own.
    pub floating: bool,
}

static NONE: BTreeMap<String, Vec<Row>> = BTreeMap::new();

impl Points {
    /// The contributions placed at `kind`, in order.
    pub fn at<'a>(&'a self, kind: &'a str) -> impl Iterator<Item = &'a Point> + 'a {
        self.all.iter().filter(move |p| p.kind == kind)
    }

    /// Shows built-in `id` as on or off, ahead of vornd's answer.
    pub fn set_builtin(&mut self, id: &str, on: bool) {
        for b in &mut self.builtins {
            if b.get("id").and_then(Value::as_str) == Some(id) {
                b["on"] = Value::Bool(on);
            }
        }
    }

    pub fn get(&self, key: &str) -> Option<&Point> {
        self.all.iter().find(|p| p.key() == key)
    }

    /// The point `id` of `kind` that `extension` contributes.
    pub fn find(&self, extension: &str, kind: &str, id: &str) -> Option<&Point> {
        self.all
            .iter()
            .find(|p| p.extension == extension && p.kind == kind && p.id == id)
    }

    /// An extension's rows, as its templates read them.
    pub fn rows_of(&self, extension: &str) -> &BTreeMap<String, Vec<Row>> {
        self.rows.get(extension).unwrap_or(&NONE)
    }

    /// A fresh list: rows of collections no template draws any more go.
    pub fn listed(&mut self, list: &[Value]) {
        self.all = list.iter().filter_map(Point::from_value).collect();
        self.loaded = true;
        self.asked.clear();
        let all = &self.all;
        self.rows
            .retain(|ext, _| all.iter().any(|p| &p.extension == ext));
        let keys: BTreeSet<String> = self.all.iter().map(Point::key).collect();
        self.windows.retain(|k| keys.contains(k));
        if self.dialog.as_ref().is_some_and(|d| !keys.contains(d)) {
            self.dialog = None;
        }
    }

    /// The `(extension, collection)` pairs no one has asked for yet, now marked asked.
    pub fn to_ask(&mut self) -> Vec<(String, String)> {
        let mut out = Vec::new();
        for p in &self.all {
            for c in p.collections() {
                let k = (p.extension.clone(), c);
                if !self.asked.contains(&k) {
                    self.asked.insert(k.clone());
                    self.pending.insert(k.clone(), false);
                    out.push(k);
                }
            }
        }
        out
    }

    /// `extension:rows`' answer, `None` if it failed; asked again if a change came meanwhile.
    pub fn fill(&mut self, extension: &str, collection: &str, rows: Option<&[Value]>) {
        let k = (extension.to_owned(), collection.to_owned());
        if self.pending.remove(&k) == Some(true) {
            self.asked.remove(&k);
        }
        let Some(rows) = rows else {
            return;
        };
        let kept = rows.iter().filter_map(Row::from_value).collect();
        self.rows
            .entry(extension.to_owned())
            .or_default()
            .insert(collection.to_owned(), kept);
    }

    /// `extension:rowsChanged`: answers the window points whose raise collection gained a row.
    pub fn changed(&mut self, extension: &str, collection: &str, ops: &[Value]) -> Vec<String> {
        if let Some(late) = self
            .pending
            .get_mut(&(extension.to_owned(), collection.to_owned()))
        {
            *late = true;
        }
        let rows = self
            .rows
            .entry(extension.to_owned())
            .or_default()
            .entry(collection.to_owned())
            .or_default();
        let before: BTreeSet<String> = rows.iter().map(|r| r.id.clone()).collect();
        for op in ops {
            apply(rows, op);
        }
        let fresh = rows.iter().any(|r| !before.contains(&r.id));
        if !fresh {
            return Vec::new();
        }
        self.at(kind::WINDOWS)
            .filter(|p| p.extension == extension && p.raise.as_deref() == Some(collection))
            .map(Point::key)
            .collect()
    }

    /// The row of `session` in the first collection of `p`'s template, for a per-session place.
    pub fn session_row<'a>(&'a self, p: &Point, session: &str) -> Option<&'a Row> {
        let Ok(Some(t)) = &p.template else {
            return None;
        };
        self.rows_of(&p.extension)
            .get(t.first()?)?
            .iter()
            .find(|r| r.id == session)
    }
}

/// Whether key `spec` (`Mod+Shift+W`) is the key pressed: `code` with modifiers `m`.
pub fn key_hit(spec: &str, code: &str, m: u16, primary: u16) -> bool {
    use crate::ui::mods;
    let mut want = 0;
    let mut parts: Vec<&str> = spec.split('+').collect();
    let Some(key) = parts.pop() else {
        return false;
    };
    for p in parts {
        want |= match p {
            "Mod" => primary,
            "Ctrl" => mods::CTRL,
            "Alt" => mods::ALT,
            "Shift" => mods::SHIFT,
            _ => return false,
        };
    }
    let held = m & (mods::CTRL | mods::SUPER | mods::ALT | mods::SHIFT);
    let named = match key.chars().next() {
        Some(c) if key.len() == 1 && c.is_ascii_alphabetic() => {
            format!("Key{}", c.to_ascii_uppercase())
        }
        Some(c) if key.len() == 1 && c.is_ascii_digit() => format!("Digit{c}"),
        _ => key.to_owned(),
    };
    held == want && named == code
}

/// What a press asks of vornd: `extension:call`'s params, when it calls.
pub fn call_params(extension: &str, press: &Press) -> Option<Value> {
    let action = press.call.as_ref()?;
    let mut args = json!({});
    if let Some(id) = &press.row {
        args["id"] = json!(id);
    }
    if let Some(v) = press.value {
        args["value"] = json!(v);
    }
    Some(json!({ "id": extension, "action": action, "args": args }))
}

/// A choice's call: a command, menu item or launcher with its context.
pub fn choose_params(p: &Point, args: &str) -> Option<Value> {
    let action = p.call.as_ref()?;
    let args: Value = serde_json::from_str(args).unwrap_or_else(|_| json!({}));
    Some(json!({ "id": p.extension, "action": action, "args": args }))
}

/// What an extension's answer asks the client to do: a dialog to open and a toast.
pub fn answered(answer: &Value) -> (Option<String>, Option<String>) {
    (
        s(answer, "open").map(str::to_owned),
        s(answer, "toast").map(str::to_owned),
    )
}

/// What a window point's process tells the app, one line on stdout.
pub fn line(request: &Request) -> String {
    match request {
        Request::Focus(id) => format!("focus {id}"),
        Request::Show => "show".to_owned(),
        Request::Hide => "hide".to_owned(),
        Request::Open(key) => format!("open {key}"),
    }
}

/// A line from a window point's process, as the app reads it.
pub fn parse(line: &str) -> Option<Request> {
    match line.trim().split_once(' ') {
        Some(("focus", id)) if !id.is_empty() => Some(Request::Focus(id.to_owned())),
        Some(("open", key)) if !key.is_empty() => Some(Request::Open(key.to_owned())),
        None if line.trim() == "show" => Some(Request::Show),
        None if line.trim() == "hide" => Some(Request::Hide),
        _ => None,
    }
}

/// What a window point's process asks of the app.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Request {
    /// Bring the app forward on this terminal.
    Focus(String),
    Show,
    Hide,
    /// Open another window point, by key.
    Open(String),
}

#[cfg(test)]
mod tests {
    use super::*;

    fn point(kind: &str, view: Value) -> Value {
        json!({ "extension": "x", "extensionName": "X", "kind": kind, "id": "p",
                "title": "P", "view": view, "errors": [] })
    }

    fn template() -> Value {
        json!({ "schema": "vorn:view@1",
                "rows": { "items": { "name": "string" } },
                "view": { "list": { "rows": "items", "item": { "text": { "value": "{row.name}" } } } } })
    }

    #[test]
    fn rows_follow_their_ops_in_place_and_raise_windows_on_new_ones() {
        let mut pts = Points::default();
        let mut w = point(kind::WINDOWS, template());
        w["raise"] = json!("items");
        pts.listed(&[point(kind::SIDEBAR, template()), w]);
        assert!(pts.all[0].template.as_ref().is_ok_and(Option::is_some));
        assert_eq!(pts.to_ask(), [("x".to_owned(), "items".to_owned())]);
        assert!(pts.to_ask().is_empty());
        pts.fill("x", "items", Some(&[json!({ "id": "a", "name": "A" })]));
        let raised = pts.changed(
            "x",
            "items",
            &[
                json!({ "op": "upsert", "row": { "id": "b", "name": "B" } }),
                json!({ "op": "upsert", "row": { "id": "a", "name": "A2" } }),
            ],
        );
        assert_eq!(raised, ["x/windows/p"]);
        let rows = &pts.rows_of("x")["items"];
        assert_eq!(
            rows.iter().map(|r| r.text("name")).collect::<Vec<_>>(),
            ["A2", "B"]
        );
        assert!(pts
            .changed("x", "items", &[json!({ "op": "delete", "id": "a" })])
            .is_empty());
        pts.changed("x", "items", &[json!({ "op": "clear" })]);
        assert!(pts.rows_of("x")["items"].is_empty());
    }

    #[test]
    fn an_answer_older_than_a_change_is_asked_again() {
        let mut pts = Points::default();
        pts.listed(&[point(kind::SIDEBAR, template())]);
        assert_eq!(pts.to_ask().len(), 1);
        let row = json!({ "op": "upsert", "row": { "id": "a", "name": "A" } });
        pts.changed("x", "items", &[row]);
        pts.fill("x", "items", Some(&[]));
        assert_eq!(pts.to_ask(), [("x".to_owned(), "items".to_owned())]);
        pts.fill("x", "items", Some(&[json!({ "id": "a", "name": "A" })]));
        assert!(pts.to_ask().is_empty());
        assert_eq!(pts.rows_of("x")["items"].len(), 1);
    }

    #[test]
    fn a_template_with_errors_keeps_them_to_show() {
        let mut v = point(kind::SIDEBAR, Value::Null);
        v["errors"] = json!(["views/a.kdl:2:1: unknown node 'buton'; did you mean 'button'?"]);
        let p = Point::from_value(&v).expect("a point");
        assert_eq!(p.template.as_ref().unwrap_err().len(), 1);
        assert_eq!(p.place(), "X › P");
    }

    #[test]
    fn keys_read_as_their_codes() {
        use crate::ui::mods;
        let primary = mods::SUPER;
        assert!(key_hit(
            "Mod+Alt+W",
            "KeyW",
            mods::SUPER | mods::ALT,
            primary
        ));
        assert!(!key_hit("Mod+Alt+W", "KeyW", mods::SUPER, primary));
        assert!(key_hit("Ctrl+1", "Digit1", mods::CTRL, primary));
        assert!(key_hit("Alt+F2", "F2", mods::ALT, primary));
    }

    #[test]
    fn presses_call_with_the_row_and_the_toggles_value() {
        let press = Press {
            call: Some("keep".into()),
            row: Some("r1".into()),
            value: Some(true),
            ..Press::default()
        };
        assert_eq!(
            call_params("x", &press),
            Some(json!({ "id": "x", "action": "keep", "args": { "id": "r1", "value": true } }))
        );
        assert_eq!(call_params("x", &Press::default()), None);
        assert_eq!(
            answered(&json!({ "open": "confirm", "toast": "Done" })),
            (Some("confirm".into()), Some("Done".into()))
        );
    }

    #[test]
    fn panel_lines_round_trip() {
        for r in [
            Request::Focus("t 1".into()),
            Request::Show,
            Request::Hide,
            Request::Open("x/windows/small".into()),
        ] {
            assert_eq!(parse(&line(&r)), Some(r));
        }
        assert_eq!(parse("nonsense"), None);
    }
}

//! What extensions contribute to the screens (`extension:contributions`):
//! each point, the trees they draw, the rows menus and attention list, and
//! which dialog and windows are open. The pure half; the RPC glue is
//! `app::points` and the drawing `screen::points`.

use std::collections::{BTreeMap, BTreeSet, HashMap};

use serde_json::{json, Map, Value};

use crate::runs::s;

/// A row an extension published: its `id` and its other fields.
#[derive(Debug, Clone, PartialEq)]
pub struct Row {
    pub id: String,
    pub fields: Map<String, Value>,
}

impl Row {
    /// A published row, if it has a string `id`.
    pub fn from_value(v: &Value) -> Option<Row> {
        let o = v.as_object()?;
        let id = o.get("id")?.as_str()?.to_owned();
        let fields = o
            .iter()
            .filter(|(k, _)| *k != "id")
            .map(|(k, v)| (k.clone(), v.clone()))
            .collect();
        Some(Row { id, fields })
    }

    /// A string field as it is held; `None` for any other.
    pub fn str(&self, field: &str) -> Option<&str> {
        if field == "id" {
            return Some(&self.id);
        }
        self.fields.get(field).and_then(Value::as_str)
    }

    /// A field as text; an absent or null field is empty.
    pub fn text(&self, field: &str) -> String {
        if field == "id" {
            return self.id.clone();
        }
        match self.fields.get(field) {
            Some(Value::String(s)) => s.clone(),
            None | Some(Value::Null) => String::new(),
            Some(other) => other.to_string(),
        }
    }
}

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
    pub const SHELL: &str = "shell";
    pub const CARDS: &str = "cards";
    pub const ATTENTION: &str = "attention";
}

/// Between a point's key and the scope a tree of it draws for: `ext/kind/id#scope`.
const SCOPE: char = '#';

/// The key of point `key`'s tree drawn for `scope`.
pub fn scoped(key: &str, scope: &str) -> String {
    format!("{key}{SCOPE}{scope}")
}

/// A tree's key as its point's key and the scope it draws for, if any.
pub fn unscope(key: &str) -> (&str, Option<&str>) {
    match key.split_once(SCOPE) {
        Some((k, s)) => (k, Some(s)),
        None => (key, None),
    }
}

/// One contribution: where it shows, and what choosing it does.
#[derive(Debug, Clone)]
pub struct Point {
    pub extension: String,
    pub extension_name: String,
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
    /// A menu item's submenu, one entry per row; attention's items.
    pub rows: Option<String>,
    /// Attention: the method a deny calls.
    pub deny: Option<String>,
    /// Drawn from the element tree its extension answers `ui.render` with.
    pub tree: bool,
    /// A built-in's settings page: the app's own page it follows in the
    /// sidebar, as `agents`; `None` lists it under Extensions.
    pub after: Option<String>,
}

impl Point {
    pub fn from_value(v: &Value) -> Option<Point> {
        let kind = s(v, "kind")?.to_owned();
        let size = v
            .get("size")
            .and_then(Value::as_array)
            .and_then(|a| Some((a.first()?.as_f64()? as f32, a.get(1)?.as_f64()? as f32)));
        let text = |k: &str| s(v, k).map(str::to_owned);
        let extension = s(v, "extension")?.to_owned();
        Some(Point {
            extension_name: text("extensionName").unwrap_or_else(|| extension.clone()),
            extension,
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
            rows: text("rows"),
            deny: text("deny"),
            tree: v.get("tree").and_then(Value::as_bool) == Some(true),
            after: text("after"),
        })
    }

    /// Drawn once per session (a card's badge or footer, a sidebar row's marks).
    pub fn per_session(&self) -> bool {
        matches!(
            self.kind.as_str(),
            kind::BADGES | kind::CARD_FOOTERS | kind::DECORATIONS
        )
    }

    /// The app settings page a settings page follows in the sidebar; without
    /// one it stays under Extensions.
    pub fn placed_after(&self) -> Option<crate::settings::Category> {
        self.after
            .as_deref()
            .filter(|_| self.kind == kind::SETTINGS)
            .and_then(crate::settings::Category::from_id)
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

    /// The rows a menu item or attention lists.
    pub fn collections(&self) -> Vec<String> {
        self.rows.iter().cloned().collect()
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

/// Sets preference `key` of the extension `id` in `list` (built-ins or packs) to `value`.
pub fn set_preference(list: &mut [Value], id: &str, key: &str, value: &Value) {
    let prefs = list
        .iter_mut()
        .filter(|e| e.get("id").and_then(Value::as_str) == Some(id))
        .filter_map(|e| e.get_mut("preferences").and_then(Value::as_array_mut))
        .flatten();
    for p in prefs.filter(|p| p.get("id").and_then(Value::as_str) == Some(key)) {
        p["value"] = value.clone();
    }
}

/// What the screens' extension controls do.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub enum PointAct {
    /// Asks before button `id` runs, or stops asking.
    Ask(Option<u64>),
    /// Opens a more menu `id`, or closes it.
    More(Option<u64>),
    /// Shows the settings page of point `key`.
    Settings(String),
    /// Chooses a command, menu item or launcher; `args` is the menu's context.
    Choose {
        key: String,
        args: String,
    },
    /// Answers attention item `row` of point `key`: its `call` when
    /// `allow`, else its `deny`.
    Answer {
        key: String,
        row: String,
        allow: bool,
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
    /// Sets extension `id`'s preference `key` to `value`, written as JSON.
    Preference {
        id: String,
        key: String,
        value: String,
    },
    /// Tells tree point `key`'s extension `event` happened.
    Tree {
        key: String,
        event: vorn_tree::Event,
    },
    /// A tree field's Return sends what it holds; a `blur` one's sends it
    /// as it loses focus instead, Return only leaving it.
    TreeField {
        key: String,
        node: String,
        blur: bool,
    },
    /// A tree button asks for a file, titled `title`: the OS dialog's
    /// answer is told to `node` as `submit` with the path.
    TreePick {
        key: String,
        node: String,
        title: String,
    },
    /// A live tree field changed: what it holds is told as `change`.
    TreeLive {
        key: String,
        node: String,
    },
    /// Checks or unchecks `rows` of list `list` in tree point `key`.
    TreeCheck {
        key: String,
        list: String,
        rows: Vec<String>,
        on: bool,
    },
}

/// Every contribution, the trees and rows they draw, and what is open.
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
    /// The more menu open.
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
    /// `workflow:nodeTypes`' node types, read with the list.
    pub node_types: Vec<Value>,
    /// Tree points' last `ui.render`, by point key, or why it cannot be drawn.
    pub trees: HashMap<String, Result<vorn_tree::Node, Vec<String>>>,
    /// Per-session tree points' children by session id, as indices into their tree, so a card finds its own at once.
    pub sessions: HashMap<String, HashMap<String, usize>>,
    /// Tree points to render again, as their extension's rows changed.
    pub stale_trees: BTreeSet<String>,
    /// Rows checked in a tree's lists, by point key and list id.
    pub checked: HashMap<(String, String), BTreeSet<String>>,
    /// How wide each scoped tree last drew, by its key, which its renders and events say.
    pub widths: HashMap<String, f32>,
    /// Collections the client reads although no point lists them, as `(extension, collection)`.
    pub wanted: BTreeSet<(String, String)>,
    /// How many times each tree was drawn afresh, so what is cut from one is cut once.
    pub gens: HashMap<String, u64>,
}

static NONE: BTreeMap<String, Vec<Row>> = BTreeMap::new();

impl Points {
    /// The contributions placed at `kind`, in order.
    pub fn at<'a>(&'a self, kind: &'a str) -> impl Iterator<Item = &'a Point> + 'a {
        self.all.iter().filter(move |p| p.kind == kind)
    }

    /// Shows built-in `id` as on or off, ahead of vornd's answer.
    /// Shows extension `id`'s preference `key` as `value` on the built-ins' pages.
    pub fn set_preference(&mut self, id: &str, key: &str, value: &Value) {
        set_preference(&mut self.builtins, id, key, value);
    }

    pub fn set_builtin(&mut self, id: &str, on: bool) {
        for b in &mut self.builtins {
            if b.get("id").and_then(Value::as_str) == Some(id) {
                b["on"] = Value::Bool(on);
            }
        }
    }

    /// The point of tree `key`, scoped or not.
    pub fn get(&self, key: &str) -> Option<&Point> {
        let key = unscope(key).0;
        self.all.iter().find(|p| p.key() == key)
    }

    /// The tree drawn for `key`, when it drew one.
    pub fn tree(&self, key: &str) -> Option<&vorn_tree::Node> {
        self.trees.get(key)?.as_ref().ok()
    }

    /// The point `id` of `kind` that `extension` contributes.
    pub fn find(&self, extension: &str, kind: &str, id: &str) -> Option<&Point> {
        self.all
            .iter()
            .find(|p| p.extension == extension && p.kind == kind && p.id == id)
    }

    /// An extension's rows.
    pub fn rows_of(&self, extension: &str) -> &BTreeMap<String, Vec<Row>> {
        self.rows.get(extension).unwrap_or(&NONE)
    }

    /// The rows a point lists (a menu item's entries, attention's items),
    /// those of `workspace` or of none.
    pub fn listed_rows<'a>(&'a self, p: &Point, workspace: &str) -> Vec<&'a Row> {
        let Some(collection) = &p.rows else {
            return Vec::new();
        };
        self.rows_of(&p.extension)
            .get(collection)
            .into_iter()
            .flatten()
            .filter(|r| r.str("workspace").is_none_or(|w| w == workspace))
            .collect()
    }

    /// Everything waiting for a person, oldest first: each attention
    /// point's rows, with the point they belong to.
    pub fn attention<'a>(&'a self, workspace: &str) -> Vec<(&'a Point, &'a Row)> {
        let mut out: Vec<(&Point, &Row)> = self
            .at(kind::ATTENTION)
            .flat_map(|p| {
                self.listed_rows(p, workspace)
                    .into_iter()
                    .map(move |r| (p, r))
            })
            .collect();
        out.sort_by(|a, b| {
            a.1.text("at")
                .cmp(&b.1.text("at"))
                .then_with(|| a.1.id.cmp(&b.1.id))
        });
        out
    }

    /// Whether row `row` of list `list` in tree point `key` is checked.
    pub fn is_checked(&self, key: &str, list: &str, row: &str) -> bool {
        self.checked
            .get(&(key.to_owned(), list.to_owned()))
            .is_some_and(|rows| rows.contains(row))
    }

    /// Checks `rows` of list `list` in tree point `key`, or unchecks them.
    pub fn check(&mut self, key: &str, list: &str, rows: &[String], on: bool) {
        let set = self
            .checked
            .entry((key.to_owned(), list.to_owned()))
            .or_default();
        for r in rows {
            if on {
                set.insert(r.clone());
            } else {
                set.remove(r);
            }
        }
    }

    /// `ui.render`'s answer for tree point `key`; checks on rows it no longer lists go.
    pub fn rendered(&mut self, key: &str, answer: Result<&Value, String>) {
        let tree = match answer {
            Ok(v) => vorn_tree::read(v).map(|r| r.tree),
            Err(e) => Err(vec![e]),
        };
        if let Ok(t) = &tree {
            let ids = node_ids(t);
            for ((k, _), rows) in &mut self.checked {
                if k == key {
                    rows.retain(|r| ids.contains(r.as_str()));
                }
            }
        }
        self.index(key, tree.as_ref().ok());
        self.trees.insert(key.to_owned(), tree);
        *self.gens.entry(key.to_owned()).or_default() += 1;
    }

    /// How many times tree `key` was drawn afresh.
    pub fn generation(&self, key: &str) -> u64 {
        self.gens.get(key).copied().unwrap_or(0)
    }

    /// Indexes a per-session tree's children by their session's id.
    fn index(&mut self, key: &str, tree: Option<&vorn_tree::Node>) {
        let per_session = self.get(key).is_some_and(Point::per_session);
        match tree.filter(|_| per_session) {
            Some(t) => {
                let by = t
                    .children
                    .iter()
                    .enumerate()
                    .filter_map(|(i, c)| Some((c.id.clone()?, i)))
                    .collect();
                self.sessions.insert(key.to_owned(), by);
            }
            None => {
                self.sessions.remove(key);
            }
        }
    }

    /// The part of per-session tree point `key` that `session` draws, if it has one.
    pub fn session_tree(&self, key: &str, session: &str) -> Option<&vorn_tree::Node> {
        let i = *self.sessions.get(key)?.get(session)?;
        self.trees.get(key)?.as_ref().ok()?.children.get(i)
    }

    /// What telling tree point `key`'s extension `event` asks of vornd; a bulk button's checks go with it.
    pub fn tell(&mut self, key: &str, event: &vorn_tree::Event) -> Option<Value> {
        let mut event = event.clone();
        if let Some(scope) = unscope(key).1 {
            event.scope = Some(scope.to_owned());
            event.width = self.widths.get(key).map(|w| w.round() as u32);
        }
        let params = tree_event_params(self.get(key)?, &event);
        if !event.selected.is_empty() {
            self.checked.retain(|(k, _), _| k != key);
        }
        Some(params)
    }

    /// The answer to telling tree point `key`: its patch applied, or the tree marked to render
    /// again; what else it asks of the client is answered for the caller to do.
    pub fn told(&mut self, key: &str, answer: Option<&Value>) -> vorn_tree::Answer {
        let answer = answer.map(vorn_tree::Answer::read).unwrap_or_default();
        if answer.same {
            return answer;
        }
        let patched = !answer.patch.is_empty()
            && match self.trees.get_mut(key) {
                Some(Ok(tree)) => vorn_tree::patch(tree, &answer.patch).is_ok(),
                _ => false,
            };
        if patched {
            let tree = self.trees.get(key).and_then(|t| t.as_ref().ok()).cloned();
            self.index(key, tree.as_ref());
            *self.gens.entry(key.to_owned()).or_default() += 1;
        } else {
            self.stale_trees.insert(key.to_owned());
        }
        answer
    }

    /// The key of window `id` of `extension`, when it contributes one.
    pub fn window_key(&self, extension: &str, id: &str) -> Option<String> {
        self.find(extension, kind::WINDOWS, id).map(Point::key)
    }

    /// Shows dialog point `key` drawn afresh, not from the tree it last answered.
    pub fn show_dialog(&mut self, key: String) {
        if self.get(&key).is_some_and(|p| p.tree) {
            self.trees.remove(&key);
            self.sessions.remove(&key);
        }
        self.dialog = Some(key);
    }

    /// What asking for tree point `key`'s tree asks of vornd, when it has none or its rows changed.
    pub fn render(&mut self, key: &str, cells: bool) -> Option<Value> {
        let p = self.get(key).filter(|p| p.tree)?;
        if self.trees.contains_key(key) && !self.stale_trees.contains(key) {
            return None;
        }
        let mut params = tree_render_params(p, cells);
        if let Some(scope) = unscope(key).1 {
            params["args"]["surface"]["scope"] = json!(scope);
            if let Some(w) = self.widths.get(key) {
                params["args"]["surface"]["width"] = json!(w);
            }
        }
        self.stale_trees.remove(key);
        Some(params)
    }

    /// Marks the trees of `extension` to render again after `ops` changed its rows: every
    /// tree drawn once, and of those drawn per scope only the ones whose scope a row is.
    pub fn trees_changed(&mut self, extension: &str, ops: &[Value]) {
        let keys: Vec<String> = self
            .all
            .iter()
            .filter(|p| p.tree && p.extension == extension)
            .map(Point::key)
            .collect();
        let clear = ops.iter().any(|o| s(o, "op") == Some("clear"));
        let ids: BTreeSet<&str> = ops
            .iter()
            .filter_map(|o| s(o, "id").or_else(|| o.get("row").and_then(|r| s(r, "id"))))
            .collect();
        let mut scoped: Vec<String> = keys
            .iter()
            .flat_map(|b| ids.iter().map(move |id| scoped(b, id)))
            .collect();
        if clear {
            scoped.extend(
                self.trees
                    .keys()
                    .filter(|k| {
                        let (base, scope) = unscope(k);
                        scope.is_some() && keys.iter().any(|b| b == base)
                    })
                    .cloned(),
            );
        }
        self.stale_trees.extend(scoped);
        self.stale_trees.extend(keys);
    }

    /// A fresh list: rows of extensions no longer listed go.
    pub fn listed(&mut self, list: &[Value]) {
        self.all = list.iter().filter_map(Point::from_value).collect();
        self.loaded = true;
        self.asked.clear();
        let all = &self.all;
        self.rows
            .retain(|ext, _| all.iter().any(|p| &p.extension == ext));
        let keys: BTreeSet<String> = self.all.iter().map(Point::key).collect();
        self.windows.retain(|k| keys.contains(k));
        self.trees.retain(|k, _| keys.contains(unscope(k).0));
        self.sessions.retain(|k, _| keys.contains(k));
        self.checked.retain(|(k, _), _| keys.contains(k));
        self.stale_trees.extend(self.trees.keys().cloned());
        if self.dialog.as_ref().is_some_and(|d| !keys.contains(d)) {
            self.dialog = None;
        }
    }

    /// The `(extension, collection)` pairs no one has asked for yet, now marked asked.
    pub fn to_ask(&mut self) -> Vec<(String, String)> {
        let mut out = Vec::new();
        let listed: BTreeSet<&str> = self.all.iter().map(|p| p.extension.as_str()).collect();
        let wanted = self
            .wanted
            .iter()
            .filter(|(e, _)| listed.contains(e.as_str()))
            .cloned();
        let all: Vec<(String, String)> = self
            .all
            .iter()
            .flat_map(|p| {
                p.collections()
                    .into_iter()
                    .map(|c| (p.extension.clone(), c))
            })
            .chain(wanted)
            .collect();
        for k in all {
            if !self.asked.contains(&k) {
                self.asked.insert(k.clone());
                self.pending.insert(k.clone(), false);
                out.push(k);
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

/// Every id in `tree`.
fn node_ids(tree: &vorn_tree::Node) -> BTreeSet<&str> {
    let mut out = BTreeSet::new();
    let mut stack = vec![tree];
    while let Some(n) = stack.pop() {
        out.extend(n.id.as_deref());
        stack.extend(&n.children);
    }
    out
}

/// What telling tree point `p` about `event` asks of vornd.
pub fn tree_event_params(p: &Point, event: &vorn_tree::Event) -> Value {
    json!({
        "id": p.extension,
        "action": vorn_tree::EVENT,
        "args": { "point": p.id, "event": event },
    })
}

/// What asking tree point `p` for its tree asks of vornd.
pub fn tree_render_params(p: &Point, cells: bool) -> Value {
    let surface = vorn_tree::Surface {
        cells,
        ..vorn_tree::Surface::default()
    };
    json!({
        "id": p.extension,
        "action": vorn_tree::RENDER,
        "args": { "point": p.id, "surface": surface },
    })
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

    fn point(kind: &str) -> Value {
        json!({ "extension": "x", "extensionName": "X", "kind": kind, "id": "p",
                "title": "P", "rows": "items", "call": "run" })
    }

    #[test]
    fn a_settings_page_follows_the_app_page_it_names() {
        let mut v = point(kind::SETTINGS);
        assert_eq!(Point::from_value(&v).expect("a point").placed_after(), None);
        v["after"] = json!("agents");
        assert_eq!(
            Point::from_value(&v).expect("a point").placed_after(),
            Some(crate::settings::Category::Agents)
        );
        v["after"] = json!("nowhere");
        assert_eq!(Point::from_value(&v).expect("a point").placed_after(), None);
        let mut menu = point(kind::MENUS);
        menu["after"] = json!("agents");
        assert_eq!(
            Point::from_value(&menu).expect("a point").placed_after(),
            None
        );
    }

    #[test]
    fn rows_follow_their_ops_in_place_and_raise_windows_on_new_ones() {
        let mut pts = Points::default();
        let mut w = point(kind::WINDOWS);
        w["raise"] = json!("items");
        w["id"] = json!("w");
        pts.listed(&[point(kind::MENUS), w]);
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
        assert_eq!(raised, ["x/windows/w"]);
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
        pts.listed(&[point(kind::MENUS)]);
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
    fn a_point_is_named_by_its_extension_and_title() {
        let p = Point::from_value(&point(kind::SIDEBAR)).expect("a point");
        assert_eq!(p.place(), "X › P");
        assert_eq!(p.key(), "x/sidebar/p");
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
    fn an_answer_says_what_to_open_and_toast() {
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

//! View options: how the Sessions and Tasks lists group, order and filter, what each row shows, and saving it all.

use serde_json::{json, Map, Value};
use vornui::{div, El, Id};

use super::{bar, col, listing, say, section, Cx, Do, Scope, Trigger, View};
use crate::phone::Phone;

/// One picked setting: its key, title, and options as (key, label, what it means).
pub struct Setting {
    pub key: &'static str,
    pub title: &'static str,
    pub options: &'static [(&'static str, &'static str, &'static str)],
    pub default: &'static str,
}

pub const SESSION_GROUP: Setting = Setting {
    key: "sessions.grouping",
    title: "Group by",
    options: &[
        ("status", "Status", ""),
        ("project", "Project", ""),
        ("worktree", "Worktree", ""),
        ("none", "Nothing", ""),
    ],
    default: "status",
};

pub const SESSION_ORDER: Setting = Setting {
    key: "sessions.ordering",
    title: "Order by",
    options: &[
        ("recent", "Recent", "last output"),
        ("created", "Created", "when it started"),
    ],
    default: "recent",
};

pub const SESSION_STATUS: Setting = Setting {
    key: "sessions.status",
    title: "Show",
    options: &[
        ("all", "All", ""),
        ("running", "Running", ""),
        ("waiting", "Waiting", ""),
        ("idle", "Idle", ""),
        ("error", "Error", ""),
    ],
    default: "all",
};

pub const TASK_GROUP: Setting = Setting {
    key: "tasks.grouping",
    title: "Group by",
    options: &[("project", "Project", ""), ("none", "None", "")],
    default: "none",
};

pub const TASK_ORDER: Setting = Setting {
    key: "tasks.ordering",
    title: "Order by",
    options: &[
        ("newest", "Newest", "most recently written"),
        ("oldest", "Oldest", "written longest ago"),
        ("manual", "Manual", "the order they are dragged into"),
    ],
    default: "manual",
};

const SETTINGS: [&Setting; 5] = [
    &SESSION_GROUP,
    &SESSION_ORDER,
    &SESSION_STATUS,
    &TASK_GROUP,
    &TASK_ORDER,
];

pub const SOURCE: &str = "tasks.source";

/// The facts a row can show, in the order it shows them, and those it starts with.
pub struct Fields {
    pub key: &'static str,
    pub all: &'static [(&'static str, &'static str)],
    pub default: &'static [&'static str],
}

pub const SESSION_FIELDS: Fields = Fields {
    key: "sessions",
    all: &[
        ("status", "Status"),
        ("branch", "Branch"),
        ("time", "Time"),
        ("project", "Project"),
        ("worktree", "Worktree"),
        ("agent", "Agent"),
    ],
    default: &["status", "branch", "time"],
};

pub const TASK_FIELDS: Fields = Fields {
    key: "tasks",
    all: &[
        ("project", "Project"),
        ("date", "Date"),
        ("status", "Status"),
        ("agent", "Agent"),
        ("branch", "Branch"),
        ("source", "Source"),
        ("id", "ID"),
    ],
    default: &["project", "date"],
};

/// The terminal's text size range, points.
pub const FONT_MIN: f32 = 8.0;
pub const FONT_MAX: f32 = 28.0;

impl View {
    /// What `s` is set to.
    pub fn pick(&self, s: &Setting) -> &str {
        self.picks.get(s.key).map_or(s.default, String::as_str)
    }

    /// The board's source: `all`, `local`, or a connector's id.
    pub fn source(&self) -> &str {
        self.picks.get(SOURCE).map_or("all", String::as_str)
    }

    /// The facts rows of `f` show, in its order.
    pub fn fields(&self, f: &Fields) -> Vec<&'static str> {
        match self.fields.get(f.key) {
            Some(on) => f
                .all
                .iter()
                .map(|(k, _)| *k)
                .filter(|k| on.iter().any(|o| o == k))
                .collect(),
            None => f.default.to_vec(),
        }
    }

    pub fn shows(&self, f: &Fields, field: &str) -> bool {
        self.fields(f).contains(&field)
    }

    fn fields_changed(&self, f: &Fields) -> bool {
        self.fields(f) != f.default
    }

    /// Whether the Sessions options differ from where they start, for the lit button.
    pub fn sessions_changed(&self) -> bool {
        [&SESSION_GROUP, &SESSION_ORDER, &SESSION_STATUS]
            .iter()
            .any(|s| self.pick(s) != s.default)
            || self.fields_changed(&SESSION_FIELDS)
    }

    pub fn tasks_changed(&self) -> bool {
        [&TASK_GROUP, &TASK_ORDER]
            .iter()
            .any(|s| self.pick(s) != s.default)
            || self.source() != "all"
            || self.archived
            || self.fields_changed(&TASK_FIELDS)
    }

    /// Turns `field` of `f` on or off.
    pub fn flip_field(&mut self, f: &Fields, field: &str) {
        let mut on: Vec<String> = self.fields(f).into_iter().map(str::to_owned).collect();
        if let Some(i) = on.iter().position(|o| o == field) {
            on.remove(i);
        } else {
            on.push(field.to_owned());
        }
        self.fields.insert(f.key, on);
    }

    /// What is worth keeping across launches, as JSON.
    pub fn prefs(&self) -> Value {
        let picks: Map<String, Value> = self
            .picks
            .iter()
            .map(|(k, v)| ((*k).to_owned(), json!(v)))
            .collect();
        let fields: Map<String, Value> = self
            .fields
            .iter()
            .map(|(k, v)| ((*k).to_owned(), json!(v)))
            .collect();
        let scope = match &self.sessions_scope {
            Some(Scope::Project(p)) => json!({ "project": p }),
            Some(Scope::Branch(b)) => json!({ "branch": b }),
            None => Value::Null,
        };
        json!({
            "picks": picks,
            "fields": fields,
            "archived": self.archived,
            "fontSize": self.font,
            "sessionsScope": scope,
            "tasksScope": self.tasks_scope,
            "column": self.column,
            "trigger": match self.trigger {
                Trigger::All => "all",
                Trigger::Manual => "manual",
                Trigger::Scheduled => "scheduled",
            },
        })
    }

    /// Takes back what [`View::prefs`] wrote, leaving anything unknown or out of range as it starts.
    pub fn restore(&mut self, v: &Value) {
        for s in SETTINGS {
            if let Some(k) = v["picks"][s.key].as_str() {
                if s.options.iter().any(|(o, _, _)| *o == k) {
                    self.picks.insert(s.key, k.to_owned());
                }
            }
        }
        if let Some(src) = v["picks"][SOURCE].as_str().filter(|s| !s.is_empty()) {
            self.picks.insert(SOURCE, src.to_owned());
        }
        for f in [&SESSION_FIELDS, &TASK_FIELDS] {
            if let Some(list) = v["fields"][f.key].as_array() {
                let on = list
                    .iter()
                    .filter_map(Value::as_str)
                    .filter(|k| f.all.iter().any(|(a, _)| a == k))
                    .map(str::to_owned)
                    .collect();
                self.fields.insert(f.key, on);
            }
        }
        self.archived = v["archived"].as_bool().unwrap_or(false);
        if let Some(size) = v["fontSize"].as_f64() {
            self.font = (size as f32).round().clamp(FONT_MIN, FONT_MAX);
        }
        self.sessions_scope = match (
            v["sessionsScope"]["project"].as_str(),
            v["sessionsScope"]["branch"].as_str(),
        ) {
            (Some(p), _) => Some(Scope::Project(p.to_owned())),
            (_, Some(b)) => Some(Scope::Branch(b.to_owned())),
            _ => None,
        };
        self.tasks_scope = v["tasksScope"].as_str().map(str::to_owned);
        if let Some(c) = v["column"].as_str() {
            if let Some(s) = vorn_app::tasks::STATUSES.iter().find(|s| s.key == c) {
                self.column = s.key;
            }
        }
        self.trigger = match v["trigger"].as_str() {
            Some("manual") => Trigger::Manual,
            Some("scheduled") => Trigger::Scheduled,
            _ => Trigger::All,
        };
    }
}

/// An option row, with what it means under it when it says.
fn pick_row(cx: &Cx, key: &'static str, option: &str, label: &str, sub: &str, on: bool) -> El {
    let el = cx.option(
        Id::new("pick").child((key, option)),
        label,
        on,
        Do::Pick(key, option.to_owned()),
    );
    if sub.is_empty() {
        el
    } else {
        el.child(say(sub, "secondary"))
    }
}

fn setting(cx: &Cx, s: &'static Setting) -> El {
    let now = cx.view().pick(s);
    section(
        s.title,
        s.options
            .iter()
            .map(|(k, label, sub)| pick_row(cx, s.key, k, label, sub, *k == now))
            .collect(),
    )
}

fn chips(cx: &Cx, f: &'static Fields) -> El {
    let mut rows: Vec<El> = f
        .all
        .iter()
        .map(|(k, label)| {
            cx.toggle(
                Id::new("field").child((f.key, *k)),
                label,
                cx.view().shows(f, k),
                Do::Field(f.key, k),
            )
        })
        .collect();
    if cx.view().fields_changed(f) {
        rows.push(cx.button(
            Id::new("fields-reset").child(f.key),
            "Reset",
            "plain",
            Do::ResetFields(f.key),
        ));
    }
    section("Each row shows", rows)
}

pub fn fields_of(key: &str) -> &'static Fields {
    if key == TASK_FIELDS.key {
        &TASK_FIELDS
    } else {
        &SESSION_FIELDS
    }
}

/// The Sessions list's View options.
pub fn sessions(cx: &Cx) -> El {
    col(vec![
        bar("View options", vec![]),
        listing(vec![
            setting(cx, &SESSION_GROUP),
            setting(cx, &SESSION_ORDER),
            setting(cx, &SESSION_STATUS),
            chips(cx, &SESSION_FIELDS),
        ]),
    ])
}

/// A connector's name, from vornd when it says, else its id.
pub fn connector_name(cx: &Cx, id: &str) -> String {
    match cx.fetch("connector:list", Value::Null) {
        super::Fetched::Ready(Value::Array(list)) => list
            .iter()
            .find(|c| c["id"] == id)
            .and_then(|c| c["name"].as_str())
            .unwrap_or(id)
            .to_owned(),
        _ => id.to_owned(),
    }
}

/// The board's View options.
pub fn tasks(cx: &Cx) -> El {
    let mut rows = vec![setting(cx, &TASK_GROUP), setting(cx, &TASK_ORDER)];
    let mut used = vorn_app::tasks::connectors(&cx.p.model.store.tasks);
    let now = cx.view().source().to_owned();
    let stranded = now != "all" && now != "local" && !used.contains(&now);
    if stranded {
        used.push(now.clone());
    }
    if !used.is_empty() {
        let mut opts = vec![
            pick_row(cx, SOURCE, "all", "All sources", "", now == "all"),
            pick_row(
                cx,
                SOURCE,
                "local",
                "Local only",
                "written here or on the desktop",
                now == "local",
            ),
        ];
        for id in &used {
            let sub = if stranded && *id == now {
                "nothing here has this source"
            } else {
                ""
            };
            opts.push(pick_row(
                cx,
                SOURCE,
                id,
                &connector_name(cx, id),
                sub,
                *id == now,
            ));
        }
        rows.push(section("Source", opts));
    }
    rows.push(chips(cx, &TASK_FIELDS));
    rows.push(section(
        "Also show",
        vec![cx.toggle(
            Id::new("tasks-archived"),
            "Archived",
            cx.view().archived,
            Do::Archived,
        )],
    ));
    col(vec![bar("View options", vec![]), listing(rows)])
}

/// The button that opens `route`, lit while its options differ from where they start.
pub fn button(id: &str, route: &str, lit: bool) -> El {
    let icon = div().sem(vornui::native::Sem::Icon {
        name: "sliders-horizontal".into(),
        tone: if lit { "lit" } else { "" },
        svg: None,
    });
    super::present_with(
        Id::new(id),
        if lit {
            "View options, changed"
        } else {
            "View options"
        },
        route.to_owned(),
        Some(icon),
    )
}

/// Makes the terminal's text `delta` points bigger, within range.
pub fn font(p: &mut Phone, delta: f32) {
    p.view.font = (p.view.font + delta).clamp(FONT_MIN, FONT_MAX);
    p.dirty = true;
}

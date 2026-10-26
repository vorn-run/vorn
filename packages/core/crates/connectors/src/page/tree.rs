//! A connector's accounts as an element tree, drawn inside its extension's
//! page in Settings: each connection with how it signs in and what uses it,
//! its key, and the add form.

use serde_json::{json, Value};
use vorn_tree::sdk::*;
use vorn_tree::{Event, EventKind, Node, Rendered, TextStyle, Tone};

use super::{text as s, CHOICES, CONNECTIONS, FILTERS, FORM, PROJECTS, SETTINGS, TRIGGERS};

/// What the page's controls run, by the id before the first `:`; after it is what they act on.
const DOES: [&str; 17] = [
    "refresh", "add", "add_to", "cancel", "save", "pick", "trigger", "project", "set", "test",
    "backfill", "signin", "rollback", "remove", "rotate", "unrotate", "newkey",
];

/// The accounts of connector `scope` (every connector's without one), from the rows by collection.
pub fn render(rows: &[(&str, Vec<Value>)], scope: Option<&str>) -> Value {
    json!(Rendered::new(page(rows, scope)))
}

/// The action an event on the page runs, and its args.
pub fn event(e: &Event) -> Result<(&'static str, Value), String> {
    let (does, on) = e.id.split_once(':').unwrap_or((e.id.as_str(), ""));
    let does = DOES
        .into_iter()
        .find(|d| *d == does)
        .ok_or_else(|| format!("no control is '{}'", e.id))?;
    let args = match e.kind {
        EventKind::Select => json!({ "id": e.value }),
        EventKind::Submit => json!({ "id": on, "value": e.value }),
        _ if on.is_empty() => Value::Null,
        _ => json!({ "id": on }),
    };
    Ok((does, args))
}

fn on(v: &Value, key: &str) -> bool {
    v[key].as_bool() == Some(true)
}

fn page(rows: &[(&str, Vec<Value>)], scope: Option<&str>) -> Node {
    let of = |coll: &str| {
        rows.iter()
            .find(|(c, _)| *c == coll)
            .map_or(&[][..], |(_, r)| r.as_slice())
    };
    let filters = of(FILTERS);
    let mine: Vec<&Value> = of(CONNECTIONS)
        .iter()
        .filter(|r| scope.is_none_or(|id| s(r, "connector") == id))
        .collect();
    let mut page = col([]).gap(8.0);
    if !mine.is_empty() {
        page = page.child(list(CONNECTIONS, mine.iter().map(|r| connection(r, filters))));
    }
    // The form shows under the connector it adds to; any connector's when unscoped.
    let form = of(FORM)
        .first()
        .filter(|f| scope.is_none_or(|id| s(f, "connector") == id));
    page.child(match form {
        Some(f) => adding(f, &of),
        None => {
            let (id, label) = match scope {
                Some(id) => (format!("add_to:{id}"), "Connect another account"),
                None => ("add".to_owned(), "Add connection"),
            };
            let label = if mine.is_empty() { "Connect an account" } else { label };
            row([button(id, label).icon("plus")])
        }
    })
}

/// One connection with its key line: Import now on hover; Test key, Rotate key…,
/// Roll back and Remove behind its menu.
fn connection(r: &Value, filters: &[Value]) -> Node {
    let id = s(r, "id");
    let name = s(r, "name");
    let status = match s(r, "state") {
        "ok" => Some(Tone::Good),
        "failing" => Some(Tone::Bad),
        "signin" => Some(Tone::Warn),
        _ => None,
    };
    let mut actions = Vec::new();
    if on(r, "needs_signin") {
        actions.push(button(format!("signin:{id}"), "Sign in").icon("log-in"));
    }
    actions.push(
        button(format!("backfill:{id}"), "Import now")
            .icon("import")
            .tip("Fetches what's new now instead of at the next check"),
    );
    let mut more = vec![button(format!("test:{id}"), "Test")
        .icon("zap")
        .tip("Checks that the account still answers")];
    if on(r, "browser") && !on(r, "needs_signin") {
        more.push(
            button(format!("signin:{id}"), "Reconnect")
                .icon("log-in")
                .tip("Signs in again in a window"),
        );
    }
    let fields = r["key_fields"].as_array().map_or(&[][..], Vec::as_slice);
    for f in fields {
        let label = if fields.len() > 1 {
            format!("Rotate {}…", s(f, "label"))
        } else {
            "Rotate key…".to_owned()
        };
        more.push(
            button(format!("rotate:{id}|{}", s(f, "key")), label)
                .icon("refresh-cw")
                .tip("Swaps in a new key; the old one stops working"),
        );
    }
    if on(r, "can_roll_back") {
        more.push(
            button(
                format!("rollback:{id}"),
                format!("Roll back to v{}", s(r, "previous")),
            )
            .icon("rotate-ccw"),
        );
    }
    more.push(
        button(format!("remove:{id}"), "Remove")
            .icon("trash-2")
            .danger()
            .confirm(format!("Remove {name}? Its tasks stay on the board.")),
    );
    let chips = filters
        .iter()
        .filter(|f| s(f, "parent") == id)
        .map(|f| s(f, "text").to_owned());
    let mut e = entry(id, name)
        .detail(s(r, "account_line"))
        .chips(chips)
        .actions(actions)
        .more(more);
    if let Some(t) = status {
        e = e.tone(t);
    }
    if let Some(k) = key_line(r) {
        e = e.child(k);
    }
    if on(r, "rotating") {
        e = e.child(rotating(r));
    }
    e
}

/// The key icon and what the key is, red when its last test failed.
fn key_line(r: &Value) -> Option<Node> {
    let line = s(r, "key_line");
    if line.is_empty() {
        return None;
    }
    let mut words = text(line).style(TextStyle::Secondary);
    if s(r, "key_state") == "failing" {
        words = words.tone(Tone::Bad);
    }
    Some(
        row([icon("key-round"), words])
            .gap(4.0)
            .id(format!("key:{}", s(r, "id")))
            .tip("Shows where the key is kept and when it last changed"),
    )
}

/// The new key's field while one is replaced: Return saves it, Cancel keeps the old one.
fn rotating(r: &Value) -> Node {
    let id = s(r, "id");
    let label = s(r, "rotating_label");
    let mut parts = vec![row([
        field(format!("newkey:{id}"), "")
            .secret()
            .placeholder(format!("New {label}, then Return"))
            .label(format!("New {label}"))
            .grow(),
        button(format!("unrotate:{id}"), "Cancel"),
    ])
    .gap(8.0)];
    if !s(r, "rotate_error").is_empty() {
        parts.push(notice(s(r, "rotate_error"), Tone::Bad));
    }
    col(parts).gap(4.0).py(4.0)
}

/// The form a new connection is set up in, from its connector to where its tasks go.
fn adding<'a>(form: &Value, of: &impl Fn(&str) -> &'a [Value]) -> Node {
    let picked = |coll: &str| {
        of(coll)
            .iter()
            .find(|r| on(r, "selected"))
            .map_or("", |r| s(r, "id"))
            .to_owned()
    };
    let options = |coll: &str| {
        of(coll)
            .iter()
            .map(|r| (s(r, "id").to_owned(), s(r, "name").to_owned()))
            .collect::<Vec<_>>()
    };
    let labelled = |label: &str, id: &str, coll: &str| {
        col([
            text(label).style(TextStyle::Secondary),
            choice(id, options(coll), &picked(coll)).label(label),
        ])
        .gap(4.0)
    };
    let mut parts = Vec::new();
    if on(form, "none_installed") {
        parts.push(notice(
            "No connector is installed yet. Connectors for other services are in Extensions › Browse.",
            Tone::Info,
        ));
    } else {
        parts.push(labelled("Service", "pick", CHOICES));
    }
    if on(form, "picked") {
        parts.push(labelled("Trigger", "trigger", TRIGGERS));
        for f in of(SETTINGS) {
            let mut head = row([text(s(f, "label")).style(TextStyle::Secondary).grow()]);
            if on(f, "kept") {
                head = head.child(text("Saved").style(TextStyle::Secondary));
            }
            let mut input = field(format!("set:{}", s(f, "id")), s(f, "value"))
                .placeholder(s(f, "hint"))
                .label(s(f, "label"));
            if on(f, "secret") {
                input = input.secret();
            }
            parts.push(col([head, input]).gap(4.0));
        }
        parts.push(labelled("Tasks go to", "project", PROJECTS));
    }
    if !s(form, "error").is_empty() {
        parts.push(notice(s(form, "error"), Tone::Bad));
    }
    let mut buttons = Vec::new();
    if on(form, "can_save") {
        buttons.push(button("save", "Save").primary());
    }
    buttons.push(button("cancel", "Cancel"));
    parts.push(row(buttons).gap(8.0));
    section("New connection", parts)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn rows(form: bool) -> Vec<(&'static str, Vec<Value>)> {
        let conn = |id: &str, name: &str| {
            json!({ "id": id, "name": name, "connector": "http",
                "account_line": "Uses a key · used by 2 workflows", "state": "ok",
                "browser": false, "needs_signin": false,
                "can_roll_back": false, "previous": "", "key_line": "Secret · ••••4f2a",
                "key_state": "ok", "key_fields": [{ "key": "secret", "label": "Secret" }],
                "rotating": id == "b", "rotating_label": "Secret", "rotate_error": "" })
        };
        let mut out = vec![
            (
                CONNECTIONS,
                vec![conn("a", "Example API"), conn("b", "Status page")],
            ),
            (
                FILTERS,
                vec![
                    json!({ "id": "a/0", "parent": "a", "text": "baseUrl: https://api.example.com" }),
                ],
            ),
        ];
        if form {
            out.push((
                FORM,
                vec![json!({ "id": "form", "connector": "acme", "picked": true, "can_save": false,
                    "none_installed": false, "error": "" })],
            ));
            out.push((
                CHOICES,
                vec![json!({ "id": "acme", "name": "Acme", "selected": true })],
            ));
            out.push((
                SETTINGS,
                vec![
                    json!({ "id": "TOKEN", "label": "TOKEN (required)", "hint": "",
                    "secret": true, "value": "", "kept": false }),
                ],
            ));
        }
        out
    }

    fn tree(rows: &[(&str, Vec<Value>)], scope: Option<&str>) -> String {
        let read = vorn_tree::read(&render(rows, scope)).expect("a valid tree");
        serde_json::to_string(&read.tree).unwrap()
    }

    #[test]
    fn lists_a_connectors_accounts_and_reads_back_clean() {
        let json = tree(&rows(false), Some("http"));
        assert!(json.contains("Uses a key · used by 2 workflows"), "{json}");
        assert!(json.contains("add_to:http"), "{json}");
        assert!(json.contains("Connect another account"), "{json}");
        assert!(json.contains("baseUrl: https://api.example.com"), "{json}");
        assert!(json.contains("\"id\":\"key:a\""), "{json}");
        assert!(json.contains("rotate:a|secret"), "{json}");
        assert!(json.contains("\"id\":\"newkey:b\""), "{json}");
        assert!(!json.contains("\"id\":\"newkey:a\""), "{json}");
        assert!(!json.contains("\"heading\""), "one unlabeled list: {json}");
    }

    #[test]
    fn another_connectors_page_shows_none_of_these_accounts() {
        let json = tree(&rows(false), Some("acme"));
        assert!(!json.contains("Example API"), "{json}");
        assert!(json.contains("add_to:acme"), "{json}");
        assert!(json.contains("Connect an account"), "{json}");
    }

    #[test]
    fn the_add_form_shows_only_under_the_connector_it_adds_to() {
        let json = tree(&rows(true), Some("acme"));
        assert!(json.contains("\"id\":\"set:TOKEN\""), "{json}");
        assert!(!json.contains("add_to:acme"), "one primary at a time: {json}");
        let json = tree(&rows(true), Some("http"));
        assert!(!json.contains("set:TOKEN"), "{json}");
        assert!(json.contains("add_to:http"), "{json}");
        let json = tree(&rows(true), None);
        assert!(json.contains("set:TOKEN"), "{json}");
        assert!(json.contains("Example API"), "{json}");
    }

    #[test]
    fn events_name_the_action_and_what_it_acts_on() {
        let ev = |id: &str, kind, value: &str| Event::new(id, kind, value);
        let (does, args) = event(&ev("remove:c1", EventKind::Click, "")).unwrap();
        assert_eq!((does, args), ("remove", json!({ "id": "c1" })));
        let (does, args) = event(&ev("pick", EventKind::Select, "acme")).unwrap();
        assert_eq!((does, args), ("pick", json!({ "id": "acme" })));
        let (does, args) = event(&ev("set:TOKEN", EventKind::Submit, "t")).unwrap();
        assert_eq!(
            (does, args),
            ("set", json!({ "id": "TOKEN", "value": "t" }))
        );
        assert_eq!(
            event(&ev("add", EventKind::Click, "")).unwrap().1,
            Value::Null
        );
        let (does, args) = event(&ev("rotate:c1|secret", EventKind::Click, "")).unwrap();
        assert_eq!((does, args), ("rotate", json!({ "id": "c1|secret" })));
        let (does, args) = event(&ev("newkey:c1", EventKind::Submit, "k")).unwrap();
        assert_eq!(
            (does, args),
            ("newkey", json!({ "id": "c1", "value": "k" }))
        );
        assert!(event(&ev("drop:x", EventKind::Click, "")).is_err());
    }
}

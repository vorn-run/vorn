//! The Connections page as an element tree: connections by connector, and the add form.

use serde_json::{json, Value};
use vorn_tree::sdk::*;
use vorn_tree::{Event, EventKind, Node, Rendered, TextStyle, Tone};

use super::{text as s, CHOICES, CONNECTIONS, FILTERS, FORM, PROJECTS, SETTINGS, TRIGGERS};

/// What the page's controls run, by the id before the first `:`; after it is what they act on.
const DOES: [&str; 14] = [
    "refresh", "add", "add_to", "cancel", "save", "pick", "trigger", "project", "set", "test",
    "backfill", "signin", "rollback", "remove",
];

/// The page drawn from its rows, by collection.
pub fn render(rows: &[(&str, Vec<Value>)]) -> Value {
    json!(Rendered::new(page(rows)))
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

fn page(rows: &[(&str, Vec<Value>)]) -> Node {
    let of = |coll: &str| {
        rows.iter()
            .find(|(c, _)| *c == coll)
            .map_or(&[][..], |(_, r)| r.as_slice())
    };
    let form = of(FORM).first();
    let mut page = col([]).gap(16.0);
    page = match form {
        None => page.child(toolbar([button("add", "Add connection")
            .icon("plus")
            .primary()])),
        Some(f) => page.child(adding(f, &of)),
    };
    let filters = of(FILTERS);
    let groups = group_by(of(CONNECTIONS), |r| s(r, "connector"))
        .into_iter()
        .map(|(id, rs)| {
            let first = rs[0];
            let entries = rs.into_iter().map(|r| connection(r, filters));
            let mut g = group(s(first, "connector_name"), entries)
                .icon(format!("connector:{id}"))
                .note(s(first, "connector_line"));
            if on(first, "can_add") {
                g = g.actions([button(format!("add_to:{id}"), "Add another").icon("plus")]);
            }
            g
        });
    page = page.child(
        list(CONNECTIONS, groups)
            .empty("No connections yet. Add one to have a workflow watch a service."),
    );
    if form.is_none() {
        page = page.child(
            text("Connectors for other services are in Extensions › Browse.")
                .style(TextStyle::Secondary),
        );
    }
    page
}

/// One connection: Test and Import now on hover, Roll back and Remove behind its menu.
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
    actions.push(button(format!("test:{id}"), "Test").icon("zap"));
    actions.push(button(format!("backfill:{id}"), "Import now").icon("import"));
    let mut more = Vec::new();
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
        .icon(s(r, "icon"))
        .detail(s(r, "line"))
        .chips(chips)
        .actions(actions)
        .more(more);
    if let Some(t) = status {
        e = e.tone(t);
    }
    e
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
            json!({ "id": id, "name": name, "connector": "http", "connector_name": "HTTP",
                "connector_line": "2 connections", "can_add": true, "icon": "connector:http",
                "line": "HTTP · synced just now", "state": "ok", "needs_signin": false,
                "can_roll_back": false, "previous": "" })
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
                vec![json!({ "id": "form", "picked": true, "can_save": false,
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

    #[test]
    fn groups_connections_by_connector_and_reads_back_clean() {
        let answer = render(&rows(false));
        let read = vorn_tree::read(&answer).expect("a valid tree");
        let json = serde_json::to_string(&read.tree).unwrap();
        assert_eq!(json.matches("\"heading\":\"HTTP\"").count(), 1, "{json}");
        assert!(json.contains("add_to:http"), "{json}");
        assert!(json.contains("baseUrl: https://api.example.com"), "{json}");
        assert!(json.contains("\"id\":\"add\""), "{json}");
        let adding = render(&rows(true));
        let read = vorn_tree::read(&adding).expect("a valid tree with the form");
        let json = serde_json::to_string(&read.tree).unwrap();
        assert!(
            !json.contains("\"id\":\"add\""),
            "one primary at a time: {json}"
        );
        assert!(json.contains("\"id\":\"set:TOKEN\""), "{json}");
    }

    #[test]
    fn events_name_the_action_and_what_it_acts_on() {
        let ev = |id: &str, kind, value: &str| Event {
            id: id.into(),
            kind,
            value: value.into(),
            selected: Vec::new(),
        };
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
        assert!(event(&ev("drop:x", EventKind::Click, "")).is_err());
    }
}

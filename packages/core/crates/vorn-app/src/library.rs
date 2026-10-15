//! The step library every + opens (`StepLibrary`): the built-in steps or
//! triggers, saved HTTP profiles, what was picked lately, then one folding
//! group per connection and per catalog connector; and the connector marks
//! (`ConnectorIcon`) its rows and the canvas draw.

use std::collections::{HashMap, HashSet};

use serde_json::Value;

use crate::runs::s;
use crate::workflows::{Anchor, Pick};

/// `HTTP_PROFILE_CONNECTOR`.
pub const HTTP_PROFILE: &str = "http";

/// `TRIGGER_ITEMS`.
pub const TRIGGERS: [(&str, &str, &str); 7] = [
    ("manual", "Manual", "zap"),
    ("recurring", "Recurring schedule", "clock"),
    ("once", "Schedule once", "calendar"),
    ("taskCreated", "Task created", "list-plus"),
    ("taskStatusChanged", "Task moved", "arrow-right-left"),
    ("sessionRestored", "Session restored", "rotate-ccw"),
    ("webhook", "Webhook", "globe"),
];

/// `STEP_ITEMS`.
pub const STEPS: [(&str, &str, &str); 6] = [
    ("agent", "Agent", "play"),
    ("script", "Script", "terminal"),
    ("httpRequest", "HTTP request", "globe"),
    ("condition", "Condition", "git-fork"),
    ("approval", "Approval gate", "hand"),
    ("loop", "Loop", "repeat"),
];

/// How a connector is drawn: by its id, or with its own glyph.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Default)]
pub struct Look {
    pub connector: String,
    /// A packaged connector's own paths and view box.
    pub icon: Option<(String, Vec<String>)>,
    pub packaged: bool,
}

/// An action someone picked, known by its connector.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Recent {
    pub connector: String,
    pub action: String,
    pub connection: Option<String>,
}

/// One line of the library.
#[derive(Debug, Clone, PartialEq)]
pub enum Entry {
    Heading(&'static str),
    Group {
        key: String,
        label: String,
        look: Option<Look>,
        detail: Option<&'static str>,
        count: usize,
        open: bool,
    },
    Row {
        key: String,
        label: String,
        pick: Pick,
        icon: Option<&'static str>,
        look: Option<Look>,
        owner: Option<String>,
        recent: Option<Recent>,
        nested: bool,
    },
}

impl Entry {
    pub fn key(&self) -> &str {
        match self {
            Entry::Heading(h) => h,
            Entry::Group { key, .. } | Entry::Row { key, .. } => key,
        }
    }

    /// Reachable by the arrow keys: every line but a heading.
    pub fn reachable(&self) -> bool {
        !matches!(self, Entry::Heading(_))
    }
}

/// What the library knows from vornd.
#[derive(Debug)]
pub struct Sources<'a> {
    pub connections: &'a [Value],
    /// `connection:listActions`, by connection.
    pub actions: &'a HashMap<String, Vec<Value>>,
    /// `connector:catalog`'s items.
    pub catalog: &'a [Value],
    /// `connector:listPacks`.
    pub packs: &'a [Value],
    /// `connector:list`, for each connector's trigger events.
    pub connectors: &'a [Value],
}

static NO_ACTIONS: std::sync::LazyLock<HashMap<String, Vec<Value>>> =
    std::sync::LazyLock::new(HashMap::new);

impl Default for Sources<'_> {
    fn default() -> Self {
        Sources {
            connections: &[],
            actions: &NO_ACTIONS,
            catalog: &[],
            packs: &[],
            connectors: &[],
        }
    }
}

/// `connectionConnectorId`.
pub fn connection_connector(c: &Value) -> &str {
    c.pointer("/filters/sdkConnectorId")
        .and_then(Value::as_str)
        .filter(|v| !v.is_empty())
        .or_else(|| s(c, "connectorId"))
        .unwrap_or("")
}

/// A `{viewBox, paths}` glyph, when it has paths to draw.
pub(crate) fn parse_icon(v: &Value) -> Option<(String, Vec<String>)> {
    let paths: Vec<String> = v
        .get("paths")?
        .as_array()?
        .iter()
        .filter_map(Value::as_str)
        .filter(|p| !p.is_empty())
        .map(str::to_owned)
        .collect();
    if paths.is_empty() {
        return None;
    }
    let vb = s(v, "viewBox").unwrap_or("0 0 24 24").to_owned();
    Some((vb, paths))
}

/// `connectorLookFor`.
pub fn look_for(src: &Sources<'_>, connection: &str) -> Option<Look> {
    let c = src
        .connections
        .iter()
        .find(|c| s(c, "id") == Some(connection))?;
    let connector = connection_connector(c).to_owned();
    let stored = c
        .pointer("/filters/sdkIcon")
        .and_then(Value::as_str)
        .and_then(|raw| serde_json::from_str::<Value>(raw).ok())
        .and_then(|v| parse_icon(&v));
    let icon = stored.or_else(|| {
        src.packs
            .iter()
            .find(|p| s(p, "id") == Some(connector.as_str()))
            .and_then(|p| p.get("icon"))
            .and_then(parse_icon)
    });
    Some(Look {
        packaged: connector != s(c, "connectorId").unwrap_or(""),
        connector,
        icon,
    })
}

/// The SVG a look draws: its own paths, a built-in mark, the plug for a
/// pack, or the generic link.
pub fn mark_svg(look: &Look) -> Option<String> {
    if let Some((vb, paths)) = &look.icon {
        let body: String = paths
            .iter()
            .map(|d| format!("<path d=\"{}\"/>", xml_attr(d)))
            .collect();
        return Some(format!(
            "<svg xmlns=\"http://www.w3.org/2000/svg\" viewBox=\"{}\" fill=\"#ffffff\">{body}</svg>",
            xml_attr(vb)
        ));
    }
    let name = match look.connector.as_str() {
        "github" => "github",
        "linear" => "linear",
        "http" => "globe",
        "mcp" | "sdk" => "mcp",
        _ if look.packaged => "mcp",
        _ => "external-link",
    };
    crate::look::icon(name, 2.0)
}

fn xml_attr(s: &str) -> String {
    s.replace('&', "&amp;")
        .replace('"', "&quot;")
        .replace('<', "&lt;")
}

/// What the anchor allows (`LibraryScope`).
#[derive(Debug, Clone, Copy, Default, PartialEq)]
pub struct Scope {
    pub body_only: bool,
    pub inside_branch: bool,
    pub triggers: bool,
    pub replacing: bool,
}

impl From<&Anchor> for Scope {
    fn from(a: &Anchor) -> Scope {
        Scope {
            body_only: a.body_only,
            inside_branch: a.inside_branch,
            triggers: a.trigger,
            replacing: a.replace.is_some(),
        }
    }
}

struct Group {
    key: String,
    label: String,
    look: Option<Look>,
    detail: Option<&'static str>,
    rows: Vec<Entry>,
}

fn row(key: String, label: String, pick: Pick, icon: Option<&'static str>) -> Entry {
    Entry::Row {
        key,
        label,
        pick,
        icon,
        look: None,
        owner: None,
        recent: None,
        nested: false,
    }
}

/// `packStateFor`'s word for a catalog connector not connected here.
fn catalog_detail(entry: &Value, packs: &[Value]) -> &'static str {
    let installed = packs.iter().any(|p| s(p, "id") == s(entry, "id"));
    let released = crate::plugins::installable(entry);
    if installed {
        "add connection"
    } else if !released {
        "not released yet"
    } else {
        "install on add"
    }
}

/// The library's lines for `scope`, `query`, what was picked lately and
/// which groups are open.
pub fn entries(
    src: &Sources<'_>,
    scope: Scope,
    query: &str,
    recent: &[Recent],
    open: &HashSet<String>,
) -> Vec<Entry> {
    let q = query.trim().to_lowercase();
    let matches =
        |texts: &[&str]| q.is_empty() || texts.iter().any(|t| t.to_lowercase().contains(&q));
    let mut flat: Vec<Entry> = Vec::new();
    let mut groups: Vec<Group> = Vec::new();
    let connection_group = |groups: &mut Vec<Group>, conn: &Value, rows: Vec<Entry>| {
        if rows.is_empty() {
            return;
        }
        let cid = s(conn, "id").unwrap_or("");
        let name = s(conn, "name").unwrap_or("").to_owned();
        let look = look_for(src, cid);
        groups.push(Group {
            key: format!("group:{cid}"),
            label: name.clone(),
            look: look.clone(),
            detail: None,
            rows: rows
                .into_iter()
                .map(|r| match r {
                    Entry::Row {
                        key,
                        label,
                        pick,
                        icon,
                        recent,
                        nested,
                        ..
                    } => Entry::Row {
                        key,
                        label,
                        pick,
                        icon,
                        look: look.clone(),
                        owner: Some(name.clone()),
                        recent,
                        nested,
                    },
                    other => other,
                })
                .collect(),
        });
    };

    if scope.triggers {
        for (kind, label, icon) in TRIGGERS {
            if matches(&[label]) {
                flat.push(row(
                    format!("trigger:{kind}"),
                    label.into(),
                    Pick::TriggerType(kind.into()),
                    Some(icon),
                ));
            }
        }
        for conn in src.connections {
            let cid = s(conn, "id").unwrap_or("");
            let name = s(conn, "name").unwrap_or("");
            let triggers = src
                .connectors
                .iter()
                .find(|c| s(c, "id") == s(conn, "connectorId"))
                .and_then(|c| c.pointer("/manifest/triggers"))
                .and_then(Value::as_array)
                .cloned()
                .unwrap_or_default();
            let rows = triggers
                .iter()
                .filter_map(|t| {
                    let kind = s(t, "type")?;
                    let label = s(t, "label").filter(|l| !l.is_empty()).unwrap_or(kind);
                    matches(&[label, name]).then(|| {
                        row(
                            format!("event:{cid}:{kind}"),
                            label.into(),
                            Pick::ConnectorTrigger {
                                connection: cid.into(),
                                event: kind.into(),
                            },
                            None,
                        )
                    })
                })
                .collect();
            connection_group(&mut groups, conn, rows);
        }
    } else {
        for (kind, label, icon) in STEPS {
            if scope.body_only && (kind == "approval" || kind == "loop") {
                continue;
            }
            if scope.inside_branch && kind == "loop" {
                continue;
            }
            if scope.replacing && (kind == "condition" || kind == "loop") {
                continue;
            }
            if !matches(&[label]) {
                continue;
            }
            flat.push(row(
                format!("type:{kind}"),
                label.into(),
                Pick::Type(kind.into()),
                Some(icon),
            ));
        }
        if !scope.body_only
            && !scope.inside_branch
            && !scope.replacing
            && matches(&["Parallel branch"])
        {
            flat.push(row(
                "parallel".into(),
                "Parallel branch".into(),
                Pick::Parallel,
                Some("split"),
            ));
        }
        if !scope.replacing {
            let profiles: Vec<Entry> = src
                .connections
                .iter()
                .filter(|c| connection_connector(c) == HTTP_PROFILE)
                .filter_map(|c| {
                    let name = s(c, "name").unwrap_or("");
                    let label = format!("Call {name}");
                    matches(&[&label]).then(|| {
                        row(
                            format!("profile:{}", s(c, "id").unwrap_or("")),
                            label,
                            Pick::HttpProfile {
                                connection: s(c, "id").unwrap_or("").into(),
                                name: name.into(),
                            },
                            Some("globe"),
                        )
                    })
                })
                .collect();
            let at = flat
                .iter()
                .position(|e| e.key() == "type:httpRequest")
                .map_or(flat.len(), |i| i + 1);
            flat.splice(at..at, profiles);
        }
        for conn in src.connections {
            let cid = s(conn, "id").unwrap_or("");
            let name = s(conn, "name").unwrap_or("");
            let connector = connection_connector(conn);
            let rows = src
                .actions
                .get(cid)
                .into_iter()
                .flatten()
                .filter_map(|a| {
                    let kind = s(a, "type")?;
                    let label = s(a, "label").filter(|l| !l.is_empty()).unwrap_or(kind);
                    matches(&[label, name]).then(|| Entry::Row {
                        key: format!("action:{cid}:{kind}"),
                        label: label.into(),
                        pick: Pick::ConnectorAction {
                            connection: cid.into(),
                            action: kind.into(),
                            label: label.into(),
                        },
                        icon: None,
                        look: None,
                        owner: None,
                        recent: (!connector.is_empty()).then(|| Recent {
                            connector: connector.into(),
                            action: kind.into(),
                            connection: Some(cid.into()),
                        }),
                        nested: false,
                    })
                })
                .collect();
            connection_group(&mut groups, conn, rows);
        }
        if !scope.replacing {
            let connected: HashSet<&str> =
                src.connections.iter().map(connection_connector).collect();
            let mut unvouched = Vec::new();
            for entry in src.catalog {
                let eid = s(entry, "id").unwrap_or("");
                let actions = entry
                    .get("actions")
                    .and_then(Value::as_array)
                    .cloned()
                    .unwrap_or_default();
                if connected.contains(eid) || actions.is_empty() {
                    continue;
                }
                let name = s(entry, "name").unwrap_or("");
                let mut keys = vec![name];
                keys.extend(
                    entry
                        .get("keywords")
                        .and_then(Value::as_array)
                        .into_iter()
                        .flatten()
                        .filter_map(Value::as_str),
                );
                let whole = matches(&keys);
                let picked: Vec<&Value> = actions
                    .iter()
                    .filter(|a| {
                        whole || {
                            let k = s(a, "type").unwrap_or("");
                            matches(&[s(a, "label").filter(|l| !l.is_empty()).unwrap_or(k)])
                        }
                    })
                    .collect();
                if picked.is_empty() {
                    continue;
                }
                let look = Look {
                    connector: eid.into(),
                    icon: entry.get("icon").and_then(parse_icon),
                    packaged: false,
                };
                let rows = picked
                    .iter()
                    .filter_map(|a| {
                        let kind = s(a, "type")?;
                        let label = s(a, "label").filter(|l| !l.is_empty()).unwrap_or(kind);
                        Some(Entry::Row {
                            key: format!("catalog:{eid}:{kind}"),
                            label: label.into(),
                            pick: Pick::CatalogAction {
                                connector: eid.into(),
                                action: kind.into(),
                                label: label.into(),
                            },
                            icon: None,
                            look: Some(look.clone()),
                            owner: Some(name.into()),
                            recent: Some(Recent {
                                connector: eid.into(),
                                action: kind.into(),
                                connection: None,
                            }),
                            nested: false,
                        })
                    })
                    .collect();
                let g = Group {
                    key: format!("catalog-group:{eid}"),
                    label: name.into(),
                    look: Some(look),
                    detail: Some(catalog_detail(entry, src.packs)),
                    rows,
                };
                if entry.get("verified").and_then(Value::as_bool) == Some(true) {
                    groups.push(g);
                } else {
                    unvouched.push(g);
                }
            }
            groups.extend(unvouched);
        }
    }

    if !q.is_empty() {
        let mut out = flat;
        for g in groups {
            out.extend(g.rows);
        }
        return out;
    }
    let mut out = Vec::new();
    if !flat.is_empty() {
        out.push(Entry::Heading(if scope.triggers {
            "Triggers"
        } else {
            "Steps"
        }));
        out.extend(flat);
    }
    let offered: Vec<&Entry> = groups.iter().flat_map(|g| g.rows.iter()).collect();
    let lately: Vec<Entry> = recent
        .iter()
        .filter_map(|r| {
            let same: Vec<&&Entry> = offered
                .iter()
                .filter(|e| matches!(e, Entry::Row { recent: Some(x), .. } if x.connector == r.connector && x.action == r.action))
                .collect();
            let pick = same
                .iter()
                .find(|e| matches!(e, Entry::Row { recent: Some(x), .. } if x.connection == r.connection))
                .or_else(|| same.first())?;
            let mut e = (**pick).clone();
            if let Entry::Row { key, .. } = &mut e {
                *key = format!("recent:{key}");
            }
            Some(e)
        })
        .collect();
    if !lately.is_empty() {
        out.push(Entry::Heading("Recent"));
        out.extend(lately);
    }
    for g in groups {
        let is_open = open.contains(&g.key);
        out.push(Entry::Group {
            key: g.key.clone(),
            label: g.label,
            look: g.look,
            detail: g.detail,
            count: g.rows.len(),
            open: is_open,
        });
        if is_open {
            out.extend(g.rows.into_iter().map(|r| match r {
                Entry::Row {
                    key,
                    label,
                    pick,
                    icon,
                    look,
                    owner,
                    recent,
                    ..
                } => Entry::Row {
                    key,
                    label,
                    pick,
                    icon,
                    look,
                    owner,
                    recent,
                    nested: true,
                },
                other => other,
            }));
        }
    }
    out
}

/// `recordRecentAction`: newest first, five kept.
pub fn remember(recent: &mut Vec<Recent>, picked: Recent) {
    recent.retain(|r| r.connector != picked.connector || r.action != picked.action);
    recent.insert(0, picked);
    recent.truncate(5);
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn keys(e: &[Entry]) -> Vec<String> {
        e.iter().map(|e| e.key().to_owned()).collect()
    }

    #[test]
    fn the_scope_decides_the_built_in_steps() {
        let src = Sources::default();
        let all = entries(&src, Scope::default(), "", &[], &HashSet::new());
        assert_eq!(keys(&all)[0], "Steps");
        assert_eq!(all.len(), 1 + 6 + 1, "steps and a parallel branch");
        let body = Scope {
            body_only: true,
            ..Scope::default()
        };
        let rows = entries(&src, body, "", &[], &HashSet::new());
        assert!(!keys(&rows)
            .iter()
            .any(|k| k == "type:approval" || k == "type:loop" || k == "parallel"));
        let replacing = Scope {
            replacing: true,
            ..Scope::default()
        };
        let rows = entries(&src, replacing, "", &[], &HashSet::new());
        assert!(!keys(&rows)
            .iter()
            .any(|k| k == "type:condition" || k == "parallel"));
        let triggers = Scope {
            triggers: true,
            ..Scope::default()
        };
        assert_eq!(
            entries(&src, triggers, "task", &[], &HashSet::new()).len(),
            2
        );
    }

    #[test]
    fn connections_fold_into_groups_and_a_search_lists_every_match() {
        let connections = [
            json!({ "id": "c1", "name": "Acme", "connectorId": "github" }),
            json!({ "id": "p1", "name": "Billing", "connectorId": "http" }),
        ];
        let actions = HashMap::from([(
            "c1".to_owned(),
            vec![json!({ "type": "createIssue", "label": "Create issue" })],
        )]);
        let catalog = [
            json!({ "id": "mysql", "name": "MySQL", "actions": [{ "type": "describe", "label": "Describe a table" }], "git": "https://x.example/c.git", "verified": true }),
        ];
        let src = Sources {
            connections: &connections,
            actions: &actions,
            catalog: &catalog,
            ..Sources::default()
        };
        let closed = entries(&src, Scope::default(), "", &[], &HashSet::new());
        let k = keys(&closed);
        assert!(k.contains(&"profile:p1".to_owned()));
        let at = k.iter().position(|k| k == "profile:p1").unwrap();
        assert_eq!(k[at - 1], "type:httpRequest");
        assert!(
            k.contains(&"group:c1".to_owned()) && !k.contains(&"action:c1:createIssue".to_owned())
        );
        let open = HashSet::from(["group:c1".to_owned()]);
        assert!(keys(&entries(&src, Scope::default(), "", &[], &open))
            .contains(&"action:c1:createIssue".to_owned()));
        let found = entries(&src, Scope::default(), "describe", &[], &HashSet::new());
        assert_eq!(keys(&found), ["catalog:mysql:describe"]);
        let mut recent = Vec::new();
        remember(
            &mut recent,
            Recent {
                connector: "github".into(),
                action: "createIssue".into(),
                connection: Some("c1".into()),
            },
        );
        let with = entries(&src, Scope::default(), "", &recent, &HashSet::new());
        assert!(keys(&with).contains(&"recent:action:c1:createIssue".to_owned()));
    }
}

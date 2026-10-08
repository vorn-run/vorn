//! Reading a connection's new items, page by page: an MCP connection's poll
//! tool and a package's trigger, both turned into the events a connector-poll
//! workflow's inbox holds and the items a backfill puts on the task board.

use serde_json::{json, Map, Value};

use crate::connections::{Item, POLL_EVENT};
use crate::js;

/// Most pages read in one poll, so a connector cannot hold a tick forever.
pub const MAX_PAGES_PER_POLL: usize = 20;
/// Runaway guard for a backfill that keeps handing back fresh cursors.
pub const MAX_BACKFILL_PAGES: usize = 1_000;

/// Keys that would reach the prototype if an argument object is re-keyed.
const UNSAFE_KEYS: [&str; 3] = ["__proto__", "constructor", "prototype"];

/// One page of a poll: its events, the cursor after it, and whether more follow.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct Page {
    pub events: Vec<Value>,
    pub next_cursor: Option<String>,
    pub has_more: bool,
}

/// An MCP connection's poll settings, read from its filters.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct McpPoll {
    pub tool: Option<String>,
    pub args: Option<String>,
    pub items_path: Option<String>,
    pub id_field: Option<String>,
    pub timestamp_field: Option<String>,
    pub title_field: Option<String>,
    pub url_field: Option<String>,
    pub cursor_arg: Option<String>,
    pub cursor_path: Option<String>,
}

impl McpPoll {
    pub fn of(filters: &Map<String, Value>) -> McpPoll {
        let get = |k: &str| {
            filters
                .get(k)
                .and_then(Value::as_str)
                .map(|s| js::trim(s).to_owned())
                .filter(|s| !s.is_empty())
        };
        McpPoll {
            tool: get("pollTool"),
            args: get("pollArgs"),
            items_path: get("itemsPath"),
            id_field: get("idField"),
            timestamp_field: get("timestampField"),
            title_field: get("titleField"),
            url_field: get("urlField"),
            cursor_arg: get("cursorArg"),
            cursor_path: get("cursorPath"),
        }
    }

    /// The tool's arguments for a poll after `cursor`.
    pub fn arguments(&self, cursor: Option<&str>) -> Result<Map<String, Value>, String> {
        let mut args = Map::new();
        if let Some(raw) = &self.args {
            match serde_json::from_str::<Value>(raw) {
                Ok(Value::Object(parsed)) => {
                    args = parsed
                        .into_iter()
                        .filter(|(k, _)| !UNSAFE_KEYS.contains(&k.as_str()))
                        .collect();
                }
                Ok(_) => return Err("Invalid pollArgs JSON: pollArgs must be a JSON object".into()),
                Err(err) => return Err(format!("Invalid pollArgs JSON: {}", json_error(raw, &err))),
            }
        }
        if let Some(arg) = &self.cursor_arg {
            if UNSAFE_KEYS.contains(&arg.as_str()) {
                return Err(format!("Cursor argument \"{arg}\" is not a usable argument name"));
            }
            if let Some(cursor) = cursor {
                args.insert(arg.clone(), json!(cursor));
            }
        }
        Ok(args)
    }

    /// The events a tool's `output` holds after `cursor` (`pollMcpConnection`).
    pub fn page(&self, cursor: Option<&str>, output: &Value, now: &str) -> Result<Page, String> {
        let raw = walk(output, self.items_path.as_deref());
        let Some(Value::Array(items)) = raw else {
            let place = match &self.items_path {
                Some(p) => format!("itemsPath \"{p}\""),
                None => "the tool result".to_owned(),
            };
            return Err(format!("MCP poll: {place} did not resolve to an array"));
        };
        let mut events = Vec::new();
        let mut newest = cursor.map(str::to_owned);
        for item in items.iter().filter_map(Value::as_object) {
            let ts = field(item, self.timestamp_field.as_deref());
            if self.timestamp_field.is_some() && self.cursor_arg.is_none() {
                let Some(ts) = &ts else { continue };
                if cursor.is_some_and(|c| js::less(ts, c)) {
                    continue;
                }
            }
            if let Some(ts) = &ts {
                if newest.as_deref().is_none_or(|n| js::less(n, ts)) {
                    newest = Some(ts.clone());
                }
            }
            let id = field(item, self.id_field.as_deref())
                .unwrap_or_else(|| Value::Object(item.clone()).to_string());
            let mut data = item.clone();
            data.insert("externalId".into(), json!(id));
            if let Some(url) = field(item, self.url_field.as_deref()) {
                data.insert("url".into(), json!(url));
            }
            if let Some(title) = field(item, self.title_field.as_deref()) {
                data.insert("title".into(), json!(title));
            }
            events.push(json!({
                "id": id,
                "type": POLL_EVENT,
                "timestamp": ts.unwrap_or_else(|| now.to_owned()),
                "data": data,
            }));
        }
        let next_cursor = if self.cursor_arg.is_some() {
            let next = walk(output, Some(self.cursor_path.as_deref().unwrap_or("nextCursor")));
            match next {
                Some(Value::String(s)) if !s.is_empty() => Some(s.clone()),
                _ => cursor.map(str::to_owned),
            }
        } else if self.timestamp_field.is_some() {
            newest.or_else(|| cursor.map(str::to_owned))
        } else {
            None
        };
        Ok(Page {
            events,
            next_cursor,
            has_more: false,
        })
    }

    /// The connection's filters with a seeded starting cursor taken out of
    /// its poll arguments, for a backfill that starts at the beginning.
    pub fn without_seed_cursor(&self, filters: &Map<String, Value>) -> Map<String, Value> {
        let (Some(arg), Some(raw)) = (&self.cursor_arg, filters.get("pollArgs").and_then(Value::as_str)) else {
            return filters.clone();
        };
        if !raw.contains(arg.as_str()) {
            return filters.clone();
        }
        let Ok(Value::Object(mut parsed)) = serde_json::from_str::<Value>(raw) else {
            return filters.clone();
        };
        parsed.remove(arg);
        let mut out = filters.clone();
        out.insert("pollArgs".into(), json!(Value::Object(parsed).to_string()));
        out
    }
}

/// `JSON.parse`'s complaint about `raw`, close to the words Node uses.
fn json_error(raw: &str, err: &serde_json::Error) -> String {
    if raw.trim().is_empty() {
        return "Unexpected end of JSON input".to_owned();
    }
    err.to_string()
}

/// `String(item[field])`, or nothing when the field is absent or null.
fn field(item: &Map<String, Value>, name: Option<&str>) -> Option<String> {
    match item.get(name?) {
        None | Some(Value::Null) => None,
        Some(v) => Some(js::to_string(v)),
    }
}

/// A dotted path into `root`, own keys only; no path is the root itself.
pub fn walk<'a>(root: &'a Value, path: Option<&str>) -> Option<&'a Value> {
    let Some(path) = path.filter(|p| !p.is_empty()) else {
        return Some(root);
    };
    let mut current = root;
    for segment in path.split('.') {
        current = match current {
            Value::Object(map) => map.get(segment)?,
            Value::Array(items) => items.get(segment.parse::<usize>().ok()?)?,
            _ => return None,
        };
    }
    Some(current)
}

/// A poll event back in the shape a backfill upserts (`eventToExternalItem`).
pub fn event_item(event: &Value) -> Item {
    let data = event.get("data").cloned().unwrap_or(Value::Null);
    let s = |v: Option<&Value>| match v {
        None | Some(Value::Null) => String::new(),
        Some(v) => js::to_string(v),
    };
    let either = |a: &str, b: &str| data.get(a).filter(|v| !v.is_null()).or_else(|| data.get(b));
    Item {
        external_id: s(data.get("externalId").filter(|v| !v.is_null()).or_else(|| event.get("id"))),
        title: s(data.get("title")),
        description: s(either("description", "body")),
        external_url: s(data.get("url")),
        status_raw: s(either("status", "state")),
        updated_at: s(event.get("timestamp")),
    }
}

/// A package trigger's page as events (`toEvent`).
pub fn sdk_page(page: &Map<String, Value>, now: &str) -> Page {
    let events = page
        .get("items")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .map(|item| {
            let updated = item
                .get("updatedAt")
                .filter(|v| js::truthy(v))
                .cloned()
                .unwrap_or_else(|| json!(now));
            json!({ "id": item.get("externalId"), "type": POLL_EVENT, "timestamp": updated, "data": item })
        })
        .collect();
    Page {
        events,
        next_cursor: page.get("nextCursor").and_then(Value::as_str).map(str::to_owned),
        has_more: page.get("hasMore") == Some(&Value::Bool(true)),
    }
}

/// A package trigger's item, as a backfill upserts it (`toExternalItem`).
pub fn sdk_item(item: &Value, now: &str) -> Item {
    let s = |k: &str| match item.get(k) {
        None | Some(Value::Null) => String::new(),
        Some(v) => js::to_string(v),
    };
    let updated = item
        .get("updatedAt")
        .filter(|v| js::truthy(v))
        .map_or_else(|| now.to_owned(), js::to_string);
    Item {
        external_id: s("externalId"),
        title: s("title"),
        description: s("description"),
        external_url: s("url"),
        status_raw: s("status"),
        updated_at: updated,
    }
}

/// The inbox rows a page's events become, with the item each carries
/// (`ConnectorItemContext`).
pub fn inbox_events(conn: &Value, events: &[Value]) -> Vec<Value> {
    events
        .iter()
        .map(|event| {
            let data = event.get("data").cloned().unwrap_or_else(|| json!({}));
            let external = match data.get("externalId") {
                None | Some(Value::Null) => event.get("id").map_or_else(|| "undefined".into(), js::to_string),
                Some(v) => js::to_string(v),
            };
            let mut item = Map::new();
            item.insert("connectionId".into(), conn.get("id").cloned().unwrap_or(Value::Null));
            item.insert("connectorId".into(), conn.get("connectorId").cloned().unwrap_or(Value::Null));
            item.insert("externalId".into(), json!(external));
            if let Some(url) = data.get("url").filter(|u| u.is_string()) {
                item.insert("externalUrl".into(), url.clone());
            }
            let title = match data.get("title") {
                Some(Value::String(t)) => t.clone(),
                None | Some(Value::Null) => String::new(),
                Some(other) => js::to_string(other),
            };
            item.insert("title".into(), json!(title));
            if let Some(body) = data.get("description").filter(|d| d.is_string()) {
                item.insert("body".into(), body.clone());
            }
            item.insert("raw".into(), data);
            json!({
                "eventId": event.get("id"),
                "eventType": event.get("type"),
                "eventTimestamp": event.get("timestamp"),
                "connectorItem": item,
            })
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn poll(filters: Value) -> McpPoll {
        McpPoll::of(filters.as_object().unwrap())
    }

    #[test]
    fn reads_arguments_and_refuses_the_prototype() {
        let cfg = poll(json!({ "pollTool": "list", "pollArgs": "{\"a\":1,\"__proto__\":2}", "cursorArg": "after" }));
        assert_eq!(Value::Object(cfg.arguments(None).unwrap()), json!({ "a": 1 }));
        assert_eq!(Value::Object(cfg.arguments(Some("c")).unwrap()), json!({ "a": 1, "after": "c" }));
        assert!(poll(json!({ "pollArgs": "[1]" })).arguments(None).unwrap_err().contains("must be a JSON object"));
        assert!(poll(json!({ "cursorArg": "constructor" })).arguments(None).unwrap_err().contains("not a usable argument name"));
    }

    #[test]
    fn filters_by_timestamp_and_keeps_items_at_the_cursor() {
        let cfg = poll(json!({ "pollTool": "list", "itemsPath": "data.items", "idField": "id", "timestampField": "at", "titleField": "name" }));
        let output = json!({ "data": { "items": [
            { "id": 1, "at": "2030-01-01", "name": "old" },
            { "id": 2, "at": "2030-01-02", "name": "same" },
            { "id": 3, "at": "2030-01-03", "name": "new" },
            { "id": 4, "name": "undated" },
        ] } });
        let page = cfg.page(Some("2030-01-02"), &output, "now").unwrap();
        let ids: Vec<&Value> = page.events.iter().map(|e| &e["id"]).collect();
        assert_eq!(ids, [&json!("2"), &json!("3")]);
        assert_eq!(page.next_cursor.as_deref(), Some("2030-01-03"));
        assert_eq!(page.events[1]["data"]["title"], "new");
        assert_eq!(page.events[1]["data"]["externalId"], "3");
        assert!(cfg.page(None, &json!({ "data": {} }), "now").unwrap_err().contains("itemsPath \"data.items\""));
    }

    #[test]
    fn a_tool_that_takes_the_cursor_decides_what_is_new() {
        let cfg = poll(json!({ "pollTool": "t", "cursorArg": "after", "timestampField": "at" }));
        let page = cfg.page(Some("x"), &json!([{ "at": "a" }]), "now").unwrap();
        assert_eq!(page.events.len(), 1);
        assert_eq!(page.next_cursor.as_deref(), Some("x"));
        let page = cfg.page(None, &json!({ "nextCursor": "n" }), "now");
        assert!(page.is_err());
        let page = poll(json!({ "pollTool": "t" })).page(None, &json!([{ "k": 1 }]), "now").unwrap();
        assert_eq!(page.events[0]["id"], "{\"k\":1}");
        assert_eq!(page.next_cursor, None);
    }

    #[test]
    fn a_backfill_starts_before_a_seeded_cursor() {
        let filters = json!({ "pollArgs": "{\"after\":\"seed\",\"a\":1}", "cursorArg": "after" });
        let cfg = poll(filters.clone());
        let clean = cfg.without_seed_cursor(filters.as_object().unwrap());
        assert_eq!(clean["pollArgs"], "{\"a\":1}");
    }

    #[test]
    fn shapes_package_pages_and_inbox_rows() {
        let page = json!({ "items": [{ "externalId": "e1", "title": "T", "url": "u", "updatedAt": "" }], "hasMore": true, "nextCursor": "n" });
        let page = sdk_page(page.as_object().unwrap(), "now");
        assert!(page.has_more);
        assert_eq!(page.events[0]["timestamp"], "now");
        let rows = inbox_events(&json!({ "id": "c", "connectorId": "sdk" }), &page.events);
        assert_eq!(rows[0]["connectorItem"]["externalId"], "e1");
        assert_eq!(rows[0]["connectorItem"]["externalUrl"], "u");
        let item = sdk_item(&json!({ "externalId": 5, "updatedAt": "t" }), "now");
        assert_eq!((item.external_id.as_str(), item.updated_at.as_str()), ("5", "t"));
        let back = event_item(&json!({ "id": "i", "timestamp": "t", "data": { "body": "b", "state": "open" } }));
        assert_eq!((back.external_id.as_str(), back.description.as_str(), back.status_raw.as_str()), ("i", "b", "open"));
    }
}

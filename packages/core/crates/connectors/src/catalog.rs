//! The connectors Vorn can offer, fetched from the connectors repository.
//!
//! What is already known is served at once: the last good fetch, cached on
//! disk, or the bundled seed on a first run with no network. A stale list is
//! refreshed in the background, and a refresh that brings a different list
//! says so, so a list on screen can read it again. The document comes off the
//! network, so every entry is repaired or dropped on its own rather than
//! trusted; one bad connector upstream never hides the rest.

use std::path::{Path, PathBuf};
use std::sync::Mutex;
use std::time::Duration;

use serde_json::{json, Map, Value};

use crate::fetch::Fetch;
use crate::manifest;

/// Where the published catalog lives.
pub const CATALOG_URL: &str =
    "https://raw.githubusercontent.com/vorn-run/connectors/main/catalog.json";
/// Long enough that opening settings twice does not fetch twice.
pub const MAX_AGE_MS: u64 = 6 * 60 * 60 * 1000;
/// A slow network must not hold up the connector list.
pub const FETCH_TIMEOUT: Duration = Duration::from_secs(5);
/// A catalog is a small document; past this it is not one.
const MAX_DOCUMENT_BYTES: u64 = 8 * 1024 * 1024;
const VERIFICATION_SCHEMA: i64 = 1;
const PORTABLE_FORMAT_VERSION: i64 = 1;
const AUTH_RUNGS: [&str; 5] = ["none", "cli", "key", "browser", "oauth"];

/// The bundled list, for a first run with no network.
pub fn seed() -> Vec<Value> {
    serde_json::from_str(include_str!("../data/catalog-seed.json"))
        .expect("the bundled catalog is JSON")
}

/// The templates a new workflow can start from when the catalog has none.
pub fn template_seed() -> Vec<Value> {
    serde_json::from_str(include_str!("../data/template-seed.json"))
        .expect("the bundled templates are JSON")
}

fn records(raw: Option<&Value>) -> Vec<&Map<String, Value>> {
    raw.and_then(Value::as_array)
        .into_iter()
        .flatten()
        .filter_map(Value::as_object)
        .collect()
}

fn list(raw: Option<&Value>) -> Value {
    match raw {
        Some(Value::Array(items)) => Value::Array(items.clone()),
        _ => json!([]),
    }
}

fn strings(raw: Option<&Value>) -> Value {
    Value::Array(
        raw.and_then(Value::as_array)
            .into_iter()
            .flatten()
            .filter(|v| v.is_string())
            .cloned()
            .collect(),
    )
}

fn text<'a>(entry: &'a Map<String, Value>, key: &str) -> Option<&'a str> {
    entry.get(key).and_then(Value::as_str)
}

fn non_empty<'a>(entry: &'a Map<String, Value>, key: &str) -> Option<&'a str> {
    text(entry, key).filter(|s| !s.is_empty())
}

/// `parseCatalog`: the entries, or `None` for a document this build cannot read.
pub fn parse_catalog(document: &Value) -> Option<Vec<Value>> {
    if document.get("version").and_then(Value::as_f64) != Some(1.0) {
        return None;
    }
    let connectors = document.get("connectors")?.as_array()?;
    let entries: Vec<Value> = connectors.iter().filter_map(normalize_entry).collect();
    (!entries.is_empty()).then_some(entries)
}

fn normalize_entry(raw: &Value) -> Option<Value> {
    let entry = raw.as_object()?;
    let id = text(entry, "id")?;
    let name = text(entry, "name")?;
    let package = non_empty(entry, "packageName")?;
    let extension = entry.get("kind").and_then(Value::as_str) == Some("extension");
    let adds = extension
        .then(|| manifest::contributes(entry.get("contributes")))
        .flatten();
    if extension && adds.is_none() {
        return None;
    }
    let dropped = [
        "packUrl",
        "sha256",
        "authRung",
        "verified",
        "kind",
        "contributes",
        "permissions",
        "activates",
    ];
    let mut out: Map<String, Value> = entry
        .iter()
        .filter(|(k, _)| !dropped.contains(&k.as_str()))
        .map(|(k, v)| (k.clone(), v.clone()))
        .collect();
    out.insert("id".into(), json!(id));
    out.insert("name".into(), json!(name));
    out.insert("packageName".into(), json!(package));
    out.insert(
        "description".into(),
        json!(text(entry, "description").unwrap_or("")),
    );
    out.insert("capabilities".into(), list(entry.get("capabilities")));
    if extension {
        out.insert("kind".into(), json!("extension"));
    }
    if let Some(adds) = adds {
        out.insert("contributes".into(), json!(adds));
    }
    if extension {
        if let Some(asks) = manifest::permissions(entry.get("permissions")) {
            out.insert("permissions".into(), json!(asks));
        }
        if let Some(shows) = manifest::activation(entry.get("activates")) {
            out.insert("activates".into(), json!(shows));
        }
    }
    if let Some(url) = non_empty(entry, "packUrl") {
        out.insert("packUrl".into(), json!(url));
    }
    if let Some(sha) = non_empty(entry, "sha256") {
        out.insert("sha256".into(), json!(sha));
    }
    if let Some(rung) = text(entry, "authRung").filter(|r| AUTH_RUNGS.contains(r)) {
        out.insert("authRung".into(), json!(rung));
    }
    if let Some(receipt) = verification(entry.get("verified")) {
        out.insert("verified".into(), receipt);
    }
    if entry.contains_key("triggers") {
        out.insert("triggers".into(), summaries(entry.get("triggers")));
    }
    if entry.contains_key("actions") {
        out.insert("actions".into(), actions(entry.get("actions")));
    }
    if entry.contains_key("env") {
        out.insert("env".into(), list(entry.get("env")));
    }
    if entry.contains_key("keywords") {
        out.insert("keywords".into(), list(entry.get("keywords")));
    }
    Some(Value::Object(out))
}

fn verification(raw: Option<&Value>) -> Option<Value> {
    let value = raw?.as_object()?;
    if value.get("schema").and_then(Value::as_f64) != Some(VERIFICATION_SCHEMA as f64) {
        return None;
    }
    let version = non_empty(value, "version")?;
    let checked = non_empty(value, "checkedAt")?;
    Some(json!({
        "schema": VERIFICATION_SCHEMA,
        "version": version,
        "checkedAt": checked,
        "checks": strings(value.get("checks")),
    }))
}

fn named(entry: &Map<String, Value>) -> bool {
    non_empty(entry, "type").is_some()
}

fn summary(entry: &Map<String, Value>) -> Map<String, Value> {
    let kind = text(entry, "type").unwrap_or("").to_owned();
    let mut out: Map<String, Value> = entry
        .iter()
        .filter(|(k, _)| k.as_str() != "description")
        .map(|(k, v)| (k.clone(), v.clone()))
        .collect();
    out.insert("type".into(), json!(kind));
    out.insert(
        "label".into(),
        json!(text(entry, "label").unwrap_or(&kind)),
    );
    if let Some(d) = text(entry, "description") {
        out.insert("description".into(), json!(d));
    }
    out
}

fn summaries(raw: Option<&Value>) -> Value {
    Value::Array(
        records(raw)
            .into_iter()
            .filter(|e| named(e))
            .map(|e| Value::Object(summary(e)))
            .collect(),
    )
}

fn actions(raw: Option<&Value>) -> Value {
    Value::Array(
        records(raw)
            .into_iter()
            .filter(|e| named(e))
            .map(|action| {
                let mut out = summary(action);
                if action.contains_key("inputs") {
                    out.insert("inputs".into(), action_inputs(action.get("inputs")));
                }
                Value::Object(out)
            })
            .collect(),
    )
}

fn action_inputs(raw: Option<&Value>) -> Value {
    Value::Array(
        records(raw)
            .into_iter()
            .filter_map(|input| {
                let key = non_empty(input, "key")?;
                let mut out = Map::new();
                out.insert("key".into(), json!(key));
                out.insert("label".into(), json!(text(input, "label").unwrap_or(key)));
                out.insert("type".into(), json!(text(input, "type").unwrap_or("string")));
                out.insert(
                    "required".into(),
                    json!(input.get("required") == Some(&Value::Bool(true))),
                );
                let options: Vec<Value> = records(input.get("options"))
                    .into_iter()
                    .filter_map(|option| {
                        let value = non_empty(option, "value")?;
                        let mut o = Map::new();
                        o.insert("value".into(), json!(value));
                        if let Some(label) = text(option, "label") {
                            o.insert("label".into(), json!(label));
                        }
                        Some(Value::Object(o))
                    })
                    .collect();
                if !options.is_empty() {
                    out.insert("options".into(), Value::Array(options));
                }
                if let Some(load) = non_empty(input, "loadOptions") {
                    out.insert("loadOptions".into(), json!(load));
                }
                Some(Value::Object(out))
            })
            .collect(),
    )
}

/// `parseTemplates`: the templates a document carries that this build can read.
pub fn parse_templates(document: &Value) -> Vec<Value> {
    document
        .get("templates")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .filter_map(normalize_template)
        .collect()
}

fn normalize_template(raw: &Value) -> Option<Value> {
    let template = raw.as_object()?;
    let id = text(template, "id")?;
    let name = text(template, "name")?;
    let portable = template.get("portable")?.as_object()?;
    if portable.get("version").and_then(Value::as_f64) != Some(PORTABLE_FORMAT_VERSION as f64) {
        return None;
    }
    let nodes = portable.get("nodes")?.as_array()?;
    if nodes.is_empty() || !portable.get("edges").is_some_and(Value::is_array) {
        return None;
    }
    let mut out = template.clone();
    out.insert("id".into(), json!(id));
    out.insert("name".into(), json!(name));
    out.insert(
        "description".into(),
        json!(text(template, "description").unwrap_or("")),
    );
    out.insert("steps".into(), strings(template.get("steps")));
    let mut portable = portable.clone();
    if let Some(requires) = portable.get("requires") {
        let kept = match requires {
            Value::Array(items) => items.iter().filter(|r| usable_requirement(r)).cloned().collect(),
            _ => Vec::new(),
        };
        portable.insert("requires".into(), Value::Array(kept));
    }
    out.insert("portable".into(), Value::Object(portable));
    Some(Value::Object(out))
}

fn usable_requirement(raw: &Value) -> bool {
    let Some(r) = raw.as_object() else {
        return false;
    };
    if !r.get("nodeId").is_some_and(Value::is_string) {
        return false;
    }
    match r.get("kind").and_then(Value::as_str) {
        Some("httpProfile") => r.get("name").is_some_and(Value::is_string),
        Some("connection") => {
            r.get("connectorId").is_some_and(Value::is_string)
                && r.get("name").is_some_and(Value::is_string)
                && r.get("event").is_none_or(Value::is_string)
        }
        _ => false,
    }
}

/// `parseMcpServers`: the MCP servers a document lists that can be started.
pub fn parse_mcp_servers(document: &Value) -> Vec<Value> {
    records(document.get("mcpServers"))
        .into_iter()
        .filter_map(|entry| {
            let id = text(entry, "id")?;
            let name = text(entry, "name")?;
            let command = non_empty(entry, "command")?;
            let mut out = entry.clone();
            out.insert("id".into(), json!(id));
            out.insert("name".into(), json!(name));
            out.insert("command".into(), json!(command));
            out.insert("args".into(), strings(entry.get("args")));
            if entry.contains_key("keywords") {
                out.insert("keywords".into(), strings(entry.get("keywords")));
            }
            if entry.contains_key("env") {
                out.insert("env".into(), strings(entry.get("env")));
            }
            Some(Value::Object(out))
        })
        .collect()
}

/// Where a catalog entry is launched from: a checkout's build when
/// `VORN_CONNECTORS_ROOT` names one, else `npx -y <package>`.
pub fn launch_spec(entry: &Value, repo_root: Option<&Path>) -> Value {
    let package = entry.get("packageName").and_then(Value::as_str).unwrap_or("");
    match local_launch_spec(&local_package_dir(package), repo_root) {
        Some((command, args)) => json!({ "command": command, "args": args }),
        None => json!({ "command": "npx", "args": ["-y", package] }),
    }
}

/// A checkout's build of the connector in `dir_name`, when there is one.
pub fn local_launch_spec(dir_name: &str, repo_root: Option<&Path>) -> Option<(String, Vec<String>)> {
    let local = repo_root?
        .join("packages")
        .join(dir_name)
        .join("dist")
        .join("index.js");
    local
        .exists()
        .then(|| ("node".to_owned(), vec![local.to_string_lossy().into_owned()]))
}

/// `@vornrun/connector-kusto` lives in `packages/kusto`.
fn local_package_dir(package: &str) -> String {
    let rest = match package.strip_prefix('@').and_then(|p| p.split_once('/')) {
        Some((_, rest)) => rest,
        None => return package.to_owned(),
    };
    rest.strip_prefix("connector-").unwrap_or(rest).to_owned()
}

/// What a catalog holds at one moment.
#[derive(Debug, Clone, PartialEq)]
struct Held {
    connectors: Vec<Value>,
    templates: Vec<Value>,
    mcp_servers: Vec<Value>,
    fetched_at: Option<u64>,
}

/// The catalog this process serves, its disk cache and how it fetches.
pub struct Catalog {
    cache: PathBuf,
    repo_root: Option<PathBuf>,
    held: Mutex<Option<Held>>,
}

impl std::fmt::Debug for Catalog {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Catalog").field("cache", &self.cache).finish_non_exhaustive()
    }
}

impl Catalog {
    /// `cache` is `~/.vorn/connector-catalog.json` in the app.
    pub fn new(cache: PathBuf, repo_root: Option<PathBuf>) -> Catalog {
        Catalog {
            cache,
            repo_root,
            held: Mutex::new(None),
        }
    }

    fn held(&self) -> std::sync::MutexGuard<'_, Option<Held>> {
        self.held.lock().unwrap_or_else(|e| e.into_inner())
    }

    fn read_cache(&self) -> Option<Held> {
        let text = std::fs::read_to_string(&self.cache).ok()?;
        let cached: Value = serde_json::from_str(&text).ok()?;
        let connectors =
            parse_catalog(&json!({ "version": 1, "connectors": cached.get("connectors") }))?;
        Some(Held {
            connectors,
            templates: parse_templates(&cached),
            mcp_servers: parse_mcp_servers(&cached),
            fetched_at: Some(
                cached
                    .get("fetchedAt")
                    .and_then(Value::as_f64)
                    .filter(|n| n.is_finite() && *n > 0.0)
                    .map_or(0, |n| n as u64),
            ),
        })
    }

    /// What is known now, and whether it is due a refresh at `now`.
    fn known(&self, now: u64) -> (Held, bool) {
        let mut held = self.held();
        if let Some(h) = held.as_ref() {
            return (h.clone(), false);
        }
        let cache = self.read_cache();
        let stale = cache
            .as_ref()
            .is_none_or(|c| now.saturating_sub(c.fetched_at.unwrap_or(0)) > MAX_AGE_MS);
        let h = cache.unwrap_or_else(|| Held {
            connectors: seed(),
            templates: Vec::new(),
            mcp_servers: Vec::new(),
            fetched_at: None,
        });
        *held = Some(h.clone());
        (h, stale)
    }

    /// `catalogSnapshot`: `{items, templates, mcpServers, fetchedAt?}`, and
    /// whether the caller should refresh it in the background.
    pub fn snapshot(&self, now: u64) -> (Value, bool) {
        let (held, stale) = self.known(now);
        (self.shape(&held), stale)
    }

    fn shape(&self, held: &Held) -> Value {
        let items: Vec<Value> = held
            .connectors
            .iter()
            .map(|entry| {
                let mut item = entry.clone();
                item["launch"] = launch_spec(entry, self.repo_root.as_deref());
                item
            })
            .collect();
        let templates = if held.templates.is_empty() {
            template_seed()
        } else {
            held.templates.clone()
        };
        let mut out = json!({ "items": items, "templates": templates, "mcpServers": held.mcp_servers });
        if let Some(at) = held.fetched_at {
            out["fetchedAt"] = json!(at);
        }
        out
    }

    /// Fetches the published catalog and adopts it if it parses: whether it
    /// was fetched, and whether what is held changed. Blocks; never fails.
    pub fn refresh(&self, fetch: &dyn Fetch, now: u64) -> (bool, bool) {
        let Ok(bytes) = fetch.get(CATALOG_URL, FETCH_TIMEOUT, MAX_DOCUMENT_BYTES, &|_, _| {}) else {
            return (false, false);
        };
        let Ok(document) = serde_json::from_slice::<Value>(&bytes) else {
            return (false, false);
        };
        let Some(connectors) = parse_catalog(&document) else {
            return (false, false);
        };
        let fresh = Held {
            connectors,
            templates: parse_templates(&document),
            mcp_servers: parse_mcp_servers(&document),
            fetched_at: Some(now),
        };
        self.write_cache(&fresh);
        let mut held = self.held();
        let changed = held.as_ref().is_none_or(|h| {
            (&h.connectors, &h.templates, &h.mcp_servers)
                != (&fresh.connectors, &fresh.templates, &fresh.mcp_servers)
        });
        *held = Some(fresh);
        (true, changed)
    }

    fn write_cache(&self, held: &Held) {
        let document = json!({
            "fetchedAt": held.fetched_at,
            "connectors": held.connectors,
            "templates": held.templates,
            "mcpServers": held.mcp_servers,
        });
        if let Some(dir) = self.cache.parent() {
            let _ = std::fs::create_dir_all(dir);
        }
        if let Ok(text) = serde_json::to_string_pretty(&document) {
            let _ = std::fs::write(&self.cache, text);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    struct Serving(Option<Value>);
    impl Fetch for Serving {
        fn get(&self, _: &str, _: Duration, _: u64, _: &dyn Fn(u64, u64)) -> Result<Vec<u8>, String> {
            self.0
                .as_ref()
                .map(|v| v.to_string().into_bytes())
                .ok_or_else(|| "offline".into())
        }
    }

    #[test]
    fn serves_the_seed_then_a_fetched_catalog_and_says_when_it_changed() {
        let dir = tempfile::tempdir().unwrap();
        let catalog = Catalog::new(dir.path().join("catalog.json"), None);
        let (first, stale) = catalog.snapshot(10);
        assert!(stale);
        assert!(first.get("fetchedAt").is_none());
        assert_eq!(first["items"][0]["launch"], json!({ "command": "npx", "args": ["-y", "@vornrun/connector-ado"] }));
        assert!(!first["templates"].as_array().unwrap().is_empty());

        assert_eq!(catalog.refresh(&Serving(None), 20), (false, false));
        let doc = json!({ "version": 1, "connectors": [{ "id": "x", "name": "X", "packageName": "@v/connector-x" }] });
        assert_eq!(catalog.refresh(&Serving(Some(doc.clone())), 30), (true, true));
        assert_eq!(catalog.refresh(&Serving(Some(doc)), 40), (true, false));
        let (now, stale) = catalog.snapshot(50);
        assert!(!stale);
        assert_eq!(now["fetchedAt"], 40);
        assert_eq!(now["items"][0]["description"], "");

        // A later process reads the cache, and only refreshes it once it is old.
        let later = Catalog::new(dir.path().join("catalog.json"), None);
        let (cached, stale) = later.snapshot(40 + MAX_AGE_MS);
        assert!(!stale);
        assert_eq!(cached["items"][0]["id"], "x");
        let older = Catalog::new(dir.path().join("catalog.json"), None);
        assert!(older.snapshot(41 + MAX_AGE_MS).1);
    }

    #[test]
    fn refuses_a_document_it_cannot_read() {
        assert!(parse_catalog(&json!({ "version": 2, "connectors": [] })).is_none());
        assert!(parse_catalog(&json!({ "version": 1, "connectors": [{ "id": "x" }] })).is_none());
        assert!(parse_catalog(&json!(null)).is_none());
    }

    #[test]
    fn finds_a_checkout_build() {
        let dir = tempfile::tempdir().unwrap();
        let built = dir.path().join("packages/kusto/dist");
        std::fs::create_dir_all(&built).unwrap();
        std::fs::write(built.join("index.js"), "").unwrap();
        let entry = json!({ "packageName": "@vornrun/connector-kusto" });
        let spec = launch_spec(&entry, Some(dir.path()));
        assert_eq!(spec["command"], "node");
        assert!(spec["args"][0].as_str().unwrap().ends_with("index.js"));
        assert_eq!(local_package_dir("plain"), "plain");
        assert_eq!(local_package_dir("@s/other"), "other");
    }
}

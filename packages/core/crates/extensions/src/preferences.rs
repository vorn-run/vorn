//! The settings extensions declare (`[[contributes.preferences]]`) and the
//! values a person gave them, kept in one file of the data dir by extension
//! id. An extension reads its own as the rows of [`COLLECTION`], each
//! `{id, value}` with its default until changed; a change is published to
//! those rows, so trees drawn from them draw again at once.
//!
//! The kinds of notice it declares (`[[contributes.notifications]]`) are
//! switches kept here too, beside one switch for all of them: the host draws
//! them, so no extension builds its own, and they go with the extension.

use std::collections::BTreeMap;
use std::path::PathBuf;

use serde_json::{json, Map, Value};
use tracing::warn;

use crate::api::ExtError;
use crate::manifest::{Contribution, Manifest, Preference, PreferenceKind, ALL_NOTICES};

/// The collection an extension reads its preferences from; only the host writes it.
pub const COLLECTION: &str = "preferences";

/// Every extension's preferences a person changed, by extension id then preference id.
#[derive(Debug, Default)]
pub struct Preferences {
    file: Option<PathBuf>,
    values: BTreeMap<String, Map<String, Value>>,
}

/// The preference a kind of notice is kept under; [`ALL_NOTICES`] is the extension-wide switch.
pub fn notice_key(kind: &str) -> String {
    format!("notify:{kind}")
}

/// Its own preferences, then a switch for each kind of notice it declares
/// and, when it declares any, the one for all of them.
fn declared(manifest: &Manifest) -> Vec<Preference> {
    let Some(c) = manifest.contributes.as_ref() else {
        return Vec::new();
    };
    let switch = |id: &str, title: &str, description: Option<String>, default: bool| Preference {
        base: Contribution {
            id: notice_key(id),
            title: title.to_owned(),
            description,
            when: None,
        },
        kind: PreferenceKind::Bool { default },
    };
    let mut out = c.preferences().to_vec();
    out.extend(c.notifications().iter().map(|n| {
        switch(
            &n.base.id,
            &n.base.title,
            n.base.description.clone(),
            n.default,
        )
    }));
    if !c.notifications().is_empty() {
        out.push(switch(ALL_NOTICES, "Notifications", None, true));
    }
    out
}

/// Which notice switch a preference is: `"kind"`, `"all"`, or none.
fn notice_role(id: &str) -> Option<&'static str> {
    match id.strip_prefix("notify:")? {
        ALL_NOTICES => Some("all"),
        _ => Some("kind"),
    }
}

impl Preferences {
    /// What `file` keeps; none or one that does not read is empty. Without a
    /// file, changes last as long as the process.
    pub fn load(file: Option<PathBuf>) -> Preferences {
        let values = file
            .as_ref()
            .and_then(|f| std::fs::read(f).ok())
            .and_then(|b| serde_json::from_slice(&b).ok())
            .unwrap_or_default();
        Preferences { file, values }
    }

    /// Each preference `manifest` declares, by id: what the person chose, or its default.
    pub fn resolved(&self, manifest: &Manifest) -> Map<String, Value> {
        let kept = self.values.get(&manifest.id);
        declared(manifest)
            .iter()
            .map(|p| {
                let value = kept
                    .and_then(|k| k.get(&p.base.id))
                    .filter(|v| p.accepts(v))
                    .cloned()
                    .unwrap_or_else(|| p.default_value());
                (p.base.id.clone(), value)
            })
            .collect()
    }

    /// Its preferences as the rows of [`COLLECTION`]: `{id, value}`.
    pub fn rows(&self, manifest: &Manifest) -> Vec<Value> {
        self.resolved(manifest)
            .into_iter()
            .map(|(id, value)| json!({ "id": id, "value": value }))
            .collect()
    }

    /// Its preferences as Settings draws them: what the manifest says of
    /// each, with its `value`.
    pub fn listed(&self, manifest: &Manifest) -> Vec<Value> {
        let resolved = self.resolved(manifest);
        declared(manifest)
            .iter()
            .map(|p| {
                let mut v = serde_json::to_value(p).unwrap_or_else(|_| json!({}));
                v["value"] = resolved.get(&p.base.id).cloned().unwrap_or(Value::Null);
                if let Some(role) = notice_role(&p.base.id) {
                    v["notifications"] = json!(role);
                }
                v
            })
            .collect()
    }

    /// Keeps `value` for `manifest`'s preference `key`, and gives the row
    /// to publish. Refuses a preference it does not declare and a value of
    /// the wrong type or not among its options.
    pub fn set(&mut self, manifest: &Manifest, key: &str, value: Value) -> Result<Value, ExtError> {
        let declared = declared(manifest);
        let Some(p) = declared.iter().find(|p| p.base.id == key) else {
            return Err(ExtError::new(
                "invalid",
                format!("{} has no preference \"{key}\"", manifest.name),
            ));
        };
        if !p.accepts(&value) {
            let wants = match &p.kind {
                PreferenceKind::Bool { .. } => "true or false".to_owned(),
                PreferenceKind::Choice { options, .. } => {
                    let ids: Vec<&str> = options.iter().map(|o| o.id.as_str()).collect();
                    format!("one of {}", ids.join(", "))
                }
            };
            return Err(ExtError::new(
                "invalid",
                format!("{} takes {wants}", p.base.title),
            ));
        }
        self.values
            .entry(manifest.id.clone())
            .or_default()
            .insert(key.to_owned(), value.clone());
        self.save();
        Ok(json!({ "id": key, "value": value }))
    }

    /// Keeps each of `values` for `extension` that the person has not set:
    /// what was chosen here wins over what is moved in.
    pub fn adopt(&mut self, extension: &str, values: Map<String, Value>) {
        let kept = self.values.entry(extension.to_owned()).or_default();
        let before = kept.len();
        for (key, value) in values {
            kept.entry(key).or_insert(value);
        }
        if kept.is_empty() {
            self.values.remove(extension);
        } else if kept.len() != before {
            self.save();
        }
    }

    /// Forgets what the person chose for `extension`, as it is removed.
    pub fn forget(&mut self, extension: &str) {
        if self.values.remove(extension).is_some() {
            self.save();
        }
    }

    fn save(&self) {
        let Some(file) = &self.file else { return };
        let saved = serde_json::to_vec_pretty(&self.values).unwrap_or_default();
        if let Err(err) = std::fs::write(file, saved) {
            warn!("[extensions] could not keep the extensions' preferences: {err}");
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::manifest::file::parse;

    fn manifest() -> Manifest {
        parse(
            r#"id = "notes"
name = "Notes"
version = "1.0.0"
api = "vorn:extension@1.0"

[service]
command = ["node", "index.js"]

[[contributes.preferences]]
id = "pane"
title = "Notes pane beside cards"
description = "Shows the notes beside a card."
type = "bool"
default = true

[[contributes.preferences]]
id = "sort"
title = "Sort by"
type = "choice"
default = "new"
options = [{ id = "new", title = "Newest" }, { id = "old", title = "Oldest" }]
"#,
        )
        .unwrap()
    }

    #[test]
    fn reads_defaults_until_changed_and_keeps_a_change() {
        let dir = tempfile::tempdir().unwrap();
        let file = dir.path().join("p.json");
        let m = manifest();
        let mut p = Preferences::load(Some(file.clone()));
        assert_eq!(
            Value::Object(p.resolved(&m)),
            json!({ "pane": true, "sort": "new" })
        );

        p.set(&m, "pane", json!(false)).unwrap();
        p.set(&m, "sort", json!("old")).unwrap();
        let again = Preferences::load(Some(file));
        assert_eq!(
            again.rows(&m),
            [
                json!({ "id": "pane", "value": false }),
                json!({ "id": "sort", "value": "old" })
            ]
        );
        let listed = again.listed(&m);
        assert_eq!(listed[0]["title"], "Notes pane beside cards");
        assert_eq!(listed[0]["type"], "bool");
        assert_eq!(listed[0]["default"], true);
        assert_eq!(listed[0]["value"], false);
        assert_eq!(listed[1]["options"][1]["title"], "Oldest");
    }

    #[test]
    fn refuses_what_the_manifest_does_not_declare() {
        let m = manifest();
        let mut p = Preferences::load(None);
        assert!(p.set(&m, "pane", json!("yes")).is_err());
        assert!(p.set(&m, "sort", json!("middle")).is_err());
        assert!(p.set(&m, "other", json!(true)).is_err());
        assert_eq!(p.resolved(&m)["pane"], true);
    }

    #[test]
    fn a_kept_value_the_manifest_no_longer_takes_reads_as_the_default() {
        let mut p = Preferences::load(None);
        p.values
            .entry("notes".into())
            .or_default()
            .insert("sort".into(), json!("gone"));
        assert_eq!(p.resolved(&manifest())["sort"], "new");
    }

    #[test]
    fn adopting_moved_values_keeps_what_the_person_already_chose() {
        let dir = tempfile::tempdir().unwrap();
        let file = dir.path().join("p.json");
        let m = manifest();
        let mut p = Preferences::load(Some(file.clone()));
        p.set(&m, "sort", json!("new")).unwrap();
        let mut moved = Map::new();
        moved.insert("pane".into(), json!(false));
        moved.insert("sort".into(), json!("old"));
        p.adopt("notes", moved);
        assert_eq!(
            Value::Object(Preferences::load(Some(file)).resolved(&m)),
            json!({ "pane": false, "sort": "new" })
        );
        p.adopt("other", Map::new());
        assert!(!p.values.contains_key("other"));
    }

    #[test]
    fn notice_kinds_are_switches_beside_one_for_all_and_go_with_the_extension() {
        let m = parse(
            r#"id = "flows"
name = "Flows"
version = "1.0.0"
api = "vorn:extension@1.0"
[service]
command = ["x"]
[[contributes.notifications]]
id = "finished"
title = "Notify when a run finishes"
description = "A workflow run ended."
[[contributes.notifications]]
id = "approval"
title = "Notify when a run needs approval"
default = false
"#,
        )
        .unwrap();
        let mut p = Preferences::load(None);
        let listed = p.listed(&m);
        let row = |id: &str| listed.iter().find(|r| r["id"] == id).cloned().unwrap();
        assert_eq!(
            row("notify:finished")["title"],
            "Notify when a run finishes"
        );
        assert_eq!(row("notify:finished")["value"], true);
        assert_eq!(
            row("notify:finished")["description"],
            "A workflow run ended."
        );
        assert_eq!(row("notify:finished")["notifications"], "kind");
        assert_eq!(row("notify:approval")["value"], false);
        assert_eq!(row("notify:all")["notifications"], "all");
        assert_eq!(row("notify:all")["value"], true);
        p.set(&m, "notify:approval", Value::Bool(true)).unwrap();
        assert_eq!(p.resolved(&m)["notify:approval"], true);
        assert!(p.set(&m, "notify:other", Value::Bool(true)).is_err());
        p.forget("flows");
        assert_eq!(p.resolved(&m)["notify:approval"], false);
    }

    #[test]
    fn a_manifest_without_notices_has_no_notice_switches() {
        let listed = Preferences::load(None).listed(&manifest());
        assert!(listed.iter().all(|r| r.get("notifications").is_none()));
    }
}

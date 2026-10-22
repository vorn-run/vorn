//! The settings extensions declare (`[[contributes.preferences]]`) and the
//! values a person gave them, kept in one file of the data dir by extension
//! id. An extension reads its own as the rows of [`COLLECTION`], each
//! `{id, value}` with its default until changed; a change is published to
//! those rows, so trees drawn from them draw again at once.

use std::collections::BTreeMap;
use std::path::PathBuf;

use serde_json::{json, Map, Value};
use tracing::warn;

use crate::api::ExtError;
use crate::manifest::{Manifest, Preference, PreferenceKind};

/// The collection an extension reads its preferences from; only the host writes it.
pub const COLLECTION: &str = "preferences";

/// Every extension's preferences a person changed, by extension id then preference id.
#[derive(Debug, Default)]
pub struct Preferences {
    file: Option<PathBuf>,
    values: BTreeMap<String, Map<String, Value>>,
}

fn declared(manifest: &Manifest) -> &[Preference] {
    manifest
        .contributes
        .as_ref()
        .map(|c| c.preferences())
        .unwrap_or_default()
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
                v
            })
            .collect()
    }

    /// Keeps `value` for `manifest`'s preference `key`, and gives the row
    /// to publish. Refuses a preference it does not declare and a value of
    /// the wrong type or not among its options.
    pub fn set(&mut self, manifest: &Manifest, key: &str, value: Value) -> Result<Value, ExtError> {
        let Some(p) = declared(manifest).iter().find(|p| p.base.id == key) else {
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
}

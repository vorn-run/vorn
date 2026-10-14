//! Sidebar sections from extensions in two tiers: what vornd's `ext:section`
//! notes say, checked again here. The client never learns how an extension
//! runs beyond the label it shows; it gets a template and rows, both JSON,
//! and draws them, or draws what is wrong with them.

use std::collections::BTreeMap;

use serde_json::Value;
use vorn_ext::template::{self, Row, Template};

/// One section, ready to draw.
#[derive(Debug, Clone, PartialEq)]
pub struct Section {
    pub title: String,
    /// A quick extension runs as the person, outside any sandbox.
    pub local: bool,
    /// The template and its rows, or every error found in either.
    pub body: Result<(Template, Vec<Row>), Vec<String>>,
    /// The extension's own failure (a timeout, gh not signed in).
    pub error: Option<String>,
}

/// Every section vornd told, by its key.
#[derive(Debug, Default)]
pub struct Sections {
    pub all: BTreeMap<String, Section>,
    /// `ext:list` was asked once.
    pub loaded: bool,
}

impl Sections {
    /// Takes one `ext:section` note.
    pub fn take(&mut self, note: &Value) {
        if note["removed"] == true {
            let dir = note["dir"].as_str().unwrap_or_default();
            self.all.retain(|k, _| !k.starts_with(dir));
            return;
        }
        let Some(key) = note["key"].as_str() else {
            return;
        };
        self.all.insert(key.to_owned(), section(note));
    }

    /// Takes `ext:list`'s answer.
    pub fn take_all(&mut self, notes: &Value) {
        self.loaded = true;
        for n in notes.as_array().into_iter().flatten() {
            self.take(n);
        }
    }
}

fn lines(errs: &[template::TemplateError]) -> Vec<String> {
    errs.iter().map(ToString::to_string).collect()
}

fn section(note: &Value) -> Section {
    let body = if let Some(errs) = note["templateErrors"].as_array() {
        Err(errs
            .iter()
            .map(|e| {
                format!(
                    "{}: {}: {}",
                    e["where"].as_str().unwrap_or_default(),
                    e["path"].as_str().unwrap_or_default(),
                    e["message"].as_str().unwrap_or_default()
                )
            })
            .collect())
    } else {
        template::parse_value(&note["template"])
            .map_err(|e| lines(&e))
            .and_then(|t| {
                let rows = match &note["rows"] {
                    Value::Null => Ok(Vec::new()),
                    rows => t.rows.check(rows).map_err(|e| lines(&e)),
                };
                rows.map(|r| (t, r))
            })
    };
    Section {
        title: note["title"]
            .as_str()
            .or(note["name"].as_str())
            .unwrap_or("Extension")
            .to_owned(),
        local: note["tier"] == "quick",
        body,
        error: note["error"].as_str().map(str::to_owned),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn template() -> Value {
        json!({ "schema": "vorn:view@0.1", "rows": { "name": "string" },
                "view": { "list": { "item": { "text": { "value": "{row.name}" } } } } })
    }

    #[test]
    fn keeps_a_valid_section_and_removes_a_folders_sections() {
        let mut s = Sections::default();
        s.take(
            &json!({ "key": "/x#checks", "dir": "/x", "title": "PR checks", "tier": "quick",
                        "template": template(), "rows": [{ "id": "1", "name": "ci" }] }),
        );
        let sec = &s.all["/x#checks"];
        assert!(sec.local);
        assert_eq!(sec.body.as_ref().unwrap().1.len(), 1);
        s.take(&json!({ "dir": "/x", "removed": true }));
        assert!(s.all.is_empty());
    }

    #[test]
    fn a_row_the_template_does_not_declare_is_an_error_not_a_blank() {
        let mut s = Sections::default();
        s.take(
            &json!({ "key": "k", "template": template(), "rows": [{ "id": "1", "nmae": "ci" }] }),
        );
        assert_eq!(
            s.all["k"].body.as_ref().unwrap_err(),
            &["rows[0].nmae: the template declares no field `nmae`"]
        );
    }

    #[test]
    fn keeps_the_places_vornd_found_in_the_file() {
        let mut s = Sections::default();
        s.take(&json!({ "key": "k", "templateErrors": [{ "where": "views/checks.kdl:16:13", "path": "view.buton", "message": "unknown node `buton`" }] }));
        assert_eq!(
            s.all["k"].body.as_ref().unwrap_err(),
            &["views/checks.kdl:16:13: view.buton: unknown node `buton`"]
        );
    }
}

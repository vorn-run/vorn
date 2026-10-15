//! An extension's templates, read from its folder: checked at install and
//! pack (a bad one refuses the package), and when a linked folder changes
//! (a bad one draws its error where it would have shown).

use std::fs;
use std::path::Path;

use serde_json::{json, Value};
use vorn_view::{file, Options, TemplateError};

use crate::manifest::{Manifest, PaneDraws, Place};

/// Template files are small; past this one is a mistake.
const MAX_TEMPLATE_BYTES: u64 = 256 * 1024;

/// Reads and checks template `path` (as `views/checks.kdl`) of the package in `dir`.
pub fn load(dir: &Path, path: &str, place: Place) -> Loaded {
    let one = |message: String| {
        vec![TemplateError {
            file: Some(path.to_owned()),
            at: None,
            path: "$".into(),
            message,
        }]
    };
    let full = dir.join(path);
    let size = fs::metadata(&full).map_err(|e| one(format!("cannot read it: {e}")))?;
    if size.len() > MAX_TEMPLATE_BYTES {
        return Err(one(format!(
            "is {} KB; a template is {} KB at most",
            size.len() / 1024,
            MAX_TEMPLATE_BYTES / 1024
        )));
    }
    let text = fs::read_to_string(&full).map_err(|e| one(format!("cannot read it: {e}")))?;
    read(path, &text, place)
}

/// A checked template, or every error found in it.
pub type Loaded = Result<Value, Vec<TemplateError>>;

/// Checks template text `text`, read from `path`, for `place`.
pub fn read(path: &str, text: &str, place: Place) -> Loaded {
    let opts = Options {
        session_row: place.per_session(),
    };
    file::to_json(path, text, opts)
}

/// Every contribution of `manifest` that a client draws or lists from a
/// template, flat and in place order: `{extension, kind, id, title, ..., view, errors}`,
/// `view` the checked template or null with its `errors`. `load` reads one template.
pub fn listed(manifest: &Manifest, load: &dyn Fn(&str, Place) -> Loaded) -> Vec<Value> {
    let Some(c) = &manifest.contributes else {
        return Vec::new();
    };
    let entry = |place: Place, mut v: Value, view: Option<&str>| {
        let (template, errors) = match view.map(|p| load(p, place)) {
            None => (Value::Null, Vec::new()),
            Some(Ok(t)) => (t, Vec::new()),
            Some(Err(e)) => (Value::Null, e.iter().map(ToString::to_string).collect()),
        };
        v["extension"] = json!(manifest.id);
        v["kind"] = json!(place.key());
        v["view"] = template;
        v["errors"] = json!(errors);
        v
    };
    let mut out = Vec::new();
    for pane in c.panes() {
        if let PaneDraws::View(path) = &pane.draws {
            let v = json!({ "id": pane.base.id, "title": pane.base.title });
            out.push(entry(Place::Panes, v, Some(path)));
        }
    }
    for place in Place::POINTS {
        for point in c.points(place) {
            let v = serde_json::to_value(point).unwrap_or_else(|_| json!({}));
            out.push(entry(place, v, point.view.as_deref()));
        }
    }
    out
}

/// Every template's errors, each as `views/x.kdl:3:5: what is wrong`.
pub fn check(manifest: &Manifest, dir: &Path) -> Vec<String> {
    let Some(c) = &manifest.contributes else {
        return Vec::new();
    };
    let mut seen = Vec::new();
    let mut errors = Vec::new();
    for (place, path) in c.views() {
        // A template shared by two places is read under the stricter.
        if seen.contains(&(path, place.per_session())) {
            continue;
        }
        seen.push((path, place.per_session()));
        if let Err(e) = load(dir, path, place) {
            errors.extend(e.iter().map(ToString::to_string));
        }
    }
    errors
}

/// [`check`] as one refusal, for install and pack.
pub fn refuse(manifest: &Manifest, dir: &Path) -> Result<(), String> {
    let errors = check(manifest, dir);
    if errors.is_empty() {
        return Ok(());
    }
    Err(format!(
        "{} has {} template error{}:\n{}",
        manifest.name,
        errors.len(),
        if errors.len() == 1 { "" } else { "s" },
        errors.join("\n")
    ))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::manifest::file::parse;

    const MANIFEST: &str = r#"
id = "checks"
name = "Checks"
version = "1.0.0"
api = "vorn:extension@0.9"

[[contributes.sidebar]]
id = "checks"
title = "Checks"
view = "views/checks.kdl"

[[contributes.card_footers]]
id = "branch"
view = "views/footer.kdl"
"#;

    #[test]
    fn lists_each_contribution_with_its_template_or_its_errors() {
        let m = parse(MANIFEST).unwrap();
        let listed = listed(&m, &|path, place| {
            if path.ends_with("footer.kdl") {
                read(
                    path,
                    "schema \"vorn:view@1\"\nrows { branch string; }\ntext \"{row.branch}\"\n",
                    place,
                )
            } else {
                read(path, "schema \"vorn:view@1\"\nbuton Go\n", place)
            }
        });
        assert_eq!(listed.len(), 2);
        assert_eq!(listed[0]["kind"], "sidebar");
        assert_eq!(listed[0]["view"], Value::Null);
        assert_eq!(
            listed[0]["errors"],
            json!(["views/checks.kdl:2:1: unknown node 'buton'; did you mean 'button'?"])
        );
        assert_eq!(listed[1]["kind"], "cardFooters");
        assert_eq!(listed[1]["extension"], "checks");
        assert_eq!(listed[1]["view"]["view"]["text"]["value"], "{row.branch}");
    }

    #[test]
    fn checks_every_template_a_package_draws() {
        let dir = tempfile::tempdir().unwrap();
        let m = parse(MANIFEST).unwrap();
        fs::create_dir(dir.path().join("views")).unwrap();
        fs::write(
            dir.path().join("views/checks.kdl"),
            "schema \"vorn:view@1\"\ncolumn {\n    buton Go call=go\n}\n",
        )
        .unwrap();
        let errors = check(&m, dir.path());
        assert_eq!(
            errors,
            [
                "views/checks.kdl:3:5: unknown node 'buton'; did you mean 'button'?",
                "views/footer.kdl: $: cannot read it: No such file or directory (os error 2)",
            ]
        );
        assert!(refuse(&m, dir.path())
            .unwrap_err()
            .starts_with("Checks has 2 template errors:\n"));
        fs::write(
            dir.path().join("views/checks.kdl"),
            "schema \"vorn:view@1\"\nbutton Go call=go\n",
        )
        .unwrap();
        fs::write(
            dir.path().join("views/footer.kdl"),
            "schema \"vorn:view@1\"\nrows { branch string; }\ntext \"{row.branch}\"\n",
        )
        .unwrap();
        assert_eq!(refuse(&m, dir.path()), Ok(()));
    }
}

//! An extension's templates, read from its folder: checked at install and
//! pack (a bad one refuses the package), and when a linked folder changes
//! (a bad one draws its error where it would have shown).

use std::fs;
use std::path::Path;

use serde_json::Value;
use vorn_view::{file, Options, TemplateError};

use crate::manifest::{Manifest, Place};

/// Template files are small; past this one is a mistake.
const MAX_TEMPLATE_BYTES: u64 = 256 * 1024;

/// Reads and checks template `path` (as `views/checks.kdl`) of the package in `dir`.
pub fn load(dir: &Path, path: &str, place: Place) -> Result<Value, Vec<TemplateError>> {
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
    let opts = Options {
        session_row: place.per_session(),
    };
    file::to_json(path, &text, opts)
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

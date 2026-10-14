//! `vorn ext promote`: scaffolds a Rust crate from a quick extension.
//!
//! The crate gets one handler stub per method the manifest names, a row type
//! per section from the fields its template declares, the views carried over
//! unchanged, and a manifest whose runtime is both `cargo run` (so the half
//! ported extension still runs, natively, over stdio) and the component
//! `pack` will build. Promotion changes where code runs, not what it says.

use std::fmt::Write as _;
use std::path::Path;

use crate::manifest::{Manifest, FILE};
use crate::template::FieldType;
use crate::view_file;
use crate::ExtError;

/// What was written, for the command to print.
#[derive(Debug, Default)]
pub struct Promoted {
    pub files: Vec<String>,
}

/// Scaffolds the crate for the extension in `from` into `to`, with the SDK at `sdk`.
pub fn promote(from: &Path, to: &Path, sdk: &Path) -> Result<Promoted, ExtError> {
    let m = Manifest::load(from)?;
    if to.join("Cargo.toml").exists() {
        return Err(ExtError::new(
            "exists",
            format!("{} already holds a crate", to.display()),
        ));
    }
    let io = |e: std::io::Error| ExtError::new("io", e.to_string());
    let crate_name = m.id.rsplit('.').next().unwrap_or(&m.id).replace('_', "-");
    let ty = pascal(&crate_name);
    let mut out = Promoted::default();
    let mut write = |rel: &str, text: &str| -> Result<(), ExtError> {
        let p = to.join(rel);
        if let Some(dir) = p.parent() {
            std::fs::create_dir_all(dir).map_err(io)?;
        }
        std::fs::write(&p, text).map_err(io)?;
        out.files.push(rel.to_owned());
        Ok(())
    };

    let mut types = String::new();
    let mut arms = String::new();
    let mut handlers = String::new();
    let was = m
        .runtime
        .command
        .as_ref()
        .map(|c| c.join(" "))
        .unwrap_or_default();
    for s in &m.contributes.sidebar {
        let text = std::fs::read_to_string(from.join(&s.view)).map_err(io)?;
        let t = view_file::parse(&text).map_err(|errs| {
            let first = errs.first().map(ToString::to_string).unwrap_or_default();
            ExtError::new("template", format!("{}: {first}", s.view))
        })?;
        let row = format!("{}Row", pascal(&s.id));
        let _ = writeln!(
            types,
            "/// One row of the `{}` section, as `{}` declares it.",
            s.id, s.view
        );
        let _ = writeln!(types, "#[derive(Debug, Clone, PartialEq, Serialize)]\npub struct {row} {{\n    pub id: String,");
        let mut enums = String::new();
        for (field, ft) in &t.rows.fields {
            let rust = match ft {
                FieldType::String | FieldType::Url => "String".to_owned(),
                FieldType::Number => "f64".to_owned(),
                FieldType::Bool => "bool".to_owned(),
                FieldType::Enum(values) => {
                    let name = format!("{}{}", pascal(&s.id), pascal(field));
                    let _ = writeln!(enums, "\n#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]\n#[serde(rename_all = \"lowercase\")]\npub enum {name} {{");
                    for v in values {
                        let _ = writeln!(enums, "    {},", pascal(v));
                    }
                    enums.push_str("}\n");
                    name
                }
            };
            let _ = writeln!(types, "    pub {}: {rust},", field.replace('-', "_"));
        }
        types.push_str("}\n");
        types.push_str(&enums);
        types.push('\n');
        let f = s.rows.replace(['.', '-'], "_");
        let _ = writeln!(
            arms,
            "            \"{}\" => {f}(args, ctx).map(|rows| json!(rows)),",
            s.rows
        );
        let _ = writeln!(
            handlers,
            "/// The rows of `{}`. Was: `{was}`.\nfn {f}(_args: Value, _ctx: &Ctx) -> Result<Vec<{row}>, Error> {{\n    Err(Error::new(\"unimplemented\", \"port `{}` from `{was}`\"))\n}}\n",
            s.id, s.rows
        );
        write(&s.view, &text)?;
    }

    write(
        "Cargo.toml",
        &format!(
            r#"[package]
name = "{crate_name}"
version = "{version}"
edition = "2021"
publish = false

[dependencies]
vorn-extension = {{ path = "{sdk}" }}
serde = {{ version = "1", features = ["derive"] }}

# What `vorn ext pack` builds the component with.
[profile.release]
opt-level = "s"
lto = true
codegen-units = 1
panic = "abort"
strip = true

[workspace]
"#,
            version = m.version,
            sdk = sdk.display()
        ),
    )?;
    write(
        "src/lib.rs",
        &format!(
            "//! {name}, promoted from a quick extension. Calls reach Vorn only through\n//! `host` and `net`; a `curl` becomes a `net::fetch` to a host the manifest lists.\n\nuse serde::Serialize;\nuse vorn_extension::{{json, Ctx, Error, Extension, Value}};\n\n{types}pub struct {ty};\n\nimpl Extension for {ty} {{\n    fn handle(method: &str, args: Value, ctx: &Ctx) -> Result<Value, Error> {{\n        match method {{\n{arms}            other => Err(Error::new(\"unknown-method\", format!(\"{name} has no {{other}}\"))),\n        }}\n    }}\n}}\n\n{handlers}vorn_extension::export!({ty});\n",
            name = m.name
        ),
    )?;
    write(
        "src/main.rs",
        &format!(
            "//! The native transport: `cargo run` speaks the same calls over stdio.\n\nfn main() {{\n    vorn_extension::serve::<{lib}::{ty}>();\n}}\n",
            lib = crate_name.replace('-', "_")
        ),
    )?;
    let mut manifest = std::fs::read_to_string(from.join(FILE)).map_err(io)?;
    let runtime = format!(
        "[runtime]\n# Natively over stdio while you port; the component once `vorn ext pack` built it.\ncommand = [\"cargo\", \"run\", \"--quiet\"]\nservice = true\ncomponent = \"{crate_name}.cwasm\"\n"
    );
    manifest = replace_table(&manifest, "runtime", &runtime);
    // Above the first table, so the keys stay top-level once uncommented.
    let hint = "# The sandbox allows only what is declared here: the hosts `net::fetch` reaches,\n# the permissions `host::call` needs, and where vornd finds the credential.\n# network = [\"api.example.com\"]\n# permissions = [\"git.read\"]\n# auth = [{ rung = \"cli\", borrow = [\"tool\", \"auth\", \"token\"], header = \"Authorization: Bearer {token}\" }]\n\n";
    let first_table = manifest.find("\n[").map_or(manifest.len(), |i| i + 1);
    manifest.insert_str(first_table, hint);
    write(FILE, &manifest)?;
    write(".gitignore", "target/\n*.cwasm\n*.wasm\n")?;
    Ok(out)
}

/// `text` with the `[name]` table (to the next header) replaced by `with`.
fn replace_table(text: &str, name: &str, with: &str) -> String {
    let header = format!("[{name}]");
    let Some(start) = text.find(&header) else {
        return format!("{text}\n{with}");
    };
    let rest = &text[start + header.len()..];
    let end = rest
        .find("\n[")
        .map_or(text.len(), |i| start + header.len() + i + 1);
    format!("{}{with}\n{}", &text[..start], &text[end..])
}

fn pascal(s: &str) -> String {
    s.split(|c: char| !c.is_ascii_alphanumeric())
        .filter(|w| !w.is_empty())
        .map(|w| {
            let mut c = w.chars();
            c.next()
                .map(|f| f.to_ascii_uppercase().to_string() + c.as_str())
                .unwrap_or_default()
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn scaffolds_a_crate_whose_manifest_still_parses() {
        let from = tempfile::tempdir().unwrap();
        std::fs::create_dir(from.path().join("views")).unwrap();
        std::fs::write(
            from.path().join(FILE),
            "id = \"local.pr-checks\"\nname = \"PR checks\"\nversion = \"0.1.0\"\napi = \"vorn:extension@0.9\"\n\n[runtime]\ncommand = [\"bash\", \"run.sh\"]\n\n[[contributes.sidebar]]\nid = \"checks\"\ntitle = \"PR checks\"\nview = \"views/checks.kdl\"\nrows = \"rows\"\n",
        )
        .unwrap();
        std::fs::write(
            from.path().join("views/checks.kdl"),
            "schema \"vorn:view@0.1\"\nrows {\n    name string\n    state enum pass fail\n}\nlist {\n    text \"{row.name}\"\n}\n",
        )
        .unwrap();
        let to = tempfile::tempdir().unwrap();
        promote(from.path(), to.path(), Path::new("../sdk")).unwrap();
        let lib = std::fs::read_to_string(to.path().join("src/lib.rs")).unwrap();
        assert!(lib.contains("pub struct ChecksRow {\n    pub id: String,\n    pub name: String,\n    pub state: ChecksState,\n}"), "{lib}");
        assert!(
            lib.contains("pub enum ChecksState {\n    Pass,\n    Fail,\n}"),
            "{lib}"
        );
        assert!(lib.contains("\"rows\" => rows(args, ctx)"), "{lib}");
        let m = Manifest::load(to.path()).unwrap();
        assert_eq!(
            m.runtime.command.as_deref(),
            Some(&["cargo".to_owned(), "run".into(), "--quiet".into()][..])
        );
        assert_eq!(m.runtime.component.as_deref(), Some("pr-checks.cwasm"));
        assert!(m.runtime.service);
    }

    #[test]
    fn names_types_in_pascal_case() {
        assert_eq!(pascal("pr-checks"), "PrChecks");
        assert_eq!(pascal("skipping"), "Skipping");
    }
}

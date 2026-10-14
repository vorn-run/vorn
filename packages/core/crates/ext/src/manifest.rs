//! The one manifest both tiers share. Only `runtime` says which tier an
//! extension runs in; contributions, permissions and templates are the same.
//!
//! The manifest is TOML a person writes, `vorn-extension.toml`, like a
//! `Cargo.toml`. Parsing is strict: an unknown field is an error, so a
//! misspelt key never turns into a feature silently switched off, and every
//! error names the file's line and the field.

use std::path::Path;
use std::time::Duration;

use serde::{Deserialize, Serialize};

use crate::doc::{line_of, Doc};
use crate::ExtError;

/// The manifest's file name in an extension's folder.
pub const FILE: &str = "vorn-extension.toml";

/// The API this build of Vorn speaks.
pub const API: &str = "vorn:extension@0.9";

/// `vorn-extension.toml`, as parsed and checked.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Manifest {
    pub id: String,
    pub name: String,
    pub version: String,
    pub api: String,
    pub runtime: Runtime,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub permissions: Vec<String>,
    /// Hosts `net.fetch` may reach. A quick extension can reach anything; this
    /// is what the install sheet shows and what the safe tier is held to.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub network: Vec<String>,
    /// Where vornd finds the credential it adds to `net.fetch`, in order.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub auth: Vec<AuthRung>,
    #[serde(default)]
    pub contributes: Contributes,
    /// The recorded checks `pack` runs against every tier.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub checks: Option<String>,
}

/// Where the code runs. Both may be named while an extension is promoted.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Runtime {
    /// The quick tier: an argv run in the extension's folder, as the person.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub command: Option<Vec<String>>,
    /// One long-lived process over stdio JSON-RPC instead of one per call.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub service: bool,
    /// The safe tier: a precompiled component (`.cwasm`), relative to the folder.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub component: Option<String>,
    /// The component's memory cap, as `"64MB"`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub memory: Option<String>,
}

/// One way to get the credential `net.fetch` adds as a header.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "rung", rename_all = "lowercase", deny_unknown_fields)]
pub enum AuthRung {
    /// A field of the extension's own connection in the vault.
    Vault { field: String, header: String },
    /// An environment variable of vornd's, used only when `env:<NAME>` is a permission.
    Env { env: String, header: String },
    /// A CLI's login: vornd runs the argv once and keeps the answer in the vault.
    Cli { borrow: Vec<String>, header: String },
}

impl AuthRung {
    /// The header template, with `{token}` where the credential goes.
    pub fn header(&self) -> &str {
        match self {
            AuthRung::Vault { header, .. }
            | AuthRung::Env { header, .. }
            | AuthRung::Cli { header, .. } => header,
        }
    }
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Contributes {
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub sidebar: Vec<Section>,
}

/// A sidebar section: a template, and the method whose answer is its rows.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Section {
    pub id: String,
    pub title: String,
    /// The template, relative to the folder.
    pub view: String,
    /// The method answering the rows.
    pub rows: String,
    /// How often the rows are asked for again, as `"30s"`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub every: Option<String>,
}

/// Which tier runs an extension.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Tier {
    /// Runs as the person, outside any sandbox.
    Quick,
    /// A component inside vornd's sandbox.
    Safe,
}

impl Manifest {
    /// Reads and checks `<dir>/vorn-extension.toml`.
    pub fn load(dir: &Path) -> Result<Manifest, ExtError> {
        let path = dir.join(FILE);
        let text = std::fs::read_to_string(&path).map_err(|e| {
            ExtError::new("manifest", format!("cannot read {}: {e}", path.display()))
        })?;
        Manifest::parse(&text)
    }

    /// Parses and checks a manifest's TOML text.
    pub fn parse(text: &str) -> Result<Manifest, ExtError> {
        let doc = Doc::parse(text)
            .map_err(|e| ExtError::new("manifest", format!("{FILE}:{}: {}", e.line, e.message)))?;
        let m: Manifest = toml::from_str(text).map_err(|e| {
            let line = e.span().map_or(1, |s| line_of(text, s.start));
            ExtError::new("manifest", format!("{FILE}:{line}: {}", e.message()))
        })?;
        m.check(&doc)?;
        Ok(m)
    }

    fn check(&self, doc: &Doc) -> Result<(), ExtError> {
        let bad = |path: &str, why: String| {
            let line = doc.line(path).unwrap_or(1);
            Err(ExtError::new(
                "manifest",
                format!("{FILE}:{line}: {path}: {why}"),
            ))
        };
        if self.api != API {
            return bad(
                "api",
                format!(
                    "this Vorn speaks {API}; the extension asks for {}",
                    self.api
                ),
            );
        }
        let valid_id = |s: &str| {
            !s.is_empty()
                && s.chars()
                    .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || "-._".contains(c))
        };
        if !valid_id(&self.id) {
            return bad(
                "id",
                format!("`{}` may hold only a-z, 0-9, `-`, `.` and `_`", self.id),
            );
        }
        match (&self.runtime.command, &self.runtime.component) {
            (None, None) => {
                return bad("runtime", "names neither a command nor a component".into())
            }
            (Some(argv), _) if argv.is_empty() => return bad("runtime.command", "is empty".into()),
            _ => {}
        }
        if let Some(mem) = &self.runtime.memory {
            if parse_size(mem).is_none() {
                return bad("runtime.memory", format!("`{mem}` is not a size like 64MB"));
            }
        }
        for (i, p) in self.permissions.iter().enumerate() {
            if !known_permission(p) {
                return bad(
                    &format!("permissions[{i}]"),
                    format!("unknown permission `{p}`"),
                );
            }
        }
        for (i, rung) in self.auth.iter().enumerate() {
            if !rung.header().contains("{token}") {
                return bad(&format!("auth[{i}].header"), "has no {token}".into());
            }
            if let AuthRung::Env { env, .. } = rung {
                if !self.permissions.iter().any(|p| p == &format!("env:{env}")) {
                    return bad(
                        &format!("auth[{i}].env"),
                        format!("reads ${env} but `env:{env}` is not a permission"),
                    );
                }
            }
        }
        for (i, s) in self.contributes.sidebar.iter().enumerate() {
            if let Some(every) = &s.every {
                if parse_duration(every).is_none() {
                    return bad(
                        &format!("contributes.sidebar[{i}].every"),
                        format!("`{every}` is not like 30s"),
                    );
                }
            }
        }
        Ok(())
    }

    /// The tier that runs when the extension is linked: the component when it
    /// is built and `dev` is not asked for, else the command.
    pub fn tier(&self, dir: &Path, dev: bool) -> Tier {
        match (&self.runtime.component, &self.runtime.command) {
            (Some(c), Some(_)) if dev || !dir.join(c).exists() => Tier::Quick,
            (Some(_), _) => Tier::Safe,
            (None, _) => Tier::Quick,
        }
    }

    /// The component's memory cap in bytes; 64 MB unless the manifest says.
    pub fn memory_cap(&self) -> usize {
        self.runtime
            .memory
            .as_deref()
            .and_then(parse_size)
            .unwrap_or(64 << 20)
    }

    pub fn allows(&self, permission: &str) -> bool {
        self.permissions.iter().any(|p| p == permission)
    }
}

fn known_permission(p: &str) -> bool {
    matches!(p, "git.read" | "storage" | "secrets.read")
        || p.strip_prefix("env:").is_some_and(|n| {
            !n.is_empty() && n.chars().all(|c| c.is_ascii_alphanumeric() || c == '_')
        })
}

/// `"30s"`, `"5m"`, `"500ms"`.
pub fn parse_duration(s: &str) -> Option<Duration> {
    let (n, unit) = s.split_at(s.find(|c: char| !c.is_ascii_digit())?);
    let n: u64 = n.parse().ok()?;
    match unit {
        "ms" => Some(Duration::from_millis(n)),
        "s" => Some(Duration::from_secs(n)),
        "m" => Some(Duration::from_secs(n * 60)),
        _ => None,
    }
}

/// `"64MB"`, `"512KB"`.
pub fn parse_size(s: &str) -> Option<usize> {
    let (n, unit) = s.split_at(s.find(|c: char| !c.is_ascii_digit())?);
    let n: usize = n.parse().ok()?;
    match unit {
        "KB" => n.checked_mul(1 << 10),
        "MB" => n.checked_mul(1 << 20),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const QUICK: &str = r#"id = "local.pr-checks"
name = "PR checks"
version = "0.1.0"
api = "vorn:extension@0.9"

[runtime]
command = ["bash", "run.sh"]

[[contributes.sidebar]]
id = "checks"
title = "PR checks"
view = "views/checks.kdl"
rows = "rows"
every = "30s"
"#;

    #[test]
    fn reads_a_quick_manifest() {
        let m = Manifest::parse(QUICK).unwrap();
        assert_eq!(m.tier(Path::new("/nowhere"), false), Tier::Quick);
        assert_eq!(m.contributes.sidebar[0].rows, "rows");
    }

    #[test]
    fn names_the_line_of_a_misspelt_field() {
        let err = Manifest::parse(&QUICK.replace("title =", "titel =")).unwrap_err();
        assert!(
            err.message
                .starts_with("vorn-extension.toml:11: unknown field `titel`"),
            "{err}"
        );
        let err =
            Manifest::parse(&QUICK.replace("every = \"30s\"", "every = \"30 s\"")).unwrap_err();
        assert_eq!(
            err.message,
            "vorn-extension.toml:14: contributes.sidebar[0].every: `30 s` is not like 30s"
        );
    }

    #[test]
    fn refuses_an_env_rung_without_its_permission() {
        let auth = "\n[[auth]]\nrung = \"env\"\nenv = \"GH_TOKEN\"\nheader = \"Authorization: Bearer {token}\"\n";
        let err = Manifest::parse(&format!("{QUICK}{auth}")).unwrap_err();
        assert_eq!(err.message, "vorn-extension.toml:18: auth[0].env: reads $GH_TOKEN but `env:GH_TOKEN` is not a permission");
        let granted = QUICK.replace("[runtime]", "permissions = [\"env:GH_TOKEN\"]\n\n[runtime]");
        Manifest::parse(&format!("{granted}{auth}")).unwrap();
    }

    #[test]
    fn picks_the_command_while_the_component_is_not_built() {
        let dir = tempfile::tempdir().unwrap();
        let text = QUICK.replace(
            r#"command = ["bash", "run.sh"]"#,
            "command = [\"cargo\", \"run\"]\ncomponent = \"ext.cwasm\"",
        );
        let m = Manifest::parse(&text).unwrap();
        assert_eq!(m.tier(dir.path(), false), Tier::Quick);
        std::fs::write(dir.path().join("ext.cwasm"), b"").unwrap();
        assert_eq!(m.tier(dir.path(), false), Tier::Safe);
        assert_eq!(m.tier(dir.path(), true), Tier::Quick);
    }

    #[test]
    fn parses_durations_and_sizes() {
        assert_eq!(parse_duration("30s"), Some(Duration::from_secs(30)));
        assert_eq!(parse_duration("500ms"), Some(Duration::from_millis(500)));
        assert_eq!(parse_duration("s"), None);
        assert_eq!(parse_size("64MB"), Some(64 << 20));
        assert_eq!(parse_size("64"), None);
    }
}

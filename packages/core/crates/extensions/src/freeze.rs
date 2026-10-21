//! The frozen core: what the extensions Vorn ships may reach, and what is
//! left of features in the core crates.
//!
//! A built-in is held to what any extension gets: its manifest reads as an
//! installed one's does, its only Vorn crate is the SDK, it uses none of the
//! SDK's hidden modules, and every bridge method or event it names is
//! public. The core is the crates `freeze.toml` gives an area; every other
//! workspace member is a host, tooling, a built-in or an exception with its
//! reason, and every place a core or host crate names a built-in is listed
//! there too, so what is left to move out is a list, kept honest by
//! [`check`] in the core's tests.

use std::collections::{BTreeMap, BTreeSet};
use std::fmt;
use std::fs;
use std::path::{Component, Path, PathBuf};

use serde::Deserialize;

use crate::events::CORE_EVENTS;
use crate::gate::CORE_METHODS;
use crate::manifest::{file, Permission};

/// The file, beside the workspace's `Cargo.toml`, that lists the core and its exceptions.
pub const FILE: &str = "freeze.toml";
/// Where the built-ins live, relative to the workspace.
const BUILTINS: &str = "extensions";
/// The one Vorn crate a built-in builds on.
const SDK: &str = "crates/extension-sdk";
/// What a core crate may hold, and nothing else.
pub const AREAS: &[&str] = &[
    "projects",
    "worktrees",
    "sessions",
    "terminals",
    "git",
    "settings",
    "palette",
    "mcp",
    "tasks",
    "workflows",
    "bridge",
    "support",
];
/// The SDK's modules that its macros use and authors never do.
const HIDDEN: &[&str] = &["vorn_extension_sdk::bindings", "vorn_extension_sdk::guest"];

/// One thing the check found, at a file and line of the workspace.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Finding {
    pub path: PathBuf,
    pub line: usize,
    pub message: String,
}

impl fmt::Display for Finding {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}:{}: {}", self.path.display(), self.line, self.message)
    }
}

#[derive(Debug, Default, Deserialize)]
#[serde(deny_unknown_fields)]
struct Freeze {
    #[serde(default)]
    core: BTreeMap<String, String>,
    #[serde(default)]
    hosts: BTreeMap<String, String>,
    #[serde(default)]
    tooling: BTreeMap<String, String>,
    #[serde(default)]
    exceptions: Exceptions,
}

#[derive(Debug, Default, Deserialize)]
#[serde(deny_unknown_fields)]
struct Exceptions {
    #[serde(default)]
    crates: BTreeMap<String, String>,
    #[serde(default)]
    modules: BTreeMap<String, String>,
    #[serde(default)]
    dependencies: BTreeMap<String, String>,
    #[serde(default)]
    hooks: BTreeMap<String, String>,
}

#[derive(Debug, Deserialize)]
struct Workspace {
    workspace: Members,
}

#[derive(Debug, Deserialize)]
struct Members {
    members: Vec<String>,
}

#[derive(Debug, Deserialize)]
struct CargoToml {
    package: Option<Package>,
    #[serde(default)]
    dependencies: BTreeMap<String, toml::Value>,
    #[serde(default, rename = "build-dependencies")]
    build_dependencies: BTreeMap<String, toml::Value>,
    #[serde(default)]
    target: BTreeMap<String, Target>,
}

#[derive(Debug, Deserialize)]
struct Package {
    name: String,
}

#[derive(Debug, Deserialize)]
struct Target {
    #[serde(default)]
    dependencies: BTreeMap<String, toml::Value>,
}

/// What it found, collected as it goes.
struct Report {
    root: PathBuf,
    found: Vec<Finding>,
}

impl Report {
    fn at(&mut self, path: &Path, line: usize, message: impl Into<String>) {
        let path = path.strip_prefix(&self.root).unwrap_or(path).to_path_buf();
        self.found.push(Finding {
            path,
            line,
            message: message.into(),
        });
    }
}

/// Checks the workspace at `root` (`packages/core`): its built-ins against
/// the public API, and its crates and hooks against `freeze.toml`.
pub fn check(root: &Path) -> Result<(), Vec<Finding>> {
    let mut report = Report {
        root: root.to_path_buf(),
        found: Vec::new(),
    };
    let freeze_path = root.join(FILE);
    let freeze: Freeze = match read(&freeze_path).and_then(|text| {
        toml::from_str(&text).map_err(|e| format!("does not read: {}", e.message()))
    }) {
        Ok(f) => f,
        Err(why) => {
            report.at(&freeze_path, 1, why);
            return Err(report.found);
        }
    };
    let builtins = builtin_dirs(root);
    for dir in &builtins {
        check_builtin(&mut report, root, dir, &freeze);
    }
    let members = check_members(&mut report, root, &freeze, &builtins);
    check_modules(&mut report, root, &freeze);
    check_hooks(&mut report, root, &freeze, &members, &builtins);
    if report.found.is_empty() {
        Ok(())
    } else {
        Err(report.found)
    }
}

fn read(path: &Path) -> Result<String, String> {
    fs::read_to_string(path).map_err(|e| format!("cannot read: {e}"))
}

/// `extensions/<name>`, each folder of the built-ins with a manifest.
fn builtin_dirs(root: &Path) -> Vec<String> {
    let mut dirs: Vec<String> = fs::read_dir(root.join(BUILTINS))
        .into_iter()
        .flatten()
        .flatten()
        .filter(|e| e.path().join(file::FILE).is_file())
        .filter_map(|e| e.file_name().to_str().map(|n| format!("{BUILTINS}/{n}")))
        .collect();
    dirs.sort();
    dirs
}

// ---- the built-ins ----

fn check_builtin(report: &mut Report, root: &Path, dir: &str, freeze: &Freeze) {
    let folder = root.join(dir);
    let manifest = folder.join(file::FILE);
    match read(&manifest) {
        Ok(text) => {
            if let Err(e) = file::parse(&text) {
                let message = format!("does not read as an installed extension's: {}", e.field);
                report.at(&manifest, e.line, format!("{message}: {}", e.message));
            }
        }
        Err(why) => report.at(&manifest, 1, why),
    }
    check_dependencies(report, root, dir, freeze);
    for source in rust_files(&folder.join("src")) {
        let Ok(text) = fs::read_to_string(&source) else {
            continue;
        };
        let scanned = scan(&text);
        for (line, code) in &scanned.code {
            for hidden in HIDDEN {
                if code.contains(hidden) {
                    report.at(
                        &source,
                        *line,
                        format!("uses {hidden}, which only the SDK's own macros may"),
                    );
                }
            }
        }
        for (line, literal) in &scanned.literals {
            if let Some(why) = private_name(literal) {
                report.at(&source, *line, why);
            }
        }
    }
}

/// Why `literal` names a bridge method that is not public, if it does.
fn private_name(literal: &str) -> Option<String> {
    let (ns, rest) = literal.split_once('.')?;
    let word = |s: &str| !s.is_empty() && s.bytes().all(|b| b.is_ascii_alphanumeric());
    // A method is `ns.name`; longer names (node types, settings keys) are not calls.
    if !word(ns) || !word(rest) {
        return None;
    }
    let bridge = CORE_METHODS
        .iter()
        .any(|(m, _)| m.split_once('.').is_some_and(|(n, _)| n == ns));
    let public = CORE_METHODS.iter().any(|(m, _)| *m == literal)
        || CORE_EVENTS.contains(&literal)
        || Permission::named(literal).is_some();
    (bridge && !public)
        .then(|| format!("names {literal}, which is no public bridge method, event or permission"))
}

fn check_dependencies(report: &mut Report, root: &Path, dir: &str, freeze: &Freeze) {
    let path = root.join(dir).join("Cargo.toml");
    let cargo: CargoToml = match read(&path)
        .and_then(|t| toml::from_str(&t).map_err(|e| format!("does not read: {}", e.message())))
    {
        Ok(c) => c,
        Err(why) => return report.at(&path, 1, why),
    };
    let tables = [&cargo.dependencies, &cargo.build_dependencies]
        .into_iter()
        .chain(cargo.target.values().map(|t| &t.dependencies));
    let text = fs::read_to_string(&path).unwrap_or_default();
    for (name, spec) in tables.flat_map(BTreeMap::iter) {
        let Some(on) = spec.get("path").and_then(toml::Value::as_str) else {
            continue;
        };
        let target = normalize(&Path::new(dir).join(on));
        let key = format!("{dir} -> {target}");
        if target != SDK && !freeze.exceptions.dependencies.contains_key(&key) {
            let line = line_of(&text, &format!("{name} =")).unwrap_or(1);
            report.at(
                &path,
                line,
                format!("depends on {target}; a built-in builds on {SDK} alone (or list \"{key}\" in {FILE})"),
            );
        }
    }
}

// ---- the workspace ----

/// Classifies every member; answers member path to crate name.
fn check_members(
    report: &mut Report,
    root: &Path,
    freeze: &Freeze,
    builtins: &[String],
) -> BTreeMap<String, String> {
    let cargo = root.join("Cargo.toml");
    let members = match read(&cargo).and_then(|t| {
        toml::from_str::<Workspace>(&t).map_err(|e| format!("does not read: {}", e.message()))
    }) {
        Ok(w) => w.workspace.members,
        Err(why) => {
            report.at(&cargo, 1, why);
            return BTreeMap::new();
        }
    };
    let freeze_path = root.join(FILE);
    let freeze_text = fs::read_to_string(&freeze_path).unwrap_or_default();
    let tables: [(&str, &BTreeMap<String, String>); 4] = [
        ("core", &freeze.core),
        ("hosts", &freeze.hosts),
        ("tooling", &freeze.tooling),
        ("exceptions.crates", &freeze.exceptions.crates),
    ];
    for (table, entries) in tables {
        for (member, said) in entries {
            let line = line_of(&freeze_text, &format!("\"{member}\"")).unwrap_or(1);
            if !members.contains(member) {
                report.at(
                    &freeze_path,
                    line,
                    format!("[{table}] names {member}, which is no workspace member"),
                );
            }
            if said.trim().is_empty() {
                report.at(
                    &freeze_path,
                    line,
                    format!("[{table}] gives {member} no reason"),
                );
            }
            if table == "core" && !AREAS.contains(&said.as_str()) {
                report.at(
                    &freeze_path,
                    line,
                    format!(
                        "{member} is in area `{said}`; the core holds only {}",
                        AREAS.join(", ")
                    ),
                );
            }
        }
    }
    let mut names = BTreeMap::new();
    for member in &members {
        let builtin = builtins.contains(member);
        let classed = tables
            .iter()
            .filter(|(_, t)| t.contains_key(member))
            .count()
            + usize::from(builtin);
        if classed != 1 {
            let line = line_of(
                &fs::read_to_string(&cargo).unwrap_or_default(),
                &format!("\"{member}\""),
            )
            .unwrap_or(1);
            let message = if classed == 0 {
                format!("{member} is neither core, host, tooling, a built-in nor an exception in {FILE}")
            } else {
                format!("{member} is classed more than once in {FILE}")
            };
            report.at(&cargo, line, message);
        }
        let manifest = root.join(member).join("Cargo.toml");
        if let Some(name) = fs::read_to_string(&manifest)
            .ok()
            .and_then(|t| toml::from_str::<CargoToml>(&t).ok())
            .and_then(|c| c.package)
        {
            names.insert(member.clone(), name.name.replace('-', "_"));
        }
    }
    names
}

/// Every module a host keeps for a built-in is listed, and every listed one is there.
fn check_modules(report: &mut Report, root: &Path, freeze: &Freeze) {
    let freeze_path = root.join(FILE);
    let freeze_text = fs::read_to_string(&freeze_path).unwrap_or_default();
    for (module, reason) in &freeze.exceptions.modules {
        let line = line_of(&freeze_text, &format!("\"{module}\"")).unwrap_or(1);
        if !root.join(module).is_file() {
            report.at(
                &freeze_path,
                line,
                format!("[exceptions.modules] names {module}, which is gone"),
            );
        }
        if reason.trim().is_empty() {
            report.at(
                &freeze_path,
                line,
                format!("[exceptions.modules] gives {module} no reason"),
            );
        }
    }
}

/// Every place a core or host crate names a built-in, by file and the name it uses.
fn check_hooks(
    report: &mut Report,
    root: &Path,
    freeze: &Freeze,
    names: &BTreeMap<String, String>,
    builtins: &[String],
) {
    let feature_crates: BTreeSet<String> = freeze
        .exceptions
        .crates
        .keys()
        .chain(builtins)
        .filter_map(|m| names.get(m))
        .map(|n| format!("{n}::"))
        .collect();
    let ids = builtin_ids(root, freeze, builtins);
    let modules: BTreeSet<&str> = freeze
        .exceptions
        .modules
        .keys()
        .map(String::as_str)
        .collect();
    let stems: BTreeSet<String> = modules
        .iter()
        .filter_map(|m| Path::new(m).file_stem()?.to_str())
        .filter(|s| *s != "mod")
        .map(|s| format!("builtin::{s}"))
        .collect();
    let mut seen = BTreeSet::new();
    for member in freeze.core.keys().chain(freeze.hosts.keys()) {
        for source in rust_files(&root.join(member).join("src")) {
            let rel = normalize(source.strip_prefix(root).unwrap_or(&source));
            if modules.contains(rel.as_str()) {
                continue;
            }
            let Ok(text) = fs::read_to_string(&source) else {
                continue;
            };
            let scanned = scan(&text);
            let mut named: Vec<(usize, String)> = Vec::new();
            for (line, code) in &scanned.code {
                for (at, _) in code.match_indices("builtin::") {
                    let end = code[at + "builtin::".len()..]
                        .find(|c: char| !(c.is_ascii_alphanumeric() || c == '_'))
                        .map_or(code.len(), |e| at + "builtin::".len() + e);
                    let token = &code[at..end];
                    if stems.contains(token) && !named.iter().any(|(_, n)| n == token) {
                        named.push((*line, token.to_owned()));
                    }
                }
                for prefix in &feature_crates {
                    let used = code.match_indices(prefix.as_str()).any(|(i, _)| {
                        code[..i]
                            .chars()
                            .next_back()
                            .is_none_or(|c| !(c.is_ascii_alphanumeric() || c == '_'))
                    });
                    let token = prefix.trim_end_matches("::").to_owned();
                    if used && !named.iter().any(|(_, n)| *n == token) {
                        named.push((*line, token));
                    }
                }
            }
            for (line, literal) in &scanned.literals {
                if ids.contains(literal) && !named.iter().any(|(_, n)| n == literal) {
                    named.push((*line, literal.clone()));
                }
            }
            for (line, token) in named {
                let key = format!("{rel}#{token}");
                if !freeze.exceptions.hooks.contains_key(&key) {
                    report.at(
                        &source,
                        line,
                        format!("names the built-in {token} from a core or host crate; move it out, or list \"{key}\" in {FILE} with a reason"),
                    );
                }
                seen.insert(key);
            }
        }
    }
    let freeze_path = root.join(FILE);
    let freeze_text = fs::read_to_string(&freeze_path).unwrap_or_default();
    for (key, reason) in &freeze.exceptions.hooks {
        let line = line_of(&freeze_text, &format!("\"{key}\"")).unwrap_or(1);
        if !seen.contains(key) {
            report.at(
                &freeze_path,
                line,
                format!("[exceptions.hooks] names {key}, which is gone; drop it"),
            );
        }
        if reason.trim().is_empty() {
            report.at(
                &freeze_path,
                line,
                format!("[exceptions.hooks] gives {key} no reason"),
            );
        }
    }
}

/// The ids of the built-ins and the exception crates' packages.
fn builtin_ids(root: &Path, freeze: &Freeze, builtins: &[String]) -> BTreeSet<String> {
    let mut manifests: Vec<PathBuf> = builtins
        .iter()
        .chain(freeze.exceptions.crates.keys())
        .map(|d| root.join(d).join(file::FILE))
        .collect();
    for module in freeze.exceptions.modules.keys() {
        let toml = root.join(module).with_extension("toml");
        manifests.push(toml);
    }
    manifests
        .iter()
        .filter_map(|m| fs::read_to_string(m).ok())
        .filter_map(|t| toml::from_str::<toml::Table>(&t).ok())
        .filter_map(|t| t.get("id").and_then(toml::Value::as_str).map(str::to_owned))
        .collect()
}

// ---- reading files ----

fn rust_files(dir: &Path) -> Vec<PathBuf> {
    let mut found = Vec::new();
    let mut stack = vec![dir.to_path_buf()];
    while let Some(d) = stack.pop() {
        for entry in fs::read_dir(&d).into_iter().flatten().flatten() {
            let path = entry.path();
            let name = entry.file_name();
            if path.is_dir() {
                if name != "tests" {
                    stack.push(path);
                }
            } else if path.extension().is_some_and(|e| e == "rs") && name != "tests.rs" {
                found.push(path);
            }
        }
    }
    found.sort();
    found
}

/// `a/b/../c` as `a/c`, with `/` between parts on every platform.
fn normalize(path: &Path) -> String {
    let mut parts: Vec<String> = Vec::new();
    for part in path.components() {
        match part {
            Component::ParentDir => {
                parts.pop();
            }
            Component::Normal(p) => parts.push(p.to_string_lossy().into_owned()),
            _ => {}
        }
    }
    parts.join("/")
}

fn line_of(text: &str, needle: &str) -> Option<usize> {
    text.lines().position(|l| l.contains(needle)).map(|i| i + 1)
}

/// A Rust file as code and string literals, each with its line; comments
/// dropped, and nothing after the first `#[cfg(test)]`.
#[derive(Debug, Default, PartialEq, Eq)]
struct Scanned {
    code: Vec<(usize, String)>,
    literals: Vec<(usize, String)>,
}

fn scan(text: &str) -> Scanned {
    let b = text.as_bytes();
    let mut out = Scanned::default();
    let (mut i, mut line) = (0, 1);
    let mut code = String::new();
    let flush = |code: &mut String, line: usize, out: &mut Scanned| {
        if !code.trim().is_empty() {
            out.code.push((line, std::mem::take(code)));
        }
        code.clear();
    };
    while i < b.len() {
        let c = b[i];
        if c == b'\n' {
            if code.contains("#[cfg(test)]") {
                code.clear();
                break;
            }
            flush(&mut code, line, &mut out);
            line += 1;
            i += 1;
        } else if b[i..].starts_with(b"//") {
            while i < b.len() && b[i] != b'\n' {
                i += 1;
            }
        } else if b[i..].starts_with(b"/*") {
            let mut depth = 0;
            while i < b.len() {
                if b[i..].starts_with(b"/*") {
                    depth += 1;
                    i += 2;
                } else if b[i..].starts_with(b"*/") {
                    depth -= 1;
                    i += 2;
                    if depth == 0 {
                        break;
                    }
                } else {
                    if b[i] == b'\n' {
                        flush(&mut code, line, &mut out);
                        line += 1;
                    }
                    i += 1;
                }
            }
        } else if let Some((hashes, start)) = raw_start(b, i) {
            let close: Vec<u8> = std::iter::once(b'"')
                .chain(std::iter::repeat_n(b'#', hashes))
                .collect();
            let (at, mut j) = (line, start);
            while j < b.len() && !b[j..].starts_with(&close) {
                j += 1;
            }
            let body = &text[start..j.min(b.len())];
            out.literals.push((at, body.to_owned()));
            line += body.matches('\n').count();
            code.push_str("\"\"");
            i = (j + close.len()).min(b.len());
        } else if c == b'"' {
            let (at, mut j) = (line, i + 1);
            let mut body = Vec::new();
            while j < b.len() && b[j] != b'"' {
                if b[j] == b'\\' {
                    j += 1;
                    if b.get(j) == Some(&b'\n') {
                        line += 1;
                    }
                } else {
                    if b[j] == b'\n' {
                        line += 1;
                    }
                    body.push(b[j]);
                }
                j += 1;
            }
            out.literals
                .push((at, String::from_utf8_lossy(&body).into_owned()));
            code.push_str("\"\"");
            i = j + 1;
        } else if c == b'\'' {
            // A char literal is `'x'` or `'\..'`; anything else is a lifetime.
            if b.get(i + 1) == Some(&b'\\') {
                let mut j = i + 2;
                while j < b.len() && b[j] != b'\'' {
                    j += 1;
                }
                i = j + 1;
            } else if let Some(len) = text[i + 1..].chars().next().map(char::len_utf8) {
                if b.get(i + 1 + len) == Some(&b'\'') {
                    i += len + 2;
                } else {
                    code.push('\'');
                    i += 1;
                }
            } else {
                i += 1;
            }
        } else {
            let len = text[i..].chars().next().map_or(1, char::len_utf8);
            code.push_str(&text[i..i + len]);
            i += len;
        }
    }
    flush(&mut code, line, &mut out);
    out
}

/// Where a raw string's body starts, and its `#` count, if one starts at `i`.
fn raw_start(b: &[u8], i: usize) -> Option<(usize, usize)> {
    let ident_before = i > 0 && (b[i - 1].is_ascii_alphanumeric() || b[i - 1] == b'_');
    if ident_before {
        return None;
    }
    let mut j = i;
    if b.get(j) == Some(&b'b') {
        j += 1;
    }
    if b.get(j) != Some(&b'r') {
        return None;
    }
    j += 1;
    let hashes = b[j..].iter().take_while(|&&c| c == b'#').count();
    j += hashes;
    (b.get(j) == Some(&b'"')).then_some((hashes, j + 1))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn scans_code_and_literals_without_comments_or_tests() {
        let text = "use a::b; // \"x.y\"\n/* \"c.d\" */ let s = \"task.list\";\nlet r = r#\"workflow.run\"#; let c = '\"'; fn f<'a>() {}\n#[cfg(test)]\nlet t = \"after.tests\";\n";
        let s = scan(text);
        assert_eq!(
            s.literals,
            [(2, "task.list".to_owned()), (3, "workflow.run".to_owned())]
        );
        assert!(s
            .code
            .iter()
            .all(|(_, c)| !c.contains("x.y") && !c.contains("after")));
        assert!(s
            .code
            .iter()
            .any(|(l, c)| *l == 3 && c.contains("fn f<'a>")));
    }

    #[test]
    fn a_bridge_name_is_public_when_it_is_a_method_an_event_or_a_permission() {
        assert_eq!(private_name("task.list"), None);
        assert_eq!(private_name("task.created"), None);
        assert_eq!(private_name("tasks.read"), None);
        assert_eq!(private_name("ui.render"), None);
        assert_eq!(private_name("not a.name"), None);
        assert_eq!(
            private_name("task.secretly").unwrap(),
            "names task.secretly, which is no public bridge method, event or permission"
        );
    }

    #[test]
    fn paths_are_normalised_with_slashes() {
        assert_eq!(
            normalize(Path::new("extensions/tasks/../../crates/mcp")),
            "crates/mcp"
        );
    }
}

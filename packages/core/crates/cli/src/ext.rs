//! `vorn ext install|link|remove|list|pack|new|compile`: extensions in the running server.
//!
//! An install shows the sheet the server read from the repository before
//! anything of it runs, and installs only once the person types the
//! extension's id, or gave it with `--confirm`. `pack`, `new` and `compile`
//! need no server: `pack` checks the manifest where the author works and runs
//! the recorded checks, `new` writes a package, and `compile` compiles a Wasm
//! component into the cache vornd runs components from (vornd holds no
//! compiler). `link` of a component builds it first, and with `--watch`
//! builds and reloads it on every save.

use std::path::{Path, PathBuf};

use serde_json::{json, Value};

use crate::client::{CommandError, Context};
use crate::exit::ExitCode;
use crate::output::table;

/// A clone, or a build fetching its dependencies, takes longer than a call.
const WORK_TIMEOUT_MS: u64 = 15 * 60 * 1000;

pub const EXT_USAGE: &str = "Usage
  vorn ext install <git url> [--branch <name>] [--confirm <id>] [--json]
  vorn ext link <folder> [--watch] [--json]
  vorn ext remove <id>
  vorn ext list [--json]
  vorn ext sandbox <id> on|off
  vorn ext pack [<folder>] [--out <file>] [--json]
  vorn ext new <id> [--wasm] [--path <folder>] [--sdk <folder>]
  vorn ext compile <file.wasm> --into <folder>

An extension runs as you. install shows what it runs, the hosts it reaches,
the Vorn methods it uses and what it adds, and runs nothing of it until you
type its id. --confirm <id> types it for a script that has read the sheet.
On macOS, sandbox on starts its programs with no network of their own and
writes only inside its folder.

new writes a package for extension <id> in ./<id>, or --path: a program, or
with --wasm a Rust crate built as a sandboxed Wasm component. link builds a
component first; --watch builds and reloads it on every save until Ctrl-C.
compile is what Vorn runs to compile a component into its cache.
";

fn text<'a>(value: &'a Value, key: &str) -> &'a str {
    value.get(key).and_then(Value::as_str).unwrap_or_default()
}

fn strings(value: &Value, key: &str) -> Vec<String> {
    value
        .get(key)
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .filter_map(Value::as_str)
        .map(str::to_owned)
        .collect()
}

/// The sheet as a person reads it in a terminal.
pub fn render_sheet(sheet: &Value) -> String {
    let mut out = String::new();
    out.push_str(&format!(
        "{} {} ({})\nPublished by {}\n",
        text(sheet, "name"),
        text(sheet, "version"),
        text(sheet, "id"),
        text(sheet, "publisher")
    ));
    let source = &sheet["source"];
    if let Some(git) = source.get("git").and_then(Value::as_str) {
        let rev = text(source, "rev");
        out.push_str(&format!("From {git} at {}\n", rev.get(..12).unwrap_or(rev)));
    }
    if let Some(d) = sheet.get("description").and_then(Value::as_str) {
        out.push_str(&format!("{d}\n"));
    }
    let commands: Vec<(String, String)> = sheet["commands"]
        .as_array()
        .into_iter()
        .flatten()
        .map(|c| {
            let argv = strings(c, "argv").join(" ");
            (text(c, "when").to_owned(), argv)
        })
        .collect();
    let mark = |part: &str| {
        if strings(sheet, "enforced").iter().any(|p| p == part) {
            " (enforced by Vorn)"
        } else if strings(sheet, "declared").iter().any(|p| p == part) {
            " (declared, not enforced)"
        } else {
            ""
        }
    };
    out.push_str(&format!("\nIt runs{}\n", mark("commands")));
    if commands.is_empty() {
        out.push_str("  nothing\n");
    }
    let width = commands.iter().map(|(w, _)| w.len()).max().unwrap_or(0);
    for (when, argv) in &commands {
        out.push_str(&format!("  {when:<width$}  {argv}\n"));
    }
    for (title, key) in [
        ("It reaches, through Vorn", "hosts"),
        ("It asks Vorn for", "methods"),
        ("It adds", "adds"),
    ] {
        let items = strings(sheet, key);
        out.push_str(&format!("\n{title}{}\n", mark(key)));
        if items.is_empty() {
            out.push_str("  nothing\n");
        }
        for item in items {
            out.push_str(&format!("  {item}\n"));
        }
    }
    if let Some(c) = sheet.get("credential").and_then(Value::as_str) {
        out.push_str(&format!("\n{c}{}\n", mark("credential")));
    }
    if let Some(v) = sheet.get("replaces").and_then(Value::as_str) {
        out.push_str(&format!("\nIt replaces version {v}.\n"));
    }
    out.push_str(&format!("\n{}\n", text(sheet, "runsAsYou")));
    out
}

async fn install(ctx: &mut Context<'_>, url: Option<&str>) -> Result<ExitCode, CommandError> {
    let Some(url) = url.filter(|u| !u.is_empty()) else {
        return Ok(ctx.usage("ext install needs a git URL", EXT_USAGE));
    };
    let mut params = json!({ "url": url });
    if let Some(branch) = &ctx.args.branch {
        params["branch"] = json!(branch);
    }
    let sheet = ctx
        .rpc
        .call_within("extension:inspect", Some(params), WORK_TIMEOUT_MS)
        .await?;
    let id = text(&sheet, "confirm").to_owned();
    ctx.io.write_err(&render_sheet(&sheet));
    let typed = match ctx.args.confirm.clone() {
        Some(typed) => typed,
        None => {
            ctx.io
                .write_err(&format!("\nType {id} to install it, as you: "));
            match ctx.io.read_line() {
                Some(line) => line,
                None => {
                    ctx.io.write_err(&format!(
                        "\nNothing was installed. Run it again in a terminal, or with --confirm {id}.\n"
                    ));
                    return Ok(ExitCode::Usage);
                }
            }
        }
    };
    if typed.trim() != id {
        ctx.io
            .write_err(&format!("Nothing was installed: that is not {id}.\n"));
        return Ok(ExitCode::Failure);
    }
    let params = json!({ "token": text(&sheet, "token"), "confirm": typed.trim() });
    ctx.io.write_err("Installing; its build runs now.\n");
    let pack = ctx
        .rpc
        .call_within("extension:install", Some(params), WORK_TIMEOUT_MS)
        .await?;
    if ctx.args.json {
        ctx.io.write(&format!("{pack}\n"));
    } else {
        ctx.io.write(&format!(
            "Installed {} {}\n",
            text(&pack, "name"),
            text(&pack, "version")
        ));
    }
    Ok(ExitCode::Ok)
}

fn absolute(folder: &str) -> PathBuf {
    let folder = folder.strip_suffix("/.").unwrap_or(folder);
    let path = Path::new(folder);
    if folder == "." {
        return std::env::current_dir().unwrap_or_else(|_| path.to_path_buf());
    }
    if path.is_absolute() {
        path.to_path_buf()
    } else {
        std::env::current_dir()
            .map(|cwd| cwd.join(path))
            .unwrap_or_else(|_| path.to_path_buf())
    }
}

/// The build a component's package runs before it links: its `[build]`, when its service is a component.
fn component_build(folder: &Path) -> Result<Option<Vec<String>>, String> {
    let at = folder.join("vorn-extension.toml");
    let text = std::fs::read_to_string(&at).map_err(|e| format!("{}: {e}", at.display()))?;
    let m = vorn_extensions::manifest::file::parse(&text).map_err(|e| e.to_string())?;
    let component = m.service.as_ref().is_some_and(|s| s.component.is_some());
    Ok(m.build.filter(|_| component))
}

/// Runs `build` in `folder` as the person would; answers the tail of what it said when it fails.
async fn run_build(folder: &Path, build: &[String]) -> Result<(), String> {
    let (program, args) = build
        .split_first()
        .ok_or_else(|| "build.command is empty".to_owned())?;
    let output = vorn_spawn::tokio_command(program)
        .args(args)
        .current_dir(folder)
        .stdin(std::process::Stdio::null())
        .output()
        .await
        .map_err(|e| format!("{} could not run: {e}", build.join(" ")))?;
    if output.status.success() {
        return Ok(());
    }
    let said = String::from_utf8_lossy(&output.stderr);
    let lines: Vec<&str> = said.lines().collect();
    Err(format!(
        "{} failed:\n{}",
        build.join(" "),
        lines[lines.len().saturating_sub(30)..].join("\n")
    ))
}

/// The newest change under `folder`, its build output and history left out.
fn newest_change(folder: &Path) -> Option<std::time::SystemTime> {
    const SKIPPED: [&str; 3] = ["target", ".git", "node_modules"];
    let mut newest = None;
    let mut pending = vec![folder.to_path_buf()];
    while let Some(dir) = pending.pop() {
        for entry in std::fs::read_dir(&dir).ok()?.flatten() {
            let name = entry.file_name();
            if SKIPPED.iter().any(|s| name == *s) {
                continue;
            }
            let Ok(meta) = entry.metadata() else { continue };
            if meta.is_dir() {
                pending.push(entry.path());
            } else if let Ok(at) = meta.modified() {
                newest = newest.max(Some(at));
            }
        }
    }
    newest
}

/// How often `--watch` looks for a save.
const WATCH_EVERY: std::time::Duration = std::time::Duration::from_millis(400);

async fn link(ctx: &mut Context<'_>, folder: Option<&str>) -> Result<ExitCode, CommandError> {
    let folder = absolute(folder.unwrap_or("."));
    let build = match component_build(&folder) {
        Ok(build) => build,
        Err(err) => {
            ctx.io.write_err(&format!("vorn: {err}\n"));
            return Ok(ExitCode::Failure);
        }
    };
    if let Some(build) = &build {
        ctx.io
            .write_err(&format!("Building: {}\n", build.join(" ")));
        if let Err(err) = run_build(&folder, build).await {
            ctx.io.write_err(&format!("vorn: {err}\n"));
            if !ctx.args.watch {
                return Ok(ExitCode::Failure);
            }
        }
    }
    let params = json!({ "path": folder.to_string_lossy() });
    let pack = ctx.call("extension:link", Some(params.clone())).await?;
    if ctx.args.json {
        ctx.io.write(&format!("{pack}\n"));
    } else {
        let live = if build.is_some() {
            "it is rebuilt by vorn ext link"
        } else {
            "edits are live"
        };
        ctx.io.write(&format!(
            "Linked {} from {}; {live}\n",
            text(&pack, "id"),
            folder.display()
        ));
    }
    if !ctx.args.watch {
        return Ok(ExitCode::Ok);
    }
    let Some(build) = build else {
        ctx.io
            .write_err("Its edits are live already; --watch rebuilds a Wasm component.\n");
        return Ok(ExitCode::Ok);
    };
    ctx.io.write_err(&format!(
        "Watching {}; a save rebuilds and reloads it. Ctrl-C stops watching; the link stays.\n",
        folder.display()
    ));
    let scan = |folder: PathBuf| async move {
        tokio::task::spawn_blocking(move || newest_change(&folder))
            .await
            .ok()
            .flatten()
    };
    let mut seen = scan(folder.clone()).await;
    loop {
        tokio::time::sleep(WATCH_EVERY).await;
        let now = scan(folder.clone()).await;
        if now <= seen {
            continue;
        }
        seen = now;
        match run_build(&folder, &build).await {
            Ok(()) => {
                ctx.call("extension:link", Some(params.clone())).await?;
                ctx.io.write_err("Rebuilt and reloaded\n");
            }
            Err(err) => ctx.io.write_err(&format!("vorn: {err}\n")),
        }
        // What the build wrote is not a save.
        seen = scan(folder.clone()).await.max(seen);
    }
}

fn new_package(ctx: &mut Context<'_>, id: Option<&str>) -> ExitCode {
    let Some(id) = id.filter(|i| !i.is_empty()) else {
        return ctx.usage("ext new needs the extension's id, as acme.hello", EXT_USAGE);
    };
    let folder = absolute(ctx.args.path.as_deref().unwrap_or(id));
    let sdk = ctx.args.sdk.as_deref().map(absolute);
    let version = env!("CARGO_PKG_VERSION");
    match crate::scaffold::write(&folder, id, ctx.args.wasm, sdk.as_deref(), version) {
        Ok(files) => {
            for f in &files {
                ctx.io.write_err(&format!(
                    "  {}\n",
                    f.strip_prefix(&folder).unwrap_or(f).display()
                ));
            }
            let next = if ctx.args.wasm {
                "cargo test, then vorn ext link --watch"
            } else {
                "vorn ext link"
            };
            ctx.io.write(&format!(
                "Wrote {id} in {}; next: cd there and run {next}\n",
                folder.display()
            ));
            ExitCode::Ok
        }
        Err(err) => {
            ctx.io.write_err(&format!("vorn: {err}\n"));
            ExitCode::Failure
        }
    }
}

fn compile(ctx: &mut Context<'_>, wasm: Option<&str>) -> ExitCode {
    let (Some(wasm), Some(into)) = (wasm.filter(|w| !w.is_empty()), ctx.args.into.clone()) else {
        return ctx.usage(
            "ext compile needs a .wasm file and --into <folder>",
            EXT_USAGE,
        );
    };
    match vorn_extensions::wasm::precompile(&absolute(wasm), &absolute(&into)) {
        Ok(out) => {
            ctx.io.write(&format!("{}\n", out.display()));
            ExitCode::Ok
        }
        Err(err) => {
            ctx.io.write_err(&format!("vorn: {err}\n"));
            ExitCode::Failure
        }
    }
}

async fn remove(ctx: &mut Context<'_>, id: Option<&str>) -> Result<ExitCode, CommandError> {
    let Some(id) = id.filter(|i| !i.is_empty()) else {
        return Ok(ctx.usage("ext remove needs an extension id", EXT_USAGE));
    };
    ctx.call("extension:remove", Some(json!({ "id": id })))
        .await?;
    ctx.io.write(&format!("Removed {id}\n"));
    Ok(ExitCode::Ok)
}

async fn sandbox(
    ctx: &mut Context<'_>,
    id: Option<&str>,
    switch: Option<&str>,
) -> Result<ExitCode, CommandError> {
    let (Some(id), Some(on)) = (
        id.filter(|i| !i.is_empty()),
        switch.and_then(|s| match s {
            "on" => Some(true),
            "off" => Some(false),
            _ => None,
        }),
    ) else {
        return Ok(ctx.usage("ext sandbox needs an extension id and on or off", EXT_USAGE));
    };
    ctx.call("extension:setSandbox", Some(json!({ "id": id, "on": on })))
        .await?;
    let said = if on {
        "starts under the sandbox profile from now on"
    } else {
        "starts without the sandbox profile from now on"
    };
    ctx.io.write(&format!("{id} {said}\n"));
    Ok(ExitCode::Ok)
}

async fn list(ctx: &mut Context<'_>) -> Result<ExitCode, CommandError> {
    let packs = ctx.call("extension:installed", None).await?;
    if ctx.args.json {
        ctx.io.write(&format!("{packs}\n"));
        return Ok(ExitCode::Ok);
    }
    let packs = packs.as_array().cloned().unwrap_or_default();
    if packs.is_empty() {
        ctx.io.write_err("No extensions.\n");
        return Ok(ExitCode::Ok);
    }
    let rows: Vec<Vec<String>> = packs
        .iter()
        .map(|p| {
            let from = if p["linked"] == true {
                format!("linked {}", text(p, "path"))
            } else if let Some(git) = p.pointer("/source/git").and_then(Value::as_str) {
                git.to_owned()
            } else {
                "pack".to_owned()
            };
            let mut state = if p["running"] == true {
                "running"
            } else {
                "stopped"
            }
            .to_owned();
            if p["sandboxed"] == true {
                state.push_str(", sandboxed");
            }
            vec![
                text(p, "id").to_owned(),
                text(p, "version").to_owned(),
                state,
                from,
            ]
        })
        .collect();
    ctx.io
        .write(&table(&["ID", "VERSION", "STATE", "FROM"], &rows, None));
    Ok(ExitCode::Ok)
}

/// The agents a running server has on; none when no server is running, which `pack` does not start.
async fn server_agents(ctx: &Context<'_>) -> Vec<String> {
    if !ctx.rpc.is_running() {
        return Vec::new();
    }
    match ctx.call("agent:list", None).await {
        Ok(Value::Array(agents)) => agents
            .iter()
            .filter_map(|a| a.get("id").and_then(Value::as_str).map(str::to_owned))
            .collect(),
        _ => Vec::new(),
    }
}

async fn pack(ctx: &mut Context<'_>, folder: Option<&str>) -> ExitCode {
    let folder = absolute(folder.unwrap_or("."));
    let installed = server_agents(ctx).await;
    let env: Vec<(String, String)> = std::env::vars().collect();
    let out = ctx.args.out.as_deref().map(absolute);
    match vorn_extensions::install::pack(&folder, &env, out.as_deref(), &installed) {
        Ok(packed) => {
            if ctx.args.json {
                ctx.io.write(&format!("{}\n", json!(packed)));
            } else {
                if let Some(checks) = &packed.checks {
                    ctx.io.write_err(&format!("{checks}\n"));
                }
                ctx.io.write(&format!(
                    "{} {}: {} files, {} bytes{}\n",
                    packed.id,
                    packed.version,
                    packed.files.len(),
                    packed.bytes,
                    out.map(|o| format!(", bundled in {}", o.display()))
                        .unwrap_or_default()
                ));
            }
            ExitCode::Ok
        }
        Err(err) => {
            ctx.io.write_err(&format!("vorn: {err}\n"));
            ExitCode::Failure
        }
    }
}

pub async fn run(ctx: &mut Context<'_>) -> ExitCode {
    let positionals = ctx.args.positionals.clone();
    let verb = positionals.get(1).map(String::as_str);
    let first = positionals.get(2).map(String::as_str);
    if ctx.args.help {
        ctx.io.write(EXT_USAGE);
        return ExitCode::Ok;
    }
    let Some(verb) = verb.filter(|v| !v.is_empty()) else {
        ctx.io.write_err(EXT_USAGE);
        return ExitCode::Usage;
    };
    match verb {
        "pack" => return pack(ctx, first).await,
        "new" => return new_package(ctx, first),
        "compile" => return compile(ctx, first),
        _ => {}
    }
    if !["install", "link", "remove", "list", "sandbox"].contains(&verb) {
        return ctx.usage(&format!("unknown ext command \"{verb}\""), EXT_USAGE);
    }
    if !ctx.server().await {
        return ExitCode::Unreachable;
    }
    let done = match verb {
        "install" => install(ctx, first).await,
        "link" => link(ctx, first).await,
        "remove" => remove(ctx, first).await,
        "sandbox" => {
            let switch = positionals.get(3).map(String::as_str);
            sandbox(ctx, first, switch).await
        }
        _ => list(ctx).await,
    };
    match done {
        Ok(code) => code,
        Err(err) => ctx.failed(&format!("ext {verb}"), err),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn renders_every_part_of_the_sheet() {
        let sheet = json!({
            "token": "t", "id": "x.y", "name": "Y", "version": "1.0.0", "publisher": "someone",
            "source": { "git": "https://example.com/y.git", "rev": "0123456789abcdef" },
            "commands": [{ "when": "build", "argv": ["npm", "ci"] }, { "when": "action go", "argv": ["sh", "go.sh"] }],
            "hosts": ["api.example.com"], "methods": ["sessions.read (sessions.list)"], "adds": [],
            "enforced": ["hosts", "methods", "credential"], "declared": ["commands", "adds"],
            "runsAsYou": "Y runs as you.", "confirm": "x.y"
        });
        let text = render_sheet(&sheet);
        assert!(text.starts_with("Y 1.0.0 (x.y)\nPublished by someone\nFrom https://example.com/y.git at 0123456789ab\n"), "{text}");
        assert!(
            text.contains("  build      npm ci\n  action go  sh go.sh\n"),
            "{text}"
        );
        assert!(
            text.contains("It runs (declared, not enforced)\n"),
            "{text}"
        );
        assert!(text.contains("It reaches, through Vorn (enforced by Vorn)\n  api.example.com\n"));
        assert!(
            text.contains("It asks Vorn for (enforced by Vorn)\n  sessions.read (sessions.list)\n")
        );
        assert!(text.contains("It adds (declared, not enforced)\n  nothing\n"));
        assert!(text.ends_with("\nY runs as you.\n"));
    }
}

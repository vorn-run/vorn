//! `vorn ext install|link|remove|list|pack`: extensions in the running server.
//!
//! An install shows the sheet the server read from the repository before
//! anything of it runs, and installs only once the person types the
//! extension's id, or gave it with `--confirm`. `pack` needs no server: it
//! checks the manifest where the author works and runs the recorded checks.

use std::path::{Path, PathBuf};

use serde_json::{json, Value};

use crate::client::{CommandError, Context};
use crate::exit::ExitCode;
use crate::output::table;

/// A clone, or a build fetching its dependencies, takes longer than a call.
const WORK_TIMEOUT_MS: u64 = 15 * 60 * 1000;

pub const EXT_USAGE: &str = "Usage
  vorn ext install <git url> [--branch <name>] [--confirm <id>] [--json]
  vorn ext link <folder> [--json]
  vorn ext remove <id>
  vorn ext list [--json]
  vorn ext pack [<folder>] [--out <file>] [--json]

An extension runs as you. install shows what it runs, the hosts it reaches,
the Vorn methods it uses and what it adds, and runs nothing of it until you
type its id. --confirm <id> types it for a script that has read the sheet.
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
    out.push_str("\nIt runs\n");
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
        out.push_str(&format!("\n{title}\n"));
        if items.is_empty() {
            out.push_str("  nothing\n");
        }
        for item in items {
            out.push_str(&format!("  {item}\n"));
        }
    }
    if let Some(c) = sheet.get("credential").and_then(Value::as_str) {
        out.push_str(&format!("\n{c}\n"));
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
    let path = Path::new(folder);
    if path.is_absolute() {
        path.to_path_buf()
    } else {
        std::env::current_dir()
            .map(|cwd| cwd.join(path))
            .unwrap_or_else(|_| path.to_path_buf())
    }
}

async fn link(ctx: &mut Context<'_>, folder: Option<&str>) -> Result<ExitCode, CommandError> {
    let folder = absolute(folder.unwrap_or("."));
    let pack = ctx
        .call(
            "extension:link",
            Some(json!({ "path": folder.to_string_lossy() })),
        )
        .await?;
    if ctx.args.json {
        ctx.io.write(&format!("{pack}\n"));
    } else {
        ctx.io.write(&format!(
            "Linked {} from {}; edits are live\n",
            text(&pack, "id"),
            folder.display()
        ));
    }
    Ok(ExitCode::Ok)
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
            let state = if p["running"] == true {
                "running"
            } else {
                "stopped"
            };
            vec![
                text(p, "id").to_owned(),
                text(p, "version").to_owned(),
                state.to_owned(),
                from,
            ]
        })
        .collect();
    ctx.io
        .write(&table(&["ID", "VERSION", "STATE", "FROM"], &rows, None));
    Ok(ExitCode::Ok)
}

fn pack(ctx: &mut Context<'_>, folder: Option<&str>) -> ExitCode {
    let folder = absolute(folder.unwrap_or("."));
    let env: Vec<(String, String)> = std::env::vars().collect();
    let out = ctx.args.out.as_deref().map(absolute);
    match vorn_extensions::install::pack(&folder, &env, out.as_deref()) {
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
    if verb == "pack" {
        return pack(ctx, first);
    }
    if !["install", "link", "remove", "list"].contains(&verb) {
        return ctx.usage(&format!("unknown ext command \"{verb}\""), EXT_USAGE);
    }
    if !ctx.server().await {
        return ExitCode::Unreachable;
    }
    let done = match verb {
        "install" => install(ctx, first).await,
        "link" => link(ctx, first).await,
        "remove" => remove(ctx, first).await,
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
            "runsAsYou": "Y runs as you.", "confirm": "x.y"
        });
        let text = render_sheet(&sheet);
        assert!(text.starts_with("Y 1.0.0 (x.y)\nPublished by someone\nFrom https://example.com/y.git at 0123456789ab\n"), "{text}");
        assert!(
            text.contains("  build      npm ci\n  action go  sh go.sh\n"),
            "{text}"
        );
        assert!(text.contains("It reaches, through Vorn\n  api.example.com\n"));
        assert!(text.contains("It adds\n  nothing\n"));
        assert!(text.ends_with("\nY runs as you.\n"));
    }
}

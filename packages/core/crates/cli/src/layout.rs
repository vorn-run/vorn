//! `vorn layout list|open|save|check`: arrangements of sessions kept as files.

use serde_json::{json, Value};

use crate::client::{Context, CommandError};
use crate::exit::ExitCode;
use crate::js::{self, field};
use crate::output::table;
use crate::paths;
use crate::session::AGENTS;

pub const LAYOUT_USAGE: &str = "Usage
  vorn layout list [--json]
  vorn layout open <name> [--project <name or path>] [--json]
  vorn layout save <name> [--project <name or path>] [--force]
  vorn layout check <file>

A layout is a TOML file in the data directory's layouts/ folder, or one an
extension offers. `open` starts its sessions and prints their ids, one per line
(with --json, the whole answer). `save` writes the project's open sessions, in
order. `check` reads a file and reports every mistake as file:line: message.

Names are matched like session targets: the whole name, then ignoring case, then
the start of one. A name offered by several extensions is ambiguous (exit 3).
Without --project, the layout's own project is used, then this directory's
repository.

Layout file
  name = \"dev\"                    project = \"vorn\"
  split = \"horizontal\"            ratio = 0.6
  [[children]]                    agent = \"claude\" | command = [\"tail\", \"-f\", \"x\"] | shell = true
                                  cwd = \"sub/dir\"  label = \"name\"
  [[children]]                    pane = \"browser\"  url = \"http://localhost:3000\"
Sessions start in tree order, which is the order the app lays cards out in.
Split direction and ratio are kept in the file but do not size cards.
";

fn names(rows: &[Value]) -> Vec<(String, String)> {
    rows.iter()
        .map(|r| {
            (
                js::string(field(r, "name")),
                js::string(field(r, "source")),
            )
        })
        .collect()
}

/// The layout `given` names: (name, source), or the candidates it is ambiguous between.
fn resolve(given: &str, rows: &[Value]) -> Result<(String, String), Result<(), Vec<(String, String)>>> {
    let all = names(rows);
    let steps: [&dyn Fn(&str) -> bool; 3] = [
        &|n| n == given,
        &|n| n.eq_ignore_ascii_case(given),
        &|n| !given.is_empty() && n.starts_with(given),
    ];
    for hit in steps {
        let mut found: Vec<&(String, String)> = all.iter().filter(|(n, _)| hit(n)).collect();
        // A person's own layout is what a bare name means.
        if found.iter().any(|(_, s)| s == "user") && found.len() > 1 {
            let mine: Vec<_> = found.iter().copied().filter(|(_, s)| s == "user").collect();
            if mine.len() == 1 {
                found = mine;
            }
        }
        match found.as_slice() {
            [] => {}
            [one] => return Ok((*one).clone()),
            many => return Err(Err(many.iter().map(|c| (*c).clone()).collect())),
        }
    }
    Err(Ok(()))
}

async fn rows(ctx: &Context<'_>) -> Result<Vec<Value>, CommandError> {
    match ctx.call("layout:list", None).await? {
        Value::Array(rows) => Ok(rows),
        _ => Ok(Vec::new()),
    }
}

async fn list(ctx: &mut Context<'_>) -> ExitCode {
    let rows = match rows(ctx).await {
        Ok(rows) => rows,
        Err(err) => return ctx.failed("could not list layouts", err),
    };
    if ctx.args.json {
        ctx.io.write(&js::as_json(&Value::Array(rows)));
        return ExitCode::Ok;
    }
    if rows.is_empty() {
        ctx.io.write_err("No layouts.\n");
        return ExitCode::Ok;
    }
    let cells: Vec<Vec<String>> = rows
        .iter()
        .map(|r| {
            let detail = match field(r, "error").and_then(Value::as_str) {
                Some(e) => format!("error: {}", e.lines().next().unwrap_or("")),
                None => js::string(field(r, "sessions")),
            };
            vec![
                js::string(field(r, "name")),
                js::string(field(r, "source")),
                field(r, "project")
                    .and_then(Value::as_str)
                    .unwrap_or("-")
                    .to_owned(),
                detail,
            ]
        })
        .collect();
    ctx.io
        .write(&table(&["NAME", "SOURCE", "PROJECT", "SESSIONS"], &cells, None));
    ExitCode::Ok
}

/// The project a layout runs in when none was named: this directory's repository.
async fn here() -> Option<String> {
    let cwd = std::env::current_dir().ok()?;
    paths::repo_root(&cwd).await
}

async fn open(ctx: &mut Context<'_>, given: Option<&str>) -> ExitCode {
    let Some(given) = given.filter(|g| !g.is_empty()) else {
        return ctx.usage("layout open needs a name", LAYOUT_USAGE);
    };
    let all = match rows(ctx).await {
        Ok(rows) => rows,
        Err(err) => return ctx.failed("could not list layouts", err),
    };
    let (name, source) = match resolve(given, &all) {
        Ok(one) => one,
        Err(Ok(())) => {
            ctx.io
                .write_err(&format!("vorn: no layout matches \"{given}\"\n"));
            return ExitCode::Failure;
        }
        Err(Err(candidates)) => return ambiguous(ctx, given, &candidates),
    };
    let own = all
        .iter()
        .find(|r| {
            js::string(field(r, "name")) == name && js::string(field(r, "source")) == source
        })
        .and_then(|r| field(r, "project").and_then(Value::as_str))
        .map(str::to_owned);
    let project = match ctx.args.project.clone().or(own) {
        Some(p) => Some(p),
        None => here().await,
    };
    let mut ask = json!({ "name": name, "source": source });
    if let Some(project) = project {
        ask["project"] = project.into();
    }
    let done = match ctx.call("layout:open", Some(ask)).await {
        Ok(done) => done,
        Err(err) => return ctx.failed("could not open the layout", err),
    };
    let errors: Vec<String> = field(&done, "errors")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .map(|e| js::string(Some(e)))
        .collect();
    if ctx.args.json {
        ctx.io.write(&js::as_json(&done));
    } else {
        let ids: Vec<String> = field(&done, "sessions")
            .and_then(Value::as_array)
            .into_iter()
            .flatten()
            .map(|s| js::string(field(s, "id")))
            .collect();
        ctx.io.write(&format!("{}\n", ids.join("\n")));
    }
    for e in &errors {
        ctx.io.write_err(&format!("vorn: {e}\n"));
    }
    if errors.is_empty() {
        ExitCode::Ok
    } else {
        ExitCode::Failure
    }
}

fn ambiguous(ctx: &mut Context<'_>, given: &str, candidates: &[(String, String)]) -> ExitCode {
    let mut text = format!("vorn: \"{given}\" names {} layouts:\n", candidates.len());
    for (name, source) in candidates {
        text.push_str(&format!("  {name}  ({source})\n"));
    }
    ctx.io.write_err(&text);
    if ctx.args.json {
        let found: Vec<Value> = candidates
            .iter()
            .map(|(n, s)| json!({ "name": n, "source": s }))
            .collect();
        let data = json!({ "error": "ambiguous", "target": given, "candidates": found });
        ctx.io.write(&js::as_json(&data));
    }
    ExitCode::Ambiguous
}

async fn save(ctx: &mut Context<'_>, given: Option<&str>) -> ExitCode {
    let Some(given) = given.filter(|g| !g.is_empty()) else {
        return ctx.usage("layout save needs a name", LAYOUT_USAGE);
    };
    let project = match ctx.args.project.clone() {
        Some(p) => Some(p),
        None => here().await,
    };
    let Some(project) = project else {
        return ctx.usage("layout save needs --project, or a repository here", LAYOUT_USAGE);
    };
    let ask = json!({
        "name": given, "project": project,
        "overwrite": ctx.args.force,
    });
    match ctx.call("layout:save", Some(ask)).await {
        Ok(done) => {
            if ctx.args.json {
                ctx.io.write(&js::as_json(&done));
            } else {
                ctx.io.write(&format!("{}\n", js::string(field(&done, "path"))));
            }
            ExitCode::Ok
        }
        Err(err) => ctx.failed("could not save the layout", err),
    }
}

/// Reads a file where the author works; every mistake is `file:line: message`.
fn check(ctx: &mut Context<'_>, file: Option<&str>) -> ExitCode {
    let Some(file) = file.filter(|f| !f.is_empty()) else {
        return ctx.usage("layout check needs a file", LAYOUT_USAGE);
    };
    let text = match std::fs::read_to_string(file) {
        Ok(text) => text,
        Err(err) => {
            ctx.io
                .write_err(&format!("vorn: could not read {file}: {err}\n"));
            return ExitCode::Failure;
        }
    };
    match vorn_layout::parse(&text, &AGENTS) {
        Ok(layout) => {
            let sessions = layout
                .leaves()
                .iter()
                .filter(|l| matches!(l, vorn_layout::Leaf::Session(_)))
                .count();
            if ctx.args.json {
                ctx.io
                    .write(&js::as_json(&json!({ "file": file, "ok": true, "sessions": sessions })));
            } else {
                ctx.io
                    .write(&format!("{file}: ok, {sessions} sessions\n"));
            }
            ExitCode::Ok
        }
        Err(problems) => {
            for p in &problems {
                ctx.io.write_err(&format!("{}\n", p.at(file)));
            }
            if ctx.args.json {
                let list: Vec<Value> = problems
                    .iter()
                    .map(|p| json!({ "line": p.line, "message": p.message }))
                    .collect();
                ctx.io
                    .write(&js::as_json(&json!({ "file": file, "ok": false, "problems": list })));
            }
            ExitCode::Failure
        }
    }
}

/// `vorn layout ...`.
pub async fn run(ctx: &mut Context<'_>) -> ExitCode {
    let positionals = ctx.args.positionals.clone();
    let verb = positionals.get(1).map(String::as_str);
    let first = positionals.get(2).map(String::as_str);
    if ctx.args.help {
        ctx.io.write(LAYOUT_USAGE);
        return ExitCode::Ok;
    }
    let Some(verb) = verb.filter(|v| !v.is_empty()) else {
        ctx.io.write_err(LAYOUT_USAGE);
        return ExitCode::Usage;
    };
    if verb == "check" {
        return check(ctx, first);
    }
    if !["list", "open", "save"].contains(&verb) {
        return ctx.usage(&format!("unknown layout command \"{verb}\""), LAYOUT_USAGE);
    }
    if !ctx.server().await {
        return ExitCode::Unreachable;
    }
    match verb {
        "list" => list(ctx).await,
        "open" => open(ctx, first).await,
        _ => save(ctx, first).await,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn rows(items: &[(&str, &str)]) -> Vec<Value> {
        items
            .iter()
            .map(|(n, s)| json!({ "name": n, "source": s }))
            .collect()
    }

    #[test]
    fn a_persons_own_layout_wins_a_bare_name() {
        let all = rows(&[("dev", "user"), ("dev", "acme.tools")]);
        assert_eq!(resolve("dev", &all).unwrap(), ("dev".into(), "user".into()));
    }

    #[test]
    fn two_extensions_with_one_name_are_ambiguous() {
        let all = rows(&[("dev", "a.x"), ("dev", "b.y"), ("docs", "user")]);
        let Err(Err(found)) = resolve("dev", &all) else {
            panic!("not ambiguous")
        };
        assert_eq!(found.len(), 2);
        assert_eq!(resolve("Doc", &all).unwrap().0, "docs");
        assert!(matches!(resolve("zz", &all), Err(Ok(()))));
    }
}

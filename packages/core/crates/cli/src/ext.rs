//! `vorn ext`: extensions in two tiers, from a terminal.
//!
//! `link`, `unlink`, `list`, `refresh`, `watch` and `bench` talk to vornd;
//! `check` and `promote` work on a folder offline; `pack` hands over to
//! `vorn-extc`, the compiler vornd does not link.

use std::path::{Path, PathBuf};

use serde_json::{json, Value};
use vorn_ext::manifest::Manifest;

use crate::output::Io;
use crate::rpc::{Client, DataDir};
use crate::ExitCode;

const USAGE: &str = "Usage: vorn ext <command>

  link [dir] [--project <dir>] [--dev]   run the extension in [dir] (default: .) for a project
  unlink [dir]                           stop it
  list [--json]                          what each linked extension's sections show
  refresh                                ask every section for its rows now
  watch                                  print every section as vornd tells it, one JSON line each
  check [dir]                            validate the manifest and templates offline
  promote <dir> <to> --sdk <path>        scaffold a Rust crate from a quick extension
  pack <crate> [--against <dir>]         build the component and run the recorded checks on every tier
  bench <dir> [--calls N] [--instances N]  the safe tier's start, call and memory inside vornd
";

struct Opts {
    positionals: Vec<String>,
    data_dir: Option<String>,
    project: Option<String>,
    sdk: Option<String>,
    against: Option<String>,
    calls: Option<u64>,
    instances: Option<u64>,
    dev: bool,
    json: bool,
}

fn parse(argv: &[String]) -> Result<Opts, String> {
    let mut o = Opts {
        positionals: Vec::new(),
        data_dir: None,
        project: None,
        sdk: None,
        against: None,
        calls: None,
        instances: None,
        dev: false,
        json: false,
    };
    let mut it = argv.iter();
    while let Some(a) = it.next() {
        let mut value = |name: &str| it.next().cloned().ok_or(format!("{name} needs a value"));
        match a.as_str() {
            "--data-dir" => o.data_dir = Some(value(a)?),
            "--project" => o.project = Some(value(a)?),
            "--sdk" => o.sdk = Some(value(a)?),
            "--against" => o.against = Some(value(a)?),
            "--calls" => o.calls = value(a)?.parse().ok(),
            "--instances" => o.instances = value(a)?.parse().ok(),
            "--dev" => o.dev = true,
            "--json" => o.json = true,
            other if other.starts_with("--") => return Err(format!("unknown option {other}")),
            _ => o.positionals.push(a.clone()),
        }
    }
    Ok(o)
}

fn absolute(p: &str) -> PathBuf {
    let path = Path::new(p);
    let abs = if path.is_absolute() {
        path.to_owned()
    } else {
        std::env::current_dir().unwrap_or_default().join(path)
    };
    abs.canonicalize().unwrap_or(abs)
}

/// `vorn ext ...`; `argv` starts after `ext`.
pub async fn run(argv: &[String], io: &mut dyn Io) -> ExitCode {
    let o = match parse(argv) {
        Ok(o) => o,
        Err(e) => {
            io.write_err(&format!("vorn: {e}\n\n{USAGE}"));
            return ExitCode::Usage;
        }
    };
    let verb = o.positionals.first().map(String::as_str).unwrap_or("");
    let dir = |i: usize| absolute(o.positionals.get(i).map_or(".", String::as_str));
    let rpc = Client::new(DataDir::resolve(o.data_dir.as_deref()), None);
    let result: Result<Option<Value>, String> = match verb {
        "link" => {
            let project = absolute(o.project.as_deref().unwrap_or("."));
            let p = json!({ "dir": dir(1), "project": project, "dev": o.dev });
            rpc.call("ext:link", Some(p))
                .await
                .map(Some)
                .map_err(|e| e.to_string())
        }
        "unlink" => rpc
            .call("ext:unlink", Some(json!({ "dir": dir(1) })))
            .await
            .map(Some)
            .map_err(|e| e.to_string()),
        "refresh" => rpc
            .call("ext:refresh", None)
            .await
            .map(|_| None)
            .map_err(|e| e.to_string()),
        "list" => match rpc.call("ext:list", None).await {
            Ok(v) if o.json => Ok(Some(v)),
            Ok(v) => {
                for s in v.as_array().into_iter().flatten() {
                    io.write(&describe(s));
                }
                Ok(None)
            }
            Err(e) => Err(e.to_string()),
        },
        "bench" => {
            let p = json!({ "dir": dir(1), "calls": o.calls, "instances": o.instances });
            rpc.call("ext:bench", Some(p))
                .await
                .map(Some)
                .map_err(|e| e.to_string())
        }
        "watch" => {
            let r = rpc
                .watch(|method, params| {
                    if method == "ext:section" {
                        let mut line = params.clone();
                        line["receivedAt"] = json!(now_ms());
                        println!("{line}");
                    }
                    true
                })
                .await;
            r.map(|()| None).map_err(|e| e.to_string())
        }
        "check" => check(&dir(1)).map(|n| {
            io.write(&format!("ok: the manifest and {n} template(s) are valid\n"));
            None
        }),
        "promote" => match (o.positionals.get(1), o.positionals.get(2), &o.sdk) {
            (Some(from), Some(to), Some(sdk)) => {
                vorn_ext::promote::promote(&absolute(from), Path::new(to), &absolute(sdk))
                    .map(|p| {
                        for f in p.files {
                            io.write(&format!("wrote {to}/{f}\n"));
                        }
                        None
                    })
                    .map_err(|e| e.message)
            }
            _ => Err("promote needs <dir> <to> --sdk <path>".into()),
        },
        "pack" => pack(&dir(1), o.against.as_deref()),
        _ => {
            io.write_err(USAGE);
            return ExitCode::Usage;
        }
    };
    match result {
        Ok(Some(v)) => {
            io.write(&format!(
                "{}\n",
                serde_json::to_string_pretty(&v).unwrap_or_default()
            ));
            ExitCode::Ok
        }
        Ok(None) => ExitCode::Ok,
        Err(e) => {
            io.write_err(&format!("vorn ext {verb}: {e}\n"));
            ExitCode::Failure
        }
    }
}

/// One section as a line or two a person reads.
fn describe(s: &Value) -> String {
    let rows = s["rows"].as_array().map_or(0, Vec::len);
    let mut out = format!(
        "{} › {} ({}): {rows} rows in {:.0} ms\n",
        s["name"].as_str().unwrap_or("?"),
        s["title"].as_str().unwrap_or("?"),
        if s["tier"] == "quick" {
            "Local, runs as you"
        } else {
            "sandboxed"
        },
        s["tookMs"].as_f64().unwrap_or(0.0)
    );
    for e in s["templateErrors"].as_array().into_iter().flatten() {
        out.push_str(&format!(
            "  {}: {}: {}\n",
            e["where"].as_str().unwrap_or(""),
            e["path"].as_str().unwrap_or(""),
            e["message"].as_str().unwrap_or("")
        ));
    }
    if let Some(e) = s["error"].as_str() {
        out.push_str(&format!("  error: {e}\n"));
    }
    out
}

/// Every template the manifest names, checked; how many there were.
fn check(dir: &Path) -> Result<usize, String> {
    let m = Manifest::load(dir).map_err(|e| e.message)?;
    let mut failed = Vec::new();
    for s in &m.contributes.sidebar {
        let text =
            std::fs::read_to_string(dir.join(&s.view)).map_err(|e| format!("{}: {e}", s.view))?;
        if let Err(errs) = vorn_ext::view_file::parse(&text) {
            for e in errs {
                let at = e.at.map_or(String::new(), |(l, c)| format!(":{l}:{c}"));
                failed.push(format!("{}{at}: {}: {}", s.view, e.path, e.message));
            }
        }
    }
    if failed.is_empty() {
        Ok(m.contributes.sidebar.len())
    } else {
        Err(format!("\n  {}", failed.join("\n  ")))
    }
}

/// Runs `vorn-extc pack`, found beside this program or on PATH.
fn pack(dir: &Path, against: Option<&str>) -> Result<Option<Value>, String> {
    let beside = std::env::current_exe()
        .ok()
        .and_then(|e| e.parent().map(|d| d.join("vorn-extc")));
    let extc = beside
        .filter(|p| p.is_file())
        .unwrap_or_else(|| PathBuf::from("vorn-extc"));
    let mut cmd = vorn_spawn::command(&extc);
    cmd.arg("pack").arg(dir);
    if let Some(a) = against {
        cmd.arg("--against").arg(absolute(a));
    }
    let status = cmd.status().map_err(|e| {
        format!(
            "cannot run {}: {e}; build it with `cargo build -p vorn-extc`",
            extc.display()
        )
    })?;
    if status.success() {
        Ok(None)
    } else {
        Err("the checks did not pass".into())
    }
}

fn now_ms() -> f64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs_f64() * 1e3)
        .unwrap_or(0.0)
}

//! Recorded checks: one recording of the outside world, replayed against
//! every tier of an extension, so `pack` proves they answer the same rows.
//!
//! The two tiers touch the world differently: a quick script runs programs
//! (`gh`), a component calls the bridge (`net.fetch`, `git.branch`). A
//! recording holds both, taken at the same moment from the same pull request,
//! and the rows each call must answer. Replaying the programs puts shims
//! first on `PATH`; replaying the bridge is a [`World`] that answers from
//! the recording and refuses anything not in it. No credential is recorded.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::sync::Mutex;

use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::bridge::{FetchRequest, FetchResponse, World};
use crate::manifest::Manifest;
use crate::ExtError;

/// `checks/recorded.json`.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct Recording {
    pub recorded_at: String,
    /// What the core methods answered (`git.branch`, ...).
    pub core: BTreeMap<String, Value>,
    /// What the quick tier's programs printed.
    pub programs: Vec<ProgramRun>,
    /// What the safe tier's requests got back.
    pub http: Vec<Exchange>,
    /// The calls to replay and the answer each must give.
    pub calls: Vec<Expect>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ProgramRun {
    pub argv: Vec<String>,
    pub stdout: String,
    pub exit: i32,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Exchange {
    pub method: String,
    pub url: String,
    pub status: u16,
    pub body: String,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Expect {
    pub method: String,
    pub args: Value,
    pub answer: Value,
}

impl Recording {
    pub fn load(path: &Path) -> Result<Recording, ExtError> {
        let text = std::fs::read_to_string(path)
            .map_err(|e| ExtError::new("checks", format!("cannot read {}: {e}", path.display())))?;
        serde_json::from_str(&text)
            .map_err(|e| ExtError::new("checks", format!("{}: {e}", path.display())))
    }

    pub fn save(&self, path: &Path) -> Result<(), ExtError> {
        let text = serde_json::to_string_pretty(self).unwrap_or_default();
        std::fs::write(path, text + "\n")
            .map_err(|e| ExtError::new("checks", format!("cannot write {}: {e}", path.display())))
    }
}

/// Answers the bridge from a recording.
pub struct Replay(pub Recording);

impl World for Replay {
    fn fetch(&self, req: &FetchRequest) -> Result<FetchResponse, ExtError> {
        self.0
            .http
            .iter()
            .find(|x| x.method == req.method && x.url == req.url)
            .map(|x| FetchResponse {
                status: x.status,
                headers: vec![],
                body: x.body.clone(),
            })
            .ok_or_else(|| {
                ExtError::new(
                    "not-recorded",
                    format!("{} {} is not in the recording", req.method, req.url),
                )
            })
    }

    fn call(&self, method: &str, _: &Value, _: &str) -> Result<Value, ExtError> {
        self.0.core.get(method).cloned().ok_or_else(|| {
            ExtError::new("not-recorded", format!("{method} is not in the recording"))
        })
    }

    fn credential(&self, _: &Manifest) -> Result<Option<String>, ExtError> {
        Ok(Some("recorded".into()))
    }
}

/// Passes the bridge through to `inner` and keeps what it answered.
pub struct Record<'a> {
    pub inner: &'a dyn World,
    pub http: Mutex<Vec<Exchange>>,
    pub core: Mutex<BTreeMap<String, Value>>,
}

impl<'a> Record<'a> {
    pub fn new(inner: &'a dyn World) -> Record<'a> {
        Record {
            inner,
            http: Mutex::default(),
            core: Mutex::default(),
        }
    }
}

impl World for Record<'_> {
    fn fetch(&self, req: &FetchRequest) -> Result<FetchResponse, ExtError> {
        let res = self.inner.fetch(req)?;
        if let Ok(mut h) = self.http.lock() {
            h.push(Exchange {
                method: req.method.clone(),
                url: req.url.clone(),
                status: res.status,
                body: res.body.clone(),
            });
        }
        Ok(res)
    }

    fn call(&self, method: &str, args: &Value, project: &str) -> Result<Value, ExtError> {
        let v = self.inner.call(method, args, project)?;
        if let Ok(mut c) = self.core.lock() {
            c.insert(method.to_owned(), v.clone());
        }
        Ok(v)
    }

    fn credential(&self, ext: &Manifest) -> Result<Option<String>, ExtError> {
        self.inner.credential(ext)
    }
}

/// Writes a shim for each recorded program into `dir`; put `dir` first on
/// `PATH` and the quick tier sees the recording instead of the world.
pub fn program_shims(rec: &Recording, dir: &Path) -> Result<(), ExtError> {
    let io = |e: std::io::Error| ExtError::new("checks", format!("cannot write a shim: {e}"));
    let mut by_name: BTreeMap<&str, Vec<(usize, &ProgramRun)>> = BTreeMap::new();
    for (i, run) in rec.programs.iter().enumerate() {
        if let Some(name) = run.argv.first() {
            by_name.entry(name.as_str()).or_default().push((i, run));
        }
    }
    for (name, runs) in by_name {
        let mut sh = String::from("#!/bin/sh\ncase \"$*\" in\n");
        for (i, run) in runs {
            let out = dir.join(format!("{i}.out"));
            std::fs::write(&out, &run.stdout).map_err(io)?;
            let args = run.argv[1..].join(" ").replace('\'', r"'\''");
            sh.push_str(&format!(
                "  '{args}') cat '{}'; exit {};;\n",
                out.display(),
                run.exit
            ));
        }
        sh.push_str(&format!(
            "esac\necho \"{name} $*: not in the recording\" >&2\nexit 97\n"
        ));
        let path = dir.join(name);
        std::fs::write(&path, sh).map_err(io)?;
        make_executable(&path).map_err(io)?;
    }
    Ok(())
}

/// Writes a shim for each of `programs` that runs the real one and logs
/// argv, stdout and exit into `dir`; [`collect_programs`] reads them back.
pub fn recording_shims(programs: &[(String, PathBuf)], dir: &Path) -> Result<(), ExtError> {
    let io = |e: std::io::Error| ExtError::new("checks", format!("cannot write a shim: {e}"));
    for (name, real) in programs {
        let log = dir.join("log");
        let sh = format!(
            "#!/bin/sh\nn=$(ls '{log}' 2>/dev/null | wc -l | tr -d ' ')\nmkdir -p '{log}/'$n\n{{ printf '%s\\0' '{name}'; for a in \"$@\"; do printf '%s\\0' \"$a\"; done; }} > '{log}/'$n/argv\n'{real}' \"$@\" > '{log}/'$n/out\ncode=$?\necho $code > '{log}/'$n/exit\ncat '{log}/'$n/out\nexit $code\n",
            log = log.display(),
            real = real.display(),
        );
        let _ = std::fs::create_dir_all(&log);
        let path = dir.join(name);
        std::fs::write(&path, sh).map_err(io)?;
        make_executable(&path).map_err(io)?;
    }
    Ok(())
}

/// The program runs a [`recording_shims`] directory logged, in order.
pub fn collect_programs(dir: &Path) -> Vec<ProgramRun> {
    let log = dir.join("log");
    let mut runs = Vec::new();
    for n in 0.. {
        let d = log.join(n.to_string());
        let Ok(argv) = std::fs::read(d.join("argv")) else {
            break;
        };
        let argv: Vec<String> = argv
            .split(|b| *b == 0)
            .filter(|a| !a.is_empty())
            .map(|a| String::from_utf8_lossy(a).into_owned())
            .collect();
        runs.push(ProgramRun {
            argv,
            stdout: std::fs::read_to_string(d.join("out")).unwrap_or_default(),
            exit: std::fs::read_to_string(d.join("exit"))
                .ok()
                .and_then(|s| s.trim().parse().ok())
                .unwrap_or(-1),
        });
    }
    runs
}

#[cfg(unix)]
fn make_executable(path: &Path) -> std::io::Result<()> {
    use std::os::unix::fs::PermissionsExt;
    std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o755))
}

#[cfg(not(unix))]
fn make_executable(_: &Path) -> std::io::Result<()> {
    Ok(())
}

/// Rows compared as a set by id, with the first difference said in words.
pub fn same_rows(a: &Value, b: &Value) -> Result<(), String> {
    let key = |v: &Value| -> Result<BTreeMap<String, Value>, String> {
        let list = v
            .as_array()
            .ok_or_else(|| format!("not a list of rows: {v}"))?;
        Ok(list
            .iter()
            .map(|r| (r["id"].as_str().unwrap_or_default().to_owned(), r.clone()))
            .collect())
    };
    let (a, b) = (key(a)?, key(b)?);
    for (id, row) in &a {
        match b.get(id) {
            None => return Err(format!("row {id} is missing from the second")),
            Some(other) if other != row => {
                return Err(format!("row {id} differs: {row} vs {other}"))
            }
            Some(_) => {}
        }
    }
    if let Some(id) = b.keys().find(|id| !a.contains_key(*id)) {
        return Err(format!("row {id} is only in the second"));
    }
    Ok(())
}

#[cfg(all(test, unix))]
mod tests {
    use super::*;
    use crate::quick::{one_shot, CallCtx, Program};
    use serde_json::json;
    use std::time::Duration;

    #[test]
    fn a_program_shim_replays_what_was_recorded_and_refuses_the_rest() {
        let rec = Recording {
            programs: vec![ProgramRun {
                argv: vec![
                    "gh".into(),
                    "pr".into(),
                    "checks".into(),
                    "--json".into(),
                    "name,state".into(),
                ],
                stdout: "[{\"id\":\"it's\"}]".into(),
                exit: 8,
            }],
            ..Recording::default()
        };
        let dir = tempfile::tempdir().unwrap();
        program_shims(&rec, dir.path()).unwrap();
        let path = format!(
            "{}:{}",
            dir.path().display(),
            std::env::var("PATH").unwrap_or_default()
        );
        let p = Program {
            argv: vec![
                "sh".into(),
                "-c".into(),
                "gh pr checks --json name,state; true".into(),
            ],
            dir: dir.path().into(),
            env: vec![("PATH".into(), path.clone())],
        };
        let ctx = CallCtx {
            project: String::new(),
            caller: "t".into(),
        };
        let v = one_shot(&p, "rows", &json!({}), &ctx, Duration::from_secs(5)).unwrap();
        assert_eq!(v, json!([{ "id": "it's" }]));
        let p = Program {
            argv: vec!["sh".into(), "-c".into(), "gh pr view".into()],
            ..p
        };
        let e = one_shot(&p, "rows", &json!({}), &ctx, Duration::from_secs(5)).unwrap_err();
        assert!(
            e.message.ends_with("gh pr view: not in the recording"),
            "{e}"
        );
    }

    #[test]
    fn compares_rows_by_id_whatever_their_order() {
        let a = json!([{ "id": "1", "s": "pass" }, { "id": "2", "s": "fail" }]);
        let b = json!([{ "id": "2", "s": "fail" }, { "id": "1", "s": "pass" }]);
        same_rows(&a, &b).unwrap();
        let c = json!([{ "id": "2", "s": "pass" }, { "id": "1", "s": "pass" }]);
        assert!(same_rows(&a, &c).unwrap_err().starts_with("row 2 differs"));
    }
}

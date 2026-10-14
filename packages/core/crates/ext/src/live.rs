//! The real world behind the gate: the network, git, and the credential.
//!
//! vornd hands in its vault ([`Vault`]); `vorn-extc` has none and records
//! with the environment and CLI rungs only. The credential is resolved once
//! per extension and kept in memory; a CLI login borrowed for it is written
//! to the vault so the next start does not run the CLI again.

use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::Mutex;
use std::time::Duration;

use serde_json::Value;

use crate::bridge::{FetchRequest, FetchResponse, World};
use crate::manifest::{AuthRung, Manifest};
use crate::ExtError;

/// One request's limit.
const TIMEOUT: Duration = Duration::from_secs(20);

/// Where an extension's secret fields are kept between runs.
pub trait Vault: Send + Sync {
    fn get(&self, ext: &str, field: &str) -> Option<String>;
    fn set(&self, ext: &str, field: &str, value: &str);
}

pub struct Live {
    vault: Option<Box<dyn Vault>>,
    /// The environment the CLI rung and git run with.
    path: Option<String>,
    tokens: Mutex<HashMap<String, String>>,
    agent: ureq::Agent,
}

impl Live {
    pub fn new(vault: Option<Box<dyn Vault>>, path: Option<String>) -> Live {
        let agent = ureq::Agent::config_builder()
            .timeout_global(Some(TIMEOUT))
            .max_redirects(0)
            .http_status_as_error(false)
            .build()
            .new_agent();
        Live {
            vault,
            path,
            tokens: Mutex::default(),
            agent,
        }
    }

    fn run(&self, argv: &[String], dir: Option<&str>) -> Result<String, ExtError> {
        let (program, args) = argv
            .split_first()
            .ok_or_else(|| ExtError::new("failed", "an empty argv"))?;
        let mut cmd = vorn_spawn::command(program);
        cmd.args(args);
        if let Some(d) = dir {
            cmd.current_dir(PathBuf::from(d));
        }
        if let Some(p) = &self.path {
            cmd.env("PATH", p);
        }
        let out = cmd
            .output()
            .map_err(|e| ExtError::new("failed", format!("cannot run {program}: {e}")))?;
        if !out.status.success() {
            return Err(ExtError::new(
                "failed",
                format!(
                    "{} failed: {}",
                    argv.join(" "),
                    String::from_utf8_lossy(&out.stderr).trim()
                ),
            ));
        }
        Ok(String::from_utf8_lossy(&out.stdout).trim().to_owned())
    }

    fn resolve(&self, ext: &Manifest) -> Option<String> {
        for rung in &ext.auth {
            let found = match rung {
                AuthRung::Vault { field, .. } => {
                    self.vault.as_ref().and_then(|v| v.get(&ext.id, field))
                }
                AuthRung::Env { env, .. } => std::env::var(env).ok().filter(|t| !t.is_empty()),
                AuthRung::Cli { borrow, .. } => {
                    let field = format!("cli:{}", borrow.join(" "));
                    let kept = self.vault.as_ref().and_then(|v| v.get(&ext.id, &field));
                    kept.or_else(|| {
                        let t = self.run(borrow, None).ok().filter(|t| !t.is_empty())?;
                        if let Some(v) = &self.vault {
                            v.set(&ext.id, &field, &t);
                        }
                        Some(t)
                    })
                }
            };
            if found.is_some() {
                return found;
            }
        }
        None
    }
}

impl World for Live {
    fn fetch(&self, req: &FetchRequest) -> Result<FetchResponse, ExtError> {
        let mut b = ureq::http::Request::builder()
            .method(req.method.as_str())
            .uri(req.url.as_str())
            .header("user-agent", "vorn-extension");
        for (n, v) in &req.headers {
            b = b.header(n.as_str(), v.as_str());
        }
        let sent = match &req.body {
            Some(body) => b.body(body.clone()).map(|r| self.agent.run(r)),
            None => b.body(()).map(|r| self.agent.run(r)),
        }
        .map_err(|e| ExtError::new("bad-request", e.to_string()))?;
        let mut res = sent.map_err(|e| ExtError::new("network", e.to_string()))?;
        let status = res.status().as_u16();
        let headers = res
            .headers()
            .iter()
            .filter(|(k, _)| matches!(k.as_str(), "content-type" | "link" | "etag"))
            .map(|(k, v)| {
                (
                    k.as_str().to_owned(),
                    String::from_utf8_lossy(v.as_bytes()).into_owned(),
                )
            })
            .collect();
        let body = res
            .body_mut()
            .read_to_string()
            .map_err(|e| ExtError::new("network", e.to_string()))?;
        Ok(FetchResponse {
            status,
            headers,
            body,
        })
    }

    fn call(&self, method: &str, _: &Value, project: &str) -> Result<Value, ExtError> {
        let argv: &[&str] = match method {
            "git.branch" => &["git", "rev-parse", "--abbrev-ref", "HEAD"],
            "git.remote" => &["git", "remote", "get-url", "origin"],
            other => {
                return Err(ExtError::new(
                    "unknown-method",
                    format!("Vorn has no method {other}"),
                ))
            }
        };
        let argv: Vec<String> = argv.iter().map(|s| (*s).to_owned()).collect();
        self.run(&argv, Some(project)).map(Value::String)
    }

    fn credential(&self, ext: &Manifest) -> Result<Option<String>, ExtError> {
        if let Some(t) = self
            .tokens
            .lock()
            .ok()
            .and_then(|t| t.get(&ext.id).cloned())
        {
            return Ok(Some(t));
        }
        let found = self.resolve(ext);
        if let (Some(t), Ok(mut tokens)) = (&found, self.tokens.lock()) {
            tokens.insert(ext.id.clone(), t.clone());
        }
        Ok(found)
    }
}

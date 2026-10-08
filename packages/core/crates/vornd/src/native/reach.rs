//! The `server:`, `tailscale:`, `token:` and `pairing:` calls, the phone's
//! half of pairing (`/api/pair/redeem` and `/api/pair/poll`), and the
//! credential and Origin checks vornd makes for itself.
//!
//! Tokens are rows in the server's database, read and written from here as
//! a second process ([`vorn_store::DeviceTokens`]). Pairing is held here, in
//! memory, once the `pairing` group is vornd's: the desktop's calls and the
//! phone's HTTP requests both reach this one state, the server relaying the
//! phone's to vornd. What only the server can do, it is told to over the
//! app's channel ([`crate::applink`]): broadcast to every client, and close
//! the sockets holding a token just revoked. Without that channel, the calls
//! that need it are the server's.

use std::io::Read;
use std::path::PathBuf;
use std::process::{Command, Stdio};
use std::sync::{Arc, Mutex, OnceLock, RwLock};
use std::time::{Duration, Instant};

use serde_json::{json, Value};
use vorn_protocol::NewDeviceToken;
use vorn_reach::origin::TrustedHosts;
use vorn_reach::pairing::{Pairing, Poll};
use vorn_reach::{reachable, sys, tailscale, token};
use vorn_store::DeviceTokens;

use super::env::SafeEnv;
use super::{Answer, Native};
use crate::applink::AppLink;

/// How long `tailscale status` may take, as the server allows it.
const TAILSCALE_TIMEOUT: Duration = Duration::from_secs(10);

/// The broadcasts the server makes when a phone asks to pair and collects.
pub const PAIRING_REQUESTED: &str = "pairing:requested";
pub const PAIRING_COLLECTED: &str = "pairing:collected";

/// What the reach calls keep between calls.
#[derive(Debug, Default)]
pub struct Reach {
    pairing: Mutex<Pairing>,
    /// `Some(None)` once looked for and not found; cleared by
    /// `tailscale:status`, as someone may just have installed it.
    tailscale: Mutex<Option<Option<PathBuf>>>,
    trusted: RwLock<TrustedHosts>,
    /// The server's port, which the URLs a browser uses name.
    server_port: OnceLock<u16>,
}

/// What vornd makes of a credential.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Verdict {
    /// The desktop's launch credential, or a live device token.
    Admitted,
    /// Malformed, unknown, tampered with or revoked.
    Refused,
    /// The database could not be read: the server decides.
    CannotTell,
}

fn now_ms() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |d| u64::try_from(d.as_millis()).unwrap_or(u64::MAX))
}

/// `new Date().toISOString()`.
fn now_iso() -> String {
    vorn_store::now_iso()
}

impl Reach {
    pub fn set_server_port(&self, port: u16) {
        let _ = self.server_port.set(port);
    }

    pub(super) fn server_port(&self) -> Option<u16> {
        self.server_port.get().copied()
    }

    /// The names, beyond addresses and `localhost`, a browser may load the
    /// web client from.
    pub fn trusted(&self) -> TrustedHosts {
        self.trusted
            .read()
            .unwrap_or_else(|e| e.into_inner())
            .clone()
    }

    /// Reads them again: this machine's name, and Tailscale's name and
    /// address when it runs. Never fails; an unreadable Tailscale trusts
    /// neither.
    pub fn refresh_trusted(&self, env: &Arc<SafeEnv>) {
        let status = self.tailscale_status(env, None);
        self.trust(&status);
    }

    fn trust(&self, status: &Value) {
        let mut names = Vec::new();
        if status["running"] == Value::Bool(true) {
            for key in ["selfIP", "selfDNSName"] {
                if let Some(s) = status[key].as_str().filter(|s| !s.is_empty()) {
                    names.push(s.to_owned());
                }
            }
        }
        let trusted = TrustedHosts::new(names, &sys::hostname());
        *self.trusted.write().unwrap_or_else(|e| e.into_inner()) = trusted;
    }

    fn tailscale_binary(&self, env: &Arc<SafeEnv>) -> Option<PathBuf> {
        let mut cached = self.tailscale.lock().unwrap_or_else(|e| e.into_inner());
        if let Some(found) = &*cached {
            return found.clone();
        }
        let found = tailscale::BINARY_PATHS
            .iter()
            .map(PathBuf::from)
            .find(|p| is_executable(p))
            .or_else(|| {
                run("which", &["tailscale"], env, TAILSCALE_TIMEOUT)
                    .filter(|s| !s.is_empty())
                    .map(PathBuf::from)
            });
        *cached = Some(found.clone());
        found
    }

    fn tailscale_status(&self, env: &Arc<SafeEnv>, app_port: Option<u16>) -> Value {
        let Some(bin) = self.tailscale_binary(env) else {
            return tailscale::not_installed();
        };
        match run(&bin, &["status", "--json"], env, TAILSCALE_TIMEOUT) {
            Some(printed) => tailscale::status(&printed, app_port),
            None => tailscale::failed(),
        }
    }

    /// The desktop's credential, or a device token checked against the
    /// database at `db`.
    pub fn verify(&self, raw: &str, desktop: Option<&[u8]>, db: Option<&PathBuf>) -> Verdict {
        if desktop.is_some_and(|d| token::constant_time_eq(raw.as_bytes(), d)) {
            return Verdict::Admitted;
        }
        if local_token(db).is_some_and(|t| token::constant_time_eq(raw.as_bytes(), &t)) {
            return Verdict::Admitted;
        }
        let Some(parsed) = token::parse(raw) else {
            return Verdict::Refused;
        };
        let Some(Ok(Some(tokens))) = db.map(|db| DeviceTokens::open(db)) else {
            return Verdict::CannotTell;
        };
        match tokens.secret(parsed.id) {
            Ok(Some(row)) if row.revoked_at.is_none() => {
                if token::secret_matches(parsed.secret, &row.token_hash) {
                    Verdict::Admitted
                } else {
                    Verdict::Refused
                }
            }
            Ok(_) => Verdict::Refused,
            Err(_) => Verdict::CannotTell,
        }
    }
}

/// The server's local credential, which it keeps beside the database.
fn local_token(db: Option<&PathBuf>) -> Option<Vec<u8>> {
    let file = db?.parent()?.join("local-token");
    let token = std::fs::read(file).ok()?;
    let token = token.trim_ascii();
    (!token.is_empty()).then(|| token.to_vec())
}

/// Whether `p` is a file this user may run.
fn is_executable(p: &std::path::Path) -> bool {
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::metadata(p).is_ok_and(|m| m.is_file() && m.permissions().mode() & 0o111 != 0)
    }
    #[cfg(not(unix))]
    {
        p.exists()
    }
}

/// Runs `program` with the safe environment and answers what it printed,
/// trimmed, when it exits cleanly within `timeout`.
fn run(
    program: impl AsRef<std::ffi::OsStr>,
    args: &[&str],
    env: &Arc<SafeEnv>,
    timeout: Duration,
) -> Option<String> {
    let mut child = Command::new(program)
        .args(args)
        .env_clear()
        .envs(env.get())
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .spawn()
        .ok()?;
    let mut stdout = child.stdout.take()?;
    let reader = std::thread::spawn(move || {
        let mut out = Vec::new();
        let _ = stdout.read_to_end(&mut out);
        out
    });
    let deadline = Instant::now() + timeout;
    let status = loop {
        match child.try_wait() {
            Ok(Some(status)) => break status,
            Ok(None) if Instant::now() < deadline => std::thread::sleep(Duration::from_millis(10)),
            _ => {
                let _ = child.kill();
                let _ = child.wait();
                return None;
            }
        }
    };
    let out = reader.join().ok()?;
    status
        .success()
        .then(|| String::from_utf8_lossy(&out).trim().to_owned())
}

/// The error while there is no database of device tokens.
fn no_tokens() -> Answer {
    Answer::Error("vornd has no database of device tokens".to_owned())
}

impl Native {
    pub(super) fn reach_call(&self, method: &str, params: &Value) -> Answer {
        match method {
            "server:reachableUrls" => self.reachable_urls(),
            "tailscale:status" => {
                // Looked for again: it may just have been installed.
                *self
                    .reach
                    .tailscale
                    .lock()
                    .unwrap_or_else(|e| e.into_inner()) = None;
                // Asked without a port until vornd knows the one clients reach.
                let port = self.reach.server_port();
                let status = self.reach.tailscale_status(&self.env, port);
                self.reach.trust(&status);
                Answer::Result(status)
            }
            "token:list" => match self.tokens() {
                Some(t) => match t.list() {
                    Ok(list) => Answer::Result(json!(list)),
                    Err(e) => Answer::Error(e.to_string()),
                },
                None => no_tokens(),
            },
            "token:create" => self.token_create(params),
            "token:revoke" => self.token_revoke(params),
            "pairing:start" => match self.pairing().start(now_ms()) {
                Ok(code) => Answer::Result(code),
                Err(e) => Answer::Error(e.to_string()),
            },
            "pairing:pending" => Answer::Result(Value::Array(self.pairing().pending(now_ms()))),
            "pairing:approve" | "pairing:deny" => {
                // The server's handler destructures its params: anything
                // but an object throws there, and is its to answer.
                let Some(id) = params.as_object().map(|o| o.get("requestId")) else {
                    return super::bad_params(method);
                };
                let id = id.cloned().unwrap_or(Value::Null);
                let mut pairing = self.pairing();
                let ok = if method == "pairing:approve" {
                    pairing.approve(&id, now_ms())
                } else {
                    pairing.deny(&id, now_ms())
                };
                Answer::Result(json!({ "ok": ok }))
            }
            "pairing:cancel" => {
                self.pairing().cancel();
                Answer::Result(json!({ "ok": true }))
            }
            _ => Answer::Forward,
        }
    }

    fn pairing(&self) -> std::sync::MutexGuard<'_, Pairing> {
        self.reach.pairing.lock().unwrap_or_else(|e| e.into_inner())
    }

    fn tokens(&self) -> Option<DeviceTokens> {
        DeviceTokens::open(self.db.get()?).ok().flatten()
    }

    fn reachable_urls(&self) -> Answer {
        let (Some(host), Some(port)) = (
            self.link().and_then(|l| l.server_host()),
            self.reach.server_port(),
        ) else {
            return Answer::Error("vornd does not know where clients reach it yet".to_owned());
        };
        let remote = host == "0.0.0.0";
        let mut tailscale_ips = Vec::new();
        if remote {
            let status = self.reach.tailscale_status(&self.env, None);
            if status["running"] == Value::Bool(true) {
                if let Some(ip) = status["selfIP"].as_str().filter(|s| !s.is_empty()) {
                    tailscale_ips.push(ip.to_owned());
                }
            }
        }
        let lan = if remote {
            sys::lan_addresses()
        } else {
            Vec::new()
        };
        Answer::Result(reachable::reachable_urls(
            port,
            remote,
            &tailscale_ips,
            &lan,
        ))
    }

    /// Mints a token for the owner, as `mintOwnerToken` does: `{token,
    /// plaintext}`, the plaintext only ever here.
    fn mint(&self, name: &str) -> Result<Value, String> {
        let tokens = self
            .tokens()
            .ok_or_else(|| "the database cannot be read".to_owned())?;
        let owner = tokens.owner().map_err(|e| e.to_string())?.ok_or_else(|| {
            "No owner user found. The database may not have been migrated.".to_owned()
        })?;
        let minted = token::mint().map_err(|e| e.to_string())?;
        let created_at = now_iso();
        tokens
            .insert(&NewDeviceToken {
                id: minted.id.clone(),
                user_id: owner.id.clone(),
                name: name.to_owned(),
                token_hash: minted.hash_hex,
                created_at: created_at.clone(),
            })
            .map_err(|e| e.to_string())?;
        Ok(json!({
            "token": {
                "id": minted.id,
                "userId": owner.id,
                "name": name,
                "createdAt": created_at,
                "lastSeenAt": null,
                "revokedAt": null,
            },
            "plaintext": minted.plaintext,
        }))
    }

    fn token_create(&self, params: &Value) -> Answer {
        let Some(fields) = params.as_object() else {
            return super::bad_params("token:create");
        };
        let label = fields
            .get("name")
            .and_then(Value::as_str)
            .map(js_trim)
            .filter(|s| !s.is_empty())
            .unwrap_or("Device");
        if self.tokens().is_none() {
            return no_tokens();
        }
        match self.mint(label) {
            Ok(v) => Answer::Result(v),
            Err(e) => Answer::Error(e),
        }
    }

    fn token_revoke(&self, params: &Value) -> Answer {
        let Some(id) = params.as_str() else {
            return super::bad_params("token:revoke");
        };
        let Some(tokens) = self.tokens() else {
            return no_tokens();
        };
        match tokens.revoke(id, &now_iso()) {
            Ok(revoked) => {
                // The sockets holding it are closed by whoever holds them.
                if let (true, Some(link)) = (revoked, self.link()) {
                    link.tell("vornd:tokenRevoked", json!({ "tokenId": id }));
                }
                Answer::Result(json!({ "revoked": revoked }))
            }
            Err(e) => Answer::Error(e.to_string()),
        }
    }

    /// The phone offers a code: `Ok` with the status and body to answer.
    pub fn pair_redeem(&self, body: &Value, address: &str) -> (u16, Value) {
        let field = |k| {
            body.as_object()
                .and_then(|o| o.get(k))
                .cloned()
                .unwrap_or(Value::Null)
        };
        let (code, name) = (field("code"), field("deviceName"));
        let now = now_ms();
        let mut pairing = self.pairing();
        match pairing.redeem(&code, &name, address, now) {
            Err(refused) => (400, json!({ "error": refused.name() })),
            Ok(id) => {
                if let Some(request) = pairing.pending_one(&id, now) {
                    self.broadcast(PAIRING_REQUESTED, request);
                }
                (200, json!({ "requestId": id }))
            }
        }
    }

    /// The phone asks what came of its request, and collects the token.
    pub fn pair_poll(&self, body: &Value) -> (u16, Value) {
        let id = body
            .as_object()
            .and_then(|o| o.get("requestId"))
            .cloned()
            .unwrap_or(Value::Null);
        let mut pairing = self.pairing();
        match pairing.poll(&id, now_ms()) {
            Poll::Pending => (200, json!({ "status": "pending" })),
            Poll::Denied => (200, json!({ "status": "denied" })),
            Poll::Expired => (200, json!({ "status": "expired" })),
            Poll::Approved { device_name } => {
                let minted = match self.mint(&device_name) {
                    Ok(m) => m,
                    Err(message) => {
                        return (
                            500,
                            json!({ "statusCode": 500, "error": "Internal Server Error", "message": message }),
                        )
                    }
                };
                let id = id.as_str().unwrap_or_default().to_owned();
                pairing.collected(&id);
                drop(pairing);
                self.broadcast(PAIRING_COLLECTED, json!({ "requestId": id }));
                let host = sys::hostname();
                let name = host.strip_suffix(".local").unwrap_or(&host);
                (
                    200,
                    json!({ "status": "approved", "token": minted["plaintext"], "name": name }),
                )
            }
        }
    }

    /// Whether pairing is held here: the server is listening on the app's
    /// channel, to announce a phone asking and collecting.
    pub fn holds_pairing(&self) -> bool {
        self.link().is_some_and(|l| l.listening())
    }

    pub(super) fn link(&self) -> Option<&Arc<AppLink>> {
        self.link.get()
    }

    /// Checks a credential as the server would: the desktop's launch
    /// credential, which is the server's local one, or a device token.
    pub fn verify_credential(&self, raw: &str) -> Verdict {
        let desktop = self.desktop.get().map(Vec::as_slice);
        self.reach.verify(raw, desktop, self.db.get())
    }

    /// Who presents `raw`, for the settings kept per viewer.
    pub fn viewer_of(&self, raw: &str) -> super::config::Viewer {
        super::config::Viewer::of_credential(raw, self.desktop.get().map(Vec::as_slice))
    }
}

/// JavaScript's `trim`: Unicode white space and the byte order mark.
fn js_trim(s: &str) -> &str {
    s.trim_matches(|c: char| c.is_whitespace() || c == '\u{feff}')
}

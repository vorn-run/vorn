//! The bridge's rules for what a program asks of Vorn, enforced here before
//! anything is done: a core method only under the permission it needs, HTTP
//! only to a declared host, and the credential added by vornd, never handed
//! to the program. What lies behind the gate (the sessions, the network, the
//! vault) is a [`World`] vornd provides, so the rules are tested without it.

use std::future::Future;
use std::path::PathBuf;
use std::pin::Pin;
use std::sync::Arc;

use serde_json::Value;

use crate::api::types::{Header, Request, Response, SessionStart};
use crate::api::ExtError;
use crate::manifest::{Manifest, Permission};

/// A future a [`World`] answers with.
pub type Answer<T> = Pin<Box<dyn Future<Output = Result<T, ExtError>> + Send>>;

/// The extension asking, as the world needs to know it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Caller {
    pub extension: String,
    /// The project the call is about, when one is.
    pub project: Option<String>,
}

/// What vornd puts behind the gate.
pub trait World: Send + Sync + 'static {
    /// A core method the gate already allowed.
    fn call(&self, caller: &Caller, method: &str, args: Value) -> Answer<Value>;
    /// A request the gate already allowed and signed.
    fn fetch(&self, caller: &Caller, request: Request) -> Answer<Response>;
    /// The secret `field` of `connection`, which must belong to the caller.
    fn secret(&self, caller: &Caller, connection: &str, field: &str) -> Answer<Option<String>>;
    /// Rows changed in `collection` by `ops` (`{op: upsert, row}`, `{op: delete, id}`, `{op: clear}`); clients drawing it are told.
    fn published(&self, _caller: &Caller, _collection: &str, _ops: &[Value]) {}
    /// A storage call the gate already allowed, on the caller's own tables,
    /// after the package's `migrations` that have not run yet.
    fn storage(
        &self,
        _caller: &Caller,
        _migrations: Option<PathBuf>,
        _call: StorageCall,
    ) -> Answer<Value> {
        Box::pin(async {
            Err(ExtError::new(
                "unsupported",
                "this Vorn keeps no extension tables",
            ))
        })
    }
    /// A session to start, the gate having checked `sessions.launch`; answers its id.
    fn start_session(&self, _caller: &Caller, _req: SessionStart) -> Answer<String> {
        Box::pin(async {
            Err(ExtError::new(
                "unsupported",
                "this Vorn starts no sessions for extensions",
            ))
        })
    }
}

/// One `storage.*` call, read.
#[derive(Debug, Clone, PartialEq)]
pub enum StorageCall {
    Prefix,
    Query { sql: String, params: Vec<Value> },
    Exec { sql: String, params: Vec<Value> },
    Get { key: String },
    Set { key: String, value: Value },
    /// Statements run as one transaction, each `(sql, params)`.
    Transaction(Vec<(String, Vec<Value>)>),
}

/// `host.start-session` and `sessions.launch`: refused unless the manifest declares `sessions.launch`.
pub fn admit_start(manifest: &Manifest) -> Result<(), ExtError> {
    admit_call(manifest, "sessions.launch").map(drop)
}

/// `storage.*` and `blobs.*`: refused unless the manifest declares `storage`.
pub fn admit_storage(manifest: &Manifest) -> Result<(), ExtError> {
    let declared = manifest
        .permissions
        .as_deref()
        .unwrap_or_default()
        .contains(&Permission::Storage);
    if declared {
        return Ok(());
    }
    Err(ExtError::new(
        "denied",
        format!(
            "{} does not declare storage, which storage.* and blobs.* need",
            manifest.id
        ),
    ))
}

/// The core methods a program may call, and the permission each needs. A
/// permission none of these needs (`network.tcp`, `auth.borrow`,
/// `http.routes`, `process.run`) is declared and shown, not enforced:
/// `storage` is held at `storage.*`, the network at `net.fetch`.
///
/// The board, the workflows, their inbox and the sign-in windows are what a
/// connector feeds: items become tasks through `task.importItems`, polled
/// pages go to the inbox with their cursor, and a window signed in again
/// wakes the steps waiting on it. `workflow.fire` fires a trigger type the
/// caller declares.
pub const CORE_METHODS: &[(&str, Permission)] = &[
    ("sessions.list", Permission::SessionsRead),
    ("sessions.get", Permission::SessionsRead),
    ("sessions.launch", Permission::SessionsLaunch),
    ("sessions.close", Permission::SessionsLaunch),
    ("terminal.read", Permission::TerminalRead),
    ("terminal.send", Permission::TerminalSend),
    ("terminal.selection", Permission::TerminalSelection),
    ("card.rename", Permission::CardRename),
    ("agent.usage", Permission::AgentUsage),
    ("git.status", Permission::GitRead),
    ("git.diff", Permission::GitRead),
    ("git.commit", Permission::GitWrite),
    ("worktrees.list", Permission::GitRead),
    ("worktrees.remove", Permission::GitWrite),
    ("worktrees.reclaim", Permission::GitWrite),
    ("worktrees.prune", Permission::GitWrite),
    ("permissions.resolve", Permission::AgentPermissions),
    ("files.read", Permission::FilesRead),
    ("files.write", Permission::FilesWrite),
    ("secrets.get", Permission::SecretsRead),
    ("task.list", Permission::TasksRead),
    ("task.get", Permission::TasksRead),
    ("agents.list", Permission::SessionsRead),
    ("task.importItems", Permission::TasksWrite),
    ("task.take", Permission::TasksWrite),
    ("task.handBack", Permission::TasksWrite),
    ("connection.upsertFromItem", Permission::TasksWrite),
    ("workflow.fire", Permission::WorkflowsWrite),
    ("task.sourceLink", Permission::TasksRead),
    ("project.list", Permission::TasksRead),
    ("workflow.get", Permission::WorkflowsRead),
    ("workflow.list", Permission::WorkflowsRead),
    ("workflow.seed", Permission::WorkflowsWrite),
    ("workflow.unseed", Permission::WorkflowsWrite),
    ("inbox.cursor", Permission::InboxWrite),
    ("inbox.recordPage", Permission::InboxWrite),
    ("inbox.recordError", Permission::InboxWrite),
    ("signin.check", Permission::BrowserSession),
    ("signin.forget", Permission::BrowserSession),
    ("signin.completed", Permission::BrowserSession),
];

/// The core methods open only to the extensions Vorn ships: they write the
/// board, the workflows, their inbox and the sign-in windows of every
/// connection, which no one installed extension owns. `task.take` and
/// `task.handBack` are Tasks' and `connection.upsertFromItem` Connectors',
/// called by the other built-ins.
pub const BUILTIN_ONLY: &[&str] = &[
    "task.importItems",
    "task.take",
    "task.handBack",
    "connection.upsertFromItem",
    "task.sourceLink",
    "workflow.get",
    "workflow.list",
    "workflow.seed",
    "workflow.unseed",
    "inbox.cursor",
    "inbox.recordPage",
    "inbox.recordError",
    "signin.check",
    "signin.forget",
    "signin.completed",
];

/// Whether the bridge holds a program to `permission`: some method or call needs it.
pub fn enforced(permission: Permission) -> bool {
    permission == Permission::Storage || CORE_METHODS.iter().any(|(_, p)| *p == permission)
}

/// Headers a program may not set: credentials come from vornd alone.
const WITHHELD: [&str; 4] = [
    "authorization",
    "cookie",
    "proxy-authorization",
    "x-api-key",
];

/// `host.call`: refused unless the manifest declares what the method needs.
pub fn admit_call(manifest: &Manifest, method: &str) -> Result<Permission, ExtError> {
    let Some((_, permission)) = CORE_METHODS.iter().find(|(m, _)| *m == method) else {
        return Err(ExtError::new(
            "unknown-method",
            format!("Vorn has no method {method}"),
        ));
    };
    let declared = manifest
        .permissions
        .as_deref()
        .unwrap_or_default()
        .contains(permission);
    if !declared {
        return Err(ExtError::new(
            "denied",
            format!(
                "{} does not declare {}, which {method} needs",
                manifest.id,
                permission.name()
            ),
        ));
    }
    Ok(*permission)
}

/// The host of an `https://` URL, lowercase and without its port; `None`
/// for any other scheme or a URL that carries credentials of its own.
pub fn https_host(url: &str) -> Option<String> {
    let rest = url
        .get(..8)
        .filter(|s| s.eq_ignore_ascii_case("https://"))
        .map(|_| &url[8..])?;
    let authority = &rest[..rest.find(['/', '?', '#']).unwrap_or(rest.len())];
    if authority.is_empty() || authority.contains('@') {
        return None;
    }
    let host = match authority.rsplit_once(':') {
        Some((h, port)) if port.bytes().all(|b| b.is_ascii_digit()) => h,
        _ => authority,
    };
    (!host.is_empty()).then(|| host.to_ascii_lowercase())
}

/// Whether `host` is one the manifest declares, `*.example.com` covering its subdomains.
pub fn declared_host(network: &[String], host: &str) -> bool {
    network.iter().any(|d| match d.strip_prefix("*.") {
        Some(parent) => host
            .strip_suffix(parent)
            .is_some_and(|sub| sub.ends_with('.') && sub.len() > 1),
        None => d == host,
    })
}

/// `net.fetch`: https to a declared host, the program's own credentials
/// dropped and vornd's added from `connection` when the manifest names one.
pub async fn fetch(
    world: &Arc<dyn World>,
    manifest: &Manifest,
    caller: &Caller,
    request: Request,
    connection: Option<String>,
) -> Result<Response, ExtError> {
    let host = https_host(&request.url).ok_or_else(|| {
        ExtError::new(
            "denied",
            format!("{} is not an https URL net.fetch may reach", request.url),
        )
    })?;
    if !declared_host(&manifest.declares.network, &host) {
        return Err(ExtError::new(
            "denied",
            format!("{} does not declare the host {host}", manifest.id),
        ));
    }
    let mut signed = Request {
        headers: request
            .headers
            .into_iter()
            .filter(|h| !WITHHELD.contains(&h.name.to_ascii_lowercase().as_str()))
            .collect(),
        ..request
    };
    if let (Some(credential), Some(connection)) = (&manifest.credential, &connection) {
        let secret = world
            .secret(caller, connection, &credential.field)
            .await?
            .ok_or_else(|| {
                ExtError::new(
                    "signed-out",
                    format!("the connection {connection} holds no {}", credential.field),
                )
            })?;
        if let Some((name, value)) = credential.header.split_once(':') {
            signed.headers.push(Header {
                name: name.trim().to_owned(),
                value: value.trim().replace("{token}", &secret),
            });
        }
    }
    world.fetch(caller, signed).await
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Mutex;

    #[derive(Default)]
    struct Seen(Mutex<Vec<Request>>);

    impl World for Seen {
        fn call(&self, caller: &Caller, _: &str, _: Value) -> Answer<Value> {
            let ext = caller.extension.clone();
            Box::pin(async move { Ok(Value::String(ext)) })
        }
        fn fetch(&self, _: &Caller, request: Request) -> Answer<Response> {
            self.0.lock().unwrap().push(request);
            Box::pin(async {
                Ok(Response {
                    status: 200,
                    headers: Vec::new(),
                    body: "{}".into(),
                })
            })
        }
        fn secret(&self, _: &Caller, connection: &str, _: &str) -> Answer<Option<String>> {
            let known = (connection == "c1").then(|| "t0ken".to_owned());
            Box::pin(async move { Ok(known) })
        }
    }

    fn manifest() -> Manifest {
        crate::manifest::file::parse(
            r#"id = "x"
name = "X"
version = "1.0.0"
api = "vorn:extension@0.9"
[service]
command = ["x"]
[declares]
network = ["api.example.com", "*.cdn.example.com"]
uses = ["git.read"]
[credential]
field = "token"
header = "Authorization: Bearer {token}"
"#,
        )
        .unwrap()
    }

    fn get(url: &str) -> Request {
        Request {
            method: "GET".into(),
            url: url.into(),
            headers: vec![
                Header {
                    name: "Authorization".into(),
                    value: "Bearer stolen".into(),
                },
                Header {
                    name: "Accept".into(),
                    value: "application/json".into(),
                },
            ],
            body: None,
        }
    }

    fn caller() -> Caller {
        Caller {
            extension: "x".into(),
            project: None,
        }
    }

    #[tokio::test]
    async fn signs_a_declared_host_with_vornds_credential_not_the_programs() {
        let seen = Arc::new(Seen::default());
        let world: Arc<dyn World> = seen.clone();
        let m = manifest();
        fetch(
            &world,
            &m,
            &caller(),
            get("https://api.example.com/v1"),
            Some("c1".into()),
        )
        .await
        .unwrap();
        let sent = seen.0.lock().unwrap().remove(0);
        let headers: Vec<(&str, &str)> = sent
            .headers
            .iter()
            .map(|h| (h.name.as_str(), h.value.as_str()))
            .collect();
        assert_eq!(
            headers,
            [
                ("Accept", "application/json"),
                ("Authorization", "Bearer t0ken")
            ]
        );
        let err = fetch(
            &world,
            &m,
            &caller(),
            get("https://api.example.com/"),
            Some("c9".into()),
        )
        .await
        .unwrap_err();
        assert_eq!(err.code, "signed-out");
    }

    #[tokio::test]
    async fn refuses_a_host_it_does_not_declare_by_name() {
        let seen = Arc::new(Seen::default());
        let world: Arc<dyn World> = seen.clone();
        let m = manifest();
        for url in [
            "https://evil.example/x",
            "http://api.example.com/x",
            "https://api.example.com@evil.example/x",
            "https://api.example.com.evil.example/",
            "https://cdn.example.com/",
        ] {
            let err = fetch(&world, &m, &caller(), get(url), None)
                .await
                .unwrap_err();
            assert_eq!(err.code, "denied", "{url}");
        }
        let err = fetch(&world, &m, &caller(), get("https://evil.example/x"), None)
            .await
            .unwrap_err();
        assert_eq!(err.message, "x does not declare the host evil.example");
        assert!(seen.0.lock().unwrap().is_empty());
        fetch(
            &world,
            &m,
            &caller(),
            get("https://a.cdn.example.com:443/x"),
            None,
        )
        .await
        .unwrap();
    }

    #[test]
    fn holds_core_calls_to_the_declared_permissions() {
        let m = manifest();
        assert_eq!(admit_call(&m, "git.status"), Ok(Permission::GitRead));
        let err = admit_call(&m, "sessions.list").unwrap_err();
        assert_eq!(
            (err.code.as_str(), err.message.as_str()),
            (
                "denied",
                "x does not declare sessions.read, which sessions.list needs"
            )
        );
        assert_eq!(
            admit_call(&m, "fs.read").unwrap_err().code,
            "unknown-method"
        );
    }

    #[test]
    fn holds_storage_to_the_storage_permission() {
        let err = admit_storage(&manifest()).unwrap_err();
        assert_eq!(
            (err.code.as_str(), err.message.as_str()),
            (
                "denied",
                "x does not declare storage, which storage.* and blobs.* need"
            )
        );
        let mut m = manifest();
        m.permissions = Some(vec![Permission::Storage]);
        assert_eq!(admit_storage(&m), Ok(()));
        assert!(enforced(Permission::Storage));
        assert!(enforced(Permission::GitWrite));
        assert!(!enforced(Permission::ProcessRun));
    }

    #[test]
    fn reads_the_host_of_an_https_url() {
        assert_eq!(
            https_host("https://API.example.com:443/x?y").as_deref(),
            Some("api.example.com")
        );
        assert_eq!(https_host("https:///x"), None);
        assert_eq!(https_host("https://u@a.com/"), None);
        assert!(declared_host(&["*.a.com".into()], "b.a.com"));
        assert!(!declared_host(&["*.a.com".into()], "a.com"));
        assert!(!declared_host(&["*.a.com".into()], "ba.com"));
    }
}

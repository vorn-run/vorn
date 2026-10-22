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
    /// Declares `process.run`: a workflow it seeds may run commands and
    /// agents, and it may run workflows not its own.
    pub runs_commands: bool,
    /// One Vorn ships, compiled in or installed as the pack this build pins:
    /// the only kind that may have a reserved id ([`crate::pack::is_reserved`])
    /// and own that namespace.
    pub builtin: bool,
}

impl Caller {
    /// The installed or linked extension `manifest` describes, about no project.
    pub fn of(manifest: &Manifest) -> Caller {
        Caller {
            extension: manifest.id.clone(),
            project: None,
            runs_commands: declares(manifest, Permission::ProcessRun),
            builtin: false,
        }
    }

    /// The built-in `manifest` describes, which Vorn itself ships.
    pub fn builtin(manifest: &Manifest) -> Caller {
        Caller {
            builtin: true,
            ..Caller::of(manifest)
        }
    }

    /// Whether workflow `id` is in its namespace: `<extension id>:`. An
    /// installed extension with a reserved id owns nothing, so Vorn's own
    /// workflows and its built-ins' stay theirs.
    pub fn owns(&self, id: &str) -> bool {
        (self.builtin || !crate::pack::is_reserved(&self.extension))
            && id
                .strip_prefix(self.extension.as_str())
                .is_some_and(|rest| rest.starts_with(':'))
    }

    fn namespace(&self) -> String {
        format!("{}:", self.extension)
    }
}

fn declares(manifest: &Manifest, permission: Permission) -> bool {
    manifest
        .permissions
        .as_deref()
        .unwrap_or_default()
        .contains(&permission)
}

/// `workflow.seed` and `workflow.unseed`: an id or prefix in the caller's
/// namespace, so an extension adds and removes only its own workflows.
pub fn admit_own(caller: &Caller, method: &str, id: &str) -> Result<(), ExtError> {
    if caller.owns(id) {
        return Ok(());
    }
    Err(ExtError::new(
        "denied",
        format!(
            "{method}: {id} is not {}'s; its workflows' ids start with {}",
            caller.extension,
            caller.namespace()
        ),
    ))
}

/// `workflow.run`: the caller's own workflows; any other, whose steps may
/// run commands, only with `process.run`.
pub fn admit_run(caller: &Caller, id: &str) -> Result<(), ExtError> {
    if caller.runs_commands || caller.owns(id) {
        return Ok(());
    }
    Err(ExtError::new(
        "denied",
        format!(
            "{} does not declare process.run, which workflow.run needs for {id}, a workflow not its own",
            caller.extension
        ),
    ))
}

/// `workflow.seed`'s `workflow`: in the caller's namespace, and with a step
/// that runs commands or agents (`runs_commands` says which node types do)
/// only with `process.run`.
pub fn admit_seed(
    caller: &Caller,
    workflow: &Value,
    runs_commands: impl Fn(&str) -> bool,
) -> Result<(), ExtError> {
    let id = workflow.get("id").and_then(Value::as_str).unwrap_or("");
    admit_own(caller, "workflow.seed", id)?;
    if caller.runs_commands {
        return Ok(());
    }
    match first_node_type(workflow, &runs_commands) {
        None => Ok(()),
        Some(step) => Err(ExtError::new(
            "denied",
            format!(
                "{} does not declare process.run, which workflow.seed needs for {id}: its {step} step runs commands",
                caller.extension
            ),
        )),
    }
}

/// The first node type `pick` takes, of the nodes at any depth of `value`:
/// a workflow's own and those of any workflow nested in its configs.
fn first_node_type(value: &Value, pick: &impl Fn(&str) -> bool) -> Option<String> {
    match value {
        Value::Object(map) => {
            let nodes = map.get("nodes").and_then(Value::as_array);
            fn kind(n: &Value) -> &str {
                n.get("type").and_then(Value::as_str).unwrap_or("")
            }
            if let Some(step) = nodes.into_iter().flatten().map(kind).find(|t| pick(t)) {
                return Some(step.to_owned());
            }
            map.values().find_map(|v| first_node_type(v, pick))
        }
        Value::Array(items) => items.iter().find_map(|v| first_node_type(v, pick)),
        _ => None,
    }
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
    Query {
        sql: String,
        params: Vec<Value>,
    },
    Exec {
        sql: String,
        params: Vec<Value>,
    },
    Get {
        key: String,
    },
    Set {
        key: String,
        value: Value,
    },
    /// Statements run as one transaction, each `(sql, params)`.
    Transaction(Vec<(String, Vec<Value>)>),
}

/// `host.start-session` and `sessions.launch`: refused unless the manifest
/// declares `sessions.launch` and `process.run`, since a session runs a program.
pub fn admit_start(manifest: &Manifest) -> Result<(), ExtError> {
    admit_call(manifest, "sessions.launch").map(drop)
}

/// The core methods that also need `process.run`: what they start runs commands.
const RUNS_COMMANDS: &[&str] = &["sessions.launch"];

/// `permissions.resolve`: answering an agent's request is answering for the
/// person, so it is allowed while the extension answers a person's own click
/// (`by_person`), and otherwise only with `process.run`.
pub fn admit_resolve(caller: &Caller, by_person: bool) -> Result<(), ExtError> {
    if by_person || caller.runs_commands {
        return Ok(());
    }
    Err(ExtError::new(
        "denied",
        format!(
            "{} answers an agent's request only as a person clicks; on its own that needs process.run",
            caller.extension
        ),
    ))
}

/// `terminal.read`, `terminal.send` and `terminal.selection` on a session:
/// one the caller `started`; any other only with `process.run` and a pane of
/// its own open on that session (`paned`).
pub fn admit_terminal(
    caller: &Caller,
    method: &str,
    started: bool,
    paned: bool,
) -> Result<(), ExtError> {
    if started || (caller.runs_commands && paned) {
        return Ok(());
    }
    let why = if caller.runs_commands {
        "it has no pane open on it"
    } else {
        "it does not declare process.run"
    };
    Err(ExtError::new(
        "denied",
        format!(
            "{} reaches with {method} only the sessions it started; not this one, as {why}",
            caller.extension
        ),
    ))
}

/// The page ops `web.call` passes to the client: reading, finding and acting
/// on what the page shows. None runs a script of the extension's own.
pub const WEB_OPS: &[&str] = &[
    "load", "history", "read", "text", "find", "act", "console", "network",
];

/// The simulator ops `video.call` passes to the client.
pub const VIDEO_OPS: &[&str] = &[
    "list",
    "boot",
    "shutdown",
    "launch",
    "terminate",
    "openUrl",
    "install",
    "screenshot",
    "save",
];

/// `web.call` and `video.call`: only an op its permission lists.
pub fn admit_op(method: &str, op: &str) -> Result<(), ExtError> {
    let ops = match method {
        "web.call" => WEB_OPS,
        "video.call" => VIDEO_OPS,
        _ => &[],
    };
    if ops.contains(&op) {
        return Ok(());
    }
    Err(ExtError::new("denied", format!("{method} has no op {op}")))
}

/// `storage.*` and `blobs.*`: refused unless the manifest declares `storage`.
pub fn admit_storage(manifest: &Manifest) -> Result<(), ExtError> {
    if declares(manifest, Permission::Storage) {
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
/// `http.routes`) is declared and shown, not enforced: `storage` is held at
/// `storage.*`, the network at `net.fetch`, and `process.run` at the
/// workflows it seeds and runs ([`admit_seed`], [`admit_run`]), the sessions
/// it starts, and what it reaches beyond its own ([`admit_resolve`],
/// [`admit_terminal`], [`crate::reach`]).
///
/// The board, the workflows, their inbox and the sign-in windows are what a
/// connector feeds: items become tasks through `task.importItems`, polled
/// pages go to the inbox with their cursor, and a window signed in again
/// wakes the steps waiting on it. `workflow.fire` fires a trigger type the
/// caller declares.
///
/// This is the whole bridge: the extensions Vorn ships call nothing an
/// installed one could not, each held to the same declared permission.
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
    ("files.list", Permission::FilesRead),
    ("files.stat", Permission::FilesRead),
    ("shell.executables", Permission::FilesRead),
    ("git.branches", Permission::GitRead),
    ("files.write", Permission::FilesWrite),
    ("secrets.get", Permission::SecretsRead),
    ("task.list", Permission::TasksRead),
    ("task.get", Permission::TasksRead),
    ("agents.list", Permission::SessionsRead),
    ("task.importItems", Permission::TasksWrite),
    ("task.changed", Permission::TasksWrite),
    ("task.take", Permission::TasksWrite),
    ("task.handBack", Permission::TasksWrite),
    ("connection.itemImport", Permission::TasksWrite),
    ("connection.synced", Permission::TasksWrite),
    ("workflow.fire", Permission::WorkflowsWrite),
    ("task.sourceLink", Permission::TasksRead),
    ("project.list", Permission::TasksRead),
    ("workflow.get", Permission::WorkflowsRead),
    ("workflow.list", Permission::WorkflowsRead),
    ("workflow.gates", Permission::WorkflowsRead),
    ("workflow.run", Permission::WorkflowsWrite),
    ("workflow.resolveGate", Permission::WorkflowsWrite),
    ("workflow.seed", Permission::WorkflowsWrite),
    ("workflow.unseed", Permission::WorkflowsWrite),
    ("inbox.cursor", Permission::InboxWrite),
    ("inbox.recordPage", Permission::InboxWrite),
    ("inbox.recordError", Permission::InboxWrite),
    ("signin.check", Permission::BrowserSession),
    ("signin.forget", Permission::BrowserSession),
    ("signin.completed", Permission::BrowserSession),
    ("web.call", Permission::Web),
    ("video.call", Permission::Video),
    ("remote.hosts", Permission::Remote),
    ("remote.saveHosts", Permission::Remote),
    ("remote.test", Permission::Remote),
    ("remote.keys", Permission::Remote),
    ("remote.storeKey", Permission::Remote),
    ("remote.importKey", Permission::Remote),
    ("remote.deleteKey", Permission::Remote),
    ("remote.vault", Permission::Remote),
];

/// Whether the bridge holds a program to `permission`: some method or call
/// needs it. `process.run` is held at the workflows a program seeds and runs.
pub fn enforced(permission: Permission) -> bool {
    matches!(permission, Permission::Storage | Permission::ProcessRun)
        || CORE_METHODS.iter().any(|(_, p)| *p == permission)
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
    if RUNS_COMMANDS.contains(&method) && !declares(manifest, Permission::ProcessRun) {
        return Err(ExtError::new(
            "denied",
            format!(
                "{} does not declare process.run, which {method} needs",
                manifest.id
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
api = "vorn:extension@1.0"
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
        Caller::of(&manifest())
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
    fn what_the_shipped_editors_call_is_held_to_a_declared_permission() {
        let mut m = manifest();
        for (method, needs) in [
            ("workflow.run", Permission::WorkflowsWrite),
            ("workflow.gates", Permission::WorkflowsRead),
            ("task.changed", Permission::TasksWrite),
            ("remote.hosts", Permission::Remote),
        ] {
            assert_eq!(
                admit_call(&m, method).unwrap_err().code,
                "denied",
                "{method}"
            );
            m.permissions.get_or_insert_with(Vec::new).push(needs);
            assert_eq!(admit_call(&m, method), Ok(needs), "{method}");
        }
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
    fn holds_workflows_to_the_callers_namespace_and_commands_to_process_run() {
        use serde_json::json;
        let x = caller();
        assert!(x.owns("x:a") && !x.owns("xy:a") && !x.owns("x") && !x.owns("import:a"));
        for reserved in crate::pack::RESERVED_IDS
            .into_iter()
            .chain(["vorn.connectors"])
        {
            let posing = Caller {
                extension: reserved.into(),
                ..x.clone()
            };
            let theirs = format!("{reserved}:c1:issues");
            assert!(!posing.owns(&theirs), "{reserved}");
            assert!(admit_own(&posing, "workflow.unseed", &theirs).is_err());
            assert!(admit_run(&posing, &theirs).is_err());
        }
        let shipped = Caller {
            extension: "vorn.connectors".into(),
            builtin: true,
            ..x.clone()
        };
        assert!(shipped.owns("vorn.connectors:c1:issues"));
        assert_eq!(admit_own(&x, "workflow.unseed", "x:conn:"), Ok(()));
        let foreign = admit_own(&x, "workflow.unseed", "import:").unwrap_err();
        assert_eq!(
            (foreign.code.as_str(), foreign.message.as_str()),
            (
                "denied",
                "workflow.unseed: import: is not x's; its workflows' ids start with x:"
            )
        );

        assert_eq!(admit_run(&x, "x:mine"), Ok(()));
        let theirs = admit_run(&x, "wf-1").unwrap_err();
        assert_eq!(
            theirs.message,
            "x does not declare process.run, which workflow.run needs for wf-1, a workflow not its own"
        );

        let runs = |t: &str| t == "script";
        let wf = |id: &str, step: &str| json!({ "id": id, "nodes": [{ "type": "trigger" }, { "type": step }] });
        assert_eq!(admit_seed(&x, &wf("x:a", "createTask"), runs), Ok(()));
        let elsewhere = admit_seed(&x, &wf("y:a", "createTask"), runs).unwrap_err();
        assert!(elsewhere
            .message
            .starts_with("workflow.seed: y:a is not x's"));
        let shell = admit_seed(&x, &wf("x:a", "script"), runs).unwrap_err();
        assert_eq!(
            shell.message,
            "x does not declare process.run, which workflow.seed needs for x:a: its script step runs commands"
        );

        let commands = Caller {
            runs_commands: true,
            ..caller()
        };
        assert_eq!(admit_run(&commands, "wf-1"), Ok(()));
        assert_eq!(admit_seed(&commands, &wf("x:a", "script"), runs), Ok(()));
        assert!(admit_seed(&commands, &wf("y:a", "script"), runs).is_err());
    }

    #[test]
    fn a_seeded_workflow_is_checked_at_every_depth() {
        let x = Caller {
            extension: "x".into(),
            project: None,
            runs_commands: false,
            builtin: false,
        };
        let runs = |t: &str| t == "script";
        let nested = serde_json::json!({
            "id": "x:a",
            "nodes": [
                { "type": "trigger" },
                { "type": "loop", "config": { "workflow": { "nodes": [
                    { "type": "createTask" },
                    { "type": "group", "nodes": [{ "type": "script" }] }
                ] } } }
            ]
        });
        let err = admit_seed(&x, &nested, runs).unwrap_err();
        assert_eq!(
            err.message,
            "x does not declare process.run, which workflow.seed needs for x:a: its script step runs commands"
        );
        let plain = serde_json::json!({ "id": "x:a", "nodes": [{ "type": "loop", "config": { "nodes": [{ "type": "createTask" }] } }] });
        assert_eq!(admit_seed(&x, &plain, runs), Ok(()));
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
        assert!(enforced(Permission::ProcessRun));
        assert!(!enforced(Permission::NetworkTcp));
    }

    #[test]
    fn starting_a_session_needs_process_run_and_closing_one_does_not() {
        let mut m = manifest();
        m.permissions = Some(vec![Permission::SessionsLaunch]);
        let err = admit_start(&m).unwrap_err();
        assert_eq!(
            (err.code.as_str(), err.message.as_str()),
            (
                "denied",
                "x does not declare process.run, which sessions.launch needs"
            )
        );
        assert_eq!(
            admit_call(&m, "sessions.close"),
            Ok(Permission::SessionsLaunch)
        );
        m.permissions
            .get_or_insert_with(Vec::new)
            .push(Permission::ProcessRun);
        assert_eq!(admit_start(&m), Ok(()));
    }

    #[test]
    fn answers_an_agents_request_only_as_a_person_clicks_or_with_process_run() {
        let x = caller();
        assert_eq!(admit_resolve(&x, true), Ok(()));
        let err = admit_resolve(&x, false).unwrap_err();
        assert_eq!(err.code, "denied");
        assert!(err.message.contains("process.run"), "{}", err.message);
        let commands = Caller {
            runs_commands: true,
            ..caller()
        };
        assert_eq!(admit_resolve(&commands, false), Ok(()));
    }

    #[test]
    fn reaches_the_terminals_of_sessions_it_started_or_has_a_pane_on_with_process_run() {
        let x = caller();
        assert_eq!(admit_terminal(&x, "terminal.send", true, false), Ok(()));
        for paned in [false, true] {
            let err = admit_terminal(&x, "terminal.read", false, paned).unwrap_err();
            assert_eq!(err.code, "denied");
            assert!(err.message.contains("process.run"), "{}", err.message);
        }
        let commands = Caller {
            runs_commands: true,
            ..caller()
        };
        assert_eq!(
            admit_terminal(&commands, "terminal.send", false, true),
            Ok(())
        );
        let err = admit_terminal(&commands, "terminal.send", false, false).unwrap_err();
        assert!(err.message.contains("no pane"), "{}", err.message);
    }

    #[test]
    fn passes_only_the_ops_each_permission_lists() {
        for op in WEB_OPS {
            assert_eq!(admit_op("web.call", op), Ok(()));
        }
        for op in VIDEO_OPS {
            assert_eq!(admit_op("video.call", op), Ok(()));
        }
        for (method, op) in [
            ("web.call", "eval"),
            ("web.call", "evaluate"),
            ("web.call", "list"),
            ("video.call", "load"),
            ("video.call", "erase"),
        ] {
            let err = admit_op(method, op).unwrap_err();
            assert_eq!(
                (err.code.as_str(), err.message),
                ("denied", format!("{method} has no op {op}"))
            );
        }
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

use std::fs;
use std::net::SocketAddr;
use std::path::Path;

use bytes::Bytes;
use http_body_util::{BodyExt, Full};
use hyper::{Request, StatusCode};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use vorn_extensions::bridge::Route;
use vorn_extensions::gate::{Answer as WorldAnswer, Caller, World as GateWorld};
use vorn_extensions::host::HostSettings;
use vorn_extensions::lifecycle::{Lifecycle, Policy};
use vorn_extensions::pack::PackStore;
use vorn_extensions::router::RouterSettings;

use super::*;

const FIXTURE: &str = include_str!("../../../../extensions/tests/fixtures/extension.mjs");

#[derive(Default)]
struct Fake {
    live: Mutex<Vec<Live>>,
    notes: OnceLock<broadcast::Sender<Value>>,
    told: Mutex<Vec<(String, Value, String)>>,
    started: Mutex<Vec<(String, SpawnSpec)>>,
    killed: Mutex<Vec<String>>,
    written: Mutex<Vec<(String, String)>>,
    renamed: Mutex<Vec<(String, String)>>,
}

impl Fake {
    fn told(&self, method: &str) -> Vec<Value> {
        guard(&self.told)
            .iter()
            .filter(|(m, _, _)| m == method)
            .map(|(_, p, _)| p.clone())
            .collect()
    }

    fn note(&self) {
        if let Some(notes) = self.notes.get() {
            let _ = notes.send(json!({}));
        }
    }
}

impl Around for Fake {
    fn live(&self) -> Option<Vec<Live>> {
        Some(guard(&self.live).clone())
    }

    fn notes(&self) -> Option<broadcast::Receiver<Value>> {
        Some(
            self.notes
                .get_or_init(|| broadcast::channel(16).0)
                .subscribe(),
        )
    }

    fn rename(&self, id: &str, name: &str) -> Result<(), String> {
        guard(&self.renamed).push((id.into(), name.into()));
        Ok(())
    }

    fn broadcast(&self, method: &str, params: Value, scope: &str) {
        guard(&self.told).push((method.into(), params, scope.into()));
    }

    fn base_env(&self) -> Vec<(String, String)> {
        vec![("PATH".into(), "/bin".into())]
    }

    fn start_terminal(
        &self,
        id: String,
        spec: SpawnSpec,
    ) -> Result<oneshot::Receiver<Result<u32, String>>, String> {
        let (tx, rx) = oneshot::channel();
        let answer = match spec.argv[0].as_str() {
            "missing" => Err("no such program".to_owned()),
            _ => Ok(7),
        };
        guard(&self.started).push((id, spec));
        let _ = tx.send(answer);
        Ok(rx)
    }

    fn kill_terminal(&self, id: &str) {
        guard(&self.killed).push(id.into());
    }

    fn read_output(
        &self,
        _id: &str,
        lines: usize,
    ) -> BoxFuture<'static, Result<Vec<String>, String>> {
        Box::pin(async move { Ok((0..lines.min(3)).map(|i| format!("line {i}")).collect()) })
    }

    fn write(&self, id: &str, text: &str) -> BoxFuture<'static, Result<(), String>> {
        guard(&self.written).push((id.into(), text.into()));
        Box::pin(async { Ok(()) })
    }

    fn git(&self) -> Git {
        Git {
            bin: "git".into(),
            env: std::env::vars().filter(|(k, _)| k == "PATH").collect(),
            ssh: None,
        }
    }
}

fn install(root: &Path, id: &str, manifest: &str) {
    let dir = root.join(id).join("1.0.0");
    fs::create_dir_all(dir.join("web")).unwrap();
    fs::write(dir.join("index.js"), FIXTURE).unwrap();
    fs::write(dir.join("package.json"), r#"{"type":"module"}"#).unwrap();
    fs::write(dir.join("web/index.html"), "<p>pane</p>").unwrap();
    fs::write(dir.join(vorn_extensions::manifest::file::FILE), manifest).unwrap();
    fs::write(
        root.join(id).join("current.json"),
        json!({ "version": "1.0.0", "installedAt": 1 }).to_string(),
    )
    .unwrap();
}

fn extension(id: &str) -> String {
    let name = id.to_uppercase();
    format!(
        r#"id = "{id}"
name = "{name}"
version = "1.0.0"
api = "vorn:extension@1.0"

[service]
command = ["node", "index.js"]

[declares]
uses = ["git.read", "terminal.read", "terminal.selection", "terminal.send", "card.rename", "agent.usage"]

[connector]

[[contributes.panes]]
id = "p"
title = "Page"
web = "web/index.html"

[[contributes.panes]]
id = "prog"
title = "Program"
command = ["lazygit", "-p"]

[[contributes.panes]]
id = "broken"
title = "Broken"
command = ["missing"]

[[contributes.footers]]
id = "f"
every = 5.0

[[contributes.link_handlers]]
id = "l"
pattern = 'PROJ-\d+'
"#
    )
}

struct World {
    packs: tempfile::TempDir,
    project: tempfile::TempDir,
    fake: Arc<Fake>,
    ext: Arc<Extensions>,
}

impl World {
    fn project(&self) -> String {
        self.project.path().to_string_lossy().into_owned()
    }

    fn session(&self, id: &str) -> Live {
        Live {
            id: id.into(),
            agent: "acme".into(),
            project_path: self.project(),
            worktree_path: None,
            agent_session_id: None,
            renamed_by_person: false,
        }
    }

    fn add(&self, session: Live) {
        guard(&self.fake.live).push(session);
    }

    fn key(&self, id: &str) -> HostKey {
        HostKey {
            extension_id: id.into(),
            project_path: self.project(),
        }
    }

    async fn call(&self, method: &str, params: Value) -> Answer {
        self.ext.answer(method, &params).await
    }

    async fn token(&self, id: &str) -> String {
        drop(self.ext.hosts.get_or_start(&self.key(id)).await.unwrap());
        self.ext.hosts.token_for(&self.key(id)).unwrap()
    }
}

/// Programs get nothing from this world; the card extensions here do not ask it.
struct NoWorld;

impl GateWorld for NoWorld {
    fn call(&self, _: &Caller, method: &str, _: Value) -> WorldAnswer<Value> {
        let err = ExtError::new("unknown-method", format!("no {method} here"));
        Box::pin(async move { Err(err) })
    }
    fn fetch(
        &self,
        _: &Caller,
        _: vorn_extensions::api::types::Request,
    ) -> WorldAnswer<vorn_extensions::api::types::Response> {
        Box::pin(async { Err(ExtError::new("network", "no network here")) })
    }
    fn secret(&self, _: &Caller, _: &str, _: &str) -> WorldAnswer<Option<String>> {
        Box::pin(async { Ok(None) })
    }
}

fn world() -> World {
    let packs = tempfile::tempdir().unwrap();
    install(packs.path(), "x", &extension("x"));
    install(packs.path(), "y", &extension("y"));
    let project = tempfile::tempdir().unwrap();
    let git = std::process::Command::new("git")
        .args(["init", "-q"])
        .current_dir(project.path())
        .status();
    assert!(git.is_ok_and(|s| s.success()));
    let settings = HostSettings {
        base_env: std::env::vars()
            .filter(|(k, _)| k == "PATH" || k == "SYSTEMROOT")
            .collect(),
        bridge_origin: "http://127.0.0.1:9".into(),
        version: "test".into(),
    };
    let store = PackStore::new(packs.path());
    let lifecycle = Lifecycle::new(Policy::default());
    let hosts = Hosts::new(Arc::clone(&lifecycle), store.clone(), settings);
    let mut router_settings = RouterSettings::new(Vec::new(), "test");
    router_settings.compiler = vorn_extensions::component::Compiler::InProcess;
    let router = Router::new(
        store.clone(),
        lifecycle,
        Arc::new(NoWorld),
        router_settings,
        vorn_extensions::events::Bus::new(),
    );
    let installs = Arc::new(Installs::new(store, "git", Vec::new()));
    let fake = Arc::new(Fake::default());
    let ext = Extensions::new(
        hosts,
        router,
        installs,
        Arc::clone(&fake) as Arc<dyn Around>,
        "http://127.0.0.1:9".into(),
        vec!["http://127.0.0.1:9".into()],
        project.path().to_path_buf(),
    );
    ext.set_page_origin("http://127.0.0.1:10".into());
    World {
        packs,
        project,
        fake,
        ext,
    }
}

fn result(answer: Answer) -> Value {
    match answer {
        Answer::Result(v) => v,
        other => panic!("expected a result, got {other:?}"),
    }
}

async fn until(mut done: impl FnMut() -> bool) {
    for _ in 0..200 {
        if done() {
            return;
        }
        tokio::time::sleep(Duration::from_millis(25)).await;
    }
}

const LOCAL: SocketAddr = SocketAddr::new(std::net::IpAddr::V4(std::net::Ipv4Addr::LOCALHOST), 4);

async fn bridge(w: &World, route: Route, token: Option<&str>, body: &str) -> (StatusCode, String) {
    let mut req = Request::post("/").header("host", "127.0.0.1:9");
    if let Some(token) = token {
        req = req.header("authorization", format!("Bearer {token}"));
    }
    let req = req.body(Full::new(Bytes::from(body.to_owned()))).unwrap();
    read(routes::answer(&w.ext, route, req, LOCAL).await).await
}

async fn read(res: hyper::Response<crate::endpoint::Body>) -> (StatusCode, String) {
    let status = res.status();
    let bytes = res.into_body().collect().await.unwrap().to_bytes();
    (status, String::from_utf8_lossy(&bytes).into_owned())
}

fn to(id: &str, method: &str) -> Route {
    Route::Bridge {
        id: id.into(),
        method: method.into(),
    }
}

fn written(w: &World) -> Vec<(String, String)> {
    guard(&w.fake.written).clone()
}

#[tokio::test]
async fn lists_the_installed_extensions() {
    let w = world();
    let list = result(w.call("extension:list", json!({})).await);
    let ids: Vec<&str> = list
        .as_array()
        .unwrap()
        .iter()
        .filter_map(|p| p["id"].as_str())
        .collect();
    assert_eq!(ids, ["x", "y"]);
}

#[tokio::test]
async fn a_call_about_an_unknown_session_fails_as_the_server_worded_it() {
    let w = world();
    for method in ["extension:activation", "extension:openPane"] {
        let answer = w.call(method, json!({ "sessionId": "nope" })).await;
        assert_eq!(answer, Answer::Error("Session not found: nope".into()));
    }
    assert_eq!(
        w.call("extension:other", json!({})).await,
        Answer::Unanswered
    );
}

#[tokio::test]
async fn activation_names_each_extension_on_the_card() {
    let w = world();
    w.add(w.session("s1"));
    let states = result(
        w.call("extension:activation", json!({ "sessionId": "s1" }))
            .await,
    );
    assert_eq!(states[0]["extensionId"], "x");
    assert_eq!(states[1]["extensionName"], "Y");
    assert_eq!(states[0]["footers"], json!(["f"]));
}

#[tokio::test]
async fn a_page_pane_is_served_while_its_grant_lasts() {
    let w = world();
    w.add(w.session("s1"));
    let params = json!({ "sessionId": "s1", "extensionId": "x", "paneId": "p" });
    let grant = result(w.call("extension:openPane", params).await);
    let nonce = grant["nonce"].as_str().unwrap().to_owned();
    assert_eq!(
        grant["url"],
        format!("http://127.0.0.1:10/extensions/x/pane/p/{nonce}/")
    );
    assert_eq!(w.ext.panes(), 1);
    assert_eq!(w.ext.hosts(), 1);

    let page = |nonce: &str, rest: &str| Route::Page {
        id: "x".into(),
        pane: "p".into(),
        nonce: nonce.into(),
        rest: rest.into(),
    };
    let req = || Request::get("/").body(Full::new(Bytes::new())).unwrap();
    let res = routes::answer(&w.ext, page(&nonce, ""), req(), LOCAL).await;
    let csp = res.headers()["content-security-policy"]
        .to_str()
        .unwrap()
        .to_owned();
    assert!(csp.ends_with("frame-ancestors http://127.0.0.1:9"), "{csp}");
    assert_eq!(read(res).await, (StatusCode::OK, "<p>pane</p>".into()));
    for (nonce, rest) in [(nonce.as_str(), "../index.js"), ("other", "")] {
        let res = routes::answer(&w.ext, page(nonce, rest), req(), LOCAL).await;
        assert_eq!(res.status(), StatusCode::NOT_FOUND);
    }

    let closed = result(
        w.call("extension:closePane", json!({ "nonce": nonce }))
            .await,
    );
    assert_eq!(closed, json!({ "closed": true }));
    let res = routes::answer(&w.ext, page(&nonce, ""), req(), LOCAL).await;
    assert_eq!(res.status(), StatusCode::NOT_FOUND);
    w.ext.stop().await;
    assert_eq!(w.ext.hosts(), 0);
}

#[tokio::test]
async fn a_program_pane_runs_in_a_terminal_holding_the_extensions_token() {
    let w = world();
    w.add(w.session("s1"));
    let params = json!({ "sessionId": "s1", "extensionId": "x", "paneId": "prog" });
    let grant = result(w.call("extension:openPane", params).await);
    let terminal = grant["terminalId"].as_str().unwrap().to_owned();
    let (id, spec) = guard(&w.fake.started).pop().unwrap();
    assert_eq!(id, terminal);
    assert_eq!(spec.argv, ["lazygit", "-p"]);
    assert_eq!(spec.cwd, w.project());
    let env: HashMap<_, _> = spec.env.into_iter().collect();
    assert_eq!(
        env.get("VORN_EXTENSION_TOKEN"),
        w.ext.hosts.token_for(&w.key("x")).as_ref()
    );
    assert_eq!(
        env["VORN_EXTENSION_HOST"],
        "http://127.0.0.1:9/extensions/x/bridge"
    );
    assert_eq!(env["VORN_SESSION_ID"], terminal);
    // An open pane keeps its host, and so the token its program holds, past any idle time.
    let lifecycle = w.ext.router().lifecycle();
    lifecycle.set_policy(Policy {
        idle: Duration::ZERO,
        ..Policy::default()
    });
    lifecycle.reap();
    assert_eq!(
        env.get("VORN_EXTENSION_TOKEN"),
        w.ext.hosts.token_for(&w.key("x")).as_ref()
    );

    let nonce = grant["nonce"].as_str().unwrap();
    w.call("extension:closePane", json!({ "nonce": nonce }))
        .await;
    assert_eq!(*guard(&w.fake.killed), [terminal]);
    let again = result(
        w.call("extension:closePane", json!({ "nonce": nonce }))
            .await,
    );
    assert_eq!(again, json!({ "closed": false }));

    let params = json!({ "sessionId": "s1", "extensionId": "x", "paneId": "broken" });
    let failed = w.call("extension:openPane", params).await;
    assert_eq!(failed, Answer::Error("no such program".into()));
    let params = json!({ "sessionId": "s1", "extensionId": "x", "paneId": "none" });
    let unknown = w.call("extension:openPane", params).await;
    let said = "The extension \"x\" contributes no pane \"none\"";
    assert_eq!(unknown, Answer::Error(said.into()));
    w.ext.stop().await;
}

#[tokio::test]
async fn footers_are_read_for_each_session_and_released_with_it() {
    let w = world();
    w.ext.start();
    w.add(w.session("s1"));
    w.fake.note();
    until(|| w.fake.told(FOOTER_ITEMS).len() >= 2).await;
    assert!(w.fake.told(FOOTER_ITEMS).len() >= 2);
    assert_eq!(w.fake.told(ACTIVATION)[0]["sessionId"], "s1");

    guard(&w.fake.live).clear();
    w.fake.note();
    until(|| w.ext.hosts() == 0).await;
    assert_eq!(w.ext.hosts(), 0);
}

#[tokio::test]
async fn a_changed_pack_takes_its_panes_and_processes_with_it() {
    let w = world();
    w.add(w.session("s1"));
    let params = json!({ "sessionId": "s1", "extensionId": "x", "paneId": "prog" });
    result(w.call("extension:openPane", params).await);
    let before = fingerprint(w.packs.path());
    let current = json!({ "version": "1.0.0", "installedAt": 22 }).to_string();
    fs::write(w.packs.path().join("x/current.json"), current).unwrap();
    assert_eq!(changed(&before, &fingerprint(w.packs.path())), ["x"]);
    w.ext.pack_changed("x").await;
    assert_eq!(w.ext.panes(), 0);
    assert_eq!(w.ext.hosts(), 0);
    assert_eq!(guard(&w.fake.killed).len(), 1);
}

#[tokio::test]
async fn the_bridge_answers_only_the_extension_its_token_was_minted_for() {
    let w = world();
    w.add(w.session("s1"));
    let x = w.token("x").await;
    let y = w.token("y").await;
    let body = r#"{"sessionId":"s1"}"#;

    let answered = bridge(&w, to("x", "status"), Some(&x), body).await;
    assert_eq!(answered, (StatusCode::OK, r#"{"result":""}"#.into()));
    let as_another = bridge(&w, to("x", "status"), Some(&y), body).await;
    assert_eq!(as_another.0, StatusCode::UNAUTHORIZED);
    let anonymous = bridge(&w, to("x", "status"), None, body).await;
    assert_eq!(anonymous.0, StatusCode::UNAUTHORIZED);
    let unknown = bridge(&w, to("x", "nothing"), Some(&x), body).await;
    assert_eq!(unknown.0, StatusCode::NOT_FOUND);
    let garbled = bridge(&w, to("x", "status"), Some(&x), "{").await;
    assert_eq!(garbled.0, StatusCode::BAD_REQUEST);
    let gone = bridge(&w, to("x", "status"), Some(&x), r#"{"sessionId":"s2"}"#).await;
    assert_eq!(gone.0, StatusCode::NOT_FOUND);

    let lines = r#"{"sessionId":"s1","lines":2}"#;
    let output = bridge(&w, to("x", "output"), Some(&x), lines).await;
    let expected = json!({ "result": "line 0\nline 1" }).to_string();
    assert_eq!(output, (StatusCode::OK, expected));
    for method in ["diff", "usage", "selection"] {
        let read = bridge(&w, to("x", method), Some(&x), body).await;
        assert_eq!(read.0, StatusCode::OK, "{method}: {}", read.1);
    }
    let send = r#"{"sessionId":"s1","text":"hi"}"#;
    let sent = bridge(&w, to("x", "send"), Some(&x), send).await;
    assert_eq!(sent.0, StatusCode::NO_CONTENT);
    assert_eq!(written(&w), [("s1".to_owned(), "hi".to_owned())]);
    let rename = r#"{"sessionId":"s1","name":"N"}"#;
    let renamed = bridge(&w, to("x", "rename"), Some(&x), rename).await;
    assert_eq!(renamed.0, StatusCode::NO_CONTENT);
    assert_eq!(*guard(&w.fake.renamed), [("s1".to_owned(), "N".to_owned())]);

    let far = SocketAddr::from(([10, 0, 0, 2], 4));
    let req = Request::post("/").body(Full::new(Bytes::new())).unwrap();
    let res = routes::answer(&w.ext, to("x", "status"), req, far).await;
    assert_eq!(res.status(), StatusCode::FORBIDDEN);
    let req = Request::post("/")
        .header("sec-fetch-site", "cross-site")
        .body(Full::new(Bytes::new()))
        .unwrap();
    let res = routes::answer(&w.ext, to("x", "status"), req, LOCAL).await;
    assert_eq!(res.status(), StatusCode::FORBIDDEN);
    w.ext.stop().await;
}

#[tokio::test]
async fn the_bridge_refuses_another_projects_session_or_one_a_person_renamed() {
    let w = world();
    let mut elsewhere = w.session("s2");
    elsewhere.project_path = "/elsewhere".into();
    w.add(elsewhere);
    let mut renamed = w.session("s3");
    renamed.renamed_by_person = true;
    w.add(renamed);
    let x = w.token("x").await;
    let other = bridge(&w, to("x", "status"), Some(&x), r#"{"sessionId":"s2"}"#).await;
    assert_eq!(other.0, StatusCode::FORBIDDEN);
    let rename = r#"{"sessionId":"s3","name":"N"}"#;
    let refused = bridge(&w, to("x", "rename"), Some(&x), rename).await;
    assert_eq!(refused.0, StatusCode::FORBIDDEN);
    w.ext.stop().await;
}

#[tokio::test]
async fn a_page_speaks_for_the_session_its_pane_was_opened_on() {
    let w = world();
    w.add(w.session("s1"));
    w.add(w.session("s2"));
    let params = json!({ "sessionId": "s1", "extensionId": "x", "paneId": "p" });
    let grant = result(w.call("extension:openPane", params).await);
    let nonce = grant["nonce"].as_str().unwrap().to_owned();
    let on = |pane: &str, nonce: &str| Route::PaneBridge {
        id: "x".into(),
        pane: pane.into(),
        nonce: nonce.into(),
        method: "send".into(),
    };
    let body = r#"{"sessionId":"s2","text":"t"}"#;
    let sent = bridge(&w, on("p", &nonce), None, body).await;
    assert_eq!(sent.0, StatusCode::NO_CONTENT);
    assert_eq!(written(&w), [("s1".to_owned(), "t".to_owned())]);
    for (pane, nonce) in [("prog", nonce.as_str()), ("p", "forged")] {
        let refused = bridge(&w, on(pane, nonce), None, body).await;
        assert_eq!(refused.0, StatusCode::UNAUTHORIZED);
    }
    w.ext.stop().await;
}

#[tokio::test]
async fn pages_are_served_on_a_loopback_port_of_their_own() {
    let w = world();
    w.add(w.session("s1"));
    let origin = routes::start_pages(&w.ext).await.unwrap();
    let params = json!({ "sessionId": "s1", "extensionId": "x", "paneId": "p" });
    let grant = result(w.call("extension:openPane", params).await);
    let addr = origin.trim_start_matches("http://").to_owned();
    let ask = |verb: &str, path: &str| {
        let req = format!("{verb} {path} HTTP/1.1\r\nHost: {addr}\r\nConnection: close\r\n\r\n");
        let addr = addr.clone();
        async move {
            let mut stream = tokio::net::TcpStream::connect(&addr).await.unwrap();
            stream.write_all(req.as_bytes()).await.unwrap();
            let mut out = String::new();
            stream.read_to_string(&mut out).await.unwrap();
            out
        }
    };
    let url = grant["url"].as_str().unwrap();
    let path = url.trim_start_matches("http://127.0.0.1:10");
    let page = ask("GET", path).await;
    assert!(page.starts_with("HTTP/1.1 200"), "{page}");
    assert!(page.ends_with("<p>pane</p>"), "{page}");
    let head = ask("HEAD", path).await;
    assert!(
        head.starts_with("HTTP/1.1 200") && !head.contains("<p>"),
        "{head}"
    );
    let missing = ask("GET", "/extensions/x/pane/p/nonce/").await;
    assert!(missing.starts_with("HTTP/1.1 404"), "{missing}");
    let elsewhere = ask("POST", "/extensions/x/bridge/status").await;
    assert!(elsewhere.starts_with("HTTP/1.1 404"), "{elsewhere}");
    let post = hyper::Method::POST;
    assert!(routes::bridge_route(&post, "/extensions/x/bridge/status").is_some());
    assert!(routes::bridge_route(&hyper::Method::GET, "/extensions/x/pane/p/n/").is_none());
    w.ext.stop().await;
}

/// A program linked from a folder answers its action, through `extension:call`
/// and as `<id>:<action>`, and removing it leaves nothing listed or routed.
#[cfg(unix)]
#[tokio::test]
async fn a_linked_program_answers_and_is_removed_without_trace() {
    let w = world();
    let folder = tempfile::tempdir().unwrap();
    let fixture =
        Path::new(env!("CARGO_MANIFEST_DIR")).join("../extensions/tests/fixtures/hello-bash");
    for file in ["vorn-extension.toml", "greet.sh"] {
        fs::copy(fixture.join(file), folder.path().join(file)).unwrap();
    }
    let sheet = result(
        w.call(
            "extension:inspect",
            json!({ "url": folder.path(), "link": true }),
        )
        .await,
    );
    assert_eq!(
        (sheet["confirm"].as_str(), sheet["token"].as_str()),
        (Some("test.hello"), Some(""))
    );
    assert!(sheet["source"]["linked"].is_string());
    assert!(!w.ext.owns("test.hello:greet"));
    let linked = result(
        w.call("extension:link", json!({ "path": folder.path() }))
            .await,
    );
    assert_eq!(
        (linked["id"].as_str(), linked["linked"].as_bool()),
        (Some("test.hello"), Some(true))
    );
    assert!(w.ext.owns("test.hello:greet"));
    assert!(!w.ext.owns("git:greet") && !w.ext.owns("test.hello:") && !w.ext.owns("nope:greet"));

    let routed = w
        .ext
        .call_action("test.hello:greet", json!({ "name": "Vorn" }))
        .await;
    assert_eq!(routed, Answer::Result(json!({ "greeting": "hello Vorn" })));
    let called = result(
        w.call(
            "extension:call",
            json!({ "id": "test.hello", "action": "greet", "args": { "name": "you" } }),
        )
        .await,
    );
    assert_eq!(called, json!({ "greeting": "hello you" }));
    let installed = result(w.call("extension:installed", json!({})).await);
    let ids: Vec<&str> = installed
        .as_array()
        .unwrap()
        .iter()
        .filter_map(|p| p["id"].as_str())
        .collect();
    assert!(ids.contains(&"test.hello"), "{ids:?}");

    let removed = result(
        w.call("extension:remove", json!({ "id": "test.hello" }))
            .await,
    );
    assert_eq!(removed, json!({ "removed": "test.hello" }));
    assert!(!w.ext.owns("test.hello:greet"));
    assert!(!w.packs.path().join("test.hello").exists());
    assert!(
        folder.path().join("greet.sh").exists(),
        "a linked folder is the person's"
    );
    let again = w
        .call("extension:remove", json!({ "id": "test.hello" }))
        .await;
    assert_eq!(
        again,
        Answer::Error("No extension \"test.hello\" is installed".into())
    );
}

/// A folder typed on the web client as `~/…` is this machine's home.
#[test]
fn a_typed_home_path_is_this_machines() {
    let home = crate::native::shell::home_dir();
    assert_eq!(super::home_path(" ~/dev/ext "), format!("{home}/dev/ext"));
    assert_eq!(super::home_path("~"), home);
    assert_eq!(super::home_path("~other/x"), "~other/x");
    assert_eq!(super::home_path("https://x/y"), "https://x/y");
}

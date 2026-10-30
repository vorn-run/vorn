//! A test vornd: its own HOME and data directory under a temporary path,
//! never the user's `~/.vorn`, stopped with the session holder it started
//! when dropped.

#![allow(dead_code)]

use std::io::{BufRead, BufReader, Read, Write};
use std::net::TcpStream;
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::time::{Duration, Instant};

use serde_json::{json, Value};
use vorn_app::client::Endpoint;
use vorn_app::notify::Recorded;
use vorn_app::ui::{Id, Input, Offscreen, RenderMode, Ui};
use vorn_app::{look, ui_config, App};

pub const PATIENCE: Duration = Duration::from_secs(30);

pub struct TestVornd {
    child: Child,
    pub home: PathBuf,
    /// The grid endpoint vornd's ready line names.
    pub grid: String,
    port: u16,
}

/// Where vornd and vorn-sessiond are: `VORN_APP_TEST_BIN`, else the
/// profile directory this test was built into.
fn bin_dir() -> Option<PathBuf> {
    if let Some(dir) = std::env::var_os("VORN_APP_TEST_BIN") {
        return Some(PathBuf::from(dir));
    }
    // target/<profile>/deps/<test> -> target/<profile>
    let exe = std::env::current_exe().ok()?;
    Some(exe.parent()?.parent()?.to_path_buf())
}

fn exe(dir: &Path, name: &str) -> PathBuf {
    dir.join(format!("{name}{}", std::env::consts::EXE_SUFFIX))
}

/// A fresh, short directory: Unix socket paths have a 104-byte limit.
fn temp_home() -> std::io::Result<PathBuf> {
    let base = if cfg!(unix) {
        PathBuf::from("/tmp")
    } else {
        std::env::temp_dir()
    };
    let nanos = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |d| d.subsec_nanos());
    static NEXT: AtomicUsize = AtomicUsize::new(0);
    let n = NEXT.fetch_add(1, Ordering::Relaxed);
    let dir = base.join(format!("va-{}-{n}-{nanos:x}", std::process::id()));
    std::fs::create_dir_all(&dir)?;
    Ok(dir)
}

impl TestVornd {
    /// A started vornd with its holder connected, or `None` (said on
    /// stderr) when the binaries are not built.
    pub fn start() -> Option<TestVornd> {
        TestVornd::start_with(&[])
    }

    /// [`TestVornd::start`] with these of Vorn's own catalog extensions also
    /// installed from a catalog served on loopback, as a person installs one;
    /// `None` when scripts/shipped.mjs has not fetched their packs.
    pub fn start_with(installed: &[&str]) -> Option<TestVornd> {
        let dir = bin_dir()?;
        let (vornd, sessiond) = (exe(&dir, "vornd"), exe(&dir, "vorn-sessiond"));
        if !vornd.is_file() || !sessiond.is_file() {
            eprintln!("skipped: no vornd/vorn-sessiond in {}", dir.display());
            return None;
        }
        let home = temp_home().expect("temp dir");
        if !installed.is_empty() {
            let mut packs = Vec::new();
            for id in installed {
                let Some(pack) = catalog_pack(id) else {
                    eprintln!(
                        "skipped: {id} was not fetched (run packages/core/scripts/shipped.mjs)"
                    );
                    return None;
                };
                packs.push(pack);
            }
            offer_in_catalog(&home, packs);
        }
        let v = TestVornd::spawn(home);
        if !installed.is_empty() {
            let ep = Endpoint::find(&v.home).expect("vornd wrote ws-port and local-token");
            let (rpc, _events) =
                vorn_app::client::Rpc::connect(&ep, std::sync::Arc::new(|| {})).expect("rpc");
            for id in installed {
                let done = rpc
                    .call(
                        "connector:installPack",
                        json!({ "kind": "catalog", "id": id }),
                    )
                    .expect("installPack");
                assert_eq!(done["ok"], true, "{id}: {done}\n{}", v.log());
            }
        }
        Some(v)
    }

    fn spawn(home: PathBuf) -> TestVornd {
        let dir = bin_dir().expect("bin dir");
        let (vornd, sessiond) = (exe(&dir, "vornd"), exe(&dir, "vorn-sessiond"));
        let mut child = Command::new(&vornd)
            .env(vorn_spawn::watch::EXIT_WITH, std::process::id().to_string())
            .env("VORN_TAILSCALE", "none")
            .arg("--data-dir")
            .arg(&home)
            .args(["--port", "0", "--exit-with-stdin"])
            .arg("--sessiond")
            .arg(&sessiond)
            .arg("--log-file")
            .arg(home.join("vornd.log"))
            .env("HOME", &home)
            .env("USERPROFILE", &home)
            .env("VORN_HOME", &home)
            .env("SHELL", "/bin/sh")
            .env("VORND_KEYCHAIN", "0")
            .env("VORN_SESSIOND_IDLE_EXIT", "2")
            .env_remove("VORN_DATA_DIR")
            // An agent's own home would be the person's, not the test's.
            .env_remove("COPILOT_HOME")
            .env_remove("XDG_DATA_HOME")
            .env_remove("HERMES_HOME")
            .env_remove("VORN_ALLOW_DEFAULT_DATA_DIR")
            .env("NO_PROXY", "127.0.0.1,localhost")
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::null())
            .spawn()
            .expect("vornd starts");
        let stdout = child.stdout.take().expect("stdout");
        let (tx, rx) = std::sync::mpsc::channel();
        std::thread::spawn(move || {
            let mut line = String::new();
            let _ = BufReader::new(stdout).read_line(&mut line);
            let _ = tx.send(line);
        });
        let mut v = TestVornd {
            child,
            home,
            grid: String::new(),
            port: 0,
        };
        let line = rx.recv_timeout(PATIENCE).unwrap_or_default();
        let ready: Value = serde_json::from_str(line.trim())
            .unwrap_or_else(|e| panic!("ready line {line:?}: {e}\n{}", v.log()));
        v.port = ready["port"]
            .as_u64()
            .and_then(|p| u16::try_from(p).ok())
            .expect("port");
        v.grid = ready["grid"].as_str().expect("grid endpoint").to_owned();
        v.wait_connected();
        v
    }

    /// Stops vornd and the terminals it held, then starts it again on the
    /// same HOME: what the last run left is carried over.
    pub fn restart(&mut self) {
        self.stop();
        let next = TestVornd::spawn(self.home.clone());
        let old = std::mem::replace(self, next);
        // Already stopped, and the HOME is the new run's now.
        std::mem::forget(old);
    }

    /// Stops vornd alone and starts it again on the same HOME: the session
    /// holder keeps the terminals, and the new vornd takes them on.
    pub fn restart_server(&mut self) {
        drop(self.child.stdin.take());
        let t = Instant::now();
        while t.elapsed() < Duration::from_secs(5) {
            if matches!(self.child.try_wait(), Ok(Some(_))) {
                break;
            }
            std::thread::sleep(Duration::from_millis(50));
        }
        let _ = self.child.kill();
        let _ = self.child.wait();
        let next = TestVornd::spawn(self.home.clone());
        let old = std::mem::replace(self, next);
        std::mem::forget(old);
    }

    fn stop(&mut self) {
        let holders = self.holder_pids();
        drop(self.child.stdin.take());
        let t = Instant::now();
        while t.elapsed() < Duration::from_secs(5) {
            if matches!(self.child.try_wait(), Ok(Some(_))) {
                break;
            }
            std::thread::sleep(Duration::from_millis(50));
        }
        let _ = self.child.kill();
        let _ = self.child.wait();
        for pid in holders {
            kill(pid);
        }
        std::thread::sleep(Duration::from_millis(200));
    }

    /// The HTTP port vornd serves on.
    pub fn port(&self) -> u16 {
        self.port
    }

    pub fn log(&self) -> String {
        std::fs::read_to_string(self.home.join("vornd.log")).unwrap_or_default()
    }

    fn get(&self, path: &str) -> Option<Value> {
        let mut s = TcpStream::connect(("127.0.0.1", self.port)).ok()?;
        s.set_read_timeout(Some(PATIENCE)).ok()?;
        // vornd's reports want the local credential it wrote beside its data.
        let token = std::fs::read_to_string(self.home.join("local-token")).unwrap_or_default();
        let req = format!(
            "GET {path} HTTP/1.1\r\nHost: 127.0.0.1\r\nAuthorization: Bearer {}\r\nConnection: close\r\n\r\n",
            token.trim()
        );
        s.write_all(req.as_bytes()).ok()?;
        let mut res = String::new();
        s.read_to_string(&mut res).ok()?;
        serde_json::from_str(res.split_once("\r\n\r\n")?.1).ok()
    }

    /// Terminals cannot start until the session holder is connected.
    fn wait_connected(&self) {
        let t = Instant::now();
        loop {
            let health = self.get("/vornd/health").unwrap_or_default();
            let sessions = self.get("/vornd/sessions").unwrap_or_default();
            if health["sessiond"]["current"].is_object() && sessions["connected"] == true {
                return;
            }
            assert!(
                t.elapsed() < PATIENCE,
                "sessiond never connected:\n{}",
                self.log()
            );
            std::thread::sleep(Duration::from_millis(50));
        }
    }

    /// vornd's process and its session holders', whose CPU time a bench reads.
    pub fn pids(&self) -> Vec<u32> {
        let mut pids = vec![self.child.id()];
        pids.extend(self.holder_pids());
        pids
    }

    fn holder_pids(&self) -> Vec<u32> {
        let Ok(dir) = std::fs::read_dir(self.home.join("run")) else {
            return Vec::new();
        };
        dir.flatten()
            .filter(|e| e.path().extension().is_some_and(|x| x == "info"))
            .filter_map(|e| std::fs::read_to_string(e.path()).ok())
            .filter_map(|t| {
                t.lines()
                    .find_map(|l| l.strip_prefix("pid=").and_then(|p| p.parse().ok()))
            })
            .collect()
    }
}

fn kill(pid: u32) {
    #[cfg(unix)]
    if let Ok(pid) = i32::try_from(pid) {
        // SAFETY: kill(2) takes any pid; a stale one fails with ESRCH.
        unsafe {
            libc::kill(pid, libc::SIGKILL);
        }
    }
    #[cfg(windows)]
    let _ = Command::new("taskkill")
        .args(["/F", "/T", "/PID", &pid.to_string()])
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status();
}

impl Drop for TestVornd {
    fn drop(&mut self) {
        self.stop();
        let _ = std::fs::remove_dir_all(&self.home);
    }
}

/// Waits for the built-ins' views, which come once their extensions start; their keys, pills and palette entries come with them.
pub fn views_listed(shot: &mut Shot, app: &mut App) {
    views_listed_in(app, &mut shot.ui, shot.size);
}

/// [`views_listed`] laying out frames on a test's own `ui`.
pub fn views_listed_in(app: &mut App, ui: &mut Ui, size: (f32, f32)) {
    wait_for("the built-ins' views", || {
        app.frame(ui, size);
        let p = &app.st.points;
        (p.find("vorn.tasks", "pages", "board").is_some()
            && p.find("vorn.workflows", "pages", "workflows").is_some())
        .then_some(())
    });
}

/// Opens the Tasks view from its sidebar tab, as a person clicks it.
#[allow(dead_code)]
pub fn open_tasks(shot: &mut Shot, app: &mut App) {
    let tab = App::control(
        "sidebar-mode",
        &vorn_app::act::Act::View(vorn_app::screen::MainView::Tasks),
    );
    shot.until(app, "the Tasks tab", |_, ui| ui.rect_of(tab));
    shot.click(app, tab);
}

/// Polls `f` until it answers, or fails the test with `what`.
#[track_caller]
pub fn wait_for<T>(what: &str, mut f: impl FnMut() -> Option<T>) -> T {
    let t = Instant::now();
    loop {
        if let Some(v) = f() {
            return v;
        }
        assert!(t.elapsed() < PATIENCE, "timed out waiting for {what}");
        std::thread::sleep(Duration::from_millis(25));
    }
}

pub fn key(code: &str, m: u16, text: Option<&str>) -> Input {
    Input::Key {
        code: code.into(),
        mods: m,
        text: text.map(Into::into),
    }
}

/// The pack of `id` scripts/shipped.mjs fetched into `shipped/catalog/`:
/// its id, version, sha256 and bytes.
fn catalog_pack(id: &str) -> Option<(String, String, String, Vec<u8>)> {
    use sha2::Digest;
    let dir = std::env::var_os("VORN_SHIPPED_DIR").map_or_else(
        || Path::new(env!("CARGO_MANIFEST_DIR")).join("../../shipped"),
        PathBuf::from,
    );
    let prefix = format!("{id}-");
    let entry = std::fs::read_dir(dir.join("catalog"))
        .ok()?
        .flatten()
        .find(|e| e.file_name().to_string_lossy().starts_with(&prefix))?;
    let name = entry.file_name().to_string_lossy().into_owned();
    let version = name
        .strip_prefix(&prefix)?
        .strip_suffix(".vorn.tgz")?
        .to_owned();
    let bytes = std::fs::read(entry.path()).ok()?;
    let sha256 = format!("{:x}", sha2::Sha256::digest(&bytes));
    Some((id.to_owned(), version, sha256, bytes))
}

/// Lists `packs` in the catalog vornd at `home` reads, each served from a
/// loopback port of its own as the release would be.
fn offer_in_catalog(home: &Path, packs: Vec<(String, String, String, Vec<u8>)>) {
    let mut entries = Vec::new();
    for (id, version, sha256, bytes) in packs {
        let listener = std::net::TcpListener::bind(("127.0.0.1", 0)).expect("a port");
        let port = listener.local_addr().expect("its address").port();
        std::thread::spawn(move || {
            for mut stream in listener.incoming().flatten() {
                let mut req = Vec::new();
                let mut byte = [0u8; 1];
                while !req.ends_with(b"\r\n\r\n") && stream.read(&mut byte).unwrap_or(0) == 1 {
                    req.push(byte[0]);
                }
                let head = format!(
                    "HTTP/1.1 200 OK\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
                    bytes.len()
                );
                let _ = stream.write_all(head.as_bytes());
                let _ = stream.write_all(&bytes);
            }
        });
        entries.push(json!({
            "id": id, "name": id, "kind": "extension", "version": version, "sha256": sha256,
            "packUrl": format!("http://127.0.0.1:{port}/{id}-{version}.vorn.tgz"),
        }));
    }
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |d| d.as_millis());
    let cache = json!({ "connectors": entries, "fetchedAt": now as f64 });
    std::fs::create_dir_all(home.join(".vorn")).expect("~/.vorn");
    std::fs::write(home.join(".vorn/connector-catalog.json"), cache.to_string())
        .expect("the catalog");
}

/// The app over `v`, past the welcome guide: it is marked seen first, so
/// tests start on the main screen as a returning user does.
pub fn connect(v: &TestVornd) -> App {
    let ep = Endpoint::find(&v.home).expect("vornd wrote ws-port and local-token");
    let (rpc, _events) =
        vorn_app::client::Rpc::connect(&ep, std::sync::Arc::new(|| {})).expect("rpc");
    let mut config = rpc.call("config:load", Value::Null).expect("config");
    config["defaults"]["hasSeenOnboarding"] = json!(2);
    rpc.call("config:save", config).expect("saved");
    let mut app = first_run(v);
    link_test_agent(&mut app);
    app
}

/// Links the test agent's adapter (`fixtures/acme-adapter`), as a person
/// links a folder, so the tests run an agent without naming a real CLI.
pub fn link_test_agent(app: &mut App) {
    let folder = Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/acme-adapter");
    if app.store.agents.get("acme").is_none() {
        app.rpc()
            .call("extension:link", json!({ "path": folder }))
            .expect("the test agent's adapter links");
    }
    wait_for("the test agent's adapter", || {
        app.pump();
        app.store.agents.get("acme").map(|_| ())
    });
}

/// The app over `v` as it opens on a first run.
pub fn first_run(v: &TestVornd) -> App {
    let ep = Endpoint::find(&v.home).expect("vornd wrote ws-port and local-token");
    assert_eq!(
        ep.grid, v.grid,
        "the grid endpoint is derived as vornd names it"
    );
    let mut app = App::connect(&ep, std::sync::Arc::new(|| {})).expect("connects and loads");
    // The headless window is unfocused: notices would reach the real OS.
    app.set_notice_sink(std::sync::Arc::new(Recorded::default()));
    app.set_data_dir(v.home.clone());
    assert_eq!(app.grid_error, None);
    wait_for("the startup calls", || {
        app.pump();
        (!app.busy()).then_some(())
    });
    app
}

/// A project named `name` in the test HOME, with the default agent a
/// plain shell so nothing real runs, and `defaults` merged in.
pub fn add_project(app: &mut App, v: &TestVornd, name: &str, defaults: Value) -> PathBuf {
    let path = v.home.join(name);
    std::fs::create_dir_all(&path).expect("project dir");
    let mut config = app.rpc().call("config:load", Value::Null).expect("config");
    config["projects"] = json!([{ "name": name, "path": path, "preferredAgents": [] }]);
    config["agentCommands"]["acme"] = json!({ "command": "/bin/sh", "args": [] });
    config["defaults"]["defaultAgent"] = json!("acme");
    if let (Some(d), Value::Object(more)) = (config["defaults"].as_object_mut(), defaults) {
        d.extend(more);
    }
    app.rpc().call("config:save", config).expect("saved");
    wait_for("the project to arrive", || {
        app.pump();
        (!app.store.projects.is_empty()).then_some(())
    });
    path
}

/// The app's UI, offscreen at 1440x900 on the GPU or the CPU.
pub struct Shot {
    pub ui: Ui,
    target: Offscreen,
    pub size: (f32, f32),
}

/// The window a shot draws: `VORN_APP_SHOT_SIZE` (`1400x900`) and
/// `VORN_APP_SHOT_SCALE`.
fn shot_window() -> ((f32, f32), f32) {
    let size = std::env::var("VORN_APP_SHOT_SIZE")
        .ok()
        .and_then(|s| {
            let (w, h) = s.split_once('x')?;
            Some((w.parse().ok()?, h.parse().ok()?))
        })
        .unwrap_or(look::WINDOW);
    let scale = std::env::var("VORN_APP_SHOT_SCALE")
        .ok()
        .and_then(|s| s.parse().ok())
        .unwrap_or(1.0);
    (size, scale)
}

impl Shot {
    pub fn new() -> Shot {
        let (size, scale) = shot_window();
        Shot::at(size, scale)
    }

    /// A `size` window at `scale`, as a Retina display draws it at 2.
    pub fn at(size: (f32, f32), scale: f32) -> Shot {
        let ui = Ui::headless(RenderMode::Auto, &ui_config(scale));
        let px = ((size.0 * scale) as u32, (size.1 * scale) as u32);
        let target = ui.offscreen(px);
        Shot { ui, target, size }
    }

    /// A frame laid out, so controls have their place, without drawing.
    pub fn frame(&mut self, app: &mut App) {
        app.frame(&mut self.ui, self.size);
    }

    pub fn draw(&mut self, app: &mut App) {
        // A full atlas empties itself; the second pass fits one frame.
        app.frame(&mut self.ui, self.size);
        if !self.ui.render_offscreen(&self.target) {
            app.frame(&mut self.ui, self.size);
            self.ui.render_offscreen(&self.target);
        }
    }

    /// Draws the frame and saves it as `$VORN_APP_SHOT-<name>.png`, when
    /// that is set.
    pub fn save(&mut self, app: &mut App, name: &str) {
        if let Some(base) = std::env::var_os("VORN_APP_SHOT") {
            // Past a menu's grow-in and the hover fades.
            self.frame(app);
            std::thread::sleep(std::time::Duration::from_millis(250));
            self.draw(app);
            let path = format!("{}-{name}.png", base.to_string_lossy());
            self.ui.save_png(&self.target, &path).expect("png");
        }
    }

    /// One key, after a frame as a window draws between keys.
    pub fn send(&mut self, app: &mut App, input: Input) {
        self.frame(app);
        app.send(&mut self.ui, input);
    }

    pub fn type_text(&mut self, app: &mut App, s: &str) {
        for ch in s.chars() {
            self.send(app, Input::char(ch));
        }
    }

    /// Types `s` and presses Enter.
    pub fn type_line(&mut self, app: &mut App, s: &str) {
        self.type_text(app, s);
        self.send(app, key("Enter", 0, Some("\r")));
    }

    /// Clicks the control `id` where a fresh frame places it.
    #[track_caller]
    pub fn click(&mut self, app: &mut App, id: Id) {
        self.frame(app);
        if app.st.settings.open && app.st.menu.is_none() && self.ui.rect_of(id).is_some() {
            self.reveal(app, vorn_app::screen::settings::CONTENT, id);
        }
        assert!(app.click(&mut self.ui, id), "{id:?} is on screen");
        self.frame(app);
    }

    /// Clicks `id` as `click` does, and says whether it was there to click.
    pub fn try_click(&mut self, app: &mut App, id: Id) -> bool {
        self.frame(app);
        if self.ui.rect_of(id).is_none()
            || (app.st.settings.open
                && app.st.menu.is_none()
                && !self.try_reveal(app, vorn_app::screen::settings::CONTENT, id))
        {
            return false;
        }
        let clicked = app.click(&mut self.ui, id);
        self.frame(app);
        clicked
    }

    /// Scrolls the scroll view `view` until `id` is inside the window.
    #[track_caller]
    pub fn reveal(&mut self, app: &mut App, view: Id, id: Id) {
        assert!(self.try_reveal(app, view, id), "{id:?} is laid out");
    }

    /// `reveal`, false when `id` leaves the layout on the way.
    fn try_reveal(&mut self, app: &mut App, view: Id, id: Id) -> bool {
        let mut probe = 0.0;
        for _ in 0..40 {
            self.frame(app);
            let Some(r) = self.ui.rect_of(id) else {
                return false;
            };
            let off = self.ui.scroll_offset(view);
            // Clipped away entirely: walk the view from the top until it shows.
            if r.h < 0.5 {
                self.ui.set_scroll_offset(view, probe);
                probe += self.size.1 / 2.0;
                continue;
            }
            if r.y + r.h > self.size.1 - 8.0 {
                self.ui
                    .set_scroll_offset(view, off + (r.y + r.h - self.size.1) + 120.0);
            } else if r.y < 0.0 {
                self.ui
                    .set_scroll_offset(view, (off + r.y - 120.0).max(0.0));
            } else {
                return true;
            }
        }
        true
    }

    /// Rests the pointer on `id`, for what shows on hover.
    #[track_caller]
    /// Chooses `label` from `session`'s card More menu, opening it first.
    pub fn card_menu(&mut self, app: &mut App, session: &str, label: &str) {
        let kept = app.st.arrange.card_shown;
        let i = self.card_menu_entry(app, session, |labels| {
            labels.iter().position(|l| l == label)
        });
        self.click(app, vorn_app::ui::item_id(vorn_app::screen::menus::MENU, i));
        app.st.arrange.card_shown = kept;
    }

    /// Opens `session`'s card More menu and waits until `pick` finds what it wants in its labels.
    pub fn card_menu_entry<T>(
        &mut self,
        app: &mut App,
        session: &str,
        mut pick: impl FnMut(&[String]) -> Option<T>,
    ) -> T {
        // Every card button in More, so a test reaches each by its label.
        app.st.arrange.card_shown = 0;
        let more = App::control(
            "card",
            &vorn_app::act::Act::Open(vorn_app::state::MenuKind::Card(session.to_owned())),
        );
        wait_for("the card menu's entry", || {
            if app.st.menu.is_none() {
                self.hover(app, vorn_app::screen::term_id(session));
                self.click(app, more);
            }
            app.pump();
            self.frame(app);
            pick(&app.menu_labels(&self.ui))
        })
    }

    pub fn hover(&mut self, app: &mut App, id: Id) {
        self.frame(app);
        let r = self
            .ui
            .rect_of(id)
            .unwrap_or_else(|| panic!("{id:?} is on screen"));
        self.ui.pointer_move(r.x + r.w / 2.0, r.y + r.h / 2.0);
        self.frame(app);
    }

    #[track_caller]
    pub fn right_click(&mut self, app: &mut App, id: Id) {
        self.frame(app);
        assert!(app.right_click(&mut self.ui, id), "{id:?} is on screen");
        self.frame(app);
    }

    /// The control a fresh frame names `label` with a button's role, the last when several do.
    pub fn button(&mut self, app: &mut App, label: &str) -> Option<Id> {
        self.labelled(app, "Button", label)
    }

    /// The last control of `role` (its accessibility role's name) labelled `label`.
    pub fn labelled(&mut self, app: &mut App, role: &str, label: &str) -> Option<Id> {
        let tree = app.frame(&mut self.ui, self.size);
        tree.nodes
            .iter()
            .filter(|(_, n)| format!("{:?}", n.role()) == role && n.label() == Some(label))
            .map(|(id, _)| Id(id.0))
            .next_back()
    }

    /// Lays out frames until `f` answers.
    #[track_caller]
    pub fn until<T>(
        &mut self,
        app: &mut App,
        what: &str,
        mut f: impl FnMut(&mut App, &Ui) -> Option<T>,
    ) -> T {
        wait_for(what, || {
            self.frame(app);
            f(app, &self.ui)
        })
    }
}

/// Runs git in `dir` with a fixed author and date, as the capture seed does.
pub fn git(dir: &Path, args: &[&str]) {
    let out = Command::new("git")
        .args(args)
        .current_dir(dir)
        .env("GIT_AUTHOR_NAME", "Test")
        .env("GIT_AUTHOR_EMAIL", "t@e.st")
        .env("GIT_COMMITTER_NAME", "Test")
        .env("GIT_COMMITTER_EMAIL", "t@e.st")
        .env("GIT_AUTHOR_DATE", "2026-10-01T10:00:00Z")
        .env("GIT_COMMITTER_DATE", "2026-10-01T10:00:00Z")
        .env("GIT_CONFIG_GLOBAL", "/dev/null")
        .env("GIT_CONFIG_NOSYSTEM", "1")
        .output()
        .expect("git runs");
    assert!(
        out.status.success(),
        "git {args:?}: {}",
        String::from_utf8_lossy(&out.stderr)
    );
}

/// The wave 3 seed (`seed-w3.mjs`): an `acme` repository with staged and
/// unstaged changes, a `feature-greet` worktree, a remote host, and a
/// shell in the repository. Answers the repository's path.
pub fn seed_acme(app: &mut App, v: &TestVornd) -> PathBuf {
    let repo = v.home.join("acme");
    std::fs::create_dir_all(repo.join("src/lib")).expect("dirs");
    let w = |p: &str, t: &str| std::fs::write(repo.join(p), t).expect("write");
    w("README.md", "# Acme\n\nA small service.\n");
    w(
        "package.json",
        "{\n  \"name\": \"acme\",\n  \"version\": \"1.0.0\"\n}\n",
    );
    w("src/app.ts", "import { greet } from './lib/greet'\n\nexport function main(): void {\n  console.log(greet('world'))\n}\n\nmain()\n");
    w(
        "src/lib/greet.ts",
        "export function greet(name: string): string {\n  return `Hello, ${name}`\n}\n",
    );
    git(&repo, &["init", "-q", "-b", "main"]);
    // vornd commits here too, with no global identity in a test HOME.
    git(&repo, &["config", "user.name", "Test"]);
    git(&repo, &["config", "user.email", "t@e.st"]);
    git(&repo, &["add", "-A"]);
    git(&repo, &["commit", "-qm", "Start the service"]);
    w("src/app.ts", "import { greet } from './lib/greet'\n\nconst names = ['world', 'team']\n\nexport function main(): void {\n  for (const name of names) console.log(greet(name))\n}\n\nmain()\n");
    w(
        "src/lib/format.ts",
        "export const upper = (s: string): string => s.toUpperCase()\n",
    );
    w(
        "README.md",
        "# Acme\n\nA small service that greets people.\n",
    );
    git(&repo, &["add", "README.md"]);
    let mut config = app.rpc().call("config:load", Value::Null).expect("config");
    let projects = config["projects"].as_array().cloned().unwrap_or_default();
    let mut projects = projects;
    projects.push(json!({ "name": "acme", "path": repo, "preferredAgents": ["acme"], "icon": "Code", "iconColor": "#6F8FAF", "workspaceId": "personal" }));
    config["projects"] = json!(projects);
    config["remoteHosts"] = json!([{ "id": "host-1", "label": "Build box", "hostname": "build.example.com", "user": "ci", "port": 2222, "authMethod": "agent" }]);
    app.rpc().call("config:save", config).expect("saved");
    app.rpc()
        .call(
            "git:createWorktree",
            json!({ "projectPath": repo, "branch": "feature-greet", "worktreeName": "feature-greet" }),
        )
        .expect("worktree");
    wait_for("the project to arrive", || {
        app.pump();
        app.store
            .projects
            .iter()
            .any(|p| p.name == "acme")
            .then_some(())
    });
    repo
}

/// Puts `tasks` straight into vornd through its task calls, each under its
/// own id, in its status; vornd's configuration carries none.
pub fn seed_tasks(rpc: &vorn_app::client::Rpc, tasks: &[Value]) {
    for t in tasks {
        let made = rpc.call("task:create", t.clone()).expect("task:create");
        assert_eq!(made["ok"], true, "{t}");
    }
}

/// Makes `workflows` the only ones vornd has, as a test's world starts.
pub fn replace_workflows(rpc: &vorn_app::client::Rpc, workflows: &[Value]) {
    let old = rpc
        .call("workflow:list", Value::Null)
        .expect("workflow:list");
    for w in old.as_array().into_iter().flatten() {
        rpc.call("workflow:delete", json!({ "id": w["id"] }))
            .expect("workflow:delete");
    }
    seed_workflows(rpc, workflows);
}

/// Puts `workflows` straight into vornd through its workflow calls.
pub fn seed_workflows(rpc: &vorn_app::client::Rpc, workflows: &[Value]) {
    for w in workflows {
        rpc.call("workflow:create", json!({ "workflow": w }))
            .expect("workflow:create");
    }
}

/// Writes `runs` straight into vornd's database, as the engine would have saved them.
pub fn seed_runs(v: &TestVornd, runs: &[Value]) {
    let mut store = vorn_store::Store::open_beside(&v.home.join("vorn.db"))
        .expect("vorn.db opens")
        .expect("vornd made vorn.db");
    for run in runs {
        let run: vorn_protocol::WorkflowExecution =
            serde_json::from_value(run.clone()).expect("a workflow run");
        store.save_workflow_run(&run).expect("run saved");
    }
}

/// A copy of example extension `name` (`examples/extensions/<name>`) in `home`,
/// its component built as its manifest's `[build]` would and put where the manifest names it.
pub fn example(home: &Path, name: &str) -> PathBuf {
    fn copy(from: &Path, to: &Path) {
        std::fs::create_dir_all(to).expect("dir");
        for e in std::fs::read_dir(from).expect("example").flatten() {
            let at = to.join(e.file_name());
            if e.file_name() == "target" {
                continue;
            }
            if e.file_type().expect("type").is_dir() {
                copy(&e.path(), &at);
            } else {
                std::fs::copy(e.path(), at).expect("copy");
            }
        }
    }
    let from = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../../../examples/extensions")
        .join(name);
    let dir = home.join(name);
    copy(&from, &dir);
    let package = format!("vorn-example-{name}");
    let built = vorn_guest_build::component(&package).unwrap_or_else(|e| panic!("{e}"));
    let at = dir
        .join("target/wasm32-wasip2/release")
        .join(format!("{}.wasm", package.replace('-', "_")));
    std::fs::create_dir_all(at.parent().expect("its folder")).expect("dir");
    std::fs::copy(built, at).expect("the component");
    dir
}

/// Posts a hook event to the endpoint vornd registered in the test HOME, as
/// an agent's hook does; answers what the hook was told. A permission request
/// waits for its answer.
pub fn post_hook(home: &Path, body: &Value) -> String {
    let dir = home.join(".vorn");
    let (port, token) = wait_for("the hook endpoint", || {
        let port = std::fs::read_to_string(dir.join("port")).ok()?;
        let token = std::fs::read_to_string(dir.join("token")).ok()?;
        Some((port.trim().to_owned(), token.trim().to_owned()))
    });
    let body = body.to_string();
    let mut stream = TcpStream::connect(format!("127.0.0.1:{port}")).expect("hooks");
    write!(
        stream,
        "POST /hooks HTTP/1.1\r\nHost: localhost\r\nAuthorization: Bearer {token}\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
        body.len()
    )
    .expect("sent");
    let mut answer = String::new();
    let _ = stream.read_to_string(&mut answer);
    assert!(answer.starts_with("HTTP/1.1 200"), "{answer}");
    answer
}

/// Whether tree point `key` has drawn, and its tree says `text`.
pub fn tree_says(app: &App, key: &str, text: &str) -> bool {
    app.st
        .points
        .trees
        .get(key)
        .and_then(|t| t.as_ref().ok())
        .is_some_and(|t| serde_json::to_string(t).is_ok_and(|j| j.contains(text)))
}

/// This state on a terminal at 120x36 and 80x24, saved as HTML and text with
/// `VORN_APP_SHOT`; each size shows every one of `shown`.
#[track_caller]
pub fn every_size(app: &mut App, name: &str, shown: &[&str]) {
    use vorn_app::ui::backend::{Backend, Cells};
    let (chrome, traffic) = (app.chrome, app.traffic);
    (app.chrome, app.traffic) = (false, false);
    for (cols, rows) in [(120u16, 36u16), (80, 24)] {
        let mut ui = Ui::cells(&ui_config(1.0));
        let size = vorn_app::ui::cells::units::px(cols, rows);
        app.frame(&mut ui, size);
        app.frame(&mut ui, size);
        let mut target = Cells::offscreen(cols, rows);
        target.present(&mut ui).expect("present");
        let text = (0..rows)
            .map(|r| target.buffer.row_text(r))
            .collect::<Vec<_>>()
            .join("\n");
        if let Some(base) = std::env::var_os("VORN_APP_SHOT") {
            let path = format!("{}-cells-{name}-{cols}x{rows}", base.to_string_lossy());
            let html = target.buffer.to_html(&format!("{name} {cols}x{rows}"));
            std::fs::write(format!("{path}.html"), html).expect("html");
            std::fs::write(format!("{path}.txt"), &text).expect("txt");
        }
        for t in shown {
            assert!(
                text.contains(t),
                "{name} {cols}x{rows} shows {t:?}:\n{text}"
            );
        }
    }
    (app.chrome, app.traffic) = (chrome, traffic);
}

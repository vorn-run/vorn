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
        let dir = bin_dir()?;
        let (vornd, sessiond) = (exe(&dir, "vornd"), exe(&dir, "vorn-sessiond"));
        if !vornd.is_file() || !sessiond.is_file() {
            eprintln!("skipped: no vornd/vorn-sessiond in {}", dir.display());
            return None;
        }
        let home = temp_home().expect("temp dir");
        Some(TestVornd::spawn(home))
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
            .env_remove("VORN_ALLOW_DEFAULT_DATA_DIR")
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

    pub fn log(&self) -> String {
        std::fs::read_to_string(self.home.join("vornd.log")).unwrap_or_default()
    }

    fn get(&self, path: &str) -> Option<Value> {
        let mut s = TcpStream::connect(("127.0.0.1", self.port)).ok()?;
        s.set_read_timeout(Some(PATIENCE)).ok()?;
        let req = format!("GET {path} HTTP/1.1\r\nHost: 127.0.0.1\r\nConnection: close\r\n\r\n");
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
/// `VORN_APP_SHOT_SCALE`, to compare with captures of today's app.
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

    /// Scrolls the scroll view `view` until `id` is inside the window.
    #[track_caller]
    pub fn reveal(&mut self, app: &mut App, view: Id, id: Id) {
        let mut probe = 0.0;
        for _ in 0..40 {
            self.frame(app);
            let Some(r) = self.ui.rect_of(id) else {
                panic!("{id:?} is laid out");
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
                return;
            }
        }
    }

    /// Rests the pointer on `id`, for what shows on hover.
    #[track_caller]
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

    /// The button `label` a template of point `key` draws for `row`.
    pub fn row_button(&mut self, app: &mut App, key: &str, row: &str, label: &str) -> Option<Id> {
        let tree = app.frame(&mut self.ui, self.size);
        let named: Vec<u64> = tree
            .nodes
            .iter()
            .filter(|(_, n)| format!("{:?}", n.role()) == "Button" && n.label() == Some(label))
            .map(|(id, _)| id.0)
            .collect();
        let slots = match &app.st.points.get(key)?.template {
            Ok(Some(t)) => vorn_components::template::slots(t),
            _ => return None,
        };
        slots
            .into_iter()
            .map(|slot| vorn_app::screen::points::control_id(key, None, slot, Some(row)))
            .find(|id| named.contains(&id.0))
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

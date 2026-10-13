//! The `vorn-app` binary.
//!
//! ```text
//! vorn-app [window] [--data-dir D]
//! vorn-app shot --out a.png [--data-dir D] [--size WxH] [--scale S] [--select N]
//!               [--keys K1,K2] [--clipboard TEXT] [--chrome off]
//! ```
//!
//! `--keys` sends keys before the shot (`Mod+KeyN`, `Alt+Digit2`,
//! `F2`, `Escape`, `KeyE,KeyC,Tab`); a bare letter types itself. A paste
//! among them reads `--clipboard`.
//!
//! Both find vornd through `D` (default `VORN_DATA_DIR`, else `~/.vorn`).
//! `window` starts the vornd shipped beside it when none answers, and waits
//! for it; `shot` draws one frame offscreen once every card has its screen.

// A shipped build opens no console window on Windows.
#![cfg_attr(all(windows, not(debug_assertions)), windows_subsystem = "windows")]

use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use vorn_app::client::rpc::Wake;
use vorn_app::client::{data_dir, Endpoint};
use vorn_app::crash::{self, CrashAct};
use vorn_app::launch::{self, Server};
use vorn_app::ui::{
    div, mods, run_with, text, El, Event, Input, Laid, MemoryClipboard, NativeWindow, Rect,
    RenderMode, Rgba, Ui, Waker, WindowApp, WindowOptions,
};
use vorn_app::update::Updater;
use vorn_app::{look, ui_config, App};

struct Args {
    mode: String,
    flags: Vec<(String, String)>,
}

impl Args {
    fn parse() -> Result<Args, String> {
        // Finder may still pass a process serial number to a bundled app.
        let mut it = std::env::args()
            .skip(1)
            .filter(|a| !a.starts_with("-psn_"))
            // A vorn:// link opens the window.
            .filter(|a| !a.starts_with("vorn:"))
            .peekable();
        let mode = match it.peek() {
            Some(m) if !m.starts_with("--") => it.next().unwrap_or_default(),
            _ => "window".to_owned(),
        };
        let mut flags = Vec::new();
        while let Some(k) = it.next() {
            let key = k.strip_prefix("--").ok_or(format!("unexpected {k:?}"))?;
            let v = it.next().ok_or(format!("--{key} needs a value"))?;
            flags.push((key.to_owned(), v));
        }
        Ok(Args { mode, flags })
    }

    fn get(&self, key: &str) -> Option<&str> {
        self.flags
            .iter()
            .find(|(k, _)| k == key)
            .map(|(_, v)| v.as_str())
    }

    fn data_dir(&self) -> Result<PathBuf, String> {
        match self.get("data-dir") {
            Some(d) => Ok(PathBuf::from(d)),
            None => data_dir().map_err(|e| e.to_string()),
        }
    }
}

fn connect(dir: &Path, wake: Wake) -> Result<App, String> {
    let ep = Endpoint::find(dir).map_err(|e| e.to_string())?;
    let mut app = App::connect(&ep, wake).map_err(|e| e.to_string())?;
    app.load_panes(dir);
    app.set_data_dir(dir.to_owned());
    Ok(app)
}

fn shot(args: &Args) -> Result<(), String> {
    let out = args.get("out").ok_or("shot needs --out")?;
    let scale: f32 = args
        .get("scale")
        .and_then(|s| s.parse().ok())
        .unwrap_or(2.0);
    let size = match args.get("size").and_then(|s| s.split_once('x')) {
        Some((w, h)) => (
            w.parse().map_err(|_| "bad --size")?,
            h.parse().map_err(|_| "bad --size")?,
        ),
        None => look::WINDOW,
    };
    let mut app = connect(&args.data_dir()?, Arc::new(|| {}))?;
    app.chrome = args.get("chrome") != Some("off");
    // Drawn window controls stand in for the traffic lights.
    app.traffic = cfg!(target_os = "macos") && !app.chrome;
    let mut ui = Ui::headless(RenderMode::from_env(), &ui_config(scale));
    if let Some(c) = args.get("clipboard") {
        ui.set_clipboard(Box::new(MemoryClipboard(Some(c.to_owned()))));
    }
    // The startup calls answer before the first frame is drawn.
    let until = Instant::now() + Duration::from_secs(5);
    while app.busy() && Instant::now() < until {
        app.pump();
        std::thread::sleep(Duration::from_millis(20));
    }
    app.frame(&mut ui, size);
    if let Some(n) = args.get("select").and_then(|n| n.parse::<usize>().ok()) {
        app.send(&mut ui, key(&format!("Mod+Digit{n}")));
    }
    for k in args.get("keys").into_iter().flat_map(|k| k.split(',')) {
        app.frame(&mut ui, size);
        app.send(&mut ui, key(k));
    }
    let px = (
        (size.0 * scale).round() as u32,
        (size.1 * scale).round() as u32,
    );
    let target = ui.offscreen(px);
    // The first frame attaches the cards; draw until each has its screen.
    let until = Instant::now() + Duration::from_secs(10);
    loop {
        app.frame(&mut ui, size);
        let ready = app
            .store
            .sessions
            .iter()
            .all(|s| s.ended() || app.pane(&s.info.id).is_some());
        if ready || Instant::now() > until {
            break;
        }
        std::thread::sleep(Duration::from_millis(50));
    }
    std::thread::sleep(Duration::from_millis(300));
    // A full atlas empties itself; the second pass fits one frame.
    app.frame(&mut ui, size);
    if !ui.render_offscreen(&target) {
        app.frame(&mut ui, size);
        ui.render_offscreen(&target);
    }
    ui.save_png(&target, out)?;
    println!("{out}");
    Ok(())
}

/// A key from its name: `Mod+KeyN`, `Alt+Digit2`, `Shift+Tab`, `F2`.
fn key(spec: &str) -> Input {
    let mut m = 0;
    let mut code = spec;
    while let Some((head, rest)) = code.split_once('+') {
        m |= match head {
            "Mod" => vorn_app::app::MOD,
            "Ctrl" => mods::CTRL,
            "Alt" => mods::ALT,
            "Shift" => mods::SHIFT,
            _ => 0,
        };
        code = rest;
    }
    // A bare letter types itself, as a keyboard's would.
    let text = code
        .strip_prefix("Key")
        .filter(|l| l.len() == 1 && m & !mods::SHIFT == 0)
        .map(|l| {
            if m & mods::SHIFT == 0 {
                l.to_lowercase()
            } else {
                l.to_owned()
            }
        });
    Input::Key {
        code: code.to_owned(),
        mods: m,
        text,
    }
}

/// Before vornd answers: a quiet screen that starts the bundled vornd once
/// and keeps trying until one answers.
struct Waiting {
    dir: PathBuf,
    wake: Wake,
    status: String,
    detail: Option<String>,
    started: Option<Instant>,
    next: Instant,
}

/// How long a started vornd has before the screen says it has not answered.
const START_WAIT: Duration = Duration::from_secs(30);
const RETRY: Duration = Duration::from_millis(500);

impl Waiting {
    fn new(dir: PathBuf, wake: Wake) -> Waiting {
        Waiting {
            dir,
            wake,
            status: String::new(),
            detail: None,
            started: None,
            next: Instant::now(),
        }
    }

    /// One attempt, when one is due: the app once vornd answers. Starts the
    /// bundled vornd after the first miss.
    fn poll(&mut self) -> Option<App> {
        let now = Instant::now();
        if now < self.next {
            return None;
        }
        self.next = now + RETRY;
        let missed = match connect(&self.dir, Arc::clone(&self.wake)) {
            Ok(app) => return Some(app),
            Err(e) => e,
        };
        match self.started {
            None => {
                self.started = Some(now);
                (self.status, self.detail) = match Server::locate() {
                    Some(server) => match server.start(&self.dir) {
                        Ok(()) => ("Starting vornd.".into(), None),
                        Err(e) => ("Could not start vornd. Retrying.".into(), Some(e)),
                    },
                    None => ("Could not reach vornd. Retrying.".into(), Some(missed)),
                };
            }
            Some(at) if now - at > START_WAIT && self.detail.is_none() => {
                self.status = "vornd has not answered. Retrying.".into();
                self.detail = Some(format!(
                    "Its log is {}.",
                    self.dir.join(launch::LOG_FILE).display()
                ));
            }
            Some(_) => {}
        }
        None
    }

    fn view(&self, ui: &Ui) -> El {
        let (w, h) = ui.viewport();
        let ink = |a| Rgba::hexa(look::INK, a);
        div()
            .size(w, h)
            .col()
            .center()
            .gap(8.0)
            .bg(Rgba::hex(look::SURFACE_BASE))
            .child(text(&self.status, 13.0, ink(look::INK_SECONDARY)))
            .child_if(
                self.detail
                    .as_deref()
                    .map(|d| text(d, 12.0, ink(look::INK_FAINT))),
            )
    }
}

/// The window's app, or the error that stopped it and how to start again.
struct Win {
    /// Set until vornd first answers.
    waiting: Option<Waiting>,
    app: Option<App>,
    error: Option<String>,
    reconnect: Box<dyn Fn() -> Result<App, String>>,
    window: Option<Arc<NativeWindow>>,
    updater: Arc<Updater>,
}

impl Win {
    /// Runs `f` on the app; a panic swaps the app for the crash screen.
    fn guard<T: Default>(&mut self, f: impl FnOnce(&mut App) -> T) -> T {
        let Some(app) = self.app.as_mut() else {
            return T::default();
        };
        match crash::guard(|| f(app)) {
            Ok(v) => v,
            Err(e) => {
                self.app = None;
                self.error = Some(e);
                T::default()
            }
        }
    }
}

impl WindowApp for Win {
    fn view(&mut self, ui: &mut Ui) -> El {
        let size = ui.viewport();
        if let Some(w) = &self.waiting {
            return w.view(ui);
        }
        match self.guard(|app| Some(app.view(ui, size))) {
            Some(el) => el,
            None => crash::view(ui, self.error.as_deref().unwrap_or_default()),
        }
    }

    fn paint(&mut self, ui: &mut Ui, laid: &Laid) {
        self.guard(|app| app.paint(ui, laid));
    }

    fn event(&mut self, ui: &mut Ui, event: Event) {
        if let Some(error) = &self.error {
            if crash::event(ui, &event, error) == Some(CrashAct::Reload) {
                match (self.reconnect)() {
                    Ok(mut app) => {
                        app.set_updater(Arc::clone(&self.updater));
                        if let Some(window) = &self.window {
                            app.opened(window);
                        }
                        self.app = Some(app);
                        self.error = None;
                    }
                    Err(e) => self.error = Some(e),
                }
            }
            return;
        }
        self.guard(|app| app.event(ui, event));
    }

    fn input(&mut self, _ui: &mut Ui, input: Input) {
        self.guard(|app| app.input(input));
    }

    fn intercept(&mut self, ui: &mut Ui, input: &Input) -> bool {
        self.guard(|app| app.intercept(ui, input))
    }

    fn background(&self) -> Rgba {
        Rgba::hex(look::SURFACE_BASE)
    }

    // Terminals take composed text.
    fn wants_ime(&self) -> bool {
        self.app.is_some()
    }

    fn ime_area(&self) -> Option<Rect> {
        None
    }

    fn opened(&mut self, window: &Arc<NativeWindow>) {
        self.window = Some(Arc::clone(window));
        self.guard(|app| app.opened(window));
    }

    fn tick(&mut self) -> Option<Instant> {
        let Some(w) = &mut self.waiting else {
            return self.guard(App::tick);
        };
        let shown = (w.status.clone(), w.detail.clone());
        let Some(mut app) = w.poll() else {
            if shown != (w.status.clone(), w.detail.clone()) {
                if let Some(window) = &self.window {
                    window.request_redraw();
                }
            }
            return Some(w.next);
        };
        self.waiting = None;
        app.set_updater(Arc::clone(&self.updater));
        if let Some(window) = &self.window {
            app.opened(window);
            // The app draws its own frame outside macOS.
            if !cfg!(target_os = "macos") {
                window.set_decorations(false);
            }
            window.request_redraw();
        }
        self.app = Some(app);
        self.guard(App::tick)
    }
}

fn window(args: &Args) -> Result<(), String> {
    // The window's waker exists only once it is open; the app's wake reaches
    // it through this slot.
    let waker: Arc<Mutex<Option<Waker>>> = Arc::default();
    let w = Arc::clone(&waker);
    let wake: Wake = Arc::new(move || {
        if let Some(w) = w.lock().ok().as_ref().and_then(|w| w.as_ref()) {
            w.wake();
        }
    });
    let dir = args.data_dir()?;
    // After an update the running vornd is the last build's: this build's
    // takes over, and the session holder keeps the terminals.
    if let Some(server) = Server::locate() {
        launch::retire_older(&dir, &server);
    }
    let updater = Updater::start(Arc::clone(&wake));
    let mut waiting = Waiting::new(dir.clone(), Arc::clone(&wake));
    // Connected before the window opens when vornd runs, so it never shows
    // the waiting screen; otherwise this starts the bundled one.
    let mut app = waiting.poll();
    if let Some(app) = &mut app {
        app.set_updater(Arc::clone(&updater));
    }
    let ready = app.is_some();
    let reconnect = Box::new(move || {
        let ep = Endpoint::find(&dir).map_err(|e| e.to_string())?;
        let mut app = App::connect(&ep, Arc::clone(&wake)).map_err(|e| e.to_string())?;
        app.set_data_dir(dir.clone());
        Ok(app)
    });
    crash::install();
    let opts = WindowOptions {
        min_size: Some((800.0, 600.0)),
        // macOS keeps its traffic lights over the app's top bar, with no
        // title drawn; elsewhere the app draws its own controls, and the
        // waiting screen has the system's until it connects.
        decorations: cfg!(target_os = "macos") || !ready,
        inset_title: cfg!(target_os = "macos").then_some(look::TRAFFIC_LIGHTS),
        ..WindowOptions::sized(look::WINDOW)
    };
    run_with("Vorn", opts, ui_config(1.0), move |_, wake| {
        if let Ok(mut slot) = waker.lock() {
            *slot = Some(wake);
        }
        Win {
            waiting: (!ready).then_some(waiting),
            app,
            error: None,
            reconnect,
            window: None,
            updater,
        }
    })
}

/// The floating widget, a window of its own over other apps.
fn widget(args: &Args) -> Result<(), String> {
    let ep = Endpoint::find(&args.data_dir()?).map_err(|e| e.to_string())?;
    let waker: Arc<Mutex<Option<Waker>>> = Arc::default();
    let w = Arc::clone(&waker);
    let wake: Wake = Arc::new(move || {
        if let Some(w) = w.lock().ok().as_ref().and_then(|w| w.as_ref()) {
            w.wake();
        }
    });
    let widget = vorn_app::widget::Widget::connect(&ep, wake)?;
    let opts = WindowOptions {
        decorations: false,
        always_on_top: true,
        skip_taskbar: true,
        transparent: true,
        corner: Some((20.0, 20.0)),
        ..WindowOptions::sized(vorn_app::widget::FULL)
    };
    run_with("Vorn widget", opts, ui_config(1.0), move |_, wake| {
        if let Ok(mut slot) = waker.lock() {
            *slot = Some(wake);
        }
        vorn_app::widget::WidgetWindow { widget }
    })
}

fn main() {
    let result = Args::parse().and_then(|args| match args.mode.as_str() {
        "shot" => shot(&args),
        "window" => window(&args),
        "widget" => widget(&args),
        other => Err(format!("unknown mode {other:?}")),
    });
    if let Err(e) = result {
        eprintln!("vorn-app: {e}");
        std::process::exit(1);
    }
}

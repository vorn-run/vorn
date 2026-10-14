//! The `vorn-app` binary.
//!
//! ```text
//! vorn-app [window] [--data-dir D]
//! vorn-app update [--channel stable|beta] [--data-dir D]
//! vorn-app --version | --help
//! vorn-app shot --out a.png [--data-dir D] [--size WxH] [--scale S] [--select N]
//!               [--keys K1,K2] [--clipboard TEXT] [--chrome off] [--layout tabs]
//! vorn-app cells --out a.html [--data-dir D] [--size COLSxROWS] [--select N]
//!               [--keys K1,K2] [--click COL:ROW] [--png a.png] [--scale S]
//!               [--layout tabs]
//! vorn-app term --session ID --out a.png [--data-dir D] [--size COLSxROWS]
//!               [--scale S]
//! ```
//!
//! `cells` draws the same frame through the Cells backend and writes it as
//! HTML, with the plain text beside it (`a.txt`) and, with `--png`, as the
//! pane painter draws a terminal screen. `term` attaches to one
//! session at `COLSxROWS`, takes that size, and draws its screen alone.
//!
//! `update` installs a newer release now, without a window, and starts it
//! (`HELP` has the details).
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
use web_time::{Duration, Instant};

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

const VERSION: &str = env!("CARGO_PKG_VERSION");

const HELP: &str = "\
Usage: Vorn [window] [--data-dir DIR]
       Vorn update [--channel stable|beta] [--data-dir DIR]
       Vorn install-cli [--data-dir DIR]
       Vorn widget [--data-dir DIR]
       Vorn --version
       Vorn --help

  window       Open Vorn (the default).
  update       Check for a newer release and install it now: download it,
               check its checksum and signature, install it, and start the
               new Vorn on DIR. Quit Vorn's window first. The channel is beta
               for a beta build, else stable; VORN_UPDATE_FEED names a feed
               to read instead.
  install-cli  Put the vorn command on PATH and say where.
  widget       Open the floating widget.

  --data-dir DIR  The data directory (default: VORN_DATA_DIR, else ~/.vorn).

shot, cells and term draw a frame offscreen, for tests and docs.
";

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
        let mode = match it.peek().map(String::as_str) {
            Some("--help" | "-h") => "help".to_owned(),
            Some("--version" | "-V") => "version".to_owned(),
            Some(m) if !m.starts_with("--") => it.next().unwrap_or_default(),
            _ => "window".to_owned(),
        };
        if matches!(mode.as_str(), "help" | "version") {
            return Ok(Args {
                mode,
                flags: Vec::new(),
            });
        }
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

    /// `--layout tabs` shows the sessions as tabs.
    fn layout(&self, app: &mut App) {
        if self.get("layout") == Some("tabs") {
            app.st.layout = vorn_app::state::LayoutMode::Tabs;
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
    app.set_notice_sink(Arc::new(vorn_app::notify::Recorded::default()));
    app.chrome = args.get("chrome") != Some("off");
    args.layout(&mut app);
    // Drawn window controls stand in for the traffic lights.
    app.traffic = cfg!(target_os = "macos") && !app.chrome;
    let mut ui = Ui::headless(RenderMode::from_env(), &ui_config(scale));
    if let Some(c) = args.get("clipboard") {
        ui.set_clipboard(Box::new(MemoryClipboard::text(c)));
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

fn cells(args: &Args) -> Result<(), String> {
    use vorn_app::ui::backend::{Backend, Cells};
    let out = args.get("out").ok_or("cells needs --out")?;
    let (cols, rows): (u16, u16) = match args.get("size").and_then(|s| s.split_once('x')) {
        Some((w, h)) => (
            w.parse().map_err(|_| "bad --size")?,
            h.parse().map_err(|_| "bad --size")?,
        ),
        None => (120, 36),
    };
    let size = vorn_app::ui::cells::units::px(cols, rows);
    let mut app = connect(&args.data_dir()?, Arc::new(|| {}))?;
    (app.chrome, app.traffic) = (false, false);
    args.layout(&mut app);
    let mut ui = Ui::cells(&ui_config(1.0));
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
    if let Some((x, y)) = args.get("click").and_then(|c| c.split_once(':')) {
        let (x, y): (f32, f32) = (
            x.parse().map_err(|_| "bad --click")?,
            y.parse().map_err(|_| "bad --click")?,
        );
        app.frame(&mut ui, size);
        ui.pointer_move(x + 0.5, y + 0.5);
        ui.pointer_down();
        app.events(&mut ui);
        ui.pointer_up();
        app.events(&mut ui);
    }
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
    let mut target = Cells::offscreen(cols, rows);
    app.frame(&mut ui, size);
    target.present(&mut ui)?;
    let title = format!("vorn {cols}x{rows}");
    std::fs::write(out, target.buffer.to_html(&title)).map_err(|e| e.to_string())?;
    let text: Vec<String> = (0..rows).map(|r| target.buffer.row_text(r)).collect();
    let txt = Path::new(out).with_extension("txt");
    std::fs::write(&txt, text.join("\n") + "\n").map_err(|e| e.to_string())?;
    if let Some(png) = args.get("png") {
        let scale = args
            .get("scale")
            .and_then(|s| s.parse().ok())
            .unwrap_or(2.0);
        paint_view(
            &buffer_view(&target.buffer),
            png,
            scale,
            vorn_app::paint::Caret::Hidden,
        )?;
    }
    println!("{out}");
    Ok(())
}

/// A cells frame as a terminal screen, so it can be painted as a session's is.
fn buffer_view(b: &vorn_app::ui::cells::Buffer) -> vorn_app::view::PaneView {
    use vorn_app::ui::cells::{attr, Sym};
    use vorn_app::view::{flags, PaneView, Run};
    let rgb = |c: [u8; 3]| (u32::from(c[0]) << 16) | (u32::from(c[1]) << 8) | u32::from(c[2]);
    let mut v = PaneView {
        cols: b.cols,
        rows: b.rows,
        bg: rgb([0; 3]),
        fg: rgb([255; 3]),
        primary: true,
        ..PaneView::default()
    };
    for row in 0..b.rows {
        for col in 0..b.cols {
            let Some(c) = b.get(col, row) else { continue };
            if c.sym == Sym::TAIL {
                continue;
            }
            let s = c.sym.as_str();
            let wide = b.get(col + 1, row).is_some_and(|n| n.sym == Sym::TAIL);
            let (mut fg, mut bg) = (c.fg, c.bg);
            if c.attr & attr::INVERSE != 0 {
                std::mem::swap(&mut fg, &mut bg);
            }
            let mut f = 0;
            for (a, fl) in [
                (attr::BOLD, flags::BOLD),
                (attr::DIM, flags::FAINT),
                (attr::ITALIC, flags::ITALIC),
                (attr::UNDERLINE, flags::UNDERLINE),
            ] {
                if c.attr & a != 0 {
                    f |= fl;
                }
            }
            if !s.is_ascii() {
                f |= flags::CLUSTER;
            }
            if wide {
                f |= flags::WIDE;
            }
            v.runs.push(Run {
                row,
                col,
                ncols: if wide { 2 } else { 1 },
                flags: f,
                fg: rgb(fg),
                bg: rgb(bg),
                text_off: v.text.len() as u32,
                text_len: s.len() as u32,
            });
            v.text.extend_from_slice(s.as_bytes());
        }
    }
    if let Some(c) = b.cursor {
        (v.cursor_x, v.cursor_y, v.cursor_visible) = (c.col, c.row, true);
    }
    v
}

/// Paints a terminal screen to a PNG with the app's pane painter.
fn paint_view(
    view: &vorn_app::view::PaneView,
    out: &str,
    scale: f32,
    caret: vorn_app::paint::Caret,
) -> Result<(), String> {
    use vorn_app::paint;
    let mut ui = Ui::headless(RenderMode::from_env(), &ui_config(scale));
    let (cw, ch) = ui.text.cell_logical();
    let size = (f32::from(view.cols) * cw, f32::from(view.rows) * ch);
    let px = (
        (size.0 * scale).round() as u32,
        (size.1 * scale).round() as u32,
    );
    let target = ui.offscreen(px);
    for _ in 0..2 {
        ui.begin(Rgba::hex(look::SURFACE_BASE));
        let _ = ui.layout(div(), size);
        let r = Rect {
            x: 0.0,
            y: 0.0,
            w: size.0,
            h: size.1,
        };
        paint::pane(&mut ui, view, r, caret, "", &[], 0.0);
        if ui.render_offscreen(&target) {
            break;
        }
    }
    ui.save_png(&target, out)
}

fn term(args: &Args) -> Result<(), String> {
    let session = args.get("session").ok_or("term needs --session")?;
    let out = args.get("out").ok_or("term needs --out")?;
    let scale: f32 = args
        .get("scale")
        .and_then(|s| s.parse().ok())
        .unwrap_or(2.0);
    let (cols, rows): (u16, u16) = match args.get("size").and_then(|s| s.split_once('x')) {
        Some((w, h)) => (
            w.parse().map_err(|_| "bad --size")?,
            h.parse().map_err(|_| "bad --size")?,
        ),
        None => (120, 36),
    };
    let ep = Endpoint::find(&args.data_dir()?).map_err(|e| e.to_string())?;
    let grid = vorn_app::grid::Grid::connect(&ep.grid, &ep.token, Arc::new(|| {}))
        .map_err(|e| e.to_string())?;
    grid.attach(session, cols, rows, 0);
    grid.resize(session, cols, rows);
    let until = Instant::now() + Duration::from_secs(10);
    while grid.view(session).is_none() && Instant::now() < until {
        std::thread::sleep(Duration::from_millis(50));
    }
    std::thread::sleep(Duration::from_millis(500));
    let view = grid.view(session).ok_or("no screen for the session")?;
    paint_view(&view, out, scale, vorn_app::paint::Caret::Live)?;
    println!("{out}");
    Ok(())
}

/// A key from its name: `Mod+KeyN`, `Alt+Digit2`, `Shift+Tab`, `F2`.
fn key(spec: &str) -> Input {
    let mut m = 0;
    let mut code = spec;
    while let Some((head, rest)) = code.split_once('+') {
        m |= match head {
            "Mod" => vorn_app::app::MOD(),
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
    /// The launch step's notice about the `vorn` command, for the first app.
    cli: Option<std::sync::mpsc::Receiver<Option<vorn_app::cli_link::Notice>>>,
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
        if let Some(rx) = self.cli.take() {
            app.set_cli_launch(rx);
        }
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
    let mut cli = Some(vorn_app::cli_link::start(dir.clone(), Arc::clone(&wake)));
    let mut waiting = Waiting::new(dir.clone(), Arc::clone(&wake));
    // Connected before the window opens when vornd runs, so it never shows
    // the waiting screen; otherwise this starts the bundled one.
    let mut app = waiting.poll();
    if let Some(app) = &mut app {
        app.set_updater(Arc::clone(&updater));
        if let Some(rx) = cli.take() {
            app.set_cli_launch(rx);
        }
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
            cli,
        }
    })
}

/// The launch step alone, for checking a package: links the `vorn` command
/// (or not) and prints what the notice would say.
fn install_cli(args: &Args) -> Result<(), String> {
    match vorn_app::cli_link::on_launch(&args.data_dir()?) {
        Some(n) if n.error => Err(n.text),
        Some(n) => {
            println!("{}", n.text);
            Ok(())
        }
        None => {
            println!("{}", vorn_app::cli_link::state().path);
            Ok(())
        }
    }
}

/// Checks for an update and installs it now; the new app starts once this
/// process exits.
fn update(args: &Args) -> Result<(), String> {
    use vorn_app::update::{self, Outcome};
    let channel = args
        .get("channel")
        .unwrap_or_else(|| update::default_channel(VERSION));
    if !matches!(channel, "stable" | "beta") {
        return Err(format!("--channel is stable or beta, not {channel:?}"));
    }
    let dir = args.data_dir()?;
    match update::update_now(channel, Some(&dir), &|step| println!("{step}"))? {
        Outcome::UpToDate => println!("Vorn {VERSION} is up to date"),
        Outcome::Manual { version } => println!(
            "Vorn {version} is out; install its package from {}",
            update::RELEASES
        ),
        Outcome::Installed { version } => println!("Installed Vorn {version}; starting it"),
    }
    Ok(())
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
        "cells" => cells(&args),
        "term" => term(&args),
        "window" => window(&args),
        "widget" => widget(&args),
        "install-cli" => install_cli(&args),
        "update" => update(&args),
        "help" => {
            print!("Vorn {VERSION}\n\n{HELP}");
            Ok(())
        }
        "version" => {
            println!("Vorn {VERSION}");
            Ok(())
        }
        other => Err(format!("unknown mode {other:?}")),
    });
    if let Err(e) = result {
        eprintln!("vorn-app: {e}");
        std::process::exit(1);
    }
}

//! The `vorn-app` binary.
//!
//! ```text
//! vorn-app [window] [--data-dir D]
//! vorn-app shot --out a.png [--data-dir D] [--size WxH] [--scale S] [--select N]
//!               [--keys K1,K2] [--chrome off]
//! ```
//!
//! `--keys` sends shortcuts before the shot (`Mod+KeyN`, `Alt+Digit2`,
//! `F2`, `Escape`).
//!
//! Both find vornd through `D` (default `VORN_DATA_DIR`, else `~/.vorn`).
//! `shot` draws one frame offscreen once every card has its screen.

use std::path::PathBuf;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use vorn_app::client::{data_dir, Endpoint};
use vorn_app::crash::{self, CrashAct};
use vorn_app::ui::{
    mods, run_with, El, Event, Input, Laid, NativeWindow, Rect, RenderMode, Rgba, Ui, Waker,
    WindowApp, WindowOptions,
};
use vorn_app::{look, ui_config, App};

struct Args {
    mode: String,
    flags: Vec<(String, String)>,
}

impl Args {
    fn parse() -> Result<Args, String> {
        let mut it = std::env::args().skip(1).peekable();
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

fn connect(args: &Args, wake: vorn_app::client::rpc::Wake) -> Result<App, String> {
    let dir = args.data_dir()?;
    let ep = Endpoint::find(&dir).map_err(|e| e.to_string())?;
    let mut app = App::connect(&ep, wake).map_err(|e| e.to_string())?;
    app.load_panes(&dir);
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
    let mut app = connect(args, Arc::new(|| {}))?;
    app.set_data_dir(args.data_dir()?);
    app.chrome = args.get("chrome") != Some("off");
    // Drawn window controls stand in for the traffic lights.
    app.traffic = cfg!(target_os = "macos") && !app.chrome;
    let mut ui = Ui::headless(RenderMode::from_env(), &ui_config(scale));
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
    Input::Key {
        code: code.to_owned(),
        mods: m,
        text: None,
    }
}

/// The window's app, or the error that stopped it and how to start again.
struct Win {
    app: Option<App>,
    error: Option<String>,
    reconnect: Box<dyn Fn() -> Result<App, String>>,
    window: Option<Arc<NativeWindow>>,
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
        self.guard(App::tick)
    }
}

fn window(args: &Args) -> Result<(), String> {
    // The window's waker exists only once it is open; connecting first
    // reports a missing vornd before any window appears.
    let waker: Arc<Mutex<Option<Waker>>> = Arc::default();
    let w = Arc::clone(&waker);
    let wake: vorn_app::client::rpc::Wake = Arc::new(move || {
        if let Some(w) = w.lock().ok().as_ref().and_then(|w| w.as_ref()) {
            w.wake();
        }
    });
    let dir = args.data_dir()?;
    let mut app = connect(args, Arc::clone(&wake))?;
    app.set_data_dir(dir.clone());
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
        // title drawn; elsewhere the app draws its own controls.
        decorations: cfg!(target_os = "macos"),
        inset_title: cfg!(target_os = "macos").then_some(look::TRAFFIC_LIGHTS),
        ..WindowOptions::sized(look::WINDOW)
    };
    run_with("Vorn", opts, ui_config(1.0), move |_, wake| {
        if let Ok(mut slot) = waker.lock() {
            *slot = Some(wake);
        }
        Win {
            app: Some(app),
            error: None,
            reconnect,
            window: None,
        }
    })
}

/// The floating widget, a window of its own over other apps.
fn widget(args: &Args) -> Result<(), String> {
    let ep = Endpoint::find(&args.data_dir()?).map_err(|e| e.to_string())?;
    let waker: Arc<Mutex<Option<Waker>>> = Arc::default();
    let w = Arc::clone(&waker);
    let wake: vorn_app::client::rpc::Wake = Arc::new(move || {
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

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
use vorn_app::ui::{
    mods, run_with, El, Event, Input, Laid, Rect, RenderMode, Rgba, Ui, Waker, WindowApp,
    WindowOptions,
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
    let ep = Endpoint::find(&args.data_dir()?).map_err(|e| e.to_string())?;
    App::connect(&ep, wake).map_err(|e| e.to_string())
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
    app.chrome = args.get("chrome") != Some("off");
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

struct Win(App);

impl WindowApp for Win {
    fn view(&mut self, ui: &mut Ui) -> El {
        let size = ui.viewport();
        self.0.view(ui, size)
    }

    fn paint(&mut self, ui: &mut Ui, laid: &Laid) {
        self.0.paint(ui, laid);
    }

    fn event(&mut self, ui: &mut Ui, event: Event) {
        self.0.event(ui, event);
    }

    fn input(&mut self, _ui: &mut Ui, input: Input) {
        self.0.input(input);
    }

    fn intercept(&mut self, ui: &mut Ui, input: &Input) -> bool {
        self.0.intercept(ui, input)
    }

    fn background(&self) -> Rgba {
        Rgba::hex(look::SURFACE_BASE)
    }

    // Terminals take composed text.
    fn wants_ime(&self) -> bool {
        true
    }

    fn ime_area(&self) -> Option<Rect> {
        None
    }
}

fn window(args: &Args) -> Result<(), String> {
    // The window's waker exists only once it is open; connecting first
    // reports a missing vornd before any window appears.
    let waker: Arc<Mutex<Option<Waker>>> = Arc::default();
    let w = Arc::clone(&waker);
    let app = connect(
        args,
        Arc::new(move || {
            if let Some(w) = w.lock().ok().as_ref().and_then(|w| w.as_ref()) {
                w.wake();
            }
        }),
    )?;
    let opts = WindowOptions {
        min_size: Some((800.0, 600.0)),
        // macOS keeps its title bar; elsewhere the app draws its own.
        decorations: cfg!(target_os = "macos"),
        ..WindowOptions::sized(look::WINDOW)
    };
    run_with("Vorn", opts, ui_config(1.0), move |_, wake| {
        if let Ok(mut slot) = waker.lock() {
            *slot = Some(wake);
        }
        Win(app)
    })
}

fn main() {
    let result = Args::parse().and_then(|args| match args.mode.as_str() {
        "shot" => shot(&args),
        "window" => window(&args),
        other => Err(format!("unknown mode {other:?}")),
    });
    if let Err(e) = result {
        eprintln!("vorn-app: {e}");
        std::process::exit(1);
    }
}

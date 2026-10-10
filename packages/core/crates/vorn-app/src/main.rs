//! The `vorn-app` binary.
//!
//! ```text
//! vorn-app [window] [--data-dir D]
//! vorn-app shot --out a.png [--data-dir D] [--size WxH] [--scale S] [--select N]
//! ```
//!
//! Both find vornd through `D` (default `VORN_DATA_DIR`, else `~/.vorn`).
//! `shot` draws one frame offscreen once every card has its screen.

use std::path::PathBuf;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use vorn_app::client::{data_dir, Endpoint};
use vorn_app::ui::{
    run_window, Gpu, Input, Offscreen, TreeUpdate, Ui, Waker, WindowApp, OFFSCREEN_FORMAT,
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
    if let Some(n) = args.get("select").and_then(|n| n.parse::<usize>().ok()) {
        app.input(jump(n));
    }
    let mut ui = Ui::new(Gpu::headless()?, OFFSCREEN_FORMAT, &ui_config(scale));
    let px = (
        (size.0 * scale).round() as u32,
        (size.1 * scale).round() as u32,
    );
    let target = Offscreen::new(&ui.gpu, px);
    // The first frame attaches the cards; draw until each has its screen.
    let until = Instant::now() + Duration::from_secs(10);
    loop {
        app.frame(&mut ui, size);
        let ready = app
            .store
            .sessions
            .iter()
            .all(|s| app.pane(&s.info.id).is_some());
        if ready || Instant::now() > until {
            break;
        }
        std::thread::sleep(Duration::from_millis(50));
    }
    std::thread::sleep(Duration::from_millis(300));
    app.frame(&mut ui, size);
    ui.render(&target.view, target.size);
    target.save_png(&ui.gpu, out)?;
    println!("{out}");
    Ok(())
}

/// Mod+`n`: selects the `n`th card.
fn jump(n: usize) -> Input {
    let m = if cfg!(target_os = "macos") {
        vorn_app::ui::mods::SUPER
    } else {
        vorn_app::ui::mods::CTRL
    };
    Input::Key {
        code: format!("Digit{n}"),
        mods: m,
        text: None,
    }
}

struct Win(App);

impl WindowApp for Win {
    fn frame(&mut self, ui: &mut Ui, size: (f32, f32)) -> TreeUpdate {
        self.0.frame(ui, size)
    }

    fn input(&mut self, input: Input) {
        self.0.input(input);
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
    run_window("Vorn", look::WINDOW, ui_config(1.0), move |_, wake| {
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

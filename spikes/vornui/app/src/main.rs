//! The vornui prototype: today's main screen and a terminal grid over a
//! test vornd, with the modes the spike measures.
//!
//! ```text
//! vornui-proto shot --screen main|grid [--panes N] [--scale S] --out a.png
//! vornui-proto bench --panes N [--secs S] --out a.json
//! vornui-proto coldstart [--panes N] [--runs R] --out a.json
//! vornui-proto ime --out a.json        (also writes PNGs next to it)
//! vornui-proto a11y --out a.json
//! vornui-proto window [--panes N]      (a real window; never run by the benches)
//! ```

mod screen;
mod term;

use std::collections::HashMap;
use std::sync::Arc;
use std::time::{Duration, Instant};

use serde_json::{json, Value};
use spike_shared::bench::{self, Frame};
use spike_shared::{look, Args, Grid, Load, PaneView, Rig};
use vornui::accesskit::{Role, TreeUpdate};
use vornui::input::from_ime;
use vornui::winit::event::Ime;
use vornui::{El, Input, Laid, Offscreen, Pacer, Rect, RenderMode, Rgba, Ui, UiConfig, Waker};

fn config(scale: f32) -> UiConfig {
    UiConfig {
        scale,
        ui_font: look::UI_FONT,
        mono_font: look::MONO_FONT,
        term_size: look::TERM_FONT,
    }
}

/// What the app shows, apart from how it is drawn: the panes' latest views
/// and the IME composition. No grid means the main screen.
struct App {
    grid: Option<Arc<Grid>>,
    views: Vec<Option<PaneView>>,
    preedit: String,
    logo: u32,
    /// Put each pane's text in the accessibility tree (costly; a11y mode).
    a11y_text: bool,
    probe_hit: bool,
    /// The typed-into pane (0) changed in the frame being built.
    changed: bool,
}

impl App {
    fn new(ui: &mut Ui, grid: Option<Arc<Grid>>) -> Result<App, String> {
        let (w, h, rgba) = vornui::decode_png(look::LOGO_PNG)?;
        let logo = ui.image(w, h, &rgba).ok_or("color atlas full")?;
        let n = grid.as_ref().map_or(0, |g| g.panes());
        for i in 0..n {
            ui.custom_a11y.insert(
                i as u64,
                (Role::Terminal, format!("pane-{i}"), String::new()),
            );
        }
        Ok(App {
            grid,
            views: vec![None; n],
            preedit: String::new(),
            logo,
            a11y_text: false,
            probe_hit: false,
            changed: false,
        })
    }

    /// Every input reaches the terminal here, from a window or a bench.
    fn handle(&mut self, input: Input) {
        let Some(g) = &self.grid else {
            return;
        };
        match input {
            Input::Key { code, mods, text } => g.key(0, &code, mods, text.as_deref()),
            Input::Preedit(s) => self.preedit = s,
            Input::Commit(s) => {
                self.preedit.clear();
                g.text(0, &s);
            }
        }
    }

    /// Pulls changed panes and builds the frame's elements.
    fn view(&mut self, ui: &mut Ui) -> El {
        self.changed = false;
        let Some(g) = self.grid.clone() else {
            return screen::main_screen(self.logo);
        };
        for i in g.take_dirty() {
            self.changed |= i == 0;
            if let Some(v) = g.view(i) {
                self.probe_hit |= v.probe_hit;
                self.views[i] = Some(v);
            }
            if self.a11y_text {
                if let Some(a) = ui.custom_a11y.get_mut(&(i as u64)) {
                    a.2 = g.screen_text(i);
                }
            }
        }
        screen::grid_screen(self.views.len())
    }

    /// Paints the panes into their laid-out boxes.
    fn paint_panes(&self, ui: &mut Ui, laid: &Laid) {
        for (id, r) in &laid.customs {
            let i = *id as usize;
            if let Some(Some(v)) = self.views.get(i) {
                let pre = if i == 0 { self.preedit.as_str() } else { "" };
                term::paint_pane(ui, v, *r, pre);
            }
        }
    }

    /// Pulls changed panes and paints the frame into `ui.scene`.
    fn paint(&mut self, ui: &mut Ui, size: (f32, f32)) -> Laid {
        ui.begin(Rgba::hex(look::SURFACE_BASE));
        let root = self.view(ui);
        let laid = ui.layout(root, size);
        self.paint_panes(ui, &laid);
        laid
    }

    fn complete(&self) -> bool {
        self.views.iter().all(Option::is_some)
    }
}

/// The app drawn offscreen, for the benches and screenshots, paced as the
/// window paces it.
struct Headless {
    ui: Ui,
    target: Offscreen,
    app: App,
    pacer: Pacer,
}

/// `VORNUI_RENDERER=cpu|gpu` picks the renderer; by default the GPU unless
/// the adapter is a software one.
fn headless_ui(scale: f32) -> Ui {
    Ui::headless(RenderMode::from_env(), &config(scale))
}

impl Headless {
    fn new(scale: f32, grid: Option<Arc<Grid>>) -> Result<Headless, String> {
        Headless::with_ui(headless_ui(scale), grid)
    }

    fn with_ui(mut ui: Ui, grid: Option<Arc<Grid>>) -> Result<Headless, String> {
        let k = ui.scale();
        let px = (
            (look::WINDOW.0 * k).round() as u32,
            (look::WINDOW.1 * k).round() as u32,
        );
        let target = ui.offscreen(px);
        let app = App::new(&mut ui, grid)?;
        Ok(Headless {
            ui,
            target,
            app,
            pacer: Pacer::new(bench::PERIOD),
        })
    }

    fn draw(&mut self) -> TreeUpdate {
        // A full atlas empties itself; the second pass fits one frame.
        let mut laid = self.app.paint(&mut self.ui, look::WINDOW);
        if !self.ui.render_offscreen(&self.target) {
            laid = self.app.paint(&mut self.ui, look::WINDOW);
            self.ui.render_offscreen(&self.target);
        }
        laid.tree
    }

    fn save_png(&mut self, out: &str) -> Result<(), String> {
        self.ui.save_png(&self.target, out)
    }
}

impl bench::Proto for Headless {
    fn key(&mut self, ch: char) {
        self.pacer.input(Instant::now());
        self.app.handle(Input::char(ch));
    }

    fn enter(&mut self) {
        self.pacer.input(Instant::now());
        self.app.handle(Input::Key {
            code: "Enter".into(),
            mods: 0,
            text: Some("\r".into()),
        });
    }

    fn urgent(&self) -> bool {
        self.pacer.urgent(Instant::now())
    }

    fn frame(&mut self) -> Frame {
        let now = Instant::now();
        self.app.probe_hit = false;
        self.draw();
        self.pacer.frame(now, self.app.changed);
        Frame {
            drawn: true,
            probe_hit: self.app.probe_hit,
            complete: self.app.complete(),
        }
    }
}

/// The terminal cell in logical pixels, which sizes the panes.
fn cell(scale: f32) -> (f32, f32) {
    vornui::TextSystem::new(scale, look::UI_FONT, look::MONO_FONT, look::TERM_FONT).cell_logical()
}

fn rig(panes: usize, load: Load, cell: (f32, f32)) -> Result<(Rig, Arc<Grid>), String> {
    let (cols, rows) = spike_shared::pane_cells(panes, cell);
    let rig = Rig::start(panes, load, cols, rows)?;
    let grid = rig.connect(cols, rows)?;
    if !spike_shared::wait_snapshots(&grid, Duration::from_secs(20)) {
        return Err("panes never got their first screen".into());
    }
    Ok((rig, grid))
}

fn ms(d: Duration) -> f64 {
    d.as_secs_f64() * 1000.0
}

fn shot(args: &Args) -> Result<Value, String> {
    let scale = args
        .get("scale")
        .and_then(|s| s.parse().ok())
        .unwrap_or(look::SCALE);
    let out = args.out("results/vornui-main.png");
    let grid_screen = args.get("screen") == Some("grid");
    let panes = args.num("panes", 8) as usize;
    let rig = if grid_screen {
        Some(rig(panes, Load::Still, cell(scale))?)
    } else {
        None
    };
    if rig.is_some() {
        // The build log is a few screens; let it land before drawing.
        std::thread::sleep(Duration::from_millis(1500));
    }
    let mut p = Headless::new(scale, rig.as_ref().map(|r| r.1.clone()))?;
    p.draw();
    // Redraws of the whole screen, every element rebuilt, nothing changed.
    let mut times = Vec::new();
    for _ in 0..120 {
        let t = Instant::now();
        p.draw();
        times.push(ms(t.elapsed()));
    }
    p.save_png(&out)?;
    let (quads, sprites) = p.ui.scene.counts();
    Ok(json!({
        "proto": "vornui",
        "screen": if grid_screen { "grid" } else { "main" },
        "panes": if grid_screen { panes } else { 0 },
        "scale": scale,
        "png": out,
        "adapter": p.ui.renderer_name(),
        "quads": quads,
        "sprites": sprites,
        "redraw_ms": bench::summary(&mut times),
    }))
}

fn run_bench(args: &Args) -> Result<Value, String> {
    let panes = args.num("panes", 8) as usize;
    let (_rig, grid) = rig(panes, Load::Busy, cell(look::SCALE))?;
    let mut p = Headless::new(look::SCALE, Some(grid.clone()))?;
    let mut v = bench::run(&grid, &mut p, &spike_shared::bench_config(args));
    v["adapter"] = json!(p.ui.renderer_name());
    v["proto"] = json!("vornui");
    Ok(v)
}

/// A fresh process to its first complete frame: GPU, fonts, grid, draw.
fn cold_child() -> Result<(), String> {
    let ui = headless_ui(look::SCALE);
    let grid = spike_shared::cold_child_grid(ui.text.cell_logical())?;
    let mut p = Headless::with_ui(ui, Some(grid.clone()))?;
    bench::first_frame(&grid, &mut p, Duration::from_secs(20)).ok_or("no first frame")?;
    spike_shared::print_first_frame();
    Ok(())
}

/// Japanese input through the IME path: winit's composition events become
/// app input, the preedit is drawn at the cursor, the commit reaches the
/// pty. The OS IME itself is not driven (no window); its events are.
fn ime(args: &Args) -> Result<Value, String> {
    let out = args.out("results/vornui-ime.json");
    let (_rig, grid) = rig(1, Load::Busy, cell(look::SCALE))?;
    let mut p = Headless::new(look::SCALE, Some(grid.clone()))?;
    for e in [Ime::Enabled, Ime::Preedit("にほんご".into(), Some((0, 12)))] {
        if let Some(i) = from_ime(&e) {
            p.app.handle(i);
        }
    }
    p.draw();
    let png_pre = out.replace(".json", "-preedit.png");
    p.save_png(&png_pre)?;
    let preedit_drawn = p.app.preedit == "にほんご";
    if let Some(i) = from_ime(&Ime::Commit("日本語".into())) {
        p.app.handle(i);
    }
    let committed = spike_shared::wait_for_text(&grid, 0, "日本語", Duration::from_secs(5));
    std::thread::sleep(Duration::from_millis(100));
    p.draw();
    let png = out.replace(".json", ".png");
    p.save_png(&png)?;
    Ok(json!({
        "proto": "vornui",
        "path": "winit Ime::Preedit/Commit -> vornui Input -> grid text",
        "preedit_drawn": preedit_drawn,
        "commit_reached_pty": committed,
        "screen_line": grid.screen_text(0).lines().find(|l| l.contains("日本語")),
        "png_preedit": png_pre,
        "png_commit": png,
        "simulated": "composition events are injected; no OS IME window is driven",
    }))
}

fn tree_json(t: &TreeUpdate) -> Value {
    let mut roles: HashMap<String, usize> = HashMap::new();
    for (_, n) in &t.nodes {
        *roles.entry(format!("{:?}", n.role())).or_default() += 1;
    }
    json!({
        "nodes": t.nodes.len(),
        "roles": roles,
        "tree": serde_json::to_value(t).unwrap_or(Value::Null),
    })
}

fn a11y() -> Result<Value, String> {
    let mut main = Headless::new(look::SCALE, None)?;
    let main_tree = main.draw();
    let (_rig, grid) = rig(2, Load::Still, cell(look::SCALE))?;
    std::thread::sleep(Duration::from_millis(500));
    let mut g = Headless::new(look::SCALE, Some(grid))?;
    g.app.a11y_text = true;
    let grid_tree = g.draw();
    Ok(json!({
        "proto": "vornui",
        "main": tree_json(&main_tree),
        "grid": tree_json(&grid_tree),
    }))
}

struct Win {
    app: App,
    /// Pane 0's box from the last layout (logical pixels), for the IME.
    pane0: Option<Rect>,
    /// The terminal cell in logical pixels, from the live text system.
    cell: (f32, f32),
}

impl vornui::App for Win {
    fn view(&mut self, ui: &mut Ui) -> El {
        self.app.view(ui)
    }

    fn paint(&mut self, ui: &mut Ui, laid: &Laid) {
        let k = ui.scale();
        self.cell = ui.text.cell_logical();
        self.pane0 = laid
            .customs
            .iter()
            .find(|(id, _)| *id == 0)
            .map(|(_, r)| Rect::new(r.x / k, r.y / k, r.w / k, r.h / k));
        self.app.paint_panes(ui, laid);
    }

    fn input(&mut self, _: &mut Ui, input: Input) {
        self.app.handle(input);
    }

    fn echoed(&mut self) -> bool {
        self.app.changed
    }

    fn wants_ime(&self) -> bool {
        self.app.grid.is_some()
    }

    fn ime_area(&self) -> Option<Rect> {
        let v = self.app.views.first()?.as_ref()?;
        let r = self.pane0?;
        let (cw, ch) = self.cell;
        Some(Rect::new(
            r.x + f32::from(v.cursor_x) * cw,
            r.y + f32::from(v.cursor_y) * ch,
            cw,
            ch,
        ))
    }

    fn background(&self) -> Rgba {
        Rgba::hex(look::SURFACE_BASE)
    }
}

/// Wakes the window whenever the grid changes.
fn watch(grid: Arc<Grid>, waker: Waker) {
    std::thread::spawn(move || {
        let mut seen = 0;
        while !grid.closed() {
            let now = grid.wait(seen, Instant::now() + Duration::from_secs(1));
            if now != seen {
                seen = now;
                waker.wake();
            }
        }
    });
}

/// A real window over a test vornd, for trying it by hand.
fn window(args: &Args) -> Result<Value, String> {
    let panes = args.num("panes", 4) as usize;
    let (_rig, grid) = rig(panes, Load::Busy, cell(look::SCALE))?;
    let mut err = None;
    vornui::run("Vorn", look::WINDOW, config(1.0), |ui, waker| {
        watch(grid.clone(), waker);
        let app = App::new(ui, Some(grid.clone())).unwrap_or_else(|e| {
            err = Some(e);
            App {
                grid: None,
                views: Vec::new(),
                preedit: String::new(),
                logo: 0,
                a11y_text: false,
                probe_hit: false,
                changed: false,
            }
        });
        Win {
            app,
            pane0: None,
            cell: ui.text.cell_logical(),
        }
    })?;
    err.map_or(Ok(json!({})), Err)
}

fn main() {
    let args = Args::parse();
    let result = match args.mode.as_str() {
        "produce" => {
            spike_shared::produce::main(args.get("kind").unwrap_or("echo"));
            return;
        }
        "coldchild" => {
            if let Err(e) = cold_child() {
                eprintln!("coldchild: {e}");
                std::process::exit(1);
            }
            return;
        }
        "shot" => shot(&args),
        "bench" => run_bench(&args),
        "coldstart" => spike_shared::cold_start(&args, cell(look::SCALE)),
        "ime" => ime(&args),
        "a11y" => a11y(),
        "window" => window(&args),
        other => Err(format!("unknown mode {other:?}")),
    };
    match result {
        Ok(v) => {
            let out = args.out("");
            if out.ends_with(".json") {
                spike_shared::write_json(&out, &v);
            } else if out.ends_with(".png") {
                spike_shared::write_json(&out.replace(".png", ".json"), &v);
            }
            let mut line = v.clone();
            if let Some(o) = line.as_object_mut() {
                o.remove("main");
                o.remove("grid");
            }
            println!("{line}");
        }
        Err(e) => {
            eprintln!("{}: {e}", args.mode);
            std::process::exit(1);
        }
    }
}

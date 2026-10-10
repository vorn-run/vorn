//! The GPUI prototype: today's main screen and a terminal grid over a test
//! vornd, drawn by GPUI's headless window (its own renderer on macOS, its
//! wgpu renderer on Windows) with the same modes as the vornui prototype.
//!
//! ```text
//! gpui-proto shot --screen main|grid [--panes N] [--scale S] --out a.png
//! gpui-proto bench --panes N [--secs S] --out a.json
//! gpui-proto coldstart [--panes N] [--runs R] --out a.json
//! gpui-proto ime --out a.json        (also writes PNGs next to it)
//! gpui-proto a11y --out a.json
//! ```

mod screen;
mod term;

use std::borrow::Cow;
use std::cell::Cell;
use std::collections::HashMap;
use std::ops::Range;
use std::rc::Rc;
use std::sync::Arc;
use std::time::{Duration, Instant};

use gpui::{
    canvas, div, font, point, prelude::*, px, size, AnyElement, AnyWindowHandle, AssetSource,
    Bounds, Context, ElementInputHandler, Entity, EntityInputHandler, FocusHandle,
    HeadlessAppContext, Image, ImageFormat, KeyDownEvent, Keystroke, Pixels,
    PlatformHeadlessRenderer, PlatformTextSystem, Role, SharedString, TextRenderingMode,
    UTF16Selection, Window, WindowHandle,
};
use serde_json::{json, Value};
use spike_shared::bench::{self, Frame};
use spike_shared::{look, Args, Grid, Load, PaneView, Rig};
use term::Metrics;

const PROTO: &str = "gpui";

const RENDERER: &str = if cfg!(windows) {
    "gpui wgpu headless"
} else {
    "gpui metal headless"
};

/// Icons as `name@stroke`, from the shared set.
struct Icons;

impl AssetSource for Icons {
    fn load(&self, path: &str) -> anyhow::Result<Option<Cow<'static, [u8]>>> {
        let (name, stroke) = path.split_once('@').unwrap_or((path, "2"));
        let stroke = stroke.parse().unwrap_or(2.0);
        Ok(look::icon(name, stroke).map(|s| Cow::Owned(s.into_bytes())))
    }

    fn list(&self, _: &str) -> anyhow::Result<Vec<SharedString>> {
        Ok(Vec::new())
    }
}

fn text_system() -> Arc<dyn PlatformTextSystem> {
    gpui_platform::current_platform(true).text_system()
}

fn renderer() -> anyhow::Result<Option<Box<dyn PlatformHeadlessRenderer>>> {
    // GPUI's Windows renderer has no offscreen path; its wgpu one does.
    #[cfg(windows)]
    {
        gpui_wgpu::WgpuHeadlessRenderer::new()
            .map(|r| Some(Box::new(r) as Box<dyn PlatformHeadlessRenderer>))
    }
    #[cfg(not(windows))]
    {
        gpui_platform::current_headless_renderer()
    }
}

fn metrics(ts: &gpui::TextSystem, scale: f32) -> Metrics {
    let f = font(look::MONO_FONT);
    let fs = px(look::TERM_FONT);
    let id = ts.resolve_font(&f);
    let cw = ts.advance(id, fs, 'M').map_or(fs * 0.6, |a| a.width);
    // The vornui prototype's line: 1.2 em on whole device pixels.
    let ch = px((look::TERM_FONT * 1.2 * scale).ceil() / scale);
    Metrics {
        font: f,
        size: fs,
        cell: size(cw, ch),
    }
}

/// The terminal cell in logical pixels, which sizes the panes.
fn cell(scale: f32) -> (f32, f32) {
    let ts = gpui::TextSystem::new(text_system());
    let m = metrics(&ts, scale);
    (f32::from(m.cell.width), f32::from(m.cell.height))
}

/// The window's root view: the panes' latest views and the IME composition.
/// No grid means the main screen.
struct Root {
    grid: Option<Arc<Grid>>,
    views: Vec<Option<Rc<PaneView>>>,
    texts: Vec<SharedString>,
    preedit: String,
    logo: Arc<Image>,
    metrics: Rc<Metrics>,
    focus: FocusHandle,
    /// Put each pane's text in the accessibility tree (costly; a11y mode).
    a11y_text: bool,
    probe_hit: Rc<Cell<bool>>,
}

impl Root {
    fn new(
        grid: Option<Arc<Grid>>,
        metrics: Metrics,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Root {
        let n = grid.as_ref().map_or(0, |g| g.panes());
        let focus = cx.focus_handle();
        window.focus(&focus, cx);
        Root {
            grid,
            views: vec![None; n],
            texts: vec![SharedString::default(); n],
            preedit: String::new(),
            logo: Arc::new(Image::from_bytes(ImageFormat::Png, look::LOGO_PNG.to_vec())),
            metrics: Rc::new(metrics),
            focus,
            a11y_text: false,
            probe_hit: Rc::new(Cell::new(false)),
        }
    }

    /// Keys that are not text; text arrives through the input handler.
    fn on_key(&mut self, ev: &KeyDownEvent, cx: &mut Context<Self>) {
        let Some(g) = &self.grid else {
            return;
        };
        let (code, text) = match ev.keystroke.key.as_str() {
            "enter" => ("Enter", Some("\r")),
            "backspace" => ("Backspace", Some("\x7f")),
            "tab" => ("Tab", Some("\t")),
            "escape" => ("Escape", Some("\x1b")),
            "left" => ("ArrowLeft", None),
            "right" => ("ArrowRight", None),
            "up" => ("ArrowUp", None),
            "down" => ("ArrowDown", None),
            _ => return,
        };
        g.key(0, code, 0, text);
        cx.stop_propagation();
    }

    fn pane(&self, i: usize, cx: &mut Context<Self>) -> AnyElement {
        let view = self.views[i].clone();
        let m = self.metrics.clone();
        let preedit = if i == 0 {
            self.preedit.clone()
        } else {
            String::new()
        };
        let entity = cx.entity();
        let focus = self.focus.clone();
        let mut d = div()
            .id(("pane", i))
            .role(Role::Terminal)
            .aria_label(format!("pane-{i}"))
            .flex_1()
            .h_full()
            .overflow_hidden();
        if self.a11y_text {
            d = d.aria_value(self.texts[i].clone());
        }
        if i == 0 {
            d = d
                .track_focus(&self.focus)
                .on_key_down(cx.listener(|this, ev: &KeyDownEvent, _, cx| this.on_key(ev, cx)));
        }
        d.child(
            canvas(
                |_, _, _| (),
                move |bounds, (), window, cx| {
                    if i == 0 {
                        window.handle_input(&focus, ElementInputHandler::new(bounds, entity), cx);
                    }
                    if let Some(v) = &view {
                        term::paint_pane(bounds, v, &m, &preedit, window, cx);
                    }
                },
            )
            .size_full(),
        )
        .into_any_element()
    }

    fn complete(&self) -> bool {
        self.views.iter().all(Option::is_some)
    }
}

impl Render for Root {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let Some(g) = self.grid.clone() else {
            return screen::main_screen(self.logo.clone());
        };
        for i in g.take_dirty() {
            if let Some(v) = g.view(i) {
                if v.probe_hit {
                    self.probe_hit.set(true);
                }
                self.views[i] = Some(Rc::new(v));
            }
        }
        if self.a11y_text {
            for (i, t) in self.texts.iter_mut().enumerate() {
                *t = g.screen_text(i).into();
            }
        }
        let panes = (0..self.views.len()).map(|i| self.pane(i, cx)).collect();
        screen::grid_screen(panes)
    }
}

impl EntityInputHandler for Root {
    fn text_for_range(
        &mut self,
        _: Range<usize>,
        _: &mut Option<Range<usize>>,
        _: &mut Window,
        _: &mut Context<Self>,
    ) -> Option<String> {
        None
    }

    fn selected_text_range(
        &mut self,
        _: bool,
        _: &mut Window,
        _: &mut Context<Self>,
    ) -> Option<UTF16Selection> {
        Some(UTF16Selection {
            range: 0..0,
            reversed: false,
        })
    }

    fn marked_text_range(&self, _: &mut Window, _: &mut Context<Self>) -> Option<Range<usize>> {
        (!self.preedit.is_empty()).then(|| 0..self.preedit.encode_utf16().count())
    }

    fn unmark_text(&mut self, _: &mut Window, cx: &mut Context<Self>) {
        self.preedit.clear();
        cx.notify();
    }

    fn replace_text_in_range(
        &mut self,
        _: Option<Range<usize>>,
        text: &str,
        _: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.preedit.clear();
        if let Some(g) = &self.grid {
            g.text(0, text);
        }
        cx.notify();
    }

    fn replace_and_mark_text_in_range(
        &mut self,
        _: Option<Range<usize>>,
        text: &str,
        _: Option<Range<usize>>,
        _: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.preedit = text.to_owned();
        cx.notify();
    }

    fn bounds_for_range(
        &mut self,
        _: Range<usize>,
        element: Bounds<Pixels>,
        _: &mut Window,
        _: &mut Context<Self>,
    ) -> Option<Bounds<Pixels>> {
        let v = self.views.first()?.as_ref()?;
        let cell = self.metrics.cell;
        Some(Bounds::new(
            point(
                element.origin.x + cell.width * f32::from(v.cursor_x),
                element.origin.y + cell.height * f32::from(v.cursor_y),
            ),
            cell,
        ))
    }

    fn character_index_for_point(
        &mut self,
        _: gpui::Point<Pixels>,
        _: &mut Window,
        _: &mut Context<Self>,
    ) -> Option<usize> {
        None
    }
}

/// The app in GPUI's headless window, for the benches and screenshots.
struct Headless {
    cx: HeadlessAppContext,
    win: WindowHandle<Root>,
    root: Entity<Root>,
    probe_hit: Rc<Cell<bool>>,
}

impl Headless {
    fn new(scale: f32, grid: Option<Arc<Grid>>) -> Result<Headless, String> {
        // The test window reads its scale when it opens (patch_zed.py).
        std::env::set_var("VORN_SPIKE_SCALE", scale.to_string());
        let mut cx = HeadlessAppContext::with_platform(text_system(), Arc::new(Icons), renderer);
        // Grayscale like vornui; the wgpu renderer drops subpixel glyphs on adapters without dual-source blending.
        let m = cx.update(|cx| {
            cx.set_text_rendering_mode(TextRenderingMode::Grayscale);
            metrics(cx.text_system(), scale)
        });
        let win = cx
            .open_window(
                size(px(look::WINDOW.0), px(look::WINDOW.1)),
                |window, cx| cx.new(|cx| Root::new(grid, m, window, cx)),
            )
            .map_err(|e| e.to_string())?;
        let root = win.root(&mut cx).map_err(|e| e.to_string())?;
        let probe_hit = cx.update(|cx| root.read(cx).probe_hit.clone());
        let mut h = Headless {
            cx,
            win,
            root,
            probe_hit,
        };
        // Images decode and the accessibility tree activates on the executor.
        h.draw();
        h.cx.run_until_parked();
        Ok(h)
    }

    fn any(&self) -> AnyWindowHandle {
        self.win.into()
    }

    fn draw(&mut self) {
        let root = self.root.clone();
        let _ = self.cx.update_window(self.any(), |_, w, cx| {
            root.update(cx, |_, cx| cx.notify());
            w.draw(cx).clear(cx);
            w.present_if_needed();
        });
    }

    fn keystroke(&mut self, k: &str) {
        let Ok(k) = Keystroke::parse(k) else {
            return;
        };
        let _ = self.cx.update_window(self.any(), |_, w, cx| {
            w.dispatch_keystroke(k, cx);
        });
    }

    fn save_png(&mut self, out: &str) -> Result<(), String> {
        let img = self
            .cx
            .capture_screenshot(self.any())
            .map_err(|e| e.to_string())?;
        if let Some(dir) = std::path::Path::new(out).parent() {
            let _ = std::fs::create_dir_all(dir);
        }
        img.save_with_format(out, image::ImageFormat::Png)
            .map_err(|e| e.to_string())
    }

    fn with_root<R>(&mut self, f: impl FnOnce(&mut Root) -> R) -> R {
        let root = self.root.clone();
        self.cx.update(|cx| root.update(cx, |r, _| f(r)))
    }
}

impl bench::Proto for Headless {
    fn key(&mut self, ch: char) {
        self.keystroke(&ch.to_string());
    }

    fn enter(&mut self) {
        self.keystroke("enter");
    }

    fn frame(&mut self) -> Frame {
        self.probe_hit.set(false);
        self.cx.run_until_parked();
        self.draw();
        let complete = self.with_root(|r| r.complete());
        Frame {
            drawn: true,
            probe_hit: self.probe_hit.get(),
            complete,
        }
    }
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
    let out = args.out("results/gpui-main.png");
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
    // Read back before the timed redraws, which queue without waiting; a
    // software adapter is still working through them otherwise.
    p.save_png(&out)?;
    // Redraws of the whole screen, every element rebuilt, nothing changed.
    let mut times = Vec::new();
    for _ in 0..120 {
        let t = Instant::now();
        p.draw();
        times.push(ms(t.elapsed()));
    }
    Ok(json!({
        "proto": PROTO,
        "screen": if grid_screen { "grid" } else { "main" },
        "panes": if grid_screen { panes } else { 0 },
        "scale": scale,
        "png": out,
        "adapter": RENDERER,
        "redraw_ms": bench::summary(&mut times),
    }))
}

fn run_bench(args: &Args) -> Result<Value, String> {
    let panes = args.num("panes", 8) as usize;
    let (_rig, grid) = rig(panes, Load::Busy, cell(look::SCALE))?;
    let mut p = Headless::new(look::SCALE, Some(grid.clone()))?;
    let mut v = bench::run(&grid, &mut p, &spike_shared::bench_config(args));
    v["adapter"] = json!(RENDERER);
    v["proto"] = json!(PROTO);
    Ok(v)
}

/// A fresh process to its first complete frame: renderer, fonts, grid, draw.
fn cold_child() -> Result<(), String> {
    let grid = spike_shared::cold_child_grid(cell(look::SCALE))?;
    let mut p = Headless::new(look::SCALE, Some(grid.clone()))?;
    bench::first_frame(&grid, &mut p, Duration::from_secs(20)).ok_or("no first frame")?;
    spike_shared::print_first_frame();
    Ok(())
}

/// Japanese input through GPUI's IME path: the platform input handler gets
/// marked text then a commit, as the OS text-input client calls it. The OS
/// IME itself is not driven (no window); its calls are.
fn ime(args: &Args) -> Result<Value, String> {
    let out = args.out("results/gpui-ime.json");
    let (_rig, grid) = rig(1, Load::Busy, cell(look::SCALE))?;
    let mut p = Headless::new(look::SCALE, Some(grid.clone()))?;
    let take = |p: &mut Headless| {
        let any = p.any();
        p.cx.update_window(any, |_, w, _| w.take_platform_input_handler())
            .ok()
            .flatten()
    };
    p.draw();
    let mut h = take(&mut p).ok_or("no input handler on the focused pane")?;
    h.replace_and_mark_text_in_range(None, "にほんご", None);
    p.draw();
    let png_pre = out.replace(".json", "-preedit.png");
    p.save_png(&png_pre)?;
    let preedit_drawn = p.with_root(|r| r.preedit == "にほんご");
    let mut h = take(&mut p).unwrap_or(h);
    h.replace_text_in_range(None, "日本語");
    let committed = spike_shared::wait_for_text(&grid, 0, "日本語", Duration::from_secs(5));
    std::thread::sleep(Duration::from_millis(100));
    p.draw();
    let png = out.replace(".json", ".png");
    p.save_png(&png)?;
    Ok(json!({
        "proto": PROTO,
        "path": "PlatformInputHandler marked text/commit -> EntityInputHandler -> grid text",
        "preedit_drawn": preedit_drawn,
        "commit_reached_pty": committed,
        "screen_line": grid.screen_text(0).lines().find(|l| l.contains("日本語")),
        "png_preedit": png_pre,
        "png_commit": png,
        "simulated": "the input handler is called as the OS IME client would; no OS IME window is driven",
    }))
}

/// GPUI's own dump of the last frame's AccessKit tree, with role counts.
fn tree_json(p: &mut Headless) -> Value {
    p.draw();
    p.cx.run_until_parked();
    p.draw();
    let any = p.any();
    let raw =
        p.cx.update_window(any, |_, w, _| w.debug_a11y_tree_json())
            .ok()
            .flatten()
            .unwrap_or_default();
    let tree: Value = serde_json::from_str(&raw).unwrap_or(Value::String(raw));
    let mut roles: HashMap<String, usize> = HashMap::new();
    count_roles(&tree, &mut roles);
    json!({
        "nodes": roles.values().sum::<usize>(),
        "roles": roles,
        "tree": tree,
    })
}

fn count_roles(v: &Value, roles: &mut HashMap<String, usize>) {
    match v {
        Value::Object(o) => {
            if let Some(Value::String(r)) = o.get("role") {
                *roles.entry(r.clone()).or_default() += 1;
            }
            o.values().for_each(|x| count_roles(x, roles));
        }
        Value::Array(a) => a.iter().for_each(|x| count_roles(x, roles)),
        _ => {}
    }
}

fn a11y() -> Result<Value, String> {
    let mut main = Headless::new(look::SCALE, None)?;
    let main_tree = tree_json(&mut main);
    let (_rig, grid) = rig(2, Load::Still, cell(look::SCALE))?;
    std::thread::sleep(Duration::from_millis(500));
    let mut g = Headless::new(look::SCALE, Some(grid))?;
    g.with_root(|r| r.a11y_text = true);
    let grid_tree = tree_json(&mut g);
    Ok(json!({ "proto": PROTO, "main": main_tree, "grid": grid_tree }))
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

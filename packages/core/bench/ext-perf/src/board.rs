//! The Tasks board drawn offscreen: the app's built-in board against the kit's board on the component's rows, loaded, scrolled and dragged.

use std::time::{Duration, Instant};

use serde_json::{json, Value};
use vorn_app::client::rpc::Event as RpcEvent;
use vorn_app::client::{Endpoint, Rpc};
use vorn_app::screen::tasks::{card_id, column_id};
use vorn_app::screen::MainView;
use vorn_app::ui::{Offscreen, RenderMode, Ui};
use vorn_app::{look, ui_config, App};
use vornui::{Id, Rgba};

use crate::kit::{self, Board};
use crate::stats::summary;
use crate::support::{self, TestVornd};
use crate::{sys, Variant};

/// The window: a laptop's default size on a Retina screen.
const SIZE: (f32, f32) = (1440.0, 900.0);
const SCALE: f32 = 2.0;
const FRAME: Duration = Duration::from_micros(16_667);

/// One input the scripts send.
#[derive(Clone, Copy)]
enum Step {
    None,
    Move(f32, f32),
    Down,
    Up,
    Wheel(f32),
}

/// What draws the board: the app, or the kit over the component's rows.
trait Surface {
    fn ui(&mut self) -> &mut Ui;
    /// Takes in what vornd sent and what the UI reported, then builds, lays out, paints and draws one frame.
    fn frame(&mut self);
    fn rect(&mut self, id: Id) -> Option<vornui::Rect> {
        self.ui().rect_of(id)
    }
}

fn apply(ui: &mut Ui, step: Step) {
    match step {
        Step::None => {}
        Step::Move(x, y) => ui.pointer_move(x, y),
        Step::Down => ui.pointer_down(),
        Step::Up => ui.pointer_up(),
        Step::Wheel(dy) => ui.wheel(dy),
    }
}

fn ms(d: Duration) -> f64 {
    d.as_secs_f64() * 1000.0
}

/// Runs `steps`, one frame each on a 60 Hz clock, and times every frame.
fn script(s: &mut dyn Surface, steps: &[Step]) -> Vec<f64> {
    let mut out = Vec::with_capacity(steps.len());
    let mut next = Instant::now();
    for &step in steps {
        let now = Instant::now();
        if next > now {
            std::thread::sleep(next - now);
        }
        next += FRAME;
        let t = Instant::now();
        apply(s.ui(), step);
        s.frame();
        out.push(ms(t.elapsed()));
    }
    out
}

/// Frame times of the idle, scroll and drag scripts.
type Frames = (Vec<f64>, Vec<f64>, Vec<f64>);

fn center(r: vornui::Rect) -> (f32, f32) {
    (r.x + r.w / 2.0, r.y + r.h / 2.0)
}

/// Scroll the first column down and back, then drag its first card to the next column and drop it.
fn scripts(s: &mut dyn Surface, scroll_id: Id, card: Id, target: Id) -> Result<Frames, String> {
    let mut idle = script(s, &[Step::None; 5]);
    // Frames past a tenth of a second get a shorter script, or a 10,000-card board takes an hour.
    let n = if crate::stats::pct(&idle, 50.0) > 100.0 {
        10
    } else {
        60
    };
    if n == 60 {
        idle.extend(script(s, &[Step::None; 25]));
    }
    let col = s
        .rect(scroll_id)
        .ok_or("the first column is not laid out")?;
    let (cx, cy) = center(col);
    let mut wheel = vec![Step::Move(cx, cy)];
    wheel.extend(std::iter::repeat_n(Step::Wheel(-120.0), n));
    wheel.extend(std::iter::repeat_n(Step::Wheel(120.0), n));
    let scroll = script(s, &wheel);
    // Back to the top, wherever the wheel left it, so the first card is there to drag.
    s.ui().set_scroll_offset(scroll_id, 0.0);
    script(s, &[Step::None; 10]);
    let from = center(s.rect(card).ok_or("the first card is not laid out")?);
    let to = center(s.rect(target).ok_or("the next column is not laid out")?);
    let mut drag = vec![Step::Move(from.0, from.1), Step::Down];
    for i in 1..=n {
        let t = i as f32 / n as f32;
        drag.push(Step::Move(
            from.0 + (to.0 - from.0) * t,
            from.1 + (to.1 - from.1) * t,
        ));
    }
    drag.push(Step::Up);
    drag.extend(std::iter::repeat_n(Step::None, n / 3));
    let drag = script(s, &drag);
    Ok((idle, scroll, drag))
}

struct AppSurface {
    app: App,
    ui: Ui,
    target: Offscreen,
}

impl Surface for AppSurface {
    fn ui(&mut self) -> &mut Ui {
        &mut self.ui
    }

    fn frame(&mut self) {
        self.app.events(&mut self.ui);
        self.app.frame(&mut self.ui, SIZE);
        if !self.ui.render_offscreen(&self.target) {
            self.app.frame(&mut self.ui, SIZE);
            self.ui.render_offscreen(&self.target);
        }
    }
}

struct KitSurface {
    board: Board,
    rpc: Rpc,
    events: std::sync::mpsc::Receiver<RpcEvent>,
    ui: Ui,
    target: Offscreen,
}

impl Surface for KitSurface {
    fn ui(&mut self) -> &mut Ui {
        &mut self.ui
    }

    fn frame(&mut self) {
        for e in self.ui.take_events() {
            self.board.event(&self.ui, &e);
        }
        for (method, args) in self.board.calls.drain(..) {
            drop(self.rpc.request(&method, args));
        }
        while let Ok(e) = self.events.try_recv() {
            if let RpcEvent::Notification { method, params } = e {
                if method == "ext:taskRows" && params["row"].is_object() {
                    self.board.upsert(params["row"].clone());
                }
            }
        }
        self.ui.begin(Rgba::hex(look::SURFACE_BASE));
        let root = self.board.view(&self.ui, SIZE);
        self.ui.layout(root, SIZE);
        self.ui.render_offscreen(&self.target);
    }
}

fn headless() -> (Ui, Offscreen) {
    let ui = Ui::headless(RenderMode::from_env(), &ui_config(SCALE));
    let target = ui.offscreen(((SIZE.0 * SCALE) as u32, (SIZE.1 * SCALE) as u32));
    (ui, target)
}

fn seed_config(rpc: &Rpc, home: &std::path::Path, tier: usize) -> Result<(), String> {
    let config = rpc
        .call("config:load", Value::Null)
        .map_err(|e| e.to_string())?;
    rpc.call("config:save", crate::tasks::bench_config(config, home)?)
        .map_err(|e| e.to_string())?;
    crate::tasks::insert(home, tier)
}

/// The first seeded task in the first column, the one the drag picks up.
const FIRST_TODO: &str = "t-000000";

fn footprint() -> f64 {
    sys::memory(std::process::id()).map_or(0.0, |m| m.0)
}

/// `case board <variant> <tier>`: `builtin`, `kit` (every card built) or `kit-window` (only the cards shown).
pub fn case(variant: Variant, tier: usize) -> Result<Value, String> {
    let kit = matches!(variant, Variant::Kit | Variant::KitWindow);
    if kit {
        Variant::Ext.apply("tasks");
    } else {
        Variant::Builtin.apply("tasks");
    }
    let v = TestVornd::start().ok_or("no vornd built for the bench")?;
    let before = footprint();
    let (load_ms, mem_loaded, (idle, scroll, drag), renderer) = if kit {
        let ep = Endpoint::find(&v.home).map_err(|e| e.to_string())?;
        let (rpc, events) =
            Rpc::connect(&ep, std::sync::Arc::new(|| {})).map_err(|e| e.to_string())?;
        seed_config(&rpc, &v.home, tier)?;
        let board = Board::new(include_str!("../board.json"), variant == Variant::KitWindow)?;
        let (ui, target) = headless();
        let mut s = KitSurface {
            board,
            rpc,
            events,
            ui,
            target,
        };
        s.frame();
        let before = footprint();
        let t = Instant::now();
        let rows = s
            .rpc
            .call("task:list", json!({ "projectName": "bench" }))
            .map_err(|e| e.to_string())?;
        s.board
            .set_rows(rows.as_array().cloned().unwrap_or_default());
        s.frame();
        let load = ms(t.elapsed());
        script(&mut s, &[Step::None; 10]);
        let loaded = footprint() - before;
        let runs = scripts(
            &mut s,
            Id::new("kit-column-scroll").child("todo"),
            kit::card_id(FIRST_TODO),
            Id::new("kit-column").child("in_progress"),
        )?;
        (load, loaded, runs, s.ui.renderer_name().to_owned())
    } else {
        let mut app = support::connect(&v);
        seed_config(app.rpc(), &v.home, tier)?;
        let config = app
            .rpc()
            .call("config:load", Value::Null)
            .map_err(|e| e.to_string())?;
        app.store.apply("config:changed", config);
        if app.store.tasks.len() != tier {
            return Err(format!(
                "seeded {tier} tasks, the app has {}",
                app.store.tasks.len()
            ));
        }
        app.st.project = Some("bench".into());
        app.st.tasks.mode = vorn_app::tasks::Mode::Board;
        let (ui, target) = headless();
        let mut s = AppSurface { app, ui, target };
        s.frame();
        let before = footprint();
        let t = Instant::now();
        let config = s
            .app
            .rpc()
            .call("config:load", Value::Null)
            .map_err(|e| e.to_string())?;
        s.app.store.apply("config:changed", config);
        s.app.st.view = MainView::Tasks;
        s.frame();
        let load = ms(t.elapsed());
        script(&mut s, &[Step::None; 10]);
        let loaded = footprint() - before;
        let runs = scripts(
            &mut s,
            Id::new("task-column-scroll").child("todo"),
            card_id(FIRST_TODO),
            column_id("in_progress"),
        )?;
        (load, loaded, runs, s.ui.renderer_name().to_owned())
    };
    let over = |f: &[f64]| f.iter().filter(|&&x| x > ms(FRAME)).count();
    Ok(json!({
        "kind": "board",
        "variant": variant.name(),
        "tier": tier,
        "renderer": renderer,
        "size": format!("{}x{}@{}", SIZE.0, SIZE.1, SCALE),
        "load_ms": load_ms,
        "idle_ms": summary(&idle),
        "scroll_ms": summary(&scroll),
        "drag_ms": summary(&drag),
        "frames": idle.len() + scroll.len() + drag.len(),
        "over_16_7_pct": 100.0 * (over(&idle) + over(&scroll) + over(&drag)) as f64 / (idle.len() + scroll.len() + drag.len()).max(1) as f64,
        "app_mb": { "before": before, "board": mem_loaded, "end": footprint() },
    }))
}

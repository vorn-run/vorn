//! Spike: Worktree cleanup drawn as declarative views (A) and as a vornui component (B); `ext-ui-host <vornd-data-dir> <out-dir> [--remove]`.

mod decl;
mod tree;
mod vornd;
mod wasm;

use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use serde_json::{json, Value};
use vornui::backend::{Backend as _, Cells};
use vornui::theme::color;
use vornui::{write_png, Id, Input, Laid, RenderMode, Ui, UiConfig};

const LOGICAL: (f32, f32) = (900.0, 560.0);
const SCALE: f32 = 2.0;
const GRID: (u16, u16) = (110, 34);

fn cfg(scale: f32) -> UiConfig {
    UiConfig::system(scale, 13.0)
}

fn rss_mb() -> f64 {
    let out = std::process::Command::new("ps")
        .args(["-o", "rss=", "-p", &std::process::id().to_string()])
        .output();
    out.ok()
        .and_then(|o| {
            String::from_utf8_lossy(&o.stdout)
                .trim()
                .parse::<f64>()
                .ok()
        })
        .unwrap_or(0.0)
        / 1024.0
}

fn stats(mut v: Vec<f64>) -> Value {
    if v.is_empty() {
        return Value::Null;
    }
    v.sort_by(|a, b| a.total_cmp(b));
    let p = |q: f64| v[((v.len() - 1) as f64 * q).round() as usize];
    json!({ "n": v.len(), "p50_ms": round(p(0.5)), "p95_ms": round(p(0.95)), "max_ms": round(p(1.0)) })
}

fn round(x: f64) -> f64 {
    (x * 1000.0).round() / 1000.0
}

/// One scripted step: inputs, then a frame, timed.
#[derive(Clone)]
enum Step {
    Move(f32, f32),
    Down,
    Up,
    Wheel(f32),
    Key(Input),
}

/// Where the script points, read from model A's layout (both models draw the same screen).
struct Spots {
    filter: (f32, f32),
    rows: Vec<(f32, f32)>,
    remove: (f32, f32),
    ok: (f32, f32),
}

fn center(ui: &Ui, id: Id) -> (f32, f32) {
    ui.rect_of(id)
        .map(|r| (r.x + r.w / 2.0, r.y + r.h / 2.0))
        .unwrap_or((0.0, 0.0))
}

/// A surface model A draws on: the client's own Ui.
struct AClient {
    ui: Ui,
    view: decl::View,
    target: Option<vornui::Offscreen>,
    cells: Option<Cells>,
    t0: Instant,
    laid: Option<Laid>,
}

impl AClient {
    fn new(mode: Option<RenderMode>, template: &str) -> AClient {
        let (ui, cells) = match mode {
            Some(m) => (Ui::headless(m, &cfg(SCALE)), None),
            None => (Ui::cells(&cfg(1.0)), Some(Cells::offscreen(GRID.0, GRID.1))),
        };
        AClient {
            ui,
            view: decl::View::new(template),
            target: None,
            cells,
            t0: Instant::now(),
            laid: None,
        }
    }

    fn size(&self) -> (f32, f32) {
        if self.cells.is_some() {
            vornui::cells::units::px(GRID.0, GRID.1)
        } else {
            LOGICAL
        }
    }

    fn apply(&mut self, s: &Step) {
        match s {
            Step::Move(x, y) => self.ui.pointer_move(*x, *y),
            Step::Down => self.ui.pointer_down(),
            Step::Up => self.ui.pointer_up(),
            Step::Wheel(dy) => self.ui.wheel(*dy),
            Step::Key(k) => {
                self.ui.key(k);
            }
        }
        for e in self.ui.take_events() {
            self.view.event(&self.ui, e);
        }
    }

    fn frame(&mut self) {
        self.ui
            .begin_at(color::SURFACE_BASE, self.t0 + Duration::from_secs(3600));
        let root = self.view.view(&self.ui);
        let size = self.size();
        self.laid = Some(self.ui.layout(root, size));
        if let Some(c) = &mut self.cells {
            c.present(&mut self.ui).expect("cells");
        } else {
            let px = ((LOGICAL.0 * SCALE) as u32, (LOGICAL.1 * SCALE) as u32);
            if self.target.is_none() {
                self.target = Some(self.ui.offscreen(px));
            }
            self.ui.render_offscreen(self.target.as_ref().unwrap());
        }
    }

    fn shoot(&mut self, path: &str) {
        if let Some(c) = &self.cells {
            std::fs::write(path, c.buffer.to_html("model A on Cells")).expect("html");
        } else {
            let t = self.target.as_ref().unwrap();
            let px = self.ui.read(t).expect("read");
            write_png(path, t.size, &px).expect("png");
        }
    }

    fn spots(&self) -> Spots {
        let list = Id::new("worktrees");
        Spots {
            filter: center(&self.ui, Id::new("filter")),
            rows: (0..self.view.rows.len())
                .map(|i| center(&self.ui, list.child(i)))
                .collect(),
            remove: center(&self.ui, Id::new("remove")),
            ok: center(&self.ui, Id::new("remove.ok")),
        }
    }
}

fn to_input(s: &Step) -> wasm::Input {
    match s {
        Step::Move(x, y) => wasm::Input::PointerMove((*x, *y)),
        Step::Down => wasm::Input::PointerDown,
        Step::Up => wasm::Input::PointerUp,
        Step::Wheel(dy) => wasm::Input::Wheel(*dy),
        Step::Key(Input::Key { code, mods, text }) => wasm::Input::Key(wasm::KeyPress {
            code: code.clone(),
            text: text.clone(),
            mods: *mods,
        }),
        Step::Key(_) => wasm::Input::PointerUp,
    }
}

fn b_shoot(f: &wasm::Frame, path: &str) {
    match f {
        wasm::Frame::Pixels(px) => {
            let size = ((LOGICAL.0 * SCALE) as u32, (LOGICAL.1 * SCALE) as u32);
            write_png(path, size, px).expect("png");
        }
        wasm::Frame::Cells(cells) => {
            let mut buf = vornui::cells::Buffer::new(GRID.0, GRID.1);
            let rgb = |c: u32| [(c >> 16) as u8, (c >> 8) as u8, c as u8];
            for (i, c) in cells.iter().enumerate() {
                buf.cells[i] = vornui::cells::Cell {
                    sym: vornui::cells::Sym::new(&c.sym),
                    fg: rgb(c.fg),
                    bg: rgb(c.bg),
                    attr: c.attr,
                    icon: 0,
                };
            }
            std::fs::write(path, buf.to_html("model B on Cells")).expect("html");
        }
    }
}

/// The script both models run: type and clear a filter, click a row, drag over rows, scroll.
fn script(s: &Spots, cells: bool) -> Vec<(&'static str, Vec<Step>)> {
    let mut out = Vec::new();
    let click = |p: (f32, f32)| vec![Step::Move(p.0, p.1), Step::Down, Step::Up];
    out.push(("focus", click(s.filter)));
    for ch in "feat".chars() {
        out.push(("type", vec![Step::Key(Input::char(ch))]));
    }
    for _ in 0..4 {
        out.push(("type", vec![Step::Key(Input::named("Backspace"))]));
    }
    let (a, b) = (s.rows[1], s.rows[s.rows.len().min(9) - 1]);
    out.push(("click", click(a)));
    out.push(("drag", vec![Step::Move(a.0, a.1), Step::Down]));
    let steps = 24;
    for i in 1..=steps {
        let t = i as f32 / steps as f32;
        out.push(("drag", vec![Step::Move(a.0, a.1 + (b.1 - a.1) * t)]));
    }
    out.push(("drag", vec![Step::Up]));
    for _ in 0..30 {
        out.push((
            "scroll",
            vec![
                Step::Move(a.0, a.1),
                Step::Wheel(if cells { 1.0 } else { 24.0 }),
            ],
        ));
    }
    for _ in 0..30 {
        out.push((
            "scroll",
            vec![Step::Wheel(if cells { -1.0 } else { -24.0 })],
        ));
    }
    out
}

fn timings(log: &[(&str, f64)]) -> Value {
    let mut by = serde_json::Map::new();
    for k in ["type", "drag", "scroll", "click", "confirm"] {
        by.insert(
            k.into(),
            stats(
                log.iter()
                    .filter(|(n, _)| *n == k)
                    .map(|(_, t)| *t)
                    .collect(),
            ),
        );
    }
    Value::Object(by)
}

/// Inventory rows repeated to `n`, for the scrolling and update runs.
fn many(rows: &str, n: usize) -> Vec<Value> {
    let base: Vec<Value> = serde_json::from_str(rows).unwrap_or_default();
    (0..n)
        .map(|i| {
            let mut r = base[i % base.len()].clone();
            r["id"] = json!(format!("{}#{i}", r["id"].as_str().unwrap_or("")));
            r["name"] = json!(format!("{}-{i}", r["name"].as_str().unwrap_or("")));
            r
        })
        .collect()
}

fn main() {
    let args: Vec<String> = std::env::args().collect();
    let (data, out) = (&args[1], &args[2]);
    let remove = args.iter().any(|a| a == "--remove");
    std::fs::create_dir_all(out).expect("out");
    let dir = std::env::current_exe()
        .unwrap()
        .parent()
        .unwrap()
        .join("../wasm32-wasip2/release");
    let (backend_wasm, ui_wasm) = (dir.join("ext_ui_backend.wasm"), dir.join("ext_ui_b.wasm"));
    let uic_wasm = dir.join("ext_ui_c.wasm");
    let mut report = serde_json::Map::new();
    report.insert("sizes".into(), json!({
        "backend_wasm_kb": std::fs::metadata(&backend_wasm).map(|m| m.len() / 1024).unwrap_or(0),
        "ui_b_wasm_kb": std::fs::metadata(&ui_wasm).map(|m| m.len() / 1024).unwrap_or(0),
        "ui_c_wasm_kb": std::fs::metadata(&uic_wasm).map(|m| m.len() / 1024).unwrap_or(0),
    }));

    let vornd = Arc::new(vornd::Vornd::open(data).expect("test vornd"));
    let inventory = || {
        vornd
            .call("worktree:inventory", json!({ "refresh": true }))
            .expect("inventory")
            .to_string()
    };

    let rss0 = rss_mb();
    let engine = wasm::engine();
    let (backend_c, backend_compile) = wasm::compile(&engine, backend_wasm.to_str().unwrap());
    let (ui_c, ui_compile) = wasm::compile(&engine, ui_wasm.to_str().unwrap());
    let (uic_c, uic_compile) = wasm::compile(&engine, uic_wasm.to_str().unwrap());
    let t = Instant::now();
    let backend = Arc::new(Mutex::new(wasm::Backend::new(&engine, &backend_c)));
    let backend_init = t.elapsed().as_secs_f64() * 1e3;
    let template = backend.lock().unwrap().view();
    let rows_json = backend.lock().unwrap().rows(&inventory());
    report.insert(
        "startup".into(),
        json!({
            "compile_ms": { "backend": round(backend_compile), "ui_b": round(ui_compile), "ui_c": round(uic_compile) },
            "backend_instantiate_ms": round(backend_init),
            "template_bytes": template.len(),
            "rows_bytes": rows_json.len(),
            "rows": serde_json::from_str::<Vec<Value>>(&rows_json).map(|r| r.len()).unwrap_or(0),
        }),
    );

    // Model A on Pixels (GPU, as Vorn Native draws), Pixels on the CPU raster, and Cells.
    let mut spots_px = None;
    let mut spots_cells = None;
    let mut a_access = None;
    for (name, mode) in [
        ("a_pixels_gpu", Some(RenderMode::Auto)),
        ("a_pixels_cpu", Some(RenderMode::Cpu)),
        ("a_cells", None),
    ] {
        let t = Instant::now();
        let mut a = AClient::new(mode, &template);
        a.view.set_rows(serde_json::from_str(&rows_json).unwrap());
        a.frame();
        a.frame();
        let first = t.elapsed().as_secs_f64() * 1e3;
        if name == "a_pixels_gpu" {
            a_access = a.laid.as_ref().map(access);
        }
        let spots = a.spots();
        let row1 = spots.rows[1];
        let ext = if mode.is_none() { "html" } else { "png" };
        a.shoot(&format!("{out}/{name}-list.{ext}"));
        let mut log = Vec::new();
        for (label, steps) in script(&spots, mode.is_none()) {
            let t = Instant::now();
            for s in &steps {
                a.apply(s);
            }
            a.frame();
            log.push((label, t.elapsed().as_secs_f64() * 1e3));
            if label == "type" && log.iter().filter(|(l, _)| *l == "type").count() == 4 {
                a.shoot(&format!("{out}/{name}-filter.{ext}"));
            }
        }
        let rm = a.spots().remove;
        let t = Instant::now();
        for st in [Step::Move(rm.0, rm.1), Step::Down, Step::Up] {
            a.apply(&st);
        }
        a.frame();
        log.push(("confirm", t.elapsed().as_secs_f64() * 1e3));
        a.shoot(&format!("{out}/{name}-confirm.{ext}"));
        let mut spots = spots;
        spots.remove = rm;
        let ok = a.spots().ok;
        if mode.is_none() {
            spots_cells = Some((spots, ok))
        } else if spots_px.is_none() {
            spots_px = Some((spots, ok))
        }
        // Update: one row changes. A sends that row only.
        let mut rows: Vec<Value> = serde_json::from_str(&rows_json).unwrap();
        let mut up = Vec::new();
        for n in [rows.len(), 500] {
            if n == 500 {
                rows = many(&rows_json, 500);
                a.view.set_rows(rows.clone());
                a.frame();
            }
            let mut changed = rows[n / 2].clone();
            changed["status"] = json!("merged");
            let patch = json!({ "op": "upsert", "row": changed }).to_string();
            let mut ts = Vec::new();
            for _ in 0..20 {
                let t = Instant::now();
                let p: Value = serde_json::from_str(&patch).unwrap();
                a.view.upsert(p["row"].clone());
                a.frame();
                ts.push(t.elapsed().as_secs_f64() * 1e3);
            }
            up.push(json!({ "rows": n, "bytes_sent": patch.len(), "apply_and_frame": stats(ts) }));
        }
        // Scrolling 500 rows: the list holds every row, as the spike does not virtualize.
        a.apply(&Step::Move(row1.0, row1.1));
        let mut sc = Vec::new();
        for i in 0..40 {
            let dy = if mode.is_none() { 1.0 } else { 24.0 } * if i < 20 { 1.0 } else { -1.0 };
            let t = Instant::now();
            a.apply(&Step::Wheel(dy));
            a.frame();
            sc.push(t.elapsed().as_secs_f64() * 1e3);
        }
        report.insert(name.into(), json!({ "first_frame_ms": round(first), "interaction": timings(&log), "update_one_row": up, "scroll_500_rows": stats(sc) }));
    }

    // Model A acts: the confirm step's OK calls the backend, then vornd.
    if remove {
        let (s, _) = spots_px.as_ref().unwrap();
        let mut a = AClient::new(Some(RenderMode::Auto), &template);
        a.view.set_rows(serde_json::from_str(&rows_json).unwrap());
        a.frame();
        for p in [s.rows[1], s.rows[2]] {
            for st in [Step::Move(p.0, p.1), Step::Down, Step::Up] {
                a.apply(&st);
            }
            a.frame();
        }
        let rm = a.spots().remove;
        for st in [Step::Move(rm.0, rm.1), Step::Down, Step::Up] {
            a.apply(&st);
        }
        a.frame();
        let ok = a.spots().ok;
        for st in [Step::Move(ok.0, ok.1), Step::Down, Step::Up] {
            a.apply(&st);
        }
        for (m, args) in std::mem::take(&mut a.view.calls) {
            let ids: Vec<String> = serde_json::from_value(args).unwrap_or_default();
            let plan = backend.lock().unwrap().plan(&ids);
            a.view.status = match plan
                .and_then(|p| vornd.call("worktree:removeMany", serde_json::from_str(&p).unwrap()))
            {
                Ok(r) => format!("{m}: {}", summary(&r)),
                Err(e) => format!("{m}: {e}"),
            };
        }
        let fresh = backend.lock().unwrap().rows(&inventory());
        a.view.set_rows(serde_json::from_str(&fresh).unwrap());
        a.frame();
        a.shoot(&format!("{out}/a_pixels_gpu-removed.png"));
    }

    // Model B: the UI component, once per surface.
    let rows_json = backend.lock().unwrap().rows(&inventory());
    for (name, cells) in [("b_pixels_cpu", false), ("b_cells", true)] {
        let (spots, ok) = if cells {
            spots_cells.as_ref().unwrap()
        } else {
            spots_px.as_ref().unwrap()
        };
        let t = Instant::now();
        let mut b = wasm::UiB::new(&engine, &ui_c, cells, if cells { 1.0 } else { SCALE });
        let init = t.elapsed().as_secs_f64() * 1e3;
        {
            let (be, vd) = (backend.clone(), vornd.clone());
            b.store.data_mut().answer = Box::new(move |m, args| {
                let ids: Vec<String> = serde_json::from_str(args).map_err(|e| e.to_string())?;
                let plan = be.lock().unwrap().plan(&ids)?;
                let r = vd.call("worktree:removeMany", serde_json::from_str(&plan).unwrap())?;
                Ok(format!("{m}: {}", summary(&r)))
            });
        }
        let (w, h) = if cells {
            (u32::from(GRID.0), u32::from(GRID.1))
        } else {
            ((LOGICAL.0 * SCALE) as u32, (LOGICAL.1 * SCALE) as u32)
        };
        let t = Instant::now();
        b.data(&rows_json);
        b.frame(w, h, &[]);
        let f = b.frame(w, h, &[]);
        let first = t.elapsed().as_secs_f64() * 1e3;
        let ext = if cells { "html" } else { "png" };
        b_shoot(&f, &format!("{out}/{name}-list.{ext}"));
        let mut log = Vec::new();
        let mut last: wasm::Frame;
        let mut typed = 0;
        for (label, steps) in script(spots, cells) {
            let ins: Vec<wasm::Input> = steps.iter().map(to_input).collect();
            let t = Instant::now();
            last = b.frame(w, h, &ins);
            log.push((label, t.elapsed().as_secs_f64() * 1e3));
            if label == "type" {
                typed += 1;
                if typed == 4 {
                    b_shoot(&last, &format!("{out}/{name}-filter.{ext}"));
                }
            }
        }
        let t = Instant::now();
        last = b.frame(
            w,
            h,
            &[
                wasm::Input::PointerMove(spots.remove),
                wasm::Input::PointerDown,
                wasm::Input::PointerUp,
            ],
        );
        log.push(("confirm", t.elapsed().as_secs_f64() * 1e3));
        b_shoot(&last, &format!("{out}/{name}-confirm.{ext}"));
        // A client must also upload what B drew before it can show it.
        let upload = if let wasm::Frame::Pixels(px) = &last {
            let mut host = Ui::headless(RenderMode::Auto, &cfg(SCALE));
            let mut ts = Vec::new();
            for _ in 0..20 {
                let t = Instant::now();
                host.begin_at(color::SURFACE_BASE, Instant::now());
                let id = host.image(w, h, px);
                let el = vornui::div().w_full().h_full().child(vornui::image(
                    id.unwrap_or(0),
                    LOGICAL.0,
                    LOGICAL.1,
                ));
                host.layout(el, LOGICAL);
                let tgt = host.offscreen((w, h));
                host.render_offscreen(&tgt);
                ts.push(t.elapsed().as_secs_f64() * 1e3);
            }
            stats(ts)
        } else {
            Value::Null
        };
        // Update: B's author pushes the rows; the spike sends them whole.
        let mut up = Vec::new();
        for n in [0usize, 500] {
            let rows: Vec<Value> = if n == 0 {
                serde_json::from_str(&rows_json).unwrap()
            } else {
                many(&rows_json, 500)
            };
            let mut rows = rows;
            let mid = rows.len() / 2;
            rows[mid]["status"] = json!("merged");
            let body = Value::Array(rows.clone()).to_string();
            b.data(&body);
            b.frame(w, h, &[]);
            let mut ts = Vec::new();
            for _ in 0..20 {
                let t = Instant::now();
                b.data(&body);
                b.frame(w, h, &[]);
                ts.push(t.elapsed().as_secs_f64() * 1e3);
            }
            up.push(json!({ "rows": rows.len(), "bytes_sent": body.len(), "apply_and_frame": stats(ts) }));
        }
        let p = spots.rows[1];
        b.frame(w, h, &[wasm::Input::PointerMove(p)]);
        let mut sc = Vec::new();
        for i in 0..40 {
            let dy = if cells { 1.0 } else { 24.0 } * if i < 20 { 1.0 } else { -1.0 };
            let t = Instant::now();
            b.frame(w, h, &[wasm::Input::Wheel(dy)]);
            sc.push(t.elapsed().as_secs_f64() * 1e3);
        }
        let scroll500 = stats(sc);
        if remove && !cells {
            b.data(&rows_json);
            b.frame(w, h, &[]);
            let picks = [spots.rows[3], spots.rows[4]];
            for p in picks {
                b.frame(
                    w,
                    h,
                    &[
                        wasm::Input::PointerMove(p),
                        wasm::Input::PointerDown,
                        wasm::Input::PointerUp,
                    ],
                );
            }
            b.frame(
                w,
                h,
                &[
                    wasm::Input::PointerMove(spots.remove),
                    wasm::Input::PointerDown,
                    wasm::Input::PointerUp,
                ],
            );
            b.frame(
                w,
                h,
                &[
                    wasm::Input::PointerMove(*ok),
                    wasm::Input::PointerDown,
                    wasm::Input::PointerUp,
                ],
            );
            let fresh = backend.lock().unwrap().rows(&inventory());
            b.data(&fresh);
            let f = b.frame(w, h, &[]);
            b_shoot(&f, &format!("{out}/{name}-removed.png"));
        }
        report.insert(
            name.into(),
            json!({
                "instantiate_and_init_ms": round(init),
                "first_frame_ms": round(first),
                "interaction": timings(&log),
                "host_upload_and_composite": upload,
                "update_whole_rows": up,
                "scroll_500_rows": scroll500,
                "peak_linear_memory_mb": round(b.store.data().peak as f64 / 1048576.0),
            }),
        );
    }

    // B's guest side when the client draws its display list: lay out, stop before raster.
    {
        let (spots, _) = spots_px.as_ref().unwrap();
        let (w, h) = ((LOGICAL.0 * SCALE) as u32, (LOGICAL.1 * SCALE) as u32);
        let t = Instant::now();
        let mut b = wasm::UiB::with(&engine, &ui_c, false, SCALE, true);
        let init = t.elapsed().as_secs_f64() * 1e3;
        let after_init = b.store.data().peak;
        b.data(&rows_json);
        b.frame(w, h, &[]);
        let after_first = b.store.data().peak;
        let mut log = Vec::new();
        for (label, steps) in script(spots, false) {
            let ins: Vec<wasm::Input> = steps.iter().map(to_input).collect();
            let t = Instant::now();
            b.frame(w, h, &ins);
            log.push((label, t.elapsed().as_secs_f64() * 1e3));
        }
        b.data(&Value::Array(many(&rows_json, 500)).to_string());
        b.frame(w, h, &[]);
        b.frame(w, h, &[wasm::Input::PointerMove(spots.rows[1])]);
        let mut sc = Vec::new();
        for i in 0..40 {
            let t = Instant::now();
            b.frame(
                w,
                h,
                &[wasm::Input::Wheel(if i < 20 { 24.0 } else { -24.0 })],
            );
            sc.push(t.elapsed().as_secs_f64() * 1e3);
        }
        report.insert(
            "b_pixels_layout_only".into(),
            json!({
                "instantiate_and_init_ms": round(init),
                "interaction": timings(&log),
                "scroll_500_rows": stats(sc),
                "peak_linear_memory_mb": round(b.store.data().peak as f64 / 1048576.0),
                "peak_after_init_mb": round(after_init as f64 / 1048576.0),
                "peak_after_first_frame_mb": round(after_first as f64 / 1048576.0),
            }),
        );
    }

    // Model C: the component returns trees; each client lays them out and draws them itself.
    let mut c_access = None;
    for (name, mode) in [
        ("c_pixels_gpu", Some(RenderMode::Auto)),
        ("c_pixels_cpu", Some(RenderMode::Cpu)),
        ("c_cells", None),
    ] {
        let cells = mode.is_none();
        let (spots, ok) = if cells {
            spots_cells.as_ref().unwrap()
        } else {
            spots_px.as_ref().unwrap()
        };
        let t = Instant::now();
        let comp = wasm::UiC::new(&engine, &uic_c);
        let init = t.elapsed().as_secs_f64() * 1e3;
        // From the client's construction, as A's first frame is.
        let t = Instant::now();
        let mut c = tree::CClient::new(mode, comp, &cfg(if cells { 1.0 } else { SCALE }));
        {
            let (be, vd) = (backend.clone(), vornd.clone());
            c.comp.store.data_mut().answer = Box::new(move |m, args| {
                let ids: Vec<String> = serde_json::from_str(args).map_err(|e| e.to_string())?;
                let plan = be.lock().unwrap().plan(&ids)?;
                let r = vd.call("worktree:removeMany", serde_json::from_str(&plan).unwrap())?;
                Ok(format!("{m}: {}", summary(&r)))
            });
        }
        c.data(&rows_json);
        c.frame();
        c.frame();
        let first = t.elapsed().as_secs_f64() * 1e3;
        let tree13 = (c.nodes.len(), tree::tree_bytes(&c.nodes));
        if name == "c_pixels_gpu" {
            c_access = c.laid.as_ref().map(access);
        }
        let ext = if cells { "html" } else { "png" };
        c.shoot(&format!("{out}/{name}-list.{ext}"));
        let mut log = Vec::new();
        for (label, steps) in script(spots, cells) {
            let t = Instant::now();
            for s in &steps {
                c.apply(s);
            }
            c.frame();
            log.push((label, t.elapsed().as_secs_f64() * 1e3));
            if label == "type" && log.iter().filter(|(l, _)| *l == "type").count() == 4 {
                c.shoot(&format!("{out}/{name}-filter.{ext}"));
            }
        }
        c.shoot(&format!("{out}/{name}-selected.{ext}"));
        let t = Instant::now();
        for st in [Step::Move(spots.remove.0, spots.remove.1), Step::Down, Step::Up] {
            c.apply(&st);
        }
        c.frame();
        log.push(("confirm", t.elapsed().as_secs_f64() * 1e3));
        c.shoot(&format!("{out}/{name}-confirm.{ext}"));
        let renders_script = c.renders.clone();
        // Update: one row changes. The client hands the component the same op model A applies.
        let mut up = Vec::new();
        let mut rows: Vec<Value> = serde_json::from_str(&rows_json).unwrap();
        let mut tree500 = (0, 0);
        for n in [rows.len(), 500] {
            if n == 500 {
                rows = many(&rows_json, 500);
                c.data(&Value::Array(rows.clone()).to_string());
                c.frame();
                tree500 = (c.nodes.len(), tree::tree_bytes(&c.nodes));
            }
            let mut changed = rows[n / 2].clone();
            changed["status"] = json!("merged");
            let patch = json!({ "op": "upsert", "row": changed }).to_string();
            let mut ts = Vec::new();
            let r0 = c.renders.len();
            for _ in 0..20 {
                let t = Instant::now();
                c.patch(&patch);
                c.frame();
                ts.push(t.elapsed().as_secs_f64() * 1e3);
            }
            up.push(json!({
                "rows": n,
                "bytes_sent": patch.len(),
                "apply_and_frame": stats(ts),
                "component_render": stats(c.renders[r0..].to_vec()),
                "tree_nodes": c.nodes.len(),
                "tree_bytes": tree::tree_bytes(&c.nodes),
            }));
        }
        c.apply(&Step::Move(spots.rows[1].0, spots.rows[1].1));
        let mut sc = Vec::new();
        let r0 = c.renders.len();
        for i in 0..40 {
            let dy = if cells { 1.0 } else { 24.0 } * if i < 20 { 1.0 } else { -1.0 };
            let t = Instant::now();
            c.apply(&Step::Wheel(dy));
            c.frame();
            sc.push(t.elapsed().as_secs_f64() * 1e3);
        }
        let scroll_renders = c.renders.len() - r0;
        if remove && name == "c_pixels_gpu" {
            c.data(&rows_json);
            c.frame();
            for p in [spots.rows[5], spots.rows[6], spots.remove, *ok] {
                for st in [Step::Move(p.0, p.1), Step::Down, Step::Up] {
                    c.apply(&st);
                }
                c.frame();
            }
            let fresh = backend.lock().unwrap().rows(&inventory());
            c.data(&fresh);
            c.frame();
            c.shoot(&format!("{out}/{name}-removed.png"));
        }
        report.insert(
            name.into(),
            json!({
                "instantiate_ms": round(init),
                "first_frame_ms": round(first),
                "interaction": timings(&log),
                "component_render_during_script": stats(renders_script),
                "update_one_row": up,
                "scroll_500_rows": stats(sc),
                "scroll_component_renders": scroll_renders,
                "tree_13_rows": { "nodes": tree13.0, "bytes": tree13.1 },
                "tree_500_rows": { "nodes": tree500.0, "bytes": tree500.1 },
                "peak_linear_memory_mb": round(c.comp.store.data().peak as f64 / 1048576.0),
            }),
        );
    }
    report.insert(
        "accessibility".into(),
        json!({ "a": a_access, "c": c_access }),
    );

    // Many extensions open: twenty B instances on Pixels, against twenty A templates.
    let before = rss_mb();
    let t = Instant::now();
    let mut held = Vec::new();
    let mut peak = 0usize;
    for _ in 0..20 {
        let mut b = wasm::UiB::new(&engine, &ui_c, false, SCALE);
        b.data(&rows_json);
        b.frame((LOGICAL.0 * SCALE) as u32, (LOGICAL.1 * SCALE) as u32, &[]);
        peak += b.store.data().peak;
        held.push(b);
    }
    let b20 = t.elapsed().as_secs_f64() * 1e3;
    let after = rss_mb();
    drop(held);
    let before_c = rss_mb();
    let t = Instant::now();
    let mut held_c = Vec::new();
    let mut peak_c = 0usize;
    for _ in 0..20 {
        let mut c = wasm::UiC::new(&engine, &uic_c);
        c.data(&rows_json);
        let nodes = c.render(wasm::tree::Surface { cells: false, width: LOGICAL.0, height: LOGICAL.1 });
        peak_c += c.store.data().peak;
        held_c.push((c, nodes));
    }
    let c20 = t.elapsed().as_secs_f64() * 1e3;
    let after_c = rss_mb();
    let t = Instant::now();
    let views: Vec<decl::View> = (0..20).map(|_| decl::View::new(&template)).collect();
    let a20 = t.elapsed().as_secs_f64() * 1e3;
    report.insert(
        "twenty_open".into(),
        json!({
            "b_init_and_first_frame_ms": round(b20),
            "b_linear_memory_mb": round(peak as f64 / 1048576.0),
            "b_process_rss_growth_mb": round(after - before),
            "c_init_and_first_tree_ms": round(c20),
            "c_linear_memory_mb": round(peak_c as f64 / 1048576.0),
            "c_process_rss_growth_mb": round(after_c - before_c),
            "a_templates_parse_ms": round(a20),
            "a_template_bytes_each": template.len(),
            "process_rss_start_mb": round(rss0),
        }),
    );
    drop(views);
    drop(held_c);
    println!(
        "{}",
        serde_json::to_string_pretty(&Value::Object(report)).unwrap()
    );
}

fn summary(r: &Value) -> String {
    let n = |k: &str| r[k].as_array().map_or(0, Vec::len);
    format!("removed {}, failed {}", n("succeeded"), n("failed"))
}

/// What a screen reader gets from one frame: nodes by role, and every named node.
fn access(l: &Laid) -> Value {
    let mut roles = std::collections::BTreeMap::<String, usize>::new();
    let mut named = Vec::new();
    for (_, n) in &l.tree.nodes {
        let role = format!("{:?}", n.role());
        *roles.entry(role.clone()).or_default() += 1;
        if let Some(label) = n.label() {
            named.push(format!("{role}: {label}"));
        }
    }
    named.sort();
    json!({ "roles": roles, "named": named })
}

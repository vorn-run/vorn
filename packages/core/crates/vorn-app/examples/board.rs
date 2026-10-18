//! The Tasks board at scale: the app's frame time and memory with 100, 1,000
//! and 10,000 tasks on the board, against a test vornd.
//!
//! ```text
//! board [--tiers 100,1000,10000] [--reps 3] [--surface window|cells] [--out results.json]
//! board case TIER SURFACE   (one measurement as JSON; the run starts these)
//! ```
//!
//! Each measurement is its own process with its own vornd on a temporary HOME
//! and data directory (never `~/.vorn`), so memory is the board's alone. The
//! tasks are written straight into vornd's database and the app loads the
//! configuration once, as it does on start.
//!
//! The app draws offscreen at 1440x900 on a 2x screen (the GPU when there is
//! one, else the CPU raster; `VORNUI_RENDERER` picks), or into a 180x32 cell
//! grid with `--surface cells`, on a 60 Hz clock. Scripts: **idle** frames;
//! **scroll**, the wheel down the To do column and back; **drag**, its first
//! card carried to In progress and dropped. Frame time is one frame's build,
//! layout, paint and draw. **Footprint** is the app's physical footprint
//! (resident size off macOS) after the board has loaded and at the end; the
//! **board** share is the growth from loading it.

#[cfg(unix)]
#[path = "../tests/support/mod.rs"]
mod support;

#[cfg(not(unix))]
fn main() {
    eprintln!("board: the bench runs on macOS and Linux");
    std::process::exit(1);
}

#[cfg(unix)]
fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let result = match args.first().map(String::as_str) {
        Some("case") => bench::case(&args[1..]).map(|v| println!("{v}")),
        _ => bench::run(&args),
    };
    if let Err(e) = result {
        eprintln!("board: {e}");
        std::process::exit(1);
    }
}

#[cfg(unix)]
mod bench {
    use std::fmt::Write as _;
    use std::time::{Duration, Instant};

    use serde_json::{json, Value};
    use vorn_app::screen::tasks::{card_id, column_id};
    use vorn_app::screen::MainView;
    use vorn_app::ui::backend::{Backend, Cells};
    use vorn_app::ui::{Id, Offscreen, Rect, RenderMode, Ui};
    use vorn_app::{ui_config, App};

    use crate::support::{self, TestVornd};

    const SIZE: (f32, f32) = (1440.0, 900.0);
    const SCALE: f32 = 2.0;
    /// The cell grid of `--surface cells`.
    const GRID: (u16, u16) = (180, 32);
    const FRAME: Duration = Duration::from_micros(16_667);
    /// The first seeded task in To do: the card the drag picks up.
    const FIRST_TODO: &str = "t-000000";

    /// The statuses seeded tasks hold, about as a busy board holds them.
    const MIX: [&str; 20] = [
        "todo",
        "todo",
        "todo",
        "todo",
        "todo",
        "todo",
        "todo",
        "todo",
        "todo",
        "in_progress",
        "in_progress",
        "in_progress",
        "in_progress",
        "in_review",
        "in_review",
        "done",
        "done",
        "done",
        "done",
        "cancelled",
    ];
    const WORDS: [&str; 8] = [
        "parser", "session", "sidebar", "gate", "schedule", "palette", "grid", "worktree",
    ];

    fn flag<'a>(args: &'a [String], key: &str) -> Option<&'a str> {
        args.windows(2)
            .find(|w| w[0].strip_prefix("--") == Some(key))
            .map(|w| w[1].as_str())
    }

    /// One seeded task; every seventh title wraps to a second line.
    fn task(i: usize) -> Value {
        let status = MIX[i % MIX.len()];
        let word = WORDS[i % WORDS.len()];
        let title = if i % 7 == 3 {
            format!(
                "Task {i}: tidy the {word} module and move its tests next to the code they cover"
            )
        } else {
            format!("Task {i}: tidy the {word} module")
        };
        let mut t = json!({
            "id": format!("t-{i:06}"),
            "projectName": "bench",
            "title": title,
            "description": "",
            "status": status,
            "order": i,
            "createdAt": "2026-10-01T10:00:00.000Z",
            "updatedAt": "2026-10-01T10:00:00.000Z",
        });
        if i.is_multiple_of(3) {
            t["assignedAgent"] = json!("shell");
        }
        if matches!(status, "done" | "cancelled") {
            t["completedAt"] = json!("2026-10-02T10:00:00.000Z");
        }
        t
    }

    /// A `bench` project with `n` tasks, written into vornd's database: saving
    /// the whole configuration takes minutes at 10,000.
    fn seed(app: &mut App, v: &TestVornd, n: usize) -> Result<(), String> {
        let path = v.home.join("bench");
        std::fs::create_dir_all(&path).map_err(|e| e.to_string())?;
        let mut config = app
            .rpc()
            .call("config:load", Value::Null)
            .map_err(|e| e.to_string())?;
        config["projects"] = json!([{ "name": "bench", "path": path, "preferredAgents": [] }]);
        app.rpc()
            .call("config:save", config)
            .map_err(|e| e.to_string())?;
        let mut store = vorn_store::Store::open_beside(&v.home.join("vorn.db"))
            .map_err(|e| e.to_string())?
            .ok_or("vornd has no database yet")?;
        for i in 0..n {
            store
                .call("dbInsertTask", json!([task(i)]))
                .map_err(|e| e.to_string())?;
        }
        Ok(())
    }

    #[derive(Clone, Copy)]
    enum Step {
        None,
        Move(f32, f32),
        Down,
        Up,
        Wheel(f32),
    }

    enum Target {
        Window(Offscreen),
        Cells(Cells),
    }

    struct Board {
        app: App,
        ui: Ui,
        target: Target,
        size: (f32, f32),
    }

    impl Board {
        fn frame(&mut self) {
            self.app.events(&mut self.ui);
            self.app.frame(&mut self.ui, self.size);
            match &mut self.target {
                Target::Window(t) => {
                    // A full atlas empties itself; the second pass fits the frame.
                    if !self.ui.render_offscreen(t) {
                        self.app.frame(&mut self.ui, self.size);
                        self.ui.render_offscreen(t);
                    }
                }
                Target::Cells(c) => {
                    let _ = c.present(&mut self.ui);
                }
            }
        }

        fn apply(&mut self, step: Step) {
            match step {
                Step::None => {}
                Step::Move(x, y) => self.ui.pointer_move(x, y),
                Step::Down => self.ui.pointer_down(),
                Step::Up => self.ui.pointer_up(),
                Step::Wheel(dy) => self.ui.wheel(dy),
            }
        }

        /// Runs `steps`, one frame each on a 60 Hz clock, timing every frame.
        fn script(&mut self, steps: &[Step]) -> Vec<f64> {
            let mut out = Vec::with_capacity(steps.len());
            let mut next = Instant::now();
            for &step in steps {
                let now = Instant::now();
                if next > now {
                    std::thread::sleep(next - now);
                }
                next += FRAME;
                let t = Instant::now();
                self.apply(step);
                self.frame();
                out.push(ms(t.elapsed()));
            }
            out
        }

        fn rect(&self, id: Id, what: &str) -> Result<Rect, String> {
            self.ui
                .rect_of(id)
                .filter(|r| r.h > 0.0)
                .ok_or_else(|| format!("{what} is not on screen"))
        }
    }

    fn ms(d: Duration) -> f64 {
        d.as_secs_f64() * 1000.0
    }

    fn center(r: Rect) -> (f32, f32) {
        (r.x + r.w / 2.0, r.y + r.h / 2.0)
    }

    /// The app's footprint in MB: physical footprint on macOS, else resident size.
    fn footprint() -> f64 {
        #[cfg(target_os = "macos")]
        {
            let Ok(pid) = i32::try_from(std::process::id()) else {
                return 0.0;
            };
            // SAFETY: rusage_info_v4 is plain data that proc_pid_rusage fills for the flavour named.
            let info = unsafe {
                let mut info: libc::rusage_info_v4 = std::mem::zeroed();
                let ptr = (&mut info as *mut libc::rusage_info_v4).cast::<libc::rusage_info_t>();
                (libc::proc_pid_rusage(pid, libc::RUSAGE_INFO_V4, ptr) == 0).then_some(info)
            };
            info.map_or(0.0, |i| i.ri_phys_footprint as f64 / (1024.0 * 1024.0))
        }
        #[cfg(not(target_os = "macos"))]
        {
            std::fs::read_to_string("/proc/self/status")
                .ok()
                .and_then(|s| {
                    s.lines()
                        .find_map(|l| l.strip_prefix("VmRSS:"))
                        .and_then(|v| v.trim().trim_end_matches("kB").trim().parse::<f64>().ok())
                })
                .map_or(0.0, |kb| kb / 1024.0)
        }
    }

    fn pct(sorted: &[f64], p: f64) -> f64 {
        if sorted.is_empty() {
            return 0.0;
        }
        let i = ((p / 100.0) * (sorted.len() - 1) as f64).round() as usize;
        sorted[i.min(sorted.len() - 1)]
    }

    fn summary(frames: &[f64]) -> Value {
        let mut s = frames.to_vec();
        s.sort_by(f64::total_cmp);
        json!({
            "p50": round(pct(&s, 50.0)),
            "p99": round(pct(&s, 99.0)),
            "max": round(s.last().copied().unwrap_or(0.0)),
            "n": s.len(),
        })
    }

    fn round(x: f64) -> f64 {
        (x * 100.0).round() / 100.0
    }

    /// `board case TIER SURFACE`: one board loaded, scrolled and dragged.
    pub fn case(args: &[String]) -> Result<Value, String> {
        let tier: usize = args
            .first()
            .and_then(|s| s.parse().ok())
            .ok_or("case TIER SURFACE")?;
        let cells = args.get(1).map(String::as_str) == Some("cells");
        let v = TestVornd::start().ok_or("no vornd built: build vornd and vorn-sessiond first")?;
        let mut app = support::connect(&v);
        seed(&mut app, &v, tier)?;
        app.st.project = Some("bench".into());
        app.st.tasks.mode = vorn_app::tasks::Mode::Board;
        let (ui, target, size) = if cells {
            let ui = Ui::cells(&ui_config(1.0));
            let size = vorn_app::ui::cells::units::px(GRID.0, GRID.1);
            (ui, Target::Cells(Cells::offscreen(GRID.0, GRID.1)), size)
        } else {
            let ui = Ui::headless(RenderMode::from_env(), &ui_config(SCALE));
            let px = ((SIZE.0 * SCALE) as u32, (SIZE.1 * SCALE) as u32);
            let target = Target::Window(ui.offscreen(px));
            (ui, target, SIZE)
        };
        let renderer = if cells {
            "cells".to_owned()
        } else {
            ui.renderer_name()
        };
        let mut b = Board {
            app,
            ui,
            target,
            size,
        };
        b.frame();
        let before = footprint();
        let t = Instant::now();
        let config = b
            .app
            .rpc()
            .call("config:load", Value::Null)
            .map_err(|e| e.to_string())?;
        b.app.store.apply("config:changed", config);
        // The board is read from the core's list, a page at a time, as the app loads it.
        let loaded =
            vorn_app::client::store::Store::load(b.app.rpc()).map_err(|e| e.to_string())?;
        b.app.store.tasks = loaded.tasks;
        if b.app.store.tasks.len() != tier {
            return Err(format!(
                "seeded {tier} tasks, the app has {}",
                b.app.store.tasks.len()
            ));
        }
        b.app.st.view = MainView::Tasks;
        b.frame();
        let load_ms = ms(t.elapsed());
        b.script(&[Step::None; 10]);
        let loaded = footprint();

        let mut idle = b.script(&[Step::None; 5]);
        // Past a tenth of a second a frame, a shorter script, or 10,000 cards take an hour.
        let n = if pct(&idle, 50.0) > 100.0 { 10 } else { 60 };
        if n == 60 {
            idle.extend(b.script(&[Step::None; 25]));
        }
        let scroll_id = vorn_app::screen::tasks::column_scroll_id("todo");
        let (cx, cy) = center(b.rect(scroll_id, "the To do column")?);
        let notch = if cells { 3.0 } else { 120.0 };
        let mut wheel = vec![Step::Move(cx, cy)];
        wheel.extend(std::iter::repeat_n(Step::Wheel(notch), n));
        wheel.extend(std::iter::repeat_n(Step::Wheel(-notch), n));
        let scroll = b.script(&wheel);
        b.ui.set_scroll_offset(scroll_id, 0.0);
        b.script(&[Step::None; 10]);
        let from = center(b.rect(card_id(FIRST_TODO), "the first card")?);
        let to = center(b.rect(column_id("in_progress"), "In progress")?);
        let mut steps = vec![Step::Move(from.0, from.1), Step::Down];
        for i in 1..=n {
            let t = i as f32 / n as f32;
            steps.push(Step::Move(
                from.0 + (to.0 - from.0) * t,
                from.1 + (to.1 - from.1) * t,
            ));
        }
        steps.push(Step::Up);
        steps.extend(std::iter::repeat_n(Step::None, n / 3));
        let drag = b.script(&steps);
        let moved = b
            .app
            .store
            .tasks
            .iter()
            .find(|t| t.id == FIRST_TODO)
            .is_some_and(|t| t.status.as_str() == "in_progress");

        let all: Vec<f64> = idle.iter().chain(&scroll).chain(&drag).copied().collect();
        let over = all.iter().filter(|&&f| f > ms(FRAME)).count();
        Ok(json!({
            "tier": tier,
            "surface": if cells { "cells" } else { "window" },
            "renderer": renderer,
            "size": if cells { format!("{}x{} cells", GRID.0, GRID.1) } else { format!("{}x{}@{}", SIZE.0, SIZE.1, SCALE) },
            "load_ms": round(load_ms),
            "idle_ms": summary(&idle),
            "scroll_ms": summary(&scroll),
            "drag_ms": summary(&drag),
            "frames": all.len(),
            "over_16_7": over,
            "dropped": moved,
            "footprint_mb": {
                "before": round(before),
                "loaded": round(loaded),
                "board": round(loaded - before),
                "end": round(footprint()),
            },
        }))
    }

    fn median(vals: &mut [f64]) -> f64 {
        vals.sort_by(f64::total_cmp);
        vals.get(vals.len() / 2).copied().unwrap_or(0.0)
    }

    /// The median of `path` (`a.b`) over the repetitions.
    fn med(reps: &[Value], path: &str) -> f64 {
        let mut vals: Vec<f64> = reps
            .iter()
            .filter_map(|r| path.split('.').try_fold(r, |v, k| v.get(k))?.as_f64())
            .collect();
        round(median(&mut vals))
    }

    pub fn run(args: &[String]) -> Result<(), String> {
        let tiers: Vec<usize> = flag(args, "tiers")
            .unwrap_or("100,1000,10000")
            .split(',')
            .map(|t| t.trim().parse().map_err(|_| format!("bad tier {t:?}")))
            .collect::<Result<_, _>>()?;
        let reps: usize = flag(args, "reps")
            .unwrap_or("3")
            .parse()
            .map_err(|_| "bad --reps")?;
        let surface = flag(args, "surface").unwrap_or("window");
        let exe = std::env::current_exe().map_err(|e| e.to_string())?;
        let mut results = Vec::new();
        for &tier in &tiers {
            let mut runs = Vec::new();
            for rep in 0..reps {
                eprintln!("board: {tier} tasks, {surface}, run {}/{reps}", rep + 1);
                let out = std::process::Command::new(&exe)
                    .args(["case", &tier.to_string(), surface])
                    .output()
                    .map_err(|e| e.to_string())?;
                if !out.status.success() {
                    return Err(String::from_utf8_lossy(&out.stderr).into_owned());
                }
                let v: Value = serde_json::from_slice(&out.stdout).map_err(|e| e.to_string())?;
                runs.push(v);
            }
            let first = runs.first().cloned().unwrap_or_default();
            results.push(json!({
                "tier": tier,
                "surface": surface,
                "renderer": first["renderer"],
                "size": first["size"],
                "reps": reps,
                "load_ms": med(&runs, "load_ms"),
                "idle_p50": med(&runs, "idle_ms.p50"),
                "scroll_p50": med(&runs, "scroll_ms.p50"),
                "scroll_p99": med(&runs, "scroll_ms.p99"),
                "drag_p50": med(&runs, "drag_ms.p50"),
                "drag_p99": med(&runs, "drag_ms.p99"),
                "max": med(&runs, "drag_ms.max").max(med(&runs, "scroll_ms.max")),
                "frames": med(&runs, "frames"),
                "over_16_7": med(&runs, "over_16_7"),
                "dropped": runs.iter().all(|r| r["dropped"] == true),
                "board_mb": med(&runs, "footprint_mb.board"),
                "end_mb": med(&runs, "footprint_mb.end"),
                "runs": runs,
            }));
        }
        print!("{}", table(&results));
        if let Some(path) = flag(args, "out") {
            let doc = json!({ "machine": machine(), "results": results });
            let text = serde_json::to_string_pretty(&doc).map_err(|e| e.to_string())?;
            std::fs::write(path, text + "\n").map_err(|e| e.to_string())?;
        }
        Ok(())
    }

    fn machine() -> Value {
        let sysctl = |k: &str| {
            std::process::Command::new("sysctl")
                .args(["-n", k])
                .output()
                .ok()
                .map(|o| String::from_utf8_lossy(&o.stdout).trim().to_owned())
                .filter(|s| !s.is_empty())
        };
        json!({
            "os": std::env::consts::OS,
            "arch": std::env::consts::ARCH,
            "cpu": sysctl("machdep.cpu.brand_string"),
            "cores": std::thread::available_parallelism().map_or(0, |n| n.get()),
        })
    }

    /// The medians as a Markdown table.
    fn table(results: &[Value]) -> String {
        let mut s = String::from(
            "| Tasks | Load | Idle p50 | Scroll p50 | Scroll p99 | Drag p50 | Drag p99 | Max | Over 16.7 ms | Board | Footprint |\n\
             | ----: | ---: | -------: | ---------: | ---------: | -------: | -------: | --: | -----------: | ----: | --------: |\n",
        );
        for r in results {
            let _ = writeln!(
                s,
                "| {} | {} ms | {} ms | {} ms | {} ms | {} ms | {} ms | {} ms | {} of {} | {} MB | {} MB |",
                r["tier"],
                r["load_ms"],
                r["idle_p50"],
                r["scroll_p50"],
                r["scroll_p99"],
                r["drag_p50"],
                r["drag_p99"],
                r["max"],
                r["over_16_7"],
                r["frames"],
                r["board_mb"],
                r["end_mb"],
            );
        }
        if let Some(r) = results.first() {
            let _ = writeln!(
                s,
                "\nRenderer: {}, {}; median of {} runs.",
                r["renderer"].as_str().unwrap_or_default(),
                r["size"].as_str().unwrap_or_default(),
                r["reps"],
            );
        }
        s
    }
}

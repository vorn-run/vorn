//! Numbers for the browser pane, driven by what a person sees so they
//! compare across implementations: the card's globe pressed until the page's
//! view is seated in the pane, and an agent's `browser:openPane` with a url
//! until the page is seated. No web view loads offscreen; the seat is where
//! the app places it. The device pane on a real simulator, booted only for
//! the bench and shut down after: the card's phone pressed until the
//! picker has its list, an agent's claim and `device:openPane` until the
//! pane has its first frame, and the frames a second that reach the pane.
//! `cargo test --test panes_bench -- --ignored --nocapture`.

#![cfg(unix)]

mod support;

use std::time::{Duration, Instant};

use serde_json::{json, Value};
use support::{add_project, key, wait_for, Shot, TestVornd};
use vorn_app::screen::term_id;
use vorn_app::ui::Id;
use vorn_app::App;

/// Rounds of each measure.
const ROUNDS: usize = 30;

fn pct(mut v: Vec<f64>, p: f64) -> f64 {
    v.sort_by(f64::total_cmp);
    v[((v.len() as f64 - 1.0) * p).round() as usize]
}

fn report(name: &str, v: Vec<f64>) {
    println!(
        "BENCH {name}: n={} p50={:.2}ms p99={:.2}ms",
        v.len(),
        pct(v.clone(), 0.5),
        pct(v, 0.99)
    );
}

/// Milliseconds from `t` until `f` answers, a frame laid out between tries.
fn since<T>(
    t: Instant,
    shot: &mut Shot,
    app: &mut App,
    mut f: impl FnMut(&App) -> Option<T>,
) -> f64 {
    loop {
        app.pump();
        shot.frame(app);
        if f(app).is_some() {
            return t.elapsed().as_secs_f64() * 1000.0;
        }
        assert!(t.elapsed() < Duration::from_secs(60), "timed out");
    }
}

fn seated(app: &App) -> Option<()> {
    (!app.placed_pages().is_empty()).then_some(())
}

fn gone(app: &App) -> Option<()> {
    app.placed_pages().is_empty().then_some(())
}

fn globe(shot: &mut Shot, app: &mut App, sid: &str) -> Id {
    shot.hover(app, term_id(sid));
    wait_for("the card's globe", || {
        app.pump();
        shot.labelled(app, "Button", "Open browser")
    })
}

#[test]
#[ignore]
fn bench_browser_pane_open() {
    let Some(v) = TestVornd::start() else {
        return;
    };
    let mut app = support::connect(&v);
    add_project(&mut app, &v, "demo", json!({}));
    let mut shot = Shot::new();
    app.answer_agents();
    shot.frame(&mut app);
    shot.send(&mut app, key("Tab", 0, Some("\t")));
    shot.type_line(&mut app, "hello");
    let sid = wait_for("the agent's card", || {
        app.pump();
        app.st.selected.clone()
    });
    shot.until(&mut app, "its screen", |app, _| app.pane(&sid).map(|_| ()));

    // The globe, pressed until the page sits in the pane, then pressed again to close it.
    let mut opens = Vec::with_capacity(ROUNDS);
    for _ in 0..=ROUNDS {
        let g = globe(&mut shot, &mut app, &sid);
        let t = Instant::now();
        shot.click(&mut app, g);
        opens.push(since(t, &mut shot, &mut app, seated));
        let g = globe(&mut shot, &mut app, &sid);
        shot.click(&mut app, g);
        since(Instant::now(), &mut shot, &mut app, gone);
    }
    // The first opens a component or a cache cold; it is reported on its own.
    let first = opens.remove(0);
    println!("BENCH browser pane open (globe), first: {first:.2}ms");
    report("browser pane open (globe)", opens);

    // An agent opening the pane at a url, as `browser_open` does.
    let mut asked = Vec::with_capacity(ROUNDS);
    for i in 0..ROUNDS {
        let t = Instant::now();
        let pending = app.rpc().request(
            "browser:openPane",
            json!({ "sessionId": sid, "url": format!("http://localhost:{}/", 3000 + i) }),
        );
        let answered: Value = wait_for("browser:openPane", || {
            app.pump();
            pending.poll()
        })
        .expect("opened");
        assert!(answered.get("url").is_some(), "{answered}");
        asked.push(since(t, &mut shot, &mut app, seated));
        let g = globe(&mut shot, &mut app, &sid);
        shot.click(&mut app, g);
        since(Instant::now(), &mut shot, &mut app, gone);
    }
    report("browser pane open (agent, with url)", asked);
}

/// The simulator the device bench boots: `VORN_BENCH_UDID`, else the first
/// available iPhone that is shut down, so one already in use is left alone.
fn bench_simulator() -> Option<String> {
    if let Ok(u) = std::env::var("VORN_BENCH_UDID") {
        return Some(u);
    }
    let out = vorn_spawn::command("xcrun")
        .args(["simctl", "list", "devices", "available", "-j"])
        .output()
        .ok()?;
    let json: Value = serde_json::from_slice(&out.stdout).ok()?;
    json["devices"]
        .as_object()?
        .iter()
        .filter(|(runtime, _)| runtime.contains("iOS"))
        .flat_map(|(_, list)| list.as_array().into_iter().flatten())
        .find(|d| {
            d["state"] == "Shutdown" && d["name"].as_str().is_some_and(|n| n.starts_with("iPhone"))
        })
        .and_then(|d| d["udid"].as_str().map(str::to_owned))
}

/// Shuts the bench's simulator down however the bench ends.
struct Shutdown(String);

impl Drop for Shutdown {
    fn drop(&mut self) {
        let _ = vorn_spawn::command("xcrun")
            .args(["simctl", "shutdown", &self.0])
            .output();
    }
}

#[test]
#[ignore]
fn bench_device_pane() {
    let Some(udid) = bench_simulator() else {
        eprintln!("skipped: no shut-down iPhone simulator");
        return;
    };
    let _down = Shutdown(udid.clone());
    let Some(v) = TestVornd::start() else {
        return;
    };
    let mut app = support::connect(&v);
    let project = add_project(&mut app, &v, "demo", json!({}));
    std::fs::write(
        project.join("package.json"),
        r#"{ "dependencies": { "react-native": "0.80.0" } }"#,
    )
    .expect("package.json");
    let mut shot = Shot::new();
    app.answer_agents();
    shot.frame(&mut app);
    shot.send(&mut app, key("Tab", 0, Some("\t")));
    shot.type_line(&mut app, "hello");
    let sid = wait_for("the agent's card", || {
        app.pump();
        app.st.selected.clone()
    });
    shot.until(&mut app, "its screen", |app, _| app.pane(&sid).map(|_| ()));

    // The phone, pressed until the picker has its list, then Escape.
    let mut picks = Vec::with_capacity(ROUNDS);
    for _ in 0..=ROUNDS {
        let phone = phone(&mut shot, &mut app, &sid, "Open device");
        let t = Instant::now();
        shot.click(&mut app, phone);
        picks.push(since(t, &mut shot, &mut app, picker_listed));
        shot.send(&mut app, key("Escape", 0, None));
        since(Instant::now(), &mut shot, &mut app, picker_closed);
    }
    let first = picks.remove(0);
    println!("BENCH device picker open (phone), first: {first:.2}ms");
    report("device picker open (phone)", picks);

    // An agent's claim, booting the simulator, until the pane's first frame.
    // vornd gives the app 15 s to answer; a slower boot is asked again.
    let t = Instant::now();
    loop {
        let pending = app
            .rpc()
            .request("device:claim", json!({ "sessionId": sid, "udid": udid }));
        let answer = slow(&mut app, || pending.poll());
        match answer {
            Ok(claimed) if claimed["ok"] == true => break,
            other => {
                eprintln!("claim again after {other:?}");
                assert!(
                    t.elapsed() < Duration::from_secs(180),
                    "the simulator boots"
                );
            }
        }
    }
    ask(&mut app, "device:openPane", json!({ "sessionId": sid }));
    let cold = since(t, &mut shot, &mut app, |a| first_frame(a, &sid));
    println!("BENCH device claim (boot) to first frame: {cold:.2}ms");

    let fps = frame_rate(&mut shot, &mut app, &sid, Duration::from_secs(10));
    println!("BENCH device frames reaching the pane: {fps:.2}/s");

    // The pane closed and opened again on the booted simulator.
    let mut opens = Vec::with_capacity(ROUNDS);
    for _ in 0..ROUNDS {
        let hide = phone(&mut shot, &mut app, &sid, "Hide device");
        shot.click(&mut app, hide);
        forget_frame(&mut app, &sid);
        since(Instant::now(), &mut shot, &mut app, |a| {
            first_frame(a, &sid).is_none().then_some(())
        });
        let t = Instant::now();
        ask(&mut app, "device:openPane", json!({ "sessionId": sid }));
        opens.push(since(t, &mut shot, &mut app, |a| first_frame(a, &sid)));
    }
    report("device pane open (agent, booted) to first frame", opens);
    ask(&mut app, "device:release", json!({ "sessionId": sid }));
}

/// The card's phone button labelled `label`.
fn phone(shot: &mut Shot, app: &mut App, sid: &str, label: &str) -> Id {
    shot.hover(app, term_id(sid));
    wait_for("the card's phone", || {
        app.pump();
        shot.labelled(app, "Button", label)
    })
}

fn picker_listed(app: &App) -> Option<()> {
    app.st
        .bench
        .picker
        .as_ref()
        .filter(|p| !p.loading)
        .map(|_| ())
}

fn picker_closed(app: &App) -> Option<()> {
    app.st.bench.picker.is_none().then_some(())
}

/// The pane has a frame to draw: its size read from the session's PNG.
fn first_frame(app: &App, sid: &str) -> Option<()> {
    app.st.bench.frames.get(sid).map(|_| ())
}

/// Drops the session's last frame, so the next open waits for a fresh one.
fn forget_frame(app: &mut App, sid: &str) {
    app.device_frames().lock().expect("frames").remove(sid);
}

/// A fresh PNG in the session's slot of the pane's frames, as a token.
fn frame_token(app: &App, sid: &str) -> Option<usize> {
    let frames = app.device_frames();
    let f = frames.lock().expect("frames");
    f.get(sid).map(|p| p.as_ptr() as usize)
}

/// New frames a second reaching the pane's frames for `sid`.
fn frame_rate(shot: &mut Shot, app: &mut App, sid: &str, over: Duration) -> f64 {
    let (t, mut last, mut n) = (Instant::now(), frame_token(app, sid), 0u32);
    while t.elapsed() < over {
        app.pump();
        shot.frame(app);
        let now = frame_token(app, sid);
        if now != last {
            n += 1;
            last = now;
        }
        std::thread::sleep(Duration::from_millis(2));
    }
    f64::from(n) / t.elapsed().as_secs_f64()
}

/// Pumps `app` until `f` answers, for up to three minutes.
fn slow<T>(app: &mut App, mut f: impl FnMut() -> Option<T>) -> T {
    let t = Instant::now();
    loop {
        app.pump();
        if let Some(v) = f() {
            return v;
        }
        assert!(t.elapsed() < Duration::from_secs(180), "timed out");
        std::thread::sleep(Duration::from_millis(5));
    }
}

/// Asks vornd as an agent's tools do, pumping the app that answers.
fn ask(app: &mut App, method: &str, params: Value) -> Value {
    let pending = app.rpc().request(method, params);
    wait_for(method, || {
        app.pump();
        pending.poll()
    })
    .unwrap_or_else(|e| panic!("{method}: {e:?}"))
}

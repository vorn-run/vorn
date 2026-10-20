//! Numbers for the browser pane, driven by what a person sees so they
//! compare across implementations: the card's globe pressed until the page's
//! view is seated in the pane, and an agent's `browser:openPane` with a url
//! until the page is seated. No web view loads offscreen; the seat is where
//! the app places it. `cargo test --test panes_bench -- --ignored --nocapture`.

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

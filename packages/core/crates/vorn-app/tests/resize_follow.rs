//! A card's terminal follows its box promptly and exactly: after the window
//! or a divider moves, the PTY takes the box's size, the program repaints at
//! it, and what the card shows is that repaint, with nothing left over from
//! the size before. A new card paints at its box's size without a nudge.
//! A drag resizes the program as it goes, never more than once a frame.
//!
//! The program is a full-screen stand-in (`fixtures/tui-standin.py`) that
//! repaints every row with the size it sees on every SIGWINCH, as an
//! agent's TUI does. Timings go to stderr and, when `VORN_RESIZE_REPORT`
//! names a file, there as JSON.

#![cfg(unix)]

mod support;

use std::path::Path;
use std::time::{Duration, Instant};

use serde_json::{json, Value};
use support::{add_project, connect, wait_for, Shot, TestVornd};
use vorn_app::act::Act;
use vorn_app::screen::flex_divider_id;
use vorn_app::screen::menus::MENU;
use vorn_app::state::MenuKind;
use vorn_app::ui::{item_id, Ui};
use vorn_app::view::PaneView;
use vorn_app::App;

/// The agent the stand-in runs as.
const AGENT: &str = "acme";

/// How long after the last size change the terminal may take to follow.
const FOLLOW: Duration = Duration::from_millis(800);
/// How long a card just opened may take: the program's spawn is in it.
const OPEN: Duration = Duration::from_secs(3);
/// How long after that the program's repaint may take to show.
const REPAINT: Duration = Duration::from_millis(1200);
/// Frames in a short drag: it may resize the program at most this often.
const DRAG_STEPS: u32 = 12;
/// How long a hand rests on the way through a drag.
const PAUSE: Duration = Duration::from_millis(200);

/// What a card's box holds in whole cells, as the frame measures it.
fn box_cells(app: &App, ui: &Ui, id: &str) -> Option<(u16, u16)> {
    let r = app.term_box(id)?;
    let cell = ui.text.cell();
    Some(((r.w / cell.w).floor() as u16, (r.h / cell.h).floor() as u16))
}

/// The stand-in's repaint serial when every row of `v` reads as one repaint
/// at `cols`x`rows`; `None` when any row is missing, stale or differs.
fn painted(v: &PaneView, (cols, rows): (u16, u16)) -> Option<u64> {
    if (v.cols, v.rows) != (cols, rows) {
        return None;
    }
    let text = v.text();
    let lines: Vec<&str> = text.split('\n').collect();
    let mut serial = None;
    for y in 0..rows {
        let line = lines.get(usize::from(y))?;
        let head = format!("R{y:02} {cols}x{rows} #");
        let rest = line.strip_prefix(head.as_str())?;
        let n: u64 = rest.split(' ').next()?.parse().ok()?;
        if *serial.get_or_insert(n) != n {
            return None;
        }
    }
    serial
}

#[derive(Debug, Default, Clone, serde::Serialize)]
struct Followed {
    /// Milliseconds from the last size change to the terminal having the box's size.
    term_ms: Option<u128>,
    /// Milliseconds from the last size change to the card showing a whole repaint at it.
    paint_ms: Option<u128>,
}

/// Frames until every card's terminal follows its box and shows the
/// program's repaint at it; how long each took from `since`.
fn follow(
    app: &mut App,
    shot: &mut Shot,
    ids: &[String],
    since: Instant,
    what: &str,
    frames: &mut Vec<u128>,
) -> Vec<Followed> {
    let mut out = vec![Followed::default(); ids.len()];
    loop {
        let t = Instant::now();
        shot.frame(app);
        app.pump();
        frames.push(t.elapsed().as_micros());
        let now = since.elapsed().as_millis();
        for (i, id) in ids.iter().enumerate() {
            let Some(cells) = box_cells(app, &shot.ui, id) else {
                continue;
            };
            let Some(v) = app.pane(id) else { continue };
            if out[i].term_ms.is_none() && (v.cols, v.rows) == cells {
                out[i].term_ms = Some(now);
            }
            if out[i].paint_ms.is_none() && painted(v, cells).is_some() {
                out[i].paint_ms = Some(now);
            }
        }
        if out.iter().all(|f| f.paint_ms.is_some()) {
            break;
        }
        assert!(
            since.elapsed() < support::PATIENCE,
            "{what}: the terminals never followed their boxes: {out:?}\n{}",
            screens(app, &shot.ui, ids)
        );
        std::thread::sleep(Duration::from_millis(4));
    }
    eprintln!("{what}: {out:?}");
    out
}

/// Each card's repaint serial now: the stand-in repaints once per SIGWINCH.
fn serials(app: &App, ui: &Ui, ids: &[String]) -> Vec<u64> {
    ids.iter()
        .map(|id| {
            let cells = box_cells(app, ui, id).expect("the card is drawn");
            let v = app.pane(id).expect("the card has a screen");
            painted(v, cells).expect("a whole repaint")
        })
        .collect()
}

/// Every card was resized at least once and at most `most` times between
/// the serials `before` and now.
fn assert_resizes(
    app: &App,
    ui: &Ui,
    ids: &[String],
    before: &[u64],
    most: u64,
    what: &str,
) -> Vec<u64> {
    let got: Vec<u64> = serials(app, ui, ids)
        .iter()
        .zip(before)
        .map(|(after, before)| after - before)
        .collect();
    eprintln!("{what}: resizes {got:?}");
    assert!(
        got.iter().all(|&n| (1..=most).contains(&n)),
        "{what}: each card should be resized once to {most} times, was {got:?}"
    );
    got
}

fn screens(app: &App, ui: &Ui, ids: &[String]) -> String {
    ids.iter()
        .map(|id| {
            let cells = box_cells(app, ui, id);
            let v = app.pane(id);
            format!(
                "{id}: box {cells:?}, term {:?}\n{}",
                v.map(|v| (v.cols, v.rows)),
                v.map(PaneView::text).unwrap_or_default()
            )
        })
        .collect::<Vec<_>>()
        .join("\n---\n")
}

/// Every card shows one whole repaint at its box's size: nothing stale.
fn assert_clean(app: &App, ui: &Ui, ids: &[String], what: &str) {
    for id in ids {
        let cells = box_cells(app, ui, id).expect("the card is drawn");
        let v = app.pane(id).expect("the card has a screen");
        assert!(
            painted(v, cells).is_some(),
            "{what}: {id} shows a stale or partial screen at {cells:?}\n{}",
            screens(app, ui, ids)
        );
    }
}

/// `open` allows for the program starting: its first paint comes after the spawn.
fn assert_prompt(got: &[Followed], what: &str, open: bool) {
    let follow = if open { OPEN } else { FOLLOW };
    for (i, f) in got.iter().enumerate() {
        let term = f.term_ms.expect("followed");
        let paint = f.paint_ms.expect("painted");
        assert!(
            term <= follow.as_millis(),
            "{what}: card {i}'s terminal took {term} ms to take its box's size"
        );
        assert!(
            paint <= (follow + REPAINT).as_millis(),
            "{what}: card {i} took {paint} ms to show the repaint"
        );
    }
}

/// A window drag by its edge: `steps` frames from `from` to `to`, a frame's
/// time apart, inside a live resize as the window reports one. A person's
/// hand pauses on the way: `pause` after a third and two thirds of it.
fn drag_window(
    app: &mut App,
    shot: &mut Shot,
    (from, to): ((f32, f32), (f32, f32)),
    steps: u32,
    pause: Duration,
) {
    shot.ui.set_live_resize(true);
    for s in 1..=steps {
        let k = s as f32 / steps as f32;
        shot.size = (from.0 + (to.0 - from.0) * k, from.1 + (to.1 - from.1) * k);
        shot.frame(app);
        app.pump();
        std::thread::sleep(Duration::from_millis(16));
        if s == steps / 3 || s == 2 * steps / 3 {
            hold_still(app, shot, pause);
        }
    }
    shot.ui.set_live_resize(false);
}

/// Frames for `d` with nothing moving, as the window draws while a hand rests.
fn hold_still(app: &mut App, shot: &mut Shot, d: Duration) {
    let until = Instant::now() + d;
    while Instant::now() < until {
        shot.frame(app);
        app.pump();
        std::thread::sleep(Duration::from_millis(16));
    }
}

/// Drags the root divider `dx` across in `steps` frames.
fn drag_divider(app: &mut App, shot: &mut Shot, dx: f32, steps: u32) {
    shot.frame(app);
    let r = shot
        .ui
        .rect_of(flex_divider_id(1))
        .expect("the root divider is on screen");
    let (x, y) = (r.x + r.w / 2.0, r.y + r.h / 2.0);
    shot.ui.pointer_move(x, y);
    shot.ui.pointer_down();
    app.events(&mut shot.ui);
    shot.frame(app);
    for s in 1..=steps {
        shot.ui.pointer_move(x + dx * s as f32 / steps as f32, y);
        app.events(&mut shot.ui);
        shot.frame(app);
        app.pump();
        std::thread::sleep(Duration::from_millis(16));
        if s == steps / 3 || s == 2 * steps / 3 {
            hold_still(app, shot, PAUSE);
        }
    }
    shot.ui.pointer_up();
    app.events(&mut shot.ui);
    shot.frame(app);
}

fn create(app: &mut App, dir: &Path) -> String {
    let info = app
        .rpc()
        .call(
            "terminal:create",
            json!({ "agentType": AGENT, "projectName": "demo", "projectPath": dir }),
        )
        .expect("agent");
    let id = info["id"].as_str().expect("id").to_owned();
    wait_for("the card", || {
        app.pump();
        app.store.session(&id).map(|_| ())
    });
    id
}

fn percentile(sorted: &[u128], p: f64) -> u128 {
    if sorted.is_empty() {
        return 0;
    }
    let i = ((sorted.len() - 1) as f64 * p).round() as usize;
    sorted[i]
}

/// The stand-in `script` under the test vornd, through the app, with the
/// timings written under `name`.
fn follows(script: &str) {
    let Some(v) = TestVornd::start() else {
        return;
    };
    let python = ["/usr/bin/python3", "/opt/homebrew/bin/python3"]
        .iter()
        .find(|p| Path::new(p).is_file());
    let Some(python) = python else {
        eprintln!("skipped: no python3 for the stand-in");
        return;
    };
    let script = Path::new(env!("CARGO_MANIFEST_DIR")).join(script);
    let mut app = connect(&v);
    let dir = add_project(&mut app, &v, "demo", json!({}));
    let mut config = app.rpc().call("config:load", Value::Null).expect("config");
    config["agentCommands"][AGENT] = json!({ "command": python, "args": [script] });
    app.rpc().call("config:save", config).expect("saved");

    let mut shot = Shot::at((1400.0, 900.0), 1.0);
    shot.frame(&mut app);
    let mut frames = Vec::new();
    let mut report = serde_json::Map::new();

    // Three cards open: each paints at its box's size without a nudge.
    let opened = Instant::now();
    let ids: Vec<String> = (0..3).map(|_| create(&mut app, &dir)).collect();
    let first = follow(&mut app, &mut shot, &ids, opened, "open", &mut frames);
    report.insert("open".into(), json!(first));

    // The window dragged smaller, then larger, over a dozen frames each.
    let mut legs = Vec::new();
    let mut resizes = Vec::new();
    // Both sides, then the width alone, then the height alone.
    for (from, to) in [
        ((1400.0, 900.0), (1000.0, 640.0)),
        ((1000.0, 640.0), (1500.0, 980.0)),
        ((1500.0, 980.0), (1100.0, 980.0)),
        ((1100.0, 980.0), (1100.0, 700.0)),
    ] {
        let before = serials(&app, &shot.ui, &ids);
        drag_window(&mut app, &mut shot, (from, to), DRAG_STEPS, PAUSE);
        let since = Instant::now();
        let got = follow(&mut app, &mut shot, &ids, since, "window drag", &mut frames);
        assert_clean(&app, &shot.ui, &ids, "after the window drag");
        // Settled: nothing more is on its way.
        std::thread::sleep(Duration::from_millis(400));
        app.pump();
        let most = u64::from(DRAG_STEPS);
        let n = assert_resizes(&app, &shot.ui, &ids, &before, most, "window drag");
        resizes.push(n);
        legs.push(got);
    }
    // A long drag, a second and a half of it: the program follows on the
    // way, at most once a frame, and ends at the last size.
    let before = serials(&app, &shot.ui, &ids);
    drag_window(
        &mut app,
        &mut shot,
        ((1100.0, 700.0), (1400.0, 900.0)),
        90,
        Duration::ZERO,
    );
    let since = Instant::now();
    let got = follow(&mut app, &mut shot, &ids, since, "long drag", &mut frames);
    assert_clean(&app, &shot.ui, &ids, "after the long drag");
    std::thread::sleep(Duration::from_millis(400));
    app.pump();
    let n = assert_resizes(&app, &shot.ui, &ids, &before, 90, "long drag");
    report.insert("long".into(), json!({ "follow": got, "resizes": n }));
    report.insert("window".into(), json!(legs));
    report.insert("window_resizes".into(), json!(resizes));

    // Flexible: one divider dragged resizes the panes beside it only.
    shot.click(
        &mut app,
        App::control("toolbar", &Act::Open(MenuKind::ViewOptions)),
    );
    let labels = app.menu_labels(&shot.ui);
    let i = labels
        .iter()
        .position(|l| l == "Flexible")
        .unwrap_or_else(|| panic!("Flexible in {labels:?}"));
    shot.click(&mut app, item_id(MENU, i));
    let since = Instant::now();
    follow(&mut app, &mut shot, &ids, since, "flexible", &mut frames);
    let mut dragged = Vec::new();
    let mut divider_resizes = Vec::new();
    for dx in [-220.0, 180.0] {
        let before = serials(&app, &shot.ui, &ids);
        drag_divider(&mut app, &mut shot, dx, DRAG_STEPS);
        let since = Instant::now();
        let got = follow(
            &mut app,
            &mut shot,
            &ids,
            since,
            "divider drag",
            &mut frames,
        );
        assert_clean(&app, &shot.ui, &ids, "after the divider drag");
        std::thread::sleep(Duration::from_millis(400));
        app.pump();
        // Only the panes beside the divider change size.
        let after = serials(&app, &shot.ui, &ids);
        let n: Vec<u64> = after.iter().zip(&before).map(|(a, b)| a - b).collect();
        eprintln!("divider drag: resizes {n:?}");
        assert!(
            n.iter().all(|&k| k <= u64::from(DRAG_STEPS)) && n.iter().any(|&k| k > 0),
            "divider drag: resizes {n:?}"
        );
        divider_resizes.push(n);
        dragged.push(got);
    }
    report.insert("divider".into(), json!(dragged));
    report.insert("divider_resizes".into(), json!(divider_resizes));

    // A new card beside them paints at its box's size, and the others keep theirs.
    let opened = Instant::now();
    let new = create(&mut app, &dir);
    let mut all = ids.clone();
    all.push(new);
    let late = follow(&mut app, &mut shot, &all, opened, "new card", &mut frames);
    assert_clean(&app, &shot.ui, &all, "after a new card");
    report.insert("new".into(), json!(late));

    let mut sorted = frames.clone();
    sorted.sort_unstable();
    let p50 = percentile(&sorted, 0.5);
    let p99 = percentile(&sorted, 0.99);
    eprintln!("frames: {} at p50 {p50} us, p99 {p99} us", sorted.len());
    report.insert("frame_us".into(), json!({ "p50": p50, "p99": p99 }));
    if let Some(path) = std::env::var_os("VORN_RESIZE_REPORT") {
        let name = script.file_stem().unwrap_or_default().to_string_lossy();
        let path = format!("{}-{name}.json", path.to_string_lossy());
        std::fs::write(path, Value::Object(report).to_string()).expect("report");
    }

    assert_prompt(&first, "open", true);
    for got in &legs {
        assert_prompt(got, "window drag", false);
    }
    for got in &dragged {
        assert_prompt(got, "divider drag", false);
    }
    assert_prompt(&late, "new card", true);
}

/// A program that repaints every row on SIGWINCH, as a curses-style TUI does.
#[test]
fn a_cards_terminal_follows_its_box_promptly_and_shows_one_repaint() {
    follows("tests/fixtures/tui-standin.py");
}

/// A program that paints like an agent's terminal UI: on the alternate
/// screen, inside a synchronized-output hold, after probing the terminal.
#[test]
fn a_synchronized_repaint_on_the_alternate_screen_follows_the_box_too() {
    follows("tests/fixtures/tui-standin-sync.py");
}

/// Frames for `for_ms` after `since`, noting when each card's screen last
/// changed and when its terminal took the box's size.
fn settle(
    app: &mut App,
    shot: &mut Shot,
    ids: &[String],
    since: Instant,
    for_ms: u64,
) -> Vec<(Option<u128>, u128)> {
    let mut last_rev: Vec<u64> = vec![0; ids.len()];
    let mut out: Vec<(Option<u128>, u128)> = vec![(None, 0); ids.len()];
    while since.elapsed() < Duration::from_millis(for_ms) {
        shot.frame(app);
        app.pump();
        let now = since.elapsed().as_millis();
        for (i, id) in ids.iter().enumerate() {
            let (Some(cells), Some(v)) = (box_cells(app, &shot.ui, id), app.pane(id)) else {
                continue;
            };
            if out[i].0.is_none() && (v.cols, v.rows) == cells {
                out[i].0 = Some(now);
            }
            if v.rev != last_rev[i] {
                last_rev[i] = v.rev;
                out[i].1 = now;
            }
        }
        std::thread::sleep(Duration::from_millis(4));
    }
    out
}

/// The real agent CLI named by `VORN_RESIZE_AGENT_BIN`, resized the same
/// way: its screens and timings go to stderr for a person to read.
#[test]
fn an_agent_cli_resized_in_a_card_diagnostic() {
    let Ok(bin) = std::env::var("VORN_RESIZE_AGENT_BIN") else {
        eprintln!("skipped: VORN_RESIZE_AGENT_BIN names no agent CLI");
        return;
    };
    let Some(v) = TestVornd::start() else {
        return;
    };
    let mut app = connect(&v);
    let dir = add_project(&mut app, &v, "demo", json!({}));
    let mut config = app.rpc().call("config:load", Value::Null).expect("config");
    config["agentCommands"][AGENT] = json!({ "command": bin, "args": [] });
    app.rpc().call("config:save", config).expect("saved");
    let mut shot = Shot::at((1400.0, 900.0), 1.0);
    shot.frame(&mut app);
    let ids: Vec<String> = (0..2).map(|_| create(&mut app, &dir)).collect();
    let got = settle(&mut app, &mut shot, &ids, Instant::now(), 9000);
    eprintln!("open: {got:?}\n{}", screens(&app, &shot.ui, &ids));
    // A long line typed into the first card's input, never sent.
    shot.click(&mut app, vorn_app::screen::term_id(&ids[0]));
    let typed = std::env::var("VORN_RESIZE_TYPE").unwrap_or_default();
    for ch in typed.chars() {
        shot.send(&mut app, vorn_app::ui::Input::char(ch));
        app.pump();
    }
    let got = settle(&mut app, &mut shot, &ids, Instant::now(), 2500);
    eprintln!("typed: {got:?}\n{}", screens(&app, &shot.ui, &ids));
    for (from, to) in [
        ((1400.0, 900.0), (1000.0, 640.0)),
        ((1000.0, 640.0), (1500.0, 980.0)),
        ((1500.0, 980.0), (900.0, 700.0)),
    ] {
        drag_window(&mut app, &mut shot, (from, to), DRAG_STEPS, PAUSE);
        let got = settle(&mut app, &mut shot, &ids, Instant::now(), 3000);
        eprintln!("drag to {to:?}: {got:?}\n{}", screens(&app, &shot.ui, &ids));
    }
    eprintln!(
        "log tail:\n{}",
        v.log()
            .lines()
            .rev()
            .take(60)
            .collect::<Vec<_>>()
            .into_iter()
            .rev()
            .collect::<Vec<_>>()
            .join("\n")
    );
}

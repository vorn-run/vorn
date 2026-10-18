//! Numbers for the Files pane and a shell's blocks, driven by what a person
//! sees so they compare across implementations: opening a large folder,
//! scrolling a long block log, and a keystroke's way to the screen with
//! blocks on. `cargo test --test files_blocks_bench -- --ignored --nocapture`.

#![cfg(unix)]

mod support;

use std::path::Path;
use std::time::{Duration, Instant};

use serde_json::{json, Value};
use support::{add_project, key, wait_for, Shot, TestVornd};
use vorn_app::ui::{mods, Id, Input};
use vorn_app::App;

fn wait(shot: &mut Shot, app: &mut App, role: &str, label: &str) -> Id {
    wait_for(&format!("{role} {label:?}"), || {
        app.pump();
        shot.labelled(app, role, label)
    })
}

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

/// Milliseconds from now until `f` answers, a frame laid out between tries.
fn timed<T>(
    shot: &mut Shot,
    app: &mut App,
    mut f: impl FnMut(&mut Shot, &mut App) -> Option<T>,
) -> f64 {
    let t = Instant::now();
    loop {
        app.pump();
        if f(shot, app).is_some() {
            return t.elapsed().as_secs_f64() * 1000.0;
        }
        assert!(t.elapsed() < Duration::from_secs(60), "timed out");
    }
}

#[test]
#[ignore]
fn bench_open_a_large_folder() {
    let Some(v) = TestVornd::start() else {
        return;
    };
    let mut app = support::connect(&v);
    let dir = v.home.join("big");
    std::fs::create_dir_all(dir.join("many")).expect("dir");
    for i in 0..5000 {
        std::fs::write(dir.join("many").join(format!("file-{i:05}.txt")), "x\n").expect("w");
    }
    for i in 0..200 {
        std::fs::write(dir.join(format!("top-{i:03}.rs")), "fn main() {}\n").expect("w");
    }
    let mut config = app.rpc().call("config:load", Value::Null).expect("config");
    config["agentCommands"]["acme"] = json!({ "command": "/bin/sh", "args": [] });
    app.rpc().call("config:save", config).expect("saved");
    let info = app
        .rpc()
        .call(
            "terminal:create",
            json!({ "agentType": "acme", "projectName": "big", "projectPath": dir }),
        )
        .expect("agent");
    let sid = info["id"].as_str().expect("id").to_owned();
    wait_for("the card", || {
        app.pump();
        app.store.session(&sid).map(|_| ())
    });
    // Where Files is a component, compile it first: a test's vornd compiles at first use, a real one at start.
    let _ = app.rpc().call(
        "extension:call",
        json!({ "id": "vorn.files", "action": "prune", "args": { "live": [sid] } }),
    );
    let mut shot = Shot::new();
    let card = App::control("card-box", &vorn_app::act::Act::Select(sid.clone()));
    shot.click(&mut app, card);
    shot.hover(&mut app, card);
    let browse = wait(&mut shot, &mut app, "Button", "Browse files");
    shot.click(&mut app, browse);
    let open = timed(&mut shot, &mut app, |s, a| {
        s.labelled(a, "TreeItem", "many")
    });
    println!("BENCH files: root of 201 entries listed in {open:.1}ms");
    let many = wait(&mut shot, &mut app, "TreeItem", "many");
    shot.click(&mut app, many);
    let expand = timed(&mut shot, &mut app, |s, a| {
        s.labelled(a, "TreeItem", "file-00000.txt")
    });
    println!("BENCH files: folder of 5000 entries opened in {expand:.1}ms");
    let mut frames = Vec::new();
    for _ in 0..60 {
        let t = Instant::now();
        shot.draw(&mut app);
        frames.push(t.elapsed().as_secs_f64() * 1000.0);
    }
    report("files frame with 5200 rows open", frames);
    let filter = wait(&mut shot, &mut app, "TextInput", "Filter files");
    shot.ui.set_focus(Some(filter));
    let mut typed = Vec::new();
    for ch in "file-0499".chars() {
        let t = Instant::now();
        shot.send(&mut app, Input::char(ch));
        shot.draw(&mut app);
        typed.push(t.elapsed().as_secs_f64() * 1000.0);
    }
    let settle = timed(&mut shot, &mut app, |s, a| {
        (s.labelled(a, "TreeItem", "file-04990.txt").is_some()
            && s.labelled(a, "TreeItem", "file-00000.txt").is_none())
        .then_some(())
    });
    report("files filter keystroke to frame", typed);
    println!("BENCH files: filter settled {settle:.1}ms after the last key");
}

fn integrated_shell() -> Option<&'static str> {
    ["/bin/zsh", "/usr/bin/zsh", "/bin/bash", "/usr/bin/bash"]
        .into_iter()
        .find(|p| Path::new(p).is_file())
}

#[test]
#[ignore]
fn bench_block_log_and_keystrokes() {
    let Some(sh) = integrated_shell() else {
        return;
    };
    let Some(v) = TestVornd::start() else {
        return;
    };
    let mut app = support::connect(&v);
    add_project(&mut app, &v, "demo", json!({ "shell": sh }));
    let mut shot = Shot::new();
    shot.send(&mut app, key("Backquote", mods::CTRL, Some("`")));
    let field = wait(&mut shot, &mut app, "TextInput", "Type a command");
    let id = wait_for("the shell's card", || {
        app.pump();
        app.st.selected.clone()
    });
    shot.until(&mut app, "the first prompt", |app, _| {
        (!app.grid()?.lines(&id)?.prompts.is_empty()).then_some(())
    });
    shot.click(&mut app, field);
    for i in 0..100 {
        shot.type_line(&mut app, &format!("seq 1 {}", 3 + i % 5));
        let want = i + 1;
        wait_for("the block", || {
            app.pump();
            let tree = app.frame(&mut shot.ui, shot.size);
            let n = tree
                .nodes
                .iter()
                .filter(|(_, n)| {
                    format!("{:?}", n.role()) == "ListItem"
                        && n.label().is_some_and(|l| l.starts_with("seq 1 "))
                })
                .count();
            (n >= want.min(100)).then_some(())
        });
    }
    let list = wait(&mut shot, &mut app, "List", "Finished commands");
    let mut frames = Vec::new();
    for step in 0..120 {
        let off = (step % 60) as f32 * 40.0;
        shot.ui.set_scroll_offset(list, off);
        let t = Instant::now();
        shot.draw(&mut app);
        frames.push(t.elapsed().as_secs_f64() * 1000.0);
    }
    report("block log scroll frame (100 blocks)", frames);

    // A key into a running program with blocks on: sent to its echo on the pane.
    shot.type_line(&mut app, "cat");
    shot.until(&mut app, "cat running past the composer", |a, ui| {
        (!ui.focused(field) && a.grid()?.lines(&id).is_some()).then_some(())
    });
    std::thread::sleep(Duration::from_millis(800));
    shot.frame(&mut app);
    let mut keys = Vec::new();
    for (i, ch) in "abcdefghijklmnopqrstuvwxyzabcdefghijklmnopqrstuvwx"
        .chars()
        .enumerate()
    {
        let t = Instant::now();
        shot.send(&mut app, Input::char(ch));
        let want = i + 1;
        loop {
            app.pump();
            shot.frame(&mut app);
            let text = app
                .pane(&id)
                .map(|p| String::from_utf8_lossy(&p.text).into_owned())
                .unwrap_or_default();
            let last = text
                .lines()
                .rev()
                .find(|l| !l.trim().is_empty())
                .unwrap_or("")
                .trim()
                .len();
            if last >= want {
                break;
            }
            assert!(
                t.elapsed() < Duration::from_secs(10),
                "echo of {ch}: {text:?}"
            );
        }
        keys.push(t.elapsed().as_secs_f64() * 1000.0);
    }
    report("keystroke to echo with blocks on", keys);
    app.grid().expect("grid").text(&id, "\r\u{4}");
    wait(&mut shot, &mut app, "ListItem", "cat");
    let field = wait(&mut shot, &mut app, "TextInput", "Type a command");

    // Typing in the composer: a key to its frame, and to its completions.
    shot.ui.set_focus(Some(field));
    let mut typed = Vec::new();
    for ch in "git checko".chars() {
        let t = Instant::now();
        shot.send(&mut app, Input::char(ch));
        shot.draw(&mut app);
        typed.push(t.elapsed().as_secs_f64() * 1000.0);
    }
    report("composer keystroke to frame", typed);
    let done = timed(&mut shot, &mut app, |s, a| {
        s.labelled(a, "ListBoxOption", "checkout")
    });
    println!("BENCH composer: completions {done:.1}ms after the last key");
}

//! The Files pane and a shell's command blocks driven only by what a person
//! sees (accessible roles and labels, the keyboard, files on disk), so the
//! same steps replay against any implementation of them. Each state is saved
//! as pixels (`VORN_APP_SHOT`) and as cells at 120x36 and 80x24.

#![cfg(unix)]

mod support;

use std::path::Path;
use std::time::Duration;

use serde_json::{json, Value};
use support::{add_project, key, seed_acme, wait_for, Shot, TestVornd};
use vorn_app::app::MOD;
use vorn_app::ui::{mods, Id};
use vorn_app::App;

/// The last control drawn with `role` and `label`, once a frame draws one.
fn wait(shot: &mut Shot, app: &mut App, role: &str, label: &str) -> Id {
    wait_for(&format!("{role} {label:?}"), || {
        app.pump();
        shot.labelled(app, role, label)
    })
}

/// Whether a frame draws a control with `role` and `label` now.
fn shown(shot: &mut Shot, app: &mut App, role: &str, label: &str) -> bool {
    shot.labelled(app, role, label).is_some()
}

/// An agent session in `dir`, its agent a plain shell.
fn agent(app: &mut App, dir: &Path) -> String {
    let mut config = app.rpc().call("config:load", Value::Null).expect("config");
    config["agentCommands"]["acme"] = json!({ "command": "/bin/sh", "args": [] });
    app.rpc().call("config:save", config).expect("saved");
    let info = app
        .rpc()
        .call(
            "terminal:create",
            json!({ "agentType": "acme", "projectName": "acme", "projectPath": dir }),
        )
        .expect("agent");
    let id = info["id"].as_str().expect("id").to_owned();
    wait_for("the agent's card", || {
        app.pump();
        app.store.session(&id).map(|_| ())
    });
    id
}

fn both(shot: &mut Shot, app: &mut App, name: &str, cells: &[&str]) {
    shot.save(app, name);
    support::every_size(app, name, cells);
}

#[test]
fn the_files_pane_browses_edits_finds_saves_and_pops_out() {
    let Some(v) = TestVornd::start() else {
        return;
    };
    let mut app = support::connect(&v);
    let repo = seed_acme(&mut app, &v);
    let mut shot = Shot::new();
    let sid = agent(&mut app, &repo);
    let card = App::control("card-box", &vorn_app::act::Act::Select(sid.clone()));
    shot.click(&mut app, card);
    shot.hover(&mut app, card);

    let browse = wait(&mut shot, &mut app, "Button", "Browse files");
    shot.click(&mut app, browse);
    let src = wait(&mut shot, &mut app, "TreeItem", "src");
    both(
        &mut shot,
        &mut app,
        "files-tree",
        &["Files", "src", "README.md"],
    );
    shot.click(&mut app, src);
    let file = wait(&mut shot, &mut app, "TreeItem", "app.ts");
    shot.click(&mut app, file);
    let editor = wait(&mut shot, &mut app, "MultilineTextInput", "Edit app.ts");
    wait(&mut shot, &mut app, "Tab", "app.ts");
    both(&mut shot, &mut app, "files-open", &["app.ts"]);

    // Typing marks it unsaved; Mod+S writes it.
    shot.click(&mut app, editor);
    shot.type_text(&mut app, "// edited\n");
    wait(&mut shot, &mut app, "Button", "Save");
    wait(&mut shot, &mut app, "Image", "Unsaved changes");
    both(&mut shot, &mut app, "files-dirty", &["app.ts"]);
    shot.send(&mut app, key("KeyS", MOD(), None));
    let path = repo.join("src/app.ts");
    wait_for("the save on disk", || {
        app.pump();
        std::fs::read_to_string(&path)
            .ok()
            .filter(|t| t.contains("// edited"))
    });
    wait_for("the edit saved", || {
        app.pump();
        (!shown(&mut shot, &mut app, "Button", "Save")).then_some(())
    });

    // Find counts and steps through the matches.
    let find = wait(&mut shot, &mut app, "Button", "Find in file");
    shot.click(&mut app, find);
    shot.type_text(&mut app, "name");
    shot.send(&mut app, key("Enter", 0, Some("\r")));
    shot.frame(&mut app);
    both(&mut shot, &mut app, "files-find", &["app.ts"]);
    shot.send(&mut app, key("Escape", 0, None));

    // An unsaved edit asks before its tab closes.
    shot.ui.set_focus(Some(editor));
    shot.type_text(&mut app, "x");
    std::thread::sleep(Duration::from_millis(450));
    let tab = wait(&mut shot, &mut app, "Tab", "app.ts");
    shot.hover(&mut app, tab);
    let close = wait(&mut shot, &mut app, "Button", "Close app.ts");
    shot.click(&mut app, close);
    wait(
        &mut shot,
        &mut app,
        "AlertDialog",
        "Discard unsaved changes?",
    );
    both(
        &mut shot,
        &mut app,
        "files-discard",
        &["Discard unsaved changes?"],
    );
    let cancel = wait(&mut shot, &mut app, "Button", "Cancel");
    shot.click(&mut app, cancel);
    wait(&mut shot, &mut app, "Tab", "app.ts");

    // A file pops out into a card of its own, and back.
    let readme = wait(&mut shot, &mut app, "TreeItem", "README.md");
    shot.hover(&mut app, readme);
    let pop = wait(
        &mut shot,
        &mut app,
        "Button",
        "Open README.md as its own card",
    );
    shot.click(&mut app, pop);
    wait(&mut shot, &mut app, "MultilineTextInput", "Edit README.md");
    both(&mut shot, &mut app, "files-card", &["README.md"]);
    let back = wait(
        &mut shot,
        &mut app,
        "Button",
        "Put back in its session card",
    );
    shot.click(&mut app, back);
    wait(&mut shot, &mut app, "Tab", "README.md");

    // Typing still settling goes with its own file when the tab changes at once.
    let readme = wait(&mut shot, &mut app, "MultilineTextInput", "Edit README.md");
    shot.click(&mut app, readme);
    shot.type_text(&mut app, "a");
    wait(&mut shot, &mut app, "Button", "Save");
    shot.type_text(&mut app, "late");
    let ts = wait(&mut shot, &mut app, "Tab", "app.ts");
    shot.click(&mut app, ts);
    wait(&mut shot, &mut app, "MultilineTextInput", "Edit app.ts");
    let md = wait(&mut shot, &mut app, "Tab", "README.md");
    shot.click(&mut app, md);
    let readme = wait(&mut shot, &mut app, "MultilineTextInput", "Edit README.md");
    wait_for("the late typing in README.md", || {
        app.pump();
        shot.frame(&mut app);
        shot.ui.edit_text(readme).contains("alate").then_some(())
    });

    // A new app finds the pane, its tabs and the unsaved edit.
    drop(app);
    let mut again = support::connect(&v);
    let mut shot = Shot::new();
    wait(&mut shot, &mut again, "Tab", "app.ts");
    wait(&mut shot, &mut again, "Image", "Unsaved changes");
}

#[test]
fn the_desktops_own_files_state_comes_over_and_its_drafts_go_only_once_all_are_taken() {
    let Some(v) = TestVornd::start() else {
        return;
    };
    let mut app = support::connect(&v);
    let repo = seed_acme(&mut app, &v);
    let sid = agent(&mut app, &repo);
    drop(app);
    let readme = repo.join("README.md").display().to_string();
    let ts = repo.join("src/app.ts").display().to_string();
    let desktop = v.home.join("desktop");
    std::fs::create_dir_all(&desktop).expect("desktop");
    let view = json!({
        "files": { &sid: { "tabs": [&readme], "active": &readme } },
        "cards": { "card:x": { "sessionId": &sid, "filePath": &ts } },
    });
    std::fs::write(desktop.join("view.json"), view.to_string()).expect("view");
    let draft = |p: &str, t: &str| json!({ "filePath": p, "text": t, "base": null });
    let mut drafts = json!({
        format!("files:{sid}:{readme}"): draft(&readme, "# Kept\n"),
        "card:x": draft(&ts, "// kept\n"),
        "card:gone": draft(&ts, "// nowhere\n"),
    });
    std::fs::write(desktop.join("drafts.json"), drafts.to_string()).expect("drafts");

    // A draft with no card to follow keeps the drafts file; the panes and cards are taken.
    let mut app = support::connect(&v);
    let mut shot = Shot::new();
    wait(&mut shot, &mut app, "Tab", "README.md");
    wait(&mut shot, &mut app, "Image", "Unsaved changes");
    wait_for("the view without the desktop's Files state", || {
        app.pump();
        shot.frame(&mut app);
        let text = std::fs::read_to_string(desktop.join("view.json")).ok()?;
        (!text.contains("\"files\"") && !text.contains("\"cards\"")).then_some(())
    });
    assert!(desktop.join("drafts.json").is_file());
    drop(app);

    // Only the draft with no home is kept; once none is left, the file goes.
    let left: Value =
        serde_json::from_str(&std::fs::read_to_string(desktop.join("drafts.json")).expect("left"))
            .expect("json");
    assert_eq!(left.as_object().map(|m| m.len()), Some(1), "{left}");
    assert!(left.get("card:gone").is_some());
    drafts = json!({ format!("files:{sid}:{readme}"): draft(&readme, "# Kept\n") });
    std::fs::write(desktop.join("drafts.json"), drafts.to_string()).expect("drafts");
    let mut app = support::connect(&v);
    let mut shot = Shot::new();
    wait_for("the drafts file gone", || {
        app.pump();
        shot.frame(&mut app);
        (!desktop.join("drafts.json").exists()).then_some(())
    });
}

/// A shell with Vorn's integration on this machine.
fn integrated_shell() -> Option<&'static str> {
    ["/bin/zsh", "/usr/bin/zsh", "/bin/bash", "/usr/bin/bash"]
        .into_iter()
        .find(|p| Path::new(p).is_file())
}

#[test]
fn a_shells_commands_show_as_blocks_with_history_and_completions() {
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
    shot.until(&mut app, "the shell's first prompt", |app, _| {
        (!app.grid()?.lines(&id)?.prompts.is_empty()).then_some(())
    });
    shot.click(&mut app, field);

    shot.type_line(&mut app, "echo vorn-42");
    wait(&mut shot, &mut app, "ListItem", "echo vorn-42");
    shot.type_line(&mut app, "false");
    wait(&mut shot, &mut app, "ListItem", "false");
    // Runs until ⌃C, so a slow machine still finds it running.
    shot.type_line(&mut app, "sleep 30");
    wait(&mut shot, &mut app, "Status", "Running sleep 30");
    std::thread::sleep(Duration::from_millis(400));
    both(
        &mut shot,
        &mut app,
        "blocks-running",
        &["sleep 30", "to interrupt"],
    );
    shot.send(&mut app, key("KeyC", mods::CTRL, Some("c")));
    wait(&mut shot, &mut app, "ListItem", "sleep 30");
    wait_for("the composer back", || {
        app.pump();
        shot.labelled(&mut app, "TextInput", "Type a command")
    });
    both(
        &mut shot,
        &mut app,
        "blocks",
        &["echo vorn-42", "vorn-42", "exit 1", "sleep 30"],
    );

    // ↑ opens the commands run before, newest first.
    let field = wait(&mut shot, &mut app, "TextInput", "Type a command");
    shot.click(&mut app, field);
    shot.send(&mut app, key("ArrowUp", 0, None));
    wait(&mut shot, &mut app, "ListBoxOption", "sleep 30");
    both(&mut shot, &mut app, "blocks-history", &["echo vorn-42"]);
    shot.send(&mut app, key("Escape", 0, None));

    // The token being typed completes.
    shot.type_text(&mut app, "git ch");
    wait(&mut shot, &mut app, "ListBoxOption", "checkout");
    both(&mut shot, &mut app, "blocks-completions", &["echo vorn-42"]);

    // The ghost of a past command offers the rest, and → takes it.
    shot.send(&mut app, key("Escape", 0, None));
    let field = wait(&mut shot, &mut app, "TextInput", "Type a command");
    shot.ui.set_focus(Some(field));
    shot.ui.set_edit_text(field, "");
    shot.type_text(&mut app, "ech");
    // → before the history answers takes the ghost once it arrives.
    shot.send(&mut app, key("ArrowRight", 0, None));
    shot.until(&mut app, "the ghost taken", |_, ui| {
        (ui.edit_text(field) == "echo vorn-42").then_some(())
    });
    // Tab takes the common start of the completions.
    shot.ui.set_edit_text(field, "");
    shot.type_text(&mut app, "git st");
    wait(&mut shot, &mut app, "ListBoxOption", "stash");
    shot.send(&mut app, key("Tab", 0, None));
    assert_eq!(shot.ui.edit_text(field), "git sta");
    shot.ui.set_edit_text(field, "");
    shot.type_text(&mut app, "git chec");
    wait(&mut shot, &mut app, "ListBoxOption", "checkout");
    shot.send(&mut app, key("Tab", 0, None));
    assert_eq!(shot.ui.edit_text(field), "git checkout ");
}

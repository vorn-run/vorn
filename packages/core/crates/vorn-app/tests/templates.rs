//! A real extension (examples/extensions/turns, a Wasm component) with an
//! element tree for every place that takes one, drawn by the one renderer on
//! each surface: the window (Pixels) and the terminal (Cells) at 120x36 and
//! 80x24. The Widget built-in, a component too, draws the same way and
//! answers an agent's real permission requests.
//! `VORN_APP_SHOT=<prefix>` saves each state: a PNG, and the terminal's
//! cells as HTML and text.

#![cfg(unix)]

mod support;

use std::fs;
use std::path::{Path, PathBuf};
use std::sync::Arc;

use serde_json::json;
use support::{add_project, connect, example, key, post_hook, tree_says, Shot, TestVornd};
use vorn_app::act::Act;
use vorn_app::app::MOD;
use vorn_app::plugins::PlugAct;
use vorn_app::screen::menus::MENU;
use vorn_app::screen::settings::CONTENT;
use vorn_app::screen::term_id;
use vorn_app::settings::{Category, SetAct, CONNECTIONS_PAGE};
use vorn_app::state::{Dialog, MenuKind};
use vorn_app::ui::backend::{Backend, Cells};
use vorn_app::ui::{cells::units, item_id, mods, Id, Ui};
use vorn_app::{ui_config, App};

const ID: &str = "example.turns";
const SIZES: [(u16, u16); 2] = [(120, 36), (80, 24)];

/// Links a copy of the example, its component built.
fn link(app: &App, home: &Path) -> PathBuf {
    let dir = example(home, "turns");
    let linked = app
        .rpc()
        .call("extension:link", json!({ "path": dir }))
        .expect("linked");
    assert_eq!(linked["id"], ID, "{linked}");
    dir
}

fn point(kind: &str, id: &str) -> String {
    format!("{ID}/{kind}/{id}")
}

/// This state on a terminal at each size, as text, saved when asked.
fn cells(app: &mut App, name: &str, end: Option<Id>) -> [String; 2] {
    let (chrome, traffic) = (app.chrome, app.traffic);
    (app.chrome, app.traffic) = (false, false);
    let out = SIZES.map(|(cols, rows)| {
        let mut ui = Ui::cells(&ui_config(1.0));
        let size = units::px(cols, rows);
        app.frame(&mut ui, size);
        if let Some(view) = end {
            ui.set_scroll_offset(view, f32::MAX / 4.0);
        }
        app.frame(&mut ui, size);
        let mut target = Cells::offscreen(cols, rows);
        target.present(&mut ui).expect("present");
        let text: Vec<String> = (0..rows).map(|r| target.buffer.row_text(r)).collect();
        let text = text.join("\n");
        if let Some(base) = std::env::var_os("VORN_APP_SHOT") {
            let path = format!("{}-cells-{name}-{cols}x{rows}", base.to_string_lossy());
            let html = target.buffer.to_html(&format!("{name} {cols}x{rows}"));
            fs::write(format!("{path}.html"), html).expect("html");
            fs::write(format!("{path}.txt"), &text).expect("txt");
        }
        text
    });
    (app.chrome, app.traffic) = (chrome, traffic);
    out
}

/// Draws this state on every surface and checks each terminal size shows
/// `wide` at 120x36 and `narrow` at 80x24.
#[track_caller]
fn every_surface(shot: &mut Shot, app: &mut App, name: &str, wide: &[&str], narrow: &[&str]) {
    every_surface_at(shot, app, name, None, wide, narrow);
}

/// `every_surface` with the terminals scrolled to the end of `end`.
#[track_caller]
fn every_surface_at(
    shot: &mut Shot,
    app: &mut App,
    name: &str,
    end: Option<Id>,
    wide: &[&str],
    narrow: &[&str],
) {
    shot.save(app, name);
    let [w, n] = cells(app, name, end);
    for t in wide {
        assert!(w.contains(t), "{name} 120x36 shows {t:?}:\n{w}");
    }
    for t in narrow {
        assert!(n.contains(t), "{name} 80x24 shows {t:?}:\n{n}");
    }
    shot.frame(app);
}

#[track_caller]
fn choose(shot: &mut Shot, app: &mut App, label: &str) {
    let labels = app.menu_labels(&shot.ui);
    let i = labels
        .iter()
        .position(|l| l == label)
        .unwrap_or_else(|| panic!("{label:?} in {labels:?}"));
    shot.click(app, item_id(MENU, i));
}

fn open_card_menu(shot: &mut Shot, app: &mut App, id: &str) {
    shot.hover(app, term_id(id));
    shot.click(
        app,
        App::control("card", &Act::Open(MenuKind::Card(id.to_owned()))),
    );
}

#[test]
fn every_template_kind_draws_on_every_surface() {
    let Some(v) = TestVornd::start() else {
        return;
    };
    let mut app = connect(&v);
    let project = add_project(&mut app, &v, "api", json!({}));
    let dir = link(&app, &v.home);
    let mut shot = Shot::new();
    let all = [
        point("sidebar", "finished"),
        point("badges", "count"),
        point("cardFooters", "last"),
        point("decorations", "line"),
        point("panes", "all"),
        point("composer", "summary"),
        point("settings", "prefs"),
        point("windows", "turns"),
        point("dialogs", "clear"),
        point("commands", "toggle"),
        point("menus", "reset"),
        point("launchers", "window"),
    ];
    shot.until(&mut app, "every contribution", |a, _| {
        all.iter()
            .all(|k| a.st.points.get(k).is_some())
            .then_some(())
    });
    let drawing = all
        .iter()
        .filter(|k| app.st.points.get(k).is_some_and(|p| p.tree))
        .count();
    assert_eq!(drawing, 9, "every place that draws, draws a tree");

    // No session yet: the empty state's launcher.
    every_surface(
        &mut shot,
        &mut app,
        "t-launcher",
        &["Watch turns"],
        &["Watch turns"],
    );

    // An agent (a shell standing in) goes quiet: a turn finished.
    let info = app
        .rpc()
        .call(
            "terminal:create",
            json!({ "agentType": "acme", "projectName": "api", "projectPath": project }),
        )
        .expect("an agent");
    let sid = info["id"].as_str().expect("id").to_owned();
    let (badge, footer, finished) = (
        point("badges", "count"),
        point("cardFooters", "last"),
        point("sidebar", "finished"),
    );
    shot.until(&mut app, "a finished turn", |a, _| {
        let counted = a.st.points.session_tree(&badge, &sid).is_some()
            && a.st.points.session_tree(&footer, &sid).is_some();
        (counted && tree_says(a, &finished, &format!("turn:{sid}-"))).then_some(())
    });
    app.st.selected = Some(sid.clone());
    // First seen without sessions, the project starts folded.
    app.st.collapsed.remove("api");
    every_surface(
        &mut shot,
        &mut app,
        "t-card",
        &[" done", "finished at", "Finished turns", "Turns: "],
        &[" done", "finished at"],
    );

    // The new-session composer: its strip over the prompt, its launcher under it.
    shot.send(&mut app, key("KeyN", MOD(), Some("n")));
    assert!(app.st.composer_open);
    let summary = point("composer", "summary");
    shot.until(&mut app, "the composer's strip", |a, _| {
        tree_says(a, &summary, "Finished turns: ").then_some(())
    });
    every_surface(
        &mut shot,
        &mut app,
        "t-composer",
        &["Finished turns: ", "Watch turns"],
        &["Finished turns: ", "Watch turns"],
    );
    shot.send(&mut app, key("Escape", 0, None));
    shot.frame(&mut app);
    assert!(!app.st.composer_open);

    // The card menu: its template pane, and the extension's item.
    open_card_menu(&mut shot, &mut app, &sid);
    let labels = app.menu_labels(&shot.ui);
    assert!(labels.iter().any(|l| l == "Reset Turn Count"), "{labels:?}");
    choose(&mut shot, &mut app, "Turns");
    let all_pane = point("panes", "all");
    shot.until(&mut app, "the tree pane", |a, _| {
        a.st.ext.panes.get(&sid)?;
        tree_says(a, &all_pane, "At work: 0 of 1").then_some(())
    });
    every_surface(
        &mut shot,
        &mut app,
        "t-pane",
        &["At work: 0 of 1"],
        &["At work: 0 of 1"],
    );
    open_card_menu(&mut shot, &mut app, &sid);
    choose(&mut shot, &mut app, "Close Turns");
    assert!(!app.st.ext.panes.contains_key(&sid));

    // Its settings page, among Vorn's own.
    shot.send(&mut app, key("Comma", MOD(), None));
    assert!(app.st.settings.open);
    let page = point("settings", "prefs");
    shot.click(
        &mut app,
        App::control(
            "settings-nav-ext",
            &Act::Point(vorn_app::points::PointAct::Settings(page.clone())),
        ),
    );
    shot.until(&mut app, "the prefs", |a, _| {
        tree_says(a, &page, "Count shells too").then_some(())
    });
    every_surface(
        &mut shot,
        &mut app,
        "t-settings",
        &[
            "Count shells too",
            "Free plan",
            "Team plan",
            "Paste a key",
            "Sign in",
            "the latest first",
        ],
        &["Count shells too", "Team plan", "Sign in"],
    );
    let team = shot
        .labelled(&mut app, "RadioButton", "Team plan")
        .expect("the plan choice");
    shot.click(&mut app, team);
    shot.until(&mut app, "the plan picked", |a, _| {
        tree_says(
            a,
            &page,
            r#""id":"team","label":"Team plan","selected":true"#,
        )
        .then_some(())
    });
    // A more menu holds the actions a row does not show; one asks before it runs.
    let more = shot.button(&mut app, "More").expect("the more button");
    shot.click(&mut app, more);
    every_surface(
        &mut shot,
        &mut app,
        "t-settings-more",
        &["Forget account"],
        &["Forget account"],
    );
    let forget = shot
        .labelled(&mut app, "MenuItem", "Forget account")
        .expect("the menu's entry");
    shot.click(&mut app, forget);
    every_surface(
        &mut shot,
        &mut app,
        "t-settings-ask",
        &["Forget Turns account?"],
        &["Forget Turns account?"],
    );
    let ok = shot
        .button(&mut app, "Forget account")
        .expect("the confirm's button");
    shot.click(&mut app, ok);
    shot.until(&mut app, "the call's toast", |a, _| {
        a.st.toasts
            .iter()
            .any(|t| t.text == "Forgot Turns account")
            .then_some(())
    });
    // Settings › Extensions: the built-ins, then what is installed.
    shot.click(
        &mut app,
        App::control(
            "settings-nav",
            &Act::Set(SetAct::Open(Category::Connectors)),
        ),
    );
    shot.until(&mut app, "the extensions", |a, _| {
        let p = &a.st.plugins;
        (p.loaded
            && p.loads == 0
            && p.listing_by_id(ID).is_some()
            && !a.st.points.builtins.is_empty())
        .then_some(())
    });
    every_surface(
        &mut shot,
        &mut app,
        "t-extensions",
        &["BUILT IN", "Connectors", "connections"],
        &["BUILT IN", "Connectors", "Turns"],
    );
    every_surface_at(
        &mut shot,
        &mut app,
        "t-extensions-installed",
        Some(CONTENT),
        &["INSTALLED", "Turns", "Linked from"],
        &["INSTALLED", "Turns"],
    );

    // Connectors is one built-in; its row opens Settings › Connections, its own page.
    shot.click(
        &mut app,
        App::control(
            "plug-builtin-row",
            &Act::Set(SetAct::Open(Category::Connections)),
        ),
    );
    assert_eq!(app.st.settings.ext_page.as_deref(), Some(CONNECTIONS_PAGE));
    shot.until(&mut app, "the connections read", |a, _| {
        let tree = a.st.points.trees.get(CONNECTIONS_PAGE)?;
        tree.as_ref().ok().map(|_| ())
    });
    every_surface(
        &mut shot,
        &mut app,
        "t-connections",
        &["Connections", "Add connection", "No connections yet"],
        &["Connections", "Add connection"],
    );
    shot.click(
        &mut app,
        App::control(
            "settings-nav",
            &Act::Set(SetAct::Open(Category::Connectors)),
        ),
    );
    shot.until(&mut app, "the extensions again", |a, _| {
        let p = &a.st.plugins;
        (p.loaded && p.loads == 0 && p.listing_by_id(ID).is_some()).then_some(())
    });

    // Its page: where it shows, what it may use, what it runs.
    let turns = app.st.plugins.listing_by_id(ID).expect("turns").key.clone();
    shot.click(
        &mut app,
        App::control(
            "plug-installed-row",
            &Act::Plug(PlugAct::Select(Some(turns.clone()))),
        ),
    );
    assert_eq!(app.st.plugins.selected.as_deref(), Some(turns.as_str()));
    every_surface(
        &mut shot,
        &mut app,
        "t-extension",
        &["Turns", "13 PLACES", "Sessions"],
        &["Turns", "13 PLACES"],
    );

    shot.click(
        &mut app,
        App::control("plug-detail-back", &Act::Plug(PlugAct::Select(None))),
    );
    // Install… offers Git, a folder's copy, or the folder linked in place.
    let picked = dir.clone();
    app.set_picker(Arc::new(move |_| Some(picked.clone())));
    let install = App::control("plug-install", &Act::Open(MenuKind::ExtInstall));
    shot.click(&mut app, install);
    assert_eq!(
        app.st.menu.as_ref().map(|m| &m.kind),
        Some(&MenuKind::ExtInstall)
    );
    // Menus open in the window's own frame, so this one is a picture and its labels.
    shot.save(&mut app, "t-install-menu");
    assert_eq!(
        app.menu_labels(&shot.ui),
        ["From Git…", "From a folder…", "Link a folder…"]
    );
    // From Git asks for the address.
    choose(&mut shot, &mut app, "From Git…");
    assert_eq!(app.st.dialog, Some(Dialog::ExtInstall));
    shot.type_text(&mut app, "https://github.com/vorn-run/turns");
    every_surface(
        &mut shot,
        &mut app,
        "t-install-git",
        &["Install from Git", "pinned copy", "Continue"],
        &["Install from Git"],
    );
    shot.send(&mut app, key("Escape", 0, None));
    shot.frame(&mut app);
    assert_eq!(app.st.dialog, None);

    // From a folder reads the folder picked and shows what it asks before it runs.
    shot.click(&mut app, install);
    choose(&mut shot, &mut app, "From a folder…");
    shot.until(&mut app, "the sheet", |a, _| {
        a.st.plugins.pending.is_some().then_some(())
    });
    let pending = app.st.plugins.pending.clone().expect("pending");
    assert!(pending.link.is_none());
    every_surface(
        &mut shot,
        &mut app,
        "t-install-sheet",
        &["Install Turns", "Runs", "to install", "Cancel"],
        // A component's build and service take more lines than a program's command.
        &["Install Turns", "a sandboxed component"],
    );
    shot.send(&mut app, key("Escape", 0, None));
    shot.frame(&mut app);
    assert_eq!(app.st.dialog, None);
    assert!(app.st.plugins.pending.is_none());

    // Link a folder reads it where it is, staging nothing.
    shot.click(&mut app, install);
    choose(&mut shot, &mut app, "Link a folder…");
    shot.until(&mut app, "the link sheet", |a, _| {
        a.st.plugins.pending.as_ref().and_then(|p| p.link.clone())
    });
    every_surface(
        &mut shot,
        &mut app,
        "t-link-sheet",
        &["Link Turns", "runs in place", "to link", "Link"],
        &["Link Turns", "a sandboxed component"],
    );
    shot.send(&mut app, key("Escape", 0, None));
    shot.frame(&mut app);
    assert_eq!(app.st.dialog, None);
    shot.click(
        &mut app,
        App::control("settings-nav", &Act::Set(SetAct::Open(Category::Keys))),
    );
    shot.send(&mut app, key("Escape", 0, None));
    shot.frame(&mut app);
    assert!(!app.st.settings.open);

    // The palette reaches Extensions, Install… and each of its choices, and each built-in's switch.
    shot.send(&mut app, key("KeyK", MOD(), None));
    shot.type_text(&mut app, "extension");
    shot.frame(&mut app);
    every_surface(
        &mut shot,
        &mut app,
        "t-palette-extensions",
        &[
            "Open Extensions",
            "Install Extension…",
            "Install Extension from Git…",
            "Install Extension from a Folder…",
            "Link an Extension Folder…",
        ],
        &["Open Extensions"],
    );
    shot.send(&mut app, key("Escape", 0, None));
    shot.frame(&mut app);
    // Install Extension… opens the page with its menu under Install….
    shot.send(&mut app, key("KeyK", MOD(), None));
    shot.type_text(&mut app, "Install Extension…");
    shot.frame(&mut app);
    shot.send(&mut app, key("Enter", 0, Some("\r")));
    shot.until(&mut app, "the install menu", |a, _| {
        (a.st.menu.as_ref().map(|m| &m.kind) == Some(&MenuKind::ExtInstall)).then_some(())
    });
    assert!(app.st.settings.open);
    shot.send(&mut app, key("Escape", 0, None));
    shot.frame(&mut app);
    shot.click(
        &mut app,
        App::control("settings-nav", &Act::Set(SetAct::Open(Category::Keys))),
    );
    shot.send(&mut app, key("Escape", 0, None));
    shot.frame(&mut app);
    assert!(!app.st.settings.open);

    // A palette command that opens its dialog.
    shot.send(&mut app, key("KeyK", MOD(), None));
    shot.type_text(&mut app, "clear finished turns");
    shot.frame(&mut app);
    every_surface(
        &mut shot,
        &mut app,
        "t-palette",
        &["Clear Finished Turns…"],
        &["Clear Finished Turns…"],
    );
    shot.send(&mut app, key("Enter", 0, Some("\r")));
    let dialog = point("dialogs", "clear");
    shot.until(&mut app, "the dialog", |a, _| {
        let open = a.st.points.dialog.as_deref() == Some(dialog.as_str());
        (open && tree_says(a, &dialog, "Clear finished turns?")).then_some(())
    });
    every_surface(
        &mut shot,
        &mut app,
        "t-dialog",
        &["Clear finished turns?"],
        &["Clear finished turns?"],
    );
    let clear = shot.button(&mut app, "Clear").expect("the dialog's Clear");
    shot.click(&mut app, clear);
    shot.until(&mut app, "the list cleared", |a, _| {
        let blank =
            a.st.points
                .trees
                .get(&finished)?
                .as_ref()
                .ok()
                .is_some_and(vorn_tree::is_blank);
        (blank && a.st.points.dialog.is_none()).then_some(())
    });

    // Its key opens its window, which floats in a window without its own.
    let window = point("windows", "turns");
    shot.send(&mut app, key("KeyT", MOD() | mods::ALT, None));
    shot.until(&mut app, "the window", |a, _| {
        let open = a.st.points.windows.contains(&window);
        (open && tree_says(a, &window, "At work: 0 of 1")).then_some(())
    });
    every_surface(
        &mut shot,
        &mut app,
        "t-window",
        &["At work: 0 of 1"],
        &["At work: 0 of 1"],
    );
    shot.send(&mut app, key("KeyT", MOD() | mods::ALT, None));
    shot.frame(&mut app);
    assert!(!app.st.points.windows.contains(&window));

    // The card menu's item calls the program with the card's session.
    open_card_menu(&mut shot, &mut app, &sid);
    choose(&mut shot, &mut app, "Reset Turn Count");
    shot.until(&mut app, "the count reset", |a, _| {
        let reset = a.st.points.trees.contains_key(&badge)
            && a.st.points.session_tree(&badge, &sid).is_none();
        reset.then_some(())
    });

    // Last, as linking again restarts the component: confirming links the folder.
    link_again(&mut shot, &mut app, &turns);
    let _ = dir;
}

/// Install… › Link a folder… on the folder the picker answers, confirmed by its id;
/// its page then offers Unlink.
fn link_again(shot: &mut Shot, app: &mut App, turns: &str) {
    shot.send(app, key("Comma", MOD(), None));
    shot.click(
        app,
        App::control(
            "settings-nav",
            &Act::Set(SetAct::Open(Category::Connectors)),
        ),
    );
    shot.frame(app);
    shot.click(
        app,
        App::control("plug-install", &Act::Open(MenuKind::ExtInstall)),
    );
    choose(shot, app, "Link a folder…");
    shot.until(app, "the link sheet", |a, _| {
        a.st.plugins.pending.as_ref().and_then(|p| p.link.clone())
    });
    shot.type_text(app, ID);
    shot.send(app, key("Enter", 0, Some("\r")));
    shot.until(app, "the folder linked", |a, _| {
        (a.st.plugins.pending.is_none() && a.st.dialog.is_none()).then_some(())
    });
    assert!(
        app.st.plugins.report.is_none(),
        "{:?}",
        app.st.plugins.report
    );
    shot.click(
        app,
        App::control(
            "plug-installed-row",
            &Act::Plug(PlugAct::Select(Some(turns.to_owned()))),
        ),
    );
    let [wide, _] = cells(app, "t-linked", None);
    assert!(wide.contains("Unlink"), "{wide}");
    shot.click(
        app,
        App::control("plug-detail-back", &Act::Plug(PlugAct::Select(None))),
    );
    shot.click(
        app,
        App::control("settings-nav", &Act::Set(SetAct::Open(Category::Keys))),
    );
    shot.send(app, key("Escape", 0, None));
    shot.frame(app);
    assert!(!app.st.settings.open);
}

/// The Widget built-in, a Wasm component: an agent's real permission request
/// with the rules it offers and a question it asks, answered from its window,
/// which folds to a line of counts and back, on every surface.
#[test]
fn the_widget_answers_requests_and_folds_to_a_line_on_every_surface() {
    let Some(v) = TestVornd::start() else {
        return;
    };
    const W: &str = "vorn.widget";
    let mut app = connect(&v);
    let project = add_project(&mut app, &v, "api", json!({}));
    app.rpc()
        .call("extension:setBuiltin", json!({ "id": W, "on": true }))
        .expect("the widget on");
    let (full, compact) = (
        format!("{W}/windows/widget"),
        format!("{W}/windows/compact"),
    );
    let mut shot = Shot::new();
    shot.until(&mut app, "the widget's windows", |a, _| {
        [&full, &compact]
            .iter()
            .all(|k| a.st.points.get(k).is_some_and(|p| p.tree))
            .then_some(())
    });
    let info = app
        .rpc()
        .call(
            "terminal:create",
            json!({ "agentType": "acme", "projectName": "api", "projectPath": project }),
        )
        .expect("an agent");
    let sid = info["id"].as_str().expect("id").to_owned();
    // Each request waits on its hook for the answer the widget gives.
    let ask = |body: serde_json::Value| {
        let home = v.home.clone();
        std::thread::spawn(move || post_hook(&home, &body))
    };
    let rule = |r: &str| json!({ "type": "addRules", "rules": [{ "toolName": "Bash", "ruleContent": r }] });
    let bash = ask(json!({
        "hook_event_name": "PermissionRequest", "session_id": "conv", "cwd": project,
        "tool_name": "Bash", "tool_input": { "command": "npm test" },
        "permission_suggestions": [rule("npm test"), rule("src/**")],
        "vorn_terminal_id": sid,
    }));
    // The question comes second, so the window lists them in that order.
    shot.until(&mut app, "the first request", |a, _| {
        let asked = a.st.points.rows_of(W).get("permissions")?;
        asked.iter().any(|r| r.text("tool") == "Bash").then_some(())
    });
    let question = ask(json!({
        "hook_event_name": "PermissionRequest", "session_id": "conv", "cwd": project,
        "tool_name": "AskUserQuestion",
        "tool_input": { "questions": [{ "header": "Colour", "question": "Which theme?",
                                        "options": [{ "label": "Dark" }, { "label": "Light" }] }] },
        "vorn_terminal_id": sid,
    }));

    shot.send(&mut app, key("KeyW", MOD() | mods::ALT, None));
    let asks = [
        "npm test",
        "Always allow `npm test`",
        "Allow Bash in src/",
        "COLOUR: Which theme?",
        "Light",
    ];
    shot.until(&mut app, "the widget", |a, _| {
        let open = a.st.points.windows.contains(&full);
        (open && asks.iter().all(|t| tree_says(a, &full, t))).then_some(())
    });
    every_surface(&mut shot, &mut app, "w-widget", &asks, &asks[..3]);

    let keep = shot
        .button(&mut app, "Always allow `npm test`")
        .expect("the offered rule");
    shot.click(&mut app, keep);
    let told = bash.join().expect("the hook's answer");
    assert!(told.contains(r#""behavior":"allow""#), "{told}");
    assert!(told.contains("updatedPermissions"), "{told}");
    shot.until(&mut app, "the request gone", |a, _| {
        (!tree_says(a, &full, "npm test")).then_some(())
    });
    let light = shot.button(&mut app, "Light").expect("the option");
    shot.click(&mut app, light);
    shot.until(&mut app, "Submit", |a, _| {
        tree_says(a, &full, "submit:").then_some(())
    });
    let submit = shot.button(&mut app, "Submit").expect("Submit");
    shot.click(&mut app, submit);
    let told = question.join().expect("the hook's answer");
    assert!(told.contains(r#""behavior":"allow""#), "{told}");

    let collapse = shot.button(&mut app, "Collapse").expect("Collapse");
    shot.click(&mut app, collapse);
    shot.until(&mut app, "the line", |a, _| {
        let w = &a.st.points.windows;
        let drawn = a.st.points.trees.get(&compact).is_some_and(|t| t.is_ok());
        (w.contains(&compact) && !w.contains(&full) && drawn).then_some(())
    });
    every_surface(
        &mut shot,
        &mut app,
        "w-compact",
        &["Expand", "Vorn", "1"],
        &["Expand", "Vorn"],
    );

    let expand = shot.button(&mut app, "Expand").expect("Expand");
    shot.click(&mut app, expand);
    shot.until(&mut app, "the widget again", |a, _| {
        let w = &a.st.points.windows;
        (w.contains(&full) && !w.contains(&compact)).then_some(())
    });
    // Show brings the agent's card forward.
    app.st.selected = None;
    let show = shot.button(&mut app, "Show").expect("the agent's Show");
    shot.click(&mut app, show);
    shot.until(&mut app, "the agent shown", |a, _| {
        (a.st.selected.as_deref() == Some(sid.as_str())).then_some(())
    });
    shot.send(&mut app, key("KeyW", MOD() | mods::ALT, None));
    shot.frame(&mut app);
    assert!(!app.st.points.windows.iter().any(|k| k.starts_with(W)));
}

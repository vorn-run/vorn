//! The settings page driven as a user would, on a vornd with its own HOME:
//! every panel opens, and what the controls change lands in vornd's config.
//! `VORN_APP_SHOT=<prefix>` saves a frame of each panel.

#![cfg(unix)]

mod support;

use serde_json::{json, Value};
use support::{key, seed_acme, wait_for, Shot, TestVornd};
use vorn_app::act::Act;
use vorn_app::app::MOD;
use vorn_app::points::PointAct;
use vorn_app::screen::settings::CONTENT;
use vorn_app::settings::{AgentField, Category, SetAct};
use vorn_app::state::MenuKind;
use vorn_app::App;

/// The id of the entry in `tree` titled `title`.
fn entry_titled(tree: &vorn_tree::Node, title: &str) -> Option<String> {
    if let vorn_tree::Kind::Entry(e) = &tree.kind {
        if e.title == title {
            return tree.id.clone();
        }
    }
    tree.children.iter().find_map(|c| entry_titled(c, title))
}

/// Whether `tree` has a node `id`, an entry's children included.
fn has_node(tree: &vorn_tree::Node, id: &str) -> bool {
    tree.id.as_deref() == Some(id) || tree.children.iter().any(|c| has_node(c, id))
}

fn control(place: &str, act: Act) -> vorn_app::ui::Id {
    App::control(place, &act)
}

fn open(shot: &mut Shot, app: &mut App, cat: Category) {
    shot.click(app, control("settings-nav", Act::Set(SetAct::Open(cat))));
    assert_eq!(app.st.settings.category, cat);
}

/// Waits until vornd's config says what `f` looks for.
fn saved(app: &mut App, what: &str, f: impl Fn(&Value) -> bool) {
    wait_for(what, || {
        app.pump();
        let c = app.rpc().call("config:load", Value::Null).ok()?;
        f(&c).then_some(())
    });
}

/// What a cells frame of the app shows, row after row.
fn cells_text(app: &mut App) -> String {
    use vorn_app::ui::backend::{Backend, Cells};
    let mut ui = vorn_app::ui::Ui::cells(&vorn_app::ui_config(1.0));
    let size = vorn_app::ui::cells::units::px(160, 40);
    app.frame(&mut ui, size);
    app.frame(&mut ui, size);
    let mut target = Cells::offscreen(160, 40);
    target.present(&mut ui).expect("present");
    (0..40)
        .map(|r| target.buffer.row_text(r))
        .collect::<Vec<_>>()
        .join("\n")
}

/// Worktree cleanup is a Wasm component; its page draws in a terminal's cells as on pixels.
#[test]
fn the_worktrees_page_a_wasm_component_draws_shows_on_cells() {
    let Some(v) = TestVornd::start() else {
        return;
    };
    let mut app = support::connect(&v);
    seed_acme(&mut app, &v);
    let page = "vorn.worktrees/settings/worktrees";
    app.st.settings.open = true;
    app.st.settings.ext_page = Some(page.to_owned());
    let mut shown = String::new();
    wait_for("the inventory in cells", || {
        shown = cells_text(&mut app);
        let tree = app.st.points.trees.get(page)?.as_ref().ok()?;
        (entry_titled(tree, "feature-greet").is_some() && shown.contains("feature-greet"))
            .then_some(())
    });
    assert!(
        shown.contains("ON DISK") && shown.contains("Removable"),
        "{shown}"
    );
}

#[test]
fn every_settings_panel_opens_and_saves_to_vornd() {
    let Some(v) = TestVornd::start() else {
        return;
    };
    let mut app = support::connect(&v);
    seed_acme(&mut app, &v);
    app.rpc()
        .call("shell:create", json!(v.home.join("acme")))
        .expect("shell");
    let mut shot = Shot::new();
    shot.frame(&mut app);

    shot.click(&mut app, control("sidebar-footer", Act::Settings));
    assert!(app.st.settings.open);
    assert_eq!(app.st.settings.category, Category::Appearance);
    shot.save(&mut app, "set-appearance");

    // Appearance: the font size steps and the layout segment write defaults.
    let before = app
        .store
        .config
        .pointer("/defaults/fontSize")
        .and_then(Value::as_i64)
        .unwrap_or(13);
    shot.click(
        &mut app,
        control("font-size", Act::Set(SetAct::FontSize(1))),
    );
    saved(&mut app, "the larger font", |c| {
        c.pointer("/defaults/fontSize").and_then(Value::as_i64) == Some(before + 1)
    });
    shot.click(
        &mut app,
        control("font-size", Act::Set(SetAct::FontSize(-1))),
    );
    saved(&mut app, "the font back", |c| {
        c.pointer("/defaults/fontSize").and_then(Value::as_i64) == Some(before)
    });
    let tabs = SetAct::Default("layoutMode".into(), "\"tabs\"".into());
    shot.click(&mut app, control("layout-mode", Act::Set(tabs)));
    saved(&mut app, "the tabs layout", |c| {
        c.pointer("/defaults/layoutMode") == Some(&json!("tabs"))
    });
    let grid = SetAct::Default("layoutMode".into(), "\"grid\"".into());
    shot.click(&mut app, control("layout-mode", Act::Set(grid)));

    // General: a switch, a select from its menu, and the shell's typed path.
    open(&mut shot, &mut app, Category::General);
    shot.until(&mut app, "the shells", |a, _| {
        a.st.settings.shells.as_ref().map(|_| ())
    });
    shot.save(&mut app, "set-general");
    let hover = app
        .store
        .config
        .pointer("/defaults/enableHoverPreview")
        .and_then(Value::as_bool)
        .unwrap_or(false);
    let act = SetAct::Default("enableHoverPreview".into(), (!hover).to_string());
    shot.click(&mut app, control("enableHoverPreview", Act::Set(act)));
    saved(&mut app, "the hover preview switch", |c| {
        c.pointer("/defaults/enableHoverPreview")
            .and_then(Value::as_bool)
            == Some(!hover)
    });
    let sel = control(
        "select",
        Act::Open(MenuKind::SetSelect("headlessStepTimeoutMinutes".into())),
    );
    shot.reveal(&mut app, CONTENT, sel);
    shot.click(&mut app, sel);
    let labels = app.menu_labels(&shot.ui);
    assert_eq!(
        labels.last().map(String::as_str),
        Some("No limit"),
        "{labels:?}"
    );
    let i = labels.iter().position(|l| l == "2 hours").expect("2 hours");
    shot.click(
        &mut app,
        vorn_app::ui::item_id(vorn_app::screen::menus::MENU, i),
    );
    saved(&mut app, "the step timeout", |c| {
        c.pointer("/defaults/headlessStepTimeoutMinutes")
            .and_then(Value::as_i64)
            == Some(120)
    });
    let picker = control("shell-picker", Act::Open(MenuKind::SetShell));
    shot.reveal(&mut app, CONTENT, picker);
    shot.click(&mut app, picker);
    assert_eq!(
        app.st.menu.as_ref().map(|m| m.kind.clone()),
        Some(MenuKind::SetShell)
    );
    shot.save(&mut app, "set-shell-menu");
    shot.click(
        &mut app,
        control("shell-option", Act::Set(SetAct::EditShell)),
    );
    assert!(app.st.settings.shell_editing);
    let shell = vorn_app::screen::settings::SHELL_PATH;
    shot.send(&mut app, key("KeyA", MOD(), None));
    shot.type_line(&mut app, "/bin/sh");
    saved(&mut app, "the typed shell", |c| {
        c.pointer("/defaults/shell") == Some(&json!("/bin/sh"))
    });
    assert!(!app.st.settings.shell_editing);
    let _ = shell;

    open(&mut shot, &mut app, Category::Updates);
    shot.save(&mut app, "set-updates");
    shot.click(
        &mut app,
        control(
            "update-channel",
            Act::Set(SetAct::Default("updateChannel".into(), "\"beta\"".into())),
        ),
    );
    saved(&mut app, "the beta channel", |c| {
        c.pointer("/defaults/updateChannel") == Some(&json!("beta"))
    });
    shot.click(
        &mut app,
        control(
            "update-channel",
            Act::Set(SetAct::Default("updateChannel".into(), "\"stable\"".into())),
        ),
    );

    open(&mut shot, &mut app, Category::Notifications);
    shot.save(&mut app, "set-notifications");
    shot.click(
        &mut app,
        control("enabled", Act::Set(SetAct::Notify("enabled".into(), true))),
    );
    saved(&mut app, "notifications on", |c| {
        c.pointer("/defaults/notifications/enabled") == Some(&json!(true))
    });
    shot.click(
        &mut app,
        control("onBell", Act::Set(SetAct::Notify("onBell".into(), false))),
    );
    saved(&mut app, "the bell off", |c| {
        c.pointer("/defaults/notifications/onBell") == Some(&json!(false))
    });
    shot.click(
        &mut app,
        control("onBell", Act::Set(SetAct::Notify("onBell".into(), true))),
    );
    shot.click(
        &mut app,
        control("enabled", Act::Set(SetAct::Notify("enabled".into(), false))),
    );

    // Worktrees is the cleanup extension's page.
    let page = "vorn.worktrees/settings/worktrees";
    shot.until(&mut app, "the extension's page", |a, _| {
        a.st.points.get(page).map(|_| ())
    });
    shot.click(
        &mut app,
        control(
            "settings-nav-ext",
            Act::Point(PointAct::Settings(page.to_owned())),
        ),
    );
    let wt = shot.until(&mut app, "the inventory", |a, _| {
        entry_titled(a.st.points.trees.get(page)?.as_ref().ok()?, "feature-greet")
    });
    shot.save(&mut app, "set-worktrees");
    // The main checkout is not one to clean up, so the page leaves it out.
    let tree = app.st.points.trees[page].as_ref().expect("a tree");
    assert_eq!(entry_titled(tree, "acme"), None, "no main checkout");
    // Remove is destructive, so it waits in the worktree's more menu and asks first.
    let base = vorn_app::ui::Id::new(page).child("tree");
    shot.click(&mut app, base.child("more").child(&wt));
    let remove = base.child(format!("remove:{wt}")).child("");
    shot.click(&mut app, remove);
    shot.save(&mut app, "set-worktrees-confirm");
    shot.click(&mut app, remove.child("ok"));
    shot.until(&mut app, "the worktree gone", |a, _| {
        let tree = a.st.points.trees.get(page)?.as_ref().ok()?;
        entry_titled(tree, "feature-greet").is_none().then_some(())
    });
    assert!(
        !std::path::Path::new(&wt).exists(),
        "the worktree's directory is removed"
    );

    open(&mut shot, &mut app, Category::Agents);
    shot.save(&mut app, "set-agents");
    let first = app.st.installed.clone().unwrap_or_default();
    for k in &first {
        let at = app.st.agent_paths.get(k).map(std::path::Path::new);
        assert!(
            at.is_some_and(|p| p.is_absolute() && p.ends_with(p.file_name().unwrap())),
            "{k} at {at:?}"
        );
    }
    // Re-detect asks the login shell again and answers where each agent is.
    shot.click(
        &mut app,
        control("agents-redetect", Act::Set(SetAct::Redetect)),
    );
    assert!(app.st.settings.detecting);
    shot.until(&mut app, "the re-detection", |app, _| {
        (!app.st.settings.detecting).then_some(())
    });
    assert_eq!(app.st.installed.clone().unwrap_or_default(), first);
    if let Some(agent) = app
        .store
        .agents
        .iter()
        .find(|a| first.contains(&a.key))
        .cloned()
    {
        let id = vorn_app::screen::settings::agent_field_id(&agent.key, AgentField::Args);
        shot.click(&mut app, id);
        shot.type_text(&mut app, "--verbose");
        let k = agent.key;
        saved(&mut app, "the agent's arguments", |c| {
            c.pointer(&format!("/agentCommands/{k}/args")) == Some(&json!(["--verbose"]))
        });
    }

    // SSH & Hosts is the SSH built-in's page, drawn as a tree.
    let page = vorn_app::settings::SSH_PAGE;
    shot.until(&mut app, "the SSH page", |a, _| {
        a.st.points.get(page).map(|_| ())
    });
    // In the sidebar it keeps 0.8's place: under Connections, after Coding Agents.
    let nav = control(
        "settings-nav-ext",
        Act::Point(PointAct::Settings(page.to_owned())),
    );
    shot.frame(&mut app);
    let y = |id| shot.ui.rect_of(id).map(|r| r.y).expect("in the sidebar");
    let agents = y(control(
        "settings-nav",
        Act::Set(SetAct::Open(Category::Agents)),
    ));
    let mcp = y(control(
        "settings-nav",
        Act::Set(SetAct::Open(Category::Mcp)),
    ));
    let ssh = y(nav);
    assert!(agents < ssh && ssh < mcp, "{agents} < {ssh} < {mcp}");
    shot.click(&mut app, nav);
    shot.until(&mut app, "the hosts", |a, _| {
        entry_titled(a.st.points.trees.get(page)?.as_ref().ok()?, "Build box")
    });
    shot.save(&mut app, "set-ssh");
    let base = vorn_app::ui::Id::new(page).child("tree");
    let button = |node: &str| base.child(node).child("");
    // The row itself opens the host, as 0.8's card did.
    shot.click(&mut app, base.child("host:host-1").child("true"));
    let user = base.child("live").child("user:host-1");
    shot.until(&mut app, "the host's form", |a, _| {
        let tree = a.st.points.trees.get(page)?.as_ref().ok()?;
        has_node(tree, "user:host-1").then_some(())
    });
    shot.save(&mut app, "set-ssh-host");
    shot.click(&mut app, user);
    shot.send(&mut app, key("KeyA", MOD(), None));
    shot.type_text(&mut app, "deploy");
    saved(&mut app, "the host's user", |c| {
        c.pointer("/remoteHosts/0/user") == Some(&json!("deploy"))
    });
    // A typed password saves when the field is left, as in 0.8.
    shot.click(&mut app, base.child("auth:host-1").child("password"));
    let password = base.child("field").child("password:host-1");
    shot.until(&mut app, "the password field", |_, ui| ui.rect_of(password));
    shot.click(&mut app, password);
    shot.type_text(&mut app, "hunter2");
    shot.click(&mut app, user);
    saved(&mut app, "the host's password", |c| {
        c.pointer("/remoteHosts/0/encryptedPassword")
            .and_then(Value::as_str)
            .is_some_and(|p| !p.is_empty())
    });
    shot.click(&mut app, base.child("auth:host-1").child("key-file"));
    saved(&mut app, "the key-file method", |c| {
        c.pointer("/remoteHosts/0/authMethod") == Some(&json!("key-file"))
    });
    // Browse asks the OS for the key file.
    app.set_picker(std::sync::Arc::new(|_| Some("/keys/id_build".into())));
    let browse = button("browse-key:host-1");
    shot.until(&mut app, "the browse button", |_, ui| ui.rect_of(browse));
    shot.click(&mut app, browse);
    shot.until(&mut app, "the picked key path", |a, _| {
        let c = a.rpc().call("config:load", Value::Null).ok()?;
        (c.pointer("/remoteHosts/0/sshKeyPath") == Some(&json!("/keys/id_build"))).then_some(())
    });
    shot.reveal(&mut app, CONTENT, button("paste"));
    shot.click(&mut app, button("paste"));
    let label = base.child("live").child("new-key:label");
    shot.until(&mut app, "the paste form", |a, _| {
        let tree = a.st.points.trees.get(page)?.as_ref().ok()?;
        has_node(tree, "new-key:label").then_some(())
    });
    shot.save(&mut app, "set-ssh-paste");
    shot.reveal(&mut app, CONTENT, label);
    shot.click(&mut app, label);
    shot.type_text(&mut app, "Work");
    let private = base.child("live").child("new-key:private");
    shot.click(&mut app, private);
    shot.type_text(&mut app, "PRIVATE");
    shot.reveal(&mut app, CONTENT, button("save-key"));
    shot.click(&mut app, button("save-key"));
    shot.until(&mut app, "the stored key", |a, _| {
        entry_titled(a.st.points.trees.get(page)?.as_ref().ok()?, "Work")
    });
    shot.reveal(&mut app, CONTENT, button("add-host"));
    shot.click(&mut app, button("add-host"));
    saved(&mut app, "a second host", |c| {
        c["remoteHosts"].as_array().map_or(0, Vec::len) == 2
    });

    open(&mut shot, &mut app, Category::Mcp);
    shot.save(&mut app, "set-mcp");
    let cmd = "/plugin install vorn".to_owned();
    shot.click(
        &mut app,
        control("mcp-copy", Act::Set(SetAct::Copy(cmd.clone()))),
    );
    assert_eq!(shot.ui.clipboard().get(), Some(cmd));

    open(&mut shot, &mut app, Category::Keys);
    shot.until(&mut app, "the keys", |a, _| {
        a.st.settings.keys.as_ref().map(|_| ())
    });
    shot.save(&mut app, "set-keys");

    open(&mut shot, &mut app, Category::About);
    shot.until(&mut app, "the core status", |a, _| {
        a.st.settings.core_version.as_ref().map(|_| ())
    });
    shot.until(&mut app, "the holders", |a, _| {
        a.st.settings.holders.as_ref().map(|_| ())
    });
    shot.save(&mut app, "set-about");

    shot.send(&mut app, key("Escape", 0, None));
    assert!(!app.st.settings.open, "Escape closes the page");
    shot.send(&mut app, key("Comma", MOD(), None));
    assert!(app.st.settings.open, "Mod+Comma opens it again");
}

/// Off, SSH takes its page, its palette entry and the project dialog's
/// host picker with it; the hosts stay in the config.
#[test]
fn switching_ssh_off_removes_its_surfaces() {
    let Some(v) = TestVornd::start() else {
        return;
    };
    let mut app = support::connect(&v);
    seed_acme(&mut app, &v);
    let mut shot = Shot::new();
    let page = vorn_app::settings::SSH_PAGE;
    let in_palette = |app: &App| {
        vorn_app::palette::commands(&app.store, &app.st, "Cmd", "Alt")
            .iter()
            .any(|e| e.label == "SSH & Hosts")
    };
    shot.until(&mut app, "the SSH page", |a, _| {
        a.st.points.get(page).map(|_| ())
    });
    assert!(in_palette(&app));
    shot.click(&mut app, control("sidebar", Act::AddProject));
    assert!(
        shot.labelled(&mut app, "Button", "Build box").is_some(),
        "the host picker offers the remote host"
    );
    app.st.dialog = None;

    app.rpc()
        .call(
            "extension:setBuiltin",
            json!({ "id": "vorn.ssh", "on": false }),
        )
        .expect("switched off");
    shot.until(&mut app, "the SSH page gone", |a, _| {
        a.st.points.get(page).is_none().then_some(())
    });
    assert!(!in_palette(&app));
    shot.click(&mut app, control("sidebar", Act::AddProject));
    assert!(
        shot.labelled(&mut app, "Button", "Build box").is_none(),
        "no host picker"
    );
    let off = app
        .rpc()
        .call(
            "ssh:testConnection",
            json!({ "hostname": "h", "user": "u" }),
        )
        .expect_err("refused");
    assert!(
        off.to_string()
            .contains("SSH is off. Turn it on in Settings"),
        "{off}"
    );
    let config = app.rpc().call("config:load", Value::Null).expect("config");
    assert_eq!(config["remoteHosts"][0]["id"], "host-1", "the hosts stay");
}

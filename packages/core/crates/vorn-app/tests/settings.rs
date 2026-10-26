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
    let large = SetAct::Default("fontSize".into(), "16".into());
    shot.click(&mut app, control("font-preset", Act::Set(large)));
    saved(&mut app, "the larger font", |c| {
        c.pointer("/defaults/fontSize").and_then(Value::as_i64) == Some(16)
    });
    let medium = SetAct::Default("fontSize".into(), "13".into());
    shot.click(&mut app, control("font-preset", Act::Set(medium)));
    saved(&mut app, "the font back", |c| {
        c.pointer("/defaults/fontSize").and_then(Value::as_i64) == Some(13)
    });
    // The steppers reach any whole size between the presets.
    shot.click(
        &mut app,
        control("font-size", Act::Set(SetAct::FontSize(1))),
    );
    saved(&mut app, "one size larger", |c| {
        c.pointer("/defaults/fontSize").and_then(Value::as_i64) == Some(14)
    });
    shot.save(&mut app, "set-font-stepped");
    shot.click(
        &mut app,
        control("font-size", Act::Set(SetAct::FontSize(-1))),
    );
    saved(&mut app, "one size smaller", |c| {
        c.pointer("/defaults/fontSize").and_then(Value::as_i64) == Some(13)
    });
    let tabs = SetAct::Default("layoutMode".into(), "\"tabs\"".into());
    shot.click(&mut app, control("layout", Act::Set(tabs)));
    saved(&mut app, "the tabs layout", |c| {
        c.pointer("/defaults/layoutMode") == Some(&json!("tabs"))
    });
    let grid = SetAct::Default("layoutMode".into(), "\"grid\"".into());
    shot.click(&mut app, control("layout", Act::Set(grid)));

    // The theme picker's tiles and the Look toggles are kept by vornd and
    // read back; a light theme picked under a fixed dark scheme fixes light.
    use vorn_app::appearance as look;
    use vorn_app::screen::settings::theme_tile_id;
    let dark = vorn_app::appearance::prefs(app.store.config.get("defaults")).scheme;
    assert_eq!(
        dark,
        vorn_theme::Scheme::Dark,
        "Vorn opens on a fixed dark scheme"
    );
    let picker = Act::Open(MenuKind::ThemePicker(vorn_theme::Appearance::Dark));
    shot.click(&mut app, control("theme-chip", picker.clone()));
    shot.save(&mut app, "set-theme-picker");
    shot.click(&mut app, theme_tile_id("nord"));
    shot.click(&mut app, control("theme-chip", picker));
    let light = SetAct::ThemeFilter(look::ThemeFilter::Light);
    shot.click(&mut app, control("theme-filter", Act::Set(light)));
    shot.click(&mut app, theme_tile_id("solarized-light"));
    let tinted = SetAct::Default(look::CHROME.into(), "\"tinted\"".into());
    shot.click(&mut app, control(look::CHROME, Act::Set(tinted)));
    saved(&mut app, "the chosen themes and style", |c| {
        c.pointer("/defaults/darkTheme") == Some(&json!("nord"))
            && c.pointer("/defaults/lightTheme") == Some(&json!("solarized-light"))
            && c.pointer("/defaults/interfaceStyle") == Some(&json!("tinted"))
            && c.pointer("/defaults/colorScheme") == Some(&json!("light"))
    });
    wait_for("the app's prefs after vornd's echo", || {
        app.pump();
        let p = look::prefs(app.store.config.get("defaults"));
        (p.dark == "nord" && p.light == "solarized-light" && p.chrome == vorn_theme::Chrome::Tinted)
            .then_some(())
    });

    // General: a switch, the agent picker, and the shell's typed path.
    open(&mut shot, &mut app, Category::General);
    shot.until(&mut app, "the shells", |a, _| {
        a.st.settings.shells.as_ref().map(|_| ())
    });
    shot.save(&mut app, "set-general");
    let reopen = app
        .store
        .config
        .pointer("/defaults/reopenSessions")
        .and_then(Value::as_bool)
        .unwrap_or(true);
    let act = SetAct::Default("reopenSessions".into(), (!reopen).to_string());
    shot.click(&mut app, control("reopenSessions", Act::Set(act)));
    saved(&mut app, "the reopen switch", |c| {
        c.pointer("/defaults/reopenSessions")
            .and_then(Value::as_bool)
            == Some(!reopen)
    });
    // The picker lists ready agents only; CI machines have none on PATH.
    app.st.installed = Some(app.store.agents.iter().map(|a| a.key.clone()).collect());
    let chip = control("agent-chip", Act::Open(MenuKind::AgentPicker));
    shot.click(&mut app, chip);
    assert_eq!(
        app.st.menu.as_ref().map(|m| m.kind.clone()),
        Some(MenuKind::AgentPicker)
    );
    shot.save(&mut app, "set-agent-picker");
    let other = (&app.store.agents)
        .into_iter()
        .map(|a| a.key.clone())
        .nth(1)
        .expect("a second agent");
    let pick = SetAct::Default("defaultAgent".into(), format!("\"{other}\""));
    shot.click(&mut app, control("agent-option", Act::Set(pick)));
    saved(&mut app, "the default agent", |c| {
        c.pointer("/defaults/defaultAgent") == Some(&json!(other))
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

    open(&mut shot, &mut app, Category::Notifications);
    shot.save(&mut app, "set-notifications");
    let notify =
        |place, key: &str, v: &str| control(place, Act::Set(SetAct::Notify(key.into(), v.into())));
    // Notifications are on until switched off, and each control writes one key.
    shot.click(&mut app, notify("notif-enabled", "enabled", "false"));
    saved(&mut app, "notifications off", |c| {
        c.pointer("/defaults/notifications/enabled") == Some(&json!(false))
    });
    shot.click(&mut app, notify("notif-enabled", "enabled", "true"));
    shot.click(&mut app, notify("notif-bell", "onBell", "false"));
    saved(&mut app, "the bell off", |c| {
        c.pointer("/defaults/notifications/onBell") == Some(&json!(false))
            && c.pointer("/defaults/notifications/enabled") == Some(&json!(true))
    });
    shot.click(&mut app, notify("notif-show", "show", "\"always\""));
    saved(&mut app, "shown always", |c| {
        c.pointer("/defaults/notifications/show") == Some(&json!("always"))
            && c.pointer("/defaults/notifications/onBell") == Some(&json!(false))
    });
    shot.click(&mut app, notify("notif-bell", "onBell", "true"));

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
    shot.until(&mut app, "the detection", |app, _| {
        (!app.st.settings.detecting).then_some(())
    });
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
        // A row opens the agent's page; its arguments wait under Advanced.
        let open = Act::Set(SetAct::OpenAgent(Some(agent.key.clone())));
        shot.click(&mut app, control(&format!("agent-row:{}", agent.key), open));
        assert_eq!(app.st.settings.agent.as_deref(), Some(agent.key.as_str()));
        shot.save(&mut app, "set-agent");
        shot.click(
            &mut app,
            control(
                "agent-advanced",
                Act::Set(SetAct::ExpandAgent(agent.key.clone())),
            ),
        );
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
    let remote = y(control(
        "settings-nav",
        Act::Set(SetAct::Open(Category::Network)),
    ));
    let ssh = y(nav);
    assert!(agents < ssh && ssh < remote, "{agents} < {ssh} < {remote}");
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

    open(&mut shot, &mut app, Category::About);
    shot.until(&mut app, "the holders", |a, _| {
        a.st.settings.holders.as_ref().map(|_| ())
    });
    shot.save(&mut app, "set-about");
    if !vorn_app::settings::IN_BROWSER {
        shot.click(
            &mut app,
            control("update-channel", Act::Open(MenuKind::UpdateChannel)),
        );
        assert_eq!(
            app.st.menu.as_ref().map(|m| m.kind.clone()),
            Some(MenuKind::UpdateChannel)
        );
        let labels = app.menu_labels(&shot.ui);
        let beta = labels.iter().position(|l| l == "Beta").expect("Beta");
        shot.click(
            &mut app,
            vorn_app::ui::item_id(vorn_app::screen::menus::MENU, beta),
        );
        saved(&mut app, "the beta channel", |c| {
            c.pointer("/defaults/updateChannel") == Some(&json!("beta"))
        });
    }

    shot.send(&mut app, key("Escape", 0, None));
    assert!(!app.st.settings.open, "Escape closes the page");
    shot.send(&mut app, key("Comma", MOD(), None));
    assert!(app.st.settings.open, "Mod+Comma opens it again");
}

/// Off, SSH takes its page, its palette entry and the project dialog's
/// host picker with it; the hosts stay in the config.
#[test]
fn uninstalling_ssh_removes_its_surfaces() {
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
        .call("extension:remove", json!({ "id": "vorn.ssh" }))
        .expect("uninstalled");
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
            .contains("SSH is not installed. Install it in Settings"),
        "{off}"
    );
    let config = app.rpc().call("config:load", Value::Null).expect("config");
    assert_eq!(config["remoteHosts"][0]["id"], "host-1", "the hosts stay");
}

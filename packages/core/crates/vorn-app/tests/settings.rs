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
use vorn_app::screen::settings::{host_id, key_field_id, CONTENT};
use vorn_app::settings::{AgentField, Category, HostField, KeyField, SetAct};
use vorn_app::state::MenuKind;
use vorn_app::App;

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
        a.st.points
            .rows_of("vorn.worktrees")
            .get("worktrees")?
            .iter()
            .find(|r| r.fields.get("name").and_then(|n| n.as_str()) == Some("feature-greet"))
            .map(|r| r.id.clone())
    });
    shot.save(&mut app, "set-worktrees");
    let remove = shot
        .row_button(&mut app, page, &wt, "Remove")
        .expect("the worktree's Remove");
    shot.click(&mut app, remove);
    shot.save(&mut app, "set-worktrees-confirm");
    shot.click(&mut app, remove.child("ok"));
    shot.until(&mut app, "the worktree gone", |a, _| {
        let rows = a.st.points.rows_of("vorn.worktrees").get("worktrees")?;
        (!rows.iter().any(|r| r.id == wt)).then_some(())
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
        .find(|a| first.iter().any(|k| *k == a.key))
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

    open(&mut shot, &mut app, Category::Ssh);
    shot.save(&mut app, "set-ssh");
    shot.click(
        &mut app,
        control(
            "ssh-host-row",
            Act::Set(SetAct::ToggleHost("host-1".into())),
        ),
    );
    shot.save(&mut app, "set-ssh-host");
    shot.click(&mut app, host_id("host-1", HostField::User));
    shot.send(&mut app, key("KeyA", MOD(), None));
    shot.type_text(&mut app, "deploy");
    saved(&mut app, "the host's user", |c| {
        c.pointer("/remoteHosts/0/user") == Some(&json!("deploy"))
    });
    shot.click(
        &mut app,
        control(
            "ssh-auth",
            Act::Set(SetAct::Auth("host-1".into(), "key-file".into())),
        ),
    );
    saved(&mut app, "the key-file method", |c| {
        c.pointer("/remoteHosts/0/authMethod") == Some(&json!("key-file"))
    });
    shot.click(&mut app, control("ssh-paste", Act::Set(SetAct::PasteKey)));
    shot.save(&mut app, "set-ssh-paste");
    shot.click(&mut app, key_field_id(KeyField::Label));
    shot.type_text(&mut app, "Work");
    shot.click(&mut app, key_field_id(KeyField::Private));
    shot.type_text(&mut app, "PRIVATE");
    shot.click(&mut app, control("ssh-key-save", Act::Set(SetAct::SaveKey)));
    shot.until(&mut app, "the stored key", |a, _| {
        a.st.settings
            .ssh
            .keys
            .iter()
            .find(|k| k.label == "Work")
            .map(|_| ())
    });
    shot.click(&mut app, control("ssh-add-host", Act::Set(SetAct::AddHost)));
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

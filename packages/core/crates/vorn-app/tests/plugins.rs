//! Settings › Extensions and Connections on a vornd with its own HOME: the
//! tabs, a listing's detail, the add forms and a connection made from one.
//! `VORN_APP_SHOT=<prefix>` saves a frame of each.

#![cfg(unix)]

mod support;

use serde_json::json;
use support::{add_project, connect, key, wait_for, Shot, TestVornd};
use vorn_app::act::Act;
use vorn_app::app::MOD;
use vorn_app::plugins::{field_id, tabs_id, Field, Form, PlugAct, Tab};
use vorn_app::points::PointAct;
use vorn_app::settings::{Category, SetAct, CONNECTIONS_PAGE};
use vorn_app::templates::{self, Fix};
use vorn_app::ui::mods;
use vorn_app::workflows::{Panel, WfAct};
use vorn_app::App;

fn plug(place: &str, act: PlugAct) -> vorn_app::ui::Id {
    App::control(place, &Act::Plug(act))
}

/// The Installed or Browse pill.
fn tab(t: Tab) -> vorn_app::ui::Id {
    let i = Tab::ALL.iter().position(|x| *x == t).unwrap_or(0);
    vorn_app::ui::widgets::item_id(tabs_id(), i)
}

fn loaded(app: &App) -> bool {
    let p = &app.st.plugins;
    p.loaded && p.loads == 0
}

#[test]
fn plugins_page_lists_adds_and_connects() {
    let Some(v) = TestVornd::start() else {
        return;
    };
    let mut app = connect(&v);
    add_project(&mut app, &v, "api", json!({}));
    // The test agent's adapter is a linked package: gone, nothing is installed.
    app.rpc()
        .call("extension:remove", json!({ "id": "test.acme" }))
        .expect("removed");
    let mut shot = Shot::new();

    // Opened from the settings nav, the page loads itself.
    app.st.settings.open = true;
    shot.click(
        &mut app,
        App::control(
            "settings-nav",
            &Act::Set(SetAct::Open(Category::Connectors)),
        ),
    );
    shot.until(&mut app, "the plugins page loaded", |app, _| {
        loaded(app).then_some(())
    });
    assert!(app.st.shown(Category::Connectors));
    // It opens on what is installed, the built-ins first.
    assert_eq!(app.st.plugins.tab, Tab::Installed);
    assert!(app.st.plugins.listing("http").is_some(), "HTTP is built in");
    shot.save(&mut app, "pl-installed");
    shot.click(&mut app, tab(Tab::Browse));
    shot.save(&mut app, "pl-browse");

    // Search narrows the list.
    shot.click(&mut app, field_id(&Field::Query));
    shot.type_text(&mut app, "zzzz-none");
    shot.frame(&mut app);
    assert!(app.st.plugins.filtered().is_empty());
    shot.save(&mut app, "pl-browse-empty");
    for _ in 0.."zzzz-none".len() {
        shot.send(&mut app, key("Backspace", 0, None));
    }
    shot.frame(&mut app);
    assert!(!app.st.plugins.filtered().is_empty());

    // Connections is the Connectors built-in's page, listed under Extensions.
    shot.click(
        &mut app,
        App::control(
            "settings-nav-ext",
            &Act::Point(PointAct::Settings(CONNECTIONS_PAGE.into())),
        ),
    );
    assert_eq!(app.st.settings.ext_page.as_deref(), Some(CONNECTIONS_PAGE));
    shot.save(&mut app, "pl-connections");
    shot.click(
        &mut app,
        App::control(
            "settings-nav",
            &Act::Set(SetAct::Open(Category::Connectors)),
        ),
    );
    shot.until(&mut app, "the page reloaded", |app, _| {
        loaded(app).then_some(())
    });

    // A listing's detail, from Browse.
    shot.click(&mut app, tab(Tab::Browse));
    shot.click(
        &mut app,
        plug("plug-browse-row", PlugAct::Select(Some("http".into()))),
    );
    assert_eq!(app.st.plugins.selected.as_deref(), Some("http"));
    shot.save(&mut app, "pl-detail");

    // The add form for a built-in, and the connection it makes.
    shot.click(&mut app, plug("plug-detail", PlugAct::Add("http".into())));
    let form = app.st.plugins.adding.as_ref().map(|a| a.form.clone());
    assert!(matches!(form, Some(Form::Builtin(_))), "{form:?}");
    shot.save(&mut app, "pl-add-http");
    let needs = app
        .st
        .plugins
        .builtin("http")
        .and_then(|b| b.pointer("/manifest/auth"))
        .and_then(|a| a.as_array())
        .cloned()
        .unwrap_or_default();
    for f in needs {
        let Some(key) = f.get("key").and_then(|k| k.as_str()) else {
            continue;
        };
        if f.get("type").and_then(|t| t.as_str()) == Some("select") {
            continue;
        }
        let id = field_id(&Field::Auth(key.to_owned()));
        shot.click(&mut app, id);
        let value = match key {
            "baseUrl" => "https://api.example.com",
            "profileName" => "Example API",
            "secret" => "sk-example",
            _ => continue,
        };
        shot.type_text(&mut app, value);
    }
    shot.save(&mut app, "pl-add-http-filled");
    shot.click(&mut app, plug("plug-save", PlugAct::Save));
    shot.until(&mut app, "the connection made", |app, _| {
        (app.st.plugins.adding.is_none()
            && app.st.settings.ext_page.as_deref() == Some(CONNECTIONS_PAGE)
            && !app.st.plugins.connections.is_empty())
        .then_some(())
    });
    shot.until(&mut app, "the list reloaded", |app, _| {
        loaded(app).then_some(())
    });
    shot.save(&mut app, "pl-connected");

    // The key lives on the connection's row: its menu tests and rotates it.
    let more = shot
        .labelled(&mut app, "Button", "More")
        .expect("the connection's menu");
    shot.click(&mut app, more);
    shot.save(&mut app, "pl-key-menu");
    let rotate = shot
        .labelled(&mut app, "MenuItem", "Rotate key\u{2026}")
        .expect("Rotate key in the connection's menu");
    shot.click(&mut app, rotate);
    wait_for("the new key's field", || {
        shot.frame(&mut app);
        shot.labelled(&mut app, "TextInput", "New Secret")
    });
    shot.save(&mut app, "pl-rotating");

    // Leaving the page forgets the form state.
    shot.click(
        &mut app,
        App::control("settings-nav", &Act::Set(SetAct::Open(Category::Network))),
    );
    assert!(!app.st.shown(Category::Connectors));
    assert!(app.st.plugins.adding.is_none());
}

/// The first unmet requirement among the templates whose fix is `wanted`.
fn template_fix(app: &App, wanted: fn(&Fix) -> bool) -> Option<(String, Fix)> {
    let wf = &app.st.wf;
    let l = templates::Listings {
        connectors: &wf.lib.connectors,
        catalog: &wf.lib.catalog,
        packs: &wf.lib.packs,
    };
    wf.lib.templates.iter().find_map(|t| {
        templates::of_template(t, &wf.connections)
            .into_iter()
            .filter(|n| n.bound.is_none())
            .find_map(|n| {
                let fix = templates::fix(&n, l);
                let node = n.req["nodeId"].as_str().unwrap_or("").to_owned();
                wanted(&fix).then_some((node, fix))
            })
    })
}

#[test]
fn a_template_requirement_is_fixed_in_the_start_from_panel() {
    let Some(v) = TestVornd::start() else {
        return;
    };
    let mut app = connect(&v);
    add_project(&mut app, &v, "web", json!({}));
    let mut shot = Shot::new();
    support::views_listed(&mut shot, &mut app);
    shot.send(&mut app, key("KeyW", MOD() | mods::SHIFT, None));
    shot.click(
        &mut app,
        App::control("sidebar", &Act::Workflow(WfAct::New)),
    );
    assert_eq!(app.st.wf.panel, Panel::StartFrom);
    let found = shot.until(&mut app, "a template asking for a profile", |app, _| {
        template_fix(app, |f| matches!(f, Fix::CreateProfile(_)))
    });
    let (node, fix) = found;
    shot.click(
        &mut app,
        App::control(&format!("wf-fix.{node}"), &Act::Workflow(WfAct::Fix(fix))),
    );
    // The form opens in the panel; Settings stays shut.
    shot.until(&mut app, "the profile form", |app, _| {
        matches!(
            app.st.plugins.adding.as_ref().map(|a| &a.form),
            Some(Form::Profile(_))
        )
        .then_some(())
    });
    assert!(!app.st.settings.open);
    assert_eq!(app.st.wf.panel, Panel::StartFrom);
    shot.save(&mut app, "wf-fix-profile");

    shot.click(&mut app, field_id(&Field::ProfileUrl));
    shot.type_text(&mut app, "https://127.0.0.1:9");
    shot.click(&mut app, plug("plug-save", PlugAct::Save));
    // Nothing answers there, so the form stays open saying why.
    shot.until(&mut app, "the profile's test", |app, _| {
        match app.st.plugins.adding.as_ref().map(|a| &a.form) {
            Some(Form::Profile(f)) => f.result.clone().map(|_| ()),
            None => Some(()),
            _ => None,
        }
    });
    shot.save(&mut app, "wf-fix-tested");
    // The profile exists even though it did not answer, so a retry updates it.
    let made = match app.st.plugins.adding.as_ref().map(|a| &a.form) {
        Some(Form::Profile(f)) => f.made.clone(),
        _ => None,
    };
    let list = app
        .rpc()
        .call("connection:list", json!({}))
        .expect("connections");
    let made = made.expect("the first save made the profile");
    assert!(list
        .as_array()
        .is_some_and(|l| l.iter().any(|c| c["id"] == made.as_str())));

    // Closing the panel puts the form away.
    shot.click(
        &mut app,
        App::control("wf-start-blank", &Act::Workflow(WfAct::StartBlank)),
    );
    shot.frame(&mut app);
    assert!(app.st.plugins.adding.is_none());
    assert!(!app.st.plugins.from_editor);
}

#[test]
fn the_sign_in_sheet_opens_checks_and_closes() {
    let Some(v) = TestVornd::start() else {
        return;
    };
    let mut app = connect(&v);
    let mut shot = Shot::new();
    let target = json!({ "name": "Example", "browser": {
        "signInUrl": "https://example.com/login",
        "origins": ["https://example.com"],
        "check": { "url": "https://example.com/me", "identity": ["login"] } } });
    assert!(app
        .open_signin("c1", &target, vorn_app::signin::From::Plugins)
        .is_none());
    assert_eq!(app.st.dialog, Some(vorn_app::state::Dialog::SignIn));
    // Offscreen there is no page to sign in on; the check fails quietly and waits.
    shot.frame(&mut app);
    shot.save(&mut app, "signin-sheet");
    shot.click(&mut app, App::control("signin-close", &Act::SignInClose));
    assert!(app.st.signin.is_none());
    assert_eq!(app.st.dialog, None);
    assert!(matches!(
        app.st.plugins.activity.get("c1"),
        Some(vorn_app::plugins::Activity::Failed(e)) if e == vorn_app::signin::CLOSED
    ));
    let bad = json!({ "name": "None" });
    assert_eq!(
        app.open_signin("c2", &bad, vorn_app::signin::From::Run)
            .as_deref(),
        Some("This connection does not sign in through a Vorn window.")
    );
}

#[test]
fn connectors_switch_off_and_on_from_the_built_ins_list() {
    let Some(v) = TestVornd::start() else {
        return;
    };
    let mut app = connect(&v);
    let mut shot = Shot::new();
    app.st.settings.open = true;
    shot.click(
        &mut app,
        App::control(
            "settings-nav",
            &Act::Set(SetAct::Open(Category::Connectors)),
        ),
    );
    shot.until(&mut app, "the built-ins listed", |app, _| {
        (loaded(app) && !app.st.points.builtins.is_empty()).then_some(())
    });
    let on = |app: &App| vorn_app::plugins::connectors_on(&app.st.points.builtins);
    assert!(on(&app));
    let switch = |on: bool| {
        App::control(
            "ext-builtin",
            &Act::Point(vorn_app::points::PointAct::Builtin {
                id: vorn_app::plugins::CONNECTORS.into(),
                on,
            }),
        )
    };
    shot.click(&mut app, tab(Tab::Installed));
    shot.until(&mut app, "Connections listed", |app, _| {
        app.st.points.get(CONNECTIONS_PAGE).map(|_| ())
    });
    shot.click(&mut app, switch(false));
    shot.until(&mut app, "Connectors switched off", |app, _| {
        (!on(app) && app.st.points.get(CONNECTIONS_PAGE).is_none()).then_some(())
    });
    assert_eq!(vorn_app::plugins::tabs(on(&app)), [Tab::Installed]);
    shot.save(&mut app, "pl-connectors-off");
    shot.click(&mut app, switch(true));
    shot.until(&mut app, "Connectors switched on", |app, _| {
        (on(app) && app.st.points.get(CONNECTIONS_PAGE).is_some()).then_some(())
    });
}

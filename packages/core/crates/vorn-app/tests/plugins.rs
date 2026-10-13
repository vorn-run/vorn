//! Settings › Plugins on a vornd with its own HOME: the tabs, a listing's
//! detail, the add forms and a connection made from one.
//! `VORN_APP_SHOT=<prefix>` saves a frame of each.

#![cfg(unix)]

mod support;

use serde_json::json;
use support::{add_project, connect, key, Shot, TestVornd};
use vorn_app::act::Act;
use vorn_app::app::MOD;
use vorn_app::plugins::{field_id, Field, Form, PlugAct, Tab};
use vorn_app::settings::{Category, SetAct};
use vorn_app::templates::{self, Fix};
use vorn_app::ui::mods;
use vorn_app::workflows::{Panel, WfAct};
use vorn_app::App;

fn plug(place: &str, act: PlugAct) -> vorn_app::ui::Id {
    App::control(place, &Act::Plug(act))
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
    // Nothing installed yet, so it opens on Browse, as the renderer does.
    assert_eq!(app.st.plugins.tab, Tab::Browse);
    assert!(app.st.plugins.listing("http").is_some(), "HTTP is built in");
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

    shot.click(&mut app, plug("plug-tab", PlugAct::Tab(Tab::Installed)));
    shot.save(&mut app, "pl-installed");
    shot.click(&mut app, plug("plug-tab", PlugAct::Tab(Tab::Connections)));
    shot.save(&mut app, "pl-connections");

    // A listing's detail, from Browse.
    shot.click(&mut app, plug("plug-tab", PlugAct::Tab(Tab::Browse)));
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
            _ => continue,
        };
        shot.type_text(&mut app, value);
    }
    shot.save(&mut app, "pl-add-http-filled");
    shot.click(&mut app, plug("plug-save", PlugAct::Save));
    shot.until(&mut app, "the connection made", |app, _| {
        (app.st.plugins.adding.is_none()
            && app.st.plugins.tab == Tab::Connections
            && !app.st.plugins.connections.is_empty())
        .then_some(())
    });
    shot.until(&mut app, "the list reloaded", |app, _| {
        loaded(app).then_some(())
    });
    shot.save(&mut app, "pl-connected");

    // Leaving the page forgets the form state.
    shot.click(
        &mut app,
        App::control("settings-nav", &Act::Set(SetAct::Open(Category::Keys))),
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
    shot.send(&mut app, key("KeyW", MOD | mods::SHIFT, None));
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

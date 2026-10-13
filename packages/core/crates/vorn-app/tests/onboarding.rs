//! The welcome guide driven as a new user meets it: it opens by itself on
//! a fresh HOME, walks its seven steps with the arrow keys and its buttons,
//! closes on Escape and stays closed on the next launch. Its buttons close
//! it and open what they name. `VORN_APP_SHOT=<prefix>` saves each step.

#![cfg(unix)]

mod support;

use serde_json::Value;
use support::{first_run, key, wait_for, Shot, TestVornd};
use vorn_app::act::Act;
use vorn_app::onboarding::{OnbAct, Then};
use vorn_app::state::Dialog;
use vorn_app::ui::Id;
use vorn_app::App;

fn onb(place: &str, a: OnbAct) -> Id {
    App::control(place, &Act::Onboard(a))
}

fn seen_on_disk(app: &mut App) -> Option<u64> {
    app.pump();
    let config = app.rpc().call("config:load", Value::Null).ok()?;
    config.pointer("/defaults/hasSeenOnboarding")?.as_u64()
}

#[test]
fn the_guide_opens_on_a_first_run_and_walks_seven_steps() {
    let Some(v) = TestVornd::start() else { return };
    let mut app = first_run(&v);
    assert_eq!(
        app.st.dialog,
        Some(Dialog::Welcome),
        "a fresh HOME opens the guide"
    );
    assert_eq!(app.st.onboard.step, 0);
    let mut shot = Shot::new();
    shot.save(&mut app, "onboarding-1");

    // The MCP setup folds open under the agents.
    shot.click(&mut app, onb("onboard", OnbAct::ToggleMcp));
    assert!(app.st.onboard.mcp_open);
    shot.save(&mut app, "onboarding-1-mcp");

    // → and Next alternate through the rest; the counter follows.
    for n in 2..=7 {
        if n % 2 == 0 {
            shot.send(&mut app, key("ArrowRight", 0, None));
        } else {
            shot.click(&mut app, onb("onboard", OnbAct::Next));
        }
        assert_eq!(app.st.onboard.step, n - 1);
        shot.save(&mut app, &format!("onboarding-{n}"));
    }
    // → past the last step stays on it.
    shot.send(&mut app, key("ArrowRight", 0, None));
    assert_eq!(app.st.onboard.step, 6);

    shot.click(&mut app, onb("onboard", OnbAct::Back));
    assert_eq!(app.st.onboard.step, 5);
    shot.send(&mut app, key("ArrowLeft", 0, None));
    assert_eq!(app.st.onboard.step, 4);
    shot.click(&mut app, onb("onboard", OnbAct::Go(1)));
    assert_eq!(app.st.onboard.step, 1);
    assert_eq!(
        seen_on_disk(&mut app),
        None,
        "nothing is saved until it closes"
    );

    shot.send(&mut app, key("Escape", 0, None));
    shot.frame(&mut app);
    assert_eq!(app.st.dialog, None);
    let seen = wait_for("hasSeenOnboarding saved", || seen_on_disk(&mut app));
    assert_eq!(seen, 2);
    drop(app);

    let mut again = first_run(&v);
    assert_eq!(
        again.st.dialog, None,
        "a returning user is not shown it again"
    );

    // The sidebar's Welcome Guide reopens it at the first step.
    let mut shot = Shot::new();
    shot.click(&mut again, App::control("sidebar-footer", &Act::Welcome));
    assert_eq!(again.st.dialog, Some(Dialog::Welcome));
    assert_eq!(again.st.onboard.step, 0);
}

#[test]
fn the_guides_buttons_close_it_and_open_what_they_name() {
    let Some(v) = TestVornd::start() else { return };
    let mut app = first_run(&v);
    let mut shot = Shot::new();

    // Add project, on the second step of a HOME with no projects.
    shot.send(&mut app, key("ArrowRight", 0, None));
    shot.click(&mut app, onb("onboard", OnbAct::Then(Then::AddProject)));
    assert_eq!(app.st.dialog, Some(Dialog::Project(None)));
    let seen = wait_for("hasSeenOnboarding saved", || seen_on_disk(&mut app));
    assert_eq!(seen, 2);

    // Try Command Palette, from the sixth step.
    shot.send(&mut app, key("Escape", 0, None));
    shot.click(&mut app, App::control("sidebar-footer", &Act::Welcome));
    for _ in 0..5 {
        shot.send(&mut app, key("ArrowRight", 0, None));
    }
    shot.click(&mut app, onb("onboard", OnbAct::Then(Then::Palette)));
    assert_eq!(app.st.dialog, Some(Dialog::Palette));

    // Try it now opens the launcher; the close button and the scrim close.
    shot.send(&mut app, key("Escape", 0, None));
    shot.click(&mut app, App::control("sidebar-footer", &Act::Welcome));
    shot.send(&mut app, key("ArrowRight", 0, None));
    shot.send(&mut app, key("ArrowRight", 0, None));
    shot.click(&mut app, onb("onboard", OnbAct::Then(Then::NewSession)));
    assert_eq!(app.st.dialog, None);
    assert!(app.st.composer_open);
    app.st.composer_open = false;

    shot.click(&mut app, App::control("sidebar-footer", &Act::Welcome));
    shot.click(&mut app, onb("onboard-x", OnbAct::Close));
    assert_eq!(app.st.dialog, None);
    shot.click(&mut app, App::control("sidebar-footer", &Act::Welcome));
    shot.click(&mut app, onb("onboard-skip", OnbAct::Close));
    assert_eq!(app.st.dialog, None);
    shot.click(&mut app, App::control("sidebar-footer", &Act::Welcome));
    // The scrim's middle is under the panel; press its corner.
    shot.frame(&mut app);
    shot.ui.pointer_move(8.0, 8.0);
    shot.ui.pointer_down();
    shot.ui.pointer_up();
    app.events(&mut shot.ui);
    assert_eq!(app.st.dialog, None);

    // Get Started on the last step closes it.
    shot.click(&mut app, App::control("sidebar-footer", &Act::Welcome));
    shot.click(&mut app, onb("onboard", OnbAct::Go(6)));
    shot.click(&mut app, onb("onboard", OnbAct::Next));
    assert_eq!(app.st.dialog, None);
}

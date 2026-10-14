//! Every kind of text input, typed into as a user would: each shows its
//! focus, a caret where the next character goes, a selection, the IME's
//! composition, and hides its placeholder once something is typed.
//! `VORN_APP_SHOT=<prefix>` saves a frame of each state.

#![cfg(unix)]

mod support;

use serde_json::json;
use support::{add_project, connect, key, Shot, TestVornd};
use vorn_app::app::MOD;
use vorn_app::screen::COMPOSER_INPUT;
use vorn_app::ui::{mods, Input};

#[test]
fn the_launcher_prompt_is_a_live_text_field() {
    let Some(v) = TestVornd::start() else {
        return;
    };
    let mut app = connect(&v);
    add_project(&mut app, &v, "demo", json!({}));
    let mut shot = Shot::new();
    shot.frame(&mut app);
    assert_eq!(
        shot.ui.focus(),
        Some(COMPOSER_INPUT),
        "the prompt has the keyboard as soon as it shows"
    );
    shot.save(&mut app, "launcher-empty");

    shot.type_text(&mut app, "fix the flaky test");
    assert_eq!(app.st.composer.text, "fix the flaky test");
    assert_eq!(shot.ui.edit_text(COMPOSER_INPUT), "fix the flaky test");
    let caret = shot.ui.edit(COMPOSER_INPUT).map(|e| e.caret());
    assert_eq!(caret, Some("fix the flaky test".len()));
    shot.save(&mut app, "launcher-typed");

    // The caret moves and typing lands where it is.
    for _ in 0..4 {
        shot.send(&mut app, key("ArrowLeft", 0, None));
    }
    shot.type_text(&mut app, "unit ");
    assert_eq!(app.st.composer.text, "fix the flaky unit test");
    shot.save(&mut app, "launcher-caret");

    // A selection shows and typing replaces it.
    shot.send(&mut app, key("KeyA", MOD(), Some("a")));
    let sel = shot.ui.edit(COMPOSER_INPUT).map(|e| e.selection());
    assert_eq!(sel, Some(0.."fix the flaky unit test".len()));
    shot.save(&mut app, "launcher-selection");
    shot.type_text(&mut app, "x");
    assert_eq!(app.st.composer.text, "x");
    shot.send(&mut app, key("Backspace", 0, None));
    assert!(app.st.composer.text.is_empty());

    // The IME's composition shows in place, then commits.
    shot.send(&mut app, Input::Preedit("にほん".into()));
    shot.save(&mut app, "launcher-ime");
    shot.send(&mut app, Input::Commit("日本".into()));
    assert_eq!(app.st.composer.text, "日本");

    // Shift+Enter starts a line; Tab still picks the project.
    shot.send(&mut app, key("Enter", mods::SHIFT, Some("\r")));
    shot.type_text(&mut app, "two");
    assert_eq!(app.st.composer.text, "日本\ntwo");
    shot.send(&mut app, key("Tab", 0, Some("\t")));
    assert_eq!(app.st.composer.project.as_deref(), Some("demo"));
    assert_eq!(shot.ui.focus(), Some(COMPOSER_INPUT));
}

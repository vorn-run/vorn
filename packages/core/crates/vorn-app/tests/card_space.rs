//! A card gives its terminal all the room that is not chrome: with no
//! extension drawing a footer, only the status bar and the resize strip sit
//! under a full-screen program.

mod support;

use serde_json::json;
use support::{connect, wait_for, Shot, TestVornd};
use vorn_app::look;
use vorn_app::screen::card_box_id;

#[cfg(unix)]
#[test]
fn a_full_screen_program_fills_its_card() {
    let Some(v) = TestVornd::start() else {
        return;
    };
    let mut app = connect(&v);
    let mut shot = Shot::new();
    let layouts = v.home.join("layouts");
    std::fs::create_dir_all(&layouts).expect("layouts");
    let project = v.home.join("demo");
    std::fs::create_dir_all(&project).expect("project");
    let p = project.to_str().expect("path");
    // A shell that switches to the alternate screen, as `top` does.
    let full = "printf '\\\\033[?1049h\\\\033[H'; sleep 60";
    std::fs::write(
        layouts.join("full.toml"),
        format!("name = \"full\"\nproject = \"{p}\"\nsplit = \"vertical\"\n\n[[children]]\ncommand = [\"sh\", \"-c\", \"{full}\"]\n\n[[children]]\nshell = true\n"),
    )
    .expect("layout");
    app.rpc()
        .call("layout:open", json!({ "name": "full" }))
        .expect("opened");
    let id = wait_for("the full-screen card", || {
        app.pump();
        shot.frame(&mut app);
        app.store
            .sessions
            .iter()
            .map(|s| s.info.id.clone())
            .find(|id| app.st.shells.get(id).is_some_and(|s| s.lines.alt))
    });
    // The composer stands aside and the terminal grows into its room.
    std::thread::sleep(std::time::Duration::from_millis(600));
    app.pump();
    shot.save(&mut app, "full-screen");
    let card = shot.ui.rect_of(card_box_id(&id)).expect("the card");
    let term = app.term_box(&id).expect("the terminal");
    let below = card.y + card.h - (term.y + term.h);
    let chrome = (look::CARD_STATUS_H + look::CARD_FOOT_H + 2.0) * shot.ui.scale();
    assert!(
        below <= chrome,
        "{below}px under the terminal, chrome is {chrome}px: {card:?} {term:?}"
    );
}

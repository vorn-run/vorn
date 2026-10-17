//! The turns settings page at 390x844 in pixels, cells and a native tree; `VORN_PHONE_SHOT=<prefix>` saves them.

use std::path::Path;
use std::sync::Arc;

use serde_json::json;
use vorn_app::points::Point;
use vorn_phone::screens;
use vorn_phone::{config, Insets, Model, Phone};
use vornui::backend::{Backend, Cells, Pixels};
use vornui::cells::units;
use vornui::native::Native;
use vornui::{App, Id, RenderMode, Rgba, Ui};

const SETTINGS: &str = concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/../../../../examples/extensions/turns/views/settings.kdl"
);
const EXT: &str = "example.turns";
const PHONE: (f32, f32) = (390.0, 844.0);
const SCALE: f32 = 3.0;

fn phone(size: (f32, f32)) -> (Phone, String) {
    let text = std::fs::read_to_string(SETTINGS).expect("settings.kdl");
    let template = vorn_view::file::parse("views/settings.kdl", &text, Default::default())
        .expect("the example's settings check");
    let mut p = Point::from_value(&json!({
        "kind": "settings",
        "extension": EXT,
        "extensionName": "Turns",
        "id": "prefs",
        "title": "Turns",
    }))
    .expect("point");
    p.template = Ok(Some(template));
    let key = p.key();
    let mut model = Model::new(Arc::new(|| {}));
    model.points.all = vec![p];
    model.points.loaded = true;
    let rows = |c: &str, v: serde_json::Value| (c.to_owned(), v);
    for (coll, list) in [
        rows(
            "prefs",
            json!([{ "id": "tools", "label": "Count tool calls", "on": true },
                   { "id": "idle", "label": "Count idle turns", "on": false }]),
        ),
        rows(
            "plans",
            json!([{ "id": "free", "name": "Free", "selected": true },
                   { "id": "pro", "name": "Pro", "selected": false }]),
        ),
        rows(
            "account",
            json!([{ "id": "a1", "name": "work@example.com", "signed_in": false }]),
        ),
        rows("finished", json!([])),
    ] {
        model
            .points
            .fill(EXT, &coll, Some(list.as_array().expect("rows")));
    }
    let mut phone = Phone::new(model);
    phone.size = size;
    phone.insets = Insets {
        top: 59.0,
        bottom: 34.0,
        ..Insets::default()
    };
    phone.settings = Some(Some(key.clone()));
    (phone, key)
}

fn frame(phone: &mut Phone, ui: &mut Ui, size: (f32, f32)) {
    ui.begin(Rgba::TRANSPARENT);
    let el = phone.view(ui);
    let laid = ui.layout(el, size);
    phone.paint(ui, &laid);
}

fn shot() -> Option<String> {
    std::env::var("VORN_PHONE_SHOT").ok()
}

/// The native tree's controls, with ids the drawn surfaces share.
fn native_controls(key: &str) -> Vec<(Id, &'static str, String)> {
    let (phone, _) = phone(PHONE);
    let mut ui = Ui::native(&config(SCALE));
    let el = screens::native(&phone, &ui, &format!("settings:{key}"));
    ui.begin(Rgba::TRANSPARENT);
    ui.layout(el, PHONE);
    let mut out = Native::default();
    out.present(&mut ui).expect("native");
    let tree = ui.native_tree().expect("tree");
    if let Some(prefix) = shot() {
        std::fs::write(format!("{prefix}-native.json"), &out.json).expect("json");
    }
    let kinds: Vec<_> = tree.walk().iter().map(|n| n.kind).collect();
    for k in [
        "section", "list", "toggle", "choice", "option", "field", "more", "button", "text",
    ] {
        assert!(kinds.contains(&k), "no {k} in {}", out.json);
    }
    let labels: Vec<_> = tree
        .walk()
        .iter()
        .filter(|n| n.kind == "section")
        .map(|n| n.label.clone())
        .collect();
    assert_eq!(labels, ["Counting", "Account", "Finished turns"]);
    let forget = tree
        .walk()
        .into_iter()
        .find(|n| n.label == "Forget account")
        .expect("the more menu lists its entries");
    assert_eq!(forget.confirm.as_deref(), Some("Forget work@example.com?"));
    tree.walk()
        .into_iter()
        .filter(|n| matches!(n.kind, "toggle" | "option" | "field" | "button"))
        .filter(|n| n.label != "Forget account")
        .map(|n| (n.id.expect("controls have ids"), n.kind, n.label.clone()))
        .collect()
}

#[test]
fn the_settings_page_fits_a_phone_in_pixels() {
    let (mut phone, key) = phone(PHONE);
    let controls = native_controls(&key);
    let mut ui = Ui::headless(RenderMode::Cpu, &config(SCALE));
    frame(&mut phone, &mut ui, PHONE);
    let px = ((PHONE.0 * SCALE) as u32, (PHONE.1 * SCALE) as u32);
    let mut b = Pixels::new(&ui, px);
    assert!(b.present(&mut ui).expect("pixels"));
    for (id, kind, label) in &controls {
        let r = ui
            .rect_of(*id)
            .unwrap_or_else(|| panic!("{kind} {label:?} is drawn"));
        assert!(
            r.x >= 0.0 && r.x + r.w <= PHONE.0 + 0.5 && r.w > 0.0,
            "{kind} {label:?} at {r:?} fits the width"
        );
    }
    let rgba = ui.read(&b.target).expect("read");
    let first = &rgba[..4];
    assert!(
        rgba.chunks_exact(4).any(|p| p != first),
        "the frame is drawn"
    );
    if let Some(prefix) = shot() {
        ui.save_png(&b.target, &format!("{prefix}-pixels.png"))
            .expect("png");
    }
}

#[test]
fn the_settings_page_fits_a_phone_in_cells() {
    let size = units::px(48, 30);
    let (mut phone, key) = phone(size);
    phone.insets = Insets::default();
    let controls = native_controls(&key);
    let mut ui = Ui::cells(&config(1.0));
    frame(&mut phone, &mut ui, size);
    let mut b = Cells::offscreen(48, 30);
    b.present(&mut ui).expect("cells");
    let text: Vec<String> = (0..30).map(|r| b.buffer.row_text(r)).collect();
    let all = text.join("\n");
    for want in ["Turns", "COUNTING", "Count tool calls", "ACCOUNT", "Free"] {
        assert!(all.contains(want), "{want:?} in\n{all}");
    }
    for (id, kind, label) in &controls {
        if let Some(r) = ui.rect_of(*id) {
            assert!(r.x + r.w <= size.0 + 0.5, "{kind} {label:?} at {r:?} fits");
        }
    }
    if let Some(prefix) = shot() {
        std::fs::write(Path::new(&format!("{prefix}-cells.txt")), all).expect("txt");
    }
}

#[test]
fn a_native_toggle_presses_what_the_drawn_one_does() {
    let (mut phone, key) = phone(PHONE);
    let controls = native_controls(&key);
    let (id, _, _) = controls
        .iter()
        .find(|(_, k, l)| *k == "toggle" && l == "Count tool calls")
        .expect("toggle");
    let mut ui = Ui::headless(RenderMode::Cpu, &config(SCALE));
    frame(&mut phone, &mut ui, PHONE);
    assert!(
        phone.model.acts.clicked(*id).is_some(),
        "the drawn page binds the id the native tree gave"
    );
}

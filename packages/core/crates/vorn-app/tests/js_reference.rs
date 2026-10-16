//! The browser, artifact and device pane logic against recorded answers in
//! `tests/fixtures/js-reference/browser-pane.json` at the repository root.
//!
//! Every recorded input runs through the Rust and must give the recorded
//! answer; numbers compare to within [`EPSILON`], since the fixture prints
//! them in JavaScript's shortest form.

use std::path::PathBuf;

use serde_json::{json, Map, Value};
use vorn_app::artifact::{self, Artboard, DocEdit};
use vorn_app::{browser, device};

const EPSILON: f64 = 1e-9;

fn fixture() -> Value {
    let path = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../../../../tests/fixtures/js-reference/browser-pane.json");
    let text = std::fs::read_to_string(&path)
        .unwrap_or_else(|e| panic!("reading {}: {e}", path.display()));
    serde_json::from_str(&text).expect("the fixture is JSON")
}

fn cases<'a>(all: &'a Value, key: &str) -> &'a [Value] {
    let list = all[key]
        .as_array()
        .unwrap_or_else(|| panic!("no `{key}` cases"));
    assert!(!list.is_empty(), "`{key}` has no cases");
    list
}

fn num(v: &Value) -> f64 {
    v.as_f64().unwrap_or_else(|| panic!("not a number: {v}"))
}

fn pair(v: &Value, a: &str, b: &str) -> (f64, f64) {
    (num(&v[a]), num(&v[b]))
}

#[track_caller]
fn close(got: f64, want: &Value, what: &Value) {
    assert!(
        (got - num(want)).abs() < EPSILON,
        "{what}: got {got}, want {want}"
    );
}

/// Equal as JSON, numbers within [`EPSILON`] whether written `12` or `12.0`.
fn same(a: &Value, b: &Value) -> bool {
    match (a, b) {
        (Value::Number(x), Value::Number(y)) => {
            (x.as_f64().unwrap() - y.as_f64().unwrap()).abs() < EPSILON
        }
        (Value::Array(x), Value::Array(y)) => {
            x.len() == y.len() && x.iter().zip(y).all(|(x, y)| same(x, y))
        }
        (Value::Object(x), Value::Object(y)) => {
            x.len() == y.len() && x.iter().all(|(k, v)| y.get(k).is_some_and(|w| same(v, w)))
        }
        _ => a == b,
    }
}

fn boards(v: &Value) -> Vec<Artboard> {
    let size = |b: &Value, k: &str| u32::try_from(b[k].as_u64().unwrap()).unwrap();
    v.as_array()
        .unwrap()
        .iter()
        .map(|b| Artboard {
            id: b["id"].as_str().unwrap().to_owned(),
            label: b["label"].as_str().unwrap().to_owned(),
            width: size(b, "width"),
            height: size(b, "height"),
        })
        .collect()
}

#[test]
fn addresses_load_and_read_as_the_bar_did() {
    let all = fixture();
    for c in cases(&all, "normalizeUrl") {
        let input = c["input"].as_str().unwrap();
        let allow = c["allowFile"].as_bool().unwrap();
        let want = c["out"].as_str();
        assert_eq!(
            browser::normalize_url(input, allow).as_deref(),
            want,
            "normalizeUrl({input:?}, {allow})"
        );
    }
    for c in cases(&all, "displayHost") {
        let input = c["input"].as_str().unwrap();
        assert_eq!(
            browser::display_host(input),
            c["out"].as_str().unwrap(),
            "displayHost({input:?})"
        );
    }
    for c in cases(&all, "flattenPageText") {
        let raw = c["raw"].as_str().unwrap();
        let max = usize::try_from(c["max"].as_u64().unwrap()).unwrap();
        assert_eq!(
            browser::flatten_page_text(raw, max),
            c["out"].as_str().unwrap(),
            "flattenPageText({raw:?}, {max})"
        );
    }
}

#[test]
fn doc_edits_merge_paragraph_by_paragraph() {
    let all = fixture();
    for c in cases(&all, "docEdits") {
        let original = c["original"].as_str().unwrap();
        let edited = c["edited"].as_str().unwrap();
        let want: Vec<String> = serde_json::from_value(c["paragraphs"].clone()).unwrap();
        assert_eq!(
            artifact::paragraphs(original),
            want,
            "paragraphs({original:?})"
        );
        let (body, edits) = artifact::merge_doc_edit(original, edited);
        let edits: Vec<Value> = edits
            .into_iter()
            .map(|DocEdit { before, after }| json!({ "before": before, "after": after }))
            .collect();
        assert_eq!(
            json!({ "body": body, "edits": edits }),
            c["merged"],
            "mergeDocEdit({original:?}, {edited:?})"
        );
    }
}

#[test]
fn the_canvas_fits_and_places_artboards() {
    let all = fixture();
    for c in cases(&all, "canvas") {
        let b = boards(&c["boards"]);
        let zoom = artifact::fit_zoom(&b, pair(&c["area"], "width", "height"));
        close(zoom, &c["zoom"], c);
        let (placed, w, h) = artifact::place_artboards(&b, zoom);
        let placed: Vec<Value> = placed
            .iter()
            .map(|p| json!({ "left": p.left, "top": p.top }))
            .collect();
        assert!(same(&Value::from(placed), &c["placed"]), "{c}");
        close(w, &c["width"], c);
        close(h, &c["height"], c);
    }
    for c in cases(&all, "clampZoom") {
        close(artifact::clamp_zoom(num(&c["input"])), &c["out"], c);
    }
    for c in cases(&all, "artboardUrl") {
        let (url, id) = (c["url"].as_str().unwrap(), c["id"].as_str().unwrap());
        assert_eq!(artifact::artboard_url(url, id), c["out"].as_str().unwrap());
    }
}

#[test]
fn the_comment_popover_lands_where_it_did() {
    let all = fixture();
    for c in cases(&all, "placePopover") {
        let r = &c["rect"];
        let rect = (
            num(&r["x"]) as f32,
            num(&r["y"]) as f32,
            num(&r["width"]) as f32,
            num(&r["height"]) as f32,
        );
        let (aw, ah) = pair(&c["area"], "width", "height");
        let (x, y) = artifact::place_popover(rect, (aw as f32, ah as f32));
        close(f64::from(x), &c["out"]["x"], c);
        close(f64::from(y), &c["out"]["y"], c);
    }
}

#[test]
fn the_device_frame_fits_and_zooms() {
    let all = fixture();
    for c in cases(&all, "bezel") {
        let points = pair(&c["screen"], "width", "height");
        let area = pair(&c["container"], "width", "height");
        let scale = device::fit_scale(points, area);
        close(scale, &c["scale"], c);
        let drawn = ((points.0 * scale).round(), (points.1 * scale).round());
        close(drawn.0, &c["width"], c);
        close(drawn.1, &c["height"], c);
        close(device::bezel_thickness(area, points), &c["thickness"], c);
        close(device::outer_radius(drawn, points), &c["outerRadius"], c);
    }
    for c in cases(&all, "steppedZoom") {
        let inward = c["direction"].as_i64() == Some(1);
        close(
            device::stepped_zoom(num(&c["current"]), inward),
            &c["out"],
            c,
        );
    }
}

#[test]
fn manifests_and_tweaks_read_as_the_page_declared() {
    let all = fixture();
    for c in cases(&all, "parseManifest") {
        let text = c["text"].as_str().unwrap();
        let got = artifact::parse_manifest(text).map(|m| serde_json::to_value(m).unwrap());
        let got = got.unwrap_or(Value::Null);
        assert!(
            same(&got, &c["out"]),
            "parseManifest({text:?}): got {got}, want {}",
            c["out"]
        );
    }
    let manifest = artifact::parse_manifest(all["tweakManifest"].as_str().unwrap())
        .expect("the tweak manifest parses");
    for c in cases(&all, "mergeTweaks") {
        let stored: Map<String, Value> = c["stored"].as_object().cloned().unwrap_or_default();
        let live = c["live"].as_object();
        let got = artifact::merge_tweaks(&manifest.tweaks, &stored, live);
        assert!(same(&Value::Object(got), &c["out"]), "{c}");
    }
}

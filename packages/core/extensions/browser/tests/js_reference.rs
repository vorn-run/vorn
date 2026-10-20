//! The address bar's rules against the answers recorded from the pane they
//! came from, in `tests/fixtures/js-reference/browser-pane.json` at the
//! repository root.

use std::path::PathBuf;

use serde_json::Value;
use vorn_ext_browser::address;

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

#[test]
fn addresses_load_and_read_as_the_bar_did() {
    let all = fixture();
    for c in cases(&all, "normalizeUrl") {
        let input = c["input"].as_str().unwrap();
        let allow = c["allowFile"].as_bool().unwrap();
        assert_eq!(
            address::normalize_url(input, allow).as_deref(),
            c["out"].as_str(),
            "normalizeUrl({input:?}, {allow})"
        );
    }
    for c in cases(&all, "displayHost") {
        let input = c["input"].as_str().unwrap();
        assert_eq!(
            address::display_host(input),
            c["out"].as_str().unwrap(),
            "displayHost({input:?})"
        );
    }
    for c in cases(&all, "flattenPageText") {
        let raw = c["raw"].as_str().unwrap();
        let max = usize::try_from(c["max"].as_u64().unwrap()).unwrap();
        assert_eq!(
            address::flatten_page_text(raw, max),
            c["out"].as_str().unwrap(),
            "flattenPageText({raw:?}, {max})"
        );
    }
}

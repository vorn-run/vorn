//! A theme-only package against the real vornd: linked, it is listed to
//! clients as a `themes` contribution, though it contributes nothing else.

mod common;

use serde_json::{json, Value};

use common::{connect, next, send, serve, CREDENTIAL};

const MANIFEST: &str = r##"id = "acme.dusk"
name = "Acme Dusk"
version = "1.0.0"
api = "vorn:extension@1.0"

[theme]
appearance = "dark"

[theme.terminal]
foreground = "#dddddd"
background = "#111111"
ansi = [
  "#000000", "#cc3333", "#33cc33", "#cccc33", "#3333cc", "#cc33cc", "#33cccc", "#cccccc",
  "#555555", "#ff5555", "#55ff55", "#ffff55", "#5555ff", "#ff55ff", "#55ffff", "#ffffff",
]
"##;

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_linked_theme_only_package_is_listed_as_a_theme() {
    let served = serve();
    let mut ws = connect(served.port, CREDENTIAL).await;
    let mut call = async |id: u64, method: &str, params: Value| {
        send(
            &mut ws,
            json!({ "jsonrpc": "2.0", "id": id, "method": method, "params": params }),
        )
        .await;
        loop {
            let frame = next(&mut ws).await;
            if frame["id"] == json!(id) {
                assert!(frame.get("error").is_none(), "{method}: {frame}");
                return frame["result"].clone();
            }
        }
    };
    let folder = tempfile::tempdir().unwrap();
    std::fs::write(folder.path().join("vorn-extension.toml"), MANIFEST).unwrap();
    call(1, "extension:link", json!({ "path": folder.path() })).await;

    let listed = call(2, "extension:contributions", Value::Null).await;
    let theme = listed
        .as_array()
        .unwrap()
        .iter()
        .find(|c| c["extension"] == "acme.dusk")
        .unwrap_or_else(|| panic!("the theme is listed: {listed}"));
    assert_eq!(theme["kind"], "themes");
    assert_eq!(theme["appearance"], "dark");
    assert_eq!(theme["theme"]["terminal"]["background"], "#111111");
}

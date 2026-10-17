//! Every manifest in the fixtures reads, writes back as `vorn-extension.toml`
//! and reads again as the same manifest; a field misspelt in any of them is
//! reported by name and line.

use std::fs;
use std::path::{Path, PathBuf};

use vorn_extensions::manifest::file::{self, to_toml};

/// Every manifest fixture and example extension, by path.
fn manifests() -> Vec<(String, String)> {
    know_shipped_agents();
    let root = Path::new(env!("CARGO_MANIFEST_DIR"));
    let mut paths: Vec<PathBuf> = ["tests/fixtures/manifests", "tests/fixtures/sdk-manifests"]
        .iter()
        .flat_map(|d| {
            fs::read_dir(root.join(d))
                .unwrap()
                .flatten()
                .map(|e| e.path())
        })
        .collect();
    paths.extend(
        fs::read_dir(root.join("../../../../examples/extensions"))
            .unwrap()
            .flatten()
            .map(|e| e.path().join(file::FILE))
            .filter(|p| p.is_file()),
    );
    for dir in ["hello-bash", "pilot"] {
        paths.push(root.join("tests/fixtures").join(dir).join(file::FILE));
    }
    paths.sort();
    paths
        .into_iter()
        .map(|p| (p.display().to_string(), fs::read_to_string(&p).unwrap()))
        .collect()
}

/// The SDK's card names an agent Vorn ships; reading its adapter says the id
/// exists, in this test's process as in any other.
fn know_shipped_agents() {
    for s in vorn_adapters::shipped() {
        vorn_agents::from_manifest_text(s.manifest).expect("a shipped adapter reads");
    }
}

#[test]
fn every_manifest_writes_back_as_it_reads() {
    let all = manifests();
    assert!(all.len() >= 10, "only {} manifests", all.len());
    for (name, text) in all {
        let manifest = file::parse(&text).unwrap_or_else(|e| panic!("{name}: {e}"));
        let toml = to_toml(&manifest);
        let back = file::parse(&toml).unwrap_or_else(|e| panic!("{name}: {e}\n{toml}"));
        assert_eq!(back, manifest, "{name}\n{toml}");
    }
}

/// In every manifest as written back, keys spread through the file,
/// misspelt in turn, are each an error naming the key and its line.
#[test]
fn a_misspelt_field_is_reported_by_name_and_line() {
    let mut checked = 0;
    for (name, text) in manifests() {
        let toml = to_toml(&file::parse(&text).unwrap());
        let lines: Vec<&str> = toml.lines().collect();
        // Keys of a free-form map (`headers`) are the author's, not the format's.
        let mut section = "";
        let mut keyed: Vec<(usize, &str)> = Vec::new();
        for (at, line) in lines.iter().enumerate() {
            if line.starts_with('[') {
                section = line;
            } else if let Some((key, _)) = line.split_once(" = ") {
                let key = key.trim();
                if !section.contains("headers") && !key.starts_with('"') && !key.contains('.') {
                    keyed.push((at, key));
                }
            }
        }
        // Every key of a small manifest; fourteen spread through a large one.
        let stride = keyed.len().div_ceil(14).max(1);
        for &(at, key) in keyed.iter().step_by(stride) {
            let line = lines[at];
            let mut broken = lines.clone();
            let misspelt = format!("{key}x");
            let edited = line.replacen(key, &misspelt, 1);
            broken[at] = &edited;
            let err = file::parse(&broken.join("\n")).unwrap_err();
            assert!(
                err.message.contains(&format!("unknown field `{misspelt}`")),
                "{name}:{}: {err}",
                at + 1
            );
            assert_eq!(err.line, at + 1, "{name}: {err}");
            checked += 1;
        }
    }
    assert!(checked > 80, "only {checked} fields were tried");
}

/// What the SDK writes for a connector's folder (`tests/connector-extension-manifest.test.ts`
/// keeps these in step with it) reads here, every key kept.
#[test]
fn reads_every_manifest_the_sdk_writes() {
    know_shipped_agents();
    let dir = Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/sdk-manifests");
    let read = |name: &str| {
        let text = std::fs::read_to_string(dir.join(format!("{name}.toml"))).unwrap();
        file::parse(&text).unwrap_or_else(|e| panic!("{name}: {e}"))
    };
    let browser = read("browser");
    assert_eq!(browser.name, "Sample \"quoted\"");
    assert_eq!(browser.protocol, Some(1));
    assert_eq!(
        browser.declares.network,
        ["sample.example", "*.sample.example"]
    );
    let sign_in = browser.auth.as_ref().unwrap().browser.as_ref().unwrap();
    assert_eq!(sign_in.origins.len(), 2);
    assert_eq!(
        browser.triggers[0].status_mapping.as_ref().unwrap()[0].suggested_local,
        "todo"
    );
    assert_eq!(browser.triggers[1].label, "same");
    let inputs = browser.actions[0].inputs.as_ref().unwrap();
    assert_eq!(inputs[0].kind, "string");
    assert!(inputs[0].required);
    assert_eq!(inputs[1].options.as_ref().unwrap().len(), 2);
    assert_eq!(inputs[2].load_options.as_deref(), Some("boards"));
    assert_eq!(
        browser.checks.as_deref(),
        Some(
            &[
                "node".to_owned(),
                "index.js".to_owned(),
                "--check".to_owned()
            ][..]
        )
    );

    let cli = read("cli");
    let borrow = cli.auth.as_ref().unwrap().borrow.as_ref().unwrap();
    assert_eq!(borrow.token_env.as_deref(), Some("GH_TOKEN"));
    assert_eq!(cli.declares.network_from, ["GH_HOSTS"]);

    let card = read("card");
    assert!(card.contributes.is_some());
    assert_eq!(card.declares.uses, ["git.read", "terminal.read"]);
}

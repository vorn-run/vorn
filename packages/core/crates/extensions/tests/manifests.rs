//! Every connector's and extension's 0.8 manifest converts to
//! `vorn-extension.toml`, validates, and reads back as the same manifest;
//! a field misspelt in any of them is reported by name and line.
//!
//! The connectors' manifests are what each of the 22 published connectors
//! answers `connector/manifest` with, recorded with the protocol their packs
//! carry; the extensions' cover every contribution and permission 0.8 had.

use std::fs;
use std::path::Path;

use serde_json::Value;
use vorn_extensions::manifest::file::{self, from_pack_json, to_toml};

fn packs(kind: &str) -> Vec<(String, Value)> {
    let dir = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests/fixtures/packs")
        .join(kind);
    let mut all: Vec<(String, Value)> = fs::read_dir(&dir)
        .unwrap()
        .flatten()
        .map(|e| {
            let text = fs::read_to_string(e.path()).unwrap();
            (
                e.file_name().to_string_lossy().into_owned(),
                serde_json::from_str(&text).unwrap(),
            )
        })
        .collect();
    all.sort_by(|a, b| a.0.cmp(&b.0));
    all
}

fn converts_and_reads_back(kind: &str, expected: usize) {
    let all = packs(kind);
    assert_eq!(all.len(), expected, "{kind}");
    for (name, json) in all {
        let manifest = from_pack_json(json.as_object().unwrap())
            .unwrap_or_else(|e| panic!("{name}: {e}"));
        let toml = to_toml(&manifest);
        let back = file::parse(&toml).unwrap_or_else(|e| panic!("{name}: {e}\n{toml}"));
        assert_eq!(back, manifest, "{name}\n{toml}");
        // What the app was shown of the pack is what it is shown now.
        let before = vorn_extensions::manifest::read(json.as_object().unwrap()).unwrap();
        let mut shown = serde_json::to_value(&back).unwrap();
        let mut was = serde_json::to_value(&before).unwrap();
        for key in ["service", "api"] {
            shown.as_object_mut().unwrap().remove(key);
            was.as_object_mut().unwrap().remove(key);
        }
        assert_eq!(shown, was, "{name}");
    }
}

#[test]
fn every_connector_manifest_converts_and_validates() {
    converts_and_reads_back("connectors", 22);
}

#[test]
fn every_extension_manifest_converts_and_validates() {
    converts_and_reads_back("extensions", 3);
}

/// In every converted manifest, keys spread through the file, misspelt in
/// turn, are each an error naming the key and its line.
#[test]
fn a_misspelt_field_is_reported_by_name_and_line() {
    let mut checked = 0;
    for kind in ["connectors", "extensions"] {
        for (name, json) in packs(kind) {
            let toml = to_toml(&from_pack_json(json.as_object().unwrap()).unwrap());
            let lines: Vec<&str> = toml.lines().collect();
            let keyed: Vec<(usize, &str)> = lines
                .iter()
                .enumerate()
                .filter_map(|(at, line)| Some((at, line.split_once(" = ")?.0.trim())))
                .filter(|(_, key)| !key.starts_with('"') && !key.contains('.'))
                .collect();
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
    }
    assert!(checked > 200, "only {checked} fields were tried");
}

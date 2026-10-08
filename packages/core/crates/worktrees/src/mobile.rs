//! Whether a project is a mobile app worth offering a device for
//! (`detectMobileProject`), from its declared dependencies and config, never
//! its build output: a managed Expo app often has no `ios/` at all.

use std::fs;
use std::path::Path;

use serde_json::{json, Value};

const EXPO_CONFIGS: [&str; 4] = [
    "app.json",
    "app.config.js",
    "app.config.ts",
    "app.config.mjs",
];
/// Directories never worth descending into: they vendor Xcode projects by the dozen.
const SKIP_DIRS: [&str; 6] = [
    "node_modules",
    "Pods",
    "build",
    "dist",
    "vendor",
    "Carthage",
];

fn answer(framework: Option<&str>, dev_client: bool) -> Value {
    json!({ "isMobile": framework.is_some(), "framework": framework, "needsDevClient": dev_client })
}

fn entries(dir: &Path) -> Vec<String> {
    fs::read_dir(dir)
        .map(|it| {
            it.filter_map(Result::ok)
                .map(|e| e.file_name().to_string_lossy().into_owned())
                .collect()
        })
        .unwrap_or_default()
}

fn names_xcode(names: &[String]) -> bool {
    names
        .iter()
        .any(|e| e.ends_with(".xcodeproj") || e.ends_with(".xcworkspace"))
}

fn has_xcode_project(dir: &Path) -> bool {
    names_xcode(&entries(dir))
}

/// `pubspec.yaml` naming Flutter: `sdk: flutter` or a `flutter:` key at a line's start.
fn names_flutter(text: &str) -> bool {
    text.lines().any(|line| {
        let t = line.trim_start();
        t.starts_with("flutter:")
            || t.strip_prefix("sdk:")
                .is_some_and(|rest| rest.trim_start().starts_with("flutter"))
    })
}

/// `{isMobile, framework, needsDevClient}` for the project at `project`.
pub fn detect(project: &str) -> Value {
    if project.is_empty() {
        return answer(None, false);
    }
    let root = Path::new(project);
    let package = fs::read_to_string(root.join("package.json"))
        .ok()
        .and_then(|t| serde_json::from_str::<Value>(&t).ok());
    if let Some(pkg) = package.as_ref().filter(|p| p.is_object()) {
        let has = |name: &str| {
            ["dependencies", "devDependencies"].iter().any(|k| {
                pkg.get(k)
                    .and_then(Value::as_object)
                    .is_some_and(|d| d.contains_key(name))
            })
        };
        if has("expo") {
            return answer(Some("expo"), !has_xcode_project(&root.join("ios")));
        }
        if has("react-native") {
            return answer(Some("react-native"), false);
        }
    }
    if let Ok(text) = fs::read_to_string(root.join("pubspec.yaml")) {
        if names_flutter(&text) {
            return answer(Some("flutter"), false);
        }
    }
    let names = entries(root);
    if names_xcode(&names) {
        return answer(Some("ios-native"), false);
    }
    if names.iter().any(|n| n == "ios") && has_xcode_project(&root.join("ios")) {
        return answer(Some("react-native"), false);
    }
    if EXPO_CONFIGS.iter().any(|c| names.iter().any(|n| n == c)) {
        return answer(Some("expo"), !has_xcode_project(&root.join("ios")));
    }
    for entry in &names {
        if entry.starts_with('.') || SKIP_DIRS.contains(&entry.as_str()) {
            continue;
        }
        if has_xcode_project(&root.join(entry)) {
            return answer(Some("ios-native"), false);
        }
    }
    answer(None, false)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn project(files: &[(&str, &str)]) -> tempfile::TempDir {
        let dir = tempfile::tempdir().unwrap();
        for (path, text) in files {
            let full = dir.path().join(path);
            fs::create_dir_all(full.parent().unwrap()).unwrap();
            fs::write(full, text).unwrap();
        }
        dir
    }

    fn of(dir: &tempfile::TempDir) -> Value {
        detect(dir.path().to_str().unwrap())
    }

    #[test]
    fn tells_each_kind_of_mobile_project_apart() {
        let expo = project(&[(
            "package.json",
            r#"{"dependencies":{"expo":"1","react-native":"1"}}"#,
        )]);
        assert_eq!(
            of(&expo),
            json!({ "isMobile": true, "framework": "expo", "needsDevClient": true })
        );
        let prebuilt = project(&[
            ("package.json", r#"{"devDependencies":{"expo":"1"}}"#),
            ("ios/App.xcworkspace/x", ""),
        ]);
        assert_eq!(of(&prebuilt)["needsDevClient"], false);
        let stray = project(&[
            ("package.json", r#"{"dependencies":{"expo":"1"}}"#),
            ("ios/.gitkeep", ""),
        ]);
        assert_eq!(of(&stray)["needsDevClient"], true);
        let web = project(&[("package.json", r#"{"dependencies":{"react":"1"}}"#)]);
        assert_eq!(of(&web)["isMobile"], false);
        let rn = project(&[("package.json", r#"{"dependencies":{"react-native":"1"}}"#)]);
        assert_eq!(of(&rn)["framework"], "react-native");
        let flutter = project(&[(
            "pubspec.yaml",
            "dependencies:\n  flutter:\n    sdk: flutter\n",
        )]);
        assert_eq!(of(&flutter)["framework"], "flutter");
        let dart = project(&[("pubspec.yaml", "name: pkg\n")]);
        assert_eq!(of(&dart)["isMobile"], false);
        let nested = project(&[
            ("app/App.xcodeproj/x", ""),
            ("node_modules/x/Y.xcodeproj/z", ""),
        ]);
        assert_eq!(of(&nested)["framework"], "ios-native");
        let vendored = project(&[
            ("node_modules/x/Y.xcodeproj/z", ""),
            ("package.json", "{broken"),
        ]);
        assert_eq!(of(&vendored)["isMobile"], false);
        let fresh = project(&[("app.json", "{}")]);
        assert_eq!(of(&fresh)["framework"], "expo");
        assert_eq!(
            detect(""),
            json!({ "isMobile": false, "framework": null, "needsDevClient": false })
        );
        assert_eq!(detect("/no/such/project")["isMobile"], false);
    }
}

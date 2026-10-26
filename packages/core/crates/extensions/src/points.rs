//! What a package contributes to the screens, as clients list it: every
//! place it draws a tree in, and every command, menu item, launcher and
//! attention point, flat and in place order.

use serde_json::{json, Map, Value};

use crate::activation::settings_hold;
use crate::manifest::{Manifest, PaneDraws, Place};

/// Every contribution of `manifest` a client draws or lists, as
/// `{extension, kind, id, title, ..., tree?}`; a pane only when it is a tree,
/// and its `[theme]` as kind `themes` with the section as written, so every
/// client reads it with `vorn_theme` as vornd checked it. One whose
/// `when.setting` names only preferences that are off in `settings` is left out;
/// a view whose `button.setting` does is listed with `button: false`.
pub fn listed(manifest: &Manifest, settings: &Map<String, Value>) -> Vec<Value> {
    let mut out = Vec::new();
    if let Some(theme) = manifest.theme() {
        out.push(json!({
            "extension": manifest.id,
            "kind": "themes",
            "id": manifest.id,
            "title": manifest.name,
            "publisher": manifest.publisher,
            "version": manifest.version,
            "appearance": theme.appearance.key(),
            "theme": manifest.theme,
        }));
    }
    let Some(c) = &manifest.contributes else {
        return out;
    };
    let entry = |place: Place, mut v: Value| {
        v["extension"] = json!(manifest.id);
        v["kind"] = json!(place.key());
        v
    };
    for pane in c.panes() {
        if pane.draws == PaneDraws::Tree && settings_hold(pane.base.when.as_ref(), settings) {
            let v = json!({ "id": pane.base.id, "title": pane.base.title, "tree": true });
            out.push(entry(Place::Panes, v));
        }
    }
    for place in Place::POINTS {
        for point in c
            .points(place)
            .iter()
            .filter(|p| settings_hold(p.base.when.as_ref(), settings))
        {
            let mut v = serde_json::to_value(point).unwrap_or_else(|_| json!({}));
            if point.button.is_some() {
                v["button"] = json!(settings_hold(point.button.as_ref(), settings));
            }
            out.push(entry(place, v));
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::manifest::file::parse;

    #[test]
    fn lists_every_place_and_the_panes_drawn_from_trees() {
        let m = parse(
            r#"id = "checks"
name = "Checks"
version = "1.0.0"
api = "vorn:extension@1.0"

[service]
command = ["node", "index.js"]

[[contributes.views]]
id = "board"
title = "Board"
tree = true

[[contributes.commands]]
id = "refresh"
title = "Refresh"
call = "refresh"

[[contributes.panes]]
id = "pane"
title = "Pane"
tree = true
"#,
        )
        .unwrap();
        let listed = listed(&m, &Map::new());
        let kinds: Vec<_> = listed
            .iter()
            .map(|p| (p["kind"].as_str().unwrap(), p["id"].as_str().unwrap()))
            .collect();
        assert_eq!(
            kinds,
            [
                ("panes", "pane"),
                ("commands", "refresh"),
                ("views", "board")
            ]
        );
        assert!(listed.iter().all(|p| p["extension"] == "checks"));
        assert_eq!(listed[2]["tree"], true);
        assert!(listed[1].get("tree").is_none());
    }

    #[test]
    fn lists_a_theme_with_its_section() {
        let text = vorn_theme::manifests()[2];
        let listed = listed(&parse(text).unwrap(), &Map::new());
        assert_eq!(listed.len(), 1);
        assert_eq!(listed[0]["kind"], "themes");
        assert_eq!(listed[0]["id"], "solarized-dark");
        assert_eq!(listed[0]["appearance"], "dark");
        assert_eq!(listed[0]["theme"]["ui"]["accent"], "#b58900");
    }

    #[test]
    fn a_point_gated_on_a_preference_follows_it() {
        let m = parse(
            r#"id = "board"
name = "Board"
version = "1.0.0"
api = "vorn:extension@1.0"

[service]
command = ["node", "index.js"]

[[contributes.panes]]
id = "pane"
title = "Pane"
tree = true
when = { setting = ["pane"] }

[[contributes.preferences]]
id = "pane"
title = "Pane beside cards"
type = "bool"
default = true
"#,
        )
        .unwrap();
        let with = |on: bool| {
            let mut settings = Map::new();
            settings.insert("pane".into(), Value::Bool(on));
            listed(&m, &settings).len()
        };
        assert_eq!(with(true), 1);
        assert_eq!(with(false), 0);
    }

    #[test]
    fn a_views_button_follows_its_preference_and_the_view_stays() {
        let m = parse(
            r#"id = "board"
name = "Board"
version = "1.0.0"
api = "vorn:extension@1.0"

[service]
command = ["node", "index.js"]

[[contributes.views]]
id = "board"
title = "Board"
tree = true
button = { setting = ["top_bar"] }

[[contributes.preferences]]
id = "top_bar"
title = "Show in the top bar"
type = "bool"
default = true
"#,
        )
        .unwrap();
        let with = |on: bool| {
            let settings = Map::from_iter([("top_bar".to_owned(), Value::Bool(on))]);
            listed(&m, &settings)
        };
        assert_eq!(with(true)[0]["button"], true);
        let off = with(false);
        assert_eq!(off.len(), 1, "the view stays");
        assert_eq!(off[0]["button"], false);
    }

    #[test]
    fn a_button_belongs_to_a_view_and_names_a_preference() {
        let manifest = |point: &str, gate: &str| {
            format!(
                r#"id = "board"
name = "Board"
version = "1.0.0"
api = "vorn:extension@1.0"

[service]
command = ["node", "index.js"]

[[contributes.{point}]]
id = "board"
title = "Board"
tree = true
button = {gate}

[[contributes.preferences]]
id = "top_bar"
title = "Show in the top bar"
type = "bool"
default = true
"#
            )
        };
        assert!(parse(&manifest("views", r#"{ setting = ["top_bar"] }"#)).is_ok());
        assert!(parse(&manifest("sidebar", r#"{ setting = ["top_bar"] }"#)).is_err());
        assert!(parse(&manifest("views", r#"{ setting = ["nope"] }"#)).is_err());
        assert!(parse(&manifest("views", r#"{ platform = ["macos"] }"#)).is_err());
    }
}

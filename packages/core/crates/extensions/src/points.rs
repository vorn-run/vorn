//! What a package contributes to the screens, as clients list it: every
//! place it draws a tree in, and every command, menu item, launcher and
//! attention point, flat and in place order.

use serde_json::{json, Value};

use crate::manifest::{Manifest, PaneDraws, Place};

/// Every contribution of `manifest` a client draws or lists, as
/// `{extension, kind, id, title, ..., tree?}`; a pane only when it is a tree.
pub fn listed(manifest: &Manifest) -> Vec<Value> {
    let Some(c) = &manifest.contributes else {
        return Vec::new();
    };
    let entry = |place: Place, mut v: Value| {
        v["extension"] = json!(manifest.id);
        v["kind"] = json!(place.key());
        v
    };
    let mut out = Vec::new();
    for pane in c.panes() {
        if pane.draws == PaneDraws::Tree {
            let v = json!({ "id": pane.base.id, "title": pane.base.title, "tree": true });
            out.push(entry(Place::Panes, v));
        }
    }
    for place in Place::POINTS {
        for point in c.points(place) {
            let v = serde_json::to_value(point).unwrap_or_else(|_| json!({}));
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
api = "vorn:extension@0.9"

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
        let listed = listed(&m);
        let kinds: Vec<_> = listed
            .iter()
            .map(|p| (p["kind"].as_str().unwrap(), p["id"].as_str().unwrap()))
            .collect();
        assert_eq!(
            kinds,
            [("panes", "pane"), ("commands", "refresh"), ("views", "board")]
        );
        assert!(listed.iter().all(|p| p["extension"] == "checks"));
        assert_eq!(listed[2]["tree"], true);
        assert!(listed[1].get("tree").is_none());
    }
}

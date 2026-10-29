//! The toolbar's items: the app's own (View, Split, New) and those extensions
//! contribute at the `toolbar` point, each by an id the arrangement keeps.

use crate::points::{kind, Point, Points};

/// The grid's view options.
pub const VIEW: &str = "view";
/// A shell beside the selected card.
pub const SPLIT: &str = "split";
/// A new session, and its menu of what else starts one.
pub const NEW: &str = "new";

/// One toolbar item, whatever adds it.
#[derive(Debug, Clone, Copy)]
pub enum Item<'a> {
    View,
    Split,
    New,
    Point(&'a Point),
}

impl<'a> Item<'a> {
    /// The item `id` names, if it is still there.
    pub fn of(points: &'a Points, id: &str) -> Option<Item<'a>> {
        Some(match id {
            VIEW => Item::View,
            SPLIT => Item::Split,
            NEW => Item::New,
            key => Item::Point(points.at(kind::TOOLBAR).find(|p| p.key() == key)?),
        })
    }

    pub fn icon(&self) -> &str {
        match self {
            Item::View => "sliders-horizontal",
            Item::Split => "columns-2",
            Item::New => "plus",
            Item::Point(p) => p.icon.as_deref().unwrap_or("puzzle"),
        }
    }

    /// Its name in the customize sheet.
    pub fn label(&self) -> &str {
        match self {
            Item::View => "View",
            Item::Split => "Split",
            Item::New => "New",
            Item::Point(p) if p.title.is_empty() => &p.id,
            Item::Point(p) => &p.title,
        }
    }

    /// What its tooltip says.
    pub fn tip(&self) -> &str {
        match self {
            Item::View => "View options",
            Item::Split => "Split right",
            Item::New => "New session",
            Item::Point(p) => p.description.as_deref().unwrap_or(self.label()),
        }
    }
}

/// Every item's id in its default order: View, what extensions add (Vorn's
/// own first), Split, then New at the end.
pub fn available(points: &Points) -> Vec<String> {
    let mut added: Vec<&Point> = points.at(kind::TOOLBAR).collect();
    added.sort_by_key(|p| !p.builtin);
    std::iter::once(VIEW.to_owned())
        .chain(added.into_iter().map(Point::key))
        .chain([SPLIT.to_owned(), NEW.to_owned()])
        .collect()
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::*;

    #[test]
    fn extensions_items_sit_between_view_and_split_vorns_first() {
        let mut points = Points::default();
        for (ext, id, builtin) in [("acme.git", "git", false), ("vorn.recent", "recent", true)] {
            let v = json!({ "extension": ext, "kind": "toolbar", "id": id, "icon": "clock",
                            "title": "Recent", "builtin": builtin, "tree": true });
            points.all.push(Point::from_value(&v).unwrap());
        }
        assert_eq!(
            available(&points),
            [
                "view",
                "vorn.recent/toolbar/recent",
                "acme.git/toolbar/git",
                "split",
                "new"
            ]
        );
        let recent = Item::of(&points, "vorn.recent/toolbar/recent").unwrap();
        assert_eq!((recent.icon(), recent.label()), ("clock", "Recent"));
        assert!(Item::of(&points, "gone/toolbar/x").is_none());
    }
}

//! The Lucide icon set (ISC; see `icons/LICENSE`): every glyph the
//! components draw, by Lucide name, and the edits that tint, thicken and
//! turn one.

macro_rules! icons {
    ($($name:literal,)*) => {
        /// Every icon in the set, by Lucide name.
        pub const ALL: &[&str] = &[$($name),*];

        /// The icon's source as Lucide ships it.
        pub fn source(name: &str) -> Option<&'static str> {
            Some(match name {
                $($name => include_str!(concat!("../icons/", $name, ".svg")),)*
                _ => return None,
            })
        }
    };
}

icons! {
    "activity",
    "align-left",
    "align-vertical-space-around",
    "archive",
    "archive-restore",
    "arrow-down",
    "arrow-left",
    "arrow-right",
    "arrow-right-left",
    "arrow-up",
    "arrow-up-down",
    "bell",
    "bold",
    "book-open",
    "bot",
    "braces",
    "briefcase",
    "building-2",
    "calendar",
    "check",
    "chevron-down",
    "chevron-left",
    "chevron-right",
    "chevron-up",
    "chevrons-left",
    "chevrons-right",
    "circle",
    "circle-alert",
    "circle-check",
    "circle-check-big",
    "circle-help",
    "circle-x",
    "clipboard-list",
    "clipboard-paste",
    "clock",
    "cloud",
    "code",
    "code-xml",
    "coffee",
    "columns-3",
    "copy",
    "corner-down-left",
    "cpu",
    "database",
    "download",
    "ellipsis",
    "external-link",
    "eye",
    "eye-off",
    "file-code",
    "file-diff",
    "file-key",
    "file-plus-2",
    "file-text",
    "flask-conical",
    "folder",
    "folder-git-2",
    "folder-open",
    "folder-plus",
    "folder-tree",
    "funnel",
    "gamepad-2",
    "git-branch",
    "git-commit-horizontal",
    "git-fork",
    "github",
    "globe",
    "grid-4",
    "group",
    "hand",
    "hard-drive",
    "heading-1",
    "heading-2",
    "heading-3",
    "heart",
    "history",
    "house",
    "image",
    "image-down",
    "image-plus",
    "import",
    "inbox",
    "info",
    "italic",
    "key",
    "key-round",
    "keyboard",
    "layers",
    "layout-dashboard",
    "layout-grid",
    "lightbulb",
    "linear",
    "link",
    "list",
    "list-filter",
    "list-ordered",
    "list-plus",
    "list-todo",
    "list-tree",
    "loader-circle",
    "lock",
    "lock-open",
    "log-in",
    "maximize",
    "maximize-2",
    "mcp",
    "message-square",
    "message-square-plus",
    "minimize-2",
    "minus",
    "monitor",
    "mouse-pointer-click",
    "music",
    "package",
    "palette",
    "panel-left",
    "panel-right",
    "paperclip",
    "pen-line",
    "pencil",
    "play",
    "plug",
    "plus",
    "power",
    "puzzle",
    "quote",
    "refresh-cw",
    "repeat",
    "replace",
    "rocket",
    "rotate-ccw",
    "rotate-cw",
    "save",
    "scan",
    "search",
    "send",
    "server",
    "settings",
    "settings-2",
    "shapes",
    "shield",
    "sliders-horizontal",
    "smartphone",
    "sparkles",
    "split",
    "square",
    "square-arrow-out-up-right",
    "square-check",
    "square-check-big",
    "square-kanban",
    "square-terminal",
    "star",
    "step-forward",
    "strikethrough",
    "tab-window",
    "terminal",
    "trash-2",
    "triangle-alert",
    "undo-2",
    "ungroup",
    "unlink",
    "upload",
    "user",
    "users",
    "wifi",
    "workflow",
    "wrench",
    "x",
    "zap",
}

pub const CHECK: &str = include_str!("../icons/check.svg");
pub const CHEVRON_DOWN: &str = include_str!("../icons/chevron-down.svg");
pub const PLUS: &str = include_str!("../icons/plus.svg");
pub const X: &str = include_str!("../icons/x.svg");

/// Icon `name` at stroke width `stroke`, white so a tint colours it.
pub fn lucide(name: &str, stroke: f32) -> Option<String> {
    Some(stroke_width(source(name)?, stroke).replace("currentColor", "#ffffff"))
}

/// A terminal stand-in for Lucide icon `name`: one or two cells of text.
pub fn glyph(name: &str) -> &'static str {
    match name {
        "activity" => "∿",
        "align-left" | "list" | "list-filter" | "funnel" | "sliders-horizontal" | "list-tree" => {
            "≡"
        }
        "archive" | "archive-restore" | "inbox" => "▤",
        "arrow-left" | "undo-2" => "←",
        "arrow-right" | "send" | "log-in" => "→",
        "arrow-right-left" | "replace" | "repeat" => "⇄",
        "arrow-up" | "upload" => "↑",
        "arrow-up-down" => "↕",
        "arrow-down" | "download" | "image-down" => "↓",
        "bell" => "◔",
        "bold" => "B",
        "italic" => "I",
        "heading-1" | "heading-2" | "heading-3" => "H",
        "braces" => "{}",
        "code" | "code-xml" => "<>",
        "check" | "circle-check" | "circle-check-big" | "square-check" | "square-check-big" => "✓",
        "chevron-down" => "▾",
        "chevron-up" => "▴",
        "chevron-left" => "‹",
        "chevron-right" => "›",
        "chevrons-left" => "«",
        "chevrons-right" => "»",
        "circle" => "○",
        "circle-alert" | "triangle-alert" => "!",
        "circle-help" => "?",
        "circle-x" | "x" => "×",
        "info" => "i",
        "clock" | "history" => "◷",
        "copy" | "clipboard-list" | "clipboard-paste" => "⧉",
        "corner-down-left" => "↵",
        "ellipsis" => "…",
        "external-link" | "square-arrow-out-up-right" | "maximize-2" => "↗",
        "minimize-2" => "↙",
        "eye" => "◉",
        "eye-off" => "◌",
        "folder" | "folder-git-2" | "folder-tree" | "folder-plus" => "▪",
        "folder-open" => "▫",
        "git-branch" | "git-fork" | "split" => "⑂",
        "git-commit-horizontal" => "◦",
        "house" => "⌂",
        "layers" | "group" | "ungroup" => "◫",
        "layout-dashboard" | "layout-grid" | "grid-4" | "square-kanban" => "▦",
        "columns-3" => "▥",
        "panel-left" => "◧",
        "panel-right" => "◨",
        "loader-circle" | "refresh-cw" | "rotate-cw" => "↻",
        "rotate-ccw" => "↺",
        "lock" => "▣",
        "lock-open" => "□",
        "maximize" | "square" | "tab-window" => "□",
        "minus" => "−",
        "monitor" => "▭",
        "list-todo" => "☑",
        "plus" | "list-plus" | "file-plus-2" | "message-square-plus" | "image-plus" => "+",
        "pen-line" | "pencil" => "✎",
        "play" | "step-forward" => "▶",
        "search" | "scan" => "⌕",
        "settings" | "settings-2" | "wrench" | "cpu" => "✲",
        "sparkles" | "star" | "zap" | "lightbulb" => "✦",
        "terminal" | "square-terminal" => "›",
        "trash-2" => "⌫",
        "user" | "users" => "◯",
        "workflow" | "plug" | "puzzle" | "mcp" => "◇",
        "rocket" => "▲",
        "globe" => "◍",
        "database" | "server" => "≣",
        "smartphone" => "▯",
        "package" | "briefcase" => "▤",
        "file-code" => "<>",
        "cloud" => "☁",
        "shield" => "◈",
        "gamepad-2" => "◎",
        "music" => "♪",
        "image" => "▨",
        "book-open" => "◫",
        "flask-conical" => "⚗",
        "heart" => "♥",
        "coffee" => "◡",
        "building-2" => "▥",
        _ => "·",
    }
}

/// The glyph for an icon SVG [`lucide`] made, however it was stroked,
/// tinted or turned; none for an SVG from elsewhere.
pub fn glyph_of(svg: &str) -> Option<&'static str> {
    use std::collections::HashMap;
    use std::sync::OnceLock;
    static BY_SHAPE: OnceLock<HashMap<String, &'static str>> = OnceLock::new();
    let names = BY_SHAPE.get_or_init(|| {
        ALL.iter()
            .filter_map(|n| Some((shape(source(n)?), *n)))
            .collect()
    });
    names.get(&shape(svg)).map(|n| glyph(n))
}

/// What identifies an icon: its children's geometry, without the
/// attributes [`lucide`], [`stroke`] and [`rotated`] change.
fn shape(svg: &str) -> String {
    const GEOMETRY: [&str; 15] = [
        "d", "cx", "cy", "r", "rx", "ry", "x", "y", "x1", "y1", "x2", "y2", "width", "height",
        "points",
    ];
    let body = svg.find('>').map_or(svg, |i| &svg[i + 1..]);
    let mut key = String::new();
    let mut rest = body;
    while let Some(i) = rest.find("=\"") {
        let name = rest[..i]
            .rsplit(|c: char| c.is_whitespace())
            .next()
            .unwrap_or("");
        let after = &rest[i + 2..];
        let end = after.find('"').unwrap_or(after.len());
        if GEOMETRY.contains(&name) {
            key.push_str(name);
            key.push('=');
            key.push_str(&after[..end]);
            key.push(';');
        }
        rest = &after[(end + 1).min(after.len())..];
    }
    key
}

/// `svg` drawn with lines `w` units wide (Lucide's `strokeWidth`).
pub fn stroke(svg: &str, w: f32) -> String {
    svg.replacen("stroke-width=\"2\"", &format!("stroke-width=\"{w}\""), 1)
}

fn stroke_width(svg: &str, w: f32) -> String {
    svg.replace("stroke-width=\"2\"", &format!("stroke-width=\"{w}\""))
}

/// `svg` turned `deg` degrees about its centre, to whole degrees so a
/// turning icon rasterizes a bounded number of times.
pub fn rotated(svg: &str, deg: f32) -> String {
    let deg = deg.round();
    if deg == 0.0 {
        return svg.to_owned();
    }
    match (svg.find('>'), svg.rfind("</svg>")) {
        (Some(open), Some(close)) if open < close => format!(
            "{}<g transform=\"rotate({deg} 12 12)\">{}</g>{}",
            &svg[..=open],
            &svg[open + 1..close],
            &svg[close..]
        ),
        _ => svg.to_owned(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_icon_finds_its_glyph_however_it_is_drawn() {
        let mut seen = std::collections::HashSet::new();
        for name in ALL {
            assert!(
                seen.insert(shape(source(name).unwrap())),
                "{name} shares a shape"
            );
            let svg = lucide(name, 1.5).unwrap();
            assert_eq!(glyph_of(&svg), Some(glyph(name)), "{name}");
            assert_eq!(
                glyph_of(&rotated(&svg, 90.0)),
                Some(glyph(name)),
                "{name} turned"
            );
        }
        assert_eq!(glyph_of("<svg><path d=\"M0 0\"/></svg>"), None);
    }

    #[test]
    fn every_icon_loads_and_takes_its_stroke() {
        for name in ALL {
            let svg = lucide(name, 1.5).unwrap_or_else(|| panic!("{name}"));
            // The brand marks are filled, not stroked.
            let filled = matches!(*name, "github" | "linear" | "mcp");
            assert!(
                (filled || svg.contains("stroke-width=\"1.5\"")) && svg.contains("#ffffff"),
                "{name}"
            );
        }
        assert!(lucide("nope", 2.0).is_none());
    }
}

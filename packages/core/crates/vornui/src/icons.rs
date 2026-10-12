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
    "arrow-left",
    "arrow-right",
    "arrow-right-left",
    "arrow-up",
    "bold",
    "book-open",
    "bot",
    "braces",
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
    "circle-help",
    "circle-x",
    "clipboard-list",
    "clipboard-paste",
    "clock",
    "cloud",
    "code",
    "code-xml",
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
    "file-plus-2",
    "file-text",
    "flask-conical",
    "folder",
    "folder-git-2",
    "folder-open",
    "folder-plus",
    "folder-tree",
    "gamepad-2",
    "git-branch",
    "git-commit-horizontal",
    "git-fork",
    "github",
    "globe",
    "grid-4",
    "group",
    "hand",
    "heading-1",
    "heading-2",
    "heading-3",
    "history",
    "house",
    "image",
    "image-down",
    "image-plus",
    "inbox",
    "info",
    "italic",
    "keyboard",
    "layers",
    "lightbulb",
    "linear",
    "link",
    "list",
    "list-filter",
    "list-ordered",
    "list-plus",
    "list-todo",
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
    "paperclip",
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
    "square-check",
    "square-terminal",
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
    "workflow",
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

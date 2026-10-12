//! Today's look as values: colours from `src/renderer/theme.css` and
//! Tailwind's grays, sizes from the components' classes, the lucide icons
//! (ISC, `assets/icons/LICENSE`) and the logo. Screens read these; nothing
//! here is redesigned.

/// 0xRRGGBB.
pub type Rgb = u32;

pub const SURFACE_BASE: Rgb = 0x0d0d0f;
pub const SURFACE_RAISED: Rgb = 0x141416;
pub const SURFACE_SUNKEN: Rgb = 0x141416;
pub const BRONZO: Rgb = 0xc9972a;
pub const INK: Rgb = 0xfaf9f7;
pub const DANGER: Rgb = 0xd4623f;
pub const WHITE: Rgb = 0xffffff;
/// Tailwind v4's grays, as sRGB.
pub const GRAY_200: Rgb = 0xe5e7eb;
pub const GRAY_300: Rgb = 0xd1d5dc;
pub const GRAY_400: Rgb = 0x99a1af;
pub const GRAY_500: Rgb = 0x6a7282;
pub const GRAY_600: Rgb = 0x4a5565;
pub const GRAY_700: Rgb = 0x364153;
pub const SURFACE_OVERLAY: Rgb = 0x1c1c20;
pub const SURFACE_PANEL: Rgb = 0x101012;
pub const GREEN_400: Rgb = 0x05df72;
pub const GREEN_500: Rgb = 0x00c758;
pub const YELLOW_500: Rgb = 0xedb200;
pub const RED_400: Rgb = 0xff6467;
pub const RED_500: Rgb = 0xfb2c36;
pub const AMBER_400: Rgb = 0xffba00;
pub const BLUE_400: Rgb = 0x54a2ff;
/// `--color-ink-secondary` and `--color-ink-faint`: white at these alphas.
pub const INK_SECONDARY: f32 = 0.55;
pub const INK_FAINT: f32 = 0.35;

/// The window's first size, in logical pixels.
pub const WINDOW: (f32, f32) = (1440.0, 900.0);
pub const TOP_BAR_H: f32 = 40.0;
pub const SIDEBAR_W: f32 = 256.0;
pub const COMPOSER_MAX_W: f32 = 800.0;
/// The terminal font size, in points.
pub const TERM_FONT: f32 = 12.0;
/// A card's header (`py-2.5` around an 18 px icon, its border included).
pub const CARD_HEADER_H: f32 = 39.0;
/// The card's status bar and its `border-t`.
pub const CARD_STATUS_H: f32 = 22.0;
/// The terminal's `pt-0.5` inside the card body.
pub const CARD_BODY_PT: f32 = 2.0;
/// The renderer's `MIN_CARD_W` and `MIN_CARD_H` for the automatic layout.
pub const MIN_CARD: (f32, f32) = (320.0, 280.0);

/// The UI font on each platform, as the app's CSS stack picks it.
pub const UI_FONT: &str = if cfg!(target_os = "macos") {
    "Helvetica Neue"
} else if cfg!(windows) {
    "Segoe UI"
} else {
    "DejaVu Sans"
};

pub const MONO_FONT: &str = if cfg!(target_os = "macos") {
    "Menlo"
} else if cfg!(windows) {
    "Consolas"
} else {
    "DejaVu Sans Mono"
};

pub const PLACEHOLDER: &str = "Describe your task...";
pub const PICK_PROJECT: &str = "Select a project to get started";
pub const PILLS: [(&str, &str); 3] = [
    ("monitor", "Terminals"),
    ("list-todo", "Tasks"),
    ("workflow", "Workflows"),
];

pub const LOGO_PNG: &[u8] = include_bytes!("../assets/vorn-logo.png");
/// The logo's aspect, width over height.
pub const LOGO_ASPECT: f32 = 400.0 / 185.0;

macro_rules! icons {
    ($($name:literal,)*) => {
        /// Every icon the screens use, by lucide name.
        pub const ICONS: &[&str] = &[$($name),*];

        fn icon_src(name: &str) -> Option<&'static str> {
            Some(match name {
                $($name => include_str!(concat!("../assets/icons/", $name, ".svg")),)*
                _ => return None,
            })
        }
    };
}

icons! {
    "activity",
    "align-vertical-space-around",
    "archive",
    "archive-restore",
    "arrow-left",
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
    "flask-conical",
    "folder",
    "folder-git-2",
    "folder-open",
    "folder-plus",
    "folder-tree",
    "gamepad-2",
    "git-branch",
    "github",
    "git-commit-horizontal",
    "git-fork",
    "globe",
    "grid-4",
    "group",
    "hand",
    "heading-1",
    "heading-2",
    "heading-3",
    "history",
    "image",
    "image-plus",
    "inbox",
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
    "minimize-2",
    "minus",
    "monitor",
    "music",
    "package",
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
    "save",
    "search",
    "send",
    "server",
    "settings",
    "settings-2",
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
    "unlink",
    "upload",
    "user",
    "workflow",
    "x",
    "zap",
}

/// `AgentIcon`'s marks, in their own colours.
pub const AGENT_ICONS: [(&str, &str); 5] = [
    ("claude", include_str!("../assets/agents/claude.svg")),
    ("copilot", include_str!("../assets/agents/copilot.svg")),
    ("codex", include_str!("../assets/agents/codex.svg")),
    ("opencode", include_str!("../assets/agents/opencode.svg")),
    ("gemini", include_str!("../assets/agents/gemini.svg")),
];

/// `ICON_MAP` and `PROJECT_ICON_OPTIONS`: the names a workflow or project
/// stores, the glyph each draws and the picker's label.
pub const ICON_OPTIONS: [(&str, &str, &str); 24] = [
    ("Folder", "folder", "Folder"),
    ("FolderGit2", "folder-git-2", "Git Repo"),
    ("Code", "code", "Code"),
    ("Globe", "globe", "Web"),
    ("Database", "database", "Database"),
    ("Server", "server", "Server"),
    ("Smartphone", "smartphone", "Mobile"),
    ("Package", "package", "Package"),
    ("FileCode", "file-code", "Script"),
    ("Terminal", "terminal", "Terminal"),
    ("Cpu", "cpu", "Hardware"),
    ("Cloud", "cloud", "Cloud"),
    ("Shield", "shield", "Security"),
    ("Zap", "zap", "Fast"),
    ("Workflow", "workflow", "Workflow"),
    ("Gamepad2", "gamepad-2", "Game"),
    ("Music", "music", "Audio"),
    ("Image", "image", "Image"),
    ("BookOpen", "book-open", "Docs"),
    ("FlaskConical", "flask-conical", "Lab"),
    ("Rocket", "rocket", "Launch"),
    ("github", "github", "GitHub"),
    ("linear", "linear", "Linear"),
    ("mcp", "mcp", "MCP"),
];

/// `ICON_COLOR_PALETTE`.
pub const ICON_COLORS: [Rgb; 8] = [
    0x6b7280, 0x3b82f6, 0x8b5cf6, 0xec4899, 0xef4444, 0xf97316, 0xeab308, 0x22c55e,
];

/// `ICON_MAP[name]`: the glyph a stored icon name draws, `Play` included.
pub fn icon_map(name: &str) -> Option<&'static str> {
    if name == "Play" {
        return Some("play");
    }
    ICON_OPTIONS
        .iter()
        .find(|(n, _, _)| *n == name)
        .map(|(_, glyph, _)| *glyph)
}

/// The icon's SVG with its stroke width set, ready to tint.
pub fn icon(name: &str, stroke: f32) -> Option<String> {
    Some(
        icon_src(name)?
            .replace("stroke-width=\"2\"", &format!("stroke-width=\"{stroke}\""))
            .replace("currentColor", "#ffffff"),
    )
}

/// The terminal's colours: the renderer's terminal theme.
pub mod term {
    use super::Rgb;

    pub const FG: Rgb = 0xd4d4d8;
    pub const BG: Rgb = 0x141416;
    pub const CURSOR: Rgb = 0xd4d4d8;
    pub const ANSI16: [Rgb; 16] = [
        0x27272a, 0xef4444, 0x22c55e, 0xeab308, 0x3b82f6, 0xa855f7, 0x06b6d4, 0xd4d4d8, 0x52525b,
        0xf87171, 0x4ade80, 0xfacc15, 0x60a5fa, 0xc084fc, 0x22d3ee, 0xfafafa,
    ];
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_icon_used_loads_and_takes_its_stroke() {
        for name in ICONS {
            let svg = icon(name, 1.5).unwrap_or_else(|| panic!("{name}"));
            // The brand marks are filled, not stroked.
            let filled = matches!(*name, "github" | "linear" | "mcp");
            assert!(
                (filled || svg.contains("stroke-width=\"1.5\"")) && svg.contains("#ffffff"),
                "{name}"
            );
        }
        assert!(icon("nope", 2.0).is_none());
    }
}

//! Today's look as values: colours from `src/renderer/theme.css` and
//! Tailwind's grays (both from vornui's tokens), sizes from the components'
//! classes and the logo. Screens read these; nothing
//! here is redesigned.

/// 0xRRGGBB.
pub type Rgb = u32;

// The palette lives once, in vornui's tokens.
pub use crate::ui::theme::rgb::{
    AMBER_400, BLUE_400, BRONZO, DANGER, GRAY_200, GRAY_300, GRAY_400, GRAY_500, GRAY_600,
    GRAY_700, GREEN_400, GREEN_500, INK, RED_400, RED_500, SKY_400, STATUS_BLUE, STATUS_SLATE,
    SURFACE_BASE, SURFACE_OVERLAY, SURFACE_PANEL, SURFACE_RAISED, SURFACE_SUNKEN, WHITE,
    YELLOW_500,
};
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

pub const PLACEHOLDER: &str = "Describe your task...";
pub const PICK_PROJECT: &str = "Select a project to get started";
pub const PILLS: [(&str, &str); 3] = [
    ("monitor", "Sessions"),
    ("list-todo", "Tasks"),
    ("workflow", "Workflows"),
];

pub const LOGO_PNG: &[u8] = include_bytes!("../assets/vorn-logo.png");
/// The logo's aspect, width over height.
pub const LOGO_ASPECT: f32 = 400.0 / 185.0;

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
    crate::ui::icons::lucide(name, stroke)
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

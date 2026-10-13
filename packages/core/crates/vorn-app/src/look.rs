//! Today's look as values: colours from `src/renderer/theme.css` and
//! Tailwind's grays (both from vornui's tokens), sizes from the components'
//! classes and the logo. Screens read these; nothing
//! here is redesigned.

/// 0xRRGGBB.
pub type Rgb = u32;

// The palette lives once, in vornui's tokens.
pub use crate::ui::theme::rgb::{
    AMBER_300, AMBER_400, AMBER_500, BLUE_300, BLUE_400, BLUE_500, BLUE_600, BRONZO, DANGER,
    DIFF_ADD, DIFF_REMOVE, EMERALD_400, GRAY_100, GRAY_200, GRAY_300, GRAY_400, GRAY_500, GRAY_600,
    GRAY_700, GRAY_900, GREEN_300, GREEN_400, GREEN_500, GREEN_600, INK, RED_200, RED_300, RED_400,
    RED_500, RED_600, SKY_400, STATUS_BLUE, STATUS_SAGE, STATUS_SLATE, SURFACE_BASE,
    SURFACE_OVERLAY, SURFACE_PANEL, SURFACE_RAISED, SURFACE_SUNKEN, WHITE, YELLOW_500,
};
/// `--color-ink-secondary` and `--color-ink-faint`: white at these alphas.
pub const INK_SECONDARY: f32 = 0.55;
pub const INK_FAINT: f32 = 0.35;

/// The window's first size, in logical pixels.
pub const WINDOW: (f32, f32) = (1440.0, 900.0);
pub const TOP_BAR_H: f32 = 40.0;
/// Room for the macOS traffic lights at a bar's left end (`TRAFFIC_LIGHT_PAD_PX`).
pub const TRAFFIC_LIGHT_PAD: f32 = 80.0;
/// Where the traffic lights sit (`trafficLightPosition`).
pub const TRAFFIC_LIGHTS: (f32, f32) = (16.0, 13.0);
pub const SIDEBAR_W: f32 = 256.0;
pub const COMPOSER_MAX_W: f32 = 800.0;
/// The terminal font size, in points.
pub const TERM_FONT: f32 = 12.0;
/// A card's header (`py-2.5` around the 22 px action cluster, which keeps
/// its space while hidden; its border included).
pub const CARD_HEADER_H: f32 = 43.0;
/// The grid card's `RowResizeHandle` strip under its status bar.
pub const CARD_FOOT_H: f32 = 6.0;
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

/// `ProjectIcon`'s and `RunIcon`'s tint for a mapped icon with no colour.
pub const ICON_DEFAULT: Rgb = 0x6b7280;

/// `ProjectIcon`'s fallback: a folder drawn with its own path.
pub const PROJECT_FOLDER: &str = "<svg xmlns=\"http://www.w3.org/2000/svg\" viewBox=\"0 0 24 24\" fill=\"none\" stroke=\"#ffffff\" stroke-width=\"1.5\"><path d=\"M3 7v10a2 2 0 002 2h14a2 2 0 002-2V9a2 2 0 00-2-2h-6l-2-2H5a2 2 0 00-2 2z\"/></svg>";

/// `ICON_COLOR_PALETTE`.
pub const ICON_COLORS: [Rgb; 8] = [
    ICON_DEFAULT,
    0x3b82f6,
    0x8b5cf6,
    0xec4899,
    0xef4444,
    0xf97316,
    0xeab308,
    0x22c55e,
];

/// `WORKSPACE_ICON_OPTIONS`: what a workspace stores, its glyph and label.
pub const WORKSPACE_ICONS: [(&str, &str, &str); 16] = [
    ("User", "user", "Personal"),
    ("Briefcase", "briefcase", "Work"),
    ("Building2", "building-2", "Company"),
    ("Users", "users", "Team"),
    ("Code", "code", "Code"),
    ("Star", "star", "Star"),
    ("Heart", "heart", "Favorite"),
    ("Coffee", "coffee", "Side Project"),
    ("Layers", "layers", "Layers"),
    ("Rocket", "rocket", "Launch"),
    ("FlaskConical", "flask-conical", "Lab"),
    ("Globe", "globe", "Web"),
    ("Shield", "shield", "Security"),
    ("Zap", "zap", "Fast"),
    ("Gamepad2", "gamepad-2", "Game"),
    ("BookOpen", "book-open", "Docs"),
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
    /// Behind selected text, as the renderer's terminal theme has it.
    pub const SELECTION: Rgb = 0x3f3f46;
    pub const ANSI16: [Rgb; 16] = [
        0x27272a, 0xef4444, 0x22c55e, 0xeab308, 0x3b82f6, 0xa855f7, 0x06b6d4, 0xd4d4d8, 0x52525b,
        0xf87171, 0x4ade80, 0xfacc15, 0x60a5fa, 0xc084fc, 0x22d3ee, 0xfafafa,
    ];
}

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
/// `--color-ink-secondary` and `--color-ink-faint`: white at these alphas.
pub const INK_SECONDARY: f32 = 0.55;
pub const INK_FAINT: f32 = 0.35;

/// The window's first size, in logical pixels.
pub const WINDOW: (f32, f32) = (1440.0, 900.0);
pub const TOP_BAR_H: f32 = 40.0;
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

/// The icon's SVG with its stroke width set, ready to tint.
pub fn icon(name: &str, stroke: f32) -> Option<String> {
    let src = match name {
        "arrow-up" => include_str!("../assets/icons/arrow-up.svg"),
        "bot" => include_str!("../assets/icons/bot.svg"),
        "chevron-down" => include_str!("../assets/icons/chevron-down.svg"),
        "chevrons-left" => include_str!("../assets/icons/chevrons-left.svg"),
        "folder" => include_str!("../assets/icons/folder.svg"),
        "folder-git-2" => include_str!("../assets/icons/folder-git-2.svg"),
        "git-branch" => include_str!("../assets/icons/git-branch.svg"),
        "layers" => include_str!("../assets/icons/layers.svg"),
        "list-todo" => include_str!("../assets/icons/list-todo.svg"),
        "monitor" => include_str!("../assets/icons/monitor.svg"),
        "panel-left" => include_str!("../assets/icons/panel-left.svg"),
        "plus" => include_str!("../assets/icons/plus.svg"),
        "rotate-ccw" => include_str!("../assets/icons/rotate-ccw.svg"),
        "sliders-horizontal" => include_str!("../assets/icons/sliders-horizontal.svg"),
        "sparkles" => include_str!("../assets/icons/sparkles.svg"),
        "terminal" => include_str!("../assets/icons/terminal.svg"),
        "workflow" => include_str!("../assets/icons/workflow.svg"),
        _ => return None,
    };
    Some(
        src.replace("stroke-width=\"2\"", &format!("stroke-width=\"{stroke}\""))
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
        for name in [
            "arrow-up",
            "bot",
            "chevron-down",
            "chevrons-left",
            "folder",
            "folder-git-2",
            "git-branch",
            "layers",
            "list-todo",
            "monitor",
            "panel-left",
            "plus",
            "rotate-ccw",
            "sliders-horizontal",
            "sparkles",
            "terminal",
            "workflow",
        ] {
            let svg = icon(name, 1.5).unwrap_or_else(|| panic!("{name}"));
            assert!(
                svg.contains("stroke-width=\"1.5\"") && svg.contains("#ffffff"),
                "{name}"
            );
        }
        assert!(icon("nope", 2.0).is_none());
    }
}

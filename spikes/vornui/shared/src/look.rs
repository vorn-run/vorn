//! Today's main screen, as data both prototypes draw from: the colors from
//! `src/renderer/theme.css` and Tailwind's grays, the sizes from the
//! components' classes, the icons (lucide, ISC, `icons/LICENSE`) and the logo.
//! Each prototype lays the screen out with its own elements; only the values
//! are shared, so a difference in the screenshots is the framework's.

/// 0xRRGGBB.
pub type Rgb = u32;

pub const SURFACE_BASE: Rgb = 0x0d0d0f;
pub const SURFACE_RAISED: Rgb = 0x141416;
pub const BRONZO: Rgb = 0xc9972a;
pub const INK: Rgb = 0xfaf9f7;
/// Tailwind v4's grays, as sRGB.
pub const GRAY_200: Rgb = 0xe5e7eb;
pub const GRAY_300: Rgb = 0xd1d5dc;
pub const GRAY_400: Rgb = 0x99a1af;
pub const GRAY_500: Rgb = 0x6a7282;
pub const GRAY_600: Rgb = 0x4a5565;
pub const WHITE: Rgb = 0xffffff;

/// The window the screenshots and benches use, in logical pixels.
pub const WINDOW: (f32, f32) = (1440.0, 900.0);
/// HiDPI: every render is at this device scale.
pub const SCALE: f32 = 2.0;
pub const TOP_BAR_H: f32 = 40.0;
pub const COMPOSER_MAX_W: f32 = 800.0;
/// The terminal font size, in points.
pub const TERM_FONT: f32 = 12.0;
/// Gap between panes in the terminal grid.
pub const GAP: f32 = 2.0;
pub const PANE_BORDER: Rgb = 0x26262a;

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
pub const TIP: &str = "Select a project to get started";
pub const PILLS: [(&str, &str); 3] = [
    ("monitor", "Terminals"),
    ("list-todo", "Tasks"),
    ("workflow", "Workflows"),
];
/// The composer's chips: icon, label, icon size, whether it is dimmed.
pub const CHIPS: [(&str, &str, f32, bool); 5] = [
    ("folder", "Select project", 13.0, true),
    ("bot", "Agent", 14.0, false),
    ("sparkles", "Default model", 13.0, false),
    ("folder-git-2", "", 13.0, true),
    ("git-branch", "main", 12.0, false),
];
/// The sessions chip's count.
pub const SESSIONS: &str = "3";

pub const LOGO_PNG: &[u8] = include_bytes!("../assets/vorn-logo.png");

/// The icon's SVG with its stroke width set, for a rasterizer that tints it.
pub fn icon(name: &str, stroke: f32) -> Option<String> {
    let src = match name {
        "monitor" => include_str!("../icons/monitor.svg"),
        "list-todo" => include_str!("../icons/list-todo.svg"),
        "workflow" => include_str!("../icons/workflow.svg"),
        "panel-left" => include_str!("../icons/panel-left.svg"),
        "sliders-horizontal" => include_str!("../icons/sliders-horizontal.svg"),
        "rotate-ccw" => include_str!("../icons/rotate-ccw.svg"),
        "plus" => include_str!("../icons/plus.svg"),
        "layers" => include_str!("../icons/layers.svg"),
        "chevron-down" => include_str!("../icons/chevron-down.svg"),
        "arrow-up" => include_str!("../icons/arrow-up.svg"),
        "folder" => include_str!("../icons/folder.svg"),
        "bot" => include_str!("../icons/bot.svg"),
        "sparkles" => include_str!("../icons/sparkles.svg"),
        "folder-git-2" => include_str!("../icons/folder-git-2.svg"),
        "git-branch" => include_str!("../icons/git-branch.svg"),
        _ => return None,
    };
    Some(
        src.replace("stroke-width=\"2\"", &format!("stroke-width=\"{stroke}\""))
            .replace("currentColor", "#ffffff"),
    )
}

/// `c` at `alpha` (0..=1) over `under`: Tailwind's `white/[0.06]` and kin
/// on an opaque surface.
pub fn over(c: Rgb, alpha: f32, under: Rgb) -> Rgb {
    let ch = |s: u32| {
        let a = ((c >> s) & 0xff) as f32;
        let b = ((under >> s) & 0xff) as f32;
        ((a * alpha + b * (1.0 - alpha)).round() as u32) << s
    };
    ch(16) | ch(8) | ch(0)
}

#[cfg(test)]
mod tests {
    #[test]
    fn every_icon_loads() {
        for name in super::PILLS
            .iter()
            .map(|p| p.0)
            .chain(super::CHIPS.iter().map(|c| c.0))
        {
            assert!(super::icon(name, 1.5).is_some(), "{name}");
        }
    }

    #[test]
    fn blend() {
        assert_eq!(super::over(0xffffff, 0.0, 0x0d0d0f), 0x0d0d0f);
        assert_eq!(super::over(0xffffff, 1.0, 0x0d0d0f), 0xffffff);
    }
}

//! The smaller panels: Appearance, General (with `ShellPicker`),
//! Notifications, Skills & MCP, Keys and About.

use serde_json::Value;

use vorn_theme::{Appearance as Look, Chrome, Materials, Scheme, Theme};

use super::{button, card, card_row, header, row, section, segmented, set, svg, toggle, Seg};
use crate::act::{Act, Acts};
use crate::look;
use crate::screen::{agent_icon, c, ico, white, Screen};
use crate::settings::{
    self, describe_field, describe_usage, sigil, SetAct, UpdateAction, UpdateStatus,
};
use crate::state::{now_ms, MenuKind};
use crate::ui::{
    div, field, icon, image, text, tooltip, Action, El, Id, Rgba, Role, Sense, TextStyle,
};

fn defaults<'a>(s: &Screen<'a>) -> Option<&'a Value> {
    s.store.config.get("defaults")
}

fn flag(s: &Screen<'_>, key: &str, default: bool) -> bool {
    defaults(s)
        .and_then(|d| d.get(key))
        .and_then(Value::as_bool)
        .unwrap_or(default)
}

fn num(s: &Screen<'_>, key: &str) -> Option<f64> {
    defaults(s).and_then(|d| d.get(key)).and_then(Value::as_f64)
}

fn default_act(key: &str, v: impl ToString) -> SetAct {
    SetAct::Default(key.to_owned(), v.to_string())
}

fn seg_act(key: &str, v: impl ToString) -> Act {
    set(default_act(key, v))
}

/// The Card Row Height slider's track.
pub const ROW_SLIDER: Id = Id(0x726f_7768_6569_6768);
/// The notification volume slider's track.
pub const VOLUME_SLIDER: Id = Id(0x766f_6c75_6d65_7364);

/// A range input: the accent up to a white knob.
fn slider(id: Id, w: f32, frac: f32, label: &str) -> El {
    let frac = frac.clamp(0.0, 1.0);
    let knob = 16.0;
    let x = frac * (w - knob);
    div()
        .id(id)
        .sense(Sense::MOVE)
        .w(w)
        .h(16.0)
        .shrink0()
        .role(Role::Slider, label)
        .child(
            div()
                .absolute(0.0, 4.0)
                .size(w, 8.0)
                .rounded(4.0)
                .bg(c(look::SLIDER_TRACK)),
        )
        .child(
            div()
                .absolute(0.0, 4.0)
                .size(x + knob / 2.0, 8.0)
                .rounded(4.0)
                .bg(c(look::BRONZO)),
        )
        .child(
            div()
                .absolute(x, 0.0)
                .size(knob, knob)
                .rounded(knob / 2.0)
                // literal-colour: a knob is white on every theme, as system switches are.
                .bg(Rgba::WHITE)
                .border(1.0, crate::ui::theme::shadow(0.12)),
        )
}

/// Inline icons the page draws, stroked on a 24 grid.
mod glyph {
    pub const MONITOR: &str =
        r#"<rect x="2" y="3" width="20" height="14" rx="2"/><path d="M8 21h8M12 17v4"/>"#;
    pub const SUN: &str = r#"<circle cx="12" cy="12" r="4"/><path d="M12 2v2M12 20v2M4.93 4.93l1.41 1.41M17.66 17.66l1.41 1.41M2 12h2M20 12h2M6.34 17.66l-1.41 1.41M19.07 4.93l-1.41 1.41"/>"#;
    pub const MOON: &str = r#"<path d="M12 3a6 6 0 009 9 9 9 0 11-9-9z"/>"#;
    pub const INFO: &str = r#"<circle cx="12" cy="12" r="10"/><path d="M12 16v-4M12 8h.01"/>"#;
    pub const PALETTE: &str = r#"<path d="M12 22a10 10 0 110-20c5.5 0 10 4 10 9a5 5 0 01-5 5h-1.8a1.7 1.7 0 00-1.2 2.9A1.7 1.7 0 0112 22z"/><circle cx="7.5" cy="10.5" r="1"/><circle cx="12" cy="7.5" r="1"/><circle cx="16.5" cy="10.5" r="1"/>"#;
    pub const CONTRAST: &str =
        r##"<circle cx="12" cy="12" r="10"/><path d="M12 2a10 10 0 010 20z" fill="#ffffff"/>"##;
    pub const CHEVRON_DOWN: &str = r#"<path d="M6 9l6 6 6-6"/>"#;
    pub const CHEVRON_UP: &str = r#"<path d="M18 15l-6-6-6 6"/>"#;
    pub const SEARCH: &str = r#"<circle cx="11" cy="11" r="7"/><path d="M21 21l-4.3-4.3"/>"#;
    pub const TERMINAL: &str = r#"<path d="M4 17l6-6-6-6M12 19h8"/>"#;
    pub const ARROW_UP_RIGHT: &str = r#"<path d="M7 17L17 7M7 7h10v10"/>"#;
    pub const ELLIPSIS: &str = r#"<circle cx="5" cy="12" r="1"/><circle cx="12" cy="12" r="1"/><circle cx="19" cy="12" r="1"/>"#;
}

/// What each Appearance, General and About row, tile and chip says when hovered.
pub(crate) mod tip {
    #[cfg(target_os = "macos")]
    pub const SCHEME: &str =
        "Which theme Vorn uses. System follows your Mac's light or dark setting.";
    #[cfg(not(target_os = "macos"))]
    pub const SCHEME: &str =
        "Which theme Vorn uses. System follows your computer's light or dark setting.";
    pub const THEME: &str = "The colours of Vorn's surfaces, text and terminals.";
    #[cfg(target_os = "macos")]
    pub const LIGHT_THEME: &str = "The theme used while your Mac is in light mode.";
    #[cfg(not(target_os = "macos"))]
    pub const LIGHT_THEME: &str = "The theme used while your computer is in light mode.";
    #[cfg(target_os = "macos")]
    pub const DARK_THEME: &str = "The theme used while your Mac is in dark mode.";
    #[cfg(not(target_os = "macos"))]
    pub const DARK_THEME: &str = "The theme used while your computer is in dark mode.";
    pub const THEME_INFO: &str = "Shows the theme's terminal colours and who made it.";
    pub const MORE_THEMES: &str = "Opens the extensions catalog, filtered to themes.";
    pub const SURFACES: &str = "How Vorn draws its sidebar, toolbar and panels.";
    pub const SOLID: &str = "Opaque surfaces everywhere.";
    pub const GLASS: &str =
        "The sidebar and toolbar show the system's translucent material; buttons sit on glass.";
    pub const TINTED_GLASS: &str = "As Glass, tinted with the theme's colours.";
    pub const NO_GLASS: &str = "Off while Reduce transparency is on in the system settings.";
    pub const CHROME: &str =
        "Paint Vorn's sidebar and panels in the theme's colours instead of Vorn's own.";
    pub const CONTRAST: &str =
        "Lighten or darken text that would be hard to read, keeping its colour.";
    pub const FONT: &str = "The font every terminal uses.";
    pub const SIZE: &str = "The text size in terminals.";
    pub const LIGATURES: &str = "This font joins symbols like != and -> into one shape.";
    pub const LAYOUT: &str = "How sessions are arranged in the window.";
    pub const GRID: &str = "Sessions side by side in a grid.";
    pub const TABS: &str = "One session at a time, switched with tabs.";
    pub const LIST: &str = "A list of sessions beside the open one.";
    pub const CARD_HEIGHT: &str = "How tall each session card is in the grid.";
    #[cfg(target_os = "macos")]
    pub const LOGIN: &str = "Start Vorn when you sign in to your Mac.";
    #[cfg(not(target_os = "macos"))]
    pub const LOGIN: &str = "Start Vorn when you sign in to your computer.";
    pub const REOPEN: &str = "Bring back the sessions that were open when Vorn quit.";
    pub const AGENT: &str = "The agent a new session starts with.";
    #[cfg(target_os = "macos")]
    pub const INSTALLED: &str = "Agents on this Mac.";
    #[cfg(not(target_os = "macos"))]
    pub const INSTALLED: &str = "Agents on this computer.";
    pub const AVAILABLE: &str = "Agents you can add.";
    #[cfg(target_os = "macos")]
    pub const SET_UP: &str =
        "This agent's adapter is installed but its command isn't on this Mac yet.";
    #[cfg(not(target_os = "macos"))]
    pub const SET_UP: &str =
        "This agent's adapter is installed but its command isn't on this computer yet.";
    pub const INSTALL: &str = "Adds this agent from the catalog.";
    pub const MORE_AGENTS: &str = "Opens the extensions catalog, filtered to agents.";
    pub const SHELL: &str = "The shell a new terminal session runs.";
    pub const CHOOSE_PATH: &str = "Use a shell that isn't in the list.";
    pub const BEHIND: &str = "Your running terminals still use the last version; restarting moves them over without ending them.";
    pub const RESTART_VORN: &str =
        "Restart Vorn so your terminals move to this version; they keep running.";
    pub const CHECK: &str = "Look for a newer version of Vorn now.";
    pub const CHANNEL: &str =
        "Beta gets new versions first; Stable gets them once they have been tested.";
    pub const RESTART_UPDATE: &str =
        "Install the new version; Vorn reopens in a few seconds and your sessions keep running.";
    pub const WHATS_NEW: &str = "Opens the release notes for this version.";
    pub const UPDATE_EXTENSIONS: &str = "Update the listed extensions to their newest versions.";
    pub const DRAWING: &str =
        "Pixels look like the desktop app where the terminal shows images; Text works in any terminal.";
}

fn glyph_icon(body: &str, size: f32, color: Rgba) -> El {
    icon(svg(body, 2.0), size, color)
}

/// What each Surfaces tile stores under `materials`.
pub(super) fn surface_act(m: Materials) -> SetAct {
    default_act(crate::appearance::MATERIALS, format!("\"{}\"", m.key()))
}

/// What the Theme colours on surfaces toggle stores under `interfaceStyle`.
pub(super) fn chrome_act(on: bool) -> SetAct {
    let chrome = if on { Chrome::Tinted } else { Chrome::Neutral };
    default_act(crate::appearance::CHROME, format!("\"{}\"", chrome.key()))
}

/// What the Readable contrast toggle stores under `contrastCorrection`.
pub(super) fn contrast_act(on: bool) -> SetAct {
    default_act(crate::appearance::CONTRAST, on)
}

/// The sizes the Aa steps write to `fontSize`, the glyph size each is
/// drawn at, and its name.
const FONT_PRESETS: [(i64, f32, &str); 3] = [
    (11, 11.0, "Small"),
    (13, 14.0, "Medium"),
    (16, 18.0, "Large"),
];

/// The field the theme picker searches in.
pub const THEME_SEARCH: Id = Id(0x7468_656d_6573_7263);

pub(super) fn appearance(s: &Screen<'_>) -> El {
    // On a terminal UI how it draws comes first, as it decides how all else reads.
    let drawing = s.term_drawing.map(|d| card(vec![drawing_row(s, d)]));
    div()
        .col()
        .child(header("Appearance", "", None))
        .children(drawing)
        .child(card(colour_rows(s)))
        .child(section("Look"))
        .child(card(look_rows(s)))
        .child(section("Terminal"))
        .child(card(terminal_rows(s)))
        .child(section("Sessions"))
        .child(card(session_rows(s)))
}

/// The id of a theme's preview tile, which the app reads the hover of.
pub fn theme_tile_id(theme: &str) -> Id {
    Id::new(&format!("theme-tile:{theme}"))
}

/// The theme stored for `side`, or Vorn's own for it.
fn chosen_theme<'a>(s: &Screen<'a>, side: Look) -> &'a Theme {
    let prefs = crate::appearance::prefs(defaults(s));
    let id = match side {
        Look::Light => prefs.light,
        Look::Dark => prefs.dark,
    };
    let installed = s.st.settings.appearance.installed();
    crate::appearance::find(installed, &id).unwrap_or(match side {
        Look::Light => vorn_theme::vorn_light(),
        Look::Dark => vorn_theme::vorn_dark(),
    })
}

/// The colour scheme, then a theme row for each side it can show.
fn colour_rows(s: &Screen<'_>) -> Vec<El> {
    use crate::appearance as a;
    let prefs = a::prefs(defaults(s));
    let seg = |label, body: &str, scheme: Scheme| {
        let on = prefs.scheme == scheme;
        let fg = if on {
            c(look::WHITE)
        } else {
            c(look::GRAY_500)
        };
        Seg {
            label,
            icon: Some(glyph_icon(body, 14.0, fg)),
            act: seg_act(a::SCHEME, format!("\"{}\"", scheme.key())),
            on,
            disabled: false,
        }
    };
    let scheme = segmented(
        s,
        a::SCHEME,
        vec![
            seg("System", glyph::MONITOR, Scheme::System),
            seg("Light", glyph::SUN, Scheme::Light),
            seg("Dark", glyph::MOON, Scheme::Dark),
        ],
        10.0,
    );
    let mut rows = vec![card_row(s, None, "Appearance", Some(tip::SCHEME), scheme)];
    for r in a::theme_rows(prefs.scheme) {
        let mark = (prefs.scheme == Scheme::System).then(|| {
            let body = match r.look {
                Look::Light => glyph::SUN,
                Look::Dark => glyph::MOON,
            };
            glyph_icon(body, 14.0, c(look::GRAY_500))
        });
        let tip = match (prefs.scheme, r.look) {
            (Scheme::System, Look::Light) => tip::LIGHT_THEME,
            (Scheme::System, Look::Dark) => tip::DARK_THEME,
            _ => tip::THEME,
        };
        rows.push(card_row(
            s,
            mark,
            r.label,
            Some(tip),
            theme_control(s, r.look, tip),
        ));
    }
    rows
}

/// The theme chip that opens the picker, and the (i) beside it.
fn theme_control(s: &Screen<'_>, side: Look, tip: &str) -> El {
    let t = chosen_theme(s, side);
    let kind = MenuKind::ThemePicker(side);
    let open = s.st.menu.as_ref().is_some_and(|m| m.kind == kind);
    let id = s.acts.on("theme-chip", Act::Open(kind));
    let chip = chip(s, id, &t.name, open)
        .pl(5.0)
        .child(theme_swatch(t, 18.0))
        .child(
            text(&t.name, 13.0, c(look::GRAY_200))
                .line_height(18.0)
                .silent(),
        )
        .child(chevron(open));
    let chip = tooltip(s.ui, chip, id, tip, None);
    let info_id = s
        .acts
        .on("theme-info", Act::Open(MenuKind::ThemeInfo(t.id.clone())));
    let info = icon_button(s, info_id, glyph::INFO, "Theme details", tip::THEME_INFO);
    div().row().items_center().gap(6.0).child(chip).child(info)
}

/// A dropdown chip's frame, lit while hovered or open.
fn chip(s: &Screen<'_>, id: Id, name: &str, open: bool) -> El {
    let lit = s.ui.hover(id).max(if open { 1.0 } else { 0.0 });
    div()
        .id(id)
        .on(Action::Click)
        .row()
        .items_center()
        .gap(8.0)
        .h(28.0)
        .pl(10.0)
        .pr(8.0)
        .rounded(4.0)
        .bg(white(0.04).mix(white(0.08), lit))
        .border(1.0, white(0.08))
        .role(Role::Button, name)
        .ring(4.0)
}

fn chevron(open: bool) -> El {
    let body = if open {
        glyph::CHEVRON_UP
    } else {
        glyph::CHEVRON_DOWN
    };
    glyph_icon(body, 12.0, c(look::GRAY_500))
}

/// A theme's window colour with its accent in the corner.
fn theme_swatch(t: &Theme, size: f32) -> El {
    use vorn_theme::Token as T;
    let dot = (size / 3.0).round();
    div()
        .size(size, size)
        .shrink0()
        .rounded(3.0)
        .clip()
        .bg(Rgba::from(t.get(T::SurfaceBase)))
        .border(1.0, white(0.12))
        .child(
            div()
                .absolute(size - dot - 3.0, size - dot - 3.0)
                .size(dot, dot)
                .rounded(dot / 2.0)
                .bg(Rgba::from(t.get(T::Accent))),
        )
}

/// A neutral 26 px icon button: gray, white on a faint wash when hovered.
fn icon_button(s: &Screen<'_>, id: Id, body: &str, label: &str, tip: &str) -> El {
    let hot = s.ui.hover(id);
    let b = div()
        .id(id)
        .on(Action::Click)
        .size(26.0, 26.0)
        .shrink0()
        .center()
        .rounded(4.0)
        .bg(white(0.0).mix(white(0.06), hot))
        .role(Role::Button, label)
        .ring(4.0)
        .child(glyph_icon(
            body,
            14.0,
            c(look::GRAY_400).mix(c(look::WHITE), hot),
        ));
    tooltip(s.ui, b, id, tip, None)
}

/// Surfaces, theme colours on them, and readable contrast.
fn look_rows(s: &Screen<'_>) -> Vec<El> {
    use crate::appearance as a;
    let prefs = a::prefs(defaults(s));
    let reduced = s.ui.reduced_transparency();
    let m = prefs.materials;
    let tiles = div()
        .row()
        .gap(8.0)
        .role(Role::RadioGroup, "Surfaces")
        .child(surface_tile(s, Materials::Solid, "Solid", m, false))
        .child(surface_tile(s, Materials::Native, "Glass", m, reduced))
        .child(surface_tile(
            s,
            Materials::Themed,
            "Tinted glass",
            m,
            reduced,
        ));
    let surfaces_tip = if cfg!(windows) {
        "How Vorn draws its sidebar, toolbar and panels. The sidebar changes at the next launch."
    } else {
        tip::SURFACES
    };
    let tinted = prefs.chrome == Chrome::Tinted;
    let chrome = toggle(
        s,
        a::CHROME,
        tinted,
        set(chrome_act(!tinted)),
        false,
        "Theme colours on surfaces",
    );
    let contrast = toggle(
        s,
        a::CONTRAST,
        prefs.contrast,
        set(contrast_act(!prefs.contrast)),
        false,
        "Readable contrast",
    );
    vec![
        card_row(s, None, "Surfaces", Some(surfaces_tip), tiles).py(12.0),
        card_row(
            s,
            Some(glyph_icon(glyph::PALETTE, 14.0, c(look::GRAY_500))),
            "Theme colours on surfaces",
            Some(tip::CHROME),
            chrome,
        ),
        card_row(
            s,
            Some(glyph_icon(glyph::CONTRAST, 14.0, c(look::GRAY_500))),
            "Readable contrast",
            Some(tip::CONTRAST),
            contrast,
        ),
    ]
}

/// One Surfaces choice: a small window drawn solid, over glass, or over
/// tinted glass, in the theme's own colours.
fn surface_tile(
    s: &Screen<'_>,
    m: Materials,
    label: &str,
    chosen: Materials,
    disabled: bool,
) -> El {
    let term = crate::ui::palette::terminal();
    let base = c(look::SURFACE_BASE);
    let panel = c(look::SURFACE_PANEL);
    let desk = Some((term.ansi[4].mix(base, 0.3), term.ansi[5].mix(base, 0.3)));
    let tint = term.ansi[4];
    let (desk, sidebar, content) = match m {
        Materials::Solid => (None, panel, base),
        Materials::Native => (desk, panel.alpha(0.7), base.alpha(0.82)),
        Materials::Themed => (
            desk,
            panel.mix(tint, 0.3).alpha(0.8),
            base.mix(tint, 0.18).alpha(0.9),
        ),
    };
    let line = |w: f32, a: f32| div().size(w, 3.0).rounded(1.5).bg(white(a));
    let mut pic = div().size(128.0, 80.0).rounded(4.0).clip().bg(base);
    if let Some((a, b)) = desk {
        pic = pic
            .child(div().absolute(0.0, 0.0).size(128.0, 80.0).bg(a))
            .child(div().absolute(64.0, 0.0).size(64.0, 80.0).bg(b));
    }
    pic = pic
        .child(
            div()
                .absolute(0.0, 0.0)
                .size(36.0, 80.0)
                .col()
                .gap(5.0)
                .p(7.0)
                .bg(sidebar)
                .border_right(1.0, white(0.08))
                .child(line(18.0, 0.5))
                .child(line(14.0, 0.3))
                .child(line(16.0, 0.3)),
        )
        .child(
            div()
                .absolute(36.0, 0.0)
                .size(92.0, 80.0)
                .col()
                .gap(5.0)
                .p(9.0)
                .bg(content)
                .child(line(52.0, 0.6))
                .child(line(38.0, 0.35))
                .child(div().size(20.0, 6.0).rounded(2.0).bg(c(look::BRONZO))),
        );
    let says = match m {
        Materials::Solid => tip::SOLID,
        Materials::Native => tip::GLASS,
        Materials::Themed => tip::TINTED_GLASS,
    };
    let says = if disabled {
        format!("{says} {}", tip::NO_GLASS)
    } else {
        says.to_owned()
    };
    let act = set(surface_act(m));
    let on = m == chosen && !disabled;
    tile(s, "surface", act, pic, label, on, disabled, &says)
}

/// A picture choice: the picture ringed in the accent when chosen, and its
/// label under it on a faint pill.
#[allow(clippy::too_many_arguments)]
fn tile(
    s: &Screen<'_>,
    place: &str,
    act: Act,
    pic: El,
    label: &str,
    on: bool,
    disabled: bool,
    tip: &str,
) -> El {
    let id = Acts::id(place, &act);
    if !disabled {
        s.acts.bind(id, act);
    }
    let hot = s.ui.hover(id);
    let edge = if on {
        c(look::BRONZO)
    } else {
        white(0.0).mix(white(0.16), hot)
    };
    let ink = if on {
        c(look::WHITE)
    } else {
        c(look::GRAY_400)
    };
    let mut el = div()
        .id(id)
        .col()
        .items_center()
        .gap(6.0)
        .role(Role::RadioButton, label)
        .selected(on)
        .child(div().p(2.0).rounded(6.0).border(2.0, edge).child(pic))
        .child(
            div()
                .px(8.0)
                .py(1.0)
                .rounded(4.0)
                .bg(white(if on { 0.12 } else { 0.0 }))
                .child(text(label, 12.0, ink).line_height(16.0).silent()),
        );
    el = if disabled {
        el.opacity(0.4).disabled(true)
    } else {
        el.on(Action::Click).ring(4.0)
    };
    tooltip(s.ui, el, id, tip, None)
}

/// The preview, the font and its size.
fn terminal_rows(s: &Screen<'_>) -> Vec<El> {
    let size = num(s, "fontSize").unwrap_or(13.0).round() as i64;
    let family = s.ui.text.term_family();
    vec![
        terminal_preview(family, size),
        card_row(s, None, "Font", Some(tip::FONT), font_chip(s, family)),
        card_row(s, None, "Size", Some(tip::SIZE), size_control(s, size)),
    ]
}

/// Two terminal lines in the theme's colours and the chosen face and size.
fn terminal_preview(family: &str, size: i64) -> El {
    let term = crate::ui::palette::terminal();
    let px = look::TERM_FONT * size as f32 / 13.0;
    let span = |t: &str, color: Rgba| {
        text(t, px, color)
            .family(family)
            .line_height((px * 1.5).round())
            .silent()
    };
    let fg = term.foreground;
    div()
        .col()
        .mx(12.0)
        .mt(12.0)
        .mb(4.0)
        .px(12.0)
        .py(10.0)
        .rounded(4.0)
        .bg(term.background)
        .border(1.0, white(0.06))
        .role(Role::Image, "Terminal preview")
        .child(
            div()
                .row()
                .child(span("~/dev/vorn ", term.ansi[2]))
                .child(span("main ", term.ansi[4]))
                .child(span("\u{276f} ", term.ansi[3]))
                .child(span("cargo test -p vornui", fg)),
        )
        .child(
            div()
                .row()
                .child(span("test result: ", fg.alpha(0.55)))
                .child(span("ok", term.ansi[2]))
                .child(span(". 216 passed; 0 failed => 1.12s", fg)),
        )
}

/// The chip naming the terminal font in its own face.
fn font_chip(s: &Screen<'_>, family: &str) -> El {
    let open =
        s.st.menu
            .as_ref()
            .is_some_and(|m| m.kind == MenuKind::FontPicker);
    let id = s.acts.on("font-chip", Act::Open(MenuKind::FontPicker));
    let chip = chip(s, id, family, open)
        .child(
            text(family, 13.0, c(look::GRAY_200))
                .family(family)
                .line_height(18.0)
                .silent(),
        )
        .child(chevron(open));
    tooltip(s.ui, chip, id, tip::FONT, None)
}

/// Three Aa steps, then the exact size.
fn size_control(s: &Screen<'_>, size: i64) -> El {
    let mut steps = div()
        .row()
        .items_center()
        .gap(2.0)
        .p(2.0)
        .rounded(4.0)
        .bg(white(0.04))
        .role(Role::RadioGroup, "Size");
    for (v, glyph_px, label) in FONT_PRESETS {
        let on = size == v;
        let id = s.acts.on("font-preset", seg_act("fontSize", v));
        let hot = s.ui.hover(id);
        let fg = if on {
            c(look::WHITE)
        } else {
            c(look::GRAY_500).mix(c(look::GRAY_300), hot)
        };
        steps = steps.child(
            div()
                .id(id)
                .on(Action::Click)
                .h(28.0)
                .w(36.0)
                .center()
                .rounded(3.0)
                .bg(white(if on { 0.1 } else { 0.0 }))
                .role(Role::RadioButton, label)
                .selected(on)
                .child(
                    text("Aa", glyph_px, fg)
                        .line_height(glyph_px + 4.0)
                        .silent(),
                ),
        );
    }
    let exact = div().w(44.0).row().justify_end().child(
        text(format!("{size} px"), 12.0, c(look::GRAY_300))
            .line_height(16.0)
            .silent(),
    );
    div()
        .row()
        .items_center()
        .gap(12.0)
        .child(steps)
        .child(exact)
}

/// Layout as three pictures, and the card height.
fn session_rows(s: &Screen<'_>) -> Vec<El> {
    let layout = defaults(s)
        .and_then(|d| d.get("layoutMode"))
        .and_then(Value::as_str)
        .unwrap_or("grid");
    let tiles = div()
        .row()
        .gap(8.0)
        .role(Role::RadioGroup, "Layout")
        .child(layout_tile(s, "grid", "Grid", layout))
        .child(layout_tile(s, "tabs", "Tabs", layout))
        .child(layout_tile(s, "list", "List", layout));
    let h = s.st.settings.row_height.round();
    let height = div()
        .row()
        .items_center()
        .gap(12.0)
        .child(slider(
            ROW_SLIDER,
            180.0,
            (h - 100.0) / 500.0,
            "Card height",
        ))
        .child(
            div().w(48.0).row().justify_end().child(
                text(format!("{h} px"), 12.0, c(look::GRAY_300))
                    .line_height(16.0)
                    .silent(),
            ),
        );
    vec![
        card_row(s, None, "Layout", Some(tip::LAYOUT), tiles).py(12.0),
        card_row(s, None, "Card height", Some(tip::CARD_HEIGHT), height),
    ]
}

/// A layout drawn small: four cards, one card under tabs, or a list
/// beside the chosen session.
fn layout_tile(s: &Screen<'_>, key: &str, label: &str, chosen: &str) -> El {
    let block = |w: f32, h: f32, a: f32| div().size(w, h).rounded(2.0).bg(white(a));
    let pair = || {
        div()
            .row()
            .gap(4.0)
            .child(block(47.0, 27.0, 0.12))
            .child(block(47.0, 27.0, 0.12))
    };
    let pic = div()
        .size(110.0, 70.0)
        .rounded(4.0)
        .clip()
        .bg(c(look::SURFACE_BASE))
        .p(6.0)
        .gap(4.0);
    let pic = match key {
        "grid" => pic.col().child(pair()).child(pair()),
        "tabs" => pic
            .col()
            .child(
                div()
                    .row()
                    .gap(3.0)
                    .child(block(24.0, 8.0, 0.24))
                    .child(block(24.0, 8.0, 0.1))
                    .child(block(24.0, 8.0, 0.1)),
            )
            .child(block(98.0, 46.0, 0.12)),
        _ => pic
            .row()
            .child(
                div()
                    .col()
                    .gap(3.0)
                    .child(block(28.0, 8.0, 0.24))
                    .child(block(28.0, 8.0, 0.1))
                    .child(block(28.0, 8.0, 0.1))
                    .child(block(28.0, 8.0, 0.1)),
            )
            .child(block(66.0, 58.0, 0.12)),
    };
    let says = match key {
        "grid" => tip::GRID,
        "tabs" => tip::TABS,
        _ => tip::LIST,
    };
    let act = seg_act("layoutMode", format!("\"{key}\""));
    tile(s, "layout", act, pic, label, chosen == key, false, says)
}

/// Where a theme comes from, as the (i) popover's footer says it.
fn provenance(t: &Theme) -> String {
    if t.publisher.is_empty() {
        format!("Vorn \u{b7} v{} \u{b7} bundled", env!("CARGO_PKG_VERSION"))
    } else if t.version.is_empty() {
        format!("{} \u{b7} installed", t.publisher)
    } else {
        format!("{} \u{b7} v{} \u{b7} installed", t.publisher, t.version)
    }
}

/// The (i) popover: a theme's 16 terminal colours and where it comes from.
pub(crate) fn theme_info(s: &Screen<'_>, id: &str) -> El {
    let installed = s.st.settings.appearance.installed();
    let Some(t) = crate::appearance::find(installed, id) else {
        return div();
    };
    let mut grid = div().col().gap(5.0);
    for (r, row) in t.terminal.ansi.chunks(8).enumerate() {
        let mut line = div().row().gap(5.0);
        for (i, &rgb) in row.iter().enumerate() {
            line = line.child(
                div()
                    .grow()
                    .min_w0()
                    .h(20.0)
                    .rounded(3.0)
                    .bg(Rgba::from(rgb))
                    .border(1.0, white(0.08))
                    .role(Role::Image, format!("Colour {}", r * 8 + i)),
            );
        }
        grid = grid.child(line);
    }
    let title = format!("{} \u{b7} terminal palette", t.name);
    crate::ui::widgets::popover("Theme details")
        .w(268.0)
        .p(12.0)
        .gap(10.0)
        .child(
            text(title, 12.0, c(look::GRAY_100))
                .weight(600)
                .line_height(16.0),
        )
        .child(grid)
        .child(text(provenance(t), 11.0, c(look::GRAY_500)).line_height(14.0))
}

/// The theme chip's popover: search, filter chips, the tiles, and more
/// themes from the catalog.
pub(crate) fn theme_picker(s: &Screen<'_>, side: Look) -> El {
    use crate::appearance::{self as a, ThemeFilter};
    let installed = s.st.settings.appearance.installed();
    let total = vorn_theme::bundled().len() + installed.len();
    let filter = s.st.settings.theme_filter;
    let chosen = chosen_theme(s, side).id.as_str();
    let focus = if s.ui.focused(THEME_SEARCH) { 1.0 } else { 0.0 };
    let placeholder = format!("Search {total} themes");
    let style = TextStyle {
        line_height: 16.0,
        ..TextStyle::ui(12.0)
    };
    let search = div()
        .row()
        .items_center()
        .gap(8.0)
        .pl(10.0)
        .pr(8.0)
        .py(6.0)
        .rounded(4.0)
        .bg(white(0.05))
        .border(1.0, white(0.1).mix(white(0.2), focus))
        .frames(THEME_SEARCH)
        .child(glyph_icon(glyph::SEARCH, 12.0, c(look::GRAY_500)))
        .child(
            field(
                THEME_SEARCH,
                &placeholder,
                &placeholder,
                style,
                look::GRAY_200,
            )
            .grow()
            .min_w0(),
        );
    let mut chips = div().row().gap(6.0).role(Role::RadioGroup, "Filter");
    for f in ThemeFilter::ALL {
        let on = filter == Some(f);
        let id = s.acts.on("theme-filter", set(SetAct::ThemeFilter(f)));
        let hot = if on { 0.0 } else { s.ui.hover(id) };
        let ink = if on {
            c(look::WHITE)
        } else {
            c(look::GRAY_400)
        };
        chips = chips.child(
            div()
                .id(id)
                .on(Action::Click)
                .h(24.0)
                .px(10.0)
                .center()
                .rounded(4.0)
                .bg(white(if on { 0.12 } else { 0.04 }).mix(white(0.08), hot))
                .border(1.0, white(if on { 0.16 } else { 0.06 }))
                .role(Role::RadioButton, f.label())
                .selected(on)
                .child(text(f.label(), 12.0, ink).line_height(16.0).silent()),
        );
    }
    let list = a::picker_themes(installed, filter, s.ui.edit_text(THEME_SEARCH));
    let mut grid = div().row().wrap_children().gap(10.0);
    if list.is_empty() {
        grid = grid.child(
            text("No themes match", 12.0, c(look::GRAY_500))
                .line_height(16.0)
                .py(24.0),
        );
    }
    for t in list {
        grid = grid.child(picker_tile(s, t, t.id == chosen));
    }
    let more_id = s
        .acts
        .on("more-themes", set(SetAct::Browse("theme".into(), None)));
    let more_ink = c(look::BRONZO).mix(c(look::WHITE), s.ui.hover(more_id) * 0.3);
    let more = div()
        .id(more_id)
        .on(Action::Click)
        .role(Role::Link, "Get more themes")
        .ring(2.0)
        .child(
            text("Get more themes\u{2026}", 12.0, more_ink)
                .line_height(16.0)
                .silent(),
        );
    let footer = div()
        .row()
        .items_center()
        .justify_between()
        .px(12.0)
        .py(8.0)
        .border_top(1.0, white(0.06))
        .child(tooltip(s.ui, more, more_id, tip::MORE_THEMES, None))
        .child(
            text("Hover to preview", 11.0, c(look::GRAY_500))
                .line_height(14.0)
                .silent(),
        );
    crate::ui::widgets::popover("Themes")
        .w(430.0)
        .child(
            div()
                .col()
                .gap(10.0)
                .p(12.0)
                .border_bottom(1.0, white(0.06))
                .child(search)
                .child(chips),
        )
        .child(
            div()
                .id(Id::new("theme-grid"))
                .max_h(250.0)
                .scroll_y()
                .p(12.0)
                .child(grid),
        )
        .child(footer)
}

/// One theme in the picker: its window colour with its accent and a
/// terminal colour, its name under it.
fn picker_tile(s: &Screen<'_>, t: &Theme, on: bool) -> El {
    use vorn_theme::Token as T;
    let id = theme_tile_id(&t.id);
    s.acts.bind(id, set(SetAct::PickTheme(t.id.clone())));
    let hot = s.ui.hover(id);
    let dot = |color: Rgba| div().size(8.0, 8.0).rounded(4.0).bg(color);
    let edge = if on {
        c(look::BRONZO)
    } else {
        white(0.08).mix(white(0.24), hot)
    };
    let ink = if on {
        c(look::WHITE)
    } else {
        c(look::GRAY_400)
    };
    div()
        .id(id)
        .on(Action::Click)
        .col()
        .gap(4.0)
        .w(92.0)
        .role(Role::RadioButton, &t.name)
        .selected(on)
        .ring(4.0)
        .child(
            div()
                .row()
                .items_end()
                .justify_end()
                .gap(4.0)
                .size(92.0, 44.0)
                .p(6.0)
                .rounded(4.0)
                .bg(Rgba::from(t.get(T::SurfaceBase)))
                .border(if on { 2.0 } else { 1.0 }, edge)
                .child(dot(Rgba::from(t.get(T::Accent))))
                .child(dot(Rgba::from(t.terminal.ansi[2]))),
        )
        .child(
            text(&t.name, 11.0, ink)
                .line_height(14.0)
                .truncate()
                .silent(),
        )
}

/// The shortest the font list gets before it opens above the row instead.
const FONT_PICKER_MIN_H: f32 = 160.0;

/// The font chip's popover: each installed monospace family in its own
/// face, with a sample line and whether it joins ligatures.
pub(crate) fn font_picker(s: &Screen<'_>) -> El {
    const SAMPLE: &str = "fn main() -> Result<()> {} != 0";
    let current = s.ui.text.term_family();
    // Fit the room below the Font row so the list never flips over the chip.
    let below =
        s.st.menu
            .as_ref()
            .map_or(f32::MAX, |m| s.size.1 - m.at.1 - 8.0);
    let mut col = div()
        .id(Id::new("font-list"))
        .col()
        .max_h(crate::screen::menus::FONT_PICKER_H.min(below.max(FONT_PICKER_MIN_H)) - 10.0)
        .scroll_y()
        .py(4.0)
        .role(Role::ListBox, "Fonts");
    let fonts = s.st.settings.fonts.as_deref().unwrap_or(&[]);
    if fonts.is_empty() {
        col = col.child(
            text("No monospace fonts found", 12.0, c(look::GRAY_500))
                .line_height(16.0)
                .px(12.0)
                .py(8.0),
        );
    }
    for f in fonts {
        let v = Value::from(f.name.as_str());
        let id = s
            .acts
            .on("font-option", seg_act(crate::appearance::FONT_FAMILY, v));
        let on = f.name == current;
        let hot = if on { 0.0 } else { s.ui.hover(id) };
        let mut top = div().row().items_center().justify_between().child(
            text(&f.name, 11.0, c(look::GRAY_400))
                .line_height(14.0)
                .silent(),
        );
        if f.ligatures {
            let tag_id = Id::new("ligatures").child(f.name.as_str());
            let tag = div()
                .px(6.0)
                .rounded(3.0)
                .border(1.0, white(0.12))
                .role(Role::Label, "Ligatures")
                .child(
                    text("Ligatures", 10.0, c(look::GRAY_400))
                        .line_height(14.0)
                        .silent(),
                );
            top = top.child(tooltip(s.ui, tag, tag_id, tip::LIGATURES, None));
        }
        col = col.child(
            div()
                .id(id)
                .on(Action::Click)
                .col()
                .gap(4.0)
                .mx(4.0)
                .px(10.0)
                .py(8.0)
                .rounded(4.0)
                .bg(white(if on { 0.08 } else { 0.0 }).mix(white(0.05), hot))
                .role(Role::ListBoxOption, &f.name)
                .selected(on)
                .child(top)
                .child(
                    text(SAMPLE, 14.0, c(look::GRAY_100))
                        .family(&f.name)
                        .line_height(20.0)
                        .truncate()
                        .silent(),
                ),
        );
    }
    crate::ui::widgets::popover("Terminal font")
        .w(330.0)
        .child(col)
}

/// The terminal UI's Drawing choice: pixels look like the desktop app,
/// where the terminal shows images; text cells work everywhere.
fn drawing_row(s: &Screen<'_>, d: crate::screen::TermDrawing) -> El {
    let chosen = defaults(s)
        .and_then(|v| v.get("tuiDrawing"))
        .and_then(Value::as_str)
        .unwrap_or("auto");
    let seg = |label, v: &str, disabled| Seg {
        label,
        icon: None,
        act: seg_act("tuiDrawing", format!("\"{v}\"")),
        on: chosen == v,
        disabled,
    };
    let choice = segmented(
        s,
        "tui-drawing",
        vec![
            seg("Automatic", "auto", false),
            seg("Pixels", "pixels", !d.supported),
            seg("Text", "text", false),
        ],
        12.0,
    );
    let now = match (d.supported, d.pixels) {
        (false, _) => "This terminal does not show images.",
        (true, true) => "Drawing pixels now.",
        (true, false) => "Drawing text now.",
    };
    let note = format!("{} {now}", tip::DRAWING);
    card_row(s, None, "Drawing", Some(&note), choice)
}

/// The family's sigil in a 24 px frame.
fn shell_glyph(family: Option<&str>) -> El {
    div()
        .size(24.0, 24.0)
        .shrink0()
        .center()
        .rounded(4.0)
        .border(1.0, white(0.12))
        .child(
            text(sigil(family), 9.0, c(look::GRAY_400))
                .mono()
                .line_height(9.0)
                .silent(),
        )
}

/// The shell picker's typed path.
pub const SHELL_PATH: Id = Id(0x7368_656c_6c70_6174);

fn shell_picker(s: &Screen<'_>) -> El {
    let value = defaults(s)
        .and_then(|d| d.get("shell"))
        .and_then(Value::as_str)
        .unwrap_or("");
    if s.st.settings.shell_editing {
        let focus = s.ui.focused(SHELL_PATH);
        return div()
            .row()
            .items_center()
            .w(240.0)
            .shrink0()
            .px(8.0)
            .h(28.0)
            .rounded(4.0)
            .bg(white(0.04))
            .border(1.0, if focus { white(0.25) } else { white(0.12) })
            .frames(SHELL_PATH)
            .child(
                crate::ui::field(
                    SHELL_PATH,
                    "Path to a shell",
                    "Path to a shell",
                    TextStyle {
                        line_height: 18.0,
                        ..TextStyle::mono(12.0)
                    },
                    c(look::GRAY_200),
                )
                .grow()
                .min_w0(),
            );
    }
    let current =
        s.st.settings
            .shells
            .iter()
            .flatten()
            .find(|sh| sh.path == value);
    let name = current.map_or_else(
        || {
            value
                .rsplit(['/', '\\'])
                .next()
                .filter(|n| !n.is_empty())
                .unwrap_or("Select a shell")
                .to_owned()
        },
        |sh| sh.name.clone(),
    );
    let open =
        s.st.menu
            .as_ref()
            .is_some_and(|m| m.kind == MenuKind::SetShell);
    let id = s
        .acts
        .on("shell-picker", crate::act::Act::Open(MenuKind::SetShell));
    let chip = chip(s, id, "Shell", open)
        .child(glyph_icon(glyph::TERMINAL, 14.0, c(look::GRAY_500)))
        .child(
            text(name, 13.0, c(look::GRAY_200))
                .line_height(18.0)
                .silent(),
        )
        .child(chevron(open))
        .role(Role::ComboBox, "Shell")
        .expanded(open);
    tooltip(s.ui, chip, id, tip::SHELL, None)
}

/// The shell picker's listbox.
pub(crate) fn shell_panel(s: &Screen<'_>) -> El {
    let value = defaults(s)
        .and_then(|d| d.get("shell"))
        .and_then(Value::as_str)
        .unwrap_or("");
    let mut col = div()
        .col()
        .min_w(288.0)
        .py(4.0)
        .rounded(4.0)
        .border(1.0, white(0.12))
        .bg(c(look::SURFACE_OVERLAY))
        .role(Role::ListBox, "Shells");
    let note = |t: &str| {
        div()
            .px(12.0)
            .py(8.0)
            .child(text(t, 12.0, c(look::GRAY_500)).line_height(16.0).silent())
    };
    match &s.st.settings.shells {
        None => col = col.child(note("Looking for shells\u{2026}")),
        Some(list) if list.is_empty() => {
            col = col.child(note("No shells detected. Choose a path instead."));
        }
        Some(list) => {
            for sh in list {
                let id = s
                    .acts
                    .on("shell-option", set(SetAct::PickShell(sh.path.clone())));
                let hot = s.ui.hover(id);
                let block = if sh.level == "full" {
                    text("Full command blocks", 11.0, c(look::GRAY_500))
                } else {
                    let tone = if sh.level == "limited" {
                        look::AMBER_500.alpha(0.8)
                    } else {
                        c(look::GRAY_500)
                    };
                    text(sh.limitation.clone().unwrap_or_default(), 11.0, tone)
                };
                let mut title = div().row().items_end().gap(6.0).child(
                    text(&sh.name, 13.0, c(look::GRAY_200))
                        .line_height(19.5)
                        .silent(),
                );
                if let Some(v) = &sh.version {
                    title = title.child(
                        text(v, 11.0, c(look::GRAY_600))
                            .mono()
                            .line_height(18.0)
                            .silent(),
                    );
                }
                let mut r = div()
                    .id(id)
                    .on(Action::Click)
                    .row()
                    .gap(8.0)
                    .px(12.0)
                    .py(8.0)
                    .bg(white(0.0).mix(white(0.06), hot))
                    .role(Role::ListBoxOption, sh.name.as_str())
                    .selected(sh.path == value)
                    .child(div().mt(1.0).child(shell_glyph(Some(&sh.family))))
                    .child(
                        div()
                            .col()
                            .grow()
                            .min_w0()
                            .child(title)
                            .child(
                                text(&sh.path, 11.0, c(look::GRAY_600))
                                    .mono()
                                    .line_height(16.5)
                                    .truncate()
                                    .silent(),
                            )
                            .child(div().mt(2.0).child(block.line_height(16.5).silent())),
                    );
                if sh.path == value {
                    r = r.child(
                        div()
                            .mt(4.0)
                            .child(ico("check", 13.0, 2.0, c(look::GRAY_400))),
                    );
                }
                col = col.child(r);
            }
        }
    }
    let edit = s.acts.on("shell-option", set(SetAct::EditShell));
    let choose = div()
        .id(edit)
        .on(Action::Click)
        .row()
        .items_center()
        .gap(8.0)
        .px(12.0)
        .py(8.0)
        .bg(white(0.0).mix(white(0.06), s.ui.hover(edit)))
        .role(Role::Button, "Choose path\u{2026}")
        .child(ico("pen-line", 13.0, 2.0, c(look::GRAY_500)))
        .child(
            text("Choose path\u{2026}", 12.0, c(look::GRAY_400))
                .line_height(16.0)
                .silent(),
        );
    col.child(div().mt(4.0).mb(4.0).h(1.0).w_full().bg(white(0.06)))
        .child(tooltip(s.ui, choose, edit, tip::CHOOSE_PATH, None))
}

/// The `vorn` command: Install, Reinstall while its folder is not on the
/// PATH, or Remove, which also stops the app putting it back.
fn cli_card(s: &Screen<'_>) -> Option<El> {
    let st =
        s.st.settings
            .cli
            .as_ref()
            .filter(|c| c.available && !c.managed)?;
    let note =
        s.st.settings
            .cli_error
            .clone()
            .or_else(|| st.note.clone())
            .unwrap_or_else(|| match (st.installed, st.on_path) {
                (true, true) => format!("Installed at {}. Run vorn --help in a terminal.", st.path),
                (true, false) => format!("{} is not on your PATH.", st.path),
                (false, _) => {
                    "The vorn command starts sessions and reads them from a terminal.".to_owned()
                }
            });
    let (label, act) = match (st.installed, st.on_path) {
        (true, true) => ("Remove", SetAct::RemoveCli),
        (true, false) => ("Reinstall", SetAct::InstallCli),
        (false, _) => ("Install", SetAct::InstallCli),
    };
    let control = button(s, "cli", label, act, !s.st.settings.cli_busy);
    Some(div().mt(24.0).child(card(vec![card_row(
        s,
        None,
        "Command-line tool",
        Some(&note),
        control,
    )])))
}

pub(super) fn general(s: &Screen<'_>) -> El {
    let reopen = flag(s, "reopenSessions", true);
    let login = (!settings::IN_BROWSER).then(|| {
        let on = flag(s, "startAtLogin", false);
        let t = toggle(
            s,
            "startAtLogin",
            on,
            set(default_act("startAtLogin", !on)),
            false,
            "Open at login",
        );
        card_row(s, None, "Open at login", Some(tip::LOGIN), t)
    });
    let t = toggle(
        s,
        "reopenSessions",
        reopen,
        set(default_act("reopenSessions", !reopen)),
        false,
        "Reopen sessions",
    );
    let startup: Vec<El> = login
        .into_iter()
        .chain([card_row(s, None, "Reopen sessions", Some(tip::REOPEN), t)])
        .collect();
    div()
        .col()
        .child(header("General", "", None))
        .child(section("Startup"))
        .child(card(startup))
        .child(section("New sessions"))
        .child(card(vec![
            card_row(s, None, "Agent", Some(tip::AGENT), agent_chip(s)),
            card_row(s, None, "Shell", Some(tip::SHELL), shell_picker(s)),
        ]))
}

/// The agent new sessions start with: the stored one, else the first.
fn default_agent<'a>(s: &Screen<'a>) -> &'a str {
    defaults(s)
        .and_then(|d| d.get("defaultAgent"))
        .and_then(Value::as_str)
        .unwrap_or(s.store.agents.default_key())
}

/// The Agent row's chip: the default agent's mark and full name.
fn agent_chip(s: &Screen<'_>) -> El {
    let key = default_agent(s);
    let name = s.store.agents.get(key).map_or(key, |a| a.name.as_str());
    let open =
        s.st.menu
            .as_ref()
            .is_some_and(|m| m.kind == MenuKind::AgentPicker);
    let id = s.acts.on("agent-chip", Act::Open(MenuKind::AgentPicker));
    let chip = chip(s, id, name, open)
        .pl(6.0)
        .child(agent_icon(s, key, 18.0))
        .child(
            text(name, 13.0, c(look::GRAY_200))
                .line_height(18.0)
                .silent(),
        )
        .child(chevron(open));
    tooltip(s.ui, chip, id, tip::AGENT, None)
}

/// The field the agent picker searches in.
pub const AGENT_SEARCH: Id = Id(0x6167_656e_7473_7263);

/// The agent picker: search, the agents on this machine, the ones that can
/// be added, and more from the catalog.
pub(crate) fn agent_picker(s: &Screen<'_>) -> El {
    use crate::plugins::{Kind, Source};
    let query = s.ui.edit_text(AGENT_SEARCH).trim().to_lowercase();
    let hit = |name: &str| query.is_empty() || name.to_lowercase().contains(&query);
    let chosen = default_agent(s);
    let on_machine = |key: &str| {
        s.st.installed
            .as_ref()
            .is_none_or(|l| l.iter().any(|k| k == key))
    };
    let heading = |label: &str, tip_text: &str| {
        let id = Id::new("agent-section").child(label);
        let t = text(label.to_uppercase(), 11.0, c(look::GRAY_500))
            .tracking(0.06)
            .line_height(14.0)
            .id(id)
            .sense(Sense::HOVER)
            .role(Role::Heading, label);
        div()
            .px(10.0)
            .pt(8.0)
            .pb(4.0)
            .child(tooltip(s.ui, t, id, tip_text, None))
    };
    let mut installed = Vec::new();
    let mut available = Vec::new();
    for a in &s.store.agents {
        if !hit(&a.name) {
            continue;
        }
        if on_machine(&a.key) {
            let id = s.acts.on(
                "agent-option",
                set(default_act("defaultAgent", format!("\"{}\"", a.key))),
            );
            let on = a.key == chosen;
            let lit = s.ui.hover(id).max(if on { 1.0 } else { 0.0 });
            let mut r = div()
                .id(id)
                .on(Action::Click)
                .row()
                .items_center()
                .gap(10.0)
                .px(10.0)
                .py(6.0)
                .rounded(4.0)
                .bg(white(0.0).mix(white(0.08), lit))
                .role(Role::ListBoxOption, a.name.as_str())
                .selected(on)
                .child(agent_icon(s, &a.key, 20.0))
                .child(
                    div().grow().min_w0().child(
                        text(&a.name, 13.0, c(look::GRAY_200))
                            .line_height(18.0)
                            .truncate()
                            .silent(),
                    ),
                );
            if on {
                r = r.child(
                    text("Default", 11.0, c(look::GRAY_500))
                        .line_height(14.0)
                        .silent(),
                );
            }
            installed.push(r);
        } else {
            available.push(available_row(
                s,
                &a.key,
                Some(&a.key),
                &a.name,
                "Set up",
                tip::SET_UP,
                set(SetAct::Open(settings::Category::Agents)),
            ));
        }
    }
    for l in &s.st.plugins.listings {
        let agent = l.kind == Kind::Extension
            && l.source == Source::Catalog
            && l.id
                .strip_prefix(AGENT_EXTENSIONS)
                .is_some_and(|k| s.store.agents.iter().all(|a| a.key != k));
        if agent && hit(&l.name) {
            let act = set(SetAct::Browse("agent".into(), Some(l.key.clone())));
            available.push(available_row(
                s,
                &l.key,
                None,
                &l.name,
                "Install",
                tip::INSTALL,
                act,
            ));
        }
    }
    let mut list = div().col().p(6.0).role(Role::ListBox, "Agents");
    if installed.is_empty() && available.is_empty() {
        list = list.child(
            text("No agents match", 12.0, c(look::GRAY_500))
                .line_height(16.0)
                .px(10.0)
                .py(16.0),
        );
    }
    if !installed.is_empty() {
        list = list
            .child(heading("Installed", tip::INSTALLED))
            .children(installed);
    }
    if !available.is_empty() {
        list = list
            .child(heading("Available", tip::AVAILABLE))
            .children(available);
    }
    let focus = if s.ui.focused(AGENT_SEARCH) { 1.0 } else { 0.0 };
    let style = TextStyle {
        line_height: 16.0,
        ..TextStyle::ui(12.0)
    };
    let search = div()
        .row()
        .items_center()
        .gap(8.0)
        .pl(10.0)
        .pr(8.0)
        .py(6.0)
        .rounded(4.0)
        .bg(white(0.05))
        .border(1.0, white(0.1).mix(white(0.2), focus))
        .frames(AGENT_SEARCH)
        .child(glyph_icon(glyph::SEARCH, 12.0, c(look::GRAY_500)))
        .child(
            field(
                AGENT_SEARCH,
                "Search agents",
                "Search agents",
                style,
                look::GRAY_200,
            )
            .grow()
            .min_w0(),
        );
    let more_id = s
        .acts
        .on("more-agents", set(SetAct::Browse("agent".into(), None)));
    let more_ink = c(look::BRONZO).mix(c(look::WHITE), s.ui.hover(more_id) * 0.3);
    let more = div()
        .id(more_id)
        .on(Action::Click)
        .role(Role::Link, "Get more agents")
        .ring(2.0)
        .child(
            text("Get more agents\u{2026}", 12.0, more_ink)
                .line_height(16.0)
                .silent(),
        );
    crate::ui::widgets::popover("Agents")
        .w(320.0)
        .child(
            div()
                .col()
                .p(10.0)
                .border_bottom(1.0, white(0.06))
                .child(search),
        )
        .child(
            div()
                .id(Id::new("agent-list"))
                .col()
                .max_h(320.0)
                .scroll_y()
                .child(list),
        )
        .child(
            div()
                .px(12.0)
                .py(8.0)
                .border_top(1.0, white(0.06))
                .child(tooltip(s.ui, more, more_id, tip::MORE_AGENTS, None)),
        )
}

/// The catalog's agent adapters are extensions under this id.
const AGENT_EXTENSIONS: &str = "vorn.agents.";

/// An agent that can be added: its mark (a known one's, else a blank),
/// faded name, and the button that adds it.
fn available_row(
    s: &Screen<'_>,
    place: &str,
    agent: Option<&str>,
    name: &str,
    label: &str,
    tip_text: &str,
    act: Act,
) -> El {
    let id = s.acts.on(&format!("agent-add:{place}"), act);
    let mark = agent.map_or_else(
        || div().size(20.0, 20.0).rounded(4.0).bg(white(0.08)),
        |k| agent_icon(s, k, 20.0),
    );
    let b = div()
        .id(id)
        .on(Action::Click)
        .px(10.0)
        .py(3.0)
        .rounded(4.0)
        .bg(white(0.06).mix(white(0.12), s.ui.hover(id)))
        .role(Role::Button, format!("{label} {name}"))
        .ring(4.0)
        .child(
            text(label, 11.0, c(look::GRAY_200))
                .line_height(14.0)
                .silent(),
        );
    div()
        .row()
        .items_center()
        .gap(10.0)
        .px(10.0)
        .py(6.0)
        .child(div().opacity(0.6).child(mark))
        .child(
            div().grow().min_w0().child(
                text(name, 13.0, c(look::GRAY_400))
                    .line_height(18.0)
                    .truncate()
                    .silent(),
            ),
        )
        .child(tooltip(s.ui, b, id, tip_text, None))
}

fn notif(s: &Screen<'_>) -> (bool, bool, bool, bool, bool, f64) {
    let n = defaults(s).and_then(|d| d.get("notifications"));
    let b = |k: &str, d: bool| {
        n.and_then(|n| n.get(k))
            .and_then(Value::as_bool)
            .unwrap_or(d)
    };
    (
        b("enabled", false),
        b("onWaiting", true),
        b("onError", true),
        b("onBell", true),
        b("soundEnabled", false),
        n.and_then(|n| n.get("soundVolume"))
            .and_then(Value::as_f64)
            .unwrap_or(0.5),
    )
}

pub(super) fn notifications(s: &Screen<'_>) -> El {
    let (enabled, waiting, error, bell, sound, volume) = notif(s);
    let sub = |key: &str, on: bool, title: &str, description: &str| {
        let t = toggle(
            s,
            key,
            on,
            SetAct::Notify(key.to_owned(), !on),
            !enabled,
            title,
        );
        row(title, description, None, t, !enabled)
    };
    let main = toggle(
        s,
        "enabled",
        enabled,
        SetAct::Notify("enabled".into(), !enabled),
        false,
        "Enable Notifications",
    );
    let mut sounds = div().col().gap(12.0).child(sub(
        "soundEnabled",
        sound,
        "Sound Effects",
        "Play a tone on waiting, error, and bell events (works even when focused)",
    ));
    if sound && enabled {
        let pct = (volume * 100.0).round();
        let test = s.acts.on("sound-test", set(SetAct::TestSound));
        let hot = s.ui.hover(test);
        sounds = sounds.child(
            div()
                .row()
                .items_center()
                .gap(12.0)
                .px(16.0)
                .py(8.0)
                .child(
                    div().w(48.0).shrink0().child(
                        text("Volume", 12.0, c(look::GRAY_400))
                            .line_height(16.0)
                            .silent(),
                    ),
                )
                .child(
                    div()
                        .id(VOLUME_SLIDER)
                        .sense(Sense::MOVE)
                        .grow()
                        .h(12.0)
                        .role(Role::Slider, "Volume")
                        .child(
                            div()
                                .absolute(0.0, 4.0)
                                .w_full()
                                .h(4.0)
                                .rounded(2.0)
                                .bg(white(0.08)),
                        )
                        .child(volume_knob(volume as f32)),
                )
                .child(
                    div().w(32.0).row().justify_end().child(
                        text(format!("{pct}%"), 12.0, c(look::GRAY_500))
                            .line_height(16.0)
                            .silent(),
                    ),
                )
                .child(
                    div()
                        .id(test)
                        .on(Action::Click)
                        .px(8.0)
                        .py(4.0)
                        .rounded(4.0)
                        .bg(white(0.06).mix(white(0.1), hot))
                        .role(Role::Button, "Test")
                        .child(
                            text("Test", 11.0, c(look::GRAY_400).mix(c(look::WHITE), hot))
                                .line_height(16.5)
                                .silent(),
                        ),
                ),
        );
    }
    div()
        .col()
        .child(header(
            "Notifications",
            "Get notified when agents need your attention",
            None,
        ))
        .child(
            div()
                .col()
                .gap(4.0)
                .child(row(
                    "Enable Notifications",
                    "Show OS notifications when agents need attention",
                    None,
                    main,
                    false,
                ))
                .child(sub(
                    "onWaiting",
                    waiting,
                    "Waiting for Input",
                    "Notify when an agent is waiting for your response",
                ))
                .child(sub(
                    "onError",
                    error,
                    "Errors",
                    "Notify when an agent encounters an error",
                ))
                .child(sub(
                    "onBell",
                    bell,
                    "Terminal Bell",
                    "Notify when an agent explicitly sends a notification signal",
                )),
        )
        .child(
            text("Sound", 14.0, c(look::GRAY_200))
                .weight(500)
                .line_height(20.0)
                .mt(32.0)
                .mb(8.0)
                .role(Role::Heading, "Sound"),
        )
        .child(
            text(
                "Play audio tones when agents change status",
                14.0,
                c(look::GRAY_500),
            )
            .line_height(20.0)
            .mb(24.0)
            .silent(),
        )
        .child(sounds)
}

/// The volume knob, placed by its share of the track (laid out by flex).
fn volume_knob(v: f32) -> El {
    let v = v.clamp(0.0, 1.0);
    div()
        .absolute(0.0, 0.0)
        .w_full()
        .h(12.0)
        .row()
        .child(div().w_pct(v * 0.999).h(1.0))
        .child(
            div()
                .size(12.0, 12.0)
                .ml(-6.0)
                .rounded(6.0)
                .bg(c(look::WHITE)),
        )
}

pub(super) fn mcp(s: &Screen<'_>) -> El {
    let mut list = div().col().gap(12.0);
    for a in &s.store.agents {
        let Some(setup) = &a.mcp else {
            continue;
        };
        let mut cmds = div().col().gap(8.0);
        for cmd in &setup.commands {
            let id = s.acts.on("mcp-copy", set(SetAct::Copy(cmd.clone())));
            let hot = s.ui.hover(id);
            let copied = s.st.settings.copied.as_ref().is_some_and(|(k, _)| k == cmd);
            cmds = cmds.child(
                div()
                    .row()
                    .items_center()
                    .gap(8.0)
                    .child(
                        div()
                            .grow()
                            .min_w0()
                            .px(12.0)
                            .py(6.0)
                            .rounded(6.0)
                            .bg(white(0.04))
                            .border(1.0, white(0.08))
                            .clip()
                            .child(
                                text(cmd.as_str(), 12.0, c(look::GRAY_300))
                                    .mono()
                                    .line_height(16.0)
                                    .silent(),
                            ),
                    )
                    .child(
                        div()
                            .id(id)
                            .on(Action::Click)
                            .px(8.0)
                            .py(4.0)
                            .rounded(4.0)
                            .shrink0()
                            .bg(white(0.04).mix(white(0.08), hot))
                            .role(Role::Button, "Copy")
                            .child(
                                text(
                                    if copied { "Copied" } else { "Copy" },
                                    12.0,
                                    c(look::GRAY_400).mix(c(look::GRAY_200), hot),
                                )
                                .line_height(16.0)
                                .silent(),
                            ),
                    ),
            );
        }
        let mut card = div()
            .col()
            .p(16.0)
            .rounded(8.0)
            .border(1.0, white(0.06))
            .bg(c(look::SURFACE_SUNKEN))
            .child(
                div()
                    .row()
                    .items_center()
                    .gap(12.0)
                    .mb(12.0)
                    .child(agent_icon(s, &a.key, 20.0))
                    .child(
                        text(a.name.as_str(), 14.0, c(look::GRAY_200))
                            .weight(500)
                            .line_height(20.0)
                            .silent(),
                    )
                    .child(
                        text(
                            if setup.in_agent {
                                format!("Run in {}", a.name)
                            } else {
                                "Run in your terminal".to_owned()
                            },
                            11.0,
                            c(look::GRAY_500),
                        )
                        .line_height(16.5)
                        .silent(),
                    ),
            )
            .child(cmds);
        if let Some(n) = &setup.note {
            card = card.child(
                text(n.as_str(), 11.0, c(look::GRAY_500))
                    .line_height(16.5)
                    .mt(8.0)
                    .wrapping()
                    .silent(),
            );
        }
        list = list.child(card);
    }
    div()
        .col()
        .child(
            text("Skills & MCP", 20.0, c(look::WHITE))
                .weight(600)
                .line_height(28.0)
                .mb(4.0)
                .role(Role::Heading, "Skills & MCP"),
        )
        .child(
            text(
                "Connect your coding agents to Vorn. Each setup below installs the MCP server along with the guidance that explains what its tools are for.",
                14.0,
                c(look::GRAY_500),
            )
            .line_height(20.0)
            .mb(24.0)
            .wrapping()
            .silent(),
        )
        .child(
            text("Set up an agent", 14.0, c(look::GRAY_200))
                .weight(500)
                .line_height(20.0)
                .mb(12.0)
                .silent(),
        )
        .child(list)
}

/// The rotation field of a key row.
pub fn rotate_id(connection: &str) -> Id {
    Id::new("key-rotate").child(connection)
}

fn key_button(
    s: &Screen<'_>,
    place: &str,
    label: &str,
    act: SetAct,
    icon_name: Option<&str>,
    enabled: bool,
) -> El {
    let id = s.acts.on(place, set(act));
    let hot = s.ui.hover(id);
    let fg = c(look::GRAY_400).mix(c(look::GRAY_200), hot);
    let mut b = div()
        .id(id)
        .row()
        .items_center()
        .gap(4.0)
        .px(8.0)
        .py(4.0)
        .rounded(2.0)
        .shrink0()
        .border(1.0, white(0.1))
        .role(Role::Button, label);
    if let Some(n) = icon_name {
        b = b.child(ico(n, 11.0, 2.0, fg));
    }
    b = b.child(text(label, 11.0, fg).line_height(16.5).silent());
    if enabled {
        b.on(Action::Click)
    } else {
        b.opacity(0.5)
    }
}

pub(super) fn keys(s: &Screen<'_>) -> El {
    let mut list = div().col().gap(8.0);
    for k in s.st.settings.keys.iter().flatten() {
        let st =
            s.st.settings
                .key_rows
                .get(&k.connection)
                .cloned()
                .unwrap_or_default();
        let summary = format!(
            "{} \u{b7} {}",
            k.fields
                .iter()
                .map(describe_field)
                .collect::<Vec<_>>()
                .join(" \u{b7} "),
            describe_usage(k.usage)
        );
        let mut top = div()
            .row()
            .items_center()
            .gap(12.0)
            .child(connector_mark(&k.connector, 14.0, c(look::GRAY_400)))
            .child(
                div()
                    .col()
                    .grow()
                    .min_w0()
                    .child(
                        text(&k.name, 13.0, c(look::GRAY_200))
                            .line_height(19.5)
                            .truncate()
                            .silent(),
                    )
                    .child(
                        text(summary, 11.0, c(look::GRAY_500))
                            .line_height(16.5)
                            .truncate()
                            .silent(),
                    ),
            );
        if let Some((ok, _)) = &st.result {
            let dot = match ok {
                None => c(look::GRAY_600),
                Some(true) => c(look::DIFF_ADD),
                Some(false) => c(look::DANGER),
            };
            top = top.child(div().size(6.0, 6.0).rounded(3.0).shrink0().bg(dot));
        }
        if k.connector == "http" {
            top = top.child(key_button(
                s,
                "key-test",
                if st.testing {
                    "Testing\u{2026}"
                } else {
                    "Test"
                },
                SetAct::TestKey(k.connection.clone()),
                None,
                !st.testing,
            ));
        }
        for f in &k.fields {
            let label = if k.fields.len() > 1 {
                format!("Rotate {}", f.label)
            } else {
                "Rotate".to_owned()
            };
            top = top.child(key_button(
                s,
                "key-rotate",
                &label,
                SetAct::Rotate(k.connection.clone(), f.key.clone()),
                Some("refresh-cw"),
                true,
            ));
        }
        let mut card = div()
            .col()
            .px(16.0)
            .py(10.0)
            .rounded(2.0)
            .bg(white(0.03))
            .border(1.0, white(0.06))
            .child(top);
        if let Some((ok, message)) = &st.result {
            let line = match ok {
                None => "This connector has nothing to check".to_owned(),
                Some(o) => message.clone().unwrap_or_else(|| {
                    if *o {
                        "Reached it"
                    } else {
                        "Could not reach it"
                    }
                    .to_owned()
                }),
            };
            let color = if *ok == Some(false) {
                c(look::DANGER)
            } else {
                c(look::GRAY_500)
            };
            card = card.child(text(line, 11.0, color).line_height(16.5).mt(6.0).silent());
        }
        if let Some(field) = &st.rotating {
            let id = rotate_id(&k.connection);
            let env = k.fields.iter().any(|f| &f.key == field && f.env.is_some());
            let empty = s.ui.edit_text(id).trim().is_empty();
            card = card.child(
                div()
                    .row()
                    .items_center()
                    .gap(8.0)
                    .mt(8.0)
                    .child(
                        div()
                            .grow()
                            .min_w0()
                            .px(8.0)
                            .py(4.0)
                            .rounded(2.0)
                            .bg(white(0.05))
                            .border(
                                1.0,
                                if s.ui.focused(id) {
                                    white(0.2)
                                } else {
                                    white(0.1)
                                },
                            )
                            .frames(id)
                            .child(
                                crate::ui::field(
                                    id,
                                    "Replacement value",
                                    if env {
                                        "{\"TOKEN\": \"\u{2026}\"}"
                                    } else {
                                        "The replacement value"
                                    },
                                    TextStyle {
                                        line_height: 16.0,
                                        ..TextStyle::ui(12.0)
                                    },
                                    c(look::GRAY_200),
                                )
                                .secure()
                                .w_full(),
                            ),
                    )
                    .child(key_button(
                        s,
                        "key-save",
                        if st.saving { "Saving\u{2026}" } else { "Save" },
                        SetAct::SaveRotate(k.connection.clone(), field.clone()),
                        None,
                        !st.saving && !empty,
                    ))
                    .child({
                        let id = s.acts.on(
                            "key-cancel",
                            set(SetAct::CancelRotate(k.connection.clone())),
                        );
                        let hot = s.ui.hover(id);
                        div()
                            .id(id)
                            .on(Action::Click)
                            .px(8.0)
                            .py(4.0)
                            .role(Role::Button, "Cancel")
                            .child(
                                text(
                                    "Cancel",
                                    11.0,
                                    c(look::GRAY_500).mix(c(look::GRAY_300), hot),
                                )
                                .line_height(16.5)
                                .silent(),
                            )
                    }),
            );
        }
        if let Some(e) = &st.error {
            card = card.child(
                text(e, 11.0, c(look::DANGER))
                    .line_height(16.5)
                    .mt(6.0)
                    .silent(),
            );
        }
        list = list.child(card);
    }
    let mut page = div()
        .col()
        .child(
            text("Keys", 20.0, c(look::WHITE))
                .weight(600)
                .line_height(28.0)
                .mb(4.0)
                .role(Role::Heading, "Keys"),
        )
        .child(
            text(
                "The secrets your connections hold, stored encrypted with your OS keychain",
                14.0,
                c(look::GRAY_500),
            )
            .line_height(20.0)
            .mb(24.0)
            .silent(),
        )
        .child(list);
    if s.st.settings.keys.as_ref().is_some_and(Vec::is_empty) {
        page = page.child(
            div()
                .col()
                .items_center()
                .px(16.0)
                .py(24.0)
                .rounded(2.0)
                .border(1.0, white(0.06))
                .child(
                    div()
                        .mb(8.0)
                        .child(ico("key-round", 16.0, 2.0, c(look::GRAY_600))),
                )
                .child(
                    text("No keys yet", 14.0, c(look::GRAY_400))
                        .line_height(20.0)
                        .silent(),
                )
                .child(
                    text(
                        "A connection that signs in with a key or a token shows up here.",
                        11.0,
                        c(look::GRAY_600),
                    )
                    .line_height(16.5)
                    .mt(4.0)
                    .silent(),
                ),
        );
    }
    page
}

pub(super) fn about(s: &Screen<'_>) -> El {
    let st = &s.st.settings;
    let app = env!("CARGO_PKG_VERSION");
    let status = st.update.clone().unwrap_or(UpdateStatus::Unsupported);
    let live = s.store.sessions.iter().filter(|x| !x.ended()).count();
    let head = settings::about_head(&status, app, now_ms(), live, !crate::update::ENDS_SESSIONS);
    let ready = match &status {
        UpdateStatus::Ready { version } => Some(version.as_str()),
        _ => None,
    };
    let mut rows = vec![about_header(s, &head, &status)];
    if let Some(b) = settings::behind(s.st.server_version.as_deref(), app, st.holders.as_ref()) {
        rows.push(holder_row(s, &b));
    }
    let notes_for = ready.unwrap_or(app);
    let notes = s.acts.on(
        "release-notes",
        Act::OpenUrl(settings::release_notes(notes_for)),
    );
    let whats_new = card_row(
        s,
        None,
        &format!("What's new in {notes_for}"),
        None,
        glyph_icon(glyph::ARROW_UP_RIGHT, 14.0, c(look::GRAY_500)),
    )
    .on(Action::Click)
    .role(Role::Link, "What's new")
    .ring(4.0);
    let whats_new = crate::ui::tooltip(s.ui, whats_new, notes, tip::WHATS_NEW, None);
    rows.push(whats_new);
    rows.push(extensions_row(s));
    let mut links = div().row().wrap_children().gap(8.0).mt(16.0);
    for (label, url) in [
        ("Website", settings::WEBSITE),
        ("Documentation", settings::DOCUMENTATION),
        ("Send feedback", settings::FEEDBACK),
        ("GitHub", settings::GITHUB),
    ] {
        let id = s
            .acts
            .on(&format!("link-{label}"), Act::OpenUrl(url.to_owned()));
        links = links.child(link_pill(s, id, label));
    }
    let col = div()
        .col()
        .child(header("About", "", None))
        .child(card(rows))
        .child(links)
        .child(
            text("\u{a9} 2026 Javier Canizalez", 11.0, c(look::GRAY_600))
                .line_height(16.0)
                .mt(16.0)
                .pl(4.0)
                .silent(),
        );
    col.child_if(cli_card(s))
}

/// The Vorn mark, the version or the update that is ready, the channel and
/// the one action.
fn about_header(s: &Screen<'_>, head: &settings::AboutHead, status: &UpdateStatus) -> El {
    let mut mark = div().w(48.0).h(36.0).shrink0().center();
    if let Some(id) = s.logo {
        mark = mark.child(image(id, 22.0 * look::LOGO_ASPECT, 22.0));
    }
    let mut detail = div().row().items_center().gap(5.0).mt(2.0);
    if head.current {
        detail = detail.child(ico("circle-check", 12.0, 2.0, c(look::GREEN_400)));
    }
    detail = detail.child(
        text(&head.detail, 12.0, c(look::GRAY_500))
            .line_height(16.0)
            .truncate()
            .silent(),
    );
    let text_col = div()
        .col()
        .grow()
        .min_w0()
        .child(
            text(&head.title, 17.0, c(look::WHITE))
                .weight(600)
                .line_height(22.0)
                .role(Role::Status, head.title.as_str()),
        )
        .child(detail);
    let mut right = div().row().items_center().gap(8.0).shrink0();
    if !settings::IN_BROWSER {
        // A ready update is one button; the channel waits until it is installed.
        if head.action != Some(UpdateAction::Restart) {
            right = right.child(channel_chip(s));
        }
        let checking = matches!(status, UpdateStatus::Checking);
        let (label, act, tip, primary) = match head.action {
            Some(UpdateAction::Restart) => (
                "Restart to update",
                SetAct::UpdateNow,
                tip::RESTART_UPDATE,
                true,
            ),
            Some(UpdateAction::Retry) => ("Try again", SetAct::CheckUpdates, tip::CHECK, false),
            _ => (
                if checking {
                    "Checking\u{2026}"
                } else {
                    "Check now"
                },
                SetAct::CheckUpdates,
                tip::CHECK,
                false,
            ),
        };
        let enabled = primary || (!checking && *status != UpdateStatus::Unsupported);
        right = right.child(pill_button(
            s,
            "update-action",
            label,
            act,
            tip,
            primary,
            enabled,
        ));
    }
    div()
        .row()
        .items_center()
        .gap(12.0)
        .px(14.0)
        .py(14.0)
        .child(mark)
        .child(text_col)
        .child(right)
}

/// The Stable / Beta chip, opening the channel menu.
fn channel_chip(s: &Screen<'_>) -> El {
    let beta = update_channel(s) == "beta";
    let name = if beta { "Beta" } else { "Stable" };
    let kind = MenuKind::UpdateChannel;
    let open = s.st.menu.as_ref().is_some_and(|m| m.kind == kind);
    let id = s.acts.on("update-channel", Act::Open(kind));
    let lit = s.ui.hover(id).max(if open { 1.0 } else { 0.0 });
    let chip = div()
        .id(id)
        .on(Action::Click)
        .row()
        .items_center()
        .gap(4.0)
        .h(22.0)
        .pl(8.0)
        .pr(6.0)
        .rounded(4.0)
        .bg(white(0.06).mix(white(0.1), lit))
        .role(Role::Button, name)
        .ring(4.0)
        .child(
            text(name, 11.0, c(look::GRAY_300))
                .line_height(14.0)
                .silent(),
        )
        .child(glyph_icon(glyph::CHEVRON_DOWN, 10.0, c(look::GRAY_500)));
    tooltip(s.ui, chip, id, tip::CHANNEL, None)
}

fn update_channel<'a>(s: &Screen<'a>) -> &'a str {
    defaults(s)
        .and_then(|d| d.get("updateChannel"))
        .and_then(Value::as_str)
        .unwrap_or("stable")
}

/// The channel menu's two picks.
pub(crate) fn channel_items(s: &Screen<'_>) -> Vec<crate::screen::menus::Item> {
    let beta = update_channel(s) == "beta";
    vec![
        crate::screen::menus::Item::pick("Stable", !beta, seg_act("updateChannel", "\"stable\"")),
        crate::screen::menus::Item::pick("Beta", beta, seg_act("updateChannel", "\"beta\"")),
    ]
}

/// The terminals row's menu: end the older holder's terminals, confirmed in the row.
pub(crate) fn holder_items(instance: &str) -> Vec<crate::screen::menus::Item> {
    let mut end = crate::screen::menus::Item::new(
        "trash-2",
        "End them\u{2026}",
        set(SetAct::EndHolder(instance.to_owned())),
    );
    end.danger = true;
    vec![end]
}

/// A neutral pill, or the surface's one filled button when `primary`.
fn pill_button(
    s: &Screen<'_>,
    place: &str,
    label: &str,
    act: SetAct,
    tip: &str,
    primary: bool,
    enabled: bool,
) -> El {
    let id = s.acts.on(place, set(act));
    let hot = s.ui.hover(id);
    let (bg, fg) = if primary {
        (
            c(look::WHITE).mix(c(look::GRAY_300), hot),
            c(look::SURFACE_BASE),
        )
    } else {
        (white(0.06).mix(white(0.1), hot), c(look::GRAY_200))
    };
    let mut b = div()
        .id(id)
        .row()
        .items_center()
        .h(28.0)
        .px(12.0)
        .rounded(4.0)
        .shrink0()
        .bg(bg)
        .role(Role::Button, label)
        .ring(4.0)
        .disabled(!enabled)
        .child(text(label, 12.0, fg).weight(500).line_height(16.0).silent());
    b = if enabled {
        b.on(Action::Click)
    } else {
        b.opacity(0.4)
    };
    tooltip(s.ui, b, id, tip, None)
}

/// Terminals still on an older build: Restart Vorn moves them over; the
/// older holder's can be ended from the row's menu, confirmed in place.
fn holder_row(s: &Screen<'_>, b: &settings::Behind) -> El {
    let st = &s.st.settings;
    let asking = b.holder.is_some() && st.confirm_end == b.holder;
    let mut words = div()
        .col()
        .grow()
        .min_w0()
        .child(
            text(&b.title, 13.0, c(look::GRAY_200))
                .line_height(18.0)
                .role(Role::Label, b.title.as_str()),
        )
        .child(
            text(
                if asking {
                    "End every terminal on that version? Their programs stop."
                } else {
                    b.detail.as_str()
                },
                12.0,
                c(look::GRAY_500),
            )
            .line_height(16.0)
            .mt(2.0)
            .wrapping()
            .silent(),
        );
    if let Some(f) = &st.end_failure {
        words = words.child(
            text(f, 12.0, c(look::RED_400))
                .line_height(16.0)
                .mt(4.0)
                .wrapping()
                .silent(),
        );
    }
    let mut controls = div().row().items_center().gap(6.0).shrink0();
    match (&b.holder, asking) {
        (Some(h), true) => {
            controls = controls
                .child(pill_button(
                    s,
                    "keep-holder",
                    "Cancel",
                    SetAct::KeepHolder,
                    tip::RESTART_VORN,
                    false,
                    true,
                ))
                .child(danger_button(s, h));
        }
        (holder, false) => {
            if !settings::IN_BROWSER {
                controls = controls.child(pill_button(
                    s,
                    "restart-vorn",
                    "Restart Vorn",
                    SetAct::RestartVorn,
                    tip::RESTART_VORN,
                    false,
                    true,
                ));
            }
            if let Some(h) = holder {
                let id = s
                    .acts
                    .on("holder-more", Act::Open(MenuKind::Holder(h.clone())));
                controls = controls.child(icon_button(s, id, glyph::ELLIPSIS, "More", "More"));
            }
        }
        (None, true) => {}
    }
    let row = div()
        .row()
        .items_center()
        .gap(12.0)
        .px(14.0)
        .py(12.0)
        .child(
            div()
                .w(48.0)
                .shrink0()
                .center()
                .child(ico("info", 16.0, 2.0, c(look::AMBER_400))),
        )
        .child(words)
        .child(controls);
    let row = match &b.holder {
        Some(h) => s.acts.context(
            row,
            Id::new("holder-row").child(h.as_str()),
            Act::Open(MenuKind::Holder(h.clone())),
        ),
        None => row,
    };
    let id = Id::new("holder-tip");
    tooltip(s.ui, row, id, tip::BEHIND, None)
}

/// End them, confirmed: red text on a neutral pill.
fn danger_button(s: &Screen<'_>, holder: &str) -> El {
    let id = s
        .acts
        .on("end-holder", set(SetAct::EndHolder(holder.to_owned())));
    let hot = s.ui.hover(id);
    div()
        .id(id)
        .on(Action::Click)
        .row()
        .items_center()
        .h(28.0)
        .px(12.0)
        .rounded(4.0)
        .shrink0()
        .bg(white(0.06).mix(white(0.1), hot))
        .role(Role::Button, "End them")
        .ring(4.0)
        .child(
            text("End them", 12.0, c(look::RED_400))
                .weight(500)
                .line_height(16.0)
                .silent(),
        )
}

/// Installed extension packs: all up to date, or the ones with an update and
/// one button that updates them.
fn extensions_row(s: &Screen<'_>) -> El {
    let (installed, outdated) = crate::plugins::pack_updates(&s.st.plugins.listings);
    let names: Vec<&str> = outdated.iter().map(|l| l.name.as_str()).collect();
    let note = settings::extensions_note(installed, &names);
    let mut control = div().row().items_center().gap(10.0).shrink0().child(
        text(note, 12.0, c(look::GRAY_500))
            .line_height(16.0)
            .silent(),
    );
    if !outdated.is_empty() {
        control = control.child(pill_button(
            s,
            "update-extensions",
            &format!("Update {}", outdated.len()),
            SetAct::UpdateExtensions,
            tip::UPDATE_EXTENSIONS,
            false,
            true,
        ));
    }
    card_row(s, None, "Extensions", None, control)
}

/// A link under the card: its name and an arrow, opening in the browser.
fn link_pill(s: &Screen<'_>, id: Id, label: &str) -> El {
    let hot = s.ui.hover(id);
    div()
        .id(id)
        .on(Action::Click)
        .row()
        .items_center()
        .gap(4.0)
        .h(26.0)
        .px(10.0)
        .rounded(4.0)
        .bg(white(0.03).mix(white(0.07), hot))
        .border(1.0, white(0.06))
        .role(Role::Link, label)
        .ring(4.0)
        .child(
            text(label, 12.0, c(look::GRAY_400).mix(c(look::GRAY_200), hot))
                .line_height(16.0)
                .silent(),
        )
        .child(glyph_icon(glyph::ARROW_UP_RIGHT, 11.0, c(look::GRAY_500)))
}

/// `ConnectorIcon`: a built-in connector's mark, else the external link.
fn connector_mark(id: &str, size: f32, color: Rgba) -> El {
    let name = match id {
        "http" => "globe",
        "mcp" | "sdk" => "mcp",
        other if look::icon(other, 2.0).is_some() => other,
        _ => "external-link",
    };
    ico(name, size, 2.0, color)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn written(a: SetAct) -> (String, String) {
        match a {
            SetAct::Default(k, v) => (k, v),
            other => panic!("not a default: {other:?}"),
        }
    }

    #[test]
    fn the_look_card_writes_the_keys_and_values_appearance_always_stored() {
        let pairs = |k: &str, v: &str| (k.to_owned(), v.to_owned());
        assert_eq!(
            written(surface_act(Materials::Solid)),
            pairs("materials", "\"solid\"")
        );
        assert_eq!(
            written(surface_act(Materials::Native)),
            pairs("materials", "\"native\"")
        );
        assert_eq!(
            written(surface_act(Materials::Themed)),
            pairs("materials", "\"themed\"")
        );
        assert_eq!(
            written(chrome_act(true)),
            pairs("interfaceStyle", "\"tinted\"")
        );
        assert_eq!(
            written(chrome_act(false)),
            pairs("interfaceStyle", "\"neutral\"")
        );
        assert_eq!(
            written(contrast_act(true)),
            pairs("contrastCorrection", "true")
        );
        assert_eq!(
            written(contrast_act(false)),
            pairs("contrastCorrection", "false")
        );
    }
}

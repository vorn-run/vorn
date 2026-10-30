//! The smaller panels: Appearance, General (with `ShellPicker`),
//! Notifications and About.

use serde_json::Value;

use vorn_theme::{Appearance as Look, Chrome, Materials, Scheme, Theme};

use super::{button, card, card_row, header, section, segmented, set, svg, toggle, Seg};
use crate::act::{Act, Acts};
use crate::look;
use crate::screen::{agent_icon, c, ico, white, Screen};
use crate::settings::{self, sigil, SetAct, UpdateAction, UpdateStatus};
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

pub(super) fn default_act(key: &str, v: impl ToString) -> SetAct {
    SetAct::Default(key.to_owned(), v.to_string())
}

fn seg_act(key: &str, v: impl ToString) -> Act {
    set(default_act(key, v))
}

/// The Card Row Height slider's track.
pub const ROW_SLIDER: Id = Id(0x726f_7768_6569_6768);

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
pub(in crate::screen) mod glyph {
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
    pub const CHECK: &str = r#"<path d="M20 6L9 17l-5-5"/>"#;
    pub const REFRESH: &str = r#"<path d="M21 12a9 9 0 01-15.5 6.2L3 16M3 12a9 9 0 0115.5-6.2L21 8M21 3v5h-5M3 21v-5h5"/>"#;
    pub const CHEVRON_RIGHT: &str = r#"<path d="M9 18l6-6-6-6"/>"#;
    pub const CHEVRON_LEFT: &str = r#"<path d="M15 18l-6-6 6-6"/>"#;
    pub const TERMINAL: &str = r#"<path d="M4 17l6-6-6-6M12 19h8"/>"#;
    pub const ARROW_UP_RIGHT: &str = r#"<path d="M7 17L17 7M7 7h10v10"/>"#;
    pub const BELL: &str = r#"<path d="M6 8a6 6 0 0112 0c0 7 3 9 3 9H3s3-2 3-9"/><path d="M10.3 21a1.9 1.9 0 003.4 0"/>"#;
    pub const BELL_OFF: &str = r#"<path d="M10.3 21a1.9 1.9 0 003.4 0M2 2l20 20M8.7 3A6 6 0 0118 8a20 20 0 00.8 5.8M17 17H3s3-2 3-9"/>"#;
    pub const APP_WINDOW: &str =
        r#"<rect x="3" y="4" width="18" height="14" rx="2"/><path d="M3 9h18"/>"#;
    pub const VOLUME: &str =
        r#"<path d="M11 5L6 9H2v6h4l5 4zM15.5 8.5a5 5 0 010 7M19 5a10 10 0 010 14"/>"#;
    pub const MESSAGE_QUESTION: &str = r#"<path d="M21 15a2 2 0 01-2 2H7l-4 4V5a2 2 0 012-2h14a2 2 0 012 2z"/><path d="M9.5 9a2.5 2.5 0 113.5 2.3c-.6.3-1 .8-1 1.5M12 15h.01"/>"#;
    pub const FILE: &str = r#"<path d="M20 7h-3a2 2 0 01-2-2V2M9 18a2 2 0 01-2-2V4a2 2 0 012-2h7l4 4v10a2 2 0 01-2 2z"/>"#;
    pub const ELLIPSIS: &str = r#"<circle cx="5" cy="12" r="1"/><circle cx="12" cy="12" r="1"/><circle cx="19" cy="12" r="1"/>"#;
}

/// What each settings row, tile and chip says when hovered.
pub(crate) mod tip {
    #[cfg(target_os = "macos")]
    pub const SCHEME: &str = "Matches your Mac's light or dark mode on System";
    #[cfg(not(target_os = "macos"))]
    pub const SCHEME: &str = "Matches your computer's light or dark mode on System";
    pub const THEME: &str = "Sets the colours of Vorn and its terminals";
    #[cfg(target_os = "macos")]
    pub const LIGHT_THEME: &str = "Applies while your Mac is in light mode";
    #[cfg(not(target_os = "macos"))]
    pub const LIGHT_THEME: &str = "Applies while your computer is in light mode";
    #[cfg(target_os = "macos")]
    pub const DARK_THEME: &str = "Applies while your Mac is in dark mode";
    #[cfg(not(target_os = "macos"))]
    pub const DARK_THEME: &str = "Applies while your computer is in dark mode";
    pub const THEME_INFO: &str = "Shows the theme's terminal colours and publisher";
    pub const MORE_THEMES: &str = "Finds more themes in the extensions catalog";
    pub const SURFACES: &str = "Changes how the sidebar and toolbar are drawn";
    pub const SURFACES_RESTART: &str =
        "Changes how the sidebar and toolbar are drawn, after a restart";
    pub const SOLID: &str = "Draws every surface opaque";
    pub const GLASS: &str = "Shows the desktop through the sidebar and toolbar";
    pub const TINTED_GLASS: &str = "Shows the desktop through, tinted in the theme's colours";
    pub const NO_GLASS: &str = "Needs Reduce transparency off in System Settings";
    pub const CHROME: &str = "Paints the sidebar and panels in the theme's colours";
    pub const CONTRAST: &str = "Lightens or darkens hard-to-read text, keeping its hue";
    pub const SMALLER: &str = "Makes terminal text 1 px smaller";
    pub const LARGER: &str = "Makes terminal text 1 px larger";
    pub const LIGATURES: &str = "Joins symbols like != and -> into one shape";
    pub const GRID: &str = "Shows sessions side by side";
    pub const TABS: &str = "Shows one session at a time, with tabs to switch";
    pub const LIST: &str = "Lists sessions beside the open one";
    pub const CARD_HEIGHT: &str = "Applies to cards in the Grid layout";
    #[cfg(target_os = "macos")]
    pub const LOGIN: &str = "Opens Vorn when you sign in to your Mac";
    #[cfg(not(target_os = "macos"))]
    pub const LOGIN: &str = "Opens Vorn when you sign in to your computer";
    pub const REOPEN: &str = "Brings back the sessions open when Vorn last quit";
    pub const AGENT: &str = "Starts new sessions with this agent";
    pub const STARTS: &str = "What the toolbar's + and its key start, at once";
    pub const STARTS_AGENT: &str = "Starts the agent above";
    pub const STARTS_SHELL: &str = "Starts a plain shell";
    pub const ARRANGE: &str = "Order and show or hide what each place shows";
    pub const CARD_SHOWN: &str =
        "How many buttons a card's header shows; the rest go in its More menu";
    pub const CUSTOMIZE: &str =
        "Opens the sheet that arranges the toolbar, the sidebar's tabs and a card's panes";
    pub const MORE_AGENTS: &str = "Finds more agents in the extensions catalog";
    #[cfg(target_os = "macos")]
    pub const AGENT_ROW: &str = "Shows Ready once Vorn finds the agent's command on this Mac";
    #[cfg(not(target_os = "macos"))]
    pub const AGENT_ROW: &str = "Shows Ready once Vorn finds the agent's command here";
    pub const TOOLS: &str = "Gives the agent Vorn's tools for panes, sessions and workflows";
    pub const DEFAULT_TAG: &str = "Marks the agent new sessions start with";
    pub const INSTALL_AGENT: &str = "Runs the installer in a terminal card you can watch";
    pub const CONNECT: &str = "Runs the setup below, then checks that it worked";
    pub const MAKE_DEFAULT: &str = "Starts new sessions with this agent";
    pub const ADVANCED: &str = "Shows the command and arguments that start this agent";
    #[cfg(target_os = "macos")]
    pub const REMOVE_AGENT: &str = "Removes the adapter from Vorn; the agent stays on your Mac";
    #[cfg(not(target_os = "macos"))]
    pub const REMOVE_AGENT: &str = "Removes the adapter from Vorn; the agent stays installed";
    pub const MANAGE_AGENTS: &str = "Opens Settings \u{203a} Agents";
    pub const DOCS: &str = "Opens the agent's docs in your browser";
    pub const REDETECT: &str = "Looks for installed agents again";
    pub const SHELL: &str = "Runs in every new terminal session";
    pub const CHOOSE_PATH: &str = "Picks a shell that isn't in the list";
    pub const BEHIND: &str = "Moves running terminals to this version when Vorn restarts";
    pub const RESTART_VORN: &str = "Restarts Vorn and moves your terminals over; they keep running";
    pub const CHECK: &str = "Asks for a newer version of Vorn now";
    pub const CHANNEL: &str = "Picks Beta for early updates or Stable for tested ones";
    pub const RESTART_UPDATE: &str =
        "Installs the update; Vorn reopens and your sessions keep running";
    pub const WHATS_NEW: &str = "Opens this version's release notes";
    pub const UPDATE_EXTENSIONS: &str = "Installs the newest version of each listed extension";
    pub const NOTIFY: &str = "Alerts you when something needs you while you're in another app";
    pub const SHOW_BACKGROUND: &str = "Notifies only while you're in another app";
    pub const SHOW_ALWAYS: &str = "Notifies even while you're looking at Vorn";
    pub const SOUND: &str = "Plays with each notification; None is silent";
    pub const ON_WAITING: &str = "Notifies when an agent stops to wait for your reply or approval";
    pub const ON_BELL: &str = "Notifies when a program in a terminal asks for your attention";
    pub const FROM_EXTENSIONS: &str = "Turns each extension's notifications on or off";
    #[cfg(target_os = "macos")]
    pub const OPEN_SYSTEM_SETTINGS: &str = "Opens macOS's notification settings for Vorn";
    #[cfg(not(target_os = "macos"))]
    pub const OPEN_SYSTEM_SETTINGS: &str = "Opens the system's notification settings for Vorn";
    pub const DRAWING: &str = "Draws pixels where the terminal shows images; Text works anywhere";
}

pub(in crate::screen) fn glyph_icon(body: &str, size: f32, color: Rgba) -> El {
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
            tip: None,
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
pub(in crate::screen) fn icon_button(
    s: &Screen<'_>,
    id: Id,
    body: &str,
    label: &str,
    tip: &str,
) -> El {
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
        tip::SURFACES_RESTART
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
    let says = if disabled { tip::NO_GLASS } else { says };
    let act = set(surface_act(m));
    let on = m == chosen && !disabled;
    tile(s, "surface", act, pic, label, on, disabled, says)
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
        card_row(s, None, "Font", None, font_chip(s, family)),
        card_row(s, None, "Size", None, size_control(s, size)),
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
    chip(s, id, family, open)
        .child(
            text(family, 13.0, c(look::GRAY_200))
                .family(family)
                .line_height(18.0)
                .silent(),
        )
        .child(chevron(open))
}

/// Three Aa steps, then the exact size between a step down and up.
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
    let exact = div().w(40.0).row().justify_center().child(
        text(format!("{size} px"), 12.0, c(look::GRAY_300))
            .line_height(16.0)
            .silent(),
    );
    div().row().items_center().gap(12.0).child(steps).child(
        div()
            .row()
            .items_center()
            .gap(4.0)
            .child(stepper(s, size, -1))
            .child(exact)
            .child(stepper(s, size, 1)),
    )
}

/// A one-px step down or up, off at the end of the range.
fn stepper(s: &Screen<'_>, size: i64, delta: i32) -> El {
    let (glyph, label, tip) = if delta < 0 {
        ("minus", "Smaller", tip::SMALLER)
    } else {
        ("plus", "Larger", tip::LARGER)
    };
    let id = s.acts.on("font-size", set(SetAct::FontSize(delta)));
    let off = settings::step_font_size(size, delta) == size;
    let hot = if off { 0.0 } else { s.ui.hover(id) };
    let el = div()
        .id(id)
        .size(24.0, 24.0)
        .center()
        .rounded(4.0)
        .bg(white(0.06 * hot))
        .role(Role::Button, label)
        .child(ico(
            glyph,
            14.0,
            2.0,
            c(look::GRAY_400).mix(c(look::WHITE), hot),
        ));
    let el = if off {
        el.opacity(0.4).disabled(true)
    } else {
        el.on(Action::Click).ring(4.0)
    };
    tooltip(s.ui, el, id, tip, None)
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
        card_row(s, None, "Layout", None, tiles).py(12.0),
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
    let more_id = s.acts.on(
        "more-themes",
        set(SetAct::Browse(Some(crate::plugins::list::Tag::Theme), None)),
    );
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
        tip: None,
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
        (false, _) => "This terminal can't show images.",
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
            col = col.child(note("No shells found. Choose a path instead."));
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
                (false, _) => "Starts and reads sessions from any terminal.".to_owned(),
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
            card_row(s, None, "New session starts", Some(tip::STARTS), starts(s)),
            card_row(s, None, "Agent", Some(tip::AGENT), agent_chip(s)),
            card_row(s, None, "Shell", Some(tip::SHELL), shell_picker(s)),
        ]))
        .child(section("Layout"))
        .child(card(vec![
            card_row(
                s,
                None,
                "Toolbar, sidebar tabs and card buttons",
                Some(tip::ARRANGE),
                customize_button(s),
            ),
            card_row(
                s,
                None,
                "Buttons on the card",
                Some(tip::CARD_SHOWN),
                crate::screen::customize::card_stepper(s),
            ),
        ]))
}

/// New session starts: the agent below, or a shell.
fn starts(s: &Screen<'_>) -> El {
    let key = crate::app::NEW_SESSION_STARTS;
    let shell = defaults(s).and_then(|d| d.get(key)).and_then(Value::as_str) == Some("shell");
    let seg = |label, v: &str, on, tip| Seg {
        label,
        icon: None,
        act: seg_act(key, format!("\"{v}\"")),
        on,
        disabled: false,
        tip: Some(tip),
    };
    segmented(
        s,
        key,
        vec![
            seg("Agent", "agent", !shell, tip::STARTS_AGENT),
            seg("Shell", "shell", shell, tip::STARTS_SHELL),
        ],
        12.0,
    )
}

/// Customize…, opening the sheet on the toolbar.
fn customize_button(s: &Screen<'_>) -> El {
    let id = s.acts.on(
        "general-customize",
        Act::Customize(crate::arrange::List::Toolbar),
    );
    let hot = s.ui.hover(id);
    let b = div()
        .id(id)
        .on(Action::Click)
        .px(12.0)
        .py(4.0)
        .rounded(4.0)
        .bg(white(0.04).mix(white(0.08), hot))
        .border(1.0, white(0.08))
        .ring(4.0)
        .role(Role::Button, "Customize")
        .child(
            text("Customize\u{2026}", 12.0, c(look::GRAY_200))
                .line_height(16.0)
                .silent(),
        );
    tooltip(s.ui, b, id, tip::CUSTOMIZE, None)
}

/// The agent new sessions start with: the stored one, else the first.
pub(in crate::screen) fn default_agent<'a>(s: &Screen<'a>) -> &'a str {
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

/// The agent picker: search, the agents ready on this machine with a check
/// on the default, then Manage agents… and more from the catalog.
pub(crate) fn agent_picker(s: &Screen<'_>) -> El {
    let query = s.ui.edit_text(AGENT_SEARCH).trim().to_lowercase();
    let chosen = default_agent(s);
    let mut list = div().col().p(6.0).role(Role::ListBox, "Agents");
    let mut any = false;
    for a in s.store.agents.iter().filter(|a| {
        settings::is_ready(s.st.installed.as_deref(), &a.key)
            && (query.is_empty() || a.name.to_lowercase().contains(&query))
    }) {
        any = true;
        let id = s.acts.on(
            "agent-option",
            set(default_act("defaultAgent", format!("\"{}\"", a.key))),
        );
        let on = a.key == chosen;
        let lit = s.ui.hover(id).max(if on { 1.0 } else { 0.0 });
        list = list.child(
            div()
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
                )
                .child_if(on.then(|| glyph_icon(glyph::CHECK, 12.0, c(look::GRAY_400)))),
        );
    }
    if !any {
        list = list.child(
            text("No agents match", 12.0, c(look::GRAY_500))
                .line_height(16.0)
                .px(10.0)
                .py(16.0),
        );
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
    let manage_id = s.acts.on(
        "manage-agents",
        set(SetAct::Open(settings::Category::Agents)),
    );
    let manage_ink = c(look::GRAY_300).mix(c(look::WHITE), s.ui.hover(manage_id));
    let manage = div()
        .id(manage_id)
        .on(Action::Click)
        .role(Role::Link, "Manage agents")
        .ring(2.0)
        .child(
            text("Manage agents\u{2026}", 12.0, manage_ink)
                .line_height(16.0)
                .silent(),
        );
    let more_id = s.acts.on(
        "more-agents",
        set(SetAct::Browse(Some(crate::plugins::list::Tag::Agent), None)),
    );
    let more_ink = c(look::BRONZO).mix(c(look::WHITE), s.ui.hover(more_id) * 0.3);
    let more = div()
        .id(more_id)
        .on(Action::Click)
        .role(Role::Link, "Get more agents")
        .ring(2.0)
        .child(
            text("Get more\u{2026}", 12.0, more_ink)
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
                .row()
                .items_center()
                .justify_between()
                .px(14.0)
                .py(10.0)
                .border_top(1.0, white(0.06))
                .child(tooltip(s.ui, manage, manage_id, tip::MANAGE_AGENTS, None))
                .child(tooltip(s.ui, more, more_id, tip::MORE_AGENTS, None)),
        )
}

/// The `defaults.notifications` preferences, as the notifier reads them.
fn notif(s: &Screen<'_>) -> crate::notify::Prefs {
    crate::notify::Prefs::of(&s.store.config)
}

/// What a Notifications control stores under `defaults.notifications[key]`.
pub(super) fn notify_act(key: &str, v: impl ToString) -> SetAct {
    SetAct::Notify(key.to_owned(), v.to_string())
}

/// A row icon on the Notifications page.
fn notif_mark(body: &str) -> El {
    glyph_icon(body, 15.0, c(look::GRAY_500))
}

pub(super) fn notifications(s: &Screen<'_>) -> El {
    use crate::notify::Show;
    let p = notif(s);
    let off = !p.enabled;
    let switch = |place: &str, key: &str, on: bool, label: &str, disabled: bool| {
        toggle(s, place, on, set(notify_act(key, !on)), disabled, label).shrink0()
    };
    let show = |label, v: Show, tip| Seg {
        label,
        icon: None,
        act: set(notify_act("show", format!("\"{}\"", v.key()))),
        on: p.show == v,
        disabled: off,
        tip: Some(tip),
    };
    let shown = segmented(
        s,
        "notif-show",
        vec![
            show(
                "When Vorn is in the background",
                Show::Background,
                tip::SHOW_BACKGROUND,
            ),
            show("Always", Show::Always, tip::SHOW_ALWAYS),
        ],
        10.0,
    );
    let mut main = Vec::new();
    if crate::notify::blocked() {
        main.push(blocked_row(s));
    }
    main.push(card_row(
        s,
        Some(notif_mark(glyph::BELL)),
        "Notifications",
        Some(tip::NOTIFY),
        switch(
            "notif-enabled",
            "enabled",
            p.enabled,
            "Notifications",
            false,
        ),
    ));
    main.push(card_row(
        s,
        Some(notif_mark(glyph::APP_WINDOW)),
        "Show them",
        None,
        shown,
    ));
    main.push(card_row(
        s,
        Some(notif_mark(glyph::VOLUME)),
        "Sound",
        None,
        sound_chip(s, &p.sound, off),
    ));
    let when = card(vec![
        card_row(
            s,
            Some(notif_mark(glyph::MESSAGE_QUESTION)),
            "An agent needs your answer",
            Some(tip::ON_WAITING),
            switch(
                "notif-waiting",
                "onWaiting",
                p.on_waiting,
                "An agent needs your answer",
                off,
            ),
        ),
        card_row(
            s,
            Some(notif_mark(glyph::TERMINAL)),
            "A terminal rings the bell",
            Some(tip::ON_BELL),
            switch(
                "notif-bell",
                "onBell",
                p.on_bell,
                "A terminal rings the bell",
                off,
            ),
        ),
    ]);
    div()
        .col()
        .child(header("Notifications", "", None))
        .child(card(main))
        .child(section("Tell me when"))
        .child(when)
        .child_if(sources_row(s).map(|r| card(vec![r]).mt(16.0)))
}

/// The system refuses Vorn's notifications: say so, with the way to fix it.
fn blocked_row(s: &Screen<'_>) -> El {
    let open = pill_button(
        s,
        "notif-system",
        "Open System Settings",
        SetAct::OpenNotifSettings,
        tip::OPEN_SYSTEM_SETTINGS,
        false,
        true,
    );
    div()
        .row()
        .items_center()
        .justify_between()
        .gap(16.0)
        .min_h(56.0)
        .px(14.0)
        .py(12.0)
        .child(
            div()
                .row()
                .items_start()
                .gap(8.0)
                .min_w0()
                .child(glyph_icon(glyph::BELL_OFF, 15.0, c(look::BRONZO)).mt(2.0))
                .child(
                    div()
                        .col()
                        .gap(2.0)
                        .min_w0()
                        .child(
                            text(BLOCKED, 13.0, c(look::GRAY_200))
                                .line_height(18.0)
                                .role(Role::Label, BLOCKED),
                        )
                        .child(
                            text(BLOCKED_FIX, 12.0, c(look::GRAY_500))
                                .line_height(16.0)
                                .wrapping()
                                .silent(),
                        ),
                ),
        )
        .child(open)
}

#[cfg(target_os = "macos")]
const BLOCKED: &str = "macOS is blocking Vorn's notifications";
#[cfg(not(target_os = "macos"))]
const BLOCKED: &str = "The system is blocking Vorn's notifications";
#[cfg(target_os = "macos")]
const BLOCKED_FIX: &str = "Turn them on for Vorn in System Settings \u{203a} Notifications";
#[cfg(not(target_os = "macos"))]
const BLOCKED_FIX: &str = "Turn them on for Vorn in the system's notification settings";

/// How a stored sound reads: Default, None or the system sound's name.
fn sound_name(sound: &str) -> &str {
    match sound {
        "default" => "Default",
        "none" => "None",
        name => name,
    }
}

/// The Sound chip, opening the list of sounds.
fn sound_chip(s: &Screen<'_>, sound: &str, disabled: bool) -> El {
    let kind = MenuKind::NotifSound;
    let open = s.st.menu.as_ref().is_some_and(|m| m.kind == kind);
    let id = s.acts.on("notif-sound", Act::Open(kind));
    let name = sound_name(sound);
    let mut el = chip(s, id, name, open)
        .child(
            text(name, 12.0, c(look::GRAY_200))
                .line_height(16.0)
                .silent(),
        )
        .child(chevron(open));
    if disabled {
        el = el.opacity(0.4);
    }
    tooltip(s.ui, el, id, tip::SOUND, None)
}

/// The Sound menu: Default, None, then the system's own sounds.
pub(crate) fn sound_items(s: &Screen<'_>) -> Vec<crate::screen::menus::Item> {
    use crate::screen::menus::Item;
    let cur = notif(s).sound;
    let pick = |label: &str, v: &str| {
        let act = set(notify_act("sound", Value::from(v)));
        Item::pick(label, cur == v, act)
    };
    let mut items = vec![pick("Default", "default"), pick("None", "none")];
    items.extend(crate::notify::system_sounds().iter().map(|n| pick(n, n)));
    items
}

/// The From extensions row: the extensions that send notifications and how
/// many are on; none installed, no row.
fn sources_row(s: &Screen<'_>) -> Option<El> {
    let lists = [
        s.st.points.builtins.as_slice(),
        s.st.plugins.packs.as_slice(),
    ];
    let sources = crate::notify::sources(&lists);
    if sources.is_empty() {
        return None;
    }
    let on = sources.iter().filter(|x| x.on).count();
    let mut marks = div().row().items_center();
    for (i, x) in sources.iter().take(4).enumerate() {
        let mark = source_mark(s, x.id, 16.0).border(2.0, c(look::SURFACE_OVERLAY));
        marks = marks.child(if i == 0 { mark } else { mark.ml(-4.0) });
    }
    let kind = MenuKind::NotifSources;
    let open = s.st.menu.as_ref().is_some_and(|m| m.kind == kind);
    let id = s.acts.on("notif-sources", Act::Open(kind));
    let label = format!("{on} of {} on", sources.len());
    let count = chip(s, id, &label, open)
        .child(
            text(&label, 12.0, c(look::GRAY_200))
                .line_height(16.0)
                .silent(),
        )
        .child(chevron(open));
    let control = div()
        .row()
        .items_center()
        .gap(8.0)
        .child(marks)
        .child(tooltip(s.ui, count, id, tip::FROM_EXTENSIONS, None));
    Some(card_row(
        s,
        Some(notif_mark(glyph::FILE)),
        "From extensions",
        Some(tip::FROM_EXTENSIONS),
        control,
    ))
}

/// An extension's mark on a small square.
fn source_mark(s: &Screen<'_>, id: &str, size: f32) -> El {
    let listing = s.st.plugins.listings.iter().find(|l| l.id == id);
    let glyph = crate::screen::plugins::glyph(listing, id, size * 0.625, c(look::GRAY_200));
    div()
        .size(size, size)
        .shrink0()
        .center()
        .rounded(4.0)
        .bg(c(look::GRAY_700))
        .child(glyph)
}

/// The From extensions popover: a switch per extension for all its
/// notifications; which kinds stay on each extension's page.
pub(crate) fn notice_sources(s: &Screen<'_>) -> El {
    use crate::points::PointAct;
    let lists = [
        s.st.points.builtins.as_slice(),
        s.st.plugins.packs.as_slice(),
    ];
    let mut list = div().col();
    for x in crate::notify::sources(&lists) {
        let act = Act::Point(PointAct::Preference {
            id: x.id.to_owned(),
            key: crate::notify::ALL_KEY.to_owned(),
            value: (!x.on).to_string(),
        });
        let place = format!("notif-source-{}", x.id);
        list = list.child(
            div()
                .row()
                .items_center()
                .justify_between()
                .gap(12.0)
                .px(10.0)
                .py(8.0)
                .child(
                    div()
                        .row()
                        .items_center()
                        .gap(10.0)
                        .min_w0()
                        .child(source_mark(s, x.id, 18.0))
                        .child(
                            text(x.name, 13.0, c(look::GRAY_200))
                                .line_height(18.0)
                                .truncate()
                                .role(Role::Label, x.name),
                        ),
                )
                .child(toggle(s, &place, x.on, act, false, x.name).shrink0()),
        );
    }
    crate::ui::widgets::popover("From extensions")
        .w(290.0)
        .p(6.0)
        .child(list)
        .child(
            text(
                "Choose which ones on each extension's page",
                11.0,
                c(look::GRAY_500),
            )
            .line_height(14.0)
            .px(10.0)
            .pt(8.0)
            .pb(4.0)
            .mt(4.0)
            .border_top(1.0, white(0.06))
            .silent(),
        )
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
pub(super) fn pill_button(
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
    // The tooltip keeps the context menu's id, so right-click still finds it.
    let (row, id) = match &b.holder {
        Some(h) => {
            let id = Id::new("holder-row").child(h.as_str());
            let row = s
                .acts
                .context(row, id, Act::Open(MenuKind::Holder(h.clone())));
            (row, id)
        }
        None => (row, Id::new("holder-tip")),
    };
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

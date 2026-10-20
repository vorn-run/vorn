//! The smaller panels: Appearance, General (with `ShellPicker`), Updates,
//! Notifications, Skills & MCP, Keys and About.

use serde_json::Value;

use super::{button, header, row, segmented, set, svg, toggle, Seg};
use crate::look;
use crate::screen::menus::Item;
use crate::screen::{agent_icon, c, ico, white, Screen};
use crate::settings::{
    self, describe_field, describe_update, describe_usage, older_note, sigil, SetAct, Tone,
    UpdateAction, UpdateStatus,
};
use crate::state::{now_ms, MenuKind};
use crate::ui::{div, icon, image, text, Action, El, Id, Rgba, Role, Sense, TextStyle};

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

/// A toggle row writing `defaults[key]`.
fn switch_row(
    s: &Screen<'_>,
    key: &str,
    on: bool,
    title: &str,
    description: &str,
    note: Option<&str>,
) -> El {
    let t = toggle(s, key, on, default_act(key, !on), false, title);
    row(title, description, note, t, false)
}

/// The font size's `w-7 h-7` steppers.
fn stepper(s: &Screen<'_>, glyph: &str, delta: i32) -> El {
    let id = s.acts.on("font-size", set(SetAct::FontSize(delta)));
    div()
        .id(id)
        .on(Action::Click)
        .size(28.0, 28.0)
        .center()
        .rounded(6.0)
        .bg(white(0.06).mix(white(0.1), s.ui.hover(id)))
        .role(Role::Button, if delta < 0 { "Smaller" } else { "Larger" })
        .child(
            text(glyph, 14.0, c(look::GRAY_300))
                .line_height(20.0)
                .silent(),
        )
}

/// The Card Row Height slider's track.
pub const ROW_SLIDER: Id = Id(0x726f_7768_6569_6768);
/// The notification volume slider's track.
pub const VOLUME_SLIDER: Id = Id(0x766f_6c75_6d65_7364);

/// A range input as Chromium draws one with `accent-blue-500`.
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
                .bg(c(look::BLUE_500)),
        )
        .child(
            div()
                .absolute(x, 0.0)
                .size(knob, knob)
                .rounded(knob / 2.0)
                .bg(c(look::BLUE_500)),
        )
}

fn small_svg(body: &str) -> El {
    icon(svg(body, 2.0), 14.0, c(look::GRAY_500))
}

pub(super) fn appearance(s: &Screen<'_>) -> El {
    let size = num(s, "fontSize").unwrap_or(f64::from(look::TERM_FONT));
    let font = div()
        .row()
        .items_center()
        .gap(8.0)
        .child(stepper(s, "-", -1))
        .child(
            div().w(32.0).row().justify_center().child(
                text(format!("{size}"), 14.0, c(look::GRAY_200))
                    .line_height(20.0)
                    .silent(),
            ),
        )
        .child(stepper(s, "+", 1));
    let h = s.st.settings.row_height.round();
    let rows = div()
        .row()
        .items_center()
        .gap(12.0)
        .child(slider(
            ROW_SLIDER,
            128.0,
            (h - 100.0) / 500.0,
            "Card Row Height",
        ))
        .child(
            div().w(48.0).row().justify_end().child(
                text(format!("{h}px"), 14.0, c(look::GRAY_400))
                    .line_height(20.0)
                    .silent(),
            ),
        );
    let layout = defaults(s)
        .and_then(|d| d.get("layoutMode"))
        .and_then(Value::as_str)
        .unwrap_or("grid");
    let task_view = defaults(s)
        .and_then(|d| d.get("taskViewMode"))
        .and_then(Value::as_str)
        .unwrap_or("list");
    let seg_icon = |active: bool, body: &str| {
        let _ = active;
        small_svg(body)
    };
    let layout_seg = segmented(
        s,
        "layout-mode",
        vec![
            Seg {
                label: "Grid",
                icon: Some(seg_icon(
                    true,
                    r#"<rect x="3" y="3" width="7" height="7"/><rect x="14" y="3" width="7" height="7"/><rect x="3" y="14" width="7" height="7"/><rect x="14" y="14" width="7" height="7"/>"#,
                )),
                act: default_act("layoutMode", "\"grid\""),
                on: layout == "grid",
                disabled: false,
            },
            Seg {
                label: "Tabs",
                icon: Some(seg_icon(
                    true,
                    r#"<rect x="3" y="3" width="18" height="18" rx="2"/><path d="M3 9h18"/><path d="M9 3v6"/>"#,
                )),
                act: default_act("layoutMode", "\"tabs\""),
                on: layout == "tabs",
                disabled: false,
            },
        ],
        12.0,
    );
    let task_seg = segmented(
        s,
        "task-view",
        vec![
            Seg {
                label: "List",
                icon: Some(small_svg(
                    r#"<line x1="8" y1="6" x2="21" y2="6"/><line x1="8" y1="12" x2="21" y2="12"/><line x1="8" y1="18" x2="21" y2="18"/><line x1="3" y1="6" x2="3.01" y2="6"/><line x1="3" y1="12" x2="3.01" y2="12"/><line x1="3" y1="18" x2="3.01" y2="18"/>"#,
                )),
                act: default_act("taskViewMode", "\"list\""),
                on: task_view == "list",
                disabled: false,
            },
            Seg {
                label: "Kanban",
                icon: Some(small_svg(
                    r#"<rect x="3" y="3" width="18" height="18" rx="2"/><path d="M9 3v18"/><path d="M15 3v18"/>"#,
                )),
                act: default_act("taskViewMode", "\"kanban\""),
                on: task_view == "kanban",
                disabled: false,
            },
        ],
        12.0,
    );
    let mut rows_col = div().col().gap(4.0).child(colour_rows(s));
    if let Some(d) = s.term_drawing {
        rows_col = rows_col.child(drawing_row(s, d));
    }
    div()
        .col()
        .child(header("Appearance", "Customize how Vorn looks", None))
        .child(
            rows_col
                .child(row(
                    "Font Size",
                    "Terminal font size in pixels",
                    None,
                    font,
                    false,
                ))
                .child(row(
                    "Card Row Height",
                    &format!("Terminal card height in pixels ({h}px)"),
                    None,
                    rows,
                    false,
                ))
                .child(row(
                    "Layout Mode",
                    "Choose between grid or tab layout for sessions",
                    None,
                    layout_seg,
                    false,
                ))
                .child(row(
                    "Default Task View",
                    "Choose between list or kanban board for tasks",
                    None,
                    task_seg,
                    false,
                )),
        )
}

/// The id of a theme's preview tile, which the app reads the hover of.
pub(crate) fn theme_tile_id(theme: &str) -> Id {
    Id::new(&format!("theme-tile:{theme}"))
}

/// A choice stored as a string in `defaults`, and its options.
fn text_seg(s: &Screen<'_>, key: &str, fallback: &str, opts: &[(&'static str, &str)]) -> El {
    let chosen = defaults(s)
        .and_then(|d| d.get(key))
        .and_then(Value::as_str)
        .unwrap_or(fallback);
    let segs = opts
        .iter()
        .map(|(label, v)| Seg {
            label,
            icon: None,
            act: default_act(key, format!("\"{v}\"")),
            on: chosen == *v,
            disabled: false,
        })
        .collect();
    segmented(s, key, segs, 12.0)
}

/// Colour scheme, the light and dark themes, interface style, contrast,
/// materials, and the palette now drawn.
fn colour_rows(s: &Screen<'_>) -> El {
    use crate::appearance as a;
    use vorn_theme::Appearance as Look;
    let prefs = a::prefs(defaults(s));
    let scheme = text_seg(
        s,
        a::SCHEME,
        prefs.scheme.key(),
        &[("System", "system"), ("Light", "light"), ("Dark", "dark")],
    );
    let chrome = text_seg(
        s,
        a::CHROME,
        prefs.chrome.key(),
        &[("Neutral", "neutral"), ("Themed", "tinted")],
    );
    let reduced = s.ui.reduced_transparency();
    let materials = text_seg(
        s,
        a::MATERIALS,
        prefs.materials.key(),
        &[
            ("Solid", "solid"),
            ("Native", "native"),
            ("Themed", "themed"),
        ],
    );
    div()
        .col()
        .child(row(
            "Colour Scheme",
            "Follow the system, or always use the light or the dark theme",
            None,
            scheme,
            false,
        ))
        .child(theme_row(s, a::LIGHT, "Light Theme", Look::Light, &prefs.light))
        .child(theme_row(s, a::DARK, "Dark Theme", Look::Dark, &prefs.dark))
        .child(row(
            "Interface Style",
            "Neutral keeps Vorn's own surfaces; Themed tints them with the theme",
            None,
            chrome,
            false,
        ))
        .child(switch_row(
            s,
            a::CONTRAST,
            prefs.contrast,
            "Contrast Correction",
            "Lift text that reads below 4.5:1, keeping its hue",
            None,
        ))
        .child(row(
            "Materials",
            "Solid surfaces, the system's translucent sidebar and blurred menus, or both tinted by the theme",
            reduced.then_some("Reduce transparency is on in the system settings, so surfaces stay solid."),
            materials,
            reduced,
        ))
        .child(palette_row())
}

/// A row of tiles, one per theme of `look`, each drawn in its own colours.
fn theme_row(
    s: &Screen<'_>,
    key: &str,
    title: &str,
    kind: vorn_theme::Appearance,
    chosen: &str,
) -> El {
    let installed = s.st.settings.appearance.installed();
    let mut tiles = div().row().wrap_children().gap(8.0).mt(12.0);
    for t in vorn_theme::bundled()
        .iter()
        .chain(installed)
        .filter(|t| t.appearance == kind)
    {
        tiles = tiles.child(theme_tile(s, key, t, t.id == chosen));
    }
    div()
        .col()
        .py(16.0)
        .border_bottom(1.0, white(0.06))
        .child(
            text(title, 14.0, c(look::GRAY_200))
                .weight(500)
                .line_height(20.0)
                .role(Role::Label, title),
        )
        .child(
            text(
                "Point at a theme to preview it; click to use it",
                12.0,
                c(look::GRAY_500),
            )
            .line_height(16.0)
            .mt(2.0)
            .silent(),
        )
        .child(tiles)
}

/// A theme's preview: its window, sidebar, ink and accent, and its eight
/// terminal colours, with its name under it.
fn theme_tile(s: &Screen<'_>, key: &str, t: &vorn_theme::Theme, on: bool) -> El {
    use vorn_theme::Token as T;
    let id = theme_tile_id(&t.id);
    s.acts
        .bind(id, set(default_act(key, format!("\"{}\"", t.id))));
    let hot = s.ui.hover(id);
    let rgba = |token| Rgba::from(t.get(token));
    let line = |w: f32, token| div().size(w, 3.0).rounded(1.5).bg(rgba(token));
    let mut ansi = div().row().gap(2.0);
    for &c in &t.terminal.ansi[1..7] {
        ansi = ansi.child(div().size(8.0, 8.0).rounded(2.0).bg(Rgba::from(c)));
    }
    let edge = if on {
        white(0.45)
    } else {
        white(0.08).mix(white(0.2), hot)
    };
    let preview = div()
        .row()
        .size(120.0, 72.0)
        .rounded(4.0)
        .clip()
        .bg(rgba(T::SurfaceBase))
        .border(1.0, edge)
        .child(
            div()
                .col()
                .gap(4.0)
                .w(32.0)
                .h_full()
                .p(6.0)
                .bg(rgba(T::SurfacePanel))
                .child(line(18.0, T::InkSecondary))
                .child(line(14.0, T::InkFaint))
                .child(line(16.0, T::InkFaint)),
        )
        .child(
            div()
                .col()
                .gap(5.0)
                .p(8.0)
                .grow()
                .child(line(48.0, T::Ink))
                .child(line(36.0, T::InkSecondary))
                .child(div().size(20.0, 6.0).rounded(2.0).bg(rgba(T::Accent)))
                .child(ansi),
        );
    let name = if on {
        c(look::GRAY_100)
    } else {
        c(look::GRAY_400)
    };
    div()
        .id(id)
        .on(Action::Click)
        .col()
        .gap(6.0)
        .role(Role::RadioButton, &t.name)
        .selected(on)
        .ring(4.0)
        .child(preview)
        .child(text(&t.name, 12.0, name).line_height(16.0).silent())
}

/// The terminal palette now drawn: 16 ANSI colours, then foreground,
/// background, cursor and selection, each with its value.
fn palette_row() -> El {
    let term = crate::ui::palette::terminal();
    let swatch = |label: String, color: Rgba| {
        div()
            .row()
            .items_center()
            .gap(6.0)
            .w(144.0)
            .child(
                div()
                    .size(14.0, 14.0)
                    .rounded(3.0)
                    .bg(color)
                    .border(1.0, white(0.08)),
            )
            .child(
                text(
                    format!("{label} #{:06x}", color.to_hex()),
                    12.0,
                    c(look::GRAY_400),
                )
                .line_height(16.0)
                .silent(),
            )
    };
    let mut grid = div().row().wrap_children().gap(6.0).mt(12.0);
    for (i, &color) in term.ansi.iter().enumerate() {
        grid = grid.child(swatch(format!("{i:>2}"), color));
    }
    for (label, color) in [
        ("fg", term.foreground),
        ("bg", term.background),
        ("cursor", term.cursor),
        ("sel", term.selection),
    ] {
        grid = grid.child(swatch(label.to_owned(), color));
    }
    div()
        .col()
        .py(16.0)
        .child(
            text("Terminal Palette", 14.0, c(look::GRAY_200))
                .weight(500)
                .line_height(20.0)
                .role(Role::Label, "Terminal Palette"),
        )
        .child(
            text(
                "The theme's colours as the terminal draws them now",
                12.0,
                c(look::GRAY_500),
            )
            .line_height(16.0)
            .mt(2.0)
            .silent(),
        )
        .child(grid)
}

/// Settings › Experimental: one switch for everything still being tried.
pub(super) fn experimental(s: &Screen<'_>) -> El {
    div()
        .col()
        .child(header(
            "Experimental",
            "Features still being tried. They may change or go away.",
            None,
        ))
        .child(switch_row(
            s,
            "experimental",
            flag(s, "experimental", false),
            "Try Experimental Features",
            "Images in terminals: pictures a program draws with the kitty graphics protocol show in its card, on every device.",
            None,
        ))
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
        act: default_act("tuiDrawing", format!("\"{v}\"")),
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
    let note = match (d.supported, d.pixels) {
        (false, _) => "This terminal does not show images",
        (true, true) => "Drawing pixels now",
        (true, false) => "Drawing text now",
    };
    row(
        "Drawing",
        "Pixels look like the desktop app where the terminal shows images; Text works in any terminal",
        Some(note),
        choice,
        false,
    )
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
            .w(288.0)
            .shrink0()
            .px(8.0)
            .py(6.0)
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
    let hot = if open { 1.0 } else { s.ui.hover(id) };
    div()
        .id(id)
        .on(Action::Click)
        .row()
        .items_center()
        .gap(8.0)
        .w(288.0)
        .shrink0()
        .px(8.0)
        .py(6.0)
        .rounded(4.0)
        .border(1.0, white(0.12).mix(white(0.25), hot))
        .role(Role::ComboBox, "Default Shell")
        .expanded(open)
        .child(shell_glyph(current.map(|c| c.family.as_str())))
        .child(
            div()
                .col()
                .grow()
                .min_w0()
                .child(
                    text(name, 13.0, c(look::GRAY_200))
                        .line_height(19.5)
                        .truncate()
                        .silent(),
                )
                .child(
                    text(value, 11.0, c(look::GRAY_600))
                        .mono()
                        .line_height(16.5)
                        .truncate()
                        .silent(),
                ),
        )
        .child(ico("chevron-down", 12.0, 2.0, c(look::GRAY_500)))
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
            col = col.child(note("No shells detected. Enter a path instead."));
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
    col.child(div().mt(4.0).mb(4.0).h(1.0).w_full().bg(white(0.06)))
        .child(
            div()
                .id(edit)
                .on(Action::Click)
                .row()
                .items_center()
                .gap(8.0)
                .px(12.0)
                .py(8.0)
                .bg(white(0.0).mix(white(0.06), s.ui.hover(edit)))
                .role(Role::Button, "Enter a path\u{2026}")
                .child(ico("pen-line", 13.0, 2.0, c(look::GRAY_500)))
                .child(
                    text("Enter a path\u{2026}", 12.0, c(look::GRAY_400))
                        .line_height(16.0)
                        .silent(),
                ),
        )
}

/// The agent picker's `flex bg-white/[0.04] rounded-lg p-0.5` buttons.
fn agent_choice(s: &Screen<'_>) -> El {
    let current = defaults(s)
        .and_then(|d| d.get("defaultAgent"))
        .and_then(Value::as_str)
        .unwrap_or(s.store.agents.default_key());
    let mut row = div()
        .row()
        .shrink0()
        .gap(2.0)
        .p(2.0)
        .rounded(8.0)
        .bg(white(0.04));
    for a in &s.store.agents {
        let installed = s.st.installed.as_ref().is_none_or(|l| l.contains(&a.key));
        let id = s.acts.on(
            "default-agent",
            set(default_act("defaultAgent", format!("\"{}\"", a.key))),
        );
        let hot = s.ui.hover(id);
        let on = current == a.key;
        let fg = if !installed {
            c(look::GRAY_600)
        } else if on {
            c(look::WHITE)
        } else {
            c(look::GRAY_500).mix(c(look::GRAY_300), hot)
        };
        let short = a.name.split(' ').next().unwrap_or(&a.name);
        let mut b = div()
            .id(id)
            .row()
            .items_center()
            .gap(6.0)
            .px(10.0)
            .py(4.0)
            .rounded(6.0)
            .role(Role::RadioButton, a.name.as_str())
            .selected(on)
            .child(agent_icon(s, &a.key, 14.0))
            .child(text(short, 12.0, fg).line_height(16.0).silent());
        if on && installed {
            b = b.bg(white(0.1));
        }
        b = if installed {
            b.on(Action::Click)
        } else {
            b.opacity(0.3)
        };
        let tip = if installed {
            a.name.clone()
        } else {
            format!("{} is not installed", a.name)
        };
        row = row.child(crate::ui::tooltip(s.ui, b, id, &tip, None));
    }
    row
}

/// A native `<select>`: the value and the system's chevron.
fn select(s: &Screen<'_>, key: &str, label: &str) -> El {
    let id = s.acts.on(
        "select",
        crate::act::Act::Open(MenuKind::SetSelect(key.to_owned())),
    );
    let open =
        s.st.menu
            .as_ref()
            .is_some_and(|m| m.kind == MenuKind::SetSelect(key.to_owned()));
    div()
        .id(id)
        .on(Action::Click)
        .row()
        .items_center()
        .w(128.0)
        .shrink0()
        .px(12.0)
        .py(6.0)
        .rounded(6.0)
        .bg(white(0.04))
        .border(1.0, if open { white(0.15) } else { white(0.08) })
        .role(Role::ComboBox, key)
        .child(
            div().grow().min_w0().child(
                text(label, 14.0, c(look::GRAY_200))
                    .line_height(20.0)
                    .silent(),
            ),
        )
        .child(ico("chevron-down", 12.0, 2.0, c(look::GRAY_400)))
}

/// A select's options: (stored value, label).
fn options(key: &str) -> &'static [(i64, &'static str)] {
    match key {
        "headlessRetentionMinutes" => &[
            (1, "1 minute"),
            (5, "5 minutes"),
            (15, "15 minutes"),
            (30, "30 minutes"),
            (60, "1 hour"),
        ],
        _ => &[
            (15, "15 minutes"),
            (30, "30 minutes"),
            (60, "1 hour"),
            (120, "2 hours"),
            (360, "6 hours"),
            (0, "No limit"),
        ],
    }
}

fn select_value(s: &Screen<'_>, key: &str) -> i64 {
    let fallback = if key == "headlessRetentionMinutes" {
        1.0
    } else {
        60.0
    };
    num(s, key).unwrap_or(fallback) as i64
}

/// The open select's entries.
pub(crate) fn select_items(s: &Screen<'_>, key: &str) -> Vec<Item> {
    let v = select_value(s, key);
    options(key)
        .iter()
        .map(|(n, l)| Item::pick(*l, *n == v, set(default_act(key, n))))
        .collect()
}

fn select_row(s: &Screen<'_>, key: &str, title: &str, description: &str) -> El {
    let v = select_value(s, key);
    let label = options(key)
        .iter()
        .find(|(n, _)| *n == v)
        .map_or_else(|| v.to_string(), |(_, l)| (*l).to_owned());
    row(title, description, None, select(s, key, &label), false)
}

/// Where the `vorn` command is, with Install, or Reinstall and Remove.
fn cli_row(s: &Screen<'_>) -> Option<El> {
    let st = s.st.settings.cli.as_ref()?;
    let note =
        s.st.settings
            .cli_error
            .clone()
            .or_else(|| st.note.clone())
            .unwrap_or_else(|| {
                if st.installed && !st.on_path {
                    format!("{} \u{2014} its folder is not on your PATH", st.path)
                } else {
                    st.path.clone()
                }
            });
    let description = if st.installed {
        "Installed. Run `vorn --help` in a terminal."
    } else {
        "The vorn command starts sessions and reads them from a terminal."
    };
    let enabled = st.available && !st.managed && !s.st.settings.cli_busy;
    let control = if st.installed {
        div()
            .row()
            .gap(6.0)
            .shrink0()
            .child(button(s, "cli", "Reinstall", SetAct::InstallCli, enabled))
            .child(button(
                s,
                "cli-remove",
                "Remove",
                SetAct::RemoveCli,
                enabled,
            ))
    } else {
        button(s, "cli", "Install", SetAct::InstallCli, enabled)
    };
    Some(row(
        "Command-Line Tool",
        description,
        Some(&note),
        control,
        !st.available,
    ))
}

pub(super) fn general(s: &Screen<'_>) -> El {
    let reopen = flag(s, "reopenSessions", true);
    let headless = flag(s, "showHeadlessAgents", true);
    let stays = settings::server_stays_up(&s.store.config);
    let mut rows = div()
        .col()
        .gap(4.0)
        .child(row(
            "Default Coding Agent",
            "Pre-selected agent when creating new sessions",
            None,
            agent_choice(s),
            false,
        ))
        .child(row(
            "Default Shell",
            "Shell used for terminal sessions. Shells differ in how much they can report about each command.",
            None,
            shell_picker(s),
            false,
        ))
        .child_if((!settings::IN_BROWSER).then(|| switch_row(
            s,
            "startAtLogin",
            flag(s, "startAtLogin", false),
            "Start Vorn When I Sign In",
            "Vorn opens with the machine, so a restart puts you back where you were.",
            reopen.then_some("With Reopen Sessions on, your agents start again at sign-in rather than when you next open Vorn."),
        )))
        .child(switch_row(
            s,
            "reopenSessions",
            reopen,
            "Reopen Sessions on Startup",
            "Bring back the panes you had open. Agents still running are reconnected to, keeping the turn they were in; ones whose process was stopped are started again where they left off, and shells reopen in the directory they were in.",
            None,
        ))
        .child(switch_row(
            s,
            "keepSessionsRunning",
            flag(s, "keepSessionsRunning", true),
            "Keep Sessions Running When Vorn Closes",
            "Agents keep working while Vorn is shut. Reopening reconnects to them instead of restarting them. A background server stays running until every session ends, then shuts down on its own.",
            stays.as_deref(),
        ))
        .child(switch_row(
            s,
            "domBlockRendering",
            flag(s, "domBlockRendering", true),
            "Command Blocks",
            "Draw finished commands as blocks with their own padding, boundary and copy button, instead of leaving them in the terminal grid",
            None,
        ))
        .child(switch_row(
            s,
            "minimalShellPrompt",
            flag(s, "minimalShellPrompt", true),
            "Minimal Shell Prompt",
            "Replace your shell prompt in terminal sessions so each command reads as a heading. Turn off to keep your own prompt exactly as your shell renders it",
            None,
        ))
        .child(switch_row(
            s,
            "enableHoverPreview",
            flag(s, "enableHoverPreview", false),
            "Hover Preview",
            "Preview sessions by hovering over them in the sidebar",
            None,
        ));
    rows = rows.child_if(cli_row(s));
    rows = rows.child(switch_row(
        s,
        "showHeadlessAgents",
        headless,
        "Show Headless Agents",
        "Display background agent sessions above the session grid",
        None,
    ));
    if headless {
        rows = rows.child(select_row(
            s,
            "headlessRetentionMinutes",
            "Completed Agent Retention",
            "How long to show completed headless agents before auto-hiding",
        ));
    }
    rows = rows.child(select_row(
        s,
        "headlessStepTimeoutMinutes",
        "Workflow Step Timeout",
        "How long a headless workflow step may run before its agent is killed and the step fails. Individual steps can override this.",
    ));
    div()
        .col()
        .child(header(
            "General",
            "Application preferences and defaults",
            None,
        ))
        .child(rows)
}

fn tone_dot(t: Tone) -> Rgba {
    match t {
        Tone::Blocked => c(look::BRONZO),
        Tone::Broken => c(look::DANGER),
        Tone::Live => c(look::INK),
        Tone::Settled => look::INK.alpha(look::INK_FAINT),
        Tone::Idle => look::INK.alpha(0.18),
    }
}

pub(super) fn updates(s: &Screen<'_>) -> El {
    let channel = defaults(s)
        .and_then(|d| d.get("updateChannel"))
        .and_then(Value::as_str)
        .unwrap_or("stable")
        .to_owned();
    let status =
        s.st.settings
            .update
            .clone()
            .unwrap_or(UpdateStatus::Unsupported);
    let view = describe_update(&status, &channel, now_ms());
    let live: Vec<_> = s.store.sessions.iter().filter(|x| !x.ended()).collect();
    let cost = (view.action == Some(UpdateAction::Restart))
        .then(|| {
            settings::update_cost(
                live.len(),
                live.iter().any(|x| x.status() == "running"),
                !crate::update::ENDS_SESSIONS,
            )
        })
        .flatten();
    let mut info = div().col().grow().min_w0().child(
        text(&view.label, 13.0, c(look::GRAY_200))
            .line_height(19.5)
            .silent(),
    );
    if let Some(cost) = cost {
        info = info.child(
            text(cost, 12.0, c(look::BRONZO))
                .line_height(16.0)
                .mt(2.0)
                .silent(),
        );
    } else if let Some(d) = &view.detail {
        info = info.child(
            text(d, 12.0, c(look::GRAY_500))
                .line_height(16.0)
                .mt(2.0)
                .silent(),
        );
    }
    if let Some(p) = view.percent {
        info = info.child(
            div().h(3.0).mt(8.0).rounded(2.0).bg(white(0.08)).child(
                div()
                    .h_full()
                    .w_pct(p as f32 / 100.0)
                    .rounded(2.0)
                    .bg(white(0.3)),
            ),
        );
    }
    let mut card = div()
        .row()
        .items_center()
        .gap(12.0)
        .mb(20.0)
        .px(16.0)
        .py(12.0)
        .rounded(8.0)
        .border(1.0, white(0.08))
        .bg(white(0.03))
        .role(Role::Status, view.label.as_str())
        .child(
            div()
                .size(5.0, 5.0)
                .rounded(3.0)
                .shrink0()
                .bg(tone_dot(view.tone)),
        )
        .child(info);
    if let Some(a) = view.action {
        let (label, lead) = match a {
            UpdateAction::Restart => ("Restart Now", true),
            UpdateAction::Download => ("Download", false),
            UpdateAction::Retry => ("Retry", false),
        };
        let id = s.acts.on("update-action", set(SetAct::UpdateNow));
        let hot = s.ui.hover(id);
        card = card.child(
            div()
                .id(id)
                .on(Action::Click)
                .px(12.0)
                .py(4.0)
                .rounded(6.0)
                .border(1.0, white(0.08))
                .bg(if lead {
                    white(0.1).mix(white(0.14), hot)
                } else {
                    white(0.06).mix(white(0.1), hot)
                })
                .child(
                    text(
                        label,
                        12.0,
                        if lead {
                            c(look::WHITE)
                        } else {
                            c(look::GRAY_300)
                        },
                    )
                    .weight(500)
                    .line_height(16.0)
                    .silent(),
                ),
        );
    }
    let checking = matches!(status, UpdateStatus::Checking);
    let can_check = !checking && status != UpdateStatus::Unsupported;
    let check = button(
        s,
        "check-updates",
        if checking {
            "Checking\u{2026}"
        } else {
            "Check Now"
        },
        SetAct::CheckUpdates,
        can_check,
    );
    let app = env!("CARGO_PKG_VERSION");
    let mut rows = div().col().gap(4.0).child(row(
        "Current version",
        &format!("Vorn {app}"),
        None,
        check,
        false,
    ));
    if let Some(server) = &s.st.server_version {
        let (description, trailing) = settings::describe_server(server, app, true);
        rows = rows.child(row(
            "Terminal server",
            &description,
            None,
            text(trailing, 12.0, c(look::GRAY_500))
                .line_height(16.0)
                .silent(),
            false,
        ));
    }
    let channel_seg = segmented(
        s,
        "update-channel",
        vec![
            Seg {
                label: "Stable",
                icon: None,
                act: default_act("updateChannel", "\"stable\""),
                on: channel == "stable",
                disabled: false,
            },
            Seg {
                label: "Beta",
                icon: None,
                act: default_act("updateChannel", "\"beta\""),
                on: channel == "beta",
                disabled: false,
            },
        ],
        12.0,
    );
    rows = rows
        .child(row(
            "Update channel",
            "Beta receives early releases; stable receives tested releases only",
            None,
            channel_seg,
            false,
        ))
        .child(switch_row(
            s,
            "updateAutoDownload",
            flag(s, "updateAutoDownload", true),
            "Download automatically",
            "Installing still waits for you to restart",
            None,
        ));
    div()
        .col()
        .child(header("Updates", "How Vorn keeps itself current", None))
        .child(card)
        .child(rows)
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
    let mut logo = div().mb(12.0).h(24.0);
    if let Some(id) = s.logo {
        logo = logo.child(image(id, 24.0 * look::LOGO_ASPECT, 24.0).opacity(0.6));
    }
    let mut col = div()
        .col()
        .child(
            text("About", 20.0, c(look::WHITE))
                .weight(600)
                .line_height(28.0)
                .mb(24.0)
                .role(Role::Heading, "About"),
        )
        .child(logo)
        .child(
            text(
                format!("Vorn v{}", env!("CARGO_PKG_VERSION")),
                14.0,
                c(look::GRAY_300),
            )
            .line_height(20.0)
            .silent(),
        )
        .child(
            text("\u{a9} 2026 Javier Canizalez", 12.0, c(look::GRAY_600))
                .line_height(16.0)
                .mt(4.0)
                .silent(),
        );
    let mut status = div().col().mt(24.0);
    if let Some(v) = &st.core_version {
        status = status.child(
            text(format!("Native core {v}"), 12.0, c(look::GRAY_500))
                .line_height(16.0)
                .silent(),
        );
    }
    let note_box = |t: String| {
        div()
            .mt(8.0)
            .px(16.0)
            .py(12.0)
            .rounded(8.0)
            .border(1.0, white(0.08))
            .bg(white(0.03))
            .child(
                text(t, 12.0, c(look::GRAY_400))
                    .line_height(16.0)
                    .wrapping()
                    .silent(),
            )
    };
    if let Some(h) = &st.holders {
        if let (Some(e), false) = (&h.error, h.current) {
            status = status.child(note_box(format!("The session holder is not running: {e}.")));
        }
        for holder in h.older.iter().filter(|o| o.sessions != Some(0)) {
            let asking = st.confirm_end.as_deref() == Some(holder.instance.as_str());
            let label = if asking { "End them?" } else { "End them" };
            let id = s.acts.on(
                "end-holder",
                set(SetAct::EndHolder(holder.instance.clone())),
            );
            let hot = s.ui.hover(id);
            status = status.child(
                div()
                    .row()
                    .items_center()
                    .gap(12.0)
                    .mt(8.0)
                    .px(16.0)
                    .py(12.0)
                    .rounded(8.0)
                    .border(1.0, white(0.08))
                    .bg(white(0.03))
                    .child(
                        div().grow().min_w0().child(
                            text(older_note(holder), 12.0, c(look::GRAY_400))
                                .line_height(16.0)
                                .wrapping()
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
                            .border(1.0, white(0.12))
                            .bg(white(0.0).mix(white(0.06), hot))
                            .role(Role::Button, label)
                            .child(
                                text(label, 12.0, c(look::GRAY_300))
                                    .line_height(16.0)
                                    .silent(),
                            ),
                    ),
            );
        }
    }
    if let Some(f) = &st.end_failure {
        status = status.child(
            text(f, 12.0, c(look::RED_400))
                .line_height(16.0)
                .mt(8.0)
                .silent(),
        );
    }
    col = col.child(status);
    col
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

//! The Extensions page: what is installed and what can be, an extension's page,
//! the add forms and the install sheet. Everything here reads the model in
//! `crate::plugins` and fires its `PlugAct`s; the RPC glue is `app::plugins`.

mod browse;
mod detail;
mod forms;
mod installed;
mod sheet;

use serde_json::Value;

pub(in crate::screen) use sheet::dialog;

use super::settings::header;
use super::{ico, white, Screen};
use crate::act::Act;
use crate::library::{mark_svg, parse_icon, Look};
use crate::plugins::{
    count, describe_contributions, details, places, tabs_id, Activity, Listing, Origin, PackStatus,
    Pending, PlugAct, Source, Tab, Tone,
};
use crate::runs::s as sv;
use crate::state::MenuKind;
use crate::ui::theme::color as k;
use crate::ui::widgets::{pills, Pill};
use crate::ui::{div, icon, text, Action, El, Rgba, Role};

/// The small corner radius.
const R: f32 = 4.0;

pub(super) fn page(s: &Screen<'_>) -> El {
    let p = &s.st.plugins;
    // Nothing to say until the first answers land; an empty list would read as "none".
    if !p.loaded {
        return div().col().child(page_header());
    }
    if let Some(a) = &p.adding {
        return div().col().child(forms::view(s, a));
    }
    if let Some(key) = p.selected.as_deref() {
        if let Some(l) = p.listing(key) {
            return detail::view(s, l);
        }
        if let Some(b) = key.strip_prefix(detail::BUILTIN).and_then(|id| {
            s.st.points
                .builtins
                .iter()
                .find(|b| sv(b, "id") == Some(id))
        }) {
            return detail::builtin(s, b);
        }
    }
    let tab = shown_tab(s);
    let col = div().col().child(page_header()).child(bar(s, tab));
    match tab {
        Tab::Installed => col.child(installed::view(s)),
        Tab::Browse => col.child(browse::view(s)),
    }
}

fn page_header() -> El {
    header(
        "Extensions",
        "Each adds to Vorn: sidebar sections, card lines, commands, windows or connections. Each runs as you, with only what it declares.",
        None,
    )
}

pub(super) fn act(a: PlugAct) -> Act {
    Act::Plug(a)
}

/// The tabs to show: Browse only while Connectors is on.
fn tabs(s: &Screen<'_>) -> Vec<Tab> {
    crate::plugins::tabs(crate::plugins::connectors_on(&s.st.points.builtins))
}

/// The tab drawn: the one chosen, unless Connectors is off and it is one of Connectors'.
fn shown_tab(s: &Screen<'_>) -> Tab {
    let tab = s.st.plugins.tab;
    if tabs(s).contains(&tab) {
        tab
    } else {
        Tab::Installed
    }
}

/// The Install menu's entries.
pub(in crate::screen) fn install_items() -> Vec<crate::screen::menus::Item> {
    Origin::ALL
        .iter()
        .map(|o| {
            crate::screen::menus::Item::new(o.icon(), o.label(), act(PlugAct::InstallFrom(*o)))
                .detail(o.detail())
        })
        .collect()
}

/// Installed and Browse, and Install… beside them.
fn bar(s: &Screen<'_>, current: Tab) -> El {
    let shown = tabs(s);
    let items: Vec<Pill<'_>> = shown.iter().map(|t| Pill::new(t.label(), None)).collect();
    let on = shown.iter().position(|t| *t == current).unwrap_or(0);
    div()
        .row()
        .items_center()
        .wrap_children()
        .gap(12.0)
        .mb(16.0)
        .child(pills(s.ui, tabs_id(), "Extensions", &items, on))
        .child(div().grow())
        .child(install_button(s))
}

/// Install…, which opens the menu of where from.
fn install_button(s: &Screen<'_>) -> El {
    let kind = MenuKind::ExtInstall;
    let open = s.st.menu.as_ref().is_some_and(|m| m.kind == kind);
    let label = "Install\u{2026}";
    let id = s.acts.on("plug-install", Act::Open(kind));
    let hot = if open { 1.0 } else { s.ui.hover(id) };
    let fg = k::GRAY_300.mix(k::WHITE, hot);
    chip(hot)
        .id(id)
        .on(Action::Click)
        .ring(R)
        .role(Role::Button, label)
        .has_popup()
        .expanded(open)
        .child(ico("plus", 11.0, 2.0, fg))
        .child(text(label, 11.5, fg).line_height(16.0).silent())
}

/// The bordered row button's box, washed by `hot`.
fn chip(hot: f32) -> El {
    div()
        .row()
        .items_center()
        .gap(4.0)
        .px(10.0)
        .py(4.0)
        .shrink0()
        .rounded(R)
        .border(1.0, white(0.1))
        .bg(white(0.06 * hot))
}

/// A listing's glyph: its own paths, else the built-in mark for its id.
pub(super) fn glyph(l: Option<&Listing>, connector: &str, size: f32, color: Rgba) -> El {
    let own = l.and_then(|l| {
        l.catalog
            .as_ref()
            .and_then(|c| c.get("icon"))
            .or(l.icon.as_ref())
            .and_then(parse_icon)
    });
    let look = Look {
        connector: connector.to_owned(),
        packaged: l.is_some_and(|l| l.source != Source::Builtin),
        icon: own,
    };
    icon(mark_svg(&look).unwrap_or_default(), size, color)
}

/// A square well around a glyph.
pub(super) fn well(size: f32, radius: f32, bg: Rgba, glyph: El) -> El {
    div()
        .size(size, size)
        .shrink0()
        .center()
        .rounded(radius)
        .bg(bg)
        .child(glyph)
}

/// A 6 px status dot.
pub(super) fn dot(color: Rgba) -> El {
    div().size(6.0, 6.0).rounded(3.0).shrink0().bg(color)
}

pub(super) fn tone_dot(t: Tone) -> Rgba {
    match t {
        Tone::Blocked => k::BRONZO,
        Tone::Broken => k::DANGER,
        Tone::Live => k::INK,
        Tone::Settled => k::INK_FAINT,
        Tone::Idle => k::INK_GHOST,
    }
}

/// Text never drops to ghost: a word is thinner than a disc.
pub(super) fn tone_text(t: Tone) -> Rgba {
    match t {
        Tone::Idle => k::INK_FAINT,
        other => tone_dot(other),
    }
}

/// A dot and a line of text in one tone.
pub(super) fn tone_line(t: Tone, line: &str, size: f32) -> El {
    div()
        .row()
        .items_center()
        .gap(6.0)
        .min_w0()
        .child(dot(tone_dot(t)))
        .child(
            text(line, size, tone_text(t))
                .line_height(size * 1.5)
                .wrapping()
                .min_w0(),
        )
}

/// What a row is doing, or why its last try failed.
pub(super) fn activity_line(s: &Screen<'_>, id: &str) -> Option<El> {
    Some(match s.st.plugins.activity.get(id)? {
        Activity::Busy(verb) => tone_line(Tone::Live, verb.phrase(), 11.0),
        Activity::Failed(e) => tone_line(Tone::Broken, e, 11.0),
    })
}

/// The 1 px install bar, when the install reports a percent.
pub(super) fn progress_bar(status: &PackStatus) -> Option<El> {
    let pct = status.percent?.clamp(0.0, 100.0) as f32;
    Some(
        div()
            .w_full()
            .max_w(220.0)
            .h(1.0)
            .mt(8.0)
            .bg(white(0.08))
            .clip()
            .child(div().h(1.0).w(220.0 * pct / 100.0).bg(k::INK)),
    )
}

/// The bordered `text-[11.5px]` row button, with an optional leading icon.
pub(super) fn chip_button(
    s: &Screen<'_>,
    place: &str,
    a: PlugAct,
    glyph: Option<&str>,
    label: &str,
    fg: Rgba,
    enabled: bool,
) -> El {
    let mut fg = fg;
    let b = if enabled {
        let id = s.acts.on(place, act(a));
        let hot = s.ui.hover(id);
        fg = fg.mix(k::WHITE, hot);
        chip(hot)
            .id(id)
            .on(Action::Click)
            .ring(R)
            .role(Role::Button, label)
    } else {
        chip(0.0)
            .opacity(0.5)
            .disabled(true)
            .role(Role::Button, label)
    };
    let mut b = b;
    if let Some(g) = glyph {
        b = b.child(ico(g, 11.0, 2.0, fg));
    }
    b.child(text(label, 11.5, fg).line_height(16.0).silent())
}

/// The filled `bg-white/10` action: "Use in a workflow", "Add a connection".
pub(super) fn filled_button(
    s: &Screen<'_>,
    place: &str,
    a: PlugAct,
    glyph: &str,
    label: &str,
) -> El {
    let id = s.acts.on(place, act(a));
    let hot = s.ui.hover(id);
    div()
        .id(id)
        .on(Action::Click)
        .ring(R)
        .row()
        .items_center()
        .gap(6.0)
        .px(12.0)
        .py(6.0)
        .shrink0()
        .rounded(R)
        .bg(white(0.1 + 0.05 * hot))
        .role(Role::Button, label)
        .child(ico(glyph, 12.0, 2.0, k::GRAY_100))
        .child(text(label, 12.0, k::GRAY_100).line_height(16.0).silent())
}

/// A bare text button in `rest`, `hot` on hover.
pub(super) fn text_button(
    s: &Screen<'_>,
    place: &str,
    a: PlugAct,
    label: &str,
    size: f32,
    rest: Rgba,
    hot_fg: Rgba,
) -> El {
    let id = s.acts.on(place, act(a));
    let fg = rest.mix(hot_fg, s.ui.hover(id));
    div()
        .id(id)
        .on(Action::Click)
        .ring(R)
        .shrink0()
        .role(Role::Button, label)
        .child(text(label, size, fg).line_height(size * 1.5).silent())
}

/// A connector's triggers and actions, or an extension's surfaces, in words.
pub(super) fn offers(s: &Screen<'_>, l: &Listing) -> Vec<String> {
    if l.kind == crate::plugins::Kind::Extension {
        let n = places(l.contributes.as_ref());
        if n > 0 {
            return vec![count(n, "place")];
        }
        let d = describe_contributions(l.contributes.as_ref());
        return if d.is_empty() { vec![] } else { vec![d] };
    }
    let d = details(l, &s.st.plugins.builtins);
    let mut out = Vec::new();
    if !d.triggers.is_empty() {
        out.push(count(d.triggers.len(), "trigger"));
    }
    if !d.actions.is_empty() {
        out.push(count(d.actions.len(), "action"));
    }
    out
}

/// The `uppercase tracking-[0.08em]` group heading.
pub(super) fn heading(label: &str) -> El {
    text(label.to_uppercase(), 10.0, k::GRAY_600)
        .tracking(0.08)
        .line_height(14.0)
        .pt(16.0)
        .pb(4.0)
        .role(Role::Heading, label)
}

fn strs<'a>(v: &'a Value, key: &str) -> Vec<&'a str> {
    v.get(key)
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .filter_map(|e| {
            e.as_str()
                .or_else(|| sv(e, "label"))
                .or_else(|| sv(e, "type"))
        })
        .collect()
}

/// What a checked pack brings, before it is kept.
pub(super) fn pending_sheet(s: &Screen<'_>, pending: &Pending) -> El {
    let pv = &pending.preview;
    let name = sv(pv, "name").unwrap_or("");
    let version = sv(pv, "version").unwrap_or("");
    let installed = sv(pv, "installedVersion").filter(|v| *v != version);
    let listing = s.st.plugins.listing(&pending.key);
    let mut col = div()
        .col()
        .p(12.0)
        .rounded(R)
        .border(1.0, white(0.1))
        .bg(white(0.02));
    let mut head = div().row().items_center().gap(8.0).child(glyph(
        listing,
        sv(pv, "id").unwrap_or(""),
        20.0,
        k::GRAY_200,
    ));
    head = head.child(
        div()
            .row()
            .items_center()
            .gap(6.0)
            .child(text(name, 13.0, k::GRAY_200).weight(500).line_height(18.0))
            .child(text(format!("v{version}"), 13.0, k::GRAY_500).line_height(18.0)),
    );
    col = col.child(head);
    if let Some(d) = sv(pv, "description").filter(|d| !d.is_empty()) {
        col = col.child(
            text(d, 11.0, k::GRAY_500)
                .line_height(16.0)
                .mt(4.0)
                .wrapping(),
        );
    }
    if let Some(prev) = installed {
        col = col.child(
            text(
                format!("Replaces v{prev}, which is kept so you can roll back."),
                11.0,
                k::GRAY_500,
            )
            .line_height(16.0)
            .mt(4.0)
            .wrapping(),
        );
    }
    let required: Vec<&str> = pv
        .get("env")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .filter(|e| e.get("required") == Some(&Value::Bool(true)))
        .filter_map(|e| sv(e, "name"))
        .collect();
    let borrows = crate::plugins::borrowable(pv);
    let probe = pv
        .pointer("/auth/probe/command")
        .and_then(Value::as_str)
        .unwrap_or("");
    let contributes = pv.get("contributes");
    let mut adds = Vec::new();
    for (key, noun) in [
        ("panes", "pane"),
        ("footers", "footer"),
        ("linkHandlers", "link"),
    ] {
        for item in contributes
            .and_then(|c| c.get(key))
            .and_then(Value::as_array)
            .into_iter()
            .flatten()
        {
            adds.push(format!("{} {noun}", sv(item, "title").unwrap_or("")));
        }
    }
    let asked: Vec<String> = pv
        .get("permissions")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .filter_map(Value::as_str)
        .map(str::to_owned)
        .collect();
    let mut rows: Vec<(String, String)> = Vec::new();
    let triggers = strs(pv, "triggers");
    if !triggers.is_empty() {
        rows.push(("Triggers".into(), triggers.join(", ")));
    }
    let actions = strs(pv, "actions");
    if !actions.is_empty() {
        rows.push(("Actions".into(), actions.join(", ")));
    }
    if !required.is_empty() {
        rows.push(("Needs".into(), required.join(", ")));
    }
    if !borrows.is_empty() {
        rows.push((
            "Borrows".into(),
            format!("{} from {probe}", borrows.join(", ")),
        ));
    }
    if let Some(reaches) = crate::plugins::reaches(pv) {
        rows.push(("Reaches".into(), reaches));
    }
    if let Some(credential) = sv(pv, "credential").filter(|c| !c.is_empty()) {
        rows.push(("Credential".into(), credential.to_owned()));
    }
    if !adds.is_empty() {
        rows.push(("Adds".into(), adds.join(", ")));
    }
    for (bucket, items) in crate::plugins::permission_rows(&asked) {
        rows.push((bucket.into(), items.join(", ")));
    }
    if !rows.is_empty() {
        let mut list = div().col().gap(4.0).mt(8.0);
        for (label, items) in rows {
            list = list.child(
                div()
                    .row()
                    .items_start()
                    .gap(8.0)
                    .child(
                        text(label, 11.0, k::GRAY_600)
                            .line_height(16.0)
                            .w(56.0)
                            .shrink0(),
                    )
                    .child(
                        text(items, 11.0, k::GRAY_400)
                            .line_height(16.0)
                            .wrapping()
                            .grow()
                            .min_w0(),
                    ),
            );
        }
        col = col.child(list);
    }
    let installing = s.st.plugins.installing;
    let confirm = if installing {
        "Installing…"
    } else if installed.is_some() {
        "Update"
    } else {
        "Install"
    };
    col.child(
        div()
            .row()
            .justify_end()
            .gap(8.0)
            .mt(12.0)
            .child(text_button(
                s,
                "plug-pending",
                PlugAct::CancelInstall,
                "Cancel",
                11.0,
                k::GRAY_500,
                k::GRAY_300,
            ))
            .child(chip_button(
                s,
                "plug-pending",
                PlugAct::ConfirmInstall,
                None,
                confirm,
                k::GRAY_200,
                !installing,
            )),
    )
}

/// What a workflow requirement's fix opened, at the top of the editor panel
/// that asked: why an install failed, the pack's confirm sheet, the form.
pub(in crate::screen) fn editor_fix(s: &Screen<'_>) -> Vec<El> {
    let p = &s.st.plugins;
    if !p.from_editor {
        return Vec::new();
    }
    let slot = |inner: El| div().p(8.0).border_bottom(1.0, white(0.08)).child(inner);
    let mut out = Vec::new();
    if let Some(e) = &p.report {
        out.push(
            div()
                .px(16.0)
                .py(8.0)
                .border_bottom(1.0, white(0.08))
                .child(
                    text(e.as_str(), 11.0, k::DANGER)
                        .line_height(16.0)
                        .wrapping(),
                ),
        );
    }
    if let Some(pending) = &p.pending {
        out.push(slot(pending_sheet(s, pending)));
    }
    if let Some(a) = &p.adding {
        out.push(slot(forms::view(s, a)));
    }
    out
}

//! The Plugins page (`ConnectorSettings`): three tabs over the listings, a
//! listing's detail, and the add forms. Everything here reads the model in
//! `crate::plugins` and fires its `PlugAct`s; the RPC glue is `app::plugins`.

mod browse;
mod connections;
mod detail;
mod forms;
mod installed;

use serde_json::Value;

use super::settings::header;
use super::{ico, white, Screen};
use crate::act::Act;
use crate::library::{mark_svg, parse_icon, Look};
use crate::plugins::{
    count, describe_contributions, details, Activity, Listing, PackStatus, Pending, PlugAct,
    Source, Tab, Tone,
};
use crate::runs::s as sv;
use crate::ui::theme::color as k;
use crate::ui::{div, icon, text, Action, El, Rgba, Role};

/// `rounded-sm` in the renderer's Tailwind.
const R: f32 = 4.0;

pub(super) fn page(s: &Screen<'_>) -> El {
    let p = &s.st.plugins;
    let mut col = div()
        .col()
        .child(header(
            "Plugins",
            "Connectors watch a service and start workflows. Extensions add footers and panes to a card. Both install as packs from the same directory.",
            None,
        ))
        .child(tabs(s, p.tab));
    // Nothing to say until the first answers land; an empty list would read as "none".
    if !p.loaded {
        return col;
    }
    let selected = p.selected.as_deref().and_then(|key| p.listing(key));
    let adding = p.adding.is_some();
    col = match p.tab {
        Tab::Installed if !adding && selected.is_none() => col
            .child_if(super::points::builtins(s))
            .child(installed::view(s)),
        Tab::Connections if !adding => col.child(connections::view(s)),
        Tab::Browse if !adding && selected.is_none() => col.child(browse::view(s)),
        _ => col,
    };
    if let (false, Some(l), false) = (p.tab == Tab::Connections, selected, adding) {
        col = col.child(detail::view(s, l));
    }
    if let Some(a) = &p.adding {
        col = col.child(forms::view(s, a));
    }
    col
}

pub(super) fn act(a: PlugAct) -> Act {
    Act::Plug(a)
}

/// The `inline-flex` segmented tabs over the page.
fn tabs(s: &Screen<'_>, current: Tab) -> El {
    let mut row = div()
        .row()
        .p(2.0)
        .gap(0.0)
        .mb(16.0)
        .rounded(R)
        .bg(white(0.04));
    for tab in Tab::ALL {
        let id = s.acts.on("plug-tab", act(PlugAct::Tab(tab)));
        let on = tab == current;
        let fg = if on {
            k::GRAY_200
        } else {
            k::GRAY_500.mix(k::GRAY_300, s.ui.hover(id))
        };
        row = row.child(
            div()
                .id(id)
                .on(Action::Click)
                .ring(R)
                .px(12.0)
                .py(4.0)
                .rounded(R)
                .bg(if on { white(0.08) } else { white(0.0) })
                .role(Role::Button, tab.label())
                .toggled(on)
                .child(text(tab.label(), 12.0, fg).line_height(18.0).silent()),
        );
    }
    div().row().child(row)
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

/// `ActivityLine`: what a row is doing, or why its last try failed.
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
    let mut b = div()
        .row()
        .items_center()
        .gap(4.0)
        .px(10.0)
        .py(4.0)
        .shrink0()
        .rounded(R)
        .border(1.0, white(0.1));
    let mut fg = fg;
    if enabled {
        let id = s.acts.on(place, act(a));
        let hot = s.ui.hover(id);
        fg = fg.mix(k::WHITE, hot);
        b = b
            .id(id)
            .on(Action::Click)
            .ring(R)
            .bg(white(0.06 * hot))
            .role(Role::Button, label);
    } else {
        b = b.opacity(0.5).disabled(true).role(Role::Button, label);
    }
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

/// "Details ›" under a row, lit while the row is hovered.
pub(super) fn details_hint(hot: f32) -> El {
    let fg = k::GRAY_500.mix(k::GRAY_300, hot);
    div()
        .row()
        .items_center()
        .gap(2.0)
        .mt(6.0)
        .child(text("Details", 11.0, fg).line_height(16.0).silent())
        .child(ico("chevron-right", 11.0, 2.0, fg))
}

/// The lettered badge beside a name: "extension", "CLI login", "verified".
pub(super) fn badge(label: &str, glyph: Option<&str>) -> El {
    let mut b = div()
        .row()
        .items_center()
        .gap(2.0)
        .shrink0()
        .px(6.0)
        .py(1.0)
        .rounded(R)
        .border(1.0, white(0.1));
    if let Some(g) = glyph {
        b = b.child(ico(g, 9.0, 2.0, k::GRAY_500));
    }
    b.child(text(label, 10.0, k::GRAY_500).line_height(14.0).silent())
}

/// A connector's triggers and actions, or an extension's surfaces, in words.
pub(super) fn offers(s: &Screen<'_>, l: &Listing) -> Vec<String> {
    if l.kind == crate::plugins::Kind::Extension {
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

/// `PackInstallConfirm`: what a checked pack brings, before it is kept.
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

/// The sheet under the row it was opened from, when it was.
pub(super) fn sheet_for(s: &Screen<'_>, key: &str) -> Option<El> {
    let p = s.st.plugins.pending.as_ref().filter(|p| p.key == key)?;
    Some(div().mt(8.0).mb(12.0).child(pending_sheet(s, p)))
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

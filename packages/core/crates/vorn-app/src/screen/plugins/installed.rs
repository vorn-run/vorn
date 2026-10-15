//! The Installed list: what ships with Vorn, each with its switch, then
//! what was installed or linked.

use serde_json::Value;

use super::{
    act, activity_line, chip_button, detail::BUILTIN, glyph, heading, progress_bar, tone_line,
    well, R,
};
use crate::act::Act;
use crate::plugins::{
    count, describe, hidden_while_off, is_implicit, pack_line, places, Kind, Listing, PlugAct, Tab,
    Tone, CONNECTORS,
};
use crate::points::PointAct;
use crate::runs::s as sv;
use crate::screen::{ico, white, Screen};
use crate::settings::{Category, SetAct};
use crate::ui::theme::color as k;
use crate::ui::{div, text, widgets, Action, El, Role};

pub(super) fn view(s: &Screen<'_>) -> El {
    let p = &s.st.plugins;
    let mut col = div().col().child(heading("Built in"));
    // Connectors leads: the one built-in with a page of its own.
    let mut builtins: Vec<&Value> = s.st.points.builtins.iter().collect();
    builtins.sort_by_key(|b| sv(b, "id") != Some(CONNECTORS));
    for b in builtins {
        col = col.child(builtin(s, b));
    }
    col = col.child(heading("Installed"));
    let mut packs: Vec<&Listing> = p.listings.iter().filter(|l| l.pack.is_some()).collect();
    packs.sort_by_key(|l| (l.kind != Kind::Extension, l.name.to_lowercase()));
    if packs.is_empty() {
        return col.child(empty(s));
    }
    for l in packs {
        col = col.child(pack(s, l));
    }
    col
}

/// "Nothing installed yet", pointing at Browse.
fn empty(s: &Screen<'_>) -> El {
    let browse = s.acts.on("plug-installed", act(PlugAct::Tab(Tab::Browse)));
    let fg = k::GRAY_300.mix(k::WHITE, s.ui.hover(browse));
    let line = |t: &str| text(t, 13.0, k::GRAY_500).line_height(20.0).silent();
    div()
        .row()
        .items_center()
        .wrap_children()
        .py(12.0)
        .border_top(1.0, white(0.06))
        .child(line("Nothing installed yet.\u{a0}"))
        .child(
            div()
                .id(browse)
                .on(Action::Click)
                .ring(R)
                .role(Role::Button, "Browse")
                .border_bottom(1.0, fg)
                .child(text("Browse", 13.0, fg).line_height(18.0).silent()),
        )
        .child(line("\u{a0}for one, or install one from Git."))
}

/// One row: a well, the name over one line, and what sits on the right.
fn row(
    s: &Screen<'_>,
    place: &str,
    open: Act,
    name: &str,
    mark: El,
    line: El,
    right: Option<El>,
) -> El {
    let id = s.acts.on(place, open);
    let hot = s.ui.hover(id);
    let left = div()
        .id(id)
        .on(Action::Click)
        .ring(R)
        .row()
        .items_center()
        .gap(12.0)
        .grow()
        .min_w0()
        .role(Role::Button, format!("About {name}"))
        .child(well(32.0, 6.0, white(0.05 + 0.03 * hot), mark))
        .child(
            div()
                .col()
                .grow()
                .min_w0()
                .child(
                    text(name, 13.5, k::GRAY_200.mix(k::WHITE, hot))
                        .weight(500)
                        .line_height(20.0)
                        .truncate(),
                )
                .child(line),
        );
    div()
        .row()
        .items_center()
        .gap(12.0)
        .py(10.0)
        .border_top(1.0, white(0.06))
        .child(left)
        .child_if(right)
}

/// The muted second line, cut to one line.
fn second(t: &str) -> El {
    text(t, 12.0, k::GRAY_500)
        .line_height(17.0)
        .mt(1.0)
        .truncate()
}

/// A built-in: its places while on (Connectors: its connections), "off"
/// while off, and its switch. Connectors' row opens Settings › Connections while on.
fn builtin(s: &Screen<'_>, b: &Value) -> El {
    let pts = &s.st.points;
    let id = sv(b, "id").unwrap_or_default();
    let name = sv(b, "name").unwrap_or(id);
    let on = b.get("on").and_then(Value::as_bool) == Some(true);
    let connectors = id == CONNECTORS;
    let desc = if connectors {
        "Watches services and starts workflows"
    } else {
        sv(b, "description").unwrap_or_default()
    };
    let tail = if on && connectors {
        let n =
            s.st.plugins
                .connections
                .iter()
                .filter(|c| !is_implicit(c))
                .count();
        format!(" \u{b7} {}", count(n, "connection"))
    } else if on {
        format!(" \u{b7} {}", count(places(b.get("contributes")), "place"))
    } else {
        " \u{b7} off".to_owned()
    };
    let line = div()
        .row()
        .min_w0()
        .mt(1.0)
        .child(
            text(desc, 12.0, k::GRAY_500)
                .line_height(17.0)
                .truncate()
                .min_w0(),
        )
        .child(text(tail, 12.0, k::GRAY_500).line_height(17.0).shrink0());
    let busy = pts.switching.is_some();
    let switch = s.acts.on(
        "ext-builtin",
        Act::Point(PointAct::Builtin {
            id: id.to_owned(),
            on: !on,
        }),
    );
    let mut switch = widgets::toggle(s.ui, switch, name, on, busy).shrink0();
    if pts.switching.as_deref() == Some(id) {
        switch = switch.opacity(0.5);
    }
    let mut col = div().col().child(row(
        s,
        "plug-builtin-row",
        if connectors && on {
            Act::Set(SetAct::Open(Category::Connections))
        } else {
            act(PlugAct::Select(Some(format!("{BUILTIN}{id}"))))
        },
        name,
        ico(
            if connectors { "plug" } else { "puzzle" },
            17.0,
            1.75,
            k::GRAY_200,
        ),
        line,
        Some(switch),
    ));
    if let Some(note) = off_note(s, b) {
        col = col.child(note.ml(44.0).mb(10.0));
    }
    col
}

/// What going off hid, with Undo; or why the switch went back.
pub(super) fn off_note(s: &Screen<'_>, b: &Value) -> Option<El> {
    let pts = &s.st.points;
    let id = sv(b, "id").unwrap_or_default();
    if let Some((_, e)) = pts.switch_error.as_ref().filter(|(i, _)| i == id) {
        return Some(tone_line(Tone::Broken, e, 11.5));
    }
    if pts.off_note.as_deref() != Some(id) || b.get("on") != Some(&Value::Bool(false)) {
        return None;
    }
    let undo = s.acts.on(
        "ext-builtin-undo",
        Act::Point(PointAct::Builtin {
            id: id.to_owned(),
            on: true,
        }),
    );
    let fg = k::GRAY_300.mix(k::WHITE, s.ui.hover(undo));
    Some(
        div()
            .row()
            .items_start()
            .gap(8.0)
            .min_w0()
            .child(
                text(hidden_while_off(b), 11.5, k::GRAY_500)
                    .line_height(17.0)
                    .wrapping()
                    .min_w0(),
            )
            .child(
                div()
                    .id(undo)
                    .on(Action::Click)
                    .ring(R)
                    .shrink0()
                    .role(Role::Button, "Undo")
                    .border_bottom(1.0, fg)
                    .child(text("Undo", 11.5, fg).line_height(16.0).silent()),
            ),
    )
}

/// An installed or linked pack; vornd has no switch for these yet.
fn pack(s: &Screen<'_>, l: &Listing) -> El {
    let state = s.st.plugins.state(l);
    let status = describe(&state);
    let mut line = div().col().min_w0().child(second(&pack_line(l)));
    line = line.child_if(progress_bar(&status));
    if let (false, Some(detail)) = (status.tone == Tone::Settled, status.detail.as_deref()) {
        line = line.child(tone_line(status.tone, detail, 11.0).mt(4.0));
    }
    line = line.child_if(activity_line(s, &l.id).map(|e| e.mt(4.0)));
    let right = status.action.filter(|_| l.installable()).map(|_| {
        chip_button(
            s,
            "plug-installed-install",
            PlugAct::Install(l.key.clone()),
            Some("refresh-cw"),
            &status.label,
            k::GRAY_300,
            !status.busy,
        )
    });
    row(
        s,
        "plug-installed-row",
        act(PlugAct::Select(Some(l.key.clone()))),
        &l.name,
        glyph(Some(l), &l.id, 17.0, k::GRAY_200),
        line,
        right,
    )
}

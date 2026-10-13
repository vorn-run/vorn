//! The Installed tab (`InstalledPlugins`): every pack on disk, extensions
//! first, each with what it offers and what it is doing.

use super::{
    act, activity_line, badge, chip_button, details_hint, glyph, heading, offers, progress_bar,
    sheet_for, tone_line, well, R,
};
use crate::plugins::{describe, Kind, Listing, PackState, PlugAct, Tab, Tone};
use crate::screen::{white, Screen};
use crate::ui::theme::color as k;
use crate::ui::{div, text, Action, El, Role};

pub(super) fn view(s: &Screen<'_>) -> El {
    let p = &s.st.plugins;
    let groups: Vec<(Kind, Vec<&Listing>)> = [Kind::Extension, Kind::Connector]
        .into_iter()
        .map(|kind| {
            let items = p
                .listings
                .iter()
                .filter(|l| l.pack.is_some() && l.kind == kind)
                .collect::<Vec<_>>();
            (kind, items)
        })
        .filter(|(_, items)| !items.is_empty())
        .collect();
    if groups.is_empty() {
        let browse = s.acts.on("plug-installed", act(PlugAct::Tab(Tab::Browse)));
        let fg = k::GRAY_300.mix(k::WHITE, s.ui.hover(browse));
        let line = |t: &str| text(t, 14.0, k::GRAY_500).line_height(20.0).silent();
        return div()
            .row()
            .items_center()
            .wrap_children()
            .py(16.0)
            .child(line("Nothing is installed yet.\u{a0}"))
            .child(
                div()
                    .id(browse)
                    .on(Action::Click)
                    .ring(R)
                    .role(Role::Button, "Browse")
                    .border_bottom(1.0, fg)
                    .child(text("Browse", 14.0, fg).line_height(18.0).silent()),
            )
            .child(line("\u{a0}for a connector or an extension."));
    }
    let both = groups.len() > 1;
    let mut col = div().col();
    for (kind, items) in groups {
        if both {
            col = col.child(heading(kind.label()));
        }
        for l in items {
            col = col.child(row(s, l));
            if let Some(sheet) = sheet_for(s, &l.key) {
                col = col.child(sheet);
            }
        }
    }
    col
}

/// The muted line: the version on disk, then what it is doing.
fn facts(l: &Listing, state: &PackState) -> String {
    let mut parts = Vec::new();
    if let Some(v) = l.pack.as_ref().and_then(|p| p.get("version")?.as_str()) {
        parts.push(format!("v{v}"));
    }
    if let PackState::Installed {
        available: Some(next),
        ..
    } = state
    {
        parts.push(format!("v{next} available"));
    }
    if l.kind != Kind::Extension {
        parts.push(if l.connected > 0 {
            crate::plugins::count(l.connected, "connection")
        } else if l.implicit {
            "ready".to_owned()
        } else {
            "no connection yet".to_owned()
        });
    }
    parts.join(" · ")
}

/// `InstalledRow`.
fn row(s: &Screen<'_>, l: &Listing) -> El {
    let state = s.st.plugins.state(l);
    let status = describe(&state);
    let extension = l.kind == Kind::Extension;
    let open = s.acts.on(
        "plug-installed-row",
        act(PlugAct::Select(Some(l.key.clone()))),
    );
    let hot = s.ui.hover(open);
    let mut name = div().row().items_center().gap(6.0).min_w0().child(
        text(&l.name, 13.5, k::GRAY_200)
            .weight(500)
            .line_height(20.0)
            .truncate(),
    );
    if extension {
        name = name.child(badge("extension", None));
    }
    let said = offers(s, l).join(", ");
    let said = match (said.is_empty(), extension) {
        (false, _) => said,
        (true, true) => "Adds no surface yet".to_owned(),
        (true, false) => "Nothing declared yet".to_owned(),
    };
    let mut body = div()
        .col()
        .min_w0()
        .grow()
        .child(name)
        .child(
            text(said, 12.5, k::GRAY_500)
                .line_height(17.0)
                .mt(2.0)
                .wrapping(),
        )
        .child(
            text(facts(l, &state), 11.0, k::GRAY_600)
                .line_height(16.0)
                .mt(6.0)
                .wrapping(),
        )
        .child_if(progress_bar(&status));
    if let (false, Some(detail)) = (status.tone == Tone::Settled, status.detail.as_deref()) {
        body = body.child(tone_line(status.tone, detail, 11.0).mt(6.0));
    }
    body = body
        .child_if(activity_line(s, &l.id).map(|e| e.mt(6.0)))
        .child(details_hint(hot));
    let left = div()
        .id(open)
        .on(Action::Click)
        .ring(R)
        .row()
        .items_start()
        .gap(12.0)
        .grow()
        .min_w0()
        .role(Role::Button, format!("About {}", l.name))
        .child(
            well(
                32.0,
                6.0,
                white(0.05),
                glyph(Some(l), &l.id, 17.0, k::GRAY_200),
            )
            .mt(2.0),
        )
        .child(body);
    let mut buttons = div().row().items_center().self_center().gap(6.0).shrink0();
    if let Some(action) = status.action.filter(|_| l.installable()) {
        let icon = match action {
            crate::plugins::PackAction::Install => "download",
            _ => "refresh-cw",
        };
        buttons = buttons.child(chip_button(
            s,
            "plug-installed-install",
            PlugAct::Install(l.key.clone()),
            Some(icon),
            &status.label,
            k::GRAY_300,
            !status.busy,
        ));
    }
    if !extension && !l.implicit && l.connected == 0 {
        buttons = buttons.child(chip_button(
            s,
            "plug-installed-add",
            PlugAct::Add(l.key.clone()),
            Some("plus"),
            "Add connection",
            k::GRAY_300,
            true,
        ));
    }
    let busy = s.st.plugins.busy(&l.id);
    buttons = buttons.child(chip_button(
        s,
        "plug-installed-remove",
        PlugAct::Remove(l.id.clone()),
        Some(if busy { "loader-circle" } else { "trash-2" }),
        "Uninstall",
        k::GRAY_400,
        !busy,
    ));
    div()
        .row()
        .items_start()
        .gap(12.0)
        .py(12.0)
        .border_top(1.0, white(0.06))
        .child(left)
        .child(buttons)
}

//! The install dialog: where the extension comes from (a repository's
//! address or a folder), then the sheet every install reads first,
//! confirmed by typing the extension's id.

use super::{act, R};
use crate::act::Act;
use crate::plugins::{field_id, Field, Pending, PlugAct};
use crate::screen::dialogs::backdrop;
use crate::screen::{ico, white, Screen};
use crate::state::Dialog;
use crate::ui::theme::color as k;
use crate::ui::{div, field, text, Action, El, Role, TextStyle};

pub(in crate::screen) fn dialog(s: &Screen<'_>) -> Option<El> {
    if s.st.dialog != Some(Dialog::ExtInstall) {
        return None;
    }
    let p = &s.st.plugins;
    let (name, inner) = match (&p.pending, &p.source) {
        (Some(pending), _) if pending.link.is_some() => ("Link extension", sheet(s, pending)),
        (Some(pending), _) => ("Install extension", sheet(s, pending)),
        (None, Some(d)) => (d.origin.title(), source(s)),
        (None, None) => return None,
    };
    let panel = div()
        .col()
        .w(520.0_f32.min(s.size.0 - 32.0))
        .rounded(12.0)
        .border(1.0, white(0.08))
        .bg(k::SURFACE_OVERLAY)
        .clip()
        .role(Role::Dialog, name)
        .child(inner);
    let panel = s.acts.click(panel, "dialog", Act::Nothing);
    Some(backdrop(s, 0.5).center().child(panel))
}

/// A one-line text field bound to `f`.
fn input(s: &Screen<'_>, f: Field, value: &str, placeholder: &str, name: &str) -> El {
    let id = field_id(&f);
    s.acts.text(id, act(PlugAct::Text(f)), value);
    let focus = if s.ui.focused(id) { 1.0 } else { 0.0 };
    let style = TextStyle {
        line_height: 18.0,
        ..TextStyle::mono(12.5)
    };
    div()
        .row()
        .items_center()
        .px(10.0)
        .py(7.0)
        .rounded(R)
        .bg(white(0.04))
        .border(1.0, white(0.1).mix(white(0.25), focus))
        .frames(id)
        .child(
            field(id, name, placeholder, style, k::GRAY_100)
                .grow()
                .min_w0(),
        )
}

fn button(
    s: &Screen<'_>,
    place: &'static str,
    a: PlugAct,
    label: &str,
    primary: bool,
    enabled: bool,
) -> El {
    let b = div()
        .row()
        .items_center()
        .px(14.0)
        .py(7.0)
        .rounded(6.0)
        .shrink0()
        .role(Role::Button, label);
    let fg = if primary { k::WHITE } else { k::GRAY_400 };
    let face = text(label, 13.0, fg)
        .weight(if primary { 500 } else { 400 })
        .line_height(18.0)
        .silent();
    if !enabled {
        return b
            .bg(white(if primary { 0.06 } else { 0.03 }))
            .opacity(0.4)
            .disabled(true)
            .child(face);
    }
    let id = s.acts.on(place, act(a));
    let hot = s.ui.hover(id);
    let base = if primary { 0.12 } else { 0.04 };
    b.id(id)
        .on(Action::Click)
        .ring(6.0)
        .bg(white(base + 0.05 * hot))
        .child(face)
}

fn footer(children: Vec<El>) -> El {
    div()
        .row()
        .items_center()
        .justify_end()
        .gap(8.0)
        .px(20.0)
        .py(14.0)
        .border_top(1.0, white(0.06))
        .children(children)
}

fn title(t: &str) -> El {
    text(t, 15.0, k::GRAY_100)
        .weight(600)
        .line_height(22.0)
        .wrapping()
}

fn error(e: &str) -> El {
    div()
        .row()
        .items_start()
        .gap(6.0)
        .mt(10.0)
        .child(ico("circle-alert", 12.0, 2.0, k::DANGER).mt(2.0))
        .child(
            text(e, 12.0, k::DANGER)
                .line_height(17.0)
                .wrapping()
                .min_w0(),
        )
}

/// Where it comes from: a repository's address, or a folder to copy or link.
fn source(s: &Screen<'_>) -> El {
    let Some(g) = &s.st.plugins.source else {
        return div();
    };
    let o = g.origin;
    let mut body = div()
        .col()
        .px(20.0)
        .pt(18.0)
        .pb(16.0)
        .child(title(o.title()))
        .child(
            text(o.help(), 12.5, k::GRAY_500)
                .line_height(18.0)
                .mt(4.0)
                .mb(14.0)
                .wrapping(),
        )
        .child(input(s, Field::Source, &g.url, o.placeholder(), o.field()));
    if let Some(e) = &g.error {
        body = body.child(error(e));
    }
    let ready = !g.url.trim().is_empty() && !g.busy;
    let next = if g.busy {
        "Reading\u{2026}"
    } else {
        "Continue"
    };
    div().col().child(body).child(footer(vec![
        button(
            s,
            "plug-git-cancel",
            PlugAct::CancelInstall,
            "Cancel",
            false,
            true,
        ),
        button(s, "plug-git-next", PlugAct::ReadSource, next, true, ready),
    ]))
}

/// The sheet: who published it, then what it reaches, asks, runs and adds.
fn sheet(s: &Screen<'_>, pending: &Pending) -> El {
    let p = &s.st.plugins;
    let view = pending.sheet();
    let mut body = div()
        .col()
        .px(20.0)
        .pt(18.0)
        .pb(16.0)
        .child(title(&view.title));
    if !view.subtitle.is_empty() {
        body = body.child(
            text(view.subtitle.as_str(), 12.0, k::GRAY_500)
                .line_height(17.0)
                .mt(2.0)
                .wrapping(),
        );
    }
    if let Some(prev) = &view.replaces {
        body = body.child(
            text(format!("Replaces v{prev}."), 12.0, k::GRAY_500)
                .line_height(17.0)
                .mt(2.0),
        );
    }
    let mut rows = div()
        .col()
        .gap(8.0)
        .mt(14.0)
        .p(12.0)
        .rounded(8.0)
        .bg(white(0.03))
        .border(1.0, white(0.06));
    for r in &view.rows {
        let (tag, tone) = if r.enforced {
            ("enforced", k::GRAY_400)
        } else {
            ("declared", k::GRAY_600)
        };
        rows = rows.child(
            div()
                .row()
                .items_start()
                .gap(12.0)
                .child(
                    text(r.label, 12.0, k::GRAY_500)
                        .line_height(17.0)
                        .w(76.0)
                        .shrink0(),
                )
                .child(
                    text(r.text.as_str(), 12.0, k::GRAY_200)
                        .line_height(17.0)
                        .wrapping()
                        .grow()
                        .min_w0(),
                )
                .child(text(tag, 11.0, tone).line_height(17.0).shrink0()),
        );
    }
    body = body.child(rows).child(
        text(view.note.as_str(), 12.5, k::GRAY_300)
            .line_height(18.0)
            .mt(12.0)
            .wrapping(),
    );
    let id = pending.id();
    body = body
        .child(
            div()
                .row()
                .mt(14.0)
                .mb(6.0)
                .child(text("Type ", 12.0, k::GRAY_500).line_height(17.0))
                .child(text(id, 12.0, k::GRAY_200).mono().line_height(17.0))
                .child(
                    text(
                        if view.linking {
                            " to link"
                        } else {
                            " to install"
                        },
                        12.0,
                        k::GRAY_500,
                    )
                    .line_height(17.0),
                ),
        )
        .child(input(s, Field::Confirm, &pending.typed, id, "Extension id"));
    if let Some(e) = &p.report {
        body = body.child(error(e));
    }
    let label = if p.installing && view.linking {
        "Linking\u{2026}"
    } else if p.installing {
        "Installing\u{2026}"
    } else if view.linking {
        "Link"
    } else if view.replaces.is_some() {
        "Update"
    } else {
        "Install"
    };
    let ready = pending.confirmed() && !p.installing;
    // The footer stays put; the sheet scrolls above it on a short window.
    let body = div()
        .id(crate::ui::Id::new("plug-sheet-scroll"))
        .col()
        .max_h((s.size.1 - 140.0).max(160.0))
        .scroll_y()
        .child(body);
    div().col().child(body).child(footer(vec![
        button(
            s,
            "plug-sheet-cancel",
            PlugAct::CancelInstall,
            "Cancel",
            false,
            !p.installing,
        ),
        button(
            s,
            "plug-sheet-install",
            PlugAct::ConfirmInstall,
            label,
            true,
            ready,
        ),
    ]))
}

//! The Installed list, one row per extension with what needs you on top,
//! and the Updates list.

use serde_json::Value;

use super::{act, logo, maker, status, tag, tone_line, R};
use crate::act::Act;
use crate::plugins::list::{self, Item, Row, Status};
use crate::plugins::{hidden_while_off, PlugAct, Tab, Tone};
use crate::points::PointAct;
use crate::runs::s as sv;
use crate::screen::{ico, white, Screen};
use crate::ui::theme::color as k;
use crate::ui::{div, text, Action, El, Role};

/// The extension the default agent comes in.
fn default_agent<'a>(s: &Screen<'a>) -> Option<&'a str> {
    let key = crate::screen::settings::default_agent(s);
    s.store.agents.get(key).map(|a| a.extension.as_str())
}

pub(super) fn view(s: &Screen<'_>) -> El {
    let p = &s.st.plugins;
    let sections = list::sections(p, &s.st.points.builtins, &p.query, default_agent(s));
    if sections.is_empty() {
        return empty(s, !p.query.trim().is_empty());
    }
    let mut col = div().col();
    for (h, rows) in sections {
        col = col.child(heading(h.label(), rows.len()));
        for r in rows {
            col = col.child(row(s, &r));
        }
    }
    col
}

/// The installed packs with a newer release.
pub(super) fn updates(s: &Screen<'_>) -> El {
    let p = &s.st.plugins;
    let q = p.query.trim().to_lowercase();
    let items: Vec<_> = list::updates(p)
        .into_iter()
        .filter(|l| q.is_empty() || l.name.to_lowercase().contains(&q))
        .collect();
    if items.is_empty() {
        return div().py(24.0).child(
            text("Everything is up to date.", 13.0, k::INK_SECONDARY)
                .line_height(20.0)
                .silent(),
        );
    }
    let mut col = div().col().child(heading("Updates", items.len()));
    for l in items {
        col = col.child(row(
            s,
            &Row {
                item: Item::Listing(l),
                status: Some(Status::Update),
            },
        ));
    }
    col
}

/// An uppercase section heading and its count.
fn heading(label: &str, n: usize) -> El {
    div()
        .row()
        .items_center()
        .gap(6.0)
        .px(10.0)
        .pt(10.0)
        .pb(2.0)
        .role(Role::Heading, label)
        .child(
            text(label.to_uppercase(), 11.0, k::INK_FAINT)
                .tracking(0.06)
                .line_height(16.0)
                .silent(),
        )
        .child(
            text(n.to_string(), 11.0, k::INK_GHOST)
                .line_height(16.0)
                .silent(),
        )
}

/// No installed extension matches, or none is installed.
fn empty(s: &Screen<'_>, searching: bool) -> El {
    let browse = s.acts.on("plug-installed", act(PlugAct::Tab(Tab::Browse)));
    let fg = k::INK_SECONDARY.mix(k::INK, s.ui.hover(browse));
    let said = if searching {
        "Nothing installed matches.\u{a0}"
    } else {
        "Nothing installed yet.\u{a0}"
    };
    div()
        .row()
        .items_center()
        .py(16.0)
        .px(10.0)
        .child(
            text(said, 13.0, k::INK_SECONDARY)
                .line_height(20.0)
                .silent(),
        )
        .child(
            div()
                .id(browse)
                .on(Action::Click)
                .ring(R)
                .role(Role::Button, "Browse")
                .border_bottom(1.0, fg)
                .child(text("Browse", 13.0, fg).line_height(18.0).silent()),
        )
}

/// One extension: logo, name and tags over what it does and who made it,
/// its one status, and a chevron; the whole row opens its page.
pub(super) fn row(s: &Screen<'_>, r: &Row<'_>) -> El {
    let item = r.item;
    let key = item.key();
    let id = s
        .acts
        .on("plug-row", act(PlugAct::Select(Some(key.clone()))));
    let hot = s.ui.hover(id);
    let (listing, own) = match item {
        Item::Listing(l) => (Some(l), None),
        Item::Builtin(b) => (None, b.get("icon")),
    };
    let place = format!("plug-row-{key}");
    let mut title = div().row().items_center().gap(8.0).min_w0().child(
        text(item.name(), 14.0, k::INK)
            .line_height(20.0)
            .truncate()
            .shrink0(),
    );
    for t in item.tags() {
        title = title.child(tag(s, &place, t));
    }
    let desc = item.description();
    let mut line = div().row().items_center().min_w0();
    if !desc.is_empty() {
        line = line.child(
            text(format!("{desc} \u{b7}\u{a0}"), 12.0, k::INK_SECONDARY)
                .line_height(17.0)
                .truncate()
                .min_w0(),
        );
    }
    line = line.child(maker(&item.maker(), 12.0));
    div()
        .id(id)
        .on(Action::Click)
        .ring(R)
        .row()
        .items_center()
        .gap(12.0)
        .px(10.0)
        .py(9.0)
        .rounded(R)
        .bg(white(0.04 * hot))
        .role(Role::Button, format!("Open {}", item.name()))
        .child(logo(s, item.id(), item.name(), listing, own, 36.0))
        .child(
            div()
                .col()
                .gap(2.0)
                .grow()
                .min_w0()
                .child(title)
                .child(line),
        )
        .child_if(r.status.as_ref().map(|st| status(s, &place, st)))
        .child(ico(
            "chevron-right",
            14.0,
            2.0,
            k::INK_FAINT.mix(k::INK_SECONDARY, hot),
        ))
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
    let fg = k::INK_SECONDARY.mix(k::INK, s.ui.hover(undo));
    Some(
        div()
            .row()
            .items_start()
            .gap(8.0)
            .min_w0()
            .child(
                text(hidden_while_off(b), 11.5, k::INK_SECONDARY)
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

//! Browse: search, the kind pills and the sign-in picker, the listings by
//! category, and how fresh the catalog is.

use serde_json::Value;

use super::{act, glyph, heading, offers, progress_bar, tone_line, well, R};
use crate::plugins::{
    catalog_age, describe, field_id, kinds_id, pick_id, rungs, sections, Field, Kind, Listing,
    PackAction, PackState, Pick, PlugAct, Source, Tone,
};
use crate::runs::s as sv;
use crate::screen::{ico, white, Screen};
use crate::state::now_ms;
use crate::ui::theme::color as k;
use crate::ui::widgets::{picker, pills, Choice, PickerLook, Pill};
use crate::ui::{div, field, text, Action, El, Role, TextStyle};

pub(super) fn view(s: &Screen<'_>) -> El {
    let p = &s.st.plugins;
    let visible = p.filtered();
    let mut col = div().col().child(search(s)).child(filters(s));
    if let Some(report) = p.report.as_ref().filter(|_| p.pending.is_none()) {
        col = col.child(tone_line(Tone::Broken, report, 11.0).mt(8.0));
    }
    let empty = visible.is_empty();
    for (category, items) in sections(visible) {
        col = col.child(heading(&category));
        for l in items {
            col = col.child(row(s, l));
        }
    }
    if empty {
        let line = if p.filtering() {
            "Nothing matches that."
        } else {
            "Nothing to show. Check your connection and try again."
        };
        col = col.child(text(line, 14.0, k::GRAY_500).line_height(20.0).py(16.0));
    }
    col.child(footer(s))
}

/// The search box, its glyph inside on the left.
fn search(s: &Screen<'_>) -> El {
    let p = &s.st.plugins;
    let id = field_id(&Field::Query);
    s.acts
        .text(id, act(PlugAct::Text(Field::Query)), p.query.as_str());
    let focus = if s.ui.focused(id) { 1.0 } else { 0.0 };
    let style = TextStyle {
        line_height: 16.0,
        ..TextStyle::ui(12.0)
    };
    div()
        .row()
        .items_center()
        .gap(8.0)
        .mb(12.0)
        .pl(10.0)
        .pr(8.0)
        .py(6.0)
        .rounded(R)
        .bg(white(0.05))
        .border(1.0, white(0.1).mix(white(0.2), focus))
        .frames(id)
        .child(ico("search", 12.0, 2.0, k::GRAY_600))
        .child(
            field(
                id,
                "Search extensions",
                "Search extensions",
                style,
                k::GRAY_200,
            )
            .grow()
            .min_w0(),
        )
}

/// All, Connectors and Interface, and the sign-in picker when it offers a choice.
fn filters(s: &Screen<'_>) -> El {
    let p = &s.st.plugins;
    let items = [
        Pill::new("All", None),
        Pill::new(Kind::Connector.label(), None),
        Pill::new(Kind::Extension.label(), None),
    ];
    let on = match p.kind {
        None => 0,
        Some(Kind::Connector) => 1,
        Some(Kind::Extension) => 2,
    };
    let rs = rungs(&p.listings);
    let rung = (rs.len() > 1).then(|| {
        let labels: Vec<String> = rs.iter().map(|r| r.label().to_owned()).collect();
        let mut choices = vec![Choice {
            label: "Any sign-in",
            hint: None,
        }];
        choices.extend(labels.iter().map(|l| Choice {
            label: l,
            hint: None,
        }));
        let chosen = p
            .rung
            .and_then(|x| rs.iter().position(|y| *y == x))
            .map_or(0, |i| i + 1);
        picker(
            s.ui,
            pick_id(&Pick::Rung),
            "Filter by sign-in",
            PickerLook::Select,
            &choices,
            chosen,
        )
    });
    div()
        .row()
        .wrap_children()
        .items_center()
        .gap(8.0)
        .mb(4.0)
        .child(pills(s.ui, kinds_id(), "Kind", &items, on))
        .child_if(rung)
}

/// The catalog's age and "Check now".
fn footer(s: &Screen<'_>) -> El {
    let p = &s.st.plugins;
    let line = |t: &str| text(t, 11.0, k::GRAY_600).line_height(16.0).silent();
    let mut check = div().row().items_center().gap(4.0);
    let label = if p.refreshing {
        "Checking"
    } else {
        "Check now"
    };
    let fg = if p.refreshing {
        check = check.opacity(0.5).role(Role::Button, label);
        k::GRAY_500
    } else {
        let id = s.acts.on("plug-catalog", act(PlugAct::CatalogRefresh));
        check = check
            .id(id)
            .on(Action::Click)
            .ring(R)
            .role(Role::Button, label);
        k::GRAY_500.mix(k::GRAY_300, s.ui.hover(id))
    };
    check = check
        .child(ico("refresh-cw", 9.0, 2.0, fg))
        .child(text(label, 11.0, fg).line_height(16.0).silent());
    div()
        .row()
        .items_center()
        .gap(6.0)
        .pt(12.0)
        .border_top(1.0, white(0.06))
        .child(line(&catalog_age(p.fetched_at, now_ms())))
        .child(line("·"))
        .child(check)
}

/// The muted line: a summary, then what it offers and how it signs in or reaches out.
fn facts(s: &Screen<'_>, l: &Listing) -> String {
    let said = offers(s, l).join(", ");
    let mut parts: Vec<String> = (!said.is_empty()).then_some(said).into_iter().collect();
    if l.kind == Kind::Extension {
        let source = l.pack.as_ref().or(l.catalog.as_ref());
        let hosts = source
            .and_then(|m| m.pointer("/declares/network"))
            .and_then(Value::as_array)
            .map_or(0, Vec::len);
        parts.push(if hosts == 0 {
            "no network".to_owned()
        } else {
            crate::plugins::count(hosts, "host")
        });
    } else if let Some(r) = l.rung {
        parts.push(r.badge().to_owned());
    }
    parts.join(" \u{b7} ")
}

/// The row's one move: Install, Update, Installed, or Add connection.
fn chip(s: &Screen<'_>, l: &Listing) -> Option<El> {
    let state = s.st.plugins.state(l);
    let status = describe(&state);
    let (label, a, on) = if l.installable() {
        match (status.action, &state) {
            (Some(PackAction::Install), _) => ("Install\u{2026}".to_owned(), true, !status.busy),
            (Some(PackAction::Update), _) => ("Update\u{2026}".to_owned(), true, !status.busy),
            (Some(_), _) => (format!("{}\u{2026}", status.label), true, !status.busy),
            (None, PackState::Installed { .. }) => ("Installed".to_owned(), false, false),
            (None, _) if status.busy => (status.label.clone(), false, false),
            (None, _) => return None,
        }
    } else if !l.implicit && l.can_add(&state) && matches!(l.source, Source::Mcp | Source::Builtin)
    {
        let id = s
            .acts
            .on("plug-browse-add", act(PlugAct::Add(l.key.clone())));
        return Some(small_chip(s, Some(id), "Add connection"));
    } else {
        return None;
    };
    let id = (a && on).then(|| {
        s.acts
            .on("plug-browse-install", act(PlugAct::Install(l.key.clone())))
    });
    Some(small_chip(s, id, &label))
}

/// A bordered chip; without an id it is quiet and does nothing.
fn small_chip(s: &Screen<'_>, id: Option<crate::ui::Id>, label: &str) -> El {
    let b = div()
        .row()
        .items_center()
        .px(10.0)
        .py(4.0)
        .shrink0()
        .rounded(R)
        .border(1.0, white(0.1));
    match id {
        Some(id) => {
            let hot = s.ui.hover(id);
            let fg = k::GRAY_300.mix(k::WHITE, hot);
            b.id(id)
                .on(Action::Click)
                .ring(R)
                .bg(white(0.06 * hot))
                .role(Role::Button, label)
                .child(text(label, 11.5, fg).line_height(16.0).silent())
        }
        None => b
            .border(1.0, white(0.05))
            .role(Role::Label, label)
            .child(text(label, 11.5, k::GRAY_500).line_height(16.0).silent()),
    }
}

/// `ConnectorRow`: the name over one line, and its one move.
fn row(s: &Screen<'_>, l: &Listing) -> El {
    let status = describe(&s.st.plugins.state(l));
    let open = s
        .acts
        .on("plug-browse-row", act(PlugAct::Select(Some(l.key.clone()))));
    let hot = s.ui.hover(open);
    let mut name = div().row().items_center().gap(6.0).min_w0().child(
        text(&l.name, 13.5, k::GRAY_200.mix(k::WHITE, hot))
            .weight(500)
            .line_height(20.0)
            .truncate(),
    );
    if l.verified.is_some() {
        name = name.child(ico("badge-check", 12.0, 2.0, k::GRAY_500).role(Role::Image, "Verified"));
    }
    let tail = facts(s, l);
    let summary = l
        .description
        .as_deref()
        .or_else(|| l.catalog.as_ref().and_then(|c| sv(c, "summary")))
        .unwrap_or("");
    let line = div()
        .row()
        .min_w0()
        .mt(1.0)
        .child(
            text(summary, 12.0, k::GRAY_500)
                .line_height(17.0)
                .truncate()
                .min_w0(),
        )
        .child_if((!tail.is_empty()).then(|| {
            let lead = if summary.is_empty() { "" } else { " \u{b7} " };
            text(format!("{lead}{tail}"), 12.0, k::GRAY_500)
                .line_height(17.0)
                .shrink0()
        }));
    let mut body = div()
        .col()
        .min_w0()
        .grow()
        .child(name)
        .child(line)
        .child_if(progress_bar(&status));
    if let (Tone::Broken, Some(d)) = (status.tone, status.detail.as_deref()) {
        body = body.child(tone_line(Tone::Broken, d, 11.0).mt(4.0));
    }
    let left = div()
        .id(open)
        .on(Action::Click)
        .ring(R)
        .row()
        .items_center()
        .gap(12.0)
        .grow()
        .min_w0()
        .role(Role::Button, format!("About {}", l.name))
        .child(well(
            32.0,
            6.0,
            white(0.05 + 0.03 * hot),
            glyph(Some(l), &l.id, 17.0, k::GRAY_200),
        ))
        .child(body);
    div()
        .row()
        .items_center()
        .gap(12.0)
        .py(10.0)
        .border_top(1.0, white(0.06))
        .child(left)
        .child_if(chip(s, l))
}

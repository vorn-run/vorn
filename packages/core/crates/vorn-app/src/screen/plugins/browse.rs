//! The Browse tab (`ConnectorDirectory`): search, the three filters, the
//! listings by category, and how fresh the catalog is.

use super::{
    act, badge, chip_button, details_hint, glyph, heading, offers, pending_sheet, progress_bar,
    sheet_for, tone_line, well, R,
};
use crate::plugins::{
    catalog_age, categories, describe, field_id, kinds, pick_id, rungs, sections, Field, Kind,
    Listing, PackAction, PackState, Pick, PlugAct, Source, Tone,
};
use crate::screen::{ico, white, Screen};
use crate::state::now_ms;
use crate::ui::theme::color as k;
use crate::ui::widgets::{picker, Choice, PickerLook};
use crate::ui::{div, field, text, Action, El, Role, TextStyle};

pub(super) fn view(s: &Screen<'_>) -> El {
    let p = &s.st.plugins;
    let visible = p.filtered();
    let mut col = div().col().child(search(s)).child(filters(s));
    if let Some(report) = &p.report {
        col = col.child(tone_line(Tone::Broken, report, 11.0).mt(8.0));
    }
    if let Some(pending) = p
        .pending
        .as_ref()
        .filter(|pd| !visible.iter().any(|l| l.key == pd.key))
    {
        col = col.child(div().mt(8.0).child(pending_sheet(s, pending)));
    }
    let empty = visible.is_empty();
    let groups = sections(visible);
    let titled = groups.len() > 1;
    for (category, items) in groups {
        if titled {
            col = col.child(heading(&category));
        }
        for l in items {
            col = col.child(row(s, l));
            if let Some(sheet) = sheet_for(s, &l.key) {
                col = col.child(sheet);
            }
        }
    }
    if empty {
        let line = if p.filtering() {
            "Nothing matches that."
        } else {
            "No connectors available. Check your connection and try again."
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
        .mb(8.0)
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
                "Search connectors",
                "Search connectors",
                style,
                k::GRAY_200,
            )
            .grow()
            .min_w0(),
        )
}

/// A dropdown per facet, each only when it would offer a choice.
fn filters(s: &Screen<'_>) -> El {
    let p = &s.st.plugins;
    let dropdown = |pick: Pick, name: &str, all: &str, labels: Vec<String>, at: Option<usize>| {
        if labels.len() < 2 {
            return None;
        }
        let mut choices = vec![Choice {
            label: all,
            hint: None,
        }];
        choices.extend(labels.iter().map(|l| Choice {
            label: l,
            hint: None,
        }));
        let chosen = at.map_or(0, |i| i + 1);
        Some(picker(
            s.ui,
            pick_id(&pick),
            name,
            PickerLook::Select,
            &choices,
            chosen,
        ))
    };
    let cats = categories(&p.listings);
    let at = p
        .category
        .as_ref()
        .and_then(|c| cats.iter().position(|x| x == c));
    let category = dropdown(
        Pick::Category,
        "Filter by category",
        "All categories",
        cats,
        at,
    );
    let ks = kinds(&p.listings);
    let at = p.kind.and_then(|x| ks.iter().position(|y| *y == x));
    let labels = ks.iter().map(|k| k.label().to_owned()).collect();
    let kind = dropdown(Pick::Kind, "Filter by kind", "Everything", labels, at);
    let rs = rungs(&p.listings);
    let at = p.rung.and_then(|x| rs.iter().position(|y| *y == x));
    let labels = rs.iter().map(|r| r.label().to_owned()).collect();
    let rung = dropdown(Pick::Rung, "Filter by sign-in", "Any sign-in", labels, at);
    let id = s.acts.on("plug-file", act(PlugAct::InstallFile));
    let hot = s.ui.hover(id);
    let fg = k::GRAY_300.mix(k::WHITE, hot);
    let label = "Install from file";
    let file = div()
        .id(id)
        .on(Action::Click)
        .ring(R)
        .row()
        .items_center()
        .gap(4.0)
        .shrink0()
        .px(8.0)
        .py(6.0)
        .rounded(R)
        .border(1.0, white(0.1))
        .bg(white(0.06 * hot))
        .role(Role::Button, label)
        .child(ico("folder-open", 11.0, 2.0, fg))
        .child(text(label, 12.0, fg).line_height(16.0).silent());
    let file = crate::ui::widgets::tooltip(
        s.ui,
        file,
        id,
        "Install a connector from a git repository on this disk",
        None,
    );
    let row = div()
        .row()
        .wrap_children()
        .items_center()
        .gap(8.0)
        .child(file)
        .child_if(category)
        .child_if(kind)
        .child_if(rung);
    div().mb(4.0).child(row)
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

/// The muted line under a row: category, offers, version, use.
fn facts(s: &Screen<'_>, l: &Listing) -> String {
    let mut parts = vec![l.category.clone()];
    let said = offers(s, l);
    if !said.is_empty() {
        parts.push(said.join(", "));
    }
    if let Some(v) = l.version() {
        parts.push(format!("v{v}"));
    }
    if l.kind != Kind::Extension {
        if l.connected > 0 {
            parts.push("in use".into());
        } else if l.implicit {
            parts.push("ready".into());
        }
    }
    parts.join(" · ")
}

/// `ConnectorRow`.
fn row(s: &Screen<'_>, l: &Listing) -> El {
    let state = s.st.plugins.state(l);
    let status = describe(&state);
    let open = s
        .acts
        .on("plug-browse-row", act(PlugAct::Select(Some(l.key.clone()))));
    let hot = s.ui.hover(open);
    let mut name = div().row().items_center().gap(6.0).min_w0().child(
        text(&l.name, 13.5, k::GRAY_200)
            .weight(500)
            .line_height(20.0)
            .truncate(),
    );
    if l.kind == Kind::Extension {
        name = name.child(badge("extension", None));
    }
    if let Some(r) = l.rung {
        name = name.child(badge(r.badge(), None));
    }
    if l.verified.is_some() {
        name = name.child(badge("verified", Some("check")));
    }
    let mut body = div().col().min_w0().grow().child(name);
    if let Some(d) = l.description.as_deref().filter(|d| !d.is_empty()) {
        body = body.child(
            text(d, 12.5, k::GRAY_500)
                .line_height(17.0)
                .mt(2.0)
                .wrapping(),
        );
    }
    body = body
        .child(
            text(facts(s, l), 11.0, k::GRAY_600)
                .line_height(16.0)
                .mt(6.0)
                .wrapping(),
        )
        .child_if(progress_bar(&status));
    if state != PackState::Absent {
        let line = match (&state, status.detail.as_deref()) {
            (PackState::NotReleased, _) | (_, None) => status.label.as_str(),
            (_, Some(d)) => d,
        };
        body = body.child(tone_line(status.tone, line, 11.0).mt(6.0));
    }
    body = body.child(details_hint(hot));
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
        let icon = if action == PackAction::Install {
            "download"
        } else {
            "refresh-cw"
        };
        buttons = buttons.child(chip_button(
            s,
            "plug-browse-install",
            PlugAct::Install(l.key.clone()),
            Some(icon),
            &status.label,
            k::GRAY_300,
            !status.busy,
        ));
    }
    if !l.implicit && l.can_add(&state) {
        let label = if l.connected > 0 {
            "Add another"
        } else if l.source == Source::Mcp {
            "Add server"
        } else {
            "Add"
        };
        buttons = buttons.child(chip_button(
            s,
            "plug-browse-add",
            PlugAct::Add(l.key.clone()),
            Some("plus"),
            label,
            k::GRAY_300,
            true,
        ));
    }
    div()
        .row()
        .items_start()
        .gap(12.0)
        .py(12.0)
        .border_top(1.0, white(0.06))
        .child(left)
        .child(buttons)
}

//! Browse: the tag chips, two feature cards, a grid of tiles each with the
//! one verb its kind takes, and how fresh the catalog is.

use super::{act, logo, maker, tag, tone_line, R};
use crate::act::Act;
use crate::plugins::list::{self, Tag, Verb};
use crate::plugins::{catalog_age, tags_id, Listing, PlugAct, Tone};
use crate::screen::{ico, white, Screen};
use crate::settings::SetAct;
use crate::state::now_ms;
use crate::ui::theme::color as k;
use crate::ui::widgets::{pills, Pill};
use crate::ui::{div, text, tooltip, Action, El, Role};

/// Tiles across.
const COLUMNS: usize = 3;

pub(super) fn view(s: &Screen<'_>) -> El {
    let p = &s.st.plugins;
    let items = list::browse(p, &p.query, p.tag);
    let mut col = div().col();
    if let Some(report) = p.report.as_ref().filter(|_| p.pending.is_none()) {
        col = col.child(tone_line(Tone::Broken, report, 11.0).mb(8.0));
    }
    let narrowed = p.tag.is_some() || !p.query.trim().is_empty();
    if !narrowed {
        col = col.child(features(s));
    }
    if items.is_empty() {
        let line = if narrowed {
            "Nothing matches that."
        } else {
            "Nothing to show. Check your connection and try again."
        };
        col = col.child(
            text(line, 13.0, k::INK_SECONDARY)
                .line_height(20.0)
                .py(16.0),
        );
    } else {
        col = col.child(grid(s, &items));
    }
    col.child(footer(s))
}

/// All, then a chip for each tag some listing carries.
pub(super) fn tags(s: &Screen<'_>) -> El {
    let p = &s.st.plugins;
    let shown = list::browse_tags(p);
    let mut items = vec![Pill::new("All", None).hint("Shows everything you can add")];
    items.extend(
        shown
            .iter()
            .map(|t| Pill::new(t.plural(), None).hint(t.tip())),
    );
    let on = p
        .tag
        .and_then(|t| shown.iter().position(|x| *x == t))
        .map_or(0, |i| i + 1);
    pills(s.ui, tags_id(), "Show", &items, on)
}

/// Agents and Connections, the two kinds most people come for.
fn features(s: &Screen<'_>) -> El {
    let p = &s.st.plugins;
    let agents = list::browse(p, "", Some(Tag::Agent));
    let connections = list::browse(p, "", Some(Tag::Connection));
    let mut row = div().row().gap(12.0).mb(16.0);
    if let Some(first) = agents.first() {
        let names: Vec<&str> = agents.iter().map(|l| l.name.as_str()).collect();
        row = row.child(feature(
            s,
            Tag::Agent,
            "Bring the agents you already pay for",
            &names.join(", "),
            first,
        ));
    }
    if let Some(first) = connections.first() {
        row = row.child(feature(
            s,
            Tag::Connection,
            "Start work from where it lands",
            "Issues, posts, feeds and queries",
            first,
        ));
    }
    row
}

/// One feature card; it filters Browse to its tag.
fn feature(s: &Screen<'_>, t: Tag, title: &str, line: &str, first: &Listing) -> El {
    let id = s.acts.on(
        &format!("plug-feature-{}", t.label()),
        Act::Set(SetAct::Browse(Some(t), None)),
    );
    let hot = s.ui.hover(id);
    let card = div()
        .id(id)
        .on(Action::Click)
        .ring(R)
        .flex(1.0)
        .row()
        .items_center()
        .gap(16.0)
        .h(120.0)
        .px(18.0)
        .rounded(R)
        .bg(white(0.03 + 0.02 * hot))
        .border(1.0, white(0.07))
        .role(Role::Button, t.plural())
        .child(logo(s, &first.id, &first.name, Some(first), None, 56.0))
        .child(
            div()
                .col()
                .gap(2.0)
                .min_w0()
                .grow()
                .child(
                    text(t.plural().to_uppercase(), 11.0, k::INK_FAINT)
                        .tracking(0.06)
                        .line_height(16.0)
                        .silent(),
                )
                .child(
                    text(title, 16.0, k::INK)
                        .weight(600)
                        .line_height(22.0)
                        .wrapping()
                        .silent(),
                )
                .child(
                    text(line, 12.0, k::INK_SECONDARY)
                        .line_height(17.0)
                        .truncate()
                        .silent(),
                ),
        );
    tooltip(s.ui, card, id, t.tip(), None)
}

/// The tiles, three across.
fn grid(s: &Screen<'_>, items: &[&Listing]) -> El {
    let mut col = div().col().gap(12.0);
    for chunk in items.chunks(COLUMNS) {
        let mut row = div().row().gap(12.0);
        for l in chunk {
            row = row.child(tile(s, l));
        }
        for _ in chunk.len()..COLUMNS {
            row = row.child(div().flex(1.0));
        }
        col = col.child(row);
    }
    col
}

/// One listing: logo, name over maker and its verb; what it does; its tags.
fn tile(s: &Screen<'_>, l: &Listing) -> El {
    let open = s
        .acts
        .on("plug-tile", act(PlugAct::Select(Some(l.key.clone()))));
    let hot = s.ui.hover(open);
    let place = format!("plug-tile-{}", l.key);
    let item = list::Item::Listing(l);
    let mut tags = div().row().wrap_children().gap(4.0);
    for t in &l.tags {
        tags = tags.child(tag(s, &place, *t));
    }
    div()
        .id(open)
        .on(Action::Click)
        .ring(R)
        .flex(1.0)
        .col()
        .gap(10.0)
        .p(14.0)
        .rounded(R)
        .bg(white(0.02 + 0.02 * hot))
        .border(1.0, white(0.07))
        .role(Role::Button, format!("Open {}", l.name))
        .child(
            div()
                .row()
                .items_center()
                .gap(10.0)
                .child(logo(s, &l.id, &l.name, Some(l), None, 40.0))
                .child(
                    div()
                        .col()
                        .grow()
                        .min_w0()
                        .child(
                            text(&l.name, 14.0, k::INK)
                                .weight(500)
                                .line_height(20.0)
                                .truncate(),
                        )
                        .child(maker(&item.maker(), 11.5)),
                )
                .child_if(verb(s, l)),
        )
        .child(
            text(item.description(), 12.0, k::INK_SECONDARY)
                .line_height(17.0)
                .wrapping()
                .silent(),
        )
        .child(tags)
}

/// Install or Connect as the kind takes, or what it is doing.
fn verb(s: &Screen<'_>, l: &Listing) -> Option<El> {
    let (label, a, tip) = match Verb::of(&s.st.plugins, l) {
        Verb::Install => (
            "Install".to_owned(),
            Some(PlugAct::Install(l.key.clone())),
            "Shows what it adds and asks for, then installs it",
        ),
        Verb::Connect => (
            "Connect".to_owned(),
            Some(PlugAct::Add(l.key.clone())),
            "Signs in to an account it can watch",
        ),
        Verb::Installed => ("Installed".to_owned(), None, "Already on this computer"),
        Verb::Busy(what) => (what, None, "Working on it now"),
        Verb::None => return None,
    };
    let pill = div()
        .row()
        .items_center()
        .px(12.0)
        .py(4.0)
        .shrink0()
        .rounded(R);
    Some(match a {
        Some(a) => {
            let id = s.acts.on(&format!("plug-verb-{}", l.key), act(a));
            let hot = s.ui.hover(id);
            let b = pill
                .id(id)
                .on(Action::Click)
                .ring(R)
                .bg(white(0.08 + 0.06 * hot))
                .role(Role::Button, format!("{label} {}", l.name))
                .child(text(&label, 12.0, k::INK).line_height(16.0).silent());
            tooltip(s.ui, b, id, tip, None)
        }
        None => {
            let id = crate::ui::Id::new("plug-verb").child(&l.key);
            let b = pill
                .id(id)
                .role(Role::Label, label.as_str())
                .child(text(&label, 12.0, k::INK_FAINT).line_height(16.0).silent());
            tooltip(s.ui, b, id, tip, None)
        }
    })
}

/// The catalog's age and "Check now".
fn footer(s: &Screen<'_>) -> El {
    let p = &s.st.plugins;
    let line = |t: &str| text(t, 11.0, k::INK_FAINT).line_height(16.0).silent();
    let mut check = div().row().items_center().gap(4.0);
    let label = if p.refreshing {
        "Checking"
    } else {
        "Check now"
    };
    let fg = if p.refreshing {
        check = check.opacity(0.5).role(Role::Button, label);
        k::INK_FAINT.rgba()
    } else {
        let id = s.acts.on("plug-catalog", act(PlugAct::CatalogRefresh));
        check = check
            .id(id)
            .on(Action::Click)
            .ring(R)
            .role(Role::Button, label);
        k::INK_FAINT.mix(k::INK_SECONDARY, s.ui.hover(id))
    };
    check = check
        .child(ico("refresh-cw", 9.0, 2.0, fg))
        .child(text(label, 11.0, fg).line_height(16.0).silent());
    div()
        .row()
        .items_center()
        .gap(6.0)
        .pt(12.0)
        .mt(16.0)
        .border_top(1.0, white(0.06))
        .child(line(&catalog_age(p.fetched_at, now_ms())))
        .child(line("·"))
        .child(check)
}

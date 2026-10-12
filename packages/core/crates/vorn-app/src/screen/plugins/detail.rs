//! A listing's page (`ConnectorDetail`): what it starts and does, how it
//! signs in, what is on disk, and the moves it offers.

use serde_json::Value;

use super::{act, activity_line, chip_button, filled_button, glyph, sheet_for, tone_line, well, R};
use crate::plugins::{
    count, describe, describe_activation, details, not_asked, permission_rows, Kind, Listing,
    PackAction, PackState, PlugAct, Source, Tab,
};
use crate::runs::s as sv;
use crate::screen::{ico, white, Screen};
use crate::ui::theme::color as k;
use crate::ui::{div, text, Action, El, Role};

pub(super) fn view(s: &Screen<'_>, l: &Listing) -> El {
    let p = &s.st.plugins;
    let entry = l.catalog.as_ref();
    let state = p.state(l);
    let status = describe(&state);
    let d = details(l, &p.builtins);
    let back_label = if p.tab == Tab::Installed {
        "All plugins"
    } else {
        "All connectors"
    };
    let back = s.acts.on("plug-detail-back", act(PlugAct::Select(None)));
    let fg = k::GRAY_500.mix(k::GRAY_300, s.ui.hover(back));
    let mut col = div().col().child(
        div().row().child(
            div()
                .id(back)
                .on(Action::Click)
                .ring(R)
                .row()
                .items_center()
                .gap(6.0)
                .mb(16.0)
                .role(Role::Button, back_label)
                .child(ico("arrow-left", 12.0, 2.0, fg))
                .child(text(back_label, 12.0, fg).line_height(16.0).silent()),
        ),
    );

    let mut sub = entry
        .and_then(|e| sv(e, "packageName"))
        .map(str::to_owned)
        .unwrap_or_else(|| {
            if l.source == Source::Builtin {
                "Built in".to_owned()
            } else {
                l.id.clone()
            }
        });
    if let Some(v) = l.version() {
        sub.push_str(&format!(" · v{v}"));
    }
    if l.connected > 0 {
        sub.push_str(&format!(" · {}", count(l.connected, "connection")));
    }
    let mut who = div()
        .col()
        .min_w0()
        .child(
            text(&l.name, 16.0, k::GRAY_100)
                .weight(500)
                .line_height(22.0)
                .role(Role::Heading, &l.name),
        )
        .child(text(sub, 11.5, k::GRAY_600).line_height(16.0).wrapping());
    if let Some(v) = &l.verified {
        who = who.child(verified(v));
    }
    col = col.child(
        div()
            .row()
            .items_start()
            .gap(12.0)
            .child(well(
                40.0,
                6.0,
                white(0.05),
                glyph(Some(l), &l.id, 21.0, k::GRAY_200),
            ))
            .child(who),
    );
    if let Some(desc) = l.description.as_deref().filter(|d| !d.is_empty()) {
        col = col.child(
            text(desc, 12.5, k::GRAY_400)
                .line_height(20.0)
                .mt(16.0)
                .wrapping(),
        );
    }

    if !d.known {
        col = col.child(
            text(
                "This connector does not describe itself yet. Add it to see what it offers.",
                12.0,
                k::GRAY_500,
            )
            .line_height(16.0)
            .mt(20.0)
            .wrapping(),
        );
    } else if l.kind == Kind::Extension {
        col = extension_sections(col, l);
    } else {
        let items = |list: &[(String, Option<String>)], none: &str| -> Vec<El> {
            if list.is_empty() {
                return vec![quiet(none)];
            }
            list.iter()
                .map(|(t, d)| item(t, d.as_deref(), None))
                .collect()
        };
        col = col
            .child(section(
                "Starts a workflow when",
                items(
                    &d.triggers,
                    "Nothing — this one is only called from a step.",
                ),
            ))
            .child(section(
                "A step can ask it to",
                items(&d.actions, "Nothing — this one only watches."),
            ));
        if !d.settings.is_empty() {
            let mut line = div().row().wrap_children().gap(0.0);
            for (i, (name, required, _)) in d.settings.iter().enumerate() {
                let lead = if i > 0 { "   " } else { "" };
                let (label, fg) = if *required {
                    (format!("{lead}{name}"), k::GRAY_400)
                } else {
                    (format!("{lead}{name} (optional)"), k::GRAY_600)
                };
                line = line.child(text(label, 11.0, fg).mono().line_height(18.0));
            }
            col = col.child(section("You will need", vec![line]));
        }
    }

    let catalog_auth = entry.and_then(|e| sv(e, "auth")).filter(|a| !a.is_empty());
    if l.rung.is_some() || catalog_auth.is_some() || l.kind == Kind::Extension {
        let mut lines = Vec::new();
        if let Some(r) = l.rung {
            lines.push(para(r.detail(), k::GRAY_300));
        }
        if let Some(origins) = l
            .pack
            .as_ref()
            .and_then(|p| p.pointer("/auth/browser/origins"))
            .and_then(Value::as_array)
        {
            let hosts: Vec<&str> = origins
                .iter()
                .filter_map(Value::as_str)
                .map(origin_label)
                .collect();
            lines.push(para(
                &format!("Acts as you on {}", hosts.join(", ")),
                k::GRAY_500,
            ));
        }
        if let Some(a) = catalog_auth {
            lines.push(para(a, k::GRAY_500));
        }
        if l.kind == Kind::Extension && l.rung.is_none() && catalog_auth.is_none() {
            lines.push(para(
                "Nothing. It only reads what the session already has.",
                k::GRAY_300,
            ));
        }
        col = col.child(section("Signs in with", lines));
    }

    if let Some(pack) = &l.pack {
        let path = sv(pack, "path").unwrap_or("");
        let mut facts = vec![fact(
            "Installed",
            &format!("v{}", sv(pack, "version").unwrap_or("")),
            false,
        )];
        facts.push(fact("On disk", path, true));
        if l.kind != Kind::Extension {
            facts.push(fact("Runs via", &format!("node {path}/index.js"), true));
        }
        col = col.child(section("On this machine", facts));
    }

    if let (Some(detail), false) = (
        status.detail.as_deref(),
        matches!(state, PackState::Installed { .. }),
    ) {
        col = col.child(tone_line(status.tone, detail, 12.0).mt(16.0));
    }
    col = col.child_if(activity_line(s, &l.id).map(|e| e.mt(16.0)));
    col = col.child(actions(s, l, &state));
    col.child_if(sheet_for(s, &l.key).map(|e| e.mt(12.0)))
}

/// The row of moves under the page.
fn actions(s: &Screen<'_>, l: &Listing, state: &PackState) -> El {
    let status = describe(state);
    let busy = s.st.plugins.busy(&l.id);
    let mut row = div().row().items_center().wrap_children().gap(8.0).mt(24.0);
    if l.implicit {
        row = row.child(filled_button(
            s,
            "plug-detail",
            PlugAct::Use,
            "workflow",
            "Use in a workflow",
        ));
    } else if l.can_add(state) {
        row = row.child(filled_button(
            s,
            "plug-detail",
            PlugAct::Add(l.key.clone()),
            "plus",
            "Add a connection",
        ));
    }
    if let Some(action) = status.action.filter(|_| l.installable()) {
        let icon = if action == PackAction::Install {
            "download"
        } else {
            "refresh-cw"
        };
        row = row.child(chip_button(
            s,
            "plug-detail",
            PlugAct::Install(l.key.clone()),
            Some(icon),
            &status.label,
            k::GRAY_300,
            !status.busy,
        ));
    }
    if let PackState::Installed {
        previous: Some(_), ..
    } = state
    {
        row = row.child(chip_button(
            s,
            "plug-detail",
            PlugAct::Rollback(l.id.clone()),
            Some(if busy { "loader-circle" } else { "undo-2" }),
            "Roll back",
            k::GRAY_400,
            !busy,
        ));
    }
    if l.pack.is_some() {
        let label = if l.kind == Kind::Extension {
            "Uninstall"
        } else {
            "Remove"
        };
        row = row.child(chip_button(
            s,
            "plug-detail",
            PlugAct::Remove(l.id.clone()),
            Some(if busy { "loader-circle" } else { "trash-2" }),
            label,
            k::DANGER,
            !busy,
        ));
    }
    if let Some(pkg) = l.catalog.as_ref().and_then(|c| sv(c, "packageName")) {
        let id = s.acts.on(
            "plug-detail-npm",
            act(PlugAct::OpenUrl(format!(
                "https://www.npmjs.com/package/{pkg}"
            ))),
        );
        let fg = k::GRAY_500.mix(k::GRAY_300, s.ui.hover(id));
        row = row.child(
            div()
                .id(id)
                .on(Action::Click)
                .ring(R)
                .row()
                .items_center()
                .gap(4.0)
                .px(8.0)
                .py(6.0)
                .role(Role::Link, "View on npm")
                .child(ico("external-link", 11.0, 2.0, fg))
                .child(text("View on npm", 12.0, fg).line_height(16.0).silent()),
        );
    }
    if l.source == Source::Catalog && l.pack.is_none() && *state == PackState::Absent {
        row = row.child(div().grow()).child(
            text(
                "Nothing is on disk until you install it.",
                11.0,
                k::GRAY_600,
            )
            .line_height(16.0),
        );
    }
    row
}

/// What an extension adds, may touch, and where it shows.
fn extension_sections(col: El, l: &Listing) -> El {
    let mut surfaces = Vec::new();
    for (key, noun) in [
        ("panes", "pane"),
        ("footers", "footer"),
        ("linkHandlers", "link"),
    ] {
        for c in l
            .contributes
            .as_ref()
            .and_then(|c| c.get(key))
            .and_then(Value::as_array)
            .into_iter()
            .flatten()
        {
            let title = format!("{} · {noun}", sv(c, "title").unwrap_or(""));
            let detail = sv(c, "description").map(str::to_owned).or_else(|| {
                sv(c, "pattern")
                    .filter(|_| noun == "link")
                    .map(|p| format!("matches {p}"))
            });
            let when = c
                .get("when")
                .filter(|w| !w.is_null())
                .map(|w| format!("Shows on {}", describe_activation(Some(w)).join(" and ")));
            surfaces.push(item(&title, detail.as_deref(), when.as_deref()));
        }
    }
    if surfaces.is_empty() {
        surfaces.push(quiet("Nothing — this one adds no surface yet."));
    }
    let stated = l.pack.is_some()
        || l.catalog
            .as_ref()
            .is_some_and(|c| c.get("permissions").is_some());
    let mut asks = Vec::new();
    if !stated {
        asks.push(quiet("The catalog does not say what it asks for yet."));
    } else {
        let rows = permission_rows(&l.permissions);
        if rows.is_empty() {
            asks.push(quiet("Nothing. It draws what it is given."));
        }
        for (bucket, items) in rows {
            asks.push(
                div()
                    .row()
                    .wrap_children()
                    .child(text(format!("{bucket} "), 12.5, k::GRAY_500).line_height(18.0))
                    .child(
                        text(items.join(", "), 12.5, k::GRAY_300)
                            .line_height(18.0)
                            .wrapping(),
                    ),
            );
        }
        let rest = not_asked(&l.permissions);
        if !rest.is_empty() {
            asks.push(quiet(&format!("Not asked: {}.", rest.join(", "))));
        }
    }
    col.child(section("Contributes", surfaces))
        .child(section("Asks to", asks))
        .child(section(
            "Shows when",
            vec![para(
                &describe_activation(l.activates.as_ref()).join(" and "),
                k::GRAY_300,
            )],
        ))
}

/// `VerifiedStrip`: the named checks and when they ran.
fn verified(v: &Value) -> El {
    let mut row = div().row().wrap_children().items_center().gap(12.0).mt(6.0);
    for check in v
        .get("checks")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .filter_map(Value::as_str)
    {
        row = row.child(
            div()
                .row()
                .items_center()
                .gap(4.0)
                .child(ico("check", 10.0, 2.0, k::GRAY_500))
                .child(text(check, 11.0, k::GRAY_500).line_height(16.0)),
        );
    }
    let day = format_day(sv(v, "checkedAt").unwrap_or(""));
    let version = sv(v, "version").unwrap_or("");
    row.child(
        text(
            format!("checked {day} against v{version}"),
            11.0,
            k::GRAY_600,
        )
        .line_height(16.0),
    )
}

/// `formatDay`: "Mar 4, 2025", or the stamp as given.
fn format_day(iso: &str) -> String {
    const MONTHS: [&str; 12] = [
        "Jan", "Feb", "Mar", "Apr", "May", "Jun", "Jul", "Aug", "Sep", "Oct", "Nov", "Dec",
    ];
    let part = |r: std::ops::Range<usize>| iso.get(r).and_then(|p| p.parse::<usize>().ok());
    match (part(0..4), part(5..7), part(8..10)) {
        (Some(y), Some(m @ 1..=12), Some(d)) => format!("{} {d}, {y}", MONTHS[m - 1]),
        _ => iso.to_owned(),
    }
}

/// `originLabel`: an origin's host.
pub(super) fn origin_label(origin: &str) -> &str {
    let host = origin.split_once("://").map_or(origin, |(_, rest)| rest);
    host.split('/').next().unwrap_or(host)
}

fn section(label: &str, children: Vec<El>) -> El {
    div()
        .col()
        .mt(20.0)
        .child(
            text(label.to_uppercase(), 10.0, k::GRAY_600)
                .tracking(0.05)
                .line_height(14.0)
                .role(Role::Heading, label),
        )
        .child(div().col().gap(8.0).mt(8.0).children(children))
}

fn item(title: &str, detail: Option<&str>, when: Option<&str>) -> El {
    let line = |t: &str, fg| text(t, 12.5, fg).line_height(18.0).wrapping();
    div()
        .col()
        .child(line(title, k::GRAY_300))
        .child_if(detail.map(|d| line(d, k::GRAY_600).mt(2.0)))
        .child_if(when.map(|w| line(w, k::GRAY_600).mt(2.0)))
}

fn para(t: &str, fg: crate::ui::Rgba) -> El {
    text(t, 12.5, fg).line_height(18.0).wrapping()
}

fn quiet(t: &str) -> El {
    text(t, 12.0, k::GRAY_600).line_height(16.0).wrapping()
}

/// `Fact`: a 72 px term beside its value.
fn fact(term: &str, value: &str, mono: bool) -> El {
    let mut v = text(value, if mono { 11.0 } else { 12.0 }, k::GRAY_300)
        .line_height(19.0)
        .wrapping()
        .grow()
        .min_w0();
    if mono {
        v = v.mono();
    }
    div()
        .row()
        .items_start()
        .gap(12.0)
        .child(
            text(term, 12.0, k::GRAY_600)
                .line_height(19.0)
                .w(72.0)
                .shrink0(),
        )
        .child(v)
}

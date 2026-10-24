//! An extension's page: what it adds and where, what it may use, what it
//! runs, and its moves. A connector's page says what it starts and does and
//! how it signs in.

use serde_json::Value;

use super::{act, activity_line, chip_button, filled_button, glyph, tone_line, well, R};
use crate::act::Act;
use crate::plugins::{
    count, describe, details, place_groups, places, runs_line, uses, Kind, Listing, PackAction,
    PackState, PlugAct, Source,
};
use crate::points::PointAct;
use crate::runs::s as sv;
use crate::screen::menus::Item;
use crate::screen::{ico, white, Screen};
use crate::state::MenuKind;
use crate::ui::theme::color as k;
use crate::ui::{div, text, Action, El, Role};

/// A built-in's key in `Plugins::selected`, before its id.
pub(super) const BUILTIN: &str = "builtin:";

/// "← Extensions", back to the list.
fn back(s: &Screen<'_>) -> El {
    let back = s.acts.on("plug-detail-back", act(PlugAct::Select(None)));
    let fg = k::GRAY_500.mix(k::GRAY_300, s.ui.hover(back));
    div().row().child(
        div()
            .id(back)
            .on(Action::Click)
            .ring(R)
            .row()
            .items_center()
            .gap(6.0)
            .mb(16.0)
            .role(Role::Button, "Extensions")
            .child(ico("arrow-left", 12.0, 2.0, fg))
            .child(text("Extensions", 12.0, fg).line_height(18.0).silent()),
    )
}

/// What an extension's page reads, from a pack or a built-in.
struct Page<'a> {
    name: &'a str,
    mark: El,
    sub: String,
    description: Option<&'a str>,
    contributes: Option<&'a Value>,
    declared: Vec<String>,
    hosts: Vec<String>,
    service: Option<&'a Value>,
    /// The extension's own on/off, drawn as the first row of its settings.
    switch: Option<Switch<'a>>,
    /// The extension's id and the preferences it declares, each with its value.
    preferences: (&'a str, Option<&'a Value>),
}

/// An extension's switch: whether it is on, what flipping it does, and the
/// tooltip saying what it does.
struct Switch<'a> {
    mark: El,
    on: bool,
    act: Act,
    busy: bool,
    tip: Option<&'a str>,
}

fn strings(v: Option<&Value>) -> Vec<String> {
    v.and_then(Value::as_array)
        .into_iter()
        .flatten()
        .filter_map(Value::as_str)
        .map(str::to_owned)
        .collect()
}

fn extension_page(s: &Screen<'_>, page: Page<'_>) -> El {
    let who = div()
        .col()
        .grow()
        .min_w0()
        .child(
            text(page.name, 16.0, k::GRAY_100)
                .weight(500)
                .line_height(24.0)
                .role(Role::Heading, page.name),
        )
        .child(
            text(page.sub, 11.5, k::GRAY_600)
                .line_height(17.25)
                .wrapping(),
        );
    let mut col = div().col().child(back(s)).child(
        div()
            .row()
            .items_start()
            .gap(12.0)
            .child(well(40.0, 6.0, white(0.05), page.mark))
            .child(who),
    );
    if let Some(desc) = page.description.filter(|d| !d.is_empty()) {
        col = col.child(
            text(desc, 12.5, k::GRAY_400)
                .line_height(20.0)
                .mt(16.0)
                .wrapping(),
        );
    }
    let (id, prefs) = page.preferences;
    let switch = page.switch.map(|w| {
        let place = format!("ext-switch-{id}");
        let toggle = crate::screen::settings::toggle(s, &place, w.on, w.act, w.busy, page.name);
        crate::screen::settings::card_row(s, Some(w.mark), page.name, w.tip, toggle.shrink0())
    });
    let rows: Vec<El> = switch
        .into_iter()
        .chain(
            prefs
                .and_then(Value::as_array)
                .into_iter()
                .flatten()
                .filter(|p| !shown_elsewhere(p))
                .filter_map(|p| preference(s, id, p)),
        )
        .collect();
    if !rows.is_empty() {
        col = col.child(section(
            "Settings",
            vec![crate::screen::settings::card(rows)],
        ));
    }
    let n = places(page.contributes);
    let groups = place_groups(page.contributes);
    let adds: Vec<El> = if groups.is_empty() {
        vec![quiet("Nothing yet.")]
    } else {
        groups
            .into_iter()
            .map(|(group, items)| term(group, &items.join(", ")))
            .collect()
    };
    col = col.child(section(&format!("Adds \u{b7} {}", count(n, "place")), adds));
    let may: Vec<El> = uses(&page.declared, &page.hosts)
        .into_iter()
        .map(|u| {
            let by = if u.enforced {
                "enforced by Vorn"
            } else {
                "declared"
            };
            div()
                .row()
                .wrap_children()
                .child(text(format!("{} \u{2014} ", u.what), 12.5, k::GRAY_300).line_height(18.75))
                .child(
                    text(format!("{} \u{2014} ", u.how), 12.5, k::GRAY_400)
                        .line_height(18.75)
                        .wrapping(),
                )
                .child(text(by, 12.5, k::GRAY_600).line_height(18.75))
        })
        .collect();
    col = col.child(section("May use", may));
    if let Some(runs) = runs_line(page.service) {
        col = col.child(
            div()
                .row()
                .wrap_children()
                .mt(20.0)
                .child(text("Runs: ", 12.5, k::GRAY_500).line_height(18.75))
                .child(
                    text(runs, 12.0, k::GRAY_300)
                        .mono()
                        .line_height(18.75)
                        .wrapping(),
                ),
        );
    }
    col
}

/// A built-in's page, with its switch.
pub(super) fn builtin(s: &Screen<'_>, b: &Value) -> El {
    let id = sv(b, "id").unwrap_or_default();
    let name = sv(b, "name").unwrap_or(id);
    let on = b.get("on").and_then(Value::as_bool) == Some(true);
    let mut sub = id.to_owned();
    if let Some(v) = sv(b, "version") {
        sub.push_str(&format!(" \u{b7} v{v}"));
    }
    sub.push_str(" \u{b7} built in");
    let act = Act::Point(PointAct::Builtin {
        id: id.to_owned(),
        on: !on,
    });
    let busy = s.st.points.switching.is_some();
    let col = extension_page(
        s,
        Page {
            name,
            mark: ico("puzzle", 21.0, 1.75, k::GRAY_200),
            sub,
            description: sv(b, "description"),
            contributes: b.get("contributes"),
            declared: strings(b.pointer("/declares/uses")),
            hosts: strings(b.pointer("/declares/network")),
            service: b.get("service"),
            switch: Some(Switch {
                mark: ico("puzzle", 15.0, 1.75, k::GRAY_400),
                on,
                act,
                busy,
                tip: sv(b, "description"),
            }),
            preferences: (id, b.get("preferences")),
        },
    );
    col.child_if(super::installed::off_note(s, b).map(|e| e.mt(16.0)))
}

pub(super) fn view(s: &Screen<'_>, l: &Listing) -> El {
    if l.kind == Kind::Extension {
        return extension(s, l);
    }
    let p = &s.st.plugins;
    let entry = l.catalog.as_ref();
    let state = p.state(l);
    let status = describe(&state);
    let d = details(l, &p.builtins);
    let mut col = div().col().child(back(s));

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
                .line_height(24.0)
                .role(Role::Heading, &l.name),
        )
        .child(text(sub, 11.5, k::GRAY_600).line_height(17.25).wrapping());
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
            // One word per piece, so the row wraps between words as the paragraph does.
            let mut line = div().row().wrap_children().gap(0.0);
            let last = d.settings.len() - 1;
            for (i, (name, required, _)) in d.settings.iter().enumerate() {
                let (label, fg) = if *required {
                    (name.clone(), k::GRAY_400)
                } else {
                    (format!("{name} (optional)"), k::GRAY_600)
                };
                let words: Vec<&str> = label.split(' ').collect();
                for (j, w) in words.iter().enumerate() {
                    // Runs of spaces collapse to one, as HTML does.
                    let gap = if j + 1 < words.len() || i < last {
                        " "
                    } else {
                        ""
                    };
                    line = line.child(
                        text(format!("{w}{gap}"), 11.0, fg)
                            .mono()
                            .line_height(18.0)
                            .silent(),
                    );
                }
            }
            col = col.child(section("You will need", vec![line]));
        }
    }

    let catalog_auth = entry.and_then(|e| sv(e, "auth")).filter(|a| !a.is_empty());
    if l.rung.is_some() || catalog_auth.is_some() {
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
        facts.push(fact("Runs via", &format!("node {path}/index.js"), true));
        col = col.child(section("On this machine", facts));
    }

    if let (Some(detail), false) = (
        status.detail.as_deref(),
        matches!(state, PackState::Installed { .. }),
    ) {
        col = col.child(tone_line(status.tone, detail, 12.0).mt(16.0));
    }
    col = col.child_if(activity_line(s, &l.id).map(|e| e.mt(16.0)));
    col.child(actions(s, l, &state))
}

/// An extension from the catalog, Git or a folder.
fn extension(s: &Screen<'_>, l: &Listing) -> El {
    let p = &s.st.plugins;
    let state = p.state(l);
    let pack = l.pack.as_ref();
    let mut sub = l.id.clone();
    if let Some(v) = l.version() {
        sub.push_str(&format!(" \u{b7} v{v}"));
    }
    if let Some(pack) = pack {
        let linked = pack.get("linked") == Some(&Value::Bool(true));
        let source = pack.get("source");
        let from = if linked {
            Some("linked, edits are live".to_owned())
        } else if let Some(g) = source.and_then(|v| sv(v, "git")) {
            Some(format!("from {}", crate::plugins::short_repo(g)))
        } else {
            source
                .and_then(|v| sv(v, "path"))
                .map(|path| format!("copied from {}", crate::plugins::tilde(path)))
        };
        if let Some(f) = from {
            sub.push_str(&format!(" \u{b7} {f}"));
        }
    }
    let manifest = pack.or(l.catalog.as_ref());
    let linked = pack.is_some_and(|p| p.get("linked") == Some(&Value::Bool(true)));
    // Off uninstalls and on installs, as a built-in's switch turns it off and on.
    let switch = match pack {
        Some(_) if !linked => Some(PlugAct::Remove(l.id.clone())),
        None if l.installable() => Some(PlugAct::Install(l.key.clone())),
        _ => None,
    }
    .map(|plug| Switch {
        mark: glyph(Some(l), &l.id, 15.0, k::GRAY_400),
        on: pack.is_some(),
        act: act(plug),
        busy: s.st.plugins.busy(&l.id) || describe(&state).busy,
        tip: manifest
            .and_then(|m| sv(m, "description"))
            .or(l.description.as_deref())
            .filter(|d| !d.is_empty()),
    });
    let col = extension_page(
        s,
        Page {
            name: &l.name,
            mark: glyph(Some(l), &l.id, 21.0, k::GRAY_200),
            sub,
            description: l.description.as_deref(),
            contributes: l.contributes.as_ref(),
            declared: l.permissions.clone(),
            hosts: strings(manifest.and_then(|m| m.pointer("/declares/network"))),
            service: manifest.and_then(|m| m.get("service")),
            switch,
            preferences: (&l.id, pack.and_then(|p| p.get("preferences"))),
        },
    );
    let status = describe(&state);
    let col = match (status.detail.as_deref(), &state) {
        (Some(detail), s2) if !matches!(s2, PackState::Installed { .. }) => {
            col.child(tone_line(status.tone, detail, 12.0).mt(16.0))
        }
        _ => col,
    };
    col.child_if(activity_line(s, &l.id).map(|e| e.mt(16.0)))
        .child(extension_actions(s, l, &state))
}

/// Sandbox, Show folder and Unlink or Uninstall; Install while it is not on disk.
fn extension_actions(s: &Screen<'_>, l: &Listing, state: &PackState) -> El {
    let status = describe(state);
    let busy = s.st.plugins.busy(&l.id);
    let mut row = div().row().items_center().wrap_children().gap(8.0).mt(24.0);
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
            &format!("{}\u{2026}", status.label),
            k::GRAY_300,
            !status.busy,
        ));
    }
    let Some(pack) = l.pack.as_ref() else {
        return row;
    };
    if pack.get("sandboxAvailable") == Some(&Value::Bool(true)) {
        let on = pack.get("sandboxed") == Some(&Value::Bool(true));
        row = row.child(chip_button(
            s,
            "plug-detail-sandbox",
            PlugAct::Sandbox(l.id.clone(), !on),
            Some("shield"),
            if on { "Sandbox: on" } else { "Sandbox: off" },
            k::GRAY_300,
            true,
        ));
    }
    if let Some(path) = sv(pack, "path").filter(|p| !p.is_empty()) {
        row = row.child(chip_button(
            s,
            "plug-detail-folder",
            PlugAct::ShowFolder(path.to_owned()),
            Some("folder-open"),
            "Show folder",
            k::GRAY_300,
            true,
        ));
    }
    let linked = pack.get("linked") == Some(&Value::Bool(true));
    row.child(chip_button(
        s,
        "plug-detail",
        PlugAct::Remove(l.id.clone()),
        Some(if busy { "loader-circle" } else { "trash-2" }),
        if linked { "Unlink" } else { "Uninstall" },
        k::DANGER,
        !busy,
    ))
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
            "Add connection",
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
        row = row.child(chip_button(
            s,
            "plug-detail",
            PlugAct::Remove(l.id.clone()),
            Some(if busy { "loader-circle" } else { "trash-2" }),
            "Uninstall",
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

/// The named checks and when they ran.
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

/// "Mar 4, 2025", or the stamp as given.
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

/// An origin's host.
pub(super) fn origin_label(origin: &str) -> &str {
    let host = origin.split_once("://").map_or(origin, |(_, rest)| rest);
    host.split('/').next().unwrap_or(host)
}

/// The most options a choice shows side by side; more open from a chip.
const MAX_SEGMENTS: usize = 3;

/// A choice preference's options: (id, title).
fn options(p: &Value) -> impl Iterator<Item = (&str, &str)> {
    p.get("options")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .filter_map(|o| Some((sv(o, "id")?, sv(o, "title").unwrap_or_default())))
}

/// The menu of extension `ext`'s choice preference `key`.
pub(in crate::screen) fn choice_items(s: &Screen<'_>, ext: &str, key: &str) -> Vec<Item> {
    let lists = [&s.st.points.builtins, &s.st.plugins.packs];
    let Some(p) = lists
        .iter()
        .flat_map(|l| l.iter())
        .filter(|e| sv(e, "id") == Some(ext))
        .filter_map(|e| e.get("preferences").and_then(Value::as_array))
        .flatten()
        .find(|p| sv(p, "id") == Some(key))
    else {
        return Vec::new();
    };
    let chosen = p.get("value").and_then(Value::as_str);
    options(p)
        .map(|(id, label)| {
            let act = Act::Point(PointAct::Preference {
                id: ext.to_owned(),
                key: key.to_owned(),
                value: Value::from(id).to_string(),
            });
            Item::pick(label, chosen == Some(id), act)
        })
        .collect()
}

/// Whether `p` is the switch for all of an extension's notifications, which
/// Settings › Notifications shows instead of its page.
fn shown_elsewhere(p: &Value) -> bool {
    sv(p, "notifications") == Some("all")
}

/// One preference: a card row with its title beside a switch, a segmented
/// choice, or a chip that opens a long one.
fn preference(s: &Screen<'_>, ext: &str, p: &Value) -> Option<El> {
    let key = sv(p, "id")?;
    let title = sv(p, "title").unwrap_or(key);
    let value = p.get("value");
    let act = |v: Value| {
        Act::Point(PointAct::Preference {
            id: ext.to_owned(),
            key: key.to_owned(),
            value: v.to_string(),
        })
    };
    let place = format!("ext-pref-{ext}-{key}");
    let control = match sv(p, "type") {
        Some("bool") => {
            let on = value.and_then(Value::as_bool) == Some(true);
            let id = s.acts.on(&place, act(Value::Bool(!on)));
            crate::ui::widgets::toggle(s.ui, id, title, on, false).shrink0()
        }
        Some("choice") if options(p).count() > MAX_SEGMENTS => {
            let kind = MenuKind::ExtChoice(ext.to_owned(), key.to_owned());
            let open = s.st.menu.as_ref().is_some_and(|m| m.kind == kind);
            let chosen = value.and_then(Value::as_str);
            let label = options(p)
                .find(|(id, _)| Some(*id) == chosen)
                .map_or("", |(_, l)| l);
            let id = s.acts.on(&place, Act::Open(kind));
            let lit = s.ui.hover(id).max(if open { 1.0 } else { 0.0 });
            div()
                .id(id)
                .on(Action::Click)
                .row()
                .items_center()
                .gap(8.0)
                .h(28.0)
                .pl(10.0)
                .pr(8.0)
                .shrink0()
                .rounded(4.0)
                .bg(white(0.04).mix(white(0.08), lit))
                .border(1.0, white(0.08))
                .role(Role::ComboBox, title)
                .expanded(open)
                .ring(R)
                .child(text(label, 13.0, k::GRAY_200).line_height(18.0).silent())
                .child(ico("chevron-down", 12.0, 2.0, k::GRAY_500))
        }
        Some("choice") => {
            let chosen = value.and_then(Value::as_str);
            let opts = options(p)
                .map(|(id, label)| crate::screen::settings::Seg {
                    label,
                    icon: None,
                    act: act(Value::from(id)),
                    on: chosen == Some(id),
                    disabled: false,
                    tip: None,
                })
                .collect();
            crate::screen::settings::segmented(s, &place, opts, 10.0)
        }
        _ => return None,
    };
    // As on Settings' own pages: the description is the row's tooltip, not a line under it.
    let tip = sv(p, "description").filter(|d| !d.is_empty());
    Some(crate::screen::settings::card_row(
        s, None, title, tip, control,
    ))
}

fn section(label: &str, children: Vec<El>) -> El {
    div()
        .col()
        .mt(20.0)
        .child(
            text(label.to_uppercase(), 10.0, k::GRAY_600)
                .tracking(0.05)
                .line_height(15.0)
                .role(Role::Heading, label),
        )
        .child(div().col().gap(8.0).mt(8.0).children(children))
}

fn item(title: &str, detail: Option<&str>, when: Option<&str>) -> El {
    let line = |t: &str, fg| text(t, 12.5, fg).line_height(18.75).wrapping();
    div()
        .col()
        .child(line(title, k::GRAY_300))
        .child_if(detail.map(|d| line(d, k::GRAY_600).mt(2.0)))
        .child_if(when.map(|w| line(w, k::GRAY_600).mt(2.0)))
}

fn para(t: &str, fg: impl Into<crate::ui::Rgba>) -> El {
    let fg = fg.into();
    text(t, 12.5, fg).line_height(18.75).wrapping()
}

/// A group's term beside what it holds.
fn term(group: &str, items: &str) -> El {
    div()
        .row()
        .items_start()
        .gap(12.0)
        .child(
            text(group, 12.5, k::GRAY_500)
                .line_height(18.75)
                .w(88.0)
                .shrink0(),
        )
        .child(
            text(items, 12.5, k::GRAY_300)
                .line_height(18.75)
                .wrapping()
                .grow()
                .min_w0(),
        )
}

fn quiet(t: &str) -> El {
    text(t, 12.0, k::GRAY_600).line_height(18.0).wrapping()
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

#[cfg(test)]
mod tests {
    use serde_json::json;

    #[test]
    fn only_the_switch_for_all_notifications_leaves_the_page() {
        let all = json!({ "id": "notify:all", "notifications": "all" });
        let kind = json!({ "id": "notify:finished", "notifications": "kind" });
        let plain = json!({ "id": "top_bar" });
        assert!(super::shown_elsewhere(&all));
        assert!(!super::shown_elsewhere(&kind));
        assert!(!super::shown_elsewhere(&plain));
    }
}

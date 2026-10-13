//! The Connections tab (`ConnectionGroups` over `ConnectionRow`): each
//! connection under its connector, with what polls it and what it last did.

use serde_json::Value;

use super::{act, activity_line, chip_button, dot, glyph, heading, text_button, well, R};
use crate::plugins::{
    chip_value, count, group_connections, is_implicit, Group, Kind, Listing, PlugAct,
};
use crate::runs::s as sv;
use crate::screen::runs::icon_button;
use crate::screen::{ico, white, Screen};
use crate::ui::theme::color as k;
use crate::ui::{div, text, Action, El, Rgba, Role};

pub(super) fn view(s: &Screen<'_>) -> El {
    let p = &s.st.plugins;
    let groups = group_connections(&p.connections, &p.listings);
    let unconnected: Vec<&Listing> = p
        .listings
        .iter()
        .filter(|l| l.pack.is_some() && l.kind == Kind::Connector && l.connected == 0)
        .filter(|l| !l.implicit)
        .collect();
    let mut col = div().col().gap(20.0);
    for g in &groups {
        col = col.child(group(s, g));
    }
    if !unconnected.is_empty() {
        let mut list = div()
            .col()
            .child(heading("Installed, no connection yet").pt(0.0));
        for l in unconnected {
            let version = l
                .pack
                .as_ref()
                .and_then(|p| sv(p, "version"))
                .map(|v| format!("v{v}"))
                .unwrap_or_default();
            list = list.child(
                div()
                    .row()
                    .items_center()
                    .gap(12.0)
                    .py(10.0)
                    .border_top(1.0, white(0.06))
                    .child(well(
                        32.0,
                        R,
                        white(0.04),
                        glyph(Some(l), &l.id, 17.0, k::GRAY_200),
                    ))
                    .child(title(&l.name, &version))
                    .child(div().grow())
                    .child(chip_button(
                        s,
                        "plug-unconnected",
                        PlugAct::Add(l.key.clone()),
                        Some("plus"),
                        "Add connection",
                        k::GRAY_500,
                        true,
                    )),
            );
        }
        col = col.child(list);
    }
    if p.connections.is_empty() && p.packs.is_empty() {
        col = col.child(
            text(
                "No connections yet. Browse the connectors Vorn can talk to.",
                14.0,
                k::GRAY_500,
            )
            .line_height(20.0)
            .wrapping(),
        );
    }
    col
}

fn title(name: &str, sub: &str) -> El {
    div()
        .col()
        .min_w0()
        .child(
            text(name, 13.5, k::GRAY_200)
                .weight(500)
                .line_height(20.0)
                .truncate(),
        )
        .child(text(sub, 11.0, k::GRAY_600).line_height(16.0))
}

/// One connector's heading, sign-in warning and connections.
fn group(s: &Screen<'_>, g: &Group<'_>) -> El {
    let p = &s.st.plugins;
    let mut sub = count(g.connections.len(), "connection");
    if let Some(v) = &g.version {
        sub.push_str(&format!(" · v{v}"));
    }
    let mut head = div()
        .row()
        .items_center()
        .gap(12.0)
        .mb(8.0)
        .child(well(
            32.0,
            R,
            white(0.04),
            glyph(g.listing, &g.connector, 17.0, k::GRAY_200),
        ))
        .child(title(&g.name, &sub))
        .child(div().grow());
    if let Some(l) = g.listing {
        head = head.child(chip_button(
            s,
            "plug-group-add",
            PlugAct::Add(l.key.clone()),
            Some("plus"),
            "Add another",
            k::GRAY_500,
            true,
        ));
    }
    let mut col = div().col().child(head);
    if let Some(st) = p
        .status_of(&g.connector)
        .filter(|st| st.get("authed") != Some(&Value::Bool(true)))
    {
        let message = sv(st, "message")
            .filter(|m| !m.is_empty())
            .unwrap_or("Not signed in.");
        col = col.child(sign_in_warning(message));
    }
    let manifest = p.builtin(&g.connector).and_then(|b| b.get("manifest"));
    let browser = g
        .listing
        .and_then(|l| l.pack.as_ref())
        .and_then(|p| p.pointer("/auth/browser"))
        .is_some();
    let mut rows = div().col().gap(8.0);
    for conn in &g.connections {
        rows = rows.child(row(s, conn, manifest, g.listing, browser));
    }
    col.child(rows)
}

/// The amber box a signed-out connector gets, `backticked` commands in mono.
fn sign_in_warning(message: &str) -> El {
    let amber = Rgba::hexa(0xfee685, 0.9);
    let mut line = div().row().wrap_children().items_center().grow().min_w0();
    for (i, part) in message.split('`').enumerate() {
        if part.is_empty() {
            continue;
        }
        line = line.child(if i % 2 == 1 {
            text(part, 11.0, Rgba::hex(0xfef3c6))
                .mono()
                .line_height(16.0)
                .px(4.0)
                .rounded(R)
                .bg(Rgba::rgba_const(0, 0, 0, 0.3))
                .border(1.0, white(0.08))
        } else {
            text(part, 11.5, amber).line_height(16.0).wrapping()
        });
    }
    div()
        .row()
        .items_start()
        .gap(8.0)
        .mb(8.0)
        .px(12.0)
        .py(8.0)
        .rounded(R)
        .border(1.0, k::AMBER_500.alpha(0.3))
        .bg(k::AMBER_500.alpha(0.04))
        .child(ico("triangle-alert", 13.0, 2.0, k::AMBER_400).mt(2.0))
        .child(line)
}

fn line11(t: impl Into<String>, fg: Rgba) -> El {
    text(t.into(), 11.0, fg).line_height(16.0)
}

/// `ConnectionRow`.
fn row(
    s: &Screen<'_>,
    conn: &Value,
    manifest: Option<&Value>,
    listing: Option<&Listing>,
    browser: bool,
) -> El {
    let p = &s.st.plugins;
    let id = sv(conn, "id").unwrap_or("");
    let connector = sv(conn, "connectorId").unwrap_or("");
    let name = sv(conn, "name").unwrap_or("");
    let busy =
        |verb| matches!(p.activity.get(id), Some(crate::plugins::Activity::Busy(v)) if *v == verb);
    let mut tools = div().row().items_center().gap(2.0).shrink0();
    tools = tools.child(icon_button(
        s,
        "plug-conn-backfill",
        act(PlugAct::Backfill(id.to_owned())),
        if busy(crate::plugins::Verb::Backfill) {
            "loader-circle"
        } else {
            "import"
        },
        "Import existing items matching this connection's filters. Bypasses the cron cursor.",
    ));
    if connector == "http" {
        tools = tools.child(icon_button(
            s,
            "plug-conn-test",
            act(PlugAct::Test(id.to_owned())),
            "activity",
            "Send a request through this profile now and report the status",
        ));
    }
    if is_implicit(conn) {
        tools = tools.child(
            div()
                .p(4.0)
                .opacity(0.4)
                .role(
                    Role::Button,
                    "This connection came with its connector. Remove the pack instead.",
                )
                .disabled(true)
                .child(ico("trash-2", 12.0, 2.0, k::INK_FAINT)),
        );
    } else {
        tools = tools.child(icon_button(
            s,
            "plug-conn-delete",
            act(PlugAct::Delete(id.to_owned())),
            if busy(crate::plugins::Verb::Delete) {
                "loader-circle"
            } else {
                "trash-2"
            },
            "Remove this connection (seeded workflows are also deleted)",
        ));
    }
    let mut col = div()
        .col()
        .px(16.0)
        .py(8.0)
        .rounded(R)
        .bg(white(0.03))
        .border(1.0, white(0.06))
        .child(
            div()
                .row()
                .items_center()
                .justify_between()
                .child(
                    div()
                        .row()
                        .items_center()
                        .gap(12.0)
                        .min_w0()
                        .child(well(
                            24.0,
                            R,
                            white(0.04),
                            glyph(
                                listing,
                                crate::library::connection_connector(conn),
                                14.0,
                                k::GRAY_200,
                            ),
                        ))
                        .child(
                            text(name, 14.0, k::GRAY_200)
                                .weight(500)
                                .line_height(20.0)
                                .truncate(),
                        ),
                )
                .child(tools),
        );
    if browser {
        col = col.child(browser_line(s, conn, id));
    }
    if let Some((ok, message)) = p.tested.get(id) {
        let fallback = if *ok { "Reachable" } else { "Failed" };
        let shown = if message.is_empty() {
            fallback
        } else {
            message
        };
        col = col.child(
            line11(shown, if *ok { k::GREEN_400 } else { k::RED_400 })
                .mt(4.0)
                .wrapping(),
        );
    }
    col = col.child(polls(s, conn, id, manifest));
    col = col.child(footer(s, conn, id));
    let secret = |key: &str| {
        crate::plugins::manifest_fields(manifest, "auth")
            .iter()
            .any(|f| sv(f, "key") == Some(key) && sv(f, "type") == Some("password"))
    };
    let chips: Vec<El> = conn
        .get("filters")
        .and_then(Value::as_object)
        .into_iter()
        .flatten()
        .filter_map(|(key, v)| chip_value(key, v, secret(key)).map(|v| (key, v)))
        .map(|(key, v)| {
            text(format!("{key}: {v}"), 10.0, k::GRAY_400)
                .line_height(14.0)
                .px(6.0)
                .py(2.0)
                .rounded(R)
                .bg(white(0.04))
        })
        .collect();
    if !chips.is_empty() {
        col = col.child(div().row().wrap_children().gap(4.0).mt(6.0).children(chips));
    }
    col
}

/// Whether the connection's browser login is live, and the moves for it.
fn browser_line(s: &Screen<'_>, conn: &Value, id: &str) -> El {
    let p = &s.st.plugins;
    let signed = conn.get("signedInAt").is_some_and(|v| !v.is_null());
    let signing = matches!(
        p.activity.get(id),
        Some(crate::plugins::Activity::Busy(crate::plugins::Verb::SignIn))
    );
    let said = match (signed, sv(conn, "signedInAs")) {
        (true, Some(who)) => format!("Signed in as {who}"),
        (true, None) => "Signed in".to_owned(),
        _ => "Not signed in".to_owned(),
    };
    let mut row = div()
        .row()
        .items_center()
        .gap(8.0)
        .mt(4.0)
        .child(dot(if signed { k::STATUS_SAGE } else { k::INK_GHOST }))
        .child(line11(said, if signed { k::GRAY_300 } else { k::GRAY_500 }));
    let again = if signed { "Sign in again" } else { "Sign in" };
    row = if signing {
        row.child(line11(again, k::GRAY_500).opacity(0.5))
    } else {
        row.child(text_button(
            s,
            "plug-conn-signin",
            PlugAct::SignIn(id.to_owned()),
            again,
            11.0,
            k::GRAY_500,
            k::GRAY_200,
        ))
    };
    if signed && !signing {
        row = row.child(text_button(
            s,
            "plug-conn-signout",
            PlugAct::SignOut(id.to_owned()),
            "Sign out",
            11.0,
            k::GRAY_500,
            k::GRAY_200,
        ));
    }
    row
}

/// The seeded workflows that poll this connection, and any the manifest misses.
fn polls(s: &Screen<'_>, conn: &Value, id: &str, manifest: Option<&Value>) -> El {
    let prefix = format!("connector:{id}:");
    let seeded: Vec<_> = s
        .store
        .workflows
        .iter()
        .filter(|w| w.id.starts_with(&prefix))
        .collect();
    let mut col = div().col().gap(4.0).mt(6.0);
    for wf in &seeded {
        let cron = wf
            .nodes
            .as_array()
            .into_iter()
            .flatten()
            .find(|n| sv(n, "type") == Some("trigger"))
            .and_then(|n| n.pointer("/config/cron"))
            .and_then(Value::as_str)
            .unwrap_or("");
        let open = s
            .acts
            .on("plug-conn-wf", act(PlugAct::OpenWorkflow(wf.id.clone())));
        let fg = k::GRAY_400.mix(k::WHITE, s.ui.hover(open));
        let polling = s.st.plugins.busy(&wf.id);
        let run = icon_button(
            s,
            "plug-conn-run",
            act(PlugAct::RunWorkflow(wf.id.clone(), id.to_owned())),
            if polling { "loader-circle" } else { "play" },
            "Poll the connector now instead of waiting for the next cron tick",
        );
        col = col.child(
            div()
                .row()
                .items_center()
                .justify_between()
                .child(
                    div()
                        .id(open)
                        .on(Action::Click)
                        .ring(R)
                        .row()
                        .items_center()
                        .gap(6.0)
                        .min_w0()
                        .role(Role::Button, &wf.name)
                        .child(ico("workflow", 12.0, 1.75, k::GRAY_500))
                        .child(line11(wf.name.as_str(), fg).truncate())
                        .child(line11(
                            format!("· {}", crate::plugins::human_cron(cron)),
                            k::GRAY_600,
                        )),
                )
                .child(run),
        );
        if let Some(crate::plugins::Activity::Failed(e)) = s.st.plugins.activity.get(&wf.id) {
            col = col.child(super::tone_line(crate::plugins::Tone::Broken, e, 11.0));
        }
    }
    let defaults = manifest
        .and_then(|m| m.get("defaultWorkflows"))
        .and_then(Value::as_array);
    for e in defaults.into_iter().flatten() {
        let event = sv(e, "event").unwrap_or("");
        let wanted = format!("{prefix}{event}");
        if seeded.iter().any(|w| w.id == wanted) {
            continue;
        }
        let ev_name = sv(e, "name").unwrap_or(event);
        col = col.child(
            div()
                .row()
                .items_center()
                .justify_between()
                .child(line11(
                    format!("No workflow for {ev_name} — polling disabled"),
                    k::GRAY_600,
                ))
                .child(reset(s, conn, event)),
        );
    }
    col
}

fn reset(s: &Screen<'_>, conn: &Value, event: &str) -> El {
    let id = s.acts.on(
        "plug-conn-reseed",
        act(PlugAct::Reseed(
            sv(conn, "id").unwrap_or("").to_owned(),
            event.to_owned(),
        )),
    );
    let hot = s.ui.hover(id);
    let label = "Reset default workflow";
    div()
        .id(id)
        .on(Action::Click)
        .ring(R)
        .px(8.0)
        .py(2.0)
        .rounded(R)
        .border(1.0, white(0.1))
        .bg(white(0.06 * hot))
        .role(Role::Button, label)
        .child(
            text(label, 10.0, k::GRAY_400.mix(k::GRAY_200, hot))
                .line_height(14.0)
                .silent(),
        )
}

/// What the row is doing, when it last synced, and the last import.
fn footer(s: &Screen<'_>, conn: &Value, id: &str) -> El {
    let p = &s.st.plugins;
    let mut row = div()
        .row()
        .items_center()
        .wrap_children()
        .gap(8.0)
        .mt(4.0)
        .child_if(activity_line(s, id));
    if let Some(at) = sv(conn, "lastSyncAt").filter(|a| !a.is_empty()) {
        row = row.child(line11(
            format!("Last synced {}", crate::tasks::locale_string(at)),
            k::GRAY_600,
        ));
    }
    let failed = |e: &str| {
        div()
            .row()
            .items_center()
            .gap(4.0)
            .child(ico("circle-alert", 10.0, 2.0, k::RED_400))
            .child(line11(e, k::RED_400).wrapping())
    };
    if let Some(e) = sv(conn, "lastSyncError").filter(|e| !e.is_empty()) {
        row = row.child(failed(e));
    }
    match p.backfilled.get(id) {
        Some(Ok((imported, updated))) => {
            let mut said = format!("+{imported} imported");
            if *updated > 0 {
                said.push_str(&format!(", {updated} updated"));
            }
            row = row.child(line11(said, k::GREEN_400));
        }
        Some(Err(e)) => row = row.child(failed(e)),
        None => {}
    }
    row
}

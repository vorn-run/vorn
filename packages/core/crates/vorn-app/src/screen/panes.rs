//! The panes beside a session's terminal: the Browser extension's tree,
//! and in its page part the address or artifact bar, banner, design canvas,
//! doc editor and comment rail; and the Device extension's tree, whose
//! screen `super::video` draws. Pages are native web views the app seats in
//! the custom boxes left for them.

use super::{c, ico, white, Screen};
use crate::act::{Act, Acts};
use crate::artifact::{self, Anchor, Comment, Tweak};
use crate::client::Session;
use crate::components::card::{card_button, CardButton, CardVariant};
use crate::look;
use crate::panes::{
    board_key, browser_key, canvas_id, compare_key, device_key, note_id, page_key, rail_text_id,
    video_part, Live, PageArtifact, PaneAct, PaneKind,
};
use crate::ui::{
    custom, div, field, text, textarea, tooltip, Action, El, Id, Place, Rgba, Role, Sense,
    TextStyle,
};
use crate::web::address::display_host;

fn ink() -> Rgba {
    c(look::INK)
}

fn ink2() -> Rgba {
    look::INK.alpha(look::INK_SECONDARY)
}

fn faint() -> Rgba {
    look::INK.alpha(look::INK_FAINT)
}

fn pane(a: PaneAct) -> Act {
    Act::Pane(a)
}

fn plural(n: usize) -> &'static str {
    if n == 1 {
        ""
    } else {
        "s"
    }
}

/// A number as JavaScript prints it: no `.0` on a whole one.
fn js_number(n: f64) -> String {
    if n.fract() == 0.0 && n.abs() < 1e15 {
        format!("{}", n as i64)
    } else {
        n.to_string()
    }
}

/// `ICON_BUTTON`: `p-1 rounded hover:bg-white/10 text-ink`, `on` holding
/// the hover look, and `disabled:opacity-25`.
fn icon_btn(
    s: &Screen<'_>,
    act: PaneAct,
    name: &str,
    label: &str,
    tip: Option<&str>,
    on: Option<Rgba>,
    enabled: bool,
) -> El {
    let id = s.acts.on("pane", pane(act));
    let hot = if enabled { s.ui.hover(id) } else { 0.0 };
    let bg = on.unwrap_or_else(|| white(0.0).mix(white(0.10), hot));
    let b = div()
        .id(id)
        .on(Action::Click)
        .p(4.0)
        .rounded(4.0)
        .shrink0()
        .bg(bg)
        .role(Role::Button, label)
        .child(ico(name, 14.0, 2.0, ink()));
    let b = if enabled {
        b
    } else {
        b.disabled(true).opacity(0.25)
    };
    match tip {
        Some(t) if enabled => tooltip(s.ui, b, id, t, None),
        _ => b,
    }
}

/// `h-6 px-2.5 rounded border-white/12 text-[12px]`: the bar's text buttons.
fn text_btn(s: &Screen<'_>, act: PaneAct, label: El, name: &str, enabled: bool) -> (El, Id) {
    let id = s.acts.on("pane", pane(act));
    let hot = if enabled { s.ui.hover(id) } else { 0.0 };
    let b = div()
        .id(id)
        .on(Action::Click)
        .row()
        .items_center()
        .gap(6.0)
        .h(24.0)
        .px(10.0)
        .rounded(4.0)
        .shrink0()
        .border(1.0, white(0.12))
        .bg(white(0.0).mix(white(0.06), hot))
        .role(Role::Button, name)
        .child(label);
    let b = if enabled {
        b
    } else {
        b.disabled(true).opacity(0.4)
    };
    (b, id)
}

/// Maximize or restore, and close.
fn controls(s: &Screen<'_>, sid: &str, kind: PaneKind, title: &str) -> El {
    let maxed = s.st.bench.maximized.get(sid) == Some(&kind);
    let (icon, tip, aria) = if maxed {
        ("minimize-2", "Restore", format!("Restore {title}"))
    } else {
        ("maximize-2", "Maximize", format!("Maximize {title}"))
    };
    div()
        .row()
        .items_center()
        .gap(2.0)
        .shrink0()
        .child(icon_btn(
            s,
            PaneAct::Maximize(sid.to_owned(), kind),
            icon,
            &aria,
            Some(tip),
            None,
            true,
        ))
        .child(icon_btn(
            s,
            PaneAct::Close(sid.to_owned(), kind),
            "x",
            &format!("Close {title}"),
            Some("Close"),
            None,
            true,
        ))
}

/// The panes a session shows: its browser, its device, its extension's.
fn kinds(s: &Screen<'_>, sid: &str) -> Vec<PaneKind> {
    let mut out = s.st.bench.kinds(sid);
    if s.st.ext.panes.contains_key(sid) {
        out.push(PaneKind::Extension);
    }
    out
}

/// The pane's glyph, the extension's, else a puzzle piece.
pub(super) fn extension_icon(icon: Option<&serde_json::Value>, size: f32, color: Rgba) -> El {
    let svg = icon.and_then(crate::library::parse_icon).and_then(|i| {
        crate::library::mark_svg(&crate::library::Look {
            connector: "sdk".to_owned(),
            packaged: true,
            icon: Some(i),
        })
    });
    match svg {
        Some(svg) => crate::ui::icon(svg, size, color),
        None => ico("puzzle", size, 2.0, color),
    }
}

/// Its own bar naming the pane and its extension, over
/// the page the pack ships or the program the host runs.
fn extension_pane(s: &Screen<'_>, sid: &str) -> El {
    let Some(p) = s.st.ext.panes.get(sid) else {
        return div();
    };
    let bar = div()
        .row()
        .items_center()
        .gap(6.0)
        .px(8.0)
        .py(4.0)
        .shrink0()
        .child(extension_icon(p.icon.as_ref(), 12.0, c(look::GRAY_500)))
        .child(
            text(p.title.as_str(), 12.0, c(look::GRAY_300))
                .weight(500)
                .line_height(18.0)
                .shrink0(),
        )
        .child(text("\u{b7}", 12.0, crate::ui::theme::color::INK_GHOST).line_height(18.0))
        .child(
            div().grow().min_w0().child(
                text(
                    p.extension_name.as_str(),
                    11.0,
                    crate::ui::theme::color::INK_SECONDARY,
                )
                .line_height(16.0)
                .truncate(),
            ),
        )
        .child(controls(s, sid, PaneKind::Extension, &p.title));
    let body = if let Some(key) = crate::extensions::view_key(p) {
        div()
            .id(crate::ui::Id::new("ext-view").child(sid))
            .col()
            .grow()
            .min_h(0.0)
            .scroll_y()
            .bg(c(look::SURFACE_PANEL))
            .child(super::points::pane(s, &key))
    } else if p.url.is_some() {
        web_box(s, &crate::extensions::page_key(sid), p.url.as_deref()).bg(c(look::SURFACE_PANEL))
    } else {
        let slot = p
            .terminal_id
            .as_ref()
            .and_then(|t| s.st.ext.slots.iter().position(|x| x == t));
        let focus = s.acts.on(
            "ext-focus",
            crate::act::Act::Ext(crate::extensions::ExtAct::Focus(sid.to_owned())),
        );
        let mut el = div()
            .id(focus)
            .on(Action::Click)
            .grow()
            .min_h(0.0)
            .w_full()
            .pt(2.0)
            .bg(c(look::SURFACE_SUNKEN));
        if let Some(i) = slot {
            el = el.child(
                custom(crate::extensions::TERM_ID_BASE + i as u64)
                    .w_full()
                    .h_full(),
            );
        }
        el
    };
    div()
        .col()
        .grow()
        .min_h(0.0)
        .min_w0()
        .role(Role::Group, format!("{}: {}", p.extension_name, p.title))
        .child(bar)
        .child(body)
}

/// The browser over the device, or the maximized one alone.
pub(super) fn column(s: &Screen<'_>, session: &Session) -> Option<El> {
    let sid = session.info.id.as_str();
    let bench = &s.st.bench;
    let mut kinds = kinds(s, sid);
    if let Some(m) = bench.maximized.get(sid) {
        kinds.retain(|k| k == m);
    }
    if kinds.is_empty() {
        return None;
    }
    let mut col = div().col().grow().min_h(0.0).min_w0().gap(1.0);
    for (n, kind) in kinds.iter().enumerate() {
        if n > 0 {
            col = col.child(div().h(1.0).w_full().shrink0().bg(white(0.06)));
        }
        col = col.child(match kind {
            PaneKind::Browser => browser_pane(s, session),
            PaneKind::Device => device_pane(s, session),
            PaneKind::Extension => extension_pane(s, sid),
        });
    }
    Some(col)
}

/// Whether a session has a pane maximized over its terminal.
pub(super) fn maximized(s: &Screen<'_>, sid: &str) -> bool {
    s.st.bench
        .maximized
        .get(sid)
        .is_some_and(|k| kinds(s, sid).contains(k))
}

/// A web view's box, or the pane's own words where none can show, with a
/// way to open `url` in a tab of its own.
fn web_box(s: &Screen<'_>, key: &str, url: Option<&str>) -> El {
    match s.st.bench.slot_id(key) {
        Some(id) => custom(id).grow().min_h(0.0).min_w0(),
        None => {
            let mut el = div().grow().min_h(0.0).min_w0().center();
            if let Some(why) = &s.st.bench.unavailable {
                el = el.p(16.0).col().gap(10.0).child(
                    text(why.as_str(), 11.0, c(look::GRAY_500))
                        .wrapping()
                        .min_w0()
                        .role(Role::Status, why.as_str()),
                );
                if let Some(url) = url.filter(|u| u.starts_with("http")) {
                    el = el.child(open_in_tab(s, url));
                }
            }
            el
        }
    }
}

/// "Open in a new tab", for a page a web view cannot hold.
fn open_in_tab(s: &Screen<'_>, url: &str) -> El {
    let id = s.acts.on("pane", Act::OpenUrl(url.to_owned()));
    let label = format!("Open {} in a new tab", display_host(url));
    div()
        .id(id)
        .on(Action::Click)
        .row()
        .items_center()
        .gap(6.0)
        .px(10.0)
        .py(5.0)
        .rounded(6.0)
        .border(1.0, white(0.10))
        .bg(white(0.04).mix(white(0.10), s.ui.hover(id)))
        .role(Role::Button, label.as_str())
        .child(ico("external-link", 12.0, 2.0, c(look::GRAY_300)))
        .child(text(label.as_str(), 12.0, c(look::GRAY_300)).truncate())
}

// ---- browser ------------------------------------------------------------

/// The Browser's tree for the session, its tab strip ending with the
/// pane's own controls; its page part is drawn by [`web_part`].
fn browser_pane(s: &Screen<'_>, session: &Session) -> El {
    let sid = session.info.id.as_str();
    let host = display_host(page_url(s, sid));
    let chrome = |tools: Vec<El>| {
        div()
            .row()
            .items_center()
            .gap(4.0)
            .shrink0()
            .children(tools)
            .child(controls(s, sid, PaneKind::Browser, &host))
    };
    let body = browser_key(&s.st.points, sid)
        .and_then(|key| super::points::draw_scoped(s, &key, Some(&chrome), None))
        .unwrap_or_else(|| div().grow());
    div()
        .col()
        .grow()
        .min_h(0.0)
        .min_w0()
        .clip()
        .bg(c(look::SURFACE_PANEL))
        .role(Role::Region, "Browser")
        .child(body)
}

/// Where the session's page is: as it last reported, else as asked.
fn page_url<'a>(s: &'a Screen<'_>, sid: &str) -> &'a str {
    let bench = &s.st.bench;
    bench
        .live(sid)
        .and_then(|l| l.url.as_deref())
        .or_else(|| bench.pages.get(sid).map(String::as_str))
        .unwrap_or("")
}

/// A session's page where the Browser's tree puts it: the address or
/// artifact bar, the banner, a failure, and the page or what reviews it.
pub(super) fn web_part(s: &Screen<'_>, sid: &str, agent: &str) -> El {
    let live = s.st.bench.live(sid);
    let url = page_url(s, sid);
    div()
        .col()
        .grow()
        .min_h(0.0)
        .min_w0()
        .child(header(s, sid, live, url, agent))
        .child_if(banner(s, sid, live))
        .child_if(live.and_then(|l| l.failed.as_deref()).map(|f| {
            div().px(8.0).py(4.0).shrink0().child(
                text(f, 10.0, look::AMBER_400.alpha(0.9))
                    .wrapping()
                    .min_w0()
                    .role(Role::Alert, f),
            )
        }))
        .child(page_body(s, sid, live, agent))
}

/// The row under the tabs: the doc's edit bar, the artifact bar, a
/// design's tweaks, or the address bar; then pick and draw.
fn header(s: &Screen<'_>, sid: &str, live: Option<&Live>, url: &str, agent: &str) -> El {
    let bench = &s.st.bench;
    let art = bench.artifact(sid);
    let editing = live.and_then(|l| l.editing.as_ref()).is_some();
    let boards = bench.boards(sid).is_some();
    let manifest = live.and_then(|l| l.manifest.as_ref());
    let mut row = div()
        .row()
        .items_center()
        .gap(2.0)
        .px(6.0)
        .py(4.0)
        .shrink0();
    match art {
        Some((a, _)) if editing => {
            row = row
                .child(
                    div()
                        .row()
                        .items_center()
                        .h(24.0)
                        .px(8.0)
                        .rounded(4.0)
                        .shrink0()
                        .border(1.0, white(0.08))
                        .child(
                            text(format!("v{}", a.version), 11.0, ink2())
                                .mono()
                                .silent(),
                        ),
                )
                .child(
                    text("doc \u{b7} you are editing", 11.0, faint())
                        .mono()
                        .truncate()
                        .ml(6.0),
                )
                .child(div().grow());
            let id = s.acts.on("pane", pane(PaneAct::Discard(sid.to_owned())));
            row = row.child(
                div()
                    .id(id)
                    .on(Action::Click)
                    .row()
                    .items_center()
                    .h(24.0)
                    .px(10.0)
                    .rounded(4.0)
                    .shrink0()
                    .bg(white(0.0).mix(white(0.06), s.ui.hover(id)))
                    .role(Role::Button, "Discard")
                    .child(text("Discard", 12.0, ink2()).silent()),
            );
        }
        Some((a, state)) => {
            row = artifact_bar(s, row, sid, &a, state, live, agent);
            if !boards {
                row = row.child_if(manifest.and_then(|m| tweak_row(s, sid, m, live)));
            }
        }
        None if manifest.is_some_and(|m| m.kind == "design") => {
            if !boards {
                row = row.child_if(manifest.and_then(|m| tweak_row(s, sid, m, live)));
            }
            row = row.child(div().grow());
        }
        None => row = address_bar(s, row, sid, live, url),
    }
    if !editing && !boards {
        let picking = live.is_some_and(|l| l.picking);
        row = row
            .child(
                icon_btn(
                    s,
                    PaneAct::Pick(sid.to_owned()),
                    "mouse-pointer-click",
                    "Pick an element for the agent",
                    Some("Point at an element to describe it to this session's agent"),
                    picking.then(|| white(0.10)),
                    true,
                )
                .toggled(picking)
                .ml(4.0),
            )
            .child(icon_btn(
                s,
                PaneAct::Ink(sid.to_owned()),
                "pencil",
                "Draw on the page for the agent",
                Some("Draw over the page, then click again to send it to this session's agent"),
                None,
                true,
            ));
    }
    row
}

/// Back, forward, reload or stop, and the address.
fn address_bar(s: &Screen<'_>, row: El, sid: &str, live: Option<&Live>, url: &str) -> El {
    let (back, forward) = live.map_or((false, false), |l| l.nav);
    let loading = live.is_some_and(|l| l.loading);
    let id = Acts::id("address", &pane(PaneAct::Address(sid.to_owned())));
    let shown = if url == "about:blank" { "" } else { url };
    let value = live
        .and_then(|l| l.draft.clone())
        .unwrap_or_else(|| shown.to_owned());
    s.acts
        .text(id, pane(PaneAct::Address(sid.to_owned())), value);
    let focused = s.ui.focused(id);
    let bg = if focused {
        white(0.07)
    } else {
        white(0.04).mix(white(0.06), s.ui.hover(id))
    };
    let style = if focused {
        TextStyle::mono(11.0)
    } else {
        TextStyle::ui(11.0)
    };
    let mut input = field(id, "Address", "Type a URL", style, c(look::GRAY_300));
    if !focused {
        input = input.text_center();
    }
    row.child(icon_btn(
        s,
        PaneAct::Back(sid.to_owned()),
        "arrow-left",
        "Go back",
        None,
        None,
        back,
    ))
    .child(icon_btn(
        s,
        PaneAct::Forward(sid.to_owned()),
        "arrow-right",
        "Go forward",
        None,
        None,
        forward,
    ))
    .child(icon_btn(
        s,
        PaneAct::ReloadOrStop(sid.to_owned()),
        if loading { "x" } else { "rotate-cw" },
        if loading { "Stop loading" } else { "Reload" },
        None,
        None,
        true,
    ))
    .child(
        div()
            .ml(4.0)
            .grow()
            .min_w0()
            .rounded(999.0)
            .px(12.0)
            .py(4.0)
            .frames(id)
            .bg(bg)
            .child(input.w_full()),
    )
}

/// The version and its menu, who made it, and editing,
/// commenting and sending.
fn artifact_bar(
    s: &Screen<'_>,
    mut row: El,
    sid: &str,
    a: &PageArtifact,
    state: Option<&artifact::State>,
    live: Option<&Live>,
    agent: &str,
) -> El {
    let versions = state.map_or(&[][..], |st| st.versions.as_slice());
    let comments = state.map_or(&[][..], |st| st.comments.as_slice());
    let drafts = comments.iter().filter(|c| c.is_draft()).count();
    let open = live.is_some_and(|l| l.versions);
    let commenting = live.is_some_and(|l| l.commenting);
    let sending = live.is_some_and(|l| l.sending);
    let queued = state.is_some_and(|st| st.queued);
    let now = crate::state::now_ms();
    let byline = |v: &artifact::Version| {
        format!(
            "{} \u{b7} {}",
            if v.author == "user" { "you" } else { agent },
            crate::runs::relative(&v.created_at, now).to_lowercase()
        )
    };

    let vid = s.acts.on("pane", pane(PaneAct::Versions(sid.to_owned())));
    let mut chip = div()
        .id(vid)
        .on(Action::Click)
        .row()
        .items_center()
        .gap(4.0)
        .h(24.0)
        .px(8.0)
        .rounded(4.0)
        .shrink0()
        .border(1.0, white(0.08))
        .bg(white(0.0).mix(white(0.06), s.ui.hover(vid)))
        .role(
            Role::Button,
            format!("Version {}, choose another", a.version),
        )
        .has_popup()
        .expanded(open)
        .child(
            text(format!("v{}", a.version), 11.0, ink2())
                .mono()
                .silent(),
        )
        .child(ico("chevron-down", 11.0, 2.0, ink2()));
    if open {
        let mut newest: Vec<&artifact::Version> = versions.iter().collect();
        newest.sort_by_key(|x| std::cmp::Reverse(x.version));
        let mut menu = div()
            .col()
            .w(240.0)
            .p(4.0)
            .rounded(4.0)
            .border(1.0, white(0.08))
            .bg(c(look::SURFACE_OVERLAY))
            .role(Role::Menu, "Versions");
        for v in newest {
            let count = comments.iter().filter(|c| c.version == v.version).count();
            let mut words = format!(
                "v{} \u{b7} {}",
                v.version,
                if v.author == "user" { "you" } else { agent }
            );
            if count > 0 {
                words.push_str(&format!(" \u{b7} {count} comment{}", plural(count)));
            }
            let current = v.version == a.version;
            let id = s
                .acts
                .on("pane", pane(PaneAct::Version(sid.to_owned(), v.version)));
            let bg = if current {
                white(0.08)
            } else {
                white(0.0).mix(white(0.05), s.ui.hover(id))
            };
            menu = menu.child(
                div()
                    .id(id)
                    .on(Action::Click)
                    .row()
                    .items_center()
                    .gap(8.0)
                    .px(8.0)
                    .h(26.0)
                    .rounded(4.0)
                    .bg(bg)
                    .role(Role::MenuItem, words.as_str())
                    .child(
                        text(words.as_str(), 12.0, if current { ink() } else { ink2() })
                            .truncate()
                            .grow()
                            .silent(),
                    )
                    .child(
                        text(
                            crate::runs::relative(&v.created_at, now).to_lowercase(),
                            11.0,
                            faint(),
                        )
                        .mono()
                        .shrink0()
                        .silent(),
                    ),
            );
        }
        chip = chip.overlay(menu, Place::BelowStart, 4.0);
    }
    row = row
        .child(chip)
        .child(
            text(
                state
                    .and_then(|st| st.version(a.version))
                    .map(byline)
                    .unwrap_or_default(),
                11.0,
                faint(),
            )
            .mono()
            .truncate()
            .ml(6.0),
        )
        .child(div().grow());

    if a.kind == "doc" {
        let latest = state.is_some_and(|st| st.artifact.latest_version == a.version);
        let (b, id) = text_btn(
            s,
            PaneAct::Edit(sid.to_owned()),
            text("Edit", 12.0, ink()).silent(),
            "Edit",
            latest,
        );
        let tip = if latest {
            "Edit the words yourself"
        } else {
            "Open the latest version to edit it"
        };
        row = row
            .child(tooltip(s.ui, b, id, tip, None))
            .child(div().w(4.0).shrink0());
    }
    let comment_tip = if commenting {
        "Stop commenting"
    } else {
        "Comment on the page"
    };
    row = row.child(
        icon_btn(
            s,
            PaneAct::Comments(sid.to_owned()),
            "message-square-plus",
            comment_tip,
            Some(comment_tip),
            commenting.then(|| white(0.10)),
            true,
        )
        .toggled(commenting),
    );
    let pill = div()
        .row()
        .center()
        .min_w(16.0)
        .h(16.0)
        .px(4.0)
        .rounded(8.0)
        .bg(white(0.12))
        .child(
            text(drafts.to_string(), 10.0, ink())
                .mono()
                .weight(600)
                .silent(),
        );
    let label = if queued { "Queued" } else { "Send to agent" };
    let (b, id) = text_btn(
        s,
        PaneAct::Send(sid.to_owned()),
        div()
            .row()
            .items_center()
            .gap(6.0)
            .child(text(label, 12.0, ink()).silent())
            .child(pill),
        label,
        drafts > 0 && !sending,
    );
    let tip = if queued {
        format!("Queued: goes when {agent} is at its prompt")
    } else if drafts > 0 {
        format!("Send {drafts} to {agent}")
    } else {
        "Comment on the page first".to_owned()
    };
    row.child(tooltip(s.ui, b.ml(4.0), id, &tip, None))
}

/// `TweakBar` as a row in the header.
fn tweak_row(s: &Screen<'_>, sid: &str, m: &artifact::Manifest, live: Option<&Live>) -> Option<El> {
    if m.tweaks.is_empty() {
        return None;
    }
    let mut row = div()
        .row()
        .items_center()
        .gap(12.0)
        .px(8.0)
        .py(4.0)
        .shrink0()
        .wrap_children()
        .role(Role::Group, "Design controls");
    for (key, t) in &m.tweaks {
        row = row.child(
            div()
                .row()
                .items_center()
                .gap(6.0)
                .child(tweak_label(key, t))
                .child(tweak_control(s, sid, key, t, live)),
        );
    }
    Some(row)
}

fn tweak_label(key: &str, t: &Tweak) -> El {
    text(t.label().unwrap_or(key), 10.5, faint()).silent()
}

/// One tweak's control: a switch, a number, a choice or swatches.
fn tweak_control(s: &Screen<'_>, sid: &str, key: &str, t: &Tweak, live: Option<&Live>) -> El {
    let label = t.label().unwrap_or(key);
    let current = live
        .and_then(|l| l.values.get(key))
        .filter(|v| {
            std::mem::discriminant(*v) == std::mem::discriminant(&t.default_value())
                || (v.is_number() && t.default_value().is_number())
        })
        .cloned()
        .unwrap_or_else(|| t.default_value());
    let set = |value: String| {
        pane(PaneAct::Tweak {
            session: sid.to_owned(),
            key: key.to_owned(),
            value,
        })
    };
    match t {
        Tweak::Boolean { .. } => {
            let on = current.as_bool().unwrap_or(false);
            let id = s.acts.on("pane-tweak", set((!on).to_string()));
            div()
                .id(id)
                .on(Action::Click)
                .w(28.0)
                .h(16.0)
                .rounded(8.0)
                .shrink0()
                .bg(if on { c(look::BRONZO) } else { white(0.12) })
                .role(Role::Switch, label)
                .toggled(on)
                .ring(8.0)
                .child(
                    div()
                        .size(10.0, 10.0)
                        .rounded(5.0)
                        .bg(c(look::WHITE))
                        .absolute(if on { 15.0 } else { 3.0 }, 3.0),
                )
        }
        Tweak::Number { unit, .. } => {
            let act = pane(PaneAct::TweakText(sid.to_owned(), key.to_owned()));
            let id = Acts::id("pane-tweak", &act);
            s.acts
                .text(id, act, current.as_f64().map(js_number).unwrap_or_default());
            div()
                .row()
                .items_center()
                .gap(4.0)
                .child(
                    div()
                        .w(68.0)
                        .px(6.0)
                        .py(2.0)
                        .rounded(4.0)
                        .bg(white(0.04))
                        .border(
                            1.0,
                            if s.ui.focused(id) {
                                white(0.25)
                            } else {
                                white(0.08)
                            },
                        )
                        .frames(id)
                        .child(field(id, label, "", TextStyle::ui(11.0), ink()).w_full()),
                )
                .child_if(unit.as_deref().map(|u| text(u, 10.5, faint()).silent()))
        }
        Tweak::Select { options, .. } => {
            let picked = current
                .as_str()
                .filter(|p| options.iter().any(|o| o == p))
                .or(options.first().map(String::as_str))
                .unwrap_or_default();
            let at = options.iter().position(|o| o == picked).unwrap_or(0);
            let next = options
                .get((at + 1) % options.len().max(1))
                .cloned()
                .unwrap_or_default();
            let id = s.acts.on("pane-tweak", set(next));
            // In a row, so the select is as wide as its value, not the column.
            div().row().child(
                div()
                    .id(id)
                    .on(Action::Click)
                    .row()
                    .items_center()
                    .gap(4.0)
                    .px(6.0)
                    .py(2.0)
                    .rounded(4.0)
                    .bg(white(0.04).mix(white(0.06), s.ui.hover(id)))
                    .border(1.0, white(0.08))
                    .role(Role::ComboBox, label)
                    .value(picked)
                    .child(text(picked, 11.0, ink()).silent())
                    .child(ico("chevron-down", 10.0, 2.0, faint())),
            )
        }
        Tweak::Color { options, .. } => {
            let colour = current.as_str().unwrap_or_default().to_owned();
            let swatch = |css: &str| {
                let fill = artifact::hex_color(css).map_or(white(0.0), c);
                div()
                    .size(16.0, 16.0)
                    .rounded(2.0)
                    .shrink0()
                    .bg(fill)
                    .border(1.0, white(0.2))
            };
            match options.as_deref() {
                Some(opts) if !opts.is_empty() => {
                    let mut group = div().row().items_center().gap(4.0).role(Role::Group, label);
                    for o in opts {
                        let id = s.acts.on("pane-tweak", set(o.clone()));
                        let picked = *o == colour;
                        let mut sw = swatch(o)
                            .id(id)
                            .on(Action::Click)
                            .role(Role::Button, o.as_str())
                            .toggled(picked)
                            .ring(2.0);
                        if picked {
                            sw = div().p(1.0).rounded(3.0).border(1.0, white(0.7)).child(sw);
                        }
                        group = group.child(sw);
                    }
                    group
                }
                _ => swatch(&colour).role(Role::Image, label),
            }
        }
    }
}

/// A newer version is out, or this one answers comments.
fn banner(s: &Screen<'_>, sid: &str, live: Option<&Live>) -> Option<El> {
    let (a, Some(state)) = s.st.bench.artifact(sid)? else {
        return None;
    };
    let key = format!("{}:{}", a.id, a.version);
    if live.and_then(|l| l.dismissed.as_deref()) == Some(key.as_str()) {
        return None;
    }
    let latest = state.artifact.latest_version;
    let behind = a.version < latest;
    let (answered, on) = state.answered(a.version);
    if !behind && answered.is_empty() {
        return None;
    }
    let comparing = live.is_some_and(|l| l.comparing);
    let words = if behind {
        format!("v{latest} is out; you are reading v{}.", a.version)
    } else {
        let n = answered.len();
        let on_v = on.map(|v| format!(" on v{v}")).unwrap_or_default();
        format!(
            "v{} answers your {n} comment{}{on_v}.",
            a.version,
            plural(n)
        )
    };
    let small = |act: PaneAct, label: String, pressed: bool| {
        let id = s.acts.on("pane", pane(act));
        let bg = if pressed {
            white(0.08)
        } else {
            white(0.0).mix(white(0.06), s.ui.hover(id))
        };
        div()
            .id(id)
            .on(Action::Click)
            .row()
            .items_center()
            .h(24.0)
            .px(8.0)
            .rounded(4.0)
            .shrink0()
            .bg(bg)
            .role(Role::Button, label.as_str())
            .child(text(label.as_str(), 12.0, ink()).silent())
    };
    let action = if behind {
        Some(small(
            PaneAct::OpenLatest(sid.to_owned()),
            format!("Open v{latest}"),
            false,
        ))
    } else {
        on.map(|v| {
            let label = if comparing {
                "Close compare".to_owned()
            } else {
                format!("Compare with v{v}")
            };
            small(PaneAct::Compare(sid.to_owned()), label, comparing).toggled(comparing)
        })
    };
    let dismiss = s.acts.on("pane", pane(PaneAct::Dismiss(sid.to_owned())));
    let x = div()
        .id(dismiss)
        .on(Action::Click)
        .p(4.0)
        .rounded(4.0)
        .shrink0()
        .bg(white(0.0).mix(white(0.10), s.ui.hover(dismiss)))
        .role(Role::Button, "Dismiss")
        .child(ico("x", 12.0, 2.0, ink()));
    Some(
        div()
            .row()
            .items_center()
            .gap(8.0)
            .px(10.0)
            .h(32.0)
            .shrink0()
            .border_top(1.0, white(0.04))
            .bg(c(look::SURFACE_SUNKEN))
            .child(text(words.as_str(), 12.0, ink2()).truncate())
            .child(div().grow())
            .child_if(action)
            .child(tooltip(s.ui, x, dismiss, "Dismiss", None)),
    )
}

/// The page, its compare page, the canvas or the doc editor, and the
/// tweaks and comments beside them.
fn page_body(s: &Screen<'_>, sid: &str, live: Option<&Live>, agent: &str) -> El {
    let bench = &s.st.bench;
    let owned = bench.artifact(sid);
    let art = owned.as_ref().map(|(a, st)| (a, *st));
    let editing = live.and_then(|l| l.editing.as_ref());
    let comparing =
        live.is_some_and(|l| l.comparing && l.compare_url.is_some()) && editing.is_none();
    let mut body = div()
        .row()
        .grow()
        .min_h(0.0)
        .min_w0()
        .bg(c(look::SURFACE_PANEL));
    let answered_on = art
        .and_then(|(a, st)| st.map(|st| st.answered(a.version).1))
        .flatten();
    if comparing {
        body = body.child(
            div()
                .col()
                .grow()
                .min_w0()
                .border_right(1.0, white(0.06))
                .child(version_header(answered_on.unwrap_or_default()))
                .child(web_box(
                    s,
                    &compare_key(sid),
                    live.and_then(|l| l.compare_url.as_deref()),
                )),
        );
    }
    let mut main = div().col().grow().min_w0().min_h(0.0);
    if comparing {
        main = main.child(version_header(art.map_or(0, |(a, _)| a.version)));
    }
    main = main.child(match (editing, bench.boards(sid)) {
        (Some(draft), _) => doc_editor(s, sid, draft, art, agent),
        (None, Some(boards)) => canvas(s, sid, boards),
        (None, None) => web_box(s, &page_key(sid), bench.pages.get(sid).map(String::as_str)),
    });
    body = body.child(main);
    if let (Some(board), Some(m), None) = (
        bench.board(sid),
        live.and_then(|l| l.manifest.as_ref())
            .filter(|m| !m.tweaks.is_empty()),
        editing,
    ) {
        body = body.child(tweak_aside(s, sid, &board.label, m, live));
    }
    if let (true, Some((a, state)), None) = (live.is_some_and(|l| l.commenting), art, editing) {
        body = body.child(rail(s, sid, a.version, state, live, agent));
    }
    body
}

fn version_header(v: u32) -> El {
    div()
        .row()
        .items_center()
        .px(10.0)
        .h(24.0)
        .shrink0()
        .child(text(format!("v{v}"), 11.0, faint()).mono().silent())
}

/// `h4`: the asides' mono uppercase heading.
fn heading(words: &str, count: Option<usize>) -> El {
    let t = |w: String| {
        text(w, 11.0, faint())
            .mono()
            .weight(600)
            .tracking(0.05)
            .silent()
    };
    div()
        .row()
        .justify_between()
        .px(12.0)
        .py(8.0)
        .shrink0()
        .border_bottom(1.0, white(0.04))
        .role(Role::Heading, words)
        .child(t(words.to_uppercase()).truncate())
        .child_if(count.map(|n| t(n.to_string())))
}

/// `bg-ink text-surface-base h-7`, the asides' main button.
fn solid_btn(s: &Screen<'_>, act: PaneAct, label: &str, enabled: bool) -> El {
    let id = s.acts.on("pane", pane(act));
    let b = div()
        .id(id)
        .on(Action::Click)
        .row()
        .center()
        .h(28.0)
        .px(8.0)
        .rounded(4.0)
        .bg(ink())
        .role(Role::Button, label)
        .child(
            text(label, 12.0, c(look::SURFACE_BASE))
                .weight(600)
                .truncate()
                .silent(),
        );
    if enabled {
        b
    } else {
        b.disabled(true).opacity(0.4)
    }
}

/// `h-6`, the asides' quiet button.
fn quiet_btn(
    s: &Screen<'_>,
    act: PaneAct,
    label: &str,
    color: Rgba,
    border: bool,
    enabled: bool,
) -> El {
    let id = s.acts.on("pane", pane(act));
    let hot = if enabled { s.ui.hover(id) } else { 0.0 };
    let mut b = div()
        .id(id)
        .on(Action::Click)
        .row()
        .center()
        .h(24.0)
        .rounded(4.0)
        .bg(white(0.0).mix(white(0.06), hot))
        .role(Role::Button, label)
        .child(text(label, 12.0, color).truncate().silent());
    if border {
        b = b.border(1.0, white(0.12));
    }
    if enabled {
        b
    } else {
        b.disabled(true).opacity(0.4)
    }
}

/// The markdown, and the edits it makes beside it.
fn doc_editor(
    s: &Screen<'_>,
    sid: &str,
    draft: &crate::panes::DocDraft,
    art: Option<(&PageArtifact, Option<&artifact::State>)>,
    agent: &str,
) -> El {
    let (_, edits) = artifact::merge_doc_edit(&draft.original, &draft.edited);
    let drafts = s.st.bench.comments(sid).0.len();
    let total = edits.len() + drafts;
    let next = art.map_or(1, |(a, st)| {
        st.map_or(a.version, |st| st.artifact.latest_version) + 1
    });
    let saving = s.st.bench.live(sid).is_some_and(|l| l.saving);
    let act = pane(PaneAct::DocText(sid.to_owned()));
    let id = Acts::id("doc-text", &act);
    s.acts.text(id, act, draft.edited.as_str());
    let mut list = div().col();
    if edits.is_empty() {
        list = list.child(
            div().px(12.0).py(12.0).child(
                text(
                    "Change the words; each changed paragraph goes as its own edit.",
                    12.0,
                    faint(),
                )
                .wrapping()
                .min_w0(),
            ),
        );
    }
    for e in &edits {
        let mut item = div()
            .col()
            .gap(4.0)
            .px(12.0)
            .py(10.0)
            .border_bottom(1.0, white(0.04))
            .child(text("Your edit", 11.0, faint()).mono());
        if !e.before.is_empty() {
            item = item.child(
                text(super::truncate(&e.before, 240), 12.0, faint())
                    .wrapping()
                    .min_w0()
                    .role(Role::Paragraph, format!("Removed: {}", e.before)),
            );
        }
        if !e.after.is_empty() {
            item = item.child(
                text(super::truncate(&e.after, 240), 12.0, ink())
                    .wrapping()
                    .min_w0(),
            );
        }
        list = list.child(item);
    }
    if drafts > 0 {
        list = list.child(
            div().px(12.0).py(10.0).child(
                text(
                    format!("And {drafts} comment{} already waiting.", plural(drafts)),
                    12.0,
                    faint(),
                )
                .wrapping()
                .min_w0(),
            ),
        );
    }
    let can = !edits.is_empty() && !saving;
    div()
        .row()
        .grow()
        .min_h(0.0)
        .bg(c(look::SURFACE_BASE))
        .child(
            div().grow().min_w0().min_h(0.0).p(16.0).child(
                textarea(id, "Document", "", TextStyle::ui(13.0), ink())
                    .w_full()
                    .h_full(),
            ),
        )
        .child(
            div()
                .col()
                .w(260.0)
                .shrink0()
                .min_h(0.0)
                .border_left(1.0, white(0.06))
                .bg(c(look::SURFACE_PANEL))
                .role(Role::Complementary, "Your edits")
                .child(
                    div()
                        .id(Id::new("doc-edits").child(sid))
                        .col()
                        .grow()
                        .min_h(0.0)
                        .scroll_y()
                        .child(heading("To send", Some(total)))
                        .child(list),
                )
                .child(
                    div()
                        .col()
                        .gap(8.0)
                        .p(12.0)
                        .border_top(1.0, white(0.06))
                        .child(solid_btn(
                            s,
                            PaneAct::SaveDoc(sid.to_owned(), true),
                            &format!("Save v{next} and send {total} to {agent}"),
                            can,
                        ))
                        .child(quiet_btn(
                            s,
                            PaneAct::SaveDoc(sid.to_owned(), false),
                            &format!("Save v{next} without sending"),
                            ink2(),
                            false,
                            can,
                        )),
                ),
        )
}

/// Every artboard side by side, and the zoom.
fn canvas(s: &Screen<'_>, sid: &str, boards: &[artifact::Artboard]) -> El {
    let bench = &s.st.bench;
    let area = s.ui.rect_of(canvas_id(sid)).map(|r| (r.w, r.h));
    let zoom = bench.canvas_zoom(sid, area);
    let selected = bench.board(sid).map(|b| b.id.as_str());
    let (placed, _, _) = artifact::place_artboards(boards, zoom);
    let mut el = div()
        .id(canvas_id(sid))
        // Sensed so the box has a measured rect to fit the zoom against.
        .sense(Sense::HOVER)
        .grow()
        .min_h(0.0)
        .min_w0()
        .clip()
        .bg(c(look::SURFACE_SUNKEN))
        .role(Role::Group, "Design canvas");
    for (b, at) in boards.iter().zip(&placed) {
        let (left, top) = (at.left as f32, at.top as f32);
        let w = (f64::from(b.width) * zoom).round() as f32;
        let h = (f64::from(b.height) * zoom).round() as f32;
        let id = s.acts.on(
            "pane-board",
            pane(PaneAct::Board(sid.to_owned(), b.id.clone())),
        );
        let hot = s.ui.hover(id);
        let chosen = selected == Some(b.id.as_str());
        el = el.child(
            div()
                .id(id)
                .on(Action::Click)
                .row()
                .items_end()
                .gap(6.0)
                .h(artifact::LABEL_HEIGHT as f32 - 4.0)
                .absolute(left, top - artifact::LABEL_HEIGHT as f32)
                .role(Role::Button, b.label.as_str())
                .selected(chosen)
                .child(
                    text(b.label.as_str(), 11.0, if chosen { ink() } else { ink2() })
                        .mono()
                        .silent(),
                )
                .child(
                    text(
                        format!("{} \u{d7} {}", b.width, b.height),
                        11.0,
                        faint().mix(ink2(), hot),
                    )
                    .mono()
                    .silent(),
                ),
        );
        if chosen {
            el = el.child(
                div()
                    .size(w + 8.0, h + 8.0)
                    .absolute(left - 4.0, top - 4.0)
                    .border(2.0, c(look::STATUS_BLUE)),
            );
        }
        let board = match bench.slot_id(&board_key(sid, &b.id)) {
            Some(cid) => custom(cid),
            None => div(),
        };
        el = el.child(board.bg(c(look::WHITE)).size(w, h).absolute(left, top));
    }
    let zoom_btn = |act: PaneAct, name: &str, label: &str| {
        let id = s.acts.on("pane", pane(act));
        let b = div()
            .id(id)
            .on(Action::Click)
            .size(24.0, 24.0)
            .center()
            .rounded(4.0)
            .bg(white(0.0).mix(white(0.06), s.ui.hover(id)))
            .role(Role::Button, label)
            .child(ico(name, 12.0, 2.0, ink2()));
        tooltip(s.ui, b, id, label, None)
    };
    let pct = format!("{}%", (zoom * 100.0).round());
    let bottom = area.map_or(0.0, |(_, h)| (h - 10.0 - 30.0).max(0.0));
    el.child(
        div()
            .row()
            .items_center()
            .gap(2.0)
            .p(2.0)
            .rounded(4.0)
            .border(1.0, white(0.12))
            .bg(c(look::SURFACE_OVERLAY))
            .absolute(10.0, bottom)
            .role(Role::Toolbar, "Zoom")
            .child(zoom_btn(
                PaneAct::Zoom(sid.to_owned(), -1),
                "minus",
                "Zoom out",
            ))
            .child(
                div().px(6.0).child(
                    text(pct.as_str(), 11.5, ink())
                        .mono()
                        .role(Role::Label, format!("Zoom {pct}")),
                ),
            )
            .child(zoom_btn(
                PaneAct::Zoom(sid.to_owned(), 1),
                "plus",
                "Zoom in",
            ))
            .child(zoom_btn(
                PaneAct::Zoom(sid.to_owned(), 0),
                "maximize",
                "Fit every artboard",
            )),
    )
}

/// The canvas's tweaks, in a column beside it.
fn tweak_aside(
    s: &Screen<'_>,
    sid: &str,
    board: &str,
    m: &artifact::Manifest,
    live: Option<&Live>,
) -> El {
    let mut col = div()
        .id(Id::new("pane-tweaks").child(sid))
        .col()
        .w(220.0)
        .shrink0()
        .min_h(0.0)
        .scroll_y()
        .border_left(1.0, white(0.06))
        .bg(c(look::SURFACE_PANEL))
        .role(Role::Complementary, "Tweaks")
        .child(heading(&format!("Tweaks \u{b7} {board}"), None));
    for (key, t) in &m.tweaks {
        col = col.child(
            div()
                .col()
                .gap(6.0)
                .px(12.0)
                .py(10.0)
                .border_bottom(1.0, white(0.04))
                .child(tweak_label(key, t))
                .child(tweak_control(s, sid, key, t, live)),
        );
    }
    col.child(div().grow()).child(
        div().px(12.0).py(10.0).child(
            text("Every artboard takes the same values.", 11.5, faint())
                .wrapping()
                .min_w0(),
        ),
    )
}

/// The quote a rail item points at: `border-l-2 border-status-blue`.
fn target(s: &Screen<'_>, sid: &str, comment: &Comment) -> El {
    let id = s.acts.on(
        "pane-rail",
        pane(PaneAct::RailReveal(sid.to_owned(), comment.id.clone())),
    );
    div()
        .id(id)
        .on(Action::Click)
        .pl(8.0)
        .border_left(2.0, c(look::STATUS_BLUE))
        .role(Role::Button, "Show on the page")
        .child(
            text(super::truncate(&comment.target(), 90), 11.5, faint())
                .mono()
                .wrapping()
                .min_w0()
                .silent(),
        )
}

/// A rail's `edit`, `delete`, `save` or `cancel`.
fn link(s: &Screen<'_>, act: PaneAct, label: &str, hover: Rgba) -> El {
    let id = s.acts.on("pane-rail", pane(act));
    div()
        .id(id)
        .on(Action::Click)
        .role(Role::Button, label)
        .child(
            text(label, 11.0, faint().mix(hover, s.ui.hover(id)))
                .mono()
                .silent(),
        )
}

/// The comments to send, the last ones sent, a note and
/// the send button.
fn rail(
    s: &Screen<'_>,
    sid: &str,
    version: u32,
    state: Option<&artifact::State>,
    live: Option<&Live>,
    agent: &str,
) -> El {
    let (drafts, sent) = s.st.bench.comments(sid);
    let found = |c: &Comment| {
        live.and_then(|l| l.found.get(&c.id)).copied() != Some(false)
            || !matches!(c.anchor, Some(Anchor::Quote { .. }))
    };
    let editing = live.and_then(|l| l.rail_edit.as_deref());
    let mut list = div()
        .id(Id::new("pane-rail").child(sid))
        .col()
        .grow()
        .min_h(0.0)
        .scroll_y()
        .child(heading("To send", Some(drafts.len())));
    if drafts.is_empty() {
        let hint = if s.st.bench.boards(sid).is_some() {
            "Click an artboard to pin a comment there."
        } else {
            "Select words on the page to comment on them."
        };
        list = list.child(
            div()
                .px(12.0)
                .py(12.0)
                .child(text(hint, 12.0, faint()).wrapping().min_w0()),
        );
    }
    for c in &drafts {
        let mut item = div()
            .col()
            .gap(6.0)
            .px(12.0)
            .py(10.0)
            .border_bottom(1.0, white(0.04))
            .child(target(s, sid, c));
        let mut links = div().row().items_center().gap(8.0);
        if !found(c) {
            links = links.child(
                text("words changed", 11.0, crate::screen::c(look::DANGER))
                    .mono()
                    .silent(),
            );
        }
        if editing == Some(c.id.as_str()) {
            let id = rail_text_id(sid);
            s.acts.text(
                id,
                pane(PaneAct::RailText(sid.to_owned())),
                s.ui.edit_text(id).to_owned(),
            );
            item = item.child(rail_field(s, id, "Edit comment", ""));
            links = links
                .child(link(s, PaneAct::RailSave(sid.to_owned()), "save", ink()))
                .child(link(
                    s,
                    PaneAct::RailCancel(sid.to_owned()),
                    "cancel",
                    ink(),
                ));
        } else {
            item = item.child(text(c.body.as_str(), 12.0, ink()).wrapping().min_w0());
            links = links
                .child(link(
                    s,
                    PaneAct::RailEdit(sid.to_owned(), c.id.clone()),
                    "edit",
                    ink(),
                ))
                .child(link(
                    s,
                    PaneAct::RailDelete(sid.to_owned(), c.id.clone()),
                    "delete",
                    crate::screen::c(look::DANGER),
                ));
        }
        list = list.child(item.child(links));
    }
    if !sent.is_empty() {
        let on = sent.iter().map(|c| c.version).max().unwrap_or_default();
        list = list.child(heading(&format!("Sent with v{on}"), None));
        for c in &sent {
            let moved = version > c.version;
            let lost = !found(c);
            let (status, color) = match (moved, lost) {
                (false, _) => ("sent".to_owned(), faint()),
                (true, true) => (
                    format!("words changed in v{version}"),
                    crate::screen::c(look::DANGER),
                ),
                (true, false) => ("still anchored".to_owned(), faint()),
            };
            list = list.child(
                div()
                    .col()
                    .gap(6.0)
                    .px(12.0)
                    .py(10.0)
                    .border_bottom(1.0, white(0.04))
                    .child(target(s, sid, c))
                    .child(text(c.body.as_str(), 12.0, ink2()).wrapping().min_w0())
                    .child(text(status, 11.0, color).mono()),
            );
        }
    }
    let note = note_id(sid);
    let typed = s.ui.edit_text(note).to_owned();
    s.acts
        .text(note, pane(PaneAct::Note(sid.to_owned())), typed.clone());
    let sending = live.is_some_and(|l| l.sending);
    let queued = state.is_some_and(|st| st.queued);
    let n = drafts.len();
    let send_label = if n > 0 {
        format!("Send {n} comment{} to {agent}", plural(n))
    } else {
        "Nothing to send yet".to_owned()
    };
    let mut footer = div()
        .col()
        .gap(8.0)
        .p(12.0)
        .border_top(1.0, white(0.06))
        .child(rail_field(
            s,
            note,
            "Note about the whole version",
            "A note about the whole version",
        ));
    if !typed.trim().is_empty() {
        footer = footer.child(quiet_btn(
            s,
            PaneAct::AddNote(sid.to_owned()),
            "Add note",
            ink(),
            true,
            true,
        ));
    }
    footer = footer.child(solid_btn(
        s,
        PaneAct::Send(sid.to_owned()),
        &send_label,
        n > 0 && !sending,
    ));
    if queued {
        footer = footer.child(
            text(
                format!("Queued. It goes once {agent} is back at its prompt."),
                11.5,
                faint(),
            )
            .wrapping()
            .min_w0(),
        );
    }
    div()
        .col()
        .w(260.0)
        .shrink0()
        .min_h(0.0)
        .border_left(1.0, white(0.06))
        .bg(c(look::SURFACE_PANEL))
        .role(Role::Complementary, "Comments")
        .child(list)
        .child(footer)
}

/// The rail's textareas: `rounded border-white/12 bg-surface-sunken`.
fn rail_field(s: &Screen<'_>, id: Id, name: &str, placeholder: &str) -> El {
    let border = if s.ui.focused(id) {
        white(0.24)
    } else {
        white(0.12)
    };
    div()
        .px(8.0)
        .py(6.0)
        .min_h(48.0)
        .rounded(4.0)
        .border(1.0, border)
        .bg(c(look::SURFACE_SUNKEN))
        .frames(id)
        .child(textarea(id, name, placeholder, TextStyle::ui(12.0), ink()).w_full())
}

// ---- device -------------------------------------------------------------

/// The Device's tree for the session, its header ending with the pane's
/// own controls; its screen is drawn by `super::video`.
fn device_pane(s: &Screen<'_>, session: &Session) -> El {
    let sid = session.info.id.as_str();
    let key = device_key(&s.st.points, sid);
    let title = key
        .as_deref()
        .and_then(|k| s.st.points.tree(k))
        .and_then(|t| video_part(t, sid))
        .map_or("Simulator", |(_, v)| v.title.as_str());
    let chrome = |tools: Vec<El>| {
        div()
            .row()
            .items_center()
            .gap(4.0)
            .shrink0()
            .children(tools)
            .child(controls(s, sid, PaneKind::Device, title))
    };
    let body = key
        .and_then(|key| super::points::draw_scoped(s, &key, Some(&chrome), None))
        .unwrap_or_else(|| div().grow());
    div()
        .col()
        .grow()
        .min_h(0.0)
        .min_w0()
        .clip()
        .bg(c(look::SURFACE_PANEL))
        .role(Role::Region, "Device")
        .child(body)
}

/// An open pane always keeps its button; a shell never offers one;
/// otherwise only a mobile project does.
fn should_show_button(mobile: Option<bool>, has_pane: bool, shell: bool) -> bool {
    if has_pane {
        return true;
    }
    !shell && mobile == Some(true)
}

/// The card menu's browser entry, as act and label, when the session has one.
pub(super) fn browser_entry(s: &Screen<'_>, session: &Session) -> Option<(Act, &'static str)> {
    let sid = session.info.id.as_str();
    let open = s.st.bench.pages.contains_key(sid);
    (browser_key(&s.st.points, sid).is_some() && (!session.is_shell() || open)).then(|| {
        (
            Act::Pane(PaneAct::ToggleBrowser(sid.to_owned())),
            if open {
                "Close browser"
            } else {
                "Open in browser"
            },
        )
    })
}

/// The card menu's device entry, as act and label, while the session has one.
pub(super) fn device_entry(s: &Screen<'_>, session: &Session) -> Option<(Act, &'static str)> {
    let sid = session.info.id.as_str();
    let bench = &s.st.bench;
    let has = bench.devices.contains(sid);
    let mobile = bench.mobile.get(&session.info.project_path).copied();
    let showing = has || bench.device_cards.contains(sid);
    device_key(&s.st.points, sid)
        .filter(|_| should_show_button(mobile, showing, session.is_shell()))
        .map(|_| {
            let label = if has { "Hide device" } else { "Open device" };
            (Act::Pane(PaneAct::DeviceButton(sid.to_owned())), label)
        })
}

/// The card's phone, before its window controls, while the session has one.
pub(super) fn card_buttons(
    s: &Screen<'_>,
    session: &Session,
    variant: CardVariant,
) -> Vec<(String, El)> {
    let button = |act: Act, icon: &str, label: &str, tip: &str| {
        let b = CardButton {
            id: s.acts.on("card", act),
            icon,
            label,
            tip,
            shortcut: None,
            danger: false,
        };
        card_button(s.ui, &b, variant.tip_place())
    };
    let sid = session.info.id.as_str();
    let bench = &s.st.bench;
    let mut out = Vec::new();
    let key = device_key(&s.st.points, sid);
    let has = bench.devices.contains(sid);
    let mobile = bench.mobile.get(&session.info.project_path).copied();
    let showing = has || bench.device_cards.contains(sid);
    if let Some(key) = key.filter(|_| should_show_button(mobile, showing, session.is_shell())) {
        // Before the pane opens the button holds the picker, then waits
        // while what was picked there starts, as in 0.8.
        let card = bench
            .device_cards
            .contains(sid)
            .then(|| s.st.points.tree(&key).and_then(|t| video_part(t, sid)))
            .flatten();
        let picker = card.and_then(|(node, v)| v.picker.as_ref().map(|p| (node, p)));
        let opening = card.is_some() && picker.is_none();
        let (label, tip) = match (has, opening) {
            (true, _) => ("Hide device", "Hide device"),
            (false, true) => ("Opening device", "Booting the simulator\u{2026}"),
            (false, false) => ("Open device", "Open device"),
        };
        let icon = if opening {
            "loader-circle"
        } else {
            "smartphone"
        };
        let mut b = button(
            Act::Pane(PaneAct::DeviceButton(sid.to_owned())),
            icon,
            label,
            tip,
        );
        if opening {
            b = b.disabled(true).opacity(0.5);
        }
        if let Some((node, p)) = picker {
            let bind = super::points::ScopedBinder {
                s,
                key: &key,
                chrome: None,
                label: std::cell::RefCell::new(None),
            };
            b = b.overlay(
                super::video::card_picker(&bind, node, p),
                Place::BelowStart,
                4.0,
            );
        }
        out.push((
            super::customize::pane_id(crate::panes::DEVICE, crate::panes::DEVICE_PANE),
            b,
        ));
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn numbers_print_as_javascript_does() {
        assert_eq!(js_number(6000.0), "6000");
        assert_eq!(js_number(1.5), "1.5");
        assert_eq!(js_number(-2.0), "-2");
    }

    #[test]
    fn the_device_button_follows_the_pane_then_the_project() {
        assert!(should_show_button(None, true, true));
        assert!(!should_show_button(Some(true), false, true));
        assert!(should_show_button(Some(true), false, false));
        assert!(!should_show_button(None, false, false));
    }
}

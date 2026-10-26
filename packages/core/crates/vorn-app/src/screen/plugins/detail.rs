//! One extension's page: its logo, maker and one status in the header with
//! a … menu for the rest; pictures; its accounts and what it adds beside
//! what it may use and where it comes from; then its settings.

use serde_json::Value;

use super::{act, activity_line, logo, maker, progress_bar, tag, tone_line, R};
use crate::act::Act;
use crate::plugins::list::{self, Item as Row, Maker, Showcase, Status, Tag, Verb};
use crate::plugins::{
    describe, details, place_groups, runs_line, uses, Kind, Listing, PackState, PlugAct, Source,
};
use crate::points::PointAct;
use crate::runs::s as sv;
use crate::screen::menus::Item;
use crate::screen::{ico, white, Screen};
use crate::settings::CONNECTIONS_PAGE;
use crate::state::MenuKind;
use crate::ui::theme::color as k;
use crate::ui::{div, text, tooltip, Action, El, Id, Rgba, Role};

/// The right column's width.
const ASIDE: f32 = 220.0;

/// The left column's narrowest, below which the right one wraps under it.
const LEFT: f32 = 340.0;

/// "‹ Extensions", back to the list.
fn back(s: &Screen<'_>) -> El {
    let back = s.acts.on("plug-detail-back", act(PlugAct::Select(None)));
    let fg = k::INK_SECONDARY.mix(k::INK, s.ui.hover(back));
    div().row().child(
        div()
            .id(back)
            .on(Action::Click)
            .ring(R)
            .row()
            .items_center()
            .gap(2.0)
            .mb(16.0)
            .role(Role::Button, "Extensions")
            .child(ico("chevron-left", 13.0, 2.0, fg))
            .child(text("Extensions", 12.0, fg).line_height(18.0).silent()),
    )
}

/// What a page reads, from a listing or a built-in.
struct Page<'a> {
    id: &'a str,
    /// What `Plugins::selected` holds, which also names the … menu.
    key: String,
    name: &'a str,
    listing: Option<&'a Listing>,
    own_icon: Option<&'a Value>,
    tags: Vec<Tag>,
    maker: Maker,
    version: Option<String>,
    origin: Option<String>,
    description: &'a str,
    status: El,
    contributes: Option<&'a Value>,
    declared: Vec<String>,
    hosts: Vec<String>,
    service: Option<&'a Value>,
    showcase: Showcase,
    /// A built-in's switch, the first row of its settings.
    switch: Option<(bool, Act)>,
    preferences: Option<&'a Value>,
}

fn strings(v: Option<&Value>) -> Vec<String> {
    v.and_then(Value::as_array)
        .into_iter()
        .flatten()
        .filter_map(Value::as_str)
        .map(str::to_owned)
        .collect()
}

/// A built-in extension's page.
pub(super) fn builtin(s: &Screen<'_>, b: &Value) -> El {
    let id = sv(b, "id").unwrap_or_default();
    let on = b.get("on").and_then(Value::as_bool) == Some(true);
    let row = Row::Builtin(b);
    let st = Status::Note(if on { "On" } else { "Off" });
    let page = Page {
        id,
        key: row.key(),
        name: row.name(),
        listing: None,
        own_icon: b.get("icon"),
        tags: row.tags(),
        maker: Maker::Vorn,
        version: sv(b, "version").map(str::to_owned),
        origin: Some("Built in".to_owned()),
        description: row.description(),
        status: pill(s, id, &st),
        contributes: b.get("contributes"),
        declared: strings(b.pointer("/declares/uses")),
        hosts: strings(b.pointer("/declares/network")),
        service: b.get("service"),
        showcase: Showcase::read(b.get("showcase")),
        switch: Some((
            on,
            Act::Point(PointAct::Builtin {
                id: id.to_owned(),
                on: !on,
            }),
        )),
        preferences: b.get("preferences"),
    };
    draw(s, page, None).child_if(super::installed::off_note(s, b).map(|e| e.mt(16.0)))
}

/// A listing's page: a connector, a pack, or one in the catalog.
pub(super) fn view(s: &Screen<'_>, l: &Listing) -> El {
    let p = &s.st.plugins;
    let pack = l.pack.as_ref();
    let manifest = pack.or(l.catalog.as_ref());
    let row = Row::Listing(l);
    let page = Page {
        id: &l.id,
        key: l.key.clone(),
        name: &l.name,
        listing: Some(l),
        own_icon: None,
        tags: l.tags.clone(),
        maker: row.maker(),
        // What is on disk, not what the catalog offers; Update shows the newer one.
        version: pack
            .and_then(|p| sv(p, "version"))
            .or(l.version())
            .map(str::to_owned),
        origin: origin(l),
        description: row.description(),
        status: header_status(s, l),
        contributes: l.contributes.as_ref(),
        declared: l.permissions.clone(),
        hosts: strings(manifest.and_then(|m| m.pointer("/declares/network"))),
        service: manifest.and_then(|m| m.get("service")),
        showcase: l.showcase.clone(),
        switch: None,
        preferences: pack.and_then(|p| p.get("preferences")),
    };
    let state = p.state(l);
    let said = describe(&state);
    let mut notes = div().col().gap(6.0);
    notes = notes.child_if(progress_bar(&said));
    if let (Some(detail), false) = (
        said.detail.as_deref(),
        matches!(state, PackState::Installed { .. }),
    ) {
        notes = notes.child(tone_line(said.tone, detail, 12.0));
    }
    notes = notes.child_if(activity_line(s, &l.id));
    draw(s, page, Some(notes))
}

/// Where a listing came from, for the header's meta line.
fn origin(l: &Listing) -> Option<String> {
    let Some(pack) = l.pack.as_ref() else {
        return match l.source {
            Source::Builtin => Some("Built in".to_owned()),
            Source::Mcp => Some("MCP server".to_owned()),
            _ => None,
        };
    };
    if pack.get("linked") == Some(&Value::Bool(true)) {
        return Some("Linked, edits are live".to_owned());
    }
    let source = pack.get("source");
    if let Some(g) = source.and_then(|v| sv(v, "git")) {
        return Some(format!("From {}", crate::plugins::short_repo(g)));
    }
    source
        .and_then(|v| sv(v, "path"))
        .map(|path| format!("Copied from {}", crate::plugins::tilde(path)))
}

/// The header's one status: what to do about it, or that it is in.
fn header_status(s: &Screen<'_>, l: &Listing) -> El {
    let p = &s.st.plugins;
    let verb = Verb::of(p, l);
    if let Verb::Busy(what) = verb {
        return pill(s, &l.id, &Status::Busy(what));
    }
    let st = list::status(p, l, None);
    if list::installed(l) || matches!(st, Some(Status::Attention(_))) {
        return match st {
            Some(Status::Update) => button(
                s,
                &l.id,
                "Update",
                Status::Update.tip(),
                PlugAct::Install(l.key.clone()),
            ),
            Some(st) => pill(s, &l.id, &st),
            None => div(),
        };
    }
    match verb {
        Verb::Install => button(
            s,
            &l.id,
            "Install",
            "Shows what it adds and asks for, then installs it",
            PlugAct::Install(l.key.clone()),
        ),
        Verb::Connect => button(
            s,
            &l.id,
            "Connect",
            "Signs in to an account it can watch",
            PlugAct::Add(l.key.clone()),
        ),
        _ => div(),
    }
}

/// The header's status pill.
fn pill(s: &Screen<'_>, id: &str, st: &Status) -> El {
    let (glyph, label, fg): (&str, String, Rgba) = match st {
        Status::Attention(_) => ("", "Needs attention".to_owned(), k::AMBER_400.rgba()),
        Status::Busy(w) => ("", w.clone(), k::INK_SECONDARY.rgba()),
        Status::Note("Off") => ("", "Off".to_owned(), k::INK_SECONDARY.rgba()),
        _ => ("check", "Installed".to_owned(), k::INK.rgba()),
    };
    let el_id = Id::new("plug-detail-status").child(id);
    let mut el = div()
        .id(el_id)
        .row()
        .items_center()
        .gap(6.0)
        .px(14.0)
        .py(6.0)
        .shrink0()
        .rounded(R)
        .bg(white(0.08))
        .role(Role::Label, label.as_str());
    if matches!(st, Status::Attention(_)) {
        el = el.child(super::dot(fg));
    } else if !glyph.is_empty() {
        el = el.child(ico(glyph, 13.0, 2.0, fg));
    }
    el = el.child(text(&label, 13.0, fg).line_height(18.0).silent());
    let tip = match st {
        Status::Attention(what) => what.as_str(),
        Status::Note("Off") => st.tip(),
        Status::Note(_) | Status::Account(_) => "On this computer and running",
        _ => st.tip(),
    };
    tooltip(s.ui, el, el_id, tip, None)
}

/// The header's one action, as a pill button.
fn button(s: &Screen<'_>, id: &str, label: &str, tip: &str, a: PlugAct) -> El {
    let bid = s.acts.on(&format!("plug-detail-do-{id}"), act(a));
    let hot = s.ui.hover(bid);
    let b = div()
        .id(bid)
        .on(Action::Click)
        .ring(R)
        .row()
        .items_center()
        .px(14.0)
        .py(6.0)
        .shrink0()
        .rounded(R)
        .bg(white(0.08 + 0.06 * hot))
        .role(Role::Button, label)
        .child(text(label, 13.0, k::INK).line_height(18.0).silent());
    tooltip(s.ui, b, bid, tip, None)
}

/// The … button, when its menu has something in it.
fn more(s: &Screen<'_>, key: &str) -> Option<El> {
    if menu_items(s, key).is_empty() {
        return None;
    }
    let kind = MenuKind::ExtMenu(key.to_owned());
    let open = s.st.menu.as_ref().is_some_and(|m| m.kind == kind);
    let id = s.acts.on("plug-detail-more", Act::Open(kind));
    let hot = if open { 1.0 } else { s.ui.hover(id) };
    let b = div()
        .id(id)
        .on(Action::Click)
        .ring(R)
        .size(30.0, 30.0)
        .shrink0()
        .center()
        .rounded(R)
        .bg(white(0.06 * hot))
        .role(Role::Button, "More actions")
        .has_popup()
        .expanded(open)
        .child(ico(
            "ellipsis",
            15.0,
            2.0,
            k::INK_SECONDARY.mix(k::INK, hot),
        ));
    Some(tooltip(s.ui, b, id, "More actions", None))
}

/// The … menu of the page `key` names: what the header does not.
pub(in crate::screen) fn menu_items(s: &Screen<'_>, key: &str) -> Vec<Item> {
    let p = &s.st.plugins;
    if let Some(id) = key.strip_prefix(list::BUILTIN) {
        let Some(b) =
            s.st.points
                .builtins
                .iter()
                .find(|b| sv(b, "id") == Some(id))
        else {
            return Vec::new();
        };
        let on = b.get("on").and_then(Value::as_bool) == Some(true);
        let act = Act::Point(PointAct::Builtin {
            id: id.to_owned(),
            on: !on,
        });
        return vec![Item::new(
            "power",
            if on { "Turn off" } else { "Turn on" },
            act,
        )];
    }
    let Some(l) = p.listing(key) else {
        return Vec::new();
    };
    let state = p.state(l);
    let busy = p.busy(&l.id);
    let mut out = Vec::new();
    if l.implicit || l.connected > 0 {
        out.push(Item::new(
            "workflow",
            "Use in a workflow",
            act(PlugAct::Use),
        ));
    }
    if let Some(pack) = l.pack.as_ref() {
        if pack.get("sandboxAvailable") == Some(&Value::Bool(true)) {
            let on = pack.get("sandboxed") == Some(&Value::Bool(true));
            out.push(
                Item::new(
                    "shield",
                    "Sandbox",
                    act(PlugAct::Sandbox(l.id.clone(), !on)),
                )
                .detail(if on { "On" } else { "Off" }),
            );
        }
        if let Some(path) = sv(pack, "path").filter(|p| !p.is_empty()) {
            out.push(Item::new(
                "folder-open",
                "Show folder",
                act(PlugAct::ShowFolder(path.to_owned())),
            ));
        }
    }
    if let PackState::Installed {
        previous: Some(v), ..
    } = &state
    {
        out.push(
            Item::new("undo-2", "Roll back", act(PlugAct::Rollback(l.id.clone())))
                .detail(format!("v{v}"))
                .disabled(busy),
        );
    }
    if let Some(pkg) = l.catalog.as_ref().and_then(|c| sv(c, "packageName")) {
        out.push(Item::new(
            "external-link",
            "View on npm",
            act(PlugAct::OpenUrl(format!(
                "https://www.npmjs.com/package/{pkg}"
            ))),
        ));
    }
    if let Some(pack) = l.pack.as_ref() {
        let linked = pack.get("linked") == Some(&Value::Bool(true));
        let mut it = Item::new(
            "trash-2",
            if linked { "Unlink" } else { "Uninstall" },
            act(PlugAct::Remove(l.id.clone())),
        )
        .disabled(busy);
        it.danger = true;
        if !out.is_empty() {
            it = it.sep();
        }
        out.push(it);
    }
    out
}

fn draw(s: &Screen<'_>, mut page: Page<'_>, notes: Option<El>) -> El {
    let status = std::mem::replace(&mut page.status, div());
    let place = format!("plug-detail-{}", page.id);
    let mut title = div().row().items_center().gap(8.0).min_w0().child(
        text(page.name, 22.0, k::INK)
            .weight(600)
            .line_height(28.0)
            .role(Role::Heading, page.name),
    );
    for t in &page.tags {
        title = title.child(tag(s, &place, *t));
    }
    let mut meta = div()
        .row()
        .items_center()
        .min_w0()
        .gap(6.0)
        .child(maker(&page.maker, 12.5));
    for part in page
        .version
        .as_deref()
        .map(|v| format!("v{v}"))
        .into_iter()
        .chain(page.origin.clone())
    {
        meta = meta
            .child(
                text("\u{b7}", 12.5, k::INK_FAINT)
                    .line_height(18.0)
                    .silent(),
            )
            .child(
                text(part, 12.5, k::INK_SECONDARY)
                    .line_height(18.0)
                    .truncate()
                    .silent(),
            );
    }
    let head = div()
        .row()
        .items_center()
        .gap(16.0)
        .child(logo(
            s,
            page.id,
            page.name,
            page.listing,
            page.own_icon,
            64.0,
        ))
        .child(
            div()
                .col()
                .gap(4.0)
                .grow()
                .min_w0()
                .child(title)
                .child(meta),
        )
        .child(status)
        .child_if(more(s, &page.key));
    let mut col = div().col().child(back(s)).child(head);
    if !page.description.is_empty() {
        col = col.child(
            text(page.description, 13.0, k::INK_SECONDARY)
                .line_height(20.0)
                .mt(16.0)
                .wrapping(),
        );
    }
    col = col.child_if(notes.map(|n| n.mt(12.0)));
    col = col.child_if(screenshots(s, &page.showcase.screenshots));
    let left = div()
        .col()
        .flex(1.0)
        .min_w(LEFT)
        .child_if(page.listing.and_then(|l| accounts(s, l)))
        .child(adds(s, &page));
    let right = div()
        .col()
        .w(ASIDE)
        .shrink0()
        .child(may_use(&page))
        .child(about(s, &page));
    // Narrow windows put the right column under the left.
    col = col.child(
        div()
            .row()
            .wrap_children()
            .items_start()
            .gap(28.0)
            .child(left)
            .child(right),
    );
    col.child_if(settings(s, &page))
}

/// The pictures, 16:10, and how many more.
fn screenshots(s: &Screen<'_>, urls: &[String]) -> Option<El> {
    const SHOWN: usize = 2;
    if urls.is_empty() {
        return None;
    }
    let (w, h) = (236.0, 147.5);
    let mut row = div().row().gap(12.0).mt(20.0);
    for url in urls.iter().take(SHOWN) {
        let body = match s.shots.get(url) {
            Some(img) => crate::ui::image(img, w, h),
            None => ico("image", 18.0, 1.75, k::INK_FAINT),
        };
        row = row.child(
            div()
                .size(w, h)
                .shrink0()
                .center()
                .clip()
                .rounded(R)
                .bg(white(0.03))
                .border(1.0, white(0.08))
                .role(Role::Image, "Screenshot")
                .child(body),
        );
    }
    if urls.len() > SHOWN {
        row = row.child(
            div()
                .size(120.0, h)
                .shrink0()
                .center()
                .rounded(R)
                .border(1.0, white(0.12))
                .dashed()
                .child(
                    text(format!("+{}", urls.len() - SHOWN), 13.0, k::INK_SECONDARY)
                        .line_height(18.0)
                        .silent(),
                ),
        );
    }
    Some(row)
}

/// A column's small heading.
fn heading(label: &str) -> El {
    text(label, 13.0, k::INK)
        .weight(600)
        .line_height(18.0)
        .mt(24.0)
        .mb(8.0)
        .role(Role::Heading, label)
}

/// Whether a listing signs in to accounts.
fn signs_in(l: &Listing) -> bool {
    l.kind == Kind::Connector || l.tags.contains(&Tag::Connection)
}

/// Its accounts, drawn by Connectors' page for this one, and a way to add one.
fn accounts(s: &Screen<'_>, l: &Listing) -> Option<El> {
    if !signs_in(l) {
        return None;
    }
    let p = &s.st.plugins;
    let state = p.state(l);
    let key = crate::points::scoped(CONNECTIONS_PAGE, &l.id);
    let mut col = div()
        .col()
        .child(heading("Accounts"))
        .child_if(crate::screen::points::draw_scoped(s, &key, None, None));
    if l.implicit {
        col = col.child(quiet("Works without signing in."));
    } else if matches!(l.source, Source::Builtin | Source::Mcp) && l.can_add(&state) {
        let label = if l.connected > 0 {
            "Connect another account"
        } else {
            "Connect an account"
        };
        let id = s
            .acts
            .on("plug-detail-connect", act(PlugAct::Add(l.key.clone())));
        let fg = k::INK_SECONDARY.mix(k::INK, s.ui.hover(id));
        let b = div()
            .id(id)
            .on(Action::Click)
            .ring(R)
            .row()
            .items_center()
            .gap(6.0)
            .py(6.0)
            .role(Role::Button, label)
            .child(ico("plus", 13.0, 2.0, fg))
            .child(text(label, 12.5, fg).line_height(18.0).silent());
        col = col.child(div().row().child(tooltip(
            s.ui,
            b,
            id,
            "Signs in to another account for workflows to use",
            None,
        )));
    }
    if let Some(r) = l.rung {
        col = col.child(quiet(r.detail()).mt(4.0));
    }
    Some(col)
}

/// What it adds: where it shows, what starts a workflow, what a step can do.
fn adds(s: &Screen<'_>, page: &Page<'_>) -> El {
    // An agent or a theme is a section of its manifest, not a contributed place.
    let mut terms: Vec<El> = [(Tag::Agent, "Agent"), (Tag::Theme, "Theme")]
        .into_iter()
        .filter(|(t, _)| page.tags.contains(t))
        .map(|(_, group)| term(group, page.name))
        .collect();
    terms.extend(
        place_groups(page.contributes)
            .into_iter()
            .map(|(group, items)| term(group, &items.join(", "))),
    );
    if let Some(l) = page.listing.filter(|l| l.kind == Kind::Connector) {
        let d = details(l, &s.st.plugins.builtins);
        let names = |v: &[(String, Option<String>)]| {
            v.iter()
                .map(|(t, _)| t.as_str())
                .collect::<Vec<_>>()
                .join(", ")
        };
        if !d.triggers.is_empty() {
            terms.push(term("Starts on", &names(&d.triggers)));
        }
        if !d.actions.is_empty() {
            terms.push(term("Steps", &names(&d.actions)));
        }
        if !d.settings.is_empty() {
            let needs: Vec<String> = d
                .settings
                .iter()
                .map(|(name, required, _)| {
                    if *required {
                        name.clone()
                    } else {
                        format!("{name} (optional)")
                    }
                })
                .collect();
            terms.push(term("Needs", &needs.join(", ")));
        }
        if !d.known {
            terms.push(quiet("Add it to see what it offers."));
        }
    }
    if terms.is_empty() {
        terms.push(quiet("Nothing yet."));
    }
    div()
        .col()
        .child(heading("What it adds"))
        .child(div().col().gap(8.0).children(terms))
}

/// What it may reach, and whether Vorn holds it to that.
fn may_use(page: &Page<'_>) -> El {
    let rows: Vec<El> = uses(&page.declared, &page.hosts)
        .into_iter()
        .map(|u| {
            let by = if u.enforced { "Enforced" } else { "Declared" };
            div()
                .col()
                .child(text(u.what, 12.5, k::INK).line_height(18.0))
                .child(
                    text(format!("{} \u{b7} {by}", u.how), 12.0, k::INK_SECONDARY)
                        .line_height(17.0)
                        .wrapping(),
                )
        })
        .collect();
    let rows = if rows.is_empty() {
        vec![quiet("Nothing beyond what it adds.")]
    } else {
        rows
    };
    div()
        .col()
        .child(heading("May use"))
        .child(div().col().gap(8.0).children(rows))
}

/// Where it comes from, its homepage, what's new and what it runs.
fn about(s: &Screen<'_>, page: &Page<'_>) -> El {
    let mut rows = Vec::new();
    let pack = page.listing.and_then(|l| l.pack.as_ref());
    let git = pack
        .and_then(|p| p.pointer("/source/git"))
        .and_then(Value::as_str);
    if let Some(g) = git {
        rows.push(link(s, "Source", &crate::plugins::short_repo(g), g));
    }
    if let Some(home) = page.showcase.homepage.as_deref() {
        let shown = home
            .split_once("://")
            .map_or(home, |(_, rest)| rest)
            .trim_end_matches('/');
        rows.push(link(s, "Homepage", shown, home));
    }
    if let Some(new) = page.showcase.whats_new.as_deref() {
        rows.push(
            div()
                .col()
                .child(text("What's new", 12.0, k::INK_FAINT).line_height(17.0))
                .child(
                    text(new, 12.5, k::INK_SECONDARY)
                        .line_height(18.0)
                        .wrapping(),
                ),
        );
    }
    if let Some(path) = pack.and_then(|p| sv(p, "path")).filter(|p| !p.is_empty()) {
        rows.push(
            div()
                .col()
                .child(text("On disk", 12.0, k::INK_FAINT).line_height(17.0))
                .child(
                    text(crate::plugins::tilde(path), 11.5, k::INK_SECONDARY)
                        .mono()
                        .line_height(17.0)
                        .wrapping(),
                ),
        );
    }
    if let Some(runs) = runs_line(page.service) {
        rows.push(
            div()
                .col()
                .child(text("Runs", 12.0, k::INK_FAINT).line_height(17.0))
                .child(
                    text(runs, 11.5, k::INK_SECONDARY)
                        .mono()
                        .line_height(17.0)
                        .wrapping(),
                ),
        );
    }
    if rows.is_empty() {
        return div();
    }
    div()
        .col()
        .child(heading("About"))
        .child(div().col().gap(10.0).children(rows))
}

/// A term over a link that opens `url`.
fn link(s: &Screen<'_>, label: &str, shown: &str, url: &str) -> El {
    let https = if url.starts_with("http") {
        url.to_owned()
    } else {
        format!(
            "https://{}",
            url.trim_start_matches("git@").replacen(':', "/", 1)
        )
    };
    let id = s.acts.on(
        &format!("plug-detail-link-{label}"),
        act(PlugAct::OpenUrl(https)),
    );
    let fg = k::INK_SECONDARY.mix(k::INK, s.ui.hover(id));
    div()
        .col()
        .child(text(label, 12.0, k::INK_FAINT).line_height(17.0))
        .child(
            div().row().child(
                div()
                    .id(id)
                    .on(Action::Click)
                    .ring(R)
                    .row()
                    .items_center()
                    .gap(4.0)
                    .min_w0()
                    .role(Role::Link, label)
                    .child(text(shown, 12.5, fg).line_height(18.0).truncate().min_w0())
                    .child(ico("external-link", 11.0, 2.0, fg)),
            ),
        )
}

/// Its switch and preferences on a card.
fn settings(s: &Screen<'_>, page: &Page<'_>) -> Option<El> {
    let switch = page.switch.clone().map(|(on, a)| {
        let place = format!("ext-switch-{}", page.id);
        let busy = s.st.points.switching.is_some();
        let toggle = crate::screen::settings::toggle(s, &place, on, a, busy, page.name);
        crate::screen::settings::card_row(
            s,
            None,
            "On",
            Some("Turns off everything it adds until you turn it back on"),
            toggle.shrink0(),
        )
    });
    let rows: Vec<El> = switch
        .into_iter()
        .chain(
            page.preferences
                .and_then(Value::as_array)
                .into_iter()
                .flatten()
                .filter(|p| !shown_elsewhere(p))
                .filter_map(|p| preference(s, page.id, p)),
        )
        .collect();
    (!rows.is_empty()).then(|| {
        div()
            .col()
            .child(crate::screen::settings::section("Settings"))
            .child(crate::screen::settings::card(rows))
    })
}

/// A group's term beside what it holds.
fn term(group: &str, items: &str) -> El {
    div()
        .row()
        .items_start()
        .gap(12.0)
        .child(
            text(group, 12.5, k::INK_FAINT)
                .line_height(18.0)
                .w(96.0)
                .shrink0(),
        )
        .child(
            text(items, 12.5, k::INK_SECONDARY)
                .line_height(18.0)
                .wrapping()
                .grow()
                .min_w0(),
        )
}

fn quiet(t: &str) -> El {
    text(t, 12.0, k::INK_FAINT).line_height(18.0).wrapping()
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

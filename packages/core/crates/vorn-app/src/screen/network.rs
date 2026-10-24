//! The Remote Access page: sharing this machine, how others reach it, the
//! devices paired with it, and the Add a device sheet.

use web_time::Instant;

use super::settings::{
    card, card_row, glyph, glyph_icon, header, icon_button, row_tip_id, section, toggle,
};
use super::{c, ico, white, Screen};
use crate::act::Act;
use crate::look;
use crate::network::{
    format_remaining, grouped, share_line, shown_code, AddressKind, DeviceKind, DeviceToken,
    NetAct, Network, DEVICE_NAME,
};
use crate::state::MenuKind;
use crate::ui::theme::color::AMBER_500;
use crate::ui::{div, field, shape, text, tooltip, Action, El, Id, Rgba, Role, TextStyle};

#[cfg(target_os = "macos")]
const SHARE: &str = "Share this Mac";
#[cfg(not(target_os = "macos"))]
const SHARE: &str = "Share this computer";

/// What each row, tile and button says while it is hovered.
mod tip {
    #[cfg(target_os = "macos")]
    pub const SHARE: &str = "Lets your phone and other computers reach the sessions on this Mac.";
    #[cfg(not(target_os = "macos"))]
    pub const SHARE: &str =
        "Lets your phone and other computers reach the sessions on this computer.";
    pub const TAILSCALE: &str =
        "Encrypted. Works wherever your devices are signed in to Tailscale.";
    pub const LAN: &str = "Not encrypted. Works only on the same home or office network.";
    pub const ADD: &str = "Pair a phone: scan the QR code with the Vorn app, or type the code.";
    pub const DEVICE: &str = "Last time this device connected.";
    pub const MORE: &str = "More";
    pub const REMOVE: &str = "Signs this device out now. It must pair again to reconnect.";
    #[cfg(target_os = "macos")]
    pub const BROWSER: &str =
        "Address to open Vorn from a browser on a device that can reach this Mac.";
    #[cfg(not(target_os = "macos"))]
    pub const BROWSER: &str =
        "Address to open Vorn from a browser on a device that can reach this computer.";
    pub const ADVANCED: &str =
        "Certificate fingerprint, server address and tokens for setups by hand.";
    pub const FINGERPRINT: &str =
        "The certificate the address presents. Compare it when you pair by hand.";
    pub const ADDRESSES: &str = "Every address Vorn answers on, for typing into another device.";
    pub const TOKEN: &str =
        "Makes a token another computer uses to connect by hand. It is shown once.";
    pub const COPY: &str = "Copy";
}

pub(super) fn page(s: &Screen<'_>) -> El {
    let n = &s.st.network;
    let on = sharing(s);
    div()
        .col()
        .child(header("Remote Access", "", None))
        .child(share_card(s, n, on))
        .child(section("Devices"))
        .child(devices(s, n))
        .child(reach_card(s, n, on).mt(24.0))
        .child_if(n.error.as_deref().map(|e| {
            text(e, 12.0, c(look::DANGER))
                .line_height(16.0)
                .mt(12.0)
                .wrapping()
        }))
}

fn sharing(s: &Screen<'_>) -> bool {
    s.store
        .config
        .pointer("/defaults/networkAccessEnabled")
        .and_then(serde_json::Value::as_bool)
        .unwrap_or(false)
}

fn net(place: &str, s: &Screen<'_>, act: NetAct) -> Id {
    s.acts.on(place, Act::Net(act))
}

/// The switch, its state in a line, and how others reach this machine.
fn share_card(s: &Screen<'_>, n: &Network, on: bool) -> El {
    let ink = if on {
        c(look::GREEN_400)
    } else {
        c(look::GRAY_500)
    };
    let switch = toggle(
        s,
        "net-share",
        on,
        Act::Net(NetAct::Share(!on)),
        false,
        SHARE,
    );
    let top = div()
        .row()
        .items_center()
        .justify_between()
        .gap(16.0)
        .child(
            div()
                .col()
                .min_w0()
                .child(
                    text(SHARE, 14.0, c(look::GRAY_200))
                        .weight(500)
                        .line_height(20.0)
                        .role(Role::Label, SHARE),
                )
                .child(
                    text(share_line(on, n.active().len()), 12.0, ink)
                        .line_height(16.0)
                        .mt(2.0)
                        .silent(),
                ),
        )
        .child(switch);
    let mut body = div()
        .col()
        .gap(12.0)
        .p(14.0)
        .child(tooltip(s.ui, top, row_tip_id(SHARE), tip::SHARE, None))
        .child(
            div()
                .row()
                .gap(8.0)
                .role(Role::RadioGroup, "How others reach it")
                .child(route_tile(s, n, AddressKind::Tailnet))
                .child(route_tile(s, n, AddressKind::Lan)),
        );
    if n.confirming {
        body = body.child(confirm(s));
    }
    card(vec![body])
}

/// One way in: its mark, its name and the trade-off in a line; the one the
/// code and the browser address use is ringed.
fn route_tile(s: &Screen<'_>, n: &Network, kind: AddressKind) -> El {
    let installed = n.status.as_ref().is_some_and(|t| t.installed);
    let (icon, title, line, tip) = match kind {
        AddressKind::Tailnet => (
            "lock",
            "Tailscale",
            match (n.tailnet(), installed) {
                (true, _) => "Encrypted · anywhere your devices are signed in",
                (false, true) => "Not running · start Tailscale to use it",
                (false, false) => "Not installed · get Tailscale",
            },
            tip::TAILSCALE,
        ),
        AddressKind::Lan => (
            "wifi",
            "This network",
            "Not encrypted · home or office only",
            tip::LAN,
        ),
    };
    let id = net("net-route", s, NetAct::Route(kind));
    let hot = s.ui.hover(id);
    let on = n.route() == kind;
    // The ring's extra pixel comes out of the padding, so nothing moves.
    let (edge, width, pad) = if on {
        (c(look::BRONZO), 2.0, 11.0)
    } else {
        (white(0.1).mix(white(0.18), hot), 1.0, 12.0)
    };
    let mark = if kind == AddressKind::Tailnet && n.tailnet() {
        c(look::GREEN_400)
    } else {
        c(look::GRAY_400)
    };
    let el = div()
        .on(Action::Click)
        .ring(4.0)
        .flex(1.0)
        .col()
        .gap(4.0)
        .p(pad)
        .rounded(4.0)
        .border(width, edge)
        .bg(white(0.0).mix(white(0.03), hot))
        .role(Role::RadioButton, title)
        .selected(on)
        .child(
            div()
                .row()
                .items_center()
                .gap(8.0)
                .child(ico(icon, 14.0, 2.0, mark))
                .child(
                    text(title, 13.0, c(look::GRAY_200))
                        .weight(500)
                        .line_height(18.0)
                        .silent(),
                ),
        )
        .child(
            text(line, 11.0, c(look::GRAY_500))
                .line_height(16.0)
                .wrapping()
                .silent(),
        );
    tooltip(s.ui, el, id, tip, None)
}

/// What the wide bind costs, before it happens without a tailnet.
fn confirm(s: &Screen<'_>) -> El {
    let para = |t: &str| {
        text(t, 12.0, c(look::GRAY_400))
            .line_height(16.0)
            .wrapping()
            .silent()
    };
    div()
        .col()
        .p(12.0)
        .rounded(4.0)
        .border(1.0, AMBER_500.alpha(0.2))
        .bg(AMBER_500.alpha(0.04))
        .child(
            text("Share without Tailscale?", 13.0, c(look::AMBER_400))
                .weight(500)
                .line_height(18.0)
                .mb(4.0),
        )
        .child(para(
            "Anything on this network can reach Vorn over plain HTTP. A device's token still \
             gates it, but it travels unencrypted, and a token runs commands here. Fine on a \
             home or office network you trust; not on public wifi.",
        ))
        .child(
            div()
                .row()
                .gap(8.0)
                .mt(10.0)
                .child(outline(
                    s,
                    "net-share",
                    "Share anyway",
                    NetAct::EnableAnyway,
                ))
                .child(ghost(s, "net-share", "Cancel", NetAct::CancelShare)),
        )
}

/// Phones and computers paired with this machine, then the way to add one.
fn devices(s: &Screen<'_>, n: &Network) -> El {
    let now = crate::state::now_ms();
    let mut rows: Vec<El> = n
        .active()
        .into_iter()
        .map(|t| device_row(s, n, t, now))
        .collect();
    rows.push(add_row(s));
    card(rows)
}

fn device_row(s: &Screen<'_>, n: &Network, t: &DeviceToken, now: f64) -> El {
    let asking = n.confirm_revoke.as_deref() == Some(t.id.as_str());
    let icon = match t.kind() {
        DeviceKind::Phone => "smartphone",
        DeviceKind::Computer => "monitor",
    };
    let line = if asking {
        "Remove it? It must pair again to reconnect.".to_owned()
    } else {
        t.seen(now)
    };
    let menu = Act::Open(MenuKind::Device(t.id.clone()));
    let controls = if asking {
        div()
            .row()
            .gap(6.0)
            .shrink0()
            .child(ghost(s, "net-revoke", "Cancel", NetAct::CancelRevoke))
            .child(remove(s, &t.id))
    } else {
        let id = s.acts.on("net-device", menu.clone());
        icon_button(s, id, glyph::ELLIPSIS, "More", tip::MORE)
    };
    let row = div()
        .row()
        .items_center()
        .gap(12.0)
        .min_h(52.0)
        .px(14.0)
        .py(8.0)
        .child(ico(icon, 18.0, 1.75, c(look::GRAY_400)))
        .child(
            div()
                .col()
                .grow()
                .min_w0()
                .child(
                    text(t.name.as_str(), 14.0, c(look::GRAY_200))
                        .line_height(20.0)
                        .truncate()
                        .role(Role::Label, t.name.as_str()),
                )
                .child(
                    text(line, 11.0, c(look::GRAY_500))
                        .line_height(16.0)
                        .silent(),
                ),
        )
        .child(controls);
    let row = s
        .acts
        .context(row, Id::new("net-device-row").child(t.id.as_str()), menu);
    tooltip(
        s.ui,
        row,
        Id::new("net-device-row").child(t.id.as_str()),
        tip::DEVICE,
        None,
    )
}

/// The device menu: Remove, which asks on the row first.
pub(crate) fn device_items(id: &str) -> Vec<crate::screen::menus::Item> {
    let mut remove = crate::screen::menus::Item::new(
        "trash-2",
        "Remove\u{2026}",
        Act::Net(NetAct::AskRevoke(id.to_owned())),
    );
    remove.danger = true;
    vec![remove]
}

/// "Add a device", the last row of the list.
fn add_row(s: &Screen<'_>) -> El {
    let id = net("net-add", s, NetAct::AddDevice);
    let hot = s.ui.hover(id);
    let row = div()
        .on(Action::Click)
        .ring(4.0)
        .row()
        .items_center()
        .justify_between()
        .gap(12.0)
        .min_h(48.0)
        .px(14.0)
        .py(8.0)
        .bg(white(0.0).mix(white(0.03), hot))
        .role(Role::Button, "Add a device")
        .child(
            div()
                .row()
                .items_center()
                .gap(12.0)
                .child(div().w(18.0).center().child(ico(
                    "plus",
                    16.0,
                    2.0,
                    c(look::GRAY_400).mix(c(look::WHITE), hot),
                )))
                .child(
                    text("Add a device", 14.0, c(look::GRAY_200))
                        .line_height(20.0)
                        .silent(),
                ),
        )
        .child(
            div()
                .row()
                .items_center()
                .gap(4.0)
                .child(
                    text("Phone", 12.0, c(look::GRAY_500))
                        .line_height(16.0)
                        .silent(),
                )
                .child(glyph_icon(glyph::CHEVRON_RIGHT, 12.0, c(look::GRAY_500))),
        );
    tooltip(s.ui, row, id, tip::ADD, None)
}

/// Where a browser opens Vorn, and what is under Advanced.
fn reach_card(s: &Screen<'_>, n: &Network, on: bool) -> El {
    let url = if on {
        n.pair_url().unwrap_or(s.web_url)
    } else {
        s.web_url
    };
    let address = div()
        .row()
        .items_center()
        .gap(8.0)
        .min_w0()
        .child(
            text(url, 11.0, c(look::GRAY_400))
                .mono()
                .line_height(16.0)
                .truncate(),
        )
        .child(copy_pill(s, n, url));
    let mut rows = vec![card_row(
        s,
        None,
        "Open in a browser",
        Some(tip::BROWSER),
        address,
    )];
    let id = net("net-advanced", s, NetAct::ToggleAdvanced);
    let hot = s.ui.hover(id);
    let chevron = if n.advanced {
        glyph::CHEVRON_DOWN
    } else {
        glyph::CHEVRON_RIGHT
    };
    let advanced = div()
        .on(Action::Click)
        .ring(4.0)
        .row()
        .items_center()
        .justify_between()
        .gap(16.0)
        .min_h(48.0)
        .px(14.0)
        .py(8.0)
        .bg(white(0.0).mix(white(0.03), hot))
        .role(Role::Button, "Advanced")
        .expanded(n.advanced)
        .child(
            text("Advanced", 13.0, c(look::GRAY_200))
                .line_height(18.0)
                .silent(),
        )
        .child(
            div()
                .row()
                .items_center()
                .gap(4.0)
                .child(
                    text("Fingerprint, tokens", 12.0, c(look::GRAY_500))
                        .line_height(16.0)
                        .silent(),
                )
                .child(glyph_icon(chevron, 12.0, c(look::GRAY_500))),
        );
    rows.push(tooltip(s.ui, advanced, id, tip::ADVANCED, None));
    if n.advanced {
        rows.push(advanced_panel(s, n));
    }
    card(rows)
}

/// The fingerprint, every address, and a token for another computer.
fn advanced_panel(s: &Screen<'_>, n: &Network) -> El {
    let sub = |label: &str, tip: &str, body: El| {
        let el = div()
            .col()
            .gap(4.0)
            .child(
                text(label, 12.0, c(look::GRAY_500))
                    .line_height(16.0)
                    .role(Role::Label, label),
            )
            .child(body);
        tooltip(s.ui, el, Id::new("net-adv-tip").child(label), tip, None)
    };
    let mono = |t: &str| {
        div().row().items_center().gap(8.0).min_w0().child(
            text(t, 11.0, c(look::GRAY_300))
                .mono()
                .line_height(16.0)
                .truncate(),
        )
    };
    let mut col = div().col().gap(16.0).px(14.0).py(12.0);
    if let Some(fp) = n.fingerprint() {
        col = col.child(sub(
            "Certificate fingerprint",
            tip::FINGERPRINT,
            mono(&grouped(fp)).child(copy_pill(s, n, fp)),
        ));
    }
    let mut urls: Vec<&str> = n
        .reachable
        .iter()
        .flat_map(|r| r.urls.iter().map(String::as_str))
        .collect();
    if !urls.contains(&s.web_url) {
        urls.push(s.web_url);
    }
    let list = div()
        .col()
        .gap(6.0)
        .children(urls.iter().map(|u| mono(u).child(copy_pill(s, n, u))));
    col = col.child(sub("Server address", tip::ADDRESSES, list));
    let token = if let Some(m) = &n.minted {
        minted(s, n, &m.name, &m.plaintext)
    } else if n.naming {
        div()
            .row()
            .items_center()
            .gap(8.0)
            .child(input(s, DEVICE_NAME, "Computer name", "Studio Mac mini").grow())
            .child(outline(s, "net-token", "Create token", NetAct::CreateToken))
            .child(ghost(s, "net-token", "Cancel", NetAct::CancelToken))
    } else {
        div().row().child(outline(
            s,
            "net-name-token",
            "Create token for another computer",
            NetAct::NameToken,
        ))
    };
    col.child(sub("Tokens", tip::TOKEN, token))
}

/// The one moment a token's plaintext is readable.
fn minted(s: &Screen<'_>, n: &Network, name: &str, plaintext: &str) -> El {
    div()
        .col()
        .gap(8.0)
        .p(12.0)
        .rounded(4.0)
        .border(1.0, white(0.08))
        .bg(white(0.03))
        .child(
            text(
                format!("Token for \u{201c}{name}\u{201d}. Copy it now; it is not shown again."),
                12.0,
                c(look::GRAY_300),
            )
            .line_height(16.0)
            .wrapping()
            .silent(),
        )
        .child(
            div()
                .row()
                .items_center()
                .gap(8.0)
                .child(
                    div().row().grow().min_w0().child(
                        text(plaintext, 12.0, c(look::INK))
                            .mono()
                            .line_height(16.0)
                            .truncate(),
                    ),
                )
                .child(copy_pill(s, n, plaintext))
                .child(ghost(s, "net-minted", "Done", NetAct::DoneMinted)),
        )
}

/// The Add a device sheet: a QR code and the code to type, then the phone
/// that used it, to approve or deny.
pub(super) fn sheet(s: &Screen<'_>) -> El {
    let n = &s.st.network;
    let faint = |t: &str, size: f32| {
        text(t, size, c(look::GRAY_500))
            .line_height(16.0)
            .wrapping()
            .silent()
    };
    let mut col = div()
        .col()
        .items_center()
        .gap(12.0)
        .w(320.0)
        .p(20.0)
        .rounded(12.0)
        .border(1.0, white(0.08))
        .bg(c(look::SURFACE_OVERLAY))
        .role(Role::Dialog, "Add a device")
        .child(
            text("Add a device", 15.0, c(look::WHITE))
                .weight(600)
                .line_height(20.0)
                .role(Role::Heading, "Add a device"),
        );
    let code = n.adding.as_ref().and_then(Option::as_ref);
    let left = code.map_or(0, |c| c.remaining(now_ms()));
    if let Some(r) = &n.asking {
        let name = if r.device_name.is_empty() {
            "A phone"
        } else {
            r.device_name.as_str()
        };
        col = col
            .child(ico("smartphone", 24.0, 1.75, c(look::GRAY_300)))
            .child(
                text(format!("{name} wants to pair"), 14.0, c(look::GRAY_200))
                    .weight(500)
                    .line_height(20.0),
            )
            .child_if((!r.address.is_empty()).then(|| {
                text(r.address.as_str(), 11.0, c(look::GRAY_500))
                    .mono()
                    .line_height(16.0)
                    .silent()
            }))
            .child(faint(
                "It can run terminals here. Approve only a phone you are holding.",
                12.0,
            ))
            .child(
                div()
                    .row()
                    .gap(8.0)
                    .child(ghost(s, "net-decide", "Deny", NetAct::Decide(false)))
                    .child(outline(s, "net-decide", "Approve", NetAct::Decide(true))),
            );
    } else if !n.can_scan() {
        col = col.child(faint(
            &format!("Turn on {SHARE} first, so your phone can reach it."),
            12.0,
        ));
    } else if let (Some(code), Some((_, svg))) = (code.filter(|_| left > 0), &n.qr) {
        col = col
            .child(qr(svg))
            .child(faint("Open the Vorn app on your phone and scan", 12.0))
            .child(faint("or type", 11.0))
            .child(
                text(shown_code(&code.code), 20.0, c(look::GRAY_200))
                    .mono()
                    .tracking(0.2)
                    .line_height(28.0)
                    .role(Role::Label, "Pairing code"),
            )
            .child(
                text(
                    format!("expires in {}", format_remaining(left)),
                    11.0,
                    c(look::GRAY_500),
                )
                .mono()
                .line_height(16.0)
                .silent(),
            );
    } else if code.is_some() {
        col = col.child(faint("That code expired.", 12.0)).child(outline(
            s,
            "net-expired",
            "Show another",
            NetAct::ShowAnother,
        ));
    } else {
        col = col.child(
            div()
                .size(160.0, 160.0)
                .center()
                .rounded(12.0)
                .bg(white(0.03))
                .child(faint("Getting a code\u{2026}", 12.0)),
        );
    }
    col = col.child(ghost(s, "net-close", "Cancel", NetAct::CloseAdding));
    let panel = s.acts.click(col, "dialog", Act::Nothing);
    super::dialogs::backdrop(s, 0.5).center().child(panel)
}

/// The code a phone scans, dark on light whatever the theme.
fn qr(svg: &str) -> El {
    // literal-colour: a QR code reads only dark on light, in every theme.
    let (paper, ink) = (
        Rgba([0.98, 0.976, 0.969, 1.0]),
        Rgba([0.05, 0.05, 0.06, 1.0]),
    );
    div()
        .size(160.0, 160.0)
        .p(10.0)
        .rounded(12.0)
        .bg(paper)
        .role(Role::Image, "QR code to scan")
        .child(shape(svg, 140.0, 140.0, ink))
}

fn now_ms() -> u64 {
    crate::state::now_ms().max(0.0) as u64
}

/// A name field on `white/[0.04]`.
fn input(s: &Screen<'_>, id: Id, name: &str, placeholder: &str) -> El {
    let focus = if s.ui.focused(id) { 1.0 } else { 0.0 };
    let style = TextStyle {
        line_height: 16.0,
        ..TextStyle::ui(12.0)
    };
    field(id, name, placeholder, style, c(look::GRAY_200))
        .px(10.0)
        .py(6.0)
        .rounded(4.0)
        .bg(white(0.04))
        .border(1.0, white(0.08).mix(white(0.2), focus))
}

/// `px-3 py-1.5 text-xs font-medium` with a hairline: the panel's action.
fn outline(s: &Screen<'_>, place: &str, label: &str, act: NetAct) -> El {
    let id = net(place, s, act);
    let hot = s.ui.hover(id);
    div()
        .id(id)
        .on(Action::Click)
        .ring(4.0)
        .px(12.0)
        .py(6.0)
        .rounded(4.0)
        .shrink0()
        .border(1.0, white(0.08))
        .bg(white(0.0).mix(white(0.06), hot))
        .role(Role::Button, label)
        .child(
            text(label, 12.0, c(look::GRAY_200))
                .weight(500)
                .line_height(16.0)
                .silent(),
        )
}

/// The same without the hairline, gray until hovered: the way out.
fn ghost(s: &Screen<'_>, place: &str, label: &str, act: NetAct) -> El {
    let id = net(place, s, act);
    let hot = s.ui.hover(id);
    div()
        .id(id)
        .on(Action::Click)
        .ring(4.0)
        .px(12.0)
        .py(6.0)
        .rounded(4.0)
        .shrink0()
        .border(1.0, white(0.0))
        .bg(white(0.0).mix(white(0.06), hot))
        .role(Role::Button, label)
        .child(
            text(label, 12.0, c(look::GRAY_500).mix(c(look::WHITE), hot))
                .weight(500)
                .line_height(16.0)
                .silent(),
        )
}

/// "Remove", confirmed: red text on a neutral pill.
fn remove(s: &Screen<'_>, id: &str) -> El {
    let bid = net("net-revoke", s, NetAct::Revoke(id.to_owned()));
    let hot = s.ui.hover(bid);
    let b = div()
        .on(Action::Click)
        .ring(4.0)
        .px(12.0)
        .py(6.0)
        .rounded(4.0)
        .bg(white(0.06).mix(white(0.1), hot))
        .role(Role::Button, "Remove")
        .child(
            text("Remove", 12.0, c(look::DANGER))
                .weight(500)
                .line_height(16.0)
                .silent(),
        );
    tooltip(s.ui, b, bid, tip::REMOVE, None)
}

/// "Copy", then "Copied" for two seconds.
fn copy_pill(s: &Screen<'_>, n: &Network, value: &str) -> El {
    let id = net("net-copy", s, NetAct::Copy(value.to_owned()));
    let hot = s.ui.hover(id);
    let label = if n.copied(value, Instant::now()) {
        "Copied"
    } else {
        "Copy"
    };
    let b = div()
        .on(Action::Click)
        .ring(4.0)
        .shrink0()
        .px(10.0)
        .py(2.0)
        .rounded(4.0)
        .bg(white(0.08).mix(white(0.12), hot))
        .role(Role::Button, "Copy to clipboard")
        .child(
            text(label, 12.0, c(look::GRAY_300).mix(c(look::WHITE), hot))
                .line_height(16.0)
                .silent(),
        );
    tooltip(s.ui, b, id, tip::COPY, None)
}

//! The Remote Access page: share this machine, with its addresses and devices,
//! or use another one.

use web_time::Instant;

use super::settings::{header, row, toggle};
use super::{c, ico, white, Screen};
use crate::act::Act;
use crate::look::{self, Color};
use crate::network::{
    address_host, address_kind, devices_label, format_remaining, grouped, others_label, pair_link,
    Adding, AddressKind, Mode, NetAct, Network, DEVICE_NAME, HOST_TOKEN, HOST_URL,
    TAILSCALE_DOWNLOAD,
};
use crate::ui::theme::color::AMBER_500;
use crate::ui::{div, field, text, Action, El, Id, Role, TextStyle};

pub(super) fn page(s: &Screen<'_>) -> El {
    let n = &s.st.network;
    let mut el = div().col().child(header(
        "Remote Access",
        "Reach this Vorn from your phone or another computer",
        None,
    ));
    if n.loading && n.status.is_none() {
        el = el.child(
            div()
                .row()
                .items_center()
                .justify_center()
                .gap(12.0)
                .py(32.0)
                .child(ico("refresh-cw", 20.0, 2.0, c(look::GRAY_500)))
                .child(
                    text("Checking network status...", 14.0, c(look::GRAY_500))
                        .line_height(20.0)
                        .silent(),
                ),
        );
    }
    el = el.child(segmented(s, n.viewing));
    match n.viewing {
        Mode::Local => el.child(share(s, n)),
        Mode::Host => el.child(another(s, n)),
    }
}

fn net(place: &str, s: &Screen<'_>, act: NetAct) -> Id {
    s.acts.on(place, Act::Net(act))
}

/// "This machine" / "Another machine".
fn segmented(s: &Screen<'_>, viewing: Mode) -> El {
    let option = |mode: Mode, label: &str| {
        let id = net("net-view", s, NetAct::View(mode));
        let on = viewing == mode;
        let fg = if on {
            c(look::WHITE)
        } else {
            c(look::GRAY_500).mix(c(look::GRAY_300), s.ui.hover(id))
        };
        div()
            .id(id)
            .on(Action::Click)
            .ring(6.0)
            .px(12.0)
            .py(4.0)
            .rounded(6.0)
            .bg(if on { white(0.1) } else { white(0.0) })
            .role(Role::RadioButton, label)
            .selected(on)
            .child(text(label, 12.0, fg).line_height(16.0).silent())
    };
    div()
        .row()
        .mb(20.0)
        .p(2.0)
        .gap(2.0)
        .rounded(8.0)
        .bg(white(0.04))
        .role(Role::RadioGroup, "Remote Access")
        .child(option(Mode::Local, "This machine"))
        .child(option(Mode::Host, "Another machine"))
}

/// `ShareThisMachine`.
fn share(s: &Screen<'_>, n: &Network) -> El {
    let enabled = s
        .store
        .config
        .pointer("/defaults/networkAccessEnabled")
        .and_then(serde_json::Value::as_bool)
        .unwrap_or(false);
    let switch = toggle(
        s,
        "net-share",
        enabled,
        Act::Net(NetAct::Share(!enabled)),
        false,
        "Share this machine",
    )
    .ring(12.0);
    let mut el = div().col().child_if(this_computer(s, n)).child(row(
        "Share this machine",
        "Let other devices open Vorn in a browser",
        None,
        switch,
        false,
    ));
    if n.confirming {
        el = el.child(confirm(s));
    }
    if enabled {
        el = el.child_if(addresses(s, n)).child(devices(s, n));
    }
    el
}

/// The web client on this computer: its address, to copy or open. Not in the
/// web client itself, which is that page already.
fn this_computer(s: &Screen<'_>, n: &Network) -> Option<El> {
    if cfg!(target_arch = "wasm32") {
        return None;
    }
    let open = s.acts.on("net-open-web", Act::OpenWebClient);
    let hot = s.ui.hover(open);
    let open_btn = div()
        .id(open)
        .on(Action::Click)
        .ring(6.0)
        .ml(8.0)
        .shrink0()
        .px(8.0)
        .py(4.0)
        .rounded(6.0)
        .bg(white(0.06).mix(white(0.1), hot))
        .role(Role::Button, "Open in browser")
        .child(
            text("Open", 12.0, c(look::GRAY_400).mix(c(look::WHITE), hot))
                .line_height(16.0)
                .silent(),
        );
    Some(
        card()
            .mb(16.0)
            .child(caption("On this computer").mb(6.0))
            .child(
                div()
                    .row()
                    .items_center()
                    .child(
                        div().row().grow().min_w0().child(
                            text(s.web_url, 14.0, c(look::GRAY_200))
                                .mono()
                                .line_height(20.0)
                                .truncate(),
                        ),
                    )
                    .child(copy_button(s, n, s.web_url))
                    .child(open_btn),
            )
            .child(
                text(
                    "The port changes each time the server starts. In a terminal, vorn web opens it.",
                    12.0,
                    c(look::GRAY_600),
                )
                .line_height(16.0)
                .mt(8.0)
                .wrapping()
                .silent(),
            ),
    )
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
        .mt(12.0)
        .p(16.0)
        .rounded(8.0)
        .border(1.0, AMBER_500.alpha(0.2))
        .bg(AMBER_500.alpha(0.04))
        .child(
            text("Enable without Tailscale?", 14.0, c(look::AMBER_400))
                .weight(500)
                .line_height(20.0)
                .mb(4.0),
        )
        .child(para(
            "Vorn will accept connections from anything on this network, over plain HTTP. A device \
             token is still required, but it travels unencrypted, so anyone who can read this \
             network's traffic can take it. A token runs commands on this machine.",
        ))
        .child(
            para("Fine on a home or office network you trust. Not on shared or public wifi.")
                .mt(6.0),
        )
        .child(
            div()
                .row()
                .gap(8.0)
                .mt(12.0)
                .child(outline(s, "net-share", "Enable anyway", NetAct::EnableAnyway))
                .child(ghost(s, "net-share", "Cancel", NetAct::CancelShare)),
        )
}

/// The first address, its kind, and the others on demand.
fn addresses(s: &Screen<'_>, n: &Network) -> Option<El> {
    let reachable = n.reachable.as_ref()?;
    let (primary, rest) = reachable.urls.split_first()?;
    let on_tailnet = n.status.as_ref().is_some_and(|t| t.running);
    let installed = n.status.as_ref().is_some_and(|t| t.installed);
    let faint = |t: &str| text(t, 12.0, c(look::GRAY_600)).line_height(16.0).silent();
    let mut note = div().row().items_center().mt(8.0);
    if on_tailnet {
        note = note.child(faint("Encrypted via Tailscale"));
    } else {
        note = note.child(faint("This network only, unencrypted"));
        if !installed {
            let open = s
                .acts
                .on("net-tailscale", Act::OpenUrl(TAILSCALE_DOWNLOAD.to_owned()));
            note = note
                .child(faint(" · "))
                .child(inline_link(s, open, "Add Tailscale"));
        }
    }
    if !rest.is_empty() {
        let id = net("net-others", s, NetAct::ToggleAddresses);
        note = note.child(faint(" · ")).child(inline_link(
            s,
            id,
            &others_label(rest.len(), n.show_all),
        ));
    }
    let mut card = card().child(caption("Open in a browser").mb(6.0)).child(
        div()
            .row()
            .items_center()
            .child(
                text(primary.as_str(), 14.0, c(look::GREEN_400))
                    .mono()
                    .line_height(20.0)
                    .truncate(),
            )
            .child(copy_button(s, n, primary)),
    );
    card = card.child(note);
    if n.show_all && !rest.is_empty() {
        let list = rest.iter().map(|url| {
            div()
                .row()
                .items_center()
                .py(4.0)
                .child(
                    div().row().grow().min_w0().child(
                        text(url.as_str(), 12.0, c(look::GRAY_400))
                            .mono()
                            .line_height(16.0)
                            .truncate(),
                    ),
                )
                .child(copy_button(s, n, url))
        });
        card = card.child(
            div()
                .col()
                .mt(12.0)
                .pt(12.0)
                .border_top(1.0, white(0.06))
                .children(list)
                .child(
                    text(
                        "Use whichever address the other device shares a network with.",
                        10.0,
                        c(look::GRAY_600),
                    )
                    .line_height(14.0)
                    .mt(6.0)
                    .wrapping()
                    .silent(),
                ),
        );
    }
    Some(card)
}

/// The devices allowed in, and the only place to add one.
fn devices(s: &Screen<'_>, n: &Network) -> El {
    let active = n.active();
    let mut head = div()
        .row()
        .items_center()
        .justify_between()
        .child(caption(&devices_label(active.len())));
    if n.adding.is_none() && n.minted.is_none() {
        head = head.child(
            link(
                s,
                "net-add",
                "Add device",
                NetAct::AddDevice,
                look::GRAY_400,
            )
            .mr(-8.0),
        );
    }
    let mut el = card().child(head);
    if let Some(m) = &n.minted {
        el = el.child(minted(s, n, &m.name, &m.plaintext));
    }
    if let Some(asking) = &n.asking {
        el = el.child(asking_panel(s, &asking.device_name, &asking.address));
    } else {
        match &n.adding {
            Some(Adding::Scan(code)) => {
                let left = code.remaining(now_ms());
                if left == 0 {
                    el = el.child(expired(s));
                } else if let Some(url) = n.pair_url() {
                    el = el.child(scan(s, n, url, &code.code, left));
                }
            }
            Some(Adding::Token) => el = el.child(token_form(s)),
            None => {}
        }
    }
    if let Some(error) = &n.error {
        el = el.child(
            text(error.as_str(), 12.0, c(look::DANGER))
                .line_height(16.0)
                .mt(8.0)
                .wrapping(),
        );
    }
    if n.tokens.is_some() && active.is_empty() && n.adding.is_none() && n.minted.is_none() {
        el = el.child(
            text(
                "No devices yet. Add one, then open the address above on it.",
                12.0,
                c(look::GRAY_600),
            )
            .line_height(16.0)
            .mt(8.0)
            .wrapping()
            .silent(),
        );
    }
    if !active.is_empty() {
        let rows = active.iter().enumerate().map(|(i, t)| {
            let right = if n.confirm_revoke.as_deref() == Some(t.id.as_str()) {
                div()
                    .row()
                    .gap(6.0)
                    .shrink0()
                    .child(revoke(s, &t.id))
                    .child(link(
                        s,
                        "net-revoke",
                        "Cancel",
                        NetAct::CancelRevoke,
                        look::GRAY_500,
                    ))
            } else {
                link(
                    s,
                    "net-remove",
                    "Remove",
                    NetAct::AskRevoke(t.id.clone()),
                    look::GRAY_500,
                )
            };
            let mut r = div()
                .row()
                .items_center()
                .gap(12.0)
                .py(8.0)
                .child(
                    div()
                        .col()
                        .grow()
                        .min_w0()
                        .child(
                            text(t.name.as_str(), 14.0, c(look::GRAY_200))
                                .line_height(20.0)
                                .truncate(),
                        )
                        .child(
                            text(t.seen(), 10.0, c(look::GRAY_600))
                                .line_height(14.0)
                                .silent(),
                        ),
                )
                .child(right);
            if i > 0 {
                r = r.border_top(1.0, white(0.04));
            }
            r
        });
        el = el.child(div().col().mt(4.0).children(rows)).child(
            text(crate::network::IDLE_EXPIRY, 10.0, c(look::GRAY_600))
                .line_height(14.0)
                .mt(4.0)
                .wrapping()
                .silent(),
        );
    }
    el
}

/// The one moment a token's plaintext is readable.
fn minted(s: &Screen<'_>, n: &Network, name: &str, plaintext: &str) -> El {
    div()
        .col()
        .mt(12.0)
        .p(12.0)
        .rounded(8.0)
        .border(1.0, c(look::BRONZO).alpha(0.3))
        .bg(c(look::SURFACE_RAISED))
        .child(
            text(
                format!("Token for \u{201c}{name}\u{201d}"),
                14.0,
                c(look::GRAY_200),
            )
            .weight(500)
            .line_height(20.0),
        )
        .child(
            text(
                "Copy it now. Only its hash is stored, so this is the only time it can be shown.",
                12.0,
                c(look::GRAY_500),
            )
            .line_height(16.0)
            .mt(4.0)
            .mb(10.0)
            .wrapping()
            .silent(),
        )
        .child(
            div()
                .row()
                .items_center()
                .px(12.0)
                .py(8.0)
                .rounded(6.0)
                .bg(c(look::SURFACE_BASE))
                .border(1.0, white(0.06))
                .child(
                    div().row().grow().min_w0().child(
                        text(plaintext, 12.0, c(look::INK))
                            .mono()
                            .line_height(16.0)
                            .truncate(),
                    ),
                )
                .child(copy_button(s, n, plaintext)),
        )
        .children(n.fingerprint().map(fingerprint))
        .child(
            div()
                .row()
                .mt(10.0)
                .child(link(s, "net-minted", "Done", NetAct::DoneMinted, look::GRAY_500).ml(-8.0)),
        )
}

/// A phone that redeemed the code, waiting on a person.
fn asking_panel(s: &Screen<'_>, device: &str, address: &str) -> El {
    let pair = |k: &str, v: &str| {
        div()
            .row()
            .justify_between()
            .gap(12.0)
            .child(text(k, 12.0, c(look::GRAY_500)).line_height(16.0).silent())
            .child(
                text(v, 12.0, c(look::GRAY_300))
                    .mono()
                    .line_height(16.0)
                    .truncate(),
            )
    };
    div()
        .col()
        .mt(12.0)
        .p(12.0)
        .rounded(8.0)
        .border(1.0, c(look::BRONZO).alpha(0.4))
        .bg(c(look::SURFACE_RAISED))
        .child(
            div()
                .row()
                .items_center()
                .gap(8.0)
                .child(
                    div()
                        .size(6.0, 6.0)
                        .rounded(3.0)
                        .shrink0()
                        .bg(c(look::BRONZO)),
                )
                .child(
                    text("A phone is asking to pair", 14.0, c(look::GRAY_200))
                        .weight(500)
                        .line_height(20.0),
                ),
        )
        .child(
            div()
                .col()
                .gap(4.0)
                .mt(8.0)
                .child(pair("Device", device))
                .child(pair("Address", address)),
        )
        .child(
            text(
                "This lets it run terminals on this machine. Only approve a phone you are holding.",
                11.0,
                c(look::GRAY_500),
            )
            .line_height(16.0)
            .mt(8.0)
            .wrapping()
            .silent(),
        )
        .child(
            div()
                .row()
                .gap(8.0)
                .mt(10.0)
                .child(outline(s, "net-decide", "Approve", NetAct::Decide(true)))
                .child(ghost(s, "net-decide", "Deny", NetAct::Decide(false))),
        )
}

/// The code to type, the link it stands for, and which address it carries.
fn scan(s: &Screen<'_>, n: &Network, pair_url: &str, code: &str, left: u64) -> El {
    let urls = n.reachable.as_ref().map_or(&[][..], |r| &r.urls[..]);
    let link_text = pair_link(pair_url, code, n.fingerprint());
    let mut col = div()
        .col()
        .min_w0()
        .mt(12.0)
        .child(
            text("Open this link in the Vorn app.", 14.0, c(look::GRAY_200))
                .line_height(20.0)
                .silent(),
        )
        .child(
            text(
                "Then approve the phone here. It will ask by name.",
                12.0,
                c(look::GRAY_500),
            )
            .line_height(16.0)
            .mt(4.0)
            .silent(),
        )
        .child(
            div()
                .row()
                .items_center()
                .mt(8.0)
                .child(
                    div().row().grow().min_w0().child(
                        text(link_text.as_str(), 12.0, c(look::GRAY_400))
                            .mono()
                            .line_height(16.0)
                            .truncate(),
                    ),
                )
                .child(copy_button(s, n, &link_text)),
        );
    if urls.len() > 1 {
        let chips = urls.iter().map(|url| {
            let id = net("net-url", s, NetAct::ChooseUrl(url.clone()));
            let chosen = url == pair_url;
            let (border, bg, fg) = if chosen {
                (white(0.2), white(0.08), c(look::GRAY_200))
            } else {
                let hot = s.ui.hover(id);
                (
                    white(0.06),
                    white(0.0).mix(white(0.06), hot),
                    c(look::GRAY_500).mix(c(look::WHITE), hot),
                )
            };
            div()
                .id(id)
                .on(Action::Click)
                .ring(6.0)
                .px(8.0)
                .py(4.0)
                .rounded(6.0)
                .border(1.0, border)
                .bg(bg)
                .role(Role::RadioButton, url.as_str())
                .selected(chosen)
                .child(
                    text(address_host(url), 11.0, fg)
                        .mono()
                        .line_height(16.0)
                        .silent(),
                )
        });
        let kind = match address_kind(pair_url) {
            AddressKind::Tailnet => "The phone must be on your tailnet.",
            AddressKind::Lan => "The phone must be able to reach this address.",
        };
        col = col.child(
            div()
                .col()
                .mt(12.0)
                .child(caption("Address in the code").mb(4.0))
                .child(div().row().wrap_children().gap(6.0).children(chips))
                .child(
                    text(kind, 11.0, c(look::GRAY_600))
                        .line_height(16.0)
                        .mt(6.0)
                        .silent(),
                ),
        );
    }
    col.child(caption("Or type this code").mt(12.0))
        .child(
            text(code, 16.0, c(look::GRAY_200))
                .mono()
                .tracking(0.1)
                .line_height(24.0)
                .mt(2.0)
                .role(Role::Label, "Pairing code"),
        )
        .child(
            text(
                format!("Expires in {}", format_remaining(left)),
                11.0,
                c(look::GRAY_500),
            )
            .mono()
            .line_height(16.0)
            .mt(4.0)
            .silent(),
        )
        .children(n.fingerprint().map(fingerprint))
        .child(
            div()
                .col()
                .items_start()
                .gap(4.0)
                .mt(16.0)
                .child(
                    link(
                        s,
                        "net-scan",
                        "Pair manually instead",
                        NetAct::PairManually,
                        look::GRAY_400,
                    )
                    .ml(-8.0),
                )
                .child(link(s, "net-scan", "Cancel", NetAct::CloseAdding, look::GRAY_500).ml(-8.0)),
        )
}

fn expired(s: &Screen<'_>) -> El {
    div()
        .col()
        .mt(12.0)
        .child(
            text("That code expired.", 12.0, c(look::GRAY_500))
                .line_height(16.0)
                .silent(),
        )
        .child(
            div()
                .row()
                .gap(8.0)
                .mt(8.0)
                .child(outline(
                    s,
                    "net-expired",
                    "Show another",
                    NetAct::ShowAnother,
                ))
                .child(ghost(s, "net-expired", "Cancel", NetAct::CloseAdding)),
        )
}

fn token_form(s: &Screen<'_>) -> El {
    div()
        .col()
        .mt(12.0)
        .child(label("What is this device?"))
        .child(input(s, DEVICE_NAME, "Device name", "My phone", false))
        .child(
            div()
                .row()
                .gap(8.0)
                .mt(10.0)
                .child(outline(s, "net-token", "Create token", NetAct::CreateToken))
                .child(ghost(s, "net-token", "Cancel", NetAct::CloseAdding)),
        )
}

/// `UseAnotherMachine`, editing: this app has no host mode to show as
/// connected.
fn another(s: &Screen<'_>, n: &Network) -> El {
    let mut form = card()
        .mt(0.0)
        .child(label("Server address"))
        .child(input(
            s,
            HOST_URL,
            "Server address",
            "192.168.0.4:61601",
            true,
        ))
        .child(label("Device token from that machine").mt(12.0))
        .child(input(
            s,
            HOST_TOKEN,
            "Device token from that machine",
            "vorn_...",
            true,
        ));
    if let Some(error) = &n.connect_error {
        form = form.child(
            text(error.as_str(), 12.0, c(look::AMBER_400).alpha(0.8))
                .line_height(16.0)
                .mt(8.0)
                .wrapping(),
        );
    }
    form = form.child(div().row().gap(8.0).mt(12.0).child(outline(
        s,
        "net-connect",
        "Connect and restart",
        NetAct::Connect,
    )));
    div().col().child(form).child(
        text(
            "Vorn restarts to apply this. Workflows and schedules run only while a desktop is \
             attached to the host, so a server on its own holds your data but fires no schedules.",
            12.0,
            c(look::GRAY_600),
        )
        .line_height(16.0)
        .mt(12.0)
        .wrapping()
        .silent(),
    )
}

fn now_ms() -> u64 {
    crate::state::now_ms().max(0.0) as u64
}

/// The bordered well each block sits in: `rounded-lg border bg-white/[0.02] p-4`.
fn card() -> El {
    div()
        .col()
        .mt(12.0)
        .p(16.0)
        .rounded(8.0)
        .border(1.0, white(0.06))
        .bg(white(0.02))
}

/// The certificate a phone will trust, for typing when pairing by hand.
fn fingerprint(fp: &str) -> El {
    div()
        .col()
        .mt(12.0)
        .child(caption("Certificate fingerprint").mb(2.0))
        .child(
            text(grouped(fp), 11.0, c(look::GRAY_400))
                .mono()
                .line_height(16.0)
                .wrapping()
                .role(Role::Label, "Certificate fingerprint"),
        )
}

/// `text-[10px] uppercase tracking-wider font-medium text-gray-600`.
fn caption(t: &str) -> El {
    text(t.to_uppercase(), 10.0, c(look::GRAY_600))
        .weight(500)
        .tracking(0.05)
        .line_height(14.0)
        .silent()
}

fn label(t: &str) -> El {
    text(t, 12.0, c(look::GRAY_400))
        .line_height(16.0)
        .mb(6.0)
        .silent()
}

/// The page's `px-3 py-2 text-sm` field on `white/[0.04]`.
fn input(s: &Screen<'_>, id: Id, name: &str, placeholder: &str, mono: bool) -> El {
    let focus = if s.ui.focused(id) { 1.0 } else { 0.0 };
    let style = TextStyle {
        line_height: 20.0,
        mono,
        ..TextStyle::ui(14.0)
    };
    field(id, name, placeholder, style, c(look::GRAY_200))
        .px(12.0)
        .py(8.0)
        .rounded(6.0)
        .bg(white(0.04))
        .border(1.0, white(0.08).mix(white(0.2), focus))
        .w_full()
}

/// `px-3 py-1.5 text-xs font-medium` with a hairline: the panel's action.
fn outline(s: &Screen<'_>, place: &str, label: &str, act: NetAct) -> El {
    let id = net(place, s, act);
    let hot = s.ui.hover(id);
    div()
        .id(id)
        .on(Action::Click)
        .ring(6.0)
        .px(12.0)
        .py(6.0)
        .rounded(6.0)
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
        .ring(6.0)
        .px(12.0)
        .py(6.0)
        .rounded(6.0)
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

/// `px-2 py-1 text-xs` text button in `rest` ink, white on hover.
fn link(s: &Screen<'_>, place: &str, label: &str, act: NetAct, rest: Color) -> El {
    let id = net(place, s, act);
    let hot = s.ui.hover(id);
    div()
        .id(id)
        .on(Action::Click)
        .ring(6.0)
        .px(8.0)
        .py(4.0)
        .rounded(6.0)
        .shrink0()
        .bg(white(0.0).mix(white(0.06), hot))
        .role(Role::Button, label)
        .child(
            text(label, 12.0, c(rest).mix(c(look::WHITE), hot))
                .line_height(16.0)
                .silent(),
        )
}

/// A link inside a sentence: gray-400 text, white on hover.
fn inline_link(s: &Screen<'_>, id: Id, label: &str) -> El {
    let fg = c(look::GRAY_400).mix(c(look::WHITE), s.ui.hover(id));
    div()
        .id(id)
        .on(Action::Click)
        .ring(4.0)
        .role(Role::Button, label)
        .child(text(label, 12.0, fg).line_height(16.0).silent())
}

/// "Revoke": red, as the destructive step it is.
fn revoke(s: &Screen<'_>, id: &str) -> El {
    let bid = net("net-revoke", s, NetAct::Revoke(id.to_owned()));
    let hot = s.ui.hover(bid);
    div()
        .id(bid)
        .on(Action::Click)
        .ring(6.0)
        .px(8.0)
        .py(4.0)
        .rounded(6.0)
        .border(1.0, c(look::DANGER).alpha(0.25))
        .bg(c(look::DANGER).alpha(0.1 * hot))
        .role(Role::Button, "Revoke")
        .child(
            text("Revoke", 12.0, c(look::DANGER))
                .weight(500)
                .line_height(16.0)
                .silent(),
        )
}

/// "Copy", then "Copied" for two seconds.
fn copy_button(s: &Screen<'_>, n: &Network, value: &str) -> El {
    let id = net("net-copy", s, NetAct::Copy(value.to_owned()));
    let hot = s.ui.hover(id);
    let label = if n.copied(value, Instant::now()) {
        "Copied"
    } else {
        "Copy"
    };
    div()
        .id(id)
        .on(Action::Click)
        .ring(6.0)
        .ml(8.0)
        .shrink0()
        .px(8.0)
        .py(4.0)
        .rounded(6.0)
        .bg(white(0.06).mix(white(0.1), hot))
        .role(Role::Button, "Copy to clipboard")
        .child(
            text(label, 12.0, c(look::GRAY_400).mix(c(look::WHITE), hot))
                .line_height(16.0)
                .silent(),
        )
}

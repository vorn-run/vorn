//! `SSHSettings`: remote hosts with their credentials and connection test,
//! and the SSH keys vornd keeps in the keychain.

use serde_json::Value;

use super::{label, section, set};
use crate::act::Act;
use crate::look::{self, Color};
use crate::screen::{c, ico, white, Screen};
use crate::settings::{
    host_subtitle, truncate_key, HostField, KeyField, SetAct, TestResult, AUTH_METHODS,
};
use crate::state::MenuKind;
use crate::ui::{area, div, field, text, Action, El, Id, Role, TextStyle};

/// A host's text field.
pub fn host_id(host: &str, f: HostField) -> Id {
    Id::new("ssh-host").child((host, f))
}

/// The paste-a-key form's text field.
pub fn key_field_id(f: KeyField) -> Id {
    Id::new("ssh-key-form").child(f)
}

fn s_of<'a>(h: &'a Value, k: &str) -> &'a str {
    h.get(k).and_then(Value::as_str).unwrap_or("")
}

/// The `bg-black/30` frame every input here has.
fn frame(s: &Screen<'_>, id: Id, inner: El) -> El {
    div()
        .w_full()
        .px(10.0)
        .py(6.0)
        .rounded(6.0)
        .bg(crate::ui::theme::shadow(0.3))
        .border(
            1.0,
            if s.ui.focused(id) {
                white(0.15)
            } else {
                white(0.06)
            },
        )
        .frames(id)
        .child(inner)
}

fn style(mono: bool) -> TextStyle {
    let base = if mono {
        TextStyle::mono(12.0)
    } else {
        TextStyle::ui(12.0)
    };
    TextStyle {
        line_height: 16.0,
        ..base
    }
}

fn host_input(
    s: &Screen<'_>,
    host: &str,
    f: HostField,
    value: String,
    placeholder: &str,
    mono: bool,
) -> El {
    let id = host_id(host, f);
    s.acts
        .text(id, set(SetAct::HostText(host.to_owned(), f)), value);
    frame(
        s,
        id,
        field(id, placeholder, placeholder, style(mono), c(look::GRAY_200))
            .placeholder_color(c(look::GRAY_600))
            .w_full(),
    )
}

fn labelled(t: &str, el: El) -> El {
    div().col().w_full().child(label(t, 4.0)).child(el)
}

fn small_btn(
    s: &Screen<'_>,
    place: &str,
    act: SetAct,
    label: &str,
    icon: Option<&str>,
    danger: bool,
    enabled: bool,
) -> El {
    let id = s.acts.on(place, set(act));
    let h = s.ui.hover(id);
    let (fg, bg, border) = if danger {
        (
            c(look::GRAY_500).mix(c(look::RED_400), h),
            white(0.03).mix(look::RED_400.alpha(0.1), h),
            white(0.04).mix(look::RED_400.alpha(0.2), h),
        )
    } else {
        (
            c(look::GRAY_400).mix(c(look::WHITE), h),
            white(0.04).mix(white(0.08), h),
            white(0.06),
        )
    };
    let mut b = div()
        .id(id)
        .row()
        .items_center()
        .gap(6.0)
        .px(12.0)
        .py(6.0)
        .rounded(6.0)
        .bg(bg)
        .border(1.0, border)
        .role(Role::Button, label);
    if let Some(i) = icon {
        b = b.child(ico(i, 11.0, 1.5, fg));
    }
    b = b.child(
        text(label, 12.0, fg)
            .weight(if danger { 400 } else { 500 })
            .line_height(16.0)
            .silent(),
    );
    if enabled {
        b.on(Action::Click)
    } else {
        b.opacity(0.5)
    }
}

fn confirm_pair(s: &Screen<'_>, place: &str, ok: SetAct) -> El {
    let yes = s.acts.on(place, set(ok));
    let no = s.acts.on(place, set(SetAct::CancelDelete));
    div()
        .row()
        .items_center()
        .gap(8.0)
        .child(
            div()
                .id(yes)
                .on(Action::Click)
                .px(12.0)
                .py(6.0)
                .rounded(6.0)
                .bg(look::RED_400.alpha(0.1 + 0.1 * s.ui.hover(yes)))
                .border(1.0, look::RED_400.alpha(0.2))
                .role(Role::Button, "Confirm Delete")
                .child(
                    text("Confirm Delete", 12.0, c(look::RED_400))
                        .line_height(16.0)
                        .silent(),
                ),
        )
        .child(
            div()
                .id(no)
                .on(Action::Click)
                .px(12.0)
                .py(6.0)
                .role(Role::Button, "Cancel")
                .child(
                    text(
                        "Cancel",
                        12.0,
                        c(look::GRAY_400).mix(c(look::WHITE), s.ui.hover(no)),
                    )
                    .line_height(16.0)
                    .silent(),
                ),
        )
}

fn host_form(s: &Screen<'_>, h: &Value, id: &str) -> El {
    let st = &s.st.settings.ssh;
    let auth = h
        .get("authMethod")
        .and_then(Value::as_str)
        .unwrap_or("agent");
    let port = h
        .get("port")
        .and_then(Value::as_f64)
        .map_or(22, |p| p as i64);
    let mut methods = div().row().gap(2.0).p(2.0).rounded(6.0).bg(white(0.04));
    for (key, name) in AUTH_METHODS {
        let bid = s
            .acts
            .on("ssh-auth", set(SetAct::Auth(id.to_owned(), key.to_owned())));
        let on = auth == key;
        let hot = s.ui.hover(bid);
        let mut b = div()
            .id(bid)
            .on(Action::Click)
            .flex(1.0)
            .row()
            .justify_center()
            .px(8.0)
            .py(6.0)
            .rounded(4.0)
            .role(Role::RadioButton, name)
            .selected(on)
            .child(
                text(
                    name,
                    11.0,
                    if on {
                        c(look::WHITE)
                    } else {
                        c(look::GRAY_500).mix(c(look::GRAY_300), hot)
                    },
                )
                .weight(500)
                .line_height(16.5)
                .silent(),
            );
        if on {
            b = b.bg(white(0.1));
        }
        methods = methods.child(b);
    }
    let small = |t: &str| text(t, 11.0, c(look::GRAY_600)).line_height(16.5).silent();
    let hint = |t: &str| {
        text(t, 10.0, c(look::GRAY_600))
            .line_height(15.0)
            .mt(4.0)
            .silent()
    };
    let credential = match auth {
        "agent" => small("Uses your running SSH agent (ssh-agent)"),
        "password" => {
            let pid = host_id(id, HostField::Password);
            let saved = !s_of(h, "encryptedPassword").is_empty();
            div()
                .col()
                .child(label("Password", 4.0))
                .child(frame(
                    s,
                    pid,
                    field(
                        pid,
                        "Password",
                        if saved { "(saved)" } else { "Enter password" },
                        style(false),
                        c(look::GRAY_200),
                    )
                    .placeholder_color(c(look::GRAY_600))
                    .secure()
                    .w_full(),
                ))
                .child(hint("Encrypted with your OS keychain"))
        }
        "key-stored" => {
            let cred = s_of(h, "credentialId");
            let name =
                st.keys
                    .iter()
                    .find(|k| k.id == cred)
                    .map_or("Select a key...".to_owned(), |k| {
                        k.key_type
                            .as_ref()
                            .map_or(k.label.clone(), |t| format!("{} ({t})", k.label))
                    });
            let sid = s
                .acts
                .on("ssh-key-select", Act::Open(MenuKind::SshKey(id.to_owned())));
            let mut col = div().col().child(label("Stored Key", 4.0)).child(
                div()
                    .id(sid)
                    .on(Action::Click)
                    .row()
                    .items_center()
                    .px(10.0)
                    .py(6.0)
                    .rounded(6.0)
                    .bg(crate::ui::theme::shadow(0.3))
                    .border(1.0, white(0.06))
                    .role(Role::ComboBox, "Stored Key")
                    .child(
                        div().grow().child(
                            text(name, 12.0, c(look::GRAY_200))
                                .line_height(16.0)
                                .silent(),
                        ),
                    )
                    .child(ico("chevron-down", 11.0, 2.0, c(look::GRAY_400))),
            );
            if st.keys.is_empty() {
                col = col.child(hint("No stored keys yet. Add one below."));
            }
            col
        }
        _ => div().col().child(label("SSH Key Path", 4.0)).child(
            div()
                .row()
                .gap(6.0)
                .child(div().grow().min_w0().child(host_input(
                    s,
                    id,
                    HostField::KeyPath,
                    s_of(h, "sshKeyPath").to_owned(),
                    "~/.ssh/id_ed25519",
                    true,
                )))
                .child({
                    let bid = s.acts.on(
                        "ssh-browse",
                        set(SetAct::HostText(id.to_owned(), HostField::KeyPath)),
                    );
                    let hot = s.ui.hover(bid);
                    div()
                        .id(bid)
                        .on(Action::Click)
                        .px(8.0)
                        .py(6.0)
                        .rounded(6.0)
                        .bg(crate::ui::theme::shadow(0.3).mix(white(0.08), hot))
                        .border(1.0, white(0.06))
                        .role(Role::Button, "Browse")
                        .child(ico(
                            "key",
                            12.0,
                            1.5,
                            c(look::GRAY_400).mix(c(look::WHITE), hot),
                        ))
                }),
        ),
    };
    let testing = st.tests.get(id) == Some(&TestResult::Testing);
    let mut actions = div()
        .row()
        .items_center()
        .justify_between()
        .pt(8.0)
        .child(small_btn(
            s,
            "ssh-test",
            SetAct::Test(id.to_owned()),
            "Test Connection",
            Some(if testing { "loader-circle" } else { "wifi" }),
            false,
            !testing,
        ));
    actions = if st.confirm.as_deref() == Some(id) {
        actions.child(confirm_pair(
            s,
            "ssh-host-delete",
            SetAct::DeleteHost(id.to_owned()),
        ))
    } else {
        actions.child(small_btn(
            s,
            "ssh-host-ask",
            SetAct::AskDelete(id.to_owned()),
            "Delete Host",
            Some("trash-2"),
            true,
            true,
        ))
    };
    let mut form = div()
        .col()
        .gap(12.0)
        .px(16.0)
        .pb(16.0)
        .pt(4.0)
        .child(labelled(
            "Label",
            host_input(
                s,
                id,
                HostField::Label,
                s_of(h, "label").to_owned(),
                "Host label",
                false,
            ),
        ))
        .child(
            div()
                .row()
                .gap(8.0)
                .child(div().grow().min_w0().child(labelled(
                    "Hostname",
                    host_input(
                        s,
                        id,
                        HostField::Hostname,
                        s_of(h, "hostname").to_owned(),
                        "e.g. dev.example.com",
                        true,
                    ),
                )))
                .child(div().w(100.0).shrink0().child(labelled(
                    "Port",
                    host_input(s, id, HostField::Port, port.to_string(), "", true),
                ))),
        )
        .child(labelled(
            "Username",
            host_input(
                s,
                id,
                HostField::User,
                s_of(h, "user").to_owned(),
                "e.g. ubuntu",
                true,
            ),
        ))
        .child(
            div()
                .col()
                .child(label("Authentication", 6.0))
                .child(methods),
        )
        .child(credential)
        .child(labelled(
            "Extra SSH Options",
            host_input(
                s,
                id,
                HostField::Options,
                s_of(h, "sshOptions").to_owned(),
                "e.g. -o ForwardAgent=yes",
                true,
            ),
        ))
        .child(actions);
    if let Some(TestResult::Done { ok, message }) = st.tests.get(id) {
        let (fg, base) = if *ok {
            (c(look::GREEN_400), look::GREEN_500)
        } else {
            (c(look::RED_400), look::RED_500)
        };
        form = form.child(
            div()
                .row()
                .items_center()
                .gap(6.0)
                .mt(8.0)
                .px(10.0)
                .py(6.0)
                .rounded(6.0)
                .bg(base.alpha(0.1))
                .border(1.0, base.alpha(0.2))
                .child(ico(
                    if *ok { "circle-check" } else { "circle-x" },
                    12.0,
                    2.0,
                    fg,
                ))
                .child(
                    text(message, 11.0, fg)
                        .line_height(16.5)
                        .wrapping()
                        .silent(),
                ),
        );
    }
    form
}

fn collapsed_row(
    s: &Screen<'_>,
    place: &str,
    act: SetAct,
    tile: El,
    title: El,
    sub: Option<El>,
    open: bool,
) -> El {
    let id = s.acts.on(place, set(act));
    let mut info = div().col().grow().min_w0().child(title);
    if let Some(sub) = sub {
        info = info.child(sub);
    }
    div()
        .id(id)
        .on(Action::Click)
        .row()
        .items_center()
        .gap(12.0)
        .px(16.0)
        .py(12.0)
        .role(Role::Button, place)
        .expanded(open)
        .child(tile)
        .child(info)
        .child(ico(
            if open {
                "chevron-down"
            } else {
                "chevron-right"
            },
            14.0,
            2.0,
            c(look::GRAY_600),
        ))
}

fn tile(icon_name: &str, color: Color, bg: Color) -> El {
    div()
        .size(36.0, 36.0)
        .center()
        .shrink0()
        .rounded(8.0)
        .bg(bg.alpha(0.1))
        .child(ico(icon_name, 18.0, 1.5, c(color)))
}

fn card(s: &Screen<'_>, id: Id, inner: Vec<El>) -> El {
    let hot = s.ui.hover(id) > 0.0;
    div()
        .id(id)
        .sense(crate::ui::Sense::HOVER)
        .col()
        .rounded(12.0)
        .clip()
        .bg(c(look::SURFACE_SUNKEN))
        .border(1.0, if hot { white(0.1) } else { white(0.06) })
        .children(inner)
}

fn dashed_button(
    s: &Screen<'_>,
    place: &str,
    act: SetAct,
    icon_name: &str,
    label: &str,
    enabled: bool,
    grow: bool,
) -> El {
    let id = s.acts.on(place, set(act));
    let h = s.ui.hover(id);
    let fg = c(look::GRAY_500).mix(c(look::WHITE), h);
    let mut b = div()
        .id(id)
        .row()
        .items_center()
        .justify_center()
        .gap(8.0)
        .py(10.0)
        .rounded(8.0)
        .border(1.0, white(0.08).mix(white(0.15), h))
        .dashed()
        .bg(white(0.0).mix(white(0.02), h))
        .role(Role::Button, label)
        .child(ico(
            icon_name,
            14.0,
            if icon_name == "plus" { 2.0 } else { 1.5 },
            fg,
        ))
        .child(text(label, 14.0, fg).line_height(20.0).silent());
    b = if grow { b.flex(1.0) } else { b.w_full() };
    if enabled {
        b.on(Action::Click)
    } else {
        b.opacity(0.4)
    }
}

fn key_form(s: &Screen<'_>) -> El {
    let input = |f: KeyField, placeholder: &str, rows: Option<f32>| {
        let id = key_field_id(f);
        s.acts
            .text(id, set(SetAct::KeyText(f)), s.ui.edit_text(id).to_owned());
        let el = match rows {
            None => field(
                id,
                placeholder,
                placeholder,
                style(false),
                c(look::GRAY_200),
            ),
            Some(_) => area(id, placeholder, placeholder, style(true), c(look::GRAY_200)),
        }
        .placeholder_color(c(look::GRAY_600))
        .w_full();
        let el = match rows {
            Some(n) => el.h(16.0 * n),
            None => el,
        };
        frame(s, id, el)
    };
    let empty =
        s.ui.edit_text(key_field_id(KeyField::Private))
            .trim()
            .is_empty();
    let save = s.acts.on("ssh-key-save", set(SetAct::SaveKey));
    let cancel = s.acts.on("ssh-key-cancel", set(SetAct::CancelKey));
    let mut save_btn = div()
        .id(save)
        .px(16.0)
        .py(6.0)
        .rounded(6.0)
        .bg(c(look::BLUE_600).mix(c(look::BLUE_500), s.ui.hover(save)))
        .role(Role::Button, "Save Key")
        .child(
            text("Save Key", 12.0, c(look::WHITE))
                .weight(500)
                .line_height(16.0)
                .silent(),
        );
    save_btn = if empty {
        save_btn.opacity(0.4)
    } else {
        save_btn.on(Action::Click)
    };
    div()
        .col()
        .gap(12.0)
        .mt(8.0)
        .p(16.0)
        .rounded(12.0)
        .border(1.0, white(0.08))
        .bg(c(look::SURFACE_SUNKEN))
        .child(
            text("New Key", 14.0, c(look::GRAY_200))
                .weight(500)
                .line_height(20.0)
                .silent(),
        )
        .child(labelled(
            "Label",
            input(KeyField::Label, "e.g. Work Server Key", None),
        ))
        .child(labelled(
            "Private key *",
            input(
                KeyField::Private,
                "-----BEGIN OPENSSH PRIVATE KEY-----",
                Some(3.0),
            ),
        ))
        .child(labelled(
            "Public key",
            input(KeyField::Public, "ssh-ed25519 AAAA...", Some(2.0)),
        ))
        .child(labelled(
            "Certificate",
            input(KeyField::Certificate, "Optional", Some(2.0)),
        ))
        .child(
            div()
                .row()
                .child(
                    text(
                        "If your key has a passphrase, remove it first with ",
                        10.0,
                        c(look::GRAY_600),
                    )
                    .line_height(15.0)
                    .silent(),
                )
                .child(
                    text("ssh-keygen -p", 10.0, c(look::GRAY_500))
                        .mono()
                        .line_height(15.0)
                        .silent(),
                ),
        )
        .child(
            div().row().gap(8.0).child(save_btn).child(
                div()
                    .id(cancel)
                    .on(Action::Click)
                    .px(16.0)
                    .py(6.0)
                    .role(Role::Button, "Cancel")
                    .child(
                        text(
                            "Cancel",
                            12.0,
                            c(look::GRAY_400).mix(c(look::WHITE), s.ui.hover(cancel)),
                        )
                        .line_height(16.0)
                        .silent(),
                    ),
            ),
        )
}

pub(super) fn page(s: &Screen<'_>) -> El {
    let st = &s.st.settings.ssh;
    let hosts: Vec<Value> = s
        .store
        .config
        .get("remoteHosts")
        .and_then(Value::as_array)
        .cloned()
        .unwrap_or_default();
    let mut col = div().col().child(super::header(
        "SSH & Hosts",
        "Configure remote hosts and SSH keys for running agents on remote machines",
        None,
    ));
    if !s.st.settings.safe_storage {
        col = col.child(
            div()
                .mb(24.0)
                .px(16.0)
                .py(12.0)
                .rounded(8.0)
                .border(1.0, look::AMBER_500.alpha(0.3))
                .bg(look::AMBER_500.alpha(0.08))
                .child(
                    text(
                        "Keychain encryption is not available on this system. Install libsecret (gnome-keyring) to use the SSH key vault.",
                        14.0,
                        c(look::AMBER_400),
                    )
                    .line_height(20.0)
                    .wrapping()
                    .silent(),
                ),
        );
    }
    let mut list = div().col().gap(8.0);
    for h in &hosts {
        let id = s_of(h, "id").to_owned();
        let open = st.hosts_open.contains(&id);
        let title = text(s_of(h, "label"), 14.0, c(look::GRAY_100))
            .weight(500)
            .line_height(20.0)
            .silent();
        let sub = text(host_subtitle(h), 11.0, c(look::GRAY_500))
            .mono()
            .line_height(16.5)
            .truncate()
            .silent();
        let mut parts = vec![collapsed_row(
            s,
            "ssh-host-row",
            SetAct::ToggleHost(id.clone()),
            tile("server", look::BLUE_400, look::BLUE_500),
            title,
            Some(sub),
            open,
        )];
        if open {
            parts.push(host_form(s, h, &id));
        }
        list = list.child(card(s, Id::new("ssh-host-card").child(&id), parts));
    }
    col = col.child(
        div()
            .col()
            .mb(32.0)
            .child(section("server", "Remote Hosts", Some(hosts.len())))
            .child(list)
            .child(div().mt(8.0).child(dashed_button(
                s,
                "ssh-add-host",
                SetAct::AddHost,
                "plus",
                "Add Host",
                true,
                false,
            ))),
    );
    let mut keys = div().col().gap(8.0);
    for k in &st.keys {
        let open = st.keys_open.contains(&k.id);
        let mut title = div().row().items_center().gap(8.0).child(
            text(&k.label, 14.0, c(look::GRAY_100))
                .weight(500)
                .line_height(20.0)
                .silent(),
        );
        if let Some(t) = &k.key_type {
            title = title.child(
                div().px(6.0).py(2.0).rounded(4.0).bg(white(0.06)).child(
                    text(t.to_uppercase(), 10.0, c(look::GRAY_400))
                        .weight(500)
                        .tracking(0.05)
                        .line_height(15.0)
                        .silent(),
                ),
            );
        }
        let sub = k.public.as_ref().map(|p| {
            text(truncate_key(p), 11.0, c(look::GRAY_600))
                .mono()
                .line_height(16.5)
                .mt(2.0)
                .truncate()
                .silent()
        });
        let mut parts = vec![collapsed_row(
            s,
            "ssh-key-row",
            SetAct::ToggleKey(k.id.clone()),
            tile("file-key", look::AMBER_400, look::AMBER_500),
            title,
            sub,
            open,
        )];
        if open {
            let mut detail = div().col().gap(12.0).px(16.0).pb(16.0).pt(4.0);
            if let Some(p) = &k.public {
                detail = detail.child(labelled(
                    "Public Key",
                    div()
                        .px(10.0)
                        .py(8.0)
                        .rounded(6.0)
                        .bg(crate::ui::theme::shadow(0.3))
                        .border(1.0, white(0.06))
                        .child(
                            text(p, 11.0, c(look::GRAY_500))
                                .mono()
                                .line_height(17.9)
                                .wrapping()
                                .silent(),
                        ),
                ));
            }
            let used: Vec<&Value> = hosts
                .iter()
                .filter(|h| {
                    s_of(h, "authMethod") == "key-stored" && s_of(h, "credentialId") == k.id
                })
                .collect();
            let used_el = if used.is_empty() {
                text("No hosts using this key", 11.0, c(look::GRAY_600))
                    .line_height(16.5)
                    .silent()
            } else {
                let mut chips = div().row().wrap_children().gap(6.0);
                for h in used {
                    chips = chips.child(
                        div()
                            .row()
                            .items_center()
                            .gap(6.0)
                            .px(8.0)
                            .py(4.0)
                            .rounded(6.0)
                            .bg(white(0.04))
                            .border(1.0, white(0.06))
                            .child(ico("server", 10.0, 1.5, c(look::BLUE_400)))
                            .child(
                                text(s_of(h, "label"), 11.0, c(look::GRAY_300))
                                    .line_height(16.5)
                                    .silent(),
                            ),
                    );
                }
                chips
            };
            detail = detail.child(div().col().child(label("Used By", 6.0)).child(used_el));
            let del = if st.confirm.as_deref() == Some(k.id.as_str()) {
                confirm_pair(s, "ssh-key-delete", SetAct::DeleteKey(k.id.clone()))
            } else {
                small_btn(
                    s,
                    "ssh-key-ask",
                    SetAct::AskDelete(k.id.clone()),
                    "Delete Key",
                    Some("trash-2"),
                    true,
                    true,
                )
            };
            detail = detail.child(div().row().justify_end().pt(8.0).child(del));
            parts.push(detail);
        }
        keys = keys.child(card(s, Id::new("ssh-key-card").child(&k.id), parts));
    }
    let mut keys_col = div()
        .col()
        .child(section("file-key", "SSH Keys", Some(st.keys.len())))
        .child(keys);
    if st.adding {
        keys_col = keys_col.child(key_form(s));
    }
    if st.importing {
        let id = key_field_id(KeyField::Import);
        keys_col = keys_col.child(
            div().mt(8.0).child(frame(
                s,
                id,
                field(
                    id,
                    "Key file to import",
                    "Path to a key file, then Enter",
                    style(true),
                    c(look::GRAY_200),
                )
                .placeholder_color(c(look::GRAY_600))
                .w_full(),
            )),
        );
    }
    let mut buttons = div().row().gap(8.0).mt(8.0).child(dashed_button(
        s,
        "ssh-import",
        SetAct::ImportKey,
        "upload",
        "Import Key",
        s.st.settings.safe_storage,
        true,
    ));
    if !st.adding {
        buttons = buttons.child(dashed_button(
            s,
            "ssh-paste",
            SetAct::PasteKey,
            "plus",
            "Paste Key",
            s.st.settings.safe_storage,
            true,
        ));
    }
    col.child(keys_col.child(buttons))
}

/// A host's stored-key select options.
pub(crate) fn ssh_key_items(s: &Screen<'_>, host: &str) -> Vec<crate::screen::menus::Item> {
    let cur = s
        .store
        .config
        .get("remoteHosts")
        .and_then(Value::as_array)
        .and_then(|l| l.iter().find(|h| s_of(h, "id") == host))
        .map(|h| s_of(h, "credentialId").to_owned())
        .unwrap_or_default();
    let mut items = vec![crate::screen::menus::Item::pick(
        "Select a key...",
        cur.is_empty(),
        set(SetAct::PickKey(host.to_owned(), String::new())),
    )];
    for k in &s.st.settings.ssh.keys {
        let label = k
            .key_type
            .as_ref()
            .map_or(k.label.clone(), |t| format!("{} ({t})", k.label));
        items.push(crate::screen::menus::Item::pick(
            label,
            cur == k.id,
            set(SetAct::PickKey(host.to_owned(), k.id.clone())),
        ));
    }
    items
}

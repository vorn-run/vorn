//! SSH: the remote hosts sessions run on and the SSH keys they log in with.
//!
//! Vorn ships it as a Wasm component (`vorn-extension.toml` beside this
//! crate). It draws the SSH & Hosts settings page as a tree and answers
//! `ssh:testConnection`. The hosts stay in Vorn's configuration and the
//! keys in its vault, both the core's, which every remote session reads
//! whether this is on or off; the component reads and writes them with the
//! `remote.*` methods and keeps only what the page shows open in `self`.
//!
//! A connection test takes up to ten seconds and the component answers one
//! call at a time, so the page's Test button never waits on it here:
//! `remote.test` with `then` answers at once and tells [`TESTED`] how it
//! went, and the page draws the test running meanwhile.

use std::collections::{BTreeMap, BTreeSet};

use vorn_extension_sdk::tree::sdk::*;
use vorn_extension_sdk::tree::{self, EventKind, Node, Surface, TextStyle, Tone};
use vorn_extension_sdk::{json, Ctx, Error, Extension, Host, Init, Op, Value};

pub mod hosts;

use hosts::Field;

/// The settings page.
pub const PAGE: &str = "hosts";
/// What an agent or a client calls to test a host, `{hostname, user,
/// port, sshKeyPath, sshOptions}` in, `{success, message, durationMs}` out.
pub const TEST_CONNECTION: &str = "ssh:testConnection";
/// The action vornd calls with a page test's result, `{key, result}`.
pub const TESTED: &str = "tested";
/// Its one row per host whose test changed, so the page is drawn again.
const TESTS: &str = "tests";

/// What a host's connection test said: still testing, or ok and why.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Test {
    Testing,
    Done { ok: bool, message: String },
}

/// A key being pasted in, as typed so far.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct NewKey {
    pub label: String,
    pub private: String,
    pub public: String,
    pub certificate: String,
}

/// What the page shows open; the hosts and keys themselves are the core's.
#[derive(Debug, Default)]
pub struct Ssh {
    pub open_hosts: BTreeSet<String>,
    pub open_keys: BTreeSet<String>,
    pub tests: BTreeMap<String, Test>,
    /// The Paste Key form, while it shows.
    pub adding: Option<NewKey>,
    /// Import Key's path field, while it shows.
    pub importing: bool,
    /// Tells apart the hosts added within one millisecond.
    added: u32,
}

fn hosts_of(host: &mut dyn Host) -> Result<Vec<Value>, Error> {
    match host.call("remote.hosts", Value::Null)? {
        Value::Array(list) => Ok(list),
        _ => Ok(Vec::new()),
    }
}

fn keys_of(host: &mut dyn Host) -> Result<Vec<Value>, Error> {
    match host.call("remote.keys", Value::Null)? {
        Value::Array(list) => Ok(list),
        _ => Ok(Vec::new()),
    }
}

fn save_hosts(host: &mut dyn Host, list: Vec<Value>) -> Result<(), Error> {
    host.call("remote.saveHosts", json!({ "hosts": list }))
        .map(drop)
}

fn now_ms() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |d| u64::try_from(d.as_millis()).unwrap_or(u64::MAX))
}

/// Redrawn: the page is asked for again.
fn redraw() -> Value {
    json!({})
}

/// Nothing the page draws changed.
fn same() -> Value {
    json!({ "same": true })
}

impl Ssh {
    /// Changes host `id` with `edit` and saves every host; `false` when
    /// there is no such host.
    fn write_host(
        host: &mut dyn Host,
        id: &str,
        edit: impl FnOnce(&mut Value),
    ) -> Result<bool, Error> {
        let mut list = hosts_of(host)?;
        let Some(h) = list.iter_mut().find(|h| hosts::id(h) == id) else {
            return Ok(false);
        };
        edit(h);
        save_hosts(host, list)?;
        Ok(true)
    }

    fn add_host(&mut self, host: &mut dyn Host) -> Result<Value, Error> {
        self.added = self.added.wrapping_add(1);
        let id = format!("host-{:x}-{:x}", now_ms(), self.added);
        let mut list = hosts_of(host)?;
        list.push(hosts::new_host(&id));
        save_hosts(host, list)?;
        self.open_hosts.insert(id);
        Ok(redraw())
    }

    /// Starts testing host `id`; the result comes back to [`TESTED`].
    fn test(&mut self, host: &mut dyn Host, id: &str) -> Result<Value, Error> {
        if self.tests.get(id) == Some(&Test::Testing) {
            return Ok(same());
        }
        let list = hosts_of(host)?;
        let Some(h) = list.iter().find(|h| hosts::id(h) == id) else {
            return Ok(same());
        };
        if let Some(why) = hosts::untestable(h) {
            self.tests.insert(
                id.to_owned(),
                Test::Done {
                    ok: false,
                    message: why.to_owned(),
                },
            );
            return Ok(redraw());
        }
        let asked = json!({ "host": h, "then": TESTED, "key": id });
        let test = match host.call("remote.test", asked) {
            Ok(_) => Test::Testing,
            Err(e) => Test::Done {
                ok: false,
                message: e.message,
            },
        };
        self.tests.insert(id.to_owned(), test);
        Ok(redraw())
    }

    /// A page test's result: kept, and the page told to draw it.
    fn tested(&mut self, host: &mut dyn Host, args: &Value) -> Value {
        let Some(id) = args["key"].as_str() else {
            return Value::Null;
        };
        let result = &args["result"];
        self.tests.insert(
            id.to_owned(),
            Test::Done {
                ok: result["success"] == json!(true),
                message: result["message"].as_str().unwrap_or_default().to_owned(),
            },
        );
        host.publish(TESTS, vec![Op::Upsert(json!({ "id": id, "at": now_ms() }))]);
        Value::Null
    }

    fn delete_host(&mut self, host: &mut dyn Host, id: &str) -> Result<Value, Error> {
        let mut list = hosts_of(host)?;
        list.retain(|h| hosts::id(h) != id);
        save_hosts(host, list)?;
        self.open_hosts.remove(id);
        self.tests.remove(id);
        Ok(redraw())
    }

    fn save_key(&mut self, host: &mut dyn Host) -> Result<Value, Error> {
        let Some(k) = &self.adding else {
            return Ok(same());
        };
        let private = k.private.trim();
        if private.is_empty() {
            return Ok(json!({ "error": "A key needs its private key" }));
        }
        let label = match k.label.trim() {
            "" => "Untitled Key",
            l => l,
        };
        let mut params = json!({ "label": label, "privateKey": private });
        for (typed, key) in [(&k.public, "publicKey"), (&k.certificate, "certificate")] {
            if !typed.trim().is_empty() {
                params[key] = json!(typed.trim());
            }
        }
        host.call("remote.storeKey", params)?;
        self.adding = None;
        Ok(redraw())
    }

    fn import_key(&mut self, host: &mut dyn Host, path: &str) -> Result<Value, Error> {
        let path = path.trim();
        if path.is_empty() {
            return Ok(same());
        }
        host.call("remote.importKey", json!({ "path": path }))?;
        self.importing = false;
        Ok(redraw())
    }

    fn delete_key(&mut self, host: &mut dyn Host, id: &str) -> Result<Value, Error> {
        host.call("remote.deleteKey", json!({ "id": id }))?;
        self.open_keys.remove(id);
        Ok(redraw())
    }

    /// What a node's event does: its id is `what` or `what:id`.
    fn page_event(&mut self, host: &mut dyn Host, e: &tree::Event) -> Result<Value, Error> {
        let (what, id) = e.id.split_once(':').unwrap_or((e.id.as_str(), ""));
        match (&e.kind, what) {
            (EventKind::Click, "add-host") => self.add_host(host),
            (EventKind::Toggle, "host") => {
                set_open(&mut self.open_hosts, id, &e.value);
                Ok(redraw())
            }
            (EventKind::Click, "test") => self.test(host, id),
            (EventKind::Click, "delete-host") => self.delete_host(host, id),
            (EventKind::Change, "new-key") => self.type_key(id, &e.value),
            (EventKind::Change, f) => {
                let Some(field) = Field::named(f) else {
                    return Ok(same());
                };
                Ssh::write_host(host, id, |h| hosts::set_field(h, field, &e.value))?;
                Ok(redraw())
            }
            (EventKind::Submit, "password") => {
                if e.value.is_empty() {
                    return Ok(same());
                }
                Ssh::write_host(host, id, |h| h["password"] = json!(e.value))?;
                Ok(json!({ "toast": "Password saved" }))
            }
            (EventKind::Select, "auth") => {
                let method = e.value.clone();
                if !hosts::AUTH_METHODS.iter().any(|(m, _)| *m == method) {
                    return Err(Error::bad_args(format!("no method {method}")));
                }
                Ssh::write_host(host, id, |h| hosts::set_auth(h, &method))?;
                Ok(redraw())
            }
            (EventKind::Select, "key") => {
                let key = if e.value == NO_KEY { "" } else { &e.value };
                Ssh::write_host(host, id, |h| h["credentialId"] = json!(key))?;
                Ok(redraw())
            }
            (EventKind::Submit, "browse-key") => {
                Ssh::write_host(host, id, |h| {
                    hosts::set_field(h, Field::KeyPath, &e.value);
                })?;
                Ok(redraw())
            }
            (EventKind::Toggle, "keyrow") => {
                set_open(&mut self.open_keys, id, &e.value);
                Ok(redraw())
            }
            (EventKind::Click, "delete-key") => self.delete_key(host, id),
            (EventKind::Click, "import") => {
                self.importing = !self.importing;
                Ok(redraw())
            }
            (EventKind::Submit, "import-path" | "import-browse") => self.import_key(host, &e.value),
            (EventKind::Click, "paste") => {
                self.adding = Some(NewKey::default());
                Ok(redraw())
            }
            (EventKind::Click, "cancel-key") => {
                self.adding = None;
                Ok(redraw())
            }
            (EventKind::Click, "save-key") => self.save_key(host),
            _ => Ok(same()),
        }
    }

    /// What was typed in the Paste Key form's field `which`; nothing it draws changes.
    fn type_key(&mut self, which: &str, typed: &str) -> Result<Value, Error> {
        let Some(k) = self.adding.as_mut() else {
            return Ok(same());
        };
        let kept = match which {
            "label" => &mut k.label,
            "private" => &mut k.private,
            "public" => &mut k.public,
            "certificate" => &mut k.certificate,
            other => return Err(Error::bad_args(format!("no key field {other}"))),
        };
        typed.clone_into(kept);
        Ok(same())
    }

    fn render_page(&self, host: &mut dyn Host) -> Result<Node, Error> {
        let (list, keys) = (hosts_of(host)?, keys_of(host)?);
        // Without an answer, as from a core before `remote.vault`, the keychain is taken as there.
        let keychain = host
            .call("remote.vault", Value::Null)
            .map_or(true, |v| v["keychain"] != json!(false));
        Ok(page(self, &list, &keys, keychain))
    }
}

/// The stored-key choice that picks none, first as 0.8's "Select a key...".
const NO_KEY: &str = "none";

/// Opens or closes row `id` of `open` as a disclosure's toggle says: `true` opens.
fn set_open(open: &mut BTreeSet<String>, id: &str, to: &str) {
    if to == "true" {
        open.insert(id.to_owned());
    } else {
        open.remove(id);
    }
}

/// A field's label over it, `gap` apart.
fn label_over(label: &str, gap: f32, input: Node) -> Node {
    col([text(label).style(TextStyle::Label), input.label(label)]).gap(gap)
}

/// A field's label over it.
fn labelled(label: &str, input: Node) -> Node {
    label_over(label, 4.0, input)
}

fn hint(line: &str) -> Node {
    text(line).style(TextStyle::Secondary)
}

/// What the page says when the OS keychain is missing, as 0.8 did; the
/// key vault's Import and Paste go with it.
pub const NO_KEYCHAIN: &str = "Keychain encryption is not available on this system. Install libsecret (gnome-keyring) to use the SSH key vault.";

/// The whole page: the keychain's warning when it is missing, the hosts,
/// then the keys.
pub fn page(ssh: &Ssh, list: &[Value], keys: &[Value], keychain: bool) -> Node {
    let hosts = list.iter().map(|h| host_entry(ssh, h, keys));
    let hosts = section(
        "Remote Hosts",
        [
            list_of("hosts", hosts, list.len(), "No remote hosts yet."),
            row([button("add-host", "Add Host").icon("plus")]),
        ],
    );
    let key_rows = keys.iter().map(|k| key_entry(ssh, k, list));
    let mut key_section = vec![list_of("keys", key_rows, keys.len(), "No stored keys yet.")];
    if let Some(k) = &ssh.adding {
        key_section.push(new_key(k));
    }
    if ssh.importing {
        // The path typed, as 0.8 took it, or one picked in the file dialog.
        key_section.push(
            row([
                field("import-path", "")
                    .placeholder("Path to a key file, then Enter")
                    .label("Key file to import")
                    .grow(),
                button("import-browse", "Browse…").picks_file("Import SSH key"),
            ])
            .gap(6.0),
        );
    }
    if keychain {
        let mut buttons = vec![button("import", "Import Key").icon("upload")];
        if ssh.adding.is_none() {
            buttons.push(button("paste", "Paste Key").icon("plus"));
        }
        key_section.push(row(buttons).gap(8.0));
    }
    let mut page = Vec::with_capacity(3);
    if !keychain {
        page.push(notice(NO_KEYCHAIN, Tone::Warn));
    }
    page.extend([hosts, section("SSH Keys", key_section)]);
    col(page).gap(24.0)
}

/// A list with its count, or what it says with nothing in it.
fn list_of(id: &str, rows: impl IntoIterator<Item = Node>, n: usize, empty: &str) -> Node {
    let rows: Vec<Node> = rows.into_iter().collect();
    if rows.is_empty() {
        return list(id, []).empty(empty);
    }
    let heading = if n == 1 {
        "1 configured".to_owned()
    } else {
        format!("{n} configured")
    };
    list(id, [group(heading, rows)])
}

/// A host's row; a click opens its form under it, as 0.8's card did.
fn host_entry(ssh: &Ssh, h: &Value, keys: &[Value]) -> Node {
    let id = hosts::id(h);
    let open = ssh.open_hosts.contains(id);
    let label = match hosts::text(h, "label") {
        "" => "Unnamed host",
        l => l,
    };
    let e = entry(format!("host:{id}"), label)
        .icon("server")
        .code(hosts::subtitle(h))
        .expanded(open);
    if open {
        e.child(host_form(ssh, h, keys, label))
    } else {
        e
    }
}

fn host_form(ssh: &Ssh, h: &Value, keys: &[Value], label: &str) -> Node {
    let id = hosts::id(h);
    let input = |f: Field, placeholder: &str| {
        let value = match f {
            Field::Port => hosts::port(h).to_string(),
            _ => hosts::text(h, f.key()).to_owned(),
        };
        field(format!("{}:{id}", f.key()), value)
            .placeholder(placeholder)
            .live()
    };
    let method = hosts::auth(h);
    let testing = ssh.tests.get(id) == Some(&Test::Testing);
    let test = button(format!("test:{id}"), "Test Connection").icon(if testing {
        "loader-circle"
    } else {
        "wifi"
    });
    let delete = button(format!("delete-host:{id}"), "Delete Host")
        .icon("trash-2")
        .danger()
        .confirm(format!(
            "Delete {label}? New sessions can no longer start on it."
        ));
    let mut form = vec![
        labelled("Label", input(Field::Label, "Host label")),
        row([
            labelled("Hostname", input(Field::Hostname, "e.g. dev.example.com")).grow(),
            labelled("Port", input(Field::Port, "22")).w(100.0),
        ])
        .gap(8.0),
        labelled("Username", input(Field::User, "e.g. ubuntu")),
        label_over(
            "Authentication",
            6.0,
            choice(format!("auth:{id}"), hosts::AUTH_METHODS, method),
        ),
        credential(h, method, keys),
        labelled(
            "Extra SSH Options",
            input(Field::Options, "e.g. -o ForwardAgent=yes"),
        ),
        row([test, delete]).between(),
    ];
    if let Some(Test::Done { ok, message }) = ssh.tests.get(id) {
        let tone = if *ok { Tone::Good } else { Tone::Bad };
        form.push(notice(message, tone));
    }
    col(form).gap(12.0)
}

/// What the host logs in with, by its method.
fn credential(h: &Value, method: &str, keys: &[Value]) -> Node {
    let id = hosts::id(h);
    match method {
        "password" => {
            let saved = !hosts::text(h, "encryptedPassword").is_empty();
            col([
                labelled(
                    "Password",
                    field(format!("password:{id}"), "")
                        .secret()
                        .submits_on_blur()
                        .placeholder(if saved { "(saved)" } else { "Enter password" }),
                ),
                hint("Encrypted with your OS keychain"),
            ])
            .gap(4.0)
        }
        "key-stored" => {
            let options: Vec<(String, String)> =
                std::iter::once((NO_KEY.to_owned(), "Select a key...".to_owned()))
                    .chain(
                        keys.iter()
                            .map(|k| (hosts::id(k).to_owned(), hosts::key_choice(k))),
                    )
                    .collect();
            let chosen = match hosts::text(h, "credentialId") {
                c if keys.iter().any(|k| hosts::id(k) == c) => c,
                _ => NO_KEY,
            };
            let picker = labelled(
                "Stored Key",
                choice(format!("key:{id}"), options, chosen).dropdown(),
            );
            if keys.is_empty() {
                col([picker, hint("No stored keys yet. Add one below.")]).gap(4.0)
            } else {
                picker
            }
        }
        "key-file" => labelled(
            "SSH Key Path",
            row([
                field(
                    format!("{}:{id}", Field::KeyPath.key()),
                    hosts::text(h, Field::KeyPath.key()),
                )
                .placeholder("~/.ssh/id_ed25519")
                .live()
                .grow(),
                button(format!("browse-key:{id}"), "Browse")
                    .icon("key")
                    .picks_file("Choose an SSH key"),
            ])
            .gap(6.0),
        ),
        _ => hint("Uses your running SSH agent (ssh-agent)"),
    }
}

/// A stored key's row; a click opens its public key, the hosts that use
/// it and Delete Key under it.
fn key_entry(ssh: &Ssh, k: &Value, list: &[Value]) -> Node {
    let id = hosts::id(k);
    let open = ssh.open_keys.contains(id);
    let label = match hosts::text(k, "label") {
        "" => "Untitled Key",
        l => l,
    };
    let public = hosts::text(k, "publicKey");
    let mut e = entry(format!("keyrow:{id}"), label)
        .icon("file-key")
        .code(hosts::truncate_key(public))
        .expanded(open);
    let kind = hosts::text(k, "keyType");
    if !kind.is_empty() {
        e = e.badge(kind.to_uppercase(), Tone::Neutral);
    }
    if !open {
        return e;
    }
    let users: Vec<&str> = list
        .iter()
        .filter(|h| hosts::auth(h) == "key-stored" && hosts::text(h, "credentialId") == id)
        .map(|h| hosts::text(h, "label"))
        .collect();
    let used = if users.is_empty() {
        hint("No hosts using this key")
    } else {
        row(users.into_iter().map(|u| badge(u, Tone::Neutral)))
            .gap(6.0)
            .wrap()
    };
    let mut detail = Vec::with_capacity(3);
    if !public.is_empty() {
        detail.push(labelled("Public Key", text(public).style(TextStyle::Mono)));
    }
    detail.push(label_over("Used By", 6.0, used));
    detail.push(
        row([button(format!("delete-key:{id}"), "Delete Key")
            .icon("trash-2")
            .danger()
            .confirm(format!(
                "Delete {label}? Hosts that use it can no longer log in with it."
            ))])
        .end(),
    );
    e.child(col(detail).gap(12.0))
}

fn new_key(k: &NewKey) -> Node {
    let input = |which: &str, value: &str| field(format!("new-key:{which}"), value).live();
    card([
        text("New Key").style(TextStyle::Title),
        labelled(
            "Label",
            input("label", &k.label).placeholder("e.g. Work Server Key"),
        ),
        labelled(
            "Private key *",
            input("private", &k.private)
                .lines(3)
                .placeholder("-----BEGIN OPENSSH PRIVATE KEY-----"),
        ),
        labelled(
            "Public key",
            input("public", &k.public)
                .lines(2)
                .placeholder("ssh-ed25519 AAAA..."),
        ),
        labelled(
            "Certificate",
            input("certificate", &k.certificate)
                .lines(2)
                .placeholder("Optional"),
        ),
        hint("If your key has a passphrase, remove it first with ssh-keygen -p"),
        row([
            button("save-key", "Save Key").primary(),
            button("cancel-key", "Cancel"),
        ])
        .gap(8.0),
    ])
}

impl Extension for Ssh {
    fn init(_: &Init, _: &mut dyn Host) -> Result<Self, Error> {
        Ok(Ssh::default())
    }

    fn handle(
        &mut self,
        host: &mut dyn Host,
        method: &str,
        args: Value,
        _: &Ctx,
    ) -> Result<Value, Error> {
        match method {
            // The host itself, as before; the test answers within its ten seconds.
            TEST_CONNECTION => host.call("remote.test", json!({ "host": args })),
            TESTED => Ok(self.tested(host, &args)),
            other => Err(Error::unknown(other)),
        }
    }

    fn render(
        &mut self,
        host: &mut dyn Host,
        point: &str,
        _: Surface,
    ) -> Result<tree::Node, Error> {
        match point {
            PAGE => self.render_page(host),
            _ => Err(Error::new(
                "unknown-action",
                format!("nothing draws {point}"),
            )),
        }
    }

    fn event(&mut self, host: &mut dyn Host, point: &str, e: tree::Event) -> Result<Value, Error> {
        if point != PAGE {
            return Err(Error::new(
                "unknown-action",
                format!("{point} has no {}", e.id),
            ));
        }
        self.page_event(host, &e)
    }
}

vorn_extension_sdk::export!(Ssh);

#[cfg(test)]
mod tests;

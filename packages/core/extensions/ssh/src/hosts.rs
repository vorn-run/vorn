//! A remote host as the configuration keeps it (`remoteHosts`, one JSON
//! object each), and the rules editing one follows: what a typed port
//! means, which credential an authentication method keeps, and what a
//! connection test needs first.

use vorn_extension_sdk::{json, Value};

/// How a host logs in, `(id, label)`, in the order the page offers them.
pub const AUTH_METHODS: [(&str, &str); 4] = [
    ("agent", "SSH Agent"),
    ("password", "Password"),
    ("key-stored", "Stored Key"),
    ("key-file", "Key File"),
];

/// The port a host without one, or with nonsense, connects on.
pub const DEFAULT_PORT: i64 = 22;

/// What a test of a host without its address says.
pub const NEEDS_ADDRESS: &str = "Hostname and username are required";

/// A text field of a host, by the key it is kept under.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Field {
    Label,
    Hostname,
    Port,
    User,
    KeyPath,
    Options,
}

impl Field {
    pub const ALL: [Field; 6] = [
        Field::Label,
        Field::Hostname,
        Field::Port,
        Field::User,
        Field::KeyPath,
        Field::Options,
    ];

    /// The key the host keeps it under, also its node's id before the host's.
    pub fn key(self) -> &'static str {
        match self {
            Field::Label => "label",
            Field::Hostname => "hostname",
            Field::Port => "port",
            Field::User => "user",
            Field::KeyPath => "sshKeyPath",
            Field::Options => "sshOptions",
        }
    }

    pub fn named(key: &str) -> Option<Field> {
        Field::ALL.into_iter().find(|f| f.key() == key)
    }
}

/// A host's text at `key`, empty when it has none.
pub fn text<'a>(host: &'a Value, key: &str) -> &'a str {
    host.get(key).and_then(Value::as_str).unwrap_or_default()
}

/// A host's id.
pub fn id(host: &Value) -> &str {
    text(host, "id")
}

/// The port a host connects on.
#[allow(clippy::cast_possible_truncation)] // a port is far inside i64
pub fn port(host: &Value) -> i64 {
    host.get("port")
        .and_then(Value::as_f64)
        .map_or(DEFAULT_PORT, |p| p as i64)
}

/// What typing `typed` in the port field keeps: a number, `22` for none or 0.
pub fn parse_port(typed: &str) -> i64 {
    typed
        .trim()
        .parse::<i64>()
        .ok()
        .filter(|p| *p != 0)
        .unwrap_or(DEFAULT_PORT)
}

/// A host's authentication method; `agent` when it names none.
pub fn auth(host: &Value) -> &str {
    host.get("authMethod")
        .and_then(Value::as_str)
        .filter(|m| !m.is_empty())
        .unwrap_or("agent")
}

/// A new host, called `New Host`, logging in with the SSH agent.
pub fn new_host(id: &str) -> Value {
    json!({
        "id": id, "label": "New Host", "hostname": "", "user": "",
        "port": DEFAULT_PORT, "authMethod": "agent",
    })
}

/// Keeps what was typed in `field`.
pub fn set_field(host: &mut Value, field: Field, typed: &str) {
    host[field.key()] = match field {
        Field::Port => json!(parse_port(typed)),
        _ => json!(typed),
    };
}

/// Switches the host to `method`, dropping the credential the others kept:
/// a key file's path, a stored key's id, a saved password.
pub fn set_auth(host: &mut Value, method: &str) {
    host["authMethod"] = json!(method);
    if let Some(o) = host.as_object_mut() {
        if method != "key-file" {
            o.remove("sshKeyPath");
        }
        if method != "key-stored" {
            o.remove("credentialId");
        }
        if method != "password" {
            o.remove("encryptedPassword");
        }
    }
}

/// Why a host cannot be tested yet, if it cannot.
pub fn untestable(host: &Value) -> Option<&'static str> {
    (text(host, "hostname").is_empty() || text(host, "user").is_empty()).then_some(NEEDS_ADDRESS)
}

/// A host's subtitle: `user@host:port`, or what is there of it.
pub fn subtitle(host: &Value) -> String {
    let (user, name, port) = (text(host, "user"), text(host, "hostname"), port(host));
    match (user.is_empty(), name.is_empty()) {
        (false, false) if port == DEFAULT_PORT => format!("{user}@{name}"),
        (false, false) => format!("{user}@{name}:{port}"),
        (_, false) => name.to_owned(),
        _ => "Not configured".to_owned(),
    }
}

/// A long public key with its middle cut out.
pub fn truncate_key(key: &str) -> String {
    let n = key.chars().count();
    if n <= 60 {
        return key.to_owned();
    }
    let head: String = key.chars().take(30).collect();
    let tail: String = key.chars().skip(n - 25).collect();
    format!("{head}...{tail}")
}

/// A stored key as the host's key choice names it: `label (type)`.
pub fn key_choice(key: &Value) -> String {
    let label = text(key, "label");
    match text(key, "keyType") {
        "" => label.to_owned(),
        kind => format!("{label} ({kind})"),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_port_is_a_number_and_anything_else_is_22() {
        assert_eq!(parse_port(" 2222 "), 2222);
        assert_eq!(parse_port(""), 22);
        assert_eq!(parse_port("0"), 22);
        assert_eq!(parse_port("ssh"), 22);
        let mut h = new_host("h");
        set_field(&mut h, Field::Port, "abc");
        assert_eq!(h["port"], json!(22));
        set_field(&mut h, Field::Port, "2200");
        assert_eq!(port(&h), 2200);
    }

    #[test]
    fn switching_the_method_drops_the_credentials_it_does_not_use() {
        let mut h = json!({
            "id": "h", "authMethod": "password", "encryptedPassword": "in-vault",
            "sshKeyPath": "~/.ssh/id", "credentialId": "k1",
        });
        set_auth(&mut h, "key-file");
        assert_eq!(h["sshKeyPath"], "~/.ssh/id");
        assert!(h.get("encryptedPassword").is_none());
        assert!(h.get("credentialId").is_none());
        set_auth(&mut h, "agent");
        assert_eq!(h, json!({ "id": "h", "authMethod": "agent" }));
    }

    #[test]
    fn a_host_is_tested_only_with_its_hostname_and_user() {
        let mut h = new_host("h");
        assert_eq!(untestable(&h), Some(NEEDS_ADDRESS));
        set_field(&mut h, Field::Hostname, "dev.example.com");
        assert_eq!(untestable(&h), Some(NEEDS_ADDRESS));
        set_field(&mut h, Field::User, "me");
        assert_eq!(untestable(&h), None);
    }

    #[test]
    fn the_subtitle_says_what_there_is_of_the_address() {
        let mut h = new_host("h");
        assert_eq!(subtitle(&h), "Not configured");
        set_field(&mut h, Field::User, "me");
        assert_eq!(subtitle(&h), "Not configured");
        set_field(&mut h, Field::Hostname, "box");
        assert_eq!(subtitle(&h), "me@box");
        set_field(&mut h, Field::Port, "2222");
        assert_eq!(subtitle(&h), "me@box:2222");
        h["user"] = json!("");
        assert_eq!(subtitle(&h), "box");
    }

    #[test]
    fn a_long_key_keeps_its_ends() {
        assert_eq!(truncate_key("ssh-ed25519 AAAA"), "ssh-ed25519 AAAA");
        let long = format!("ssh-rsa {} me@box", "A".repeat(80));
        let cut = truncate_key(&long);
        assert!(cut.starts_with("ssh-rsa AAAA") && cut.ends_with("me@box"));
        assert_eq!(cut.chars().count(), 30 + 3 + 25);
    }

    #[test]
    fn every_field_is_named_by_its_key() {
        for f in Field::ALL {
            assert_eq!(Field::named(f.key()), Some(f));
        }
        assert_eq!(Field::named("password"), None);
        assert_eq!(key_choice(&json!({ "label": "Work" })), "Work");
        assert_eq!(
            key_choice(&json!({ "label": "Work", "keyType": "ed25519" })),
            "Work (ed25519)"
        );
    }
}

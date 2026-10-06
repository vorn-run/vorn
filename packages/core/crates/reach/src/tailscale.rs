//! Tailscale's status, as `tailscale status --json` reports it, in the shape
//! the app shows: whether it is installed and running, this machine's
//! address and name, the peers, and where the web client is on the tailnet.
//!
//! Running the program is the caller's; this reads what it printed.

use serde_json::{json, Map, Value};

/// Where the program is looked for first, in order, before `PATH`.
pub const BINARY_PATHS: &[&str] = &[
    // The macOS app.
    "/Applications/Tailscale.app/Contents/MacOS/Tailscale",
    // Homebrew and the open-source build.
    "/usr/local/bin/tailscale",
];

/// The status when no program was found.
pub fn not_installed() -> Value {
    json!({
        "installed": false,
        "running": false,
        "backendState": "NotInstalled",
        "selfIP": "",
        "selfDNSName": "",
        "peers": [],
    })
}

/// The status when the program failed or printed something unreadable.
pub fn failed() -> Value {
    json!({
        "installed": true,
        "running": false,
        "backendState": "Error",
        "selfIP": "",
        "selfDNSName": "",
        "peers": [],
    })
}

/// Reads what `tailscale status --json` printed (trimmed). `app_port` is
/// the server's port, for the web client's URL. Anything that does not read
/// as a status is [`failed`].
pub fn status(printed: &str, app_port: Option<u16>) -> Value {
    serde_json::from_str::<Value>(printed)
        .ok()
        .and_then(|raw| read(&raw, app_port))
        .unwrap_or_else(failed)
}

/// `value[0]` as JavaScript reads it: an array's first element, a string's
/// first UTF-16 unit, or nothing.
fn first(value: Option<&Value>) -> Option<Value> {
    match value? {
        Value::Array(items) => items.first().cloned(),
        Value::String(s) => {
            let unit = s.encode_utf16().next()?;
            Some(Value::String(String::from_utf16_lossy(&[unit])))
        }
        Value::Object(map) => map.get("0").cloned(),
        _ => None,
    }
}

/// `x ?? fallback`.
fn or(value: Option<Value>, fallback: Value) -> Value {
    value.filter(|v| !v.is_null()).unwrap_or(fallback)
}

/// A field as `obj?.key` reads it: nothing from a value that is not an object.
fn field<'a>(obj: Option<&'a Value>, key: &str) -> Option<&'a Value> {
    match obj? {
        Value::Object(map) => map.get(key),
        _ => None,
    }
}

/// `(name ?? '').replace(/\.$/, '')`; `None` where JavaScript would throw.
fn dns_name(value: Option<&Value>) -> Option<Value> {
    match value {
        None | Some(Value::Null) => Some(Value::String(String::new())),
        Some(Value::String(s)) => Some(Value::String(s.strip_suffix('.').unwrap_or(s).to_owned())),
        Some(_) => None,
    }
}

fn read(raw: &Value, app_port: Option<u16>) -> Option<Value> {
    // `null.BackendState` throws; any other value reads as having no fields.
    if raw.is_null() {
        return None;
    }
    let backend = field(Some(raw), "BackendState").cloned();
    let running = backend.as_ref().and_then(Value::as_str) == Some("Running");
    let own = field(Some(raw), "Self");
    let self_ip = or(
        first(field(Some(raw), "TailscaleIPs")).or_else(|| first(field(own, "TailscaleIPs"))),
        Value::String(String::new()),
    );
    let self_dns = dns_name(field(own, "DNSName"))?;

    // `Object.values(status.Peer ?? {})`.
    let listed: Vec<Value> = match field(Some(raw), "Peer") {
        None | Some(Value::Null | Value::Bool(_) | Value::Number(_)) => Vec::new(),
        Some(Value::Object(map)) => map.values().cloned().collect(),
        Some(Value::Array(items)) => items.clone(),
        Some(Value::String(s)) => s
            .encode_utf16()
            .map(|u| Value::String(String::from_utf16_lossy(&[u])))
            .collect(),
    };
    let mut peers = Vec::new();
    for peer in &listed {
        // `p.HostName` on null throws.
        if peer.is_null() {
            return None;
        }
        let mut out = Map::new();
        out.insert(
            "ip".into(),
            or(
                first(field(Some(peer), "TailscaleIPs")),
                Value::String(String::new()),
            ),
        );
        if let Some(v) = field(Some(peer), "HostName") {
            out.insert("hostname".into(), v.clone());
        }
        out.insert("dnsName".into(), dns_name(field(Some(peer), "DNSName"))?);
        if let Some(v) = field(Some(peer), "OS") {
            out.insert("os".into(), v.clone());
        }
        let online = field(Some(peer), "Online").is_some_and(truthy);
        out.insert("online".into(), Value::Bool(online));
        peers.push(Value::Object(out));
    }

    let mut out = Map::new();
    out.insert("installed".into(), Value::Bool(true));
    out.insert("running".into(), Value::Bool(running));
    if let Some(b) = backend {
        out.insert("backendState".into(), b);
    }
    out.insert("selfIP".into(), self_ip.clone());
    out.insert("selfDNSName".into(), self_dns);
    if let Some(os) = field(own, "OS") {
        out.insert("selfOS".into(), os.clone());
    }
    out.insert("peers".into(), Value::Array(peers));
    if let (true, Some(port)) = (running, app_port.filter(|&p| p != 0)) {
        let ip = match &self_ip {
            Value::String(s) => s.clone(),
            other => other.to_string(),
        };
        out.insert(
            "appUrl".into(),
            Value::String(format!("http://{ip}:{port}/app/")),
        );
    }
    Some(Value::Object(out))
}

/// JavaScript truthiness, for `!!value`.
fn truthy(v: &Value) -> bool {
    match v {
        Value::Null => false,
        Value::Bool(b) => *b,
        Value::Number(n) => n.as_f64().is_some_and(|f| f != 0.0 && !f.is_nan()),
        Value::String(s) => !s.is_empty(),
        Value::Array(_) | Value::Object(_) => true,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const RUNNING: &str = r#"{
        "BackendState": "Running",
        "TailscaleIPs": ["100.64.0.1", "fd7a::1"],
        "Self": { "HostName": "mac", "DNSName": "mac.tail1.ts.net.", "OS": "macOS", "TailscaleIPs": ["100.64.0.9"] },
        "Peer": {
            "nodekey:b": { "HostName": "phone", "DNSName": "phone.tail1.ts.net.", "OS": "iOS", "TailscaleIPs": ["100.64.0.2"], "Online": true },
            "nodekey:a": { "HostName": "nas", "DNSName": "", "OS": "linux" }
        }
    }"#;

    #[test]
    fn reads_a_running_status() {
        assert_eq!(
            status(RUNNING, Some(3456)),
            json!({
                "installed": true, "running": true, "backendState": "Running",
                "selfIP": "100.64.0.1", "selfDNSName": "mac.tail1.ts.net", "selfOS": "macOS",
                "peers": [
                    { "ip": "100.64.0.2", "hostname": "phone", "dnsName": "phone.tail1.ts.net", "os": "iOS", "online": true },
                    { "ip": "", "hostname": "nas", "dnsName": "", "os": "linux", "online": false }
                ],
                "appUrl": "http://100.64.0.1:3456/app/"
            })
        );
        // Peers stay in the order the program printed them.
        let peers = status(RUNNING, None)["peers"].clone();
        assert_eq!(peers[0]["hostname"], "phone");
        assert!(status(RUNNING, None).get("appUrl").is_none());
    }

    #[test]
    fn a_stopped_tailscale_has_no_app_url() {
        let s = status(r#"{"BackendState":"Stopped","Self":{}}"#, Some(1));
        assert_eq!(
            s,
            json!({
                "installed": true, "running": false, "backendState": "Stopped",
                "selfIP": "", "selfDNSName": "", "peers": []
            })
        );
    }

    #[test]
    fn reads_odd_shapes_as_javascript_does() {
        assert_eq!(
            status("[]", None),
            json!({ "installed": true, "running": false, "selfIP": "", "selfDNSName": "", "peers": [] })
        );
        assert_eq!(
            status(r#"{"TailscaleIPs":"abc","Peer":[1]}"#, None)["selfIP"],
            "a"
        );
        assert_eq!(
            status(r#"{"Peer":[1]}"#, None)["peers"],
            json!([{ "ip": "", "dnsName": "", "online": false }])
        );
    }

    #[test]
    fn something_unreadable_is_an_error_status() {
        for printed in [
            "",
            "not json",
            "null",
            r#"{"Peer":{"x":null}}"#,
            r#"{"Self":{"DNSName":3}}"#,
        ] {
            assert_eq!(status(printed, Some(1)), failed(), "{printed}");
        }
    }
}

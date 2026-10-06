//! Where the web client can be reached right now.
//!
//! Addresses are enumerated on every call rather than remembered: they
//! change on every DHCP renewal, network switch and VPN toggle, with no event
//! for any of it.

use serde_json::{json, Value};

/// `{urls, port, remote}`: every address a browser on another machine can
/// use, Tailscale's first (the only encrypted one), none while the server is
/// bound to loopback (`remote` false).
pub fn reachable_urls(port: u16, remote: bool, tailscale_ips: &[String], lan: &[String]) -> Value {
    if !remote {
        return json!({ "urls": [], "port": port, "remote": false });
    }
    let mut seen: Vec<&str> = Vec::new();
    let mut urls = Vec::new();
    for host in tailscale_ips
        .iter()
        .filter(|h| !h.is_empty())
        .chain(lan.iter())
    {
        if seen.contains(&host.as_str()) {
            continue;
        }
        seen.push(host);
        urls.push(format!("http://{host}:{port}/app/"));
    }
    json!({ "urls": urls, "port": port, "remote": true })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn nothing_while_bound_to_loopback() {
        assert_eq!(
            reachable_urls(3456, false, &["100.1.1.1".into()], &["10.0.0.2".into()]),
            json!({ "urls": [], "port": 3456, "remote": false })
        );
    }

    #[test]
    fn tailscale_first_then_the_lan_without_repeats() {
        assert_eq!(
            reachable_urls(
                3456,
                true,
                &["100.1.1.1".into(), String::new()],
                &["10.0.0.2".into(), "100.1.1.1".into(), "192.168.0.4".into()]
            ),
            json!({
                "urls": [
                    "http://100.1.1.1:3456/app/",
                    "http://10.0.0.2:3456/app/",
                    "http://192.168.0.4:3456/app/"
                ],
                "port": 3456,
                "remote": true
            })
        );
    }
}

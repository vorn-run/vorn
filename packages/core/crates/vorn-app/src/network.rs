//! The Remote Access page's model (`NetworkSettings`): sharing this machine,
//! pairing devices with a code, and using another machine from here.

use std::time::{Duration, Instant};

use serde::Deserialize;

use crate::ui::Id;

/// "What is this device?": the name a minted token carries.
pub const DEVICE_NAME: Id = Id(0x6e65_745f_6465_7669);
/// "Server address" on the use-another-machine form.
pub const HOST_URL: Id = Id(0x6e65_745f_686f_7374);
/// "Device token from that machine" on that form.
pub const HOST_TOKEN: Id = Id(0x6e65_745f_746f_6b6e);

/// Where "Add Tailscale" leads.
pub const TAILSCALE_DOWNLOAD: &str = "https://tailscale.com/download";

/// How long a copy button says "Copied" (`CopyButton`'s timeout).
pub const COPIED_FOR: Duration = Duration::from_secs(2);

/// How long after the share switch the addresses are asked again: vornd
/// rebinds on the config change, which takes a moment.
pub const REBIND_SETTLE: Duration = Duration::from_millis(500);

/// The use-another-machine form's error when a field is empty.
pub const BOTH_NEEDED: &str = "Both an address and a token are needed.";

/// What the form says when both fields are filled: this app always talks
/// to the vornd on this machine, so it cannot switch to a host.
pub const NO_HOST_MODE: &str = "This app connects only to the Vorn on this machine for now.";

/// Which half of the page is on screen (`Mode` in `NetworkSettings`).
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Hash)]
pub enum Mode {
    /// "This machine": share it with other devices.
    #[default]
    Local,
    /// "Another machine": point this desktop at a Vorn elsewhere.
    Host,
}

/// What a control on the Remote Access page does.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub enum NetAct {
    /// Loads what the page shows.
    Refresh,
    /// The segmented control at the top.
    View(Mode),
    /// The "Share this machine" switch, asking for this value.
    Share(bool),
    /// "Enable anyway" under the warning shown without Tailscale.
    EnableAnyway,
    /// "Cancel" under that warning.
    CancelShare,
    /// "N other addresses" / "Hide".
    ToggleAddresses,
    /// A copy button, which says "Copied" for a moment.
    Copy(String),
    /// "Add device": a code when a phone could reach this machine, else
    /// the token form.
    AddDevice,
    /// "Show another" once a code expired.
    ShowAnother,
    /// "Pair manually instead": retires the code for the token form.
    PairManually,
    /// "Cancel" while adding: retires any code.
    CloseAdding,
    /// Picks the address the code carries.
    ChooseUrl(String),
    /// "Create token" (or Enter in the name field).
    CreateToken,
    /// "Done" under a freshly minted token.
    DoneMinted,
    /// "Remove" on a device, which asks first.
    AskRevoke(String),
    /// "Cancel" on that question.
    CancelRevoke,
    /// "Revoke": the device's token stops working.
    Revoke(String),
    /// Approves (`true`) or denies the phone asking to pair.
    Decide(bool),
    /// "Connect and restart" on the use-another-machine form.
    Connect,
}

/// `TailscaleStatus`, the parts the page reads.
#[derive(Debug, Clone, Default, PartialEq, Eq, Deserialize)]
#[serde(default)]
pub struct Tailscale {
    pub installed: bool,
    pub running: bool,
}

impl Tailscale {
    /// What the page shows when the status could not be read.
    pub fn failed() -> Tailscale {
        Tailscale::default()
    }
}

/// `ReachableUrls`: where a browser on another device reaches this
/// machine, tailnet first.
#[derive(Debug, Clone, Default, PartialEq, Eq, Deserialize)]
#[serde(default)]
pub struct Reachable {
    pub urls: Vec<String>,
    pub port: u16,
    /// vornd listens beyond loopback.
    pub remote: bool,
}

/// `DeviceToken`, as `token:list` lists it.
#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DeviceToken {
    pub id: String,
    pub name: String,
    #[serde(default)]
    pub last_seen_at: Option<String>,
    #[serde(default)]
    pub revoked_at: Option<String>,
}

impl DeviceToken {
    /// The line under the device's name.
    pub fn seen(&self) -> String {
        match self.last_seen_at.as_deref().filter(|s| !s.is_empty()) {
            Some(at) => format!("Last seen {}", crate::tasks::locale_string(at)),
            None => "Never connected".to_owned(),
        }
    }
}

/// `PairingRequest`: a phone that typed or scanned the code.
#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PairingRequest {
    pub request_id: String,
    #[serde(default)]
    pub device_name: String,
    #[serde(default)]
    pub address: String,
}

/// A pairing code on screen and when vornd forgets it (ms since the epoch).
#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Code {
    pub code: String,
    pub expires_at: u64,
}

impl Code {
    /// What is left of it at `now_ms`.
    pub fn remaining(&self, now_ms: u64) -> u64 {
        self.expires_at.saturating_sub(now_ms)
    }
}

/// How a device is being added, if one is.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Adding {
    /// A code to scan or type; past its deadline it reads "That code expired".
    Scan(Code),
    /// The form that mints a token to copy.
    Token,
}

/// A token just minted: its plaintext is shown this once.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Minted {
    pub name: String,
    pub plaintext: String,
}

/// What the Remote Access page knows beyond vornd's models.
#[derive(Debug, Default)]
pub struct Network {
    /// The page was open last frame; closing it retires any code.
    pub open: bool,
    /// A status fetch is on its way.
    pub loading: bool,
    pub status: Option<Tailscale>,
    pub reachable: Option<Reachable>,
    pub viewing: Mode,
    /// The share switch is on its way up without a tailnet.
    pub confirming: bool,
    /// Every address shows, not only the first.
    pub show_all: bool,
    /// `None` until `token:list` first answers.
    pub tokens: Option<Vec<DeviceToken>>,
    pub adding: Option<Adding>,
    pub asking: Option<PairingRequest>,
    /// The address the code carries, when the user picked one.
    pub chosen_url: Option<String>,
    pub minted: Option<Minted>,
    /// The device whose "Remove" asks to confirm.
    pub confirm_revoke: Option<String>,
    /// The device list's error line.
    pub error: Option<String>,
    /// The use-another-machine form's error line.
    pub connect_error: Option<String>,
    /// The text a copy button copied, and when.
    pub copied: Option<(String, Instant)>,
    /// When to ask for the addresses again after the share switch.
    pub recheck_at: Option<Instant>,
}

impl Network {
    /// The devices not revoked.
    pub fn active(&self) -> Vec<&DeviceToken> {
        self.tokens
            .iter()
            .flatten()
            .filter(|t| t.revoked_at.is_none())
            .collect()
    }

    /// A phone can redeem a code only when vornd listens beyond loopback.
    pub fn can_scan(&self) -> bool {
        self.reachable
            .as_ref()
            .is_some_and(|r| r.remote && !r.urls.is_empty())
    }

    /// The address the code carries: the chosen one while it is still
    /// offered, else the first.
    pub fn pair_url(&self) -> Option<&str> {
        let urls = self.reachable.as_ref().map_or(&[][..], |r| &r.urls[..]);
        self.chosen_url
            .as_deref()
            .filter(|u| urls.iter().any(|x| x == u))
            .or_else(|| urls.first().map(String::as_str))
    }

    /// Whether the copy button for `text` says "Copied" at `now`.
    pub fn copied(&self, text: &str, now: Instant) -> bool {
        self.copied
            .as_ref()
            .is_some_and(|(t, at)| t == text && now.duration_since(*at) < COPIED_FOR)
    }

    /// Puts away what the device list was doing (`closeAdding`).
    pub fn close_adding(&mut self) {
        self.adding = None;
        self.asking = None;
        self.chosen_url = None;
    }

    /// What the page forgets when it closes.
    pub fn closed(&mut self) {
        *self = Network::default();
    }
}

/// Which network an address reaches this machine over (`addressKind`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AddressKind {
    /// Tailscale's 100.64.0.0/10, encrypted.
    Tailnet,
    /// Anything else: this network only.
    Lan,
}

/// `addressKind`: Tailscale hands out addresses from 100.64.0.0/10 and uses
/// it for nothing else, so the split is exact.
pub fn address_kind(url: &str) -> AddressKind {
    let Some(host) = hostname(url) else {
        return AddressKind::Lan;
    };
    let mut octets = host.split('.').map(|p| p.parse::<f64>().ok());
    match (octets.next().flatten(), octets.next().flatten()) {
        (Some(a), Some(b)) if a == 100.0 && (64.0..=127.0).contains(&b) => AddressKind::Tailnet,
        _ => AddressKind::Lan,
    }
}

/// `addressHost`: the host alone, which tells two routes of one kind apart.
pub fn address_host(url: &str) -> String {
    hostname(url).unwrap_or_else(|| url.to_owned())
}

/// `new URL(url).hostname`, for the absolute URLs vornd hands out; `None`
/// where `new URL` would throw (no scheme, no host).
fn hostname(url: &str) -> Option<String> {
    let (scheme, rest) = url.trim().split_once("://")?;
    let valid_scheme = scheme.starts_with(|c: char| c.is_ascii_alphabetic())
        && scheme
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || matches!(c, '+' | '-' | '.'));
    if !valid_scheme {
        return None;
    }
    let authority = rest.split(['/', '?', '#']).next().unwrap_or_default();
    let host_port = authority.rsplit_once('@').map_or(authority, |(_, h)| h);
    let host = if host_port.starts_with('[') {
        host_port.find(']').map(|end| &host_port[..=end])?
    } else {
        host_port.split(':').next().unwrap_or_default()
    };
    (!host.is_empty()).then(|| host.to_ascii_lowercase())
}

/// `formatRemaining`: whole seconds left, rounded up, as `m:ss`.
pub fn format_remaining(ms: u64) -> String {
    let total = ms.div_ceil(1000);
    format!("{}:{:02}", total / 60, total % 60)
}

/// The link the pairing code stands for: `vorn://pair?url=…&code=…`.
pub fn pair_link(url: &str, code: &str) -> String {
    format!("vorn://pair?url={}&code={code}", crate::runs::encode(url))
}

/// The device list's heading: "Devices", with the count when there are any.
pub fn devices_label(active: usize) -> String {
    if active > 0 {
        format!("Devices · {active}")
    } else {
        "Devices".to_owned()
    }
}

/// The toggle under the first address: "Hide", or how many others there are.
pub fn others_label(others: usize, showing: bool) -> String {
    if showing {
        "Hide".to_owned()
    } else {
        format!(
            "{others} other address{}",
            if others > 1 { "es" } else { "" }
        )
    }
}

/// What the use-another-machine form says for these fields.
pub fn connect_error(url: &str, token: &str) -> &'static str {
    if url.trim().is_empty() || token.trim().is_empty() {
        BOTH_NEEDED
    } else {
        NO_HOST_MODE
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn tailnet_is_exactly_the_cgnat_range() {
        assert_eq!(
            address_kind("http://100.64.0.1:3000/app/"),
            AddressKind::Tailnet
        );
        assert_eq!(address_kind("http://100.127.255.9/"), AddressKind::Tailnet);
        assert_eq!(address_kind("http://100.63.0.1/"), AddressKind::Lan);
        assert_eq!(address_kind("http://100.128.0.1/"), AddressKind::Lan);
        assert_eq!(
            address_kind("http://192.168.1.4:61601/app/"),
            AddressKind::Lan
        );
        assert_eq!(
            address_kind("http://machine.tail.ts.net/"),
            AddressKind::Lan
        );
        assert_eq!(address_kind("not a url"), AddressKind::Lan);
        assert_eq!(address_kind("100.64.0.1:3000"), AddressKind::Lan);
    }

    #[test]
    fn hosts_drop_scheme_port_path_and_userinfo() {
        assert_eq!(address_host("http://192.168.1.4:61601/app/"), "192.168.1.4");
        assert_eq!(address_host("http://u:p@Host.Local:1/x?y#z"), "host.local");
        assert_eq!(address_host("http://[fe80::1]:80/"), "[fe80::1]");
        assert_eq!(address_host("192.168.0.4:61601"), "192.168.0.4:61601");
        assert_eq!(address_host("http:///app"), "http:///app");
    }

    #[test]
    fn the_countdown_rounds_up_to_whole_seconds() {
        assert_eq!(format_remaining(300_000), "5:00");
        assert_eq!(format_remaining(299_001), "5:00");
        assert_eq!(format_remaining(59_000), "0:59");
        assert_eq!(format_remaining(1), "0:01");
        assert_eq!(format_remaining(0), "0:00");
        let code = Code {
            code: "ABCD".into(),
            expires_at: 10_000,
        };
        assert_eq!(code.remaining(4_000), 6_000);
        assert_eq!(code.remaining(20_000), 0);
    }

    #[test]
    fn the_pair_link_encodes_the_address() {
        assert_eq!(
            pair_link("http://100.64.0.1:3000/app/", "AB12"),
            "vorn://pair?url=http%3A%2F%2F100.64.0.1%3A3000%2Fapp%2F&code=AB12"
        );
    }

    #[test]
    fn labels_read_as_the_page_writes_them() {
        assert_eq!(devices_label(0), "Devices");
        assert_eq!(devices_label(2), "Devices · 2");
        assert_eq!(others_label(1, false), "1 other address");
        assert_eq!(others_label(3, false), "3 other addresses");
        assert_eq!(others_label(3, true), "Hide");
        let never = DeviceToken {
            id: "a".into(),
            name: "Phone".into(),
            last_seen_at: None,
            revoked_at: None,
        };
        assert_eq!(never.seen(), "Never connected");
        let seen = DeviceToken {
            last_seen_at: Some("2030-01-01T09:00:00.000Z".into()),
            ..never
        };
        assert_eq!(seen.seen(), "Last seen 1/1/2030, 9:00:00 AM");
    }

    #[test]
    fn the_code_carries_the_chosen_address_while_it_is_offered() {
        let mut n = Network {
            reachable: Some(Reachable {
                urls: vec![
                    "http://100.64.0.1:1/app/".into(),
                    "http://10.0.0.2:1/app/".into(),
                ],
                port: 1,
                remote: true,
            }),
            ..Network::default()
        };
        assert!(n.can_scan());
        assert_eq!(n.pair_url(), Some("http://100.64.0.1:1/app/"));
        n.chosen_url = Some("http://10.0.0.2:1/app/".into());
        assert_eq!(n.pair_url(), Some("http://10.0.0.2:1/app/"));
        n.chosen_url = Some("http://gone:1/app/".into());
        assert_eq!(n.pair_url(), Some("http://100.64.0.1:1/app/"));
        n.reachable = Some(Reachable::default());
        assert!(!n.can_scan());
        assert_eq!(n.pair_url(), None);
    }

    #[test]
    fn revoked_devices_are_not_listed() {
        let t = |id: &str, revoked: bool| DeviceToken {
            id: id.into(),
            name: id.into(),
            last_seen_at: None,
            revoked_at: revoked.then(|| "2030-01-01T00:00:00Z".into()),
        };
        let n = Network {
            tokens: Some(vec![t("a", false), t("b", true), t("c", false)]),
            ..Network::default()
        };
        let ids: Vec<_> = n.active().iter().map(|t| t.id.as_str()).collect();
        assert_eq!(ids, ["a", "c"]);
    }

    #[test]
    fn copied_lasts_two_seconds_for_that_text_only() {
        let at = Instant::now();
        let n = Network {
            copied: Some(("x".into(), at)),
            ..Network::default()
        };
        assert!(n.copied("x", at + Duration::from_millis(1999)));
        assert!(!n.copied("x", at + COPIED_FOR));
        assert!(!n.copied("y", at));
    }

    #[test]
    fn the_form_asks_for_both_fields() {
        assert_eq!(connect_error("", "vorn_x"), BOTH_NEEDED);
        assert_eq!(connect_error("10.0.0.2:1", "  "), BOTH_NEEDED);
        assert_eq!(connect_error("10.0.0.2:1", "vorn_x"), NO_HOST_MODE);
    }

    #[test]
    fn wire_shapes_decode() {
        let r: Reachable =
            serde_json::from_value(serde_json::json!({ "urls": [], "port": 3, "remote": false }))
                .unwrap();
        assert_eq!(r.port, 3);
        let s: Tailscale = serde_json::from_value(serde_json::json!({
            "installed": true, "running": false, "backendState": "Stopped", "peers": []
        }))
        .unwrap();
        assert!(s.installed && !s.running);
        let p: PairingRequest = serde_json::from_value(serde_json::json!({
            "requestId": "r", "deviceName": "Ana", "address": "10.0.0.2", "askedAt": 1, "status": "pending"
        }))
        .unwrap();
        assert_eq!(p.device_name, "Ana");
    }
}

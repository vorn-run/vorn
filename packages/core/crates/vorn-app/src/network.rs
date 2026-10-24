//! The Remote Access page's model: sharing this machine, how others reach
//! it, and the devices paired with it.

use web_time::{Duration, Instant};

use serde::Deserialize;

use crate::ui::Id;

/// "What is this device?": the name a minted token carries.
pub const DEVICE_NAME: Id = Id(0x6e65_745f_6465_7669);
/// Where "Add Tailscale" leads.
pub const TAILSCALE_DOWNLOAD: &str = "https://tailscale.com/download";

/// How long a copy button says "Copied".
pub const COPIED_FOR: Duration = Duration::from_secs(2);

/// How long after the share switch the addresses are asked again: vornd
/// rebinds on the config change, which takes a moment.
pub const REBIND_SETTLE: Duration = Duration::from_millis(500);

/// What a control on the Remote Access page does.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub enum NetAct {
    /// Loads what the page shows.
    Refresh,
    /// The "Share this machine" switch, asking for this value.
    Share(bool),
    /// "Enable anyway" under the warning shown without Tailscale.
    EnableAnyway,
    /// "Cancel" under that warning.
    CancelShare,
    /// A "how others reach it" tile: the route the code and the browser
    /// address use.
    Route(AddressKind),
    /// "Advanced": the fingerprint, the addresses and the token form.
    ToggleAdvanced,
    /// A copy button, which says "Copied" for a moment.
    Copy(String),
    /// "Add a device": the sheet with a code to scan.
    AddDevice,
    /// "Show another" once a code expired.
    ShowAnother,
    /// Closes the sheet and retires its code.
    CloseAdding,
    /// "Create token for another computer": the name field.
    NameToken,
    /// "Cancel" beside that field.
    CancelToken,
    /// "Create token" (or Enter in the name field).
    CreateToken,
    /// "Done" under a freshly minted token.
    DoneMinted,
    /// "Remove…" in a device's menu, which asks first.
    AskRevoke(String),
    /// "Cancel" on that question.
    CancelRevoke,
    /// "Revoke": the device's token stops working.
    Revoke(String),
    /// Approves (`true`) or denies the phone asking to pair.
    Decide(bool),
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
    /// The SHA-256 of the certificate the urls are served with.
    pub fingerprint: Option<String>,
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
    #[serde(default)]
    pub last_address: Option<String>,
}

/// What kind of device a token stands for, by the icon it gets.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DeviceKind {
    Phone,
    Computer,
}

impl DeviceToken {
    /// Tokens carry no kind, so the name tells: a phone names itself.
    pub fn kind(&self) -> DeviceKind {
        let name = self.name.to_lowercase();
        let phone = ["iphone", "ipad", "android", "pixel", "galaxy", "phone"]
            .iter()
            .any(|w| name.contains(w));
        if phone {
            DeviceKind::Phone
        } else {
            DeviceKind::Computer
        }
    }

    /// The line under the device's name, `now_ms` since the epoch.
    pub fn seen(&self, now_ms: f64) -> String {
        let when = self
            .last_seen_at
            .as_deref()
            .filter(|s| !s.is_empty())
            .map(|at| ago(at, now_ms));
        match (self.kind(), when) {
            (DeviceKind::Phone, Some(w)) => format!("Last seen {w}"),
            (DeviceKind::Phone, None) => "Never connected".to_owned(),
            (DeviceKind::Computer, Some(w)) => format!("Another Vorn · last seen {w}"),
            (DeviceKind::Computer, None) => "Another Vorn · never connected".to_owned(),
        }
    }
}

/// How long ago `iso` was, in words: "just now", "2 hours ago", "yesterday".
fn ago(iso: &str, now_ms: f64) -> String {
    let Some(t) = crate::tasks::ms(iso) else {
        return "a while ago".to_owned();
    };
    let mins = ((now_ms - t) / 60_000.0).floor().max(0.0) as u64;
    let n = |n: u64, unit: &str| {
        let s = if n == 1 { "" } else { "s" };
        format!("{n} {unit}{s} ago")
    };
    match mins {
        0 => "just now".to_owned(),
        1..=59 => n(mins, "minute"),
        60..=1439 => n(mins / 60, "hour"),
        1440..=2879 => "yesterday".to_owned(),
        2880..=43_199 => n(mins / 1440, "day"),
        _ => format!("on {}", crate::tasks::short_date(iso)),
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
    /// The share switch is on its way up without a tailnet.
    pub confirming: bool,
    /// The route the user picked; else the tailnet when it runs.
    pub route: Option<AddressKind>,
    /// "Advanced" is open.
    pub advanced: bool,
    /// The token form under Advanced is open.
    pub naming: bool,
    /// `None` until `token:list` first answers.
    pub tokens: Option<Vec<DeviceToken>>,
    /// The Add a device sheet is open: `None` inside until vornd hands a code.
    pub adding: Option<Option<Code>>,
    /// A `pairing:start` went out for the open sheet.
    pub scan_sent: bool,
    pub asking: Option<PairingRequest>,
    /// The QR the sheet draws, and the link it encodes, made once per link.
    pub qr: Option<(String, String)>,
    pub minted: Option<Minted>,
    /// The device whose "Remove" asks to confirm.
    pub confirm_revoke: Option<String>,
    /// The page's error line.
    pub error: Option<String>,
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

    /// Tailscale is up, so the tailnet route is there to pick.
    pub fn tailnet(&self) -> bool {
        self.status.as_ref().is_some_and(|s| s.running)
    }

    /// The route the code and the browser address use: the one picked while
    /// it is there, else the tailnet when it runs, else this network.
    pub fn route(&self) -> AddressKind {
        match self.route {
            Some(AddressKind::Tailnet) | None if self.tailnet() => AddressKind::Tailnet,
            _ => AddressKind::Lan,
        }
    }

    /// The address the code carries: the first on the route, else the first.
    pub fn pair_url(&self) -> Option<&str> {
        let urls = self.reachable.as_ref().map_or(&[][..], |r| &r.urls[..]);
        let route = self.route();
        urls.iter()
            .find(|u| address_kind(u) == route)
            .or_else(|| urls.first())
            .map(String::as_str)
    }

    /// The link the sheet's QR encodes, while it shows a live code.
    pub fn code_link(&self, now_ms: u64) -> Option<String> {
        let code = self.adding.as_ref()?.as_ref()?;
        if code.remaining(now_ms) == 0 || self.asking.is_some() {
            return None;
        }
        Some(pair_link(self.pair_url()?, &code.code, self.fingerprint()))
    }

    /// The certificate fingerprint a phone pins, while vornd shares this machine.
    pub fn fingerprint(&self) -> Option<&str> {
        self.reachable
            .as_ref()
            .and_then(|r| r.fingerprint.as_deref())
            .filter(|f| !f.is_empty())
    }

    /// Whether the copy button for `text` says "Copied" at `now`.
    pub fn copied(&self, text: &str, now: Instant) -> bool {
        self.copied
            .as_ref()
            .is_some_and(|(t, at)| t == text && now.duration_since(*at) < COPIED_FOR)
    }

    /// Puts the sheet away.
    pub fn close_adding(&mut self) {
        self.adding = None;
        self.scan_sent = false;
        self.asking = None;
        self.qr = None;
    }

    /// What the page forgets when it closes.
    pub fn closed(&mut self) {
        *self = Network::default();
    }
}

/// Which network an address reaches this machine over.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum AddressKind {
    /// Tailscale's 100.64.0.0/10, encrypted.
    Tailnet,
    /// Anything else: this network only.
    Lan,
}

/// Tailscale hands out addresses from 100.64.0.0/10 and uses
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

/// The host alone, which tells two routes of one kind apart.
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

/// Whole seconds left, rounded up, as `m:ss`.
pub fn format_remaining(ms: u64) -> String {
    let total = ms.div_ceil(1000);
    format!("{}:{:02}", total / 60, total % 60)
}

/// The link the pairing code stands for: `vorn://pair?url=…&code=…&fp=…`,
/// `fp` the certificate the phone is to trust.
pub fn pair_link(url: &str, code: &str, fingerprint: Option<&str>) -> String {
    let link = format!("vorn://pair?url={}&code={code}", crate::runs::encode(url));
    match fingerprint {
        Some(fp) => format!("{link}&fp={fp}"),
        None => link,
    }
}

/// A fingerprint in groups of four, to compare by eye or type.
pub fn grouped(fingerprint: &str) -> String {
    fingerprint
        .as_bytes()
        .chunks(4)
        .map(String::from_utf8_lossy)
        .collect::<Vec<_>>()
        .join(" ")
}

/// The line under "Share this Mac": off, or on with how many devices.
pub fn share_line(on: bool, devices: usize) -> String {
    match (on, devices) {
        (false, _) => "Off".to_owned(),
        (true, 0) => "On · No devices paired".to_owned(),
        (true, 1) => "On · 1 device paired".to_owned(),
        (true, n) => format!("On · {n} devices paired"),
    }
}

/// The code as the sheet shows it: `ABCD EFGH`, its groups split by a space.
pub fn shown_code(code: &str) -> String {
    code.replace('-', " ")
}

/// `link` as a QR code: one SVG path of dark modules over a `0 0 N N` box,
/// for `shape` to tint. Runs of modules merge so the edges between them
/// do not show. `None` when the link is too long for a code.
pub fn qr_svg(link: &str) -> Option<String> {
    use qrcodegen::{QrCode, QrCodeEcc};
    use std::fmt::Write;
    let qr = QrCode::encode_text(link, QrCodeEcc::Medium).ok()?;
    let n = qr.size();
    let mut path = String::new();
    for y in 0..n {
        let mut x = 0;
        while x < n {
            if !qr.get_module(x, y) {
                x += 1;
                continue;
            }
            let start = x;
            while x < n && qr.get_module(x, y) {
                x += 1;
            }
            let _ = write!(path, "M{start} {y}h{}v1h-{}z", x - start, x - start);
        }
    }
    Some(format!(
        r#"<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 {n} {n}"><path d="{path}"/></svg>"#
    ))
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
            pair_link("http://100.64.0.1:3000/app/", "AB12", None),
            "vorn://pair?url=http%3A%2F%2F100.64.0.1%3A3000%2Fapp%2F&code=AB12"
        );
        assert_eq!(
            pair_link("https://10.0.0.2:3000/app/", "AB12", Some("ab12cd")),
            "vorn://pair?url=https%3A%2F%2F10.0.0.2%3A3000%2Fapp%2F&code=AB12&fp=ab12cd"
        );
        assert_eq!(grouped("0123456789"), "0123 4567 89");
    }

    #[test]
    fn labels_read_as_the_page_writes_them() {
        assert_eq!(share_line(false, 3), "Off");
        assert_eq!(share_line(true, 0), "On · No devices paired");
        assert_eq!(share_line(true, 1), "On · 1 device paired");
        assert_eq!(share_line(true, 2), "On · 2 devices paired");
        assert_eq!(shown_code("ABCD-EFGH"), "ABCD EFGH");
        let at = "2030-01-01T09:00:00.000Z";
        let t = crate::tasks::ms(at).unwrap();
        let phone = DeviceToken {
            id: "a".into(),
            name: "Ana's iPhone".into(),
            last_seen_at: None,
            revoked_at: None,
            last_address: None,
        };
        assert_eq!(phone.seen(t), "Never connected");
        let phone = DeviceToken {
            last_seen_at: Some(at.into()),
            ..phone
        };
        assert_eq!(phone.seen(t + 1000.0), "Last seen just now");
        assert_eq!(phone.seen(t + 7_200_000.0), "Last seen 2 hours ago");
        assert_eq!(phone.seen(t + 60_000.0), "Last seen 1 minute ago");
        assert_eq!(phone.seen(t + 3.0 * 86_400_000.0), "Last seen 3 days ago");
        let mac = DeviceToken {
            name: "Studio Mac mini".into(),
            ..phone
        };
        assert_eq!(
            mac.seen(t + 86_400_000.0),
            "Another Vorn · last seen yesterday"
        );
    }

    #[test]
    fn a_device_is_a_phone_when_its_name_says_so() {
        let kind = |name: &str| {
            DeviceToken {
                id: "a".into(),
                name: name.into(),
                last_seen_at: None,
                revoked_at: None,
                last_address: None,
            }
            .kind()
        };
        assert_eq!(kind("Javier's iPhone"), DeviceKind::Phone);
        assert_eq!(kind("Pixel 9"), DeviceKind::Phone);
        assert_eq!(kind("android tablet"), DeviceKind::Phone);
        assert_eq!(kind("My phone"), DeviceKind::Phone);
        assert_eq!(kind("Studio Mac mini"), DeviceKind::Computer);
        assert_eq!(kind("Device"), DeviceKind::Computer);
    }

    #[test]
    fn the_qr_is_one_path_of_merged_runs() {
        let svg = qr_svg("vorn://pair?url=http%3A%2F%2F10.0.0.2%3A1%2F&code=ABCD-EFGH").unwrap();
        let n = qrcodegen::QrCode::encode_text(
            "vorn://pair?url=http%3A%2F%2F10.0.0.2%3A1%2F&code=ABCD-EFGH",
            qrcodegen::QrCodeEcc::Medium,
        )
        .unwrap()
        .size();
        assert!(svg.starts_with("<svg") && svg.contains(&format!(r#"viewBox="0 0 {n} {n}""#)));
        assert_eq!(svg.matches("<path").count(), 1);
        // The finder pattern's top edge is one run of seven modules.
        assert!(svg.contains("M0 0h7v1h-7z"));
        assert!(qr_svg(&"x".repeat(5000)).is_none());
    }

    #[test]
    fn the_code_carries_the_address_of_the_chosen_route() {
        let mut n = Network {
            reachable: Some(Reachable {
                urls: vec![
                    "http://10.0.0.2:1/app/".into(),
                    "http://100.64.0.1:1/app/".into(),
                ],
                port: 1,
                remote: true,
                fingerprint: Some("ab".repeat(32)),
            }),
            ..Network::default()
        };
        assert!(n.can_scan());
        // No tailnet: this network, whatever was picked.
        assert_eq!(n.route(), AddressKind::Lan);
        n.route = Some(AddressKind::Tailnet);
        assert_eq!(n.route(), AddressKind::Lan);
        assert_eq!(n.pair_url(), Some("http://10.0.0.2:1/app/"));
        n.status = Some(Tailscale {
            installed: true,
            running: true,
        });
        assert_eq!(n.route(), AddressKind::Tailnet);
        assert_eq!(n.pair_url(), Some("http://100.64.0.1:1/app/"));
        assert_eq!(n.fingerprint(), Some("ab".repeat(32).as_str()));
        n.route = Some(AddressKind::Lan);
        assert_eq!(n.pair_url(), Some("http://10.0.0.2:1/app/"));
        n.route = None;
        assert_eq!(n.route(), AddressKind::Tailnet);
        assert_eq!(n.code_link(0), None);
        n.adding = Some(Some(Code {
            code: "ABCD-EFGH".into(),
            expires_at: 10,
        }));
        assert!(n.code_link(0).unwrap().contains("100.64.0.1"));
        assert_eq!(n.code_link(10), None);
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
            last_address: None,
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

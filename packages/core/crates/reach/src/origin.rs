//! Which browsers may open a socket to the server.
//!
//! Browsers apply neither CORS nor the same-origin policy to a WebSocket
//! upgrade, and they let a page on any origin connect to `localhost`, so
//! without this any website the user visited could drive the server.
//! Browsers always set `Origin` and page script cannot forge it.
//!
//! The rule is same-origin: the `Origin` must name the `Host` the client
//! dialled. On its own that has one hole, a name the attacker chose pointed
//! at a private address (DNS rebinding), so the origin must also be one that
//! cannot be rebound: an IP literal, `localhost`, or a name in
//! [`TrustedHosts`].
//!
//! A client that sends no `Origin` is not a browser (a browser cannot leave
//! the header off an upgrade) and goes on to the credential check, which is
//! the control that applies to it.
//!
//! Both headers are read the way a WHATWG URL parser reads them, so that an
//! origin is allowed only in its canonical form (no userinfo, path, upper
//! case, spelled-out default port or shorthand address) and a `Host` is
//! compared as the parser would serialize it. Two narrowings, both refusals
//! where a full parser might allow: a host must be ASCII, and a domain label
//! may hold only letters, digits, `-` and `_`, with no `xn--` label.

use std::collections::BTreeSet;
use std::net::Ipv6Addr;

/// Names the operator vouched for, beyond IP literals and `localhost`: this
/// machine's name and its Tailscale name and address.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct TrustedHosts(BTreeSet<String>);

impl TrustedHosts {
    /// `hosts`, plus `machine` and `machine.local`, lowercased and without a
    /// trailing dot. Empty names are dropped.
    pub fn new<I, S>(hosts: I, machine: &str) -> TrustedHosts
    where
        I: IntoIterator<Item = S>,
        S: AsRef<str>,
    {
        let mut set = BTreeSet::new();
        let machine_local = format!("{machine}.local");
        for host in hosts
            .into_iter()
            .map(|h| h.as_ref().to_owned())
            .chain([machine.to_owned(), machine_local])
        {
            if !host.is_empty() {
                set.insert(normalise_hostname(&host));
            }
        }
        TrustedHosts(set)
    }

    pub fn contains(&self, hostname: &str) -> bool {
        self.0.contains(hostname)
    }
}

/// Lowercase and drop one trailing dot, which is legal in a name.
fn normalise_hostname(hostname: &str) -> String {
    let lower = hostname.to_lowercase();
    match lower.strip_suffix('.') {
        Some(stripped) => stripped.to_owned(),
        None => lower,
    }
}

/// Whether an upgrade carrying `origin` (`None` when absent) to `host` may
/// proceed. Every path but the absent header fails closed.
pub fn is_allowed_upgrade(
    origin: Option<&str>,
    host: Option<&str>,
    trusted: &TrustedHosts,
) -> bool {
    let Some(origin) = origin else {
        return true;
    };
    let Some(parsed) = canonical_origin(origin) else {
        return false;
    };
    let Some(host) = host.and_then(host_header) else {
        return false;
    };
    if parsed.host != host {
        return false;
    }
    let hostname = normalise_hostname(&parsed.hostname);
    is_ip_literal(&hostname) || hostname == "localhost" || trusted.contains(&hostname)
}

/// `[..]`, or four dotted groups of one to three digits. A shape test: the
/// parser has already refused anything else that looks numeric.
fn is_ip_literal(hostname: &str) -> bool {
    if hostname.starts_with('[') && hostname.ends_with(']') {
        return true;
    }
    let parts: Vec<&str> = hostname.split('.').collect();
    parts.len() == 4
        && parts
            .iter()
            .all(|p| (1..=3).contains(&p.len()) && p.bytes().all(|b| b.is_ascii_digit()))
}

struct Origin {
    /// `hostname[:port]`, as `URL.host` gives it.
    host: String,
    hostname: String,
}

/// The origin, when `origin` is an `http` or `https` origin in exactly the
/// form a parser would serialize it.
fn canonical_origin(origin: &str) -> Option<Origin> {
    let (rest, default_port) = match origin.strip_prefix("http://") {
        Some(rest) => (rest, 80),
        None => (origin.strip_prefix("https://")?, 443),
    };
    let (hostname, port) = split_host_port(rest)?;
    if let Some(port) = port {
        // Canonical: digits, no leading zero, in range, never the default.
        if port.is_empty()
            || !port.bytes().all(|b| b.is_ascii_digit())
            || (port.len() > 1 && port.starts_with('0'))
        {
            return None;
        }
        let n: u32 = port.parse().ok()?;
        if n > 65_535 || n == default_port {
            return None;
        }
    }
    if canonical_host(hostname)? != hostname {
        return None;
    }
    Some(Origin {
        host: rest.to_owned(),
        hostname: hostname.to_owned(),
    })
}

/// `host` and the text after its `:`, for an authority with nothing else in
/// it. `None` when it holds anything an origin cannot.
fn split_host_port(authority: &str) -> Option<(&str, Option<&str>)> {
    if authority.starts_with('[') {
        let end = authority.find(']')?;
        let (host, after) = authority.split_at(end + 1);
        return match after {
            "" => Some((host, None)),
            _ => Some((host, Some(after.strip_prefix(':')?))),
        };
    }
    match authority.split_once(':') {
        Some((host, port)) => Some((host, Some(port))),
        None => Some((authority, None)),
    }
}

/// The `Host` header as `new URL('http://' + raw).host` gives it, lowercased;
/// `None` where the parser would throw, or where this reads more narrowly
/// than the parser (see the module notes).
pub fn host_header(raw: &str) -> Option<String> {
    // The parser trims C0 controls and spaces, and drops tabs and newlines.
    let cleaned: String = raw
        .trim_matches(|c: char| c <= ' ')
        .chars()
        .filter(|c| !matches!(c, '\t' | '\n' | '\r'))
        .collect();
    let authority = cleaned
        .split(['/', '\\', '?', '#'])
        .next()
        .unwrap_or_default();
    let authority = authority.rsplit_once('@').map_or(authority, |(_, a)| a);
    let (host, port) = split_host_port(authority)?;
    let host = canonical_host(host)?;
    let port = match port {
        None => None,
        Some(p) => {
            if !p.bytes().all(|b| b.is_ascii_digit()) {
                return None;
            }
            let digits = p.trim_start_matches('0');
            if p.is_empty() {
                None
            } else if digits.len() > 5 {
                return None;
            } else {
                let n: u32 = if digits.is_empty() {
                    0
                } else {
                    digits.parse().ok()?
                };
                if n > 65_535 {
                    return None;
                }
                (n != 80).then_some(n)
            }
        }
    };
    Some(match port {
        Some(n) => format!("{host}:{n}"),
        None => host,
    })
}

/// A host as the parser serializes it for an `http` URL: a bracketed IPv6
/// address compressed, an address ending in a number as dotted decimal, a
/// name lowercased. `None` for one the parser refuses, or one outside the
/// narrowings in the module notes.
fn canonical_host(host: &str) -> Option<String> {
    if host.is_empty() {
        return None;
    }
    if let Some(inner) = host.strip_prefix('[') {
        let inner = inner.strip_suffix(']')?;
        let addr: Ipv6Addr = inner.parse().ok()?;
        return Some(format!("[{}]", serialize_ipv6(&addr)));
    }
    if !host.is_ascii() || host.contains('%') {
        return None;
    }
    let lower = host.to_ascii_lowercase();
    if lower
        .bytes()
        .any(|b| !(b.is_ascii_alphanumeric() || matches!(b, b'-' | b'_' | b'.')))
    {
        return None;
    }
    if ends_in_a_number(&lower) {
        return parse_ipv4(&lower).map(|a| {
            let [a, b, c, d] = a.to_be_bytes();
            format!("{a}.{b}.{c}.{d}")
        });
    }
    if lower.split('.').any(|label| label.starts_with("xn--")) {
        return None;
    }
    Some(lower)
}

/// The URL Standard's "ends in a number": the last label, past one trailing
/// dot, is all digits or `0x` and hex digits.
fn ends_in_a_number(host: &str) -> bool {
    let mut labels: Vec<&str> = host.split('.').collect();
    if labels.last() == Some(&"") {
        if labels.len() == 1 {
            return false;
        }
        labels.pop();
    }
    let last = labels.last().copied().unwrap_or_default();
    if !last.is_empty() && last.bytes().all(|b| b.is_ascii_digit()) {
        return true;
    }
    parse_ipv4_number(last).is_some()
}

/// The URL Standard's IPv4 number: decimal, `0`-prefixed octal or `0x` hex.
fn parse_ipv4_number(part: &str) -> Option<u64> {
    if part.is_empty() {
        return None;
    }
    let (digits, radix) = if let Some(hex) = part.strip_prefix("0x").or(part.strip_prefix("0X")) {
        (hex, 16)
    } else if part.len() > 1 && part.starts_with('0') {
        (&part[1..], 8)
    } else {
        (part, 10)
    };
    if digits.is_empty() {
        return Some(0);
    }
    if !digits.chars().all(|c| c.is_digit(radix)) {
        return None;
    }
    // Anything this long is out of range whatever its radix.
    if digits.len() > 20 {
        return Some(u64::MAX);
    }
    Some(u64::from_str_radix(digits, radix).unwrap_or(u64::MAX))
}

/// The URL Standard's IPv4 parser.
fn parse_ipv4(host: &str) -> Option<u32> {
    let mut parts: Vec<&str> = host.split('.').collect();
    if parts.last() == Some(&"") && parts.len() > 1 {
        parts.pop();
    }
    if parts.len() > 4 {
        return None;
    }
    let numbers: Vec<u64> = parts
        .iter()
        .map(|p| parse_ipv4_number(p))
        .collect::<Option<_>>()?;
    let (last, init) = numbers.split_last()?;
    if init.iter().any(|&n| n > 255) {
        return None;
    }
    if *last >= 256u64.pow(5 - numbers.len() as u32) {
        return None;
    }
    let mut ipv4 = *last;
    for (i, n) in init.iter().enumerate() {
        ipv4 += n * 256u64.pow(3 - i as u32);
    }
    u32::try_from(ipv4).ok()
}

/// The URL Standard's IPv6 serializer: lowercase hex pieces without leading
/// zeros, the first longest run of two or more zero pieces as `::`.
fn serialize_ipv6(addr: &Ipv6Addr) -> String {
    let pieces = addr.segments();
    let mut best: Option<(usize, usize)> = None;
    let mut i = 0;
    while i < 8 {
        if pieces[i] == 0 {
            let start = i;
            while i < 8 && pieces[i] == 0 {
                i += 1;
            }
            let len = i - start;
            if len >= 2 && best.is_none_or(|(_, l)| len > l) {
                best = Some((start, len));
            }
        } else {
            i += 1;
        }
    }
    let mut out = String::new();
    let mut i = 0;
    while i < 8 {
        if let Some((start, len)) = best {
            if i == start {
                out.push_str(if i == 0 { "::" } else { ":" });
                i += len;
                continue;
            }
        }
        out.push_str(&format!("{:x}", pieces[i]));
        if i != 7 {
            out.push(':');
        }
        i += 1;
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn trusted() -> TrustedHosts {
        TrustedHosts::new(["my-mac.tail1234.ts.net.", "100.64.0.1"], "My-Mac")
    }

    fn allowed(origin: &str, host: &str) -> bool {
        is_allowed_upgrade(Some(origin), Some(host), &trusted())
    }

    #[test]
    fn a_client_without_an_origin_goes_on_to_the_credential_check() {
        assert!(is_allowed_upgrade(None, None, &TrustedHosts::default()));
        assert!(is_allowed_upgrade(
            None,
            Some("x"),
            &TrustedHosts::default()
        ));
    }

    #[test]
    fn same_origin_on_an_address_that_cannot_be_rebound() {
        assert!(allowed("http://127.0.0.1:3456", "127.0.0.1:3456"));
        assert!(allowed("http://localhost:3456", "localhost:3456"));
        assert!(allowed("http://[::1]:3456", "[::1]:3456"));
        assert!(allowed("http://192.168.1.20:3456", "192.168.1.20:3456"));
        assert!(allowed("https://localhost", "localhost"));
        // The Host is read as an http URL, so 443 is not its default port.
        assert!(!allowed("https://localhost", "localhost:443"));
        assert!(allowed("http://my-mac:3456", "my-mac:3456"));
        assert!(allowed("http://my-mac.local:3456", "MY-MAC.local:3456"));
        assert!(allowed(
            "http://my-mac.tail1234.ts.net:3456",
            "my-mac.tail1234.ts.net:3456"
        ));
        // A trailing dot is the same name.
        assert!(allowed("http://localhost.:3456", "localhost.:3456"));
    }

    #[test]
    fn a_name_nobody_vouched_for_is_refused_even_when_it_matches() {
        assert!(!allowed(
            "http://vorn.attacker.test:3456",
            "vorn.attacker.test:3456"
        ));
    }

    #[test]
    fn origin_and_host_must_name_the_same_place() {
        assert!(!allowed("http://localhost:3456", "localhost:3457"));
        assert!(!allowed("http://127.0.0.1:3456", "localhost:3456"));
        assert!(!is_allowed_upgrade(
            Some("http://localhost:3456"),
            None,
            &trusted()
        ));
    }

    #[test]
    fn only_a_canonical_origin_is_allowed() {
        for origin in [
            "null",
            "",
            "file:///x",
            "ws://localhost:3456",
            "HTTP://localhost:3456",
            "http://LOCALHOST:3456",
            "http://localhost:3456/",
            "http://evil.example@127.0.0.1:3456",
            "http://127.0.0.1:080",
            "http://127.0.0.1:",
            "http://localhost:80",
            "https://localhost:443",
            "http://127.1:3456",
            "http://0x7f.0.0.1:3456",
            "http://[0:0:0:0:0:0:0:1]:3456",
            "http://[::1]:99999",
            "http://a, http://b",
            " http://localhost:3456",
            "http://xn--nxasmq6b:1",
        ] {
            assert!(!allowed(origin, "localhost:3456"), "{origin}");
            assert!(!allowed(origin, "127.0.0.1:3456"), "{origin}");
        }
    }

    #[test]
    fn reads_the_host_header_as_a_parser_would() {
        assert_eq!(
            host_header("LOCALHOST:3456").as_deref(),
            Some("localhost:3456")
        );
        assert_eq!(host_header("localhost:80").as_deref(), Some("localhost"));
        assert_eq!(host_header("localhost:0080").as_deref(), Some("localhost"));
        assert_eq!(host_header("localhost:").as_deref(), Some("localhost"));
        assert_eq!(host_header("127.1:3456").as_deref(), Some("127.0.0.1:3456"));
        assert_eq!(host_header("0x7f.1:1").as_deref(), Some("127.0.0.1:1"));
        assert_eq!(host_header("017.1:1").as_deref(), Some("15.0.0.1:1"));
        assert_eq!(host_header("2130706433").as_deref(), Some("127.0.0.1"));
        assert_eq!(host_header("[0:0::1]:5").as_deref(), Some("[::1]:5"));
        assert_eq!(host_header("x@127.0.0.1:5").as_deref(), Some("127.0.0.1:5"));
        assert_eq!(host_header("a:5/path").as_deref(), Some("a:5"));
        assert_eq!(host_header("a:0").as_deref(), Some("a:0"));
        assert_eq!(host_header("1.2.3.4.").as_deref(), Some("1.2.3.4"));
        assert_eq!(host_header("a\\b:5").as_deref(), Some("a"));
        assert_eq!(host_header("08.1.1.1"), None);
        assert_eq!(host_header("999.1.1.1"), None);
        assert_eq!(host_header("1.2.3.4.5"), None);
        assert_eq!(host_header("a:99999"), None);
        assert_eq!(host_header("a:x"), None);
        assert_eq!(host_header(""), None);
        assert_eq!(host_header("a b"), None);
    }

    #[test]
    fn serializes_ipv6_as_the_url_standard_does() {
        let s = |t: &str| serialize_ipv6(&t.parse().unwrap());
        assert_eq!(s("::1"), "::1");
        assert_eq!(s("::"), "::");
        assert_eq!(s("1:0:0:2:0:0:0:3"), "1:0:0:2::3");
        assert_eq!(s("1:0:2:3:4:5:6:7"), "1:0:2:3:4:5:6:7");
        assert_eq!(s("FE80::ABCD"), "fe80::abcd");
        assert_eq!(s("::ffff:1.2.3.4"), "::ffff:102:304");
        assert_eq!(s("1:2:3:4:5:6:7:0"), "1:2:3:4:5:6:7:0");
    }
}

//! What the address bar and an agent may load: the rules that turn what was
//! typed into a web address, a search, or nothing, and keep a `file:`
//! address inside its session's folder. Pure, so the same rules are tested
//! here and held by the app's recorded answers.

use url::Url;

/// What a new tab shows.
pub const DEFAULT_URL: &str = "about:blank";

/// Where the address bar sends text that is not a web address.
const SEARCH_URL: &str = "https://www.google.com/search";

/// Names that read as a scheme before a port, never as a host.
const SCHEME_NAMES: [&str; 7] = [
    "javascript",
    "file",
    "data",
    "blob",
    "vbscript",
    "about",
    "chrome",
];

/// What the address bar opens for what was typed: the web address it names,
/// or a search for words that name none. `None` for an address it will not load.
pub fn address_target(input: &str) -> Option<String> {
    let raw = input.trim();
    if raw.is_empty() {
        return None;
    }
    if looks_like_address(raw) {
        return normalize_url(raw, false);
    }
    let mut url = Url::parse(SEARCH_URL).ok()?;
    url.query_pairs_mut().append_pair("q", raw);
    Some(url.to_string())
}

/// A scheme, a port, a dotted host or `localhost`; a lone word or a phrase is a search.
fn looks_like_address(raw: &str) -> bool {
    if raw.chars().any(char::is_whitespace) {
        return false;
    }
    if host_and_port(raw) || has_scheme(raw) || raw.starts_with("//") {
        return true;
    }
    let host = raw.split(['/', '?', '#']).next().unwrap_or_default();
    host.contains('.') || host.eq_ignore_ascii_case("localhost") || host.starts_with('[')
}

/// What the address bar loads for what was typed, or `None`
/// for something it will not load. `file:` only with `allow_file`, with no
/// host, and without its query and hash.
pub fn normalize_url(input: &str, allow_file: bool) -> Option<String> {
    let raw = input.trim();
    if raw.is_empty() {
        return None;
    }
    if host_and_port(raw) {
        let name = raw[..raw.find(':')?].to_ascii_lowercase();
        return if SCHEME_NAMES.contains(&name.as_str()) {
            None
        } else {
            build_url(raw)
        };
    }
    if has_scheme(raw) {
        let mut parsed = Url::parse(raw).ok()?;
        return match parsed.scheme() {
            "about" => (raw == DEFAULT_URL).then(|| DEFAULT_URL.to_owned()),
            "file" => {
                if !allow_file || parsed.host_str().is_some_and(|h| !h.is_empty()) {
                    return None;
                }
                parsed.set_query(None);
                parsed.set_fragment(None);
                Some(parsed.to_string())
            }
            "http" | "https" => Some(parsed.to_string()),
            _ => None,
        };
    }
    build_url(raw.strip_prefix("//").unwrap_or(raw))
}

/// `/^[a-z0-9.-]+:\d+(\/.*)?$/i`: `localhost:3000`, which a URL parser
/// would read as the scheme `localhost`.
fn host_and_port(raw: &str) -> bool {
    let Some((name, rest)) = raw.split_once(':') else {
        return false;
    };
    let digits = rest.find('/').map_or(rest, |i| &rest[..i]);
    !name.is_empty()
        && name
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || b == b'.' || b == b'-')
        && !digits.is_empty()
        && digits.bytes().all(|b| b.is_ascii_digit())
        && !rest[digits.len()..].contains('\n')
}

/// `/^[a-z][a-z0-9+.-]*:/i`.
fn has_scheme(raw: &str) -> bool {
    let Some(i) = raw.find(':') else {
        return false;
    };
    let name = &raw.as_bytes()[..i];
    name.first().is_some_and(u8::is_ascii_alphabetic)
        && name
            .iter()
            .all(|&b| b.is_ascii_alphanumeric() || b == b'+' || b == b'.' || b == b'-')
}

fn build_url(host_and_path: &str) -> Option<String> {
    let scheme = if is_local_host(host_and_path) {
        "http://"
    } else {
        "https://"
    };
    let parsed = Url::parse(&format!("{scheme}{host_and_path}")).ok()?;
    parsed.host_str().filter(|h| !h.is_empty())?;
    Some(parsed.to_string())
}

fn is_local_host(value: &str) -> bool {
    let authority = value.split('/').next().unwrap_or_default();
    let host = if authority.starts_with('[') {
        authority[..authority.find(']').map_or(0, |i| i + 1)].to_ascii_lowercase()
    } else {
        authority
            .split(':')
            .next()
            .unwrap_or_default()
            .to_ascii_lowercase()
    };
    matches!(
        host.as_str(),
        "localhost" | "127.0.0.1" | "0.0.0.0" | "[::1]"
    ) || host.ends_with(".local")
        || host.ends_with(".localhost")
}

/// What a tab is called before its page has a title.
pub fn display_host(url: &str) -> String {
    if url == DEFAULT_URL {
        return "New tab".to_owned();
    }
    let Ok(parsed) = Url::parse(url) else {
        return url.to_owned();
    };
    if parsed.scheme() == "file" {
        let Some(name) = parsed.path().split('/').rfind(|s| !s.is_empty()) else {
            return url.to_owned();
        };
        return percent_decode(name).unwrap_or_else(|| name.to_owned());
    }
    match (parsed.host_str().filter(|h| !h.is_empty()), parsed.port()) {
        (None, _) => url.to_owned(),
        (Some(h), Some(p)) => format!("{h}:{p}"),
        (Some(h), None) => h.to_owned(),
    }
}

/// `decodeURIComponent`, or `None` where it would throw.
fn percent_decode(s: &str) -> Option<String> {
    let mut out = Vec::with_capacity(s.len());
    let bytes = s.as_bytes();
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] == b'%' {
            let hex = s.get(i + 1..i + 3)?;
            out.push(u8::from_str_radix(hex, 16).ok()?);
            i += 3;
        } else {
            out.push(bytes[i]);
            i += 1;
        }
    }
    String::from_utf8(out).ok()
}

/// Page text on one line, cut at `max` characters.
pub fn flatten_page_text(raw: &str, max: usize) -> String {
    let spaced: String = raw
        .chars()
        .map(|c| if c.is_control() { ' ' } else { c })
        .collect();
    let one_line = spaced.split_whitespace().collect::<Vec<_>>().join(" ");
    // JavaScript counts UTF-16 units; the cut lands on the same character
    // for text in the Basic Multilingual Plane.
    if one_line.encode_utf16().count() > max {
        let cut: String = one_line.chars().take(max).collect();
        format!("{cut}\u{2026}")
    } else {
        one_line
    }
}

/// What the agent may load in a session whose folder is `root`. A `file:`
/// address must sit inside the root, because an agent reads pages it did
/// not choose. The component sees no disk, so this is the address's own
/// path, its `..` already resolved by parsing; the app checks the real
/// path, links followed, again before it loads one.
pub fn loadable_url(input: &str, root: Option<&str>) -> Option<String> {
    let root = root.filter(|r| !r.trim().is_empty());
    let normalized = normalize_url(input, root.is_some())?;
    if !normalized.starts_with("file:") {
        return Some(normalized);
    }
    // The root as a directory address, encoded the way the page's is, so
    // the two compare as text: `C:\work` reads as `/C:/work/`.
    let root = root?.replace('\\', "/");
    let root = root.trim_end_matches('/');
    let lead = if root.starts_with('/') { "" } else { "/" };
    let base = Url::parse(&format!("file://{lead}{root}/")).ok()?;
    normalized.starts_with(base.as_str()).then_some(normalized)
}

/// Where a paged read resumes; a cursor from another page
/// generation starts over.
pub fn parse_cursor(cursor: Option<&str>, generation: u32) -> usize {
    let Some((gen, at)) = cursor.and_then(|c| c.split_once(':')) else {
        return 0;
    };
    if gen.parse::<u32>().ok() != Some(generation) {
        return 0;
    }
    at.parse().unwrap_or(0)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn cursors_resume_only_on_the_same_page() {
        assert_eq!(parse_cursor(None, 3), 0);
        assert_eq!(parse_cursor(Some("3:120"), 3), 120);
        assert_eq!(parse_cursor(Some("2:120"), 3), 0);
        assert_eq!(parse_cursor(Some("3:x"), 3), 0);
        assert_eq!(parse_cursor(Some("junk"), 3), 0);
    }

    #[test]
    fn file_addresses_stay_inside_the_session() {
        let root = "/work/proj";
        assert_eq!(
            loadable_url("file:///work/proj/a.html", Some(root)).as_deref(),
            Some("file:///work/proj/a.html")
        );
        assert_eq!(
            loadable_url("file:///work/proj/", Some("/work/proj/")).as_deref(),
            Some("file:///work/proj/")
        );
        assert_eq!(loadable_url("file:///work/secret.html", Some(root)), None);
        assert_eq!(
            loadable_url("file:///work/proj/../secret.html", Some(root)),
            None,
            "a climb out is resolved before the check"
        );
        assert_eq!(
            loadable_url("file:///work/project-b/a.html", Some(root)),
            None,
            "a sibling that shares the prefix is outside"
        );
        assert_eq!(loadable_url("file:///work/proj/a.html", None), None);
        assert_eq!(
            loadable_url("example.com", Some(root)).as_deref(),
            Some("https://example.com/")
        );
    }

    #[test]
    fn the_address_bar_reads_hosts_ports_and_schemes() {
        let n = |s: &str| normalize_url(s, false);
        assert_eq!(n("example.com").as_deref(), Some("https://example.com/"));
        assert_eq!(
            n("localhost:3000").as_deref(),
            Some("http://localhost:3000/")
        );
        assert_eq!(n("javascript:1").as_deref(), None);
        assert_eq!(n("about:blank").as_deref(), Some("about:blank"));
        assert_eq!(n("about:config").as_deref(), None);
        assert_eq!(n("file:///a/b.html"), None);
        assert_eq!(
            normalize_url("file:///a/b.html?x#y", true).as_deref(),
            Some("file:///a/b.html")
        );
        assert_eq!(n("//app.local/x").as_deref(), Some("http://app.local/x"));
        assert_eq!(n("   ").as_deref(), None);
    }

    #[test]
    fn words_become_a_search_and_addresses_load() {
        let a = |s: &str| address_target(s);
        assert_eq!(
            a("google").as_deref(),
            Some("https://www.google.com/search?q=google")
        );
        assert_eq!(
            a(" rust borrow checker ").as_deref(),
            Some("https://www.google.com/search?q=rust+borrow+checker")
        );
        assert_eq!(
            a("a&b").as_deref(),
            Some("https://www.google.com/search?q=a%26b")
        );
        assert_eq!(a("google.com").as_deref(), Some("https://google.com/"));
        assert_eq!(
            a("https://example.com/a?b=1").as_deref(),
            Some("https://example.com/a?b=1")
        );
        assert_eq!(a("localhost").as_deref(), Some("http://localhost/"));
        assert_eq!(
            a("localhost:3000/x").as_deref(),
            Some("http://localhost:3000/x")
        );
        assert_eq!(
            a("127.0.0.1:8080").as_deref(),
            Some("http://127.0.0.1:8080/")
        );
        assert_eq!(a("about:blank").as_deref(), Some("about:blank"));
        assert_eq!(a("javascript:alert(1)"), None);
        assert_eq!(a(""), None);
    }
}

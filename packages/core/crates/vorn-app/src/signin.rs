//! A connection's signed-in profile (`connection-sessions.ts`): the sign-in
//! window, and the hidden pages that make calls as the person signed in
//! there. The pure half lives here: which origins a call may reach, which
//! headers it may send, the script a page runs, and who a check names.

use serde_json::{json, Map, Value};

/// Every view of a connection's profile has a key under this.
const KEY_PREFIX: &str = "connection:";

/// `CONNECTION_PROFILE_PREFIX`: a profile's folder name before the id.
const PROFILE_PREFIX: &str = "vorn-connection-";

/// The largest body a signed-in call hands back.
pub const MAX_BODY: usize = 4 * 1024 * 1024;
/// The largest answer handed back as bytes; a bigger one is refused, never cut short.
pub const MAX_BYTES: usize = 16 * 1024 * 1024;
/// What a call over that limit is refused with, as the desktop words it.
pub const BYTES_REFUSAL: &str = "The answer is over the 16 MiB a signed-in call carries";

/// How often the sign-in window asks whether anyone signed in.
pub const CHECK_EVERY: std::time::Duration = std::time::Duration::from_secs(2);

/// Long enough to type a password and pass a second factor.
pub const SIGN_IN_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(10 * 60);

/// A runner nobody has used for this long is closed.
pub const RUNNER_IDLE: std::time::Duration = std::time::Duration::from_secs(60);

pub const TIMED_OUT: &str = "Sign-in timed out. Try again.";
pub const CLOSED: &str = "The sign-in window closed before anyone signed in.";

/// The view key of `connection`'s sign-in window.
pub fn window_key(connection: &str) -> String {
    format!("{KEY_PREFIX}{connection}#signin")
}

/// The view key of `connection`'s hidden page on `origin`.
pub fn runner_key(connection: &str, origin: &str) -> String {
    format!("{KEY_PREFIX}{connection} {origin}")
}

/// The connection a view key belongs to, when it is one of these.
pub fn connection_of(key: &str) -> Option<&str> {
    let rest = key.strip_prefix(KEY_PREFIX)?;
    Some(rest.split([' ', '#']).next().unwrap_or(rest))
}

/// The profile folder of `connection`, as the desktop names it.
pub fn profile_folder(connection: &str) -> String {
    let safe: String = connection
        .chars()
        .map(|c| {
            if c.is_ascii_alphanumeric() || c == '-' {
                c
            } else {
                '_'
            }
        })
        .collect();
    format!("{PROFILE_PREFIX}{safe}")
}

/// A stable 16-byte identifier for `connection`'s data store (macOS).
pub fn profile_uuid(connection: &str) -> [u8; 16] {
    // FNV-1a twice with different seeds: stable across runs, unlike `DefaultHasher`.
    let fnv = |seed: u64| {
        connection.bytes().fold(seed, |h, b| {
            (h ^ u64::from(b)).wrapping_mul(0x0100_0000_01b3)
        })
    };
    let mut out = [0u8; 16];
    out[..8].copy_from_slice(&fnv(0xcbf2_9ce4_8422_2325).to_be_bytes());
    out[8..].copy_from_slice(&fnv(0x8422_2325_cbf2_9ce4).to_be_bytes());
    // A version 4, variant 1 layout, so WebKit takes it as a UUID.
    out[6] = (out[6] & 0x0f) | 0x40;
    out[8] = (out[8] & 0x3f) | 0x80;
    out
}

/// Who opened a sign-in window, so the result goes back there.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum From {
    /// A connection row on the Plugins page.
    Plugins,
    /// A run waiting on a signed-out connection.
    Run,
}

/// The sign-in window of one connection, while it is open.
#[derive(Debug, Clone)]
pub struct Window {
    pub connection: String,
    /// The connector's name, for the title.
    pub name: String,
    /// The connector's `auth.browser`: where to sign in and how to check.
    pub browser: Value,
    pub url: String,
    pub from: From,
    pub opened: std::time::Instant,
    pub next_check: std::time::Instant,
    pub checking: bool,
}

impl Window {
    /// The window for `connection.browserAuth`'s answer, or why there is none.
    pub fn open(
        connection: &str,
        target: &Value,
        link: Option<&str>,
        from: From,
        now: std::time::Instant,
    ) -> Result<Window, String> {
        let browser = target
            .get("browser")
            .filter(|b| b.is_object())
            .ok_or("This connection does not sign in through a Vorn window.")?;
        let origins = origins(browser);
        if link.is_some_and(|l| !within_origins(&origins, l)) {
            return Err(format!("That link is not on {}.", origins.join(", ")));
        }
        let url = link
            .or_else(|| browser.get("signInUrl").and_then(Value::as_str))
            .ok_or("This connection names no page to sign in on.")?
            .to_owned();
        Ok(Window {
            connection: connection.to_owned(),
            name: target
                .get("name")
                .and_then(Value::as_str)
                .unwrap_or("")
                .to_owned(),
            browser: browser.clone(),
            url,
            from,
            opened: now,
            next_check: now + CHECK_EVERY,
            checking: false,
        })
    }

    /// The title the desktop gave its window.
    pub fn title(&self) -> String {
        format!("Sign in \u{b7} {}", self.name)
    }
}

/// `https://host` of `url`, when it is an https address on the default port.
pub fn origin_of(url: &str) -> Option<String> {
    let rest = url.strip_prefix("https://")?;
    let authority = rest.split(['/', '?', '#']).next()?;
    if authority.is_empty() || authority.contains(['@', ':']) {
        return None;
    }
    Some(format!("https://{}", authority.to_ascii_lowercase()))
}

/// `ORIGIN_PATTERN`: `https://host` or `https://*.host`, two labels at least.
fn valid_origin(origin: &str) -> bool {
    let Some(host) = origin.strip_prefix("https://") else {
        return false;
    };
    let host = host.strip_prefix("*.").unwrap_or(host);
    let labels: Vec<&str> = host.split('.').collect();
    labels.len() >= 2
        && labels
            .iter()
            .all(|l| !l.is_empty() && l.chars().all(|c| c.is_ascii_alphanumeric() || c == '-'))
}

/// `withinOrigins`: whether `url` is on one of the declared origins.
pub fn within_origins(origins: &[String], url: &str) -> bool {
    let Some(origin) = origin_of(url) else {
        return false;
    };
    let target = &origin["https://".len()..];
    origins.iter().any(|o| {
        if !valid_origin(o) {
            return false;
        }
        let o = o.to_ascii_lowercase();
        match o.strip_prefix("https://*.") {
            Some(host) => target.ends_with(&format!(".{host}")),
            None => target == &o["https://".len()..],
        }
    })
}

/// `allowedSessionHeader`: one the page does not set itself and that does not say who you are.
fn allowed_header(name: &str) -> bool {
    const FORBIDDEN: [&str; 7] = [
        "cookie",
        "cookie2",
        "authorization",
        "host",
        "origin",
        "referer",
        "content-length",
    ];
    let lower = name.to_ascii_lowercase();
    !name.is_empty()
        && name
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || "!#$%&'*+.^_`|~-".contains(c))
        && !FORBIDDEN.contains(&lower.as_str())
        && !lower.starts_with("sec-")
        && !lower.starts_with("proxy-")
}

/// `sessionHeaders`: the string headers a connector may send, and none of the rest.
pub fn session_headers(value: Option<&Value>) -> Map<String, Value> {
    value
        .and_then(Value::as_object)
        .into_iter()
        .flatten()
        .filter(|(k, v)| v.is_string() && allowed_header(k))
        .map(|(k, v)| (k.clone(), v.clone()))
        .collect()
}

/// The script a runner page executes for call `n`, built from JSON so
/// nothing a connector sends runs as code. It posts its answer back.
pub fn fetch_script(n: u64, request: &Value) -> String {
    let method = request
        .get("method")
        .and_then(Value::as_str)
        .unwrap_or("GET")
        .to_ascii_uppercase();
    let mut init = json!({
        "method": method,
        "headers": Value::Object(session_headers(request.get("headers"))),
        "credentials": "include",
    });
    if let Some(body) = request.get("body").filter(|b| !b.is_null()) {
        init["body"] = body.clone();
    }
    let url = request.get("url").and_then(Value::as_str).unwrap_or("");
    let url = Value::String(url.to_owned());
    let refusal = Value::String(BYTES_REFUSAL.to_owned());
    // Bytes are counted as they arrive, so a chunked or compressed answer stops at the limit too.
    let read = if request.get("binaryBody") == Some(&Value::Bool(true)) {
        format!(
            "const refuse = (size) => {{ if (size > {MAX_BYTES}) throw new Error({refusal}) }};
    refuse(Number(res.headers.get('content-length')));
    const reader = res.body && res.body.getReader();
    const parts = []; let total = 0;
    while (reader) {{ const {{ done, value }} = await reader.read(); if (done) break;
      total += value.byteLength; if (total > {MAX_BYTES}) await reader.cancel(); refuse(total); parts.push(value); }}
    let bin = ''; for (const part of parts) for (let i = 0; i < part.length; i += 32768) bin += String.fromCharCode.apply(null, part.subarray(i, i + 32768));
    const out = {{ body: '', bodyBase64: btoa(bin) }};"
        )
    } else {
        format!("const text = await res.text(); const out = {{ body: text.slice(0, {MAX_BODY}) }};")
    };
    format!(
        "(async () => {{ const post = (m) => window.ipc.postMessage(JSON.stringify(Object.assign({{ t: 'session', n: {n} }}, m)));
  try {{
    const res = await fetch({url}, {init});
    {read}
    post({{ ok: true, answer: Object.assign({{ status: res.status, headers: Object.fromEntries(res.headers) }}, out) }});
  }} catch (e) {{ post({{ ok: false, error: String(e && e.message || e) }}); }}
}})()"
    )
}

/// `identityFrom`: who the check's answer names, from the manifest's paths.
pub fn identity_from(body: &str, paths: &[String]) -> Option<String> {
    let parsed: Value = serde_json::from_str(body).ok()?;
    let values: Vec<String> = paths
        .iter()
        .filter_map(|p| p.split('.').try_fold(&parsed, |at, k| at.get(k)))
        .filter_map(|v| match v {
            Value::String(s) if !s.trim().is_empty() => Some(s.clone()),
            Value::Number(n) => Some(n.to_string()),
            _ => None,
        })
        .collect();
    let (first, rest) = values.split_first()?;
    Some(if rest.is_empty() {
        first.clone()
    } else {
        format!("{first} ({})", rest.join(", "))
    })
}

/// The `session:fetch` request a check makes: GET, JSON unless told otherwise.
pub fn check_request(browser: &Value) -> Value {
    let declared = browser
        .pointer("/check/headers")
        .cloned()
        .unwrap_or(json!({}));
    let mut headers = Map::new();
    let has_accept = declared
        .as_object()
        .is_some_and(|h| h.keys().any(|k| k.eq_ignore_ascii_case("accept")));
    if !has_accept {
        headers.insert("accept".into(), json!("application/json"));
    }
    if let Some(d) = declared.as_object() {
        headers.extend(d.clone());
    }
    json!({
        "url": browser.pointer("/check/url").and_then(Value::as_str).unwrap_or(""),
        "method": "GET",
        "headers": headers,
    })
}

/// A check's answer: whether someone is signed in, and as whom.
pub fn checked(answer: &Value, browser: &Value) -> (bool, Option<String>) {
    let status = answer.get("status").and_then(Value::as_u64).unwrap_or(0);
    let signed_in = (200..300).contains(&status);
    let paths: Vec<String> = browser
        .pointer("/check/identity")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .filter_map(Value::as_str)
        .map(str::to_owned)
        .collect();
    let body = answer.get("body").and_then(Value::as_str).unwrap_or("");
    let identity = signed_in.then(|| identity_from(body, &paths)).flatten();
    (signed_in, identity)
}

/// The declared origins of a `browser` sign-in.
pub fn origins(browser: &Value) -> Vec<String> {
    browser
        .get("origins")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .filter_map(Value::as_str)
        .map(str::to_owned)
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn o(list: &[&str]) -> Vec<String> {
        list.iter().map(|s| (*s).to_owned()).collect()
    }

    #[test]
    fn origins_admit_hosts_and_wildcards_only_over_https() {
        let list = o(&["https://example.com", "https://*.svc.io"]);
        assert!(within_origins(&list, "https://example.com/a?b"));
        assert!(within_origins(&list, "https://EXAMPLE.com"));
        assert!(within_origins(&list, "https://api.svc.io/x"));
        assert!(
            !within_origins(&list, "https://svc.io/x"),
            "a wildcard needs a subdomain"
        );
        assert!(!within_origins(&list, "http://example.com"));
        assert!(!within_origins(&list, "https://example.com:8443/"));
        assert!(!within_origins(&list, "https://user@example.com/"));
        assert!(!within_origins(&list, "https://evil-example.com"));
        assert!(!within_origins(
            &o(&["https://localhost"]),
            "https://localhost/"
        ));
    }

    #[test]
    fn headers_drop_what_the_page_sets_or_names_you() {
        let h = json!({ "accept": "x", "Cookie": "c", "Sec-Fetch": "s", "X-Ok": "1",
                        "Proxy-A": "p", "bad name": "b", "n": 3, "Authorization": "a" });
        let kept = session_headers(Some(&h));
        let mut keys: Vec<&str> = kept.keys().map(String::as_str).collect();
        keys.sort_unstable();
        assert_eq!(keys, ["X-Ok", "accept"]);
    }

    #[test]
    fn the_script_carries_the_request_as_data() {
        let js = fetch_script(
            7,
            &json!({ "url": "https://a.io/\"); alert(1); (\"", "method": "post",
                                          "headers": { "Cookie": "x" }, "body": "{}" }),
        );
        assert!(js.contains(r#""https://a.io/\"); alert(1); (\"""#));
        assert!(js.contains(r#""method":"POST""#));
        assert!(!js.contains("Cookie"));
        assert!(js.contains("n: 7"));
        assert!(!js.contains("bodyBase64"));
        let bin = fetch_script(8, &json!({ "url": "https://a.io/f", "binaryBody": true }));
        assert!(bin.contains("bodyBase64") && bin.contains(BYTES_REFUSAL));
    }

    #[test]
    fn identity_joins_every_named_field() {
        let body = r#"{"user":{"name":"Ada","id":42},"email":" "}"#;
        let paths = o(&["user.name", "email", "user.id", "missing.x"]);
        assert_eq!(identity_from(body, &paths).as_deref(), Some("Ada (42)"));
        assert_eq!(identity_from("not json", &paths), None);
        assert_eq!(identity_from("{}", &paths), None);
    }

    #[test]
    fn a_check_is_signed_in_on_2xx_with_who() {
        let browser = json!({ "check": { "url": "https://a.io/me", "identity": ["login"] } });
        let req = check_request(&browser);
        assert_eq!(req["headers"]["accept"], "application/json");
        let ok = json!({ "status": 200, "body": "{\"login\":\"ada\"}" });
        assert_eq!(checked(&ok, &browser), (true, Some("ada".into())));
        assert_eq!(
            checked(&json!({ "status": 401, "body": "" }), &browser),
            (false, None)
        );
    }

    #[test]
    fn keys_and_profiles_name_their_connection() {
        assert_eq!(connection_of(&window_key("c1")), Some("c1"));
        assert_eq!(connection_of(&runner_key("c1", "https://a.io")), Some("c1"));
        assert_eq!(connection_of("s1#page"), None);
        assert_eq!(profile_folder("a/b"), "vorn-connection-a_b");
        assert_eq!(profile_uuid("x"), profile_uuid("x"));
        assert_ne!(profile_uuid("x"), profile_uuid("y"));
        assert_eq!(origin_of("https://A.io/x").as_deref(), Some("https://a.io"));
    }
}

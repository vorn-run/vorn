//! Feed addresses and one GET through [`Host::fetch`], within the size limit,
//! as the connector's `http.ts` does them.
//!
//! Two differences from the program, both from what `fetch` answers: the
//! body arrives decoded as UTF-8, and there is no address after redirects,
//! so relative links resolve against the address requested.

use std::sync::LazyLock;

use regex::Regex;
use vorn_extension_sdk::{json, Header, Host, Request, Value};

use crate::feed::{parse_feed, Parsed, Source};
use crate::text::js_trim;
use crate::xml::parse_url;

pub const FEED_ACCEPT: &str = "application/rss+xml, application/atom+xml, application/feed+json, application/xml;q=0.9, text/xml;q=0.8, */*;q=0.5";
pub const MAX_BYTES: usize = 5 * 1024 * 1024;

pub fn page_accept() -> String {
    format!("text/html, application/xhtml+xml, {FEED_ACCEPT}")
}

/// A feed that could not be read, and why.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FeedError {
    pub url: String,
    pub reason: String,
}

impl FeedError {
    pub fn new(url: &str, reason: impl Into<String>) -> FeedError {
        FeedError {
            url: url.to_owned(),
            reason: reason.into(),
        }
    }

    pub fn message(&self) -> String {
        format!("{}: {}", self.url, self.reason)
    }

    pub fn to_json(&self) -> Value {
        json!({ "url": self.url, "error": self.reason })
    }
}

static SCHEME: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"(?i)^[a-z][a-z0-9+.-]*:").expect("a valid pattern"));
static HOST_PORT: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"^[^:/?#]+:[0-9]+(?:[/?#]|$)").expect("a valid pattern"));
static SEPARATORS: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"[\r\n,]+").expect("a valid pattern"));

/// The address to request: trimmed, `https://` assumed without a scheme, and only http or https.
pub fn feed_address(raw: &str) -> Result<String, FeedError> {
    let trimmed = js_trim(raw);
    let with_scheme = if SCHEME.is_match(trimmed) && !HOST_PORT.is_match(trimmed) {
        trimmed.to_owned()
    } else {
        format!("https://{trimmed}")
    };
    let url = parse_url(&with_scheme, "")
        .ok_or_else(|| FeedError::new(trimmed, "not a valid address"))?;
    if !(url.starts_with("http:") || url.starts_with("https:")) {
        return Err(FeedError::new(
            trimmed,
            "refused, only http and https feeds are read",
        ));
    }
    Ok(url)
}

/// `String(value ?? '')` for the values a config or args carry.
pub fn js_string(value: &Value) -> String {
    match value {
        Value::Null => String::new(),
        Value::String(s) => s.clone(),
        other => other.to_string(),
    }
}

/// Addresses one per line or comma separated, blanks and repeats dropped.
pub fn feed_list(value: &Value) -> Vec<String> {
    let mut out: Vec<String> = Vec::new();
    for part in SEPARATORS.split(&js_string(value)).map(js_trim) {
        if !part.is_empty() && !out.iter().any(|o| o == part) {
            out.push(part.to_owned());
        }
    }
    out
}

/// Addresses to request, repeats dropped, and the ones that cannot be requested as failures.
pub fn unique_feeds(raws: &[String]) -> (Vec<String>, Vec<FeedError>) {
    let (mut urls, mut invalid) = (Vec::<String>::new(), Vec::new());
    for raw in raws {
        match feed_address(raw) {
            Ok(url) if !urls.contains(&url) => urls.push(url),
            Ok(_) => {}
            Err(e) => invalid.push(e),
        }
    }
    (urls, invalid)
}

pub struct Fetched {
    pub body: String,
    pub url: String,
    pub etag: String,
    pub last_modified: String,
}

/// Conditional-GET validators sent back to the server.
#[derive(Default)]
pub struct Validators<'a> {
    pub etag: Option<&'a str>,
    pub last_modified: Option<&'a str>,
}

fn header<'a>(headers: &'a [Header], name: &str) -> Option<&'a str> {
    headers
        .iter()
        .find(|h| h.name.eq_ignore_ascii_case(name))
        .map(|h| h.value.as_str())
}

/// One GET within the size limit; none for a 304 to validators the caller sent.
pub fn send(
    host: &mut dyn Host,
    url: &str,
    user_agent: &str,
    accept: &str,
    validators: &Validators<'_>,
) -> Result<Option<Fetched>, FeedError> {
    let etag = validators.etag.filter(|v| !v.is_empty());
    let last_modified = validators.last_modified.filter(|v| !v.is_empty());
    let mut headers = vec![
        Header {
            name: "accept".into(),
            value: accept.into(),
        },
        Header {
            name: "user-agent".into(),
            value: user_agent.into(),
        },
    ];
    if let Some(v) = etag {
        headers.push(Header {
            name: "if-none-match".into(),
            value: v.into(),
        });
    }
    if let Some(v) = last_modified {
        headers.push(Header {
            name: "if-modified-since".into(),
            value: v.into(),
        });
    }
    let req = Request {
        method: "GET".into(),
        url: url.into(),
        headers,
        body: None,
    };
    let res = host
        .fetch(&req, None)
        .map_err(|e| FeedError::new(url, e.message))?;
    if res.status == 304 && (etag.is_some() || last_modified.is_some()) {
        return Ok(None);
    }
    if !(200..300).contains(&res.status) {
        return Err(FeedError::new(url, format!("HTTP {}", res.status)));
    }
    let declared =
        header(&res.headers, "content-length").and_then(|v| v.trim().parse::<usize>().ok());
    if declared.unwrap_or(0) > MAX_BYTES || res.body.len() > MAX_BYTES {
        return Err(FeedError::new(url, "larger than 5 MB"));
    }
    Ok(Some(Fetched {
        etag: header(&res.headers, "etag").unwrap_or("").to_owned(),
        last_modified: header(&res.headers, "last-modified")
            .unwrap_or("")
            .to_owned(),
        body: res
            .body
            .strip_prefix('\u{feff}')
            .map_or(res.body.clone(), str::to_owned),
        url: url.to_owned(),
    }))
}

/// One unconditional GET; a 304 is a failure like any other status.
pub fn fetch_body(
    host: &mut dyn Host,
    url: &str,
    user_agent: &str,
    accept: &str,
) -> Result<Fetched, FeedError> {
    send(host, url, user_agent, accept, &Validators::default())?
        .ok_or_else(|| FeedError::new(url, "HTTP 304"))
}

/// A fetched body as a feed, or the reason it is not one against its address.
pub fn parse_fetched(url: &str, got: &Fetched) -> Result<Parsed, FeedError> {
    parse_feed(
        &got.body,
        &Source {
            feed_url: url,
            final_url: &got.url,
        },
    )
    .map_err(|e| FeedError::new(url, e))
}

/// Fetch and parse one feed, unconditionally.
pub fn read_feed_at(host: &mut dyn Host, raw: &str, user_agent: &str) -> Result<Parsed, FeedError> {
    let url = feed_address(raw)?;
    parse_fetched(&url, &fetch_body(host, &url, user_agent, FEED_ACCEPT)?)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn addresses_assume_https_and_refuse_other_schemes() {
        assert_eq!(
            feed_address(" e.com/feed ").as_deref(),
            Ok("https://e.com/feed")
        );
        assert_eq!(
            feed_address("localhost:8080/x").as_deref(),
            Ok("https://localhost:8080/x")
        );
        assert_eq!(feed_address("http://e.com").as_deref(), Ok("http://e.com/"));
        let refused = feed_address("ftp://e.com").expect_err("refused");
        assert_eq!(
            refused.reason,
            "refused, only http and https feeds are read"
        );
        assert_eq!(
            feed_address("https://").expect_err("invalid").reason,
            "not a valid address"
        );
    }

    #[test]
    fn lists_split_on_lines_and_commas() {
        assert_eq!(feed_list(&json!("a, b\r\n\na,c")), ["a", "b", "c"]);
        assert!(feed_list(&Value::Null).is_empty());
    }
}

//! The HTTP connector: a connection is an auth profile, a base URL and the
//! rules that carry its secret into every request made through it. Requests
//! are sent here, so the secret never reaches a client and no browser's CORS
//! applies; a profile only ever signs requests to its own origin, and a
//! redirect is reported rather than followed.

use std::time::Duration;

use serde_json::{json, Map, Value};

use crate::fetch::{Http, Request};

/// How long one request may take.
pub const TIMEOUT: Duration = Duration::from_secs(30);

const METHODS: [&str; 7] = ["GET", "HEAD", "POST", "PUT", "PATCH", "DELETE", "OPTIONS"];

/// What a profile carries: its fields, the secret already read from the vault.
#[derive(Debug, Clone, Default)]
pub struct Profile {
    pub base_url: String,
    pub auth_header: String,
    pub auth_query: String,
    pub auth_body: String,
    pub secret: String,
}

impl Profile {
    /// From a profile connection's filters and its secret.
    pub fn of(filters: &Map<String, Value>, secret: Option<&str>) -> Profile {
        let s = |k: &str| filters.get(k).and_then(Value::as_str).unwrap_or("").to_owned();
        Profile {
            base_url: s("baseUrl"),
            auth_header: s("authHeader"),
            auth_query: s("authQuery"),
            auth_body: s("authBody"),
            secret: secret.unwrap_or("").to_owned(),
        }
    }
}

/// Why a profile cannot sign: its secret is stored but cannot be read here.
pub fn locked_error(filters: &Map<String, Value>, secret_readable: bool) -> Option<String> {
    let stored = filters.get("secret").and_then(Value::as_str).is_some_and(|s| !s.is_empty());
    (stored && !secret_readable).then(|| {
        "This profile's secret is locked - decryption is unavailable or has not synced yet.".to_owned()
    })
}

/// Why a connection cannot sign a request: not a profile, or a locked one.
pub fn profile_error(connector_id: &str, filters: &Map<String, Value>, secret_readable: bool) -> Option<String> {
    if connector_id != crate::connections::HTTP {
        return Some(format!(
            "Connection belongs to the {connector_id} connector, not an HTTP auth profile"
        ));
    }
    locked_error(filters, secret_readable)
}

fn failure(error: impl Into<String>) -> Value {
    json!({ "success": false, "error": error.into() })
}

/// The request a profile makes of `method url` with its injections applied,
/// or the action result that refuses it.
pub fn prepare(
    profile: &Profile,
    method: &str,
    url: &str,
    headers: Vec<(String, String)>,
    body: Option<String>,
) -> Result<Request, Value> {
    let method = method.to_uppercase();
    if !METHODS.contains(&method.as_str()) {
        let shown = if method.is_empty() { "(none)" } else { method.as_str() };
        return Err(failure(format!("Invalid HTTP method: {shown}")));
    }
    let base = match profile.base_url.as_str() {
        "" => None,
        given => match url::Url::parse(given) {
            Ok(base) => Some(base),
            Err(_) => return Err(failure(format!("Invalid URL: {url}"))),
        },
    };
    let parsed = match &base {
        Some(base) => base.join(url),
        None => url::Url::parse(url),
    };
    let Ok(mut target) = parsed else {
        return Err(failure(format!("Invalid URL: {url}")));
    };
    let injects = !profile.auth_header.is_empty()
        || !profile.auth_query.is_empty()
        || !profile.auth_body.trim().is_empty()
        || !profile.secret.is_empty();
    if injects {
        let origin = base.as_ref().map(|b| b.origin().ascii_serialization());
        if origin.as_deref() != Some(target.origin().ascii_serialization().as_str()) {
            return Err(failure(format!(
                "This profile only signs requests to {}; refusing {}",
                origin.as_deref().unwrap_or("its base URL (none set)"),
                target.origin().ascii_serialization()
            )));
        }
    }
    let inject = |template: &str| template.replace("{{secret}}", &profile.secret);
    let mut headers = headers;
    if let Some((name, value)) = profile.auth_header.split_once(':') {
        let name = name.trim().to_owned();
        headers.retain(|(k, _)| *k != name);
        headers.push((name, inject(value.trim())));
    }
    if let Some((name, value)) = profile.auth_query.split_once('=') {
        let (name, value) = (name.trim().to_owned(), inject(value.trim()));
        let kept: Vec<(String, String)> = target
            .query_pairs()
            .filter(|(k, _)| *k != name)
            .map(|(k, v)| (k.into_owned(), v.into_owned()))
            .collect();
        target.query_pairs_mut().clear().extend_pairs(kept).append_pair(&name, &value);
    }
    let mut body = body;
    let auth_body = profile.auth_body.trim();
    if !auth_body.is_empty() {
        let injected = serde_json::from_str::<Value>(&inject(auth_body));
        let current = match body.as_deref().map(str::trim) {
            Some(b) if !b.is_empty() => serde_json::from_str::<Value>(b),
            _ => Ok(json!({})),
        };
        if let (Ok(Value::Object(add)), Ok(Value::Object(mut merged))) = (injected, current) {
            merged.extend(add);
            body = Some(Value::Object(merged).to_string());
            if !headers.iter().any(|(k, _)| k.eq_ignore_ascii_case("content-type")) {
                headers.push(("Content-Type".into(), "application/json".into()));
            }
        }
    }
    let send = body.filter(|b| !b.is_empty() && method != "GET" && method != "HEAD");
    Ok(Request {
        method,
        url: target.to_string(),
        headers,
        body: send,
    })
}

/// Sends a request through a profile (`performHttpRequest`): the action
/// result, `{status, headers, body}` as output on any status. Blocks.
pub fn perform(
    http: &Http,
    profile: &Profile,
    method: &str,
    url: &str,
    headers: Vec<(String, String)>,
    body: Option<String>,
) -> Value {
    let request = match prepare(profile, method, url, headers, body) {
        Ok(r) => r,
        Err(refused) => return refused,
    };
    match http.send(&request, TIMEOUT) {
        Ok(response) => {
            let body = serde_json::from_str::<Value>(&response.body)
                .unwrap_or(Value::String(response.body));
            json!({ "success": true, "output": {
                "status": response.status,
                "headers": response.headers,
                "body": body,
            } })
        }
        Err(error) => failure(error),
    }
}

/// The connector's actions (`httpConnector.execute`): `test` asks the base
/// URL, `request` sends one through the profile.
pub fn execute(http: &Http, action: &str, profile: &Profile, args: &Map<String, Value>) -> Value {
    let s = |k: &str| args.get(k).and_then(Value::as_str).unwrap_or("").to_owned();
    match action {
        "test" => {
            if profile.base_url.is_empty() {
                return failure("Set a base URL to test this profile");
            }
            perform(http, profile, "GET", &profile.base_url.clone(), Vec::new(), None)
        }
        "request" => {
            let url = s("url");
            if url.is_empty() {
                return failure("url is required");
            }
            let headers = args
                .get("headers")
                .and_then(Value::as_object)
                .map(|h| {
                    h.iter()
                        .map(|(k, v)| (k.clone(), crate::js::to_string(v)))
                        .collect()
                })
                .unwrap_or_default();
            let method = Some(s("method")).filter(|m| !m.is_empty()).unwrap_or_else(|| "GET".into());
            let body = Some(s("body")).filter(|b| !b.is_empty());
            perform(http, profile, &method, &url, headers, body)
        }
        other => failure(format!("Unknown action: {other}")),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn profile() -> Profile {
        Profile {
            base_url: "https://api.example.com/v1/".into(),
            auth_header: "Authorization: Bearer {{secret}}".into(),
            auth_query: "key={{secret}}".into(),
            auth_body: r#"{"token":"{{secret}}"}"#.into(),
            secret: "s3cr3t".into(),
        }
    }

    #[test]
    fn signs_a_request_to_its_own_origin() {
        let req = prepare(&profile(), "post", "items?x=1", vec![], Some(r#"{"a":1}"#.into())).unwrap();
        assert_eq!(req.method, "POST");
        assert_eq!(req.url, "https://api.example.com/v1/items?x=1&key=s3cr3t");
        assert!(req.headers.contains(&("Authorization".into(), "Bearer s3cr3t".into())));
        assert!(req.headers.contains(&("Content-Type".into(), "application/json".into())));
        assert_eq!(req.body.as_deref(), Some(r#"{"a":1,"token":"s3cr3t"}"#));
    }

    #[test]
    fn refuses_another_origin_and_odd_methods() {
        let refused = prepare(&profile(), "GET", "https://evil.example/x", vec![], None).unwrap_err();
        assert_eq!(refused["error"], "This profile only signs requests to https://api.example.com; refusing https://evil.example");
        assert_eq!(prepare(&profile(), "TRACE", "/", vec![], None).unwrap_err()["error"], "Invalid HTTP method: TRACE");
        assert_eq!(prepare(&Profile::default(), "GET", "nope", vec![], None).unwrap_err()["error"], "Invalid URL: nope");
        // No injection, so any URL may be asked; a GET carries no body.
        let plain = prepare(&Profile::default(), "GET", "https://x.example/", vec![], Some("b".into())).unwrap();
        assert_eq!(plain.body, None);
    }

    #[test]
    fn says_why_a_profile_cannot_sign() {
        let stored = json!({ "secret": "vorn-vault" });
        let stored = stored.as_object().unwrap();
        assert!(locked_error(stored, false).unwrap().contains("locked"));
        assert_eq!(locked_error(stored, true), None);
        assert_eq!(
            profile_error("mcp", stored, true).unwrap(),
            "Connection belongs to the mcp connector, not an HTTP auth profile"
        );
        assert_eq!(execute(&Http, "test", &Profile::default(), &Map::new())["error"], "Set a base URL to test this profile");
        assert_eq!(execute(&Http, "nope", &Profile::default(), &Map::new())["error"], "Unknown action: nope");
    }
}

//! Sign-in windows as a bridge service: a program that acts through a site a
//! person signed in to in a desktop window is granted a token for that
//! window, and each request it sends through it is checked here before the
//! desktop makes it.
//!
//! Which window a grant is for is the caller's key (a connection's id). The
//! grant names the origins the window may be asked about; a request to any
//! other address is refused. The requests of one call are kept, so a call
//! that fails can say whether the desktop was away or the site signed the
//! person out.

use std::collections::{BTreeMap, HashMap};
use std::sync::{Arc, Mutex, MutexGuard, PoisonError};

use serde_json::{json, Map, Value};

use crate::manifest::within_origins;
use crate::token;

/// The most requests of one call kept to explain it failing.
pub const KEPT_CALLS: usize = 50;
/// The largest request a program may send through a window.
pub const MAX_WINDOW_REQUEST: usize = 1024 * 1024;

fn lock<T>(m: &Mutex<T>) -> MutexGuard<'_, T> {
    m.lock().unwrap_or_else(PoisonError::into_inner)
}

/// One program's grant to call through a window.
#[derive(Debug)]
pub struct Grant {
    token: String,
    /// The sign-in the manifest declares: its address, origins and check.
    pub browser: Value,
    /// The requests of each call in flight, by the call's key.
    calls: Mutex<HashMap<String, Vec<Value>>>,
}

impl Grant {
    /// The origins the window may be asked about.
    pub fn origins(&self) -> Vec<String> {
        self.browser
            .get("origins")
            .and_then(Value::as_array)
            .into_iter()
            .flatten()
            .filter_map(Value::as_str)
            .map(str::to_owned)
            .collect()
    }

    /// Keeps what became of one request of call `key`.
    pub fn record(&self, key: Option<&str>, entry: Value) {
        let Some(key) = key else { return };
        if let Some(calls) = lock(&self.calls).get_mut(key) {
            calls.push(entry);
            if calls.len() > KEPT_CALLS {
                let excess = calls.len() - KEPT_CALLS;
                calls.drain(..excess);
            }
        }
    }
}

/// A request a program asked to send through its window, checked.
#[derive(Debug, Clone)]
pub struct Admitted {
    pub grant: Arc<Grant>,
    /// `{url, method, headers?, body?, binaryBody?}`, as the desktop takes it.
    pub request: Value,
    pub method: String,
    pub path: String,
}

/// Every grant, by the key of the window it is for.
#[derive(Debug, Default)]
pub struct Windows {
    grants: Mutex<HashMap<String, Arc<Grant>>>,
}

impl Windows {
    /// Grants the program of `key` a token to call through its window; any
    /// grant it had is replaced.
    pub fn grant(&self, key: &str, browser: Value) -> Result<String, String> {
        let token = token::mint().map_err(|e| format!("no token for a window: {e}"))?;
        lock(&self.grants).insert(
            key.to_owned(),
            Arc::new(Grant {
                token: token.clone(),
                browser,
                calls: Mutex::default(),
            }),
        );
        Ok(token)
    }

    pub fn revoke(&self, key: &str) {
        lock(&self.grants).remove(key);
    }

    pub fn get(&self, key: &str) -> Option<Arc<Grant>> {
        lock(&self.grants).get(key).cloned()
    }

    /// Starts keeping the requests of one call through `key`'s window; the
    /// call's key, when the window has a grant.
    pub fn begin(&self, key: &str) -> Option<(Arc<Grant>, String)> {
        let grant = self.get(key)?;
        let call = token::mint().ok()?;
        lock(&grant.calls).insert(call.clone(), Vec::new());
        Some((grant, call))
    }

    /// The requests call `call` made, and stops keeping them.
    pub fn end(grant: &Grant, call: &str) -> Vec<Value> {
        lock(&grant.calls).remove(call).unwrap_or_default()
    }

    /// Checks a request a program sent for `key`'s window with `bearer`:
    /// the status and body to refuse it with, or the request to send.
    pub fn admit(
        &self,
        key: &str,
        bearer: Option<&str>,
        body: &[u8],
    ) -> Result<Admitted, (u16, Value)> {
        let refuse = |status: u16, message: &str| (status, json!({ "error": message }));
        let grant = self.get(key);
        let known = grant
            .as_ref()
            .zip(bearer)
            .is_some_and(|(g, t)| token::same(t, &g.token));
        let Some(grant) = grant.filter(|_| known) else {
            return Err(refuse(401, "This endpoint does not know that caller"));
        };
        if body.len() > MAX_WINDOW_REQUEST {
            return Err(refuse(413, "That request is too large"));
        }
        let Some(request) = read_window_request(body) else {
            return Err(refuse(400, "Send { url, method, headers?, body? }"));
        };
        let method = request["method"].as_str().unwrap_or("").to_owned();
        if !matches!(
            method.as_str(),
            "GET" | "HEAD" | "POST" | "PUT" | "PATCH" | "DELETE"
        ) {
            return Err(refuse(
                405,
                &format!("{method} is not a method a signed-in call may use"),
            ));
        }
        let url = request["url"].as_str().unwrap_or("").to_owned();
        if !within_origins(&grant.origins(), &url) {
            return Err(refuse(
                403,
                &format!("{url} is not on one of this connection's origins"),
            ));
        }
        let path = url_path(&url);
        Ok(Admitted {
            grant,
            request,
            method,
            path,
        })
    }
}

/// The path of an absolute URL, empty when it is not one.
fn url_path(url: &str) -> String {
    let Some((_, rest)) = url.split_once("://") else {
        return String::new();
    };
    let start = rest.find('/').unwrap_or(rest.len());
    let rest = &rest[start..];
    let end = rest.find(['?', '#']).unwrap_or(rest.len());
    if rest[..end].is_empty() {
        "/".to_owned()
    } else {
        rest[..end].to_owned()
    }
}

/// A window request a program sent: `{url, method, headers?, body?, binaryBody?}`.
pub fn read_window_request(body: &[u8]) -> Option<Value> {
    let value: Value = serde_json::from_slice(body).ok()?;
    let v = value.as_object()?;
    let url = v.get("url")?.as_str()?;
    let method = v.get("method")?.as_str()?.to_uppercase();
    if v.get("body").is_some_and(|b| !b.is_string()) {
        return None;
    }
    let mut out = Map::new();
    out.insert("url".into(), json!(url));
    out.insert("method".into(), json!(method));
    if let Some(headers) = v.get("headers").and_then(Value::as_object) {
        let kept: BTreeMap<&String, &Value> =
            headers.iter().filter(|(_, v)| v.is_string()).collect();
        out.insert("headers".into(), json!(kept));
    }
    if let Some(body) = v.get("body") {
        out.insert("body".into(), body.clone());
    }
    if v.get("binaryBody") == Some(&Value::Bool(true)) {
        out.insert("binaryBody".into(), json!(true));
    }
    Some(Value::Object(out))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reads_a_window_request_and_refuses_what_is_not_one() {
        let req = read_window_request(br#"{"url":"https://a.example/x","method":"post","headers":{"a":"1","b":2},"body":"x"}"#).unwrap();
        assert_eq!(req["method"], "POST");
        assert_eq!(req["headers"], json!({ "a": "1" }));
        assert!(read_window_request(br#"{"url":"u","method":"GET","body":3}"#).is_none());
        assert!(read_window_request(b"[]").is_none());
    }

    #[test]
    fn admits_a_known_caller_to_its_origins_only() {
        let windows = Windows::default();
        let token = windows
            .grant("c1", json!({ "origins": ["https://app.example"] }))
            .unwrap();
        let ask = |bearer: Option<&str>, body: &str| windows.admit("c1", bearer, body.as_bytes());
        let ok = ask(
            Some(&token),
            r#"{"url":"https://app.example/api/me?x=1","method":"get"}"#,
        )
        .unwrap();
        assert_eq!((ok.method.as_str(), ok.path.as_str()), ("GET", "/api/me"));
        assert_eq!(ask(Some("guess"), "{}").unwrap_err().0, 401);
        assert_eq!(ask(None, "{}").unwrap_err().0, 401);
        assert_eq!(ask(Some(&token), "nope").unwrap_err().0, 400);
        assert_eq!(
            ask(
                Some(&token),
                r#"{"url":"https://app.example/","method":"TRACE"}"#
            )
            .unwrap_err()
            .0,
            405
        );
        let elsewhere = ask(
            Some(&token),
            r#"{"url":"https://evil.example/","method":"GET"}"#,
        )
        .unwrap_err();
        assert_eq!(elsewhere.0, 403);
        assert_eq!(
            elsewhere.1["error"],
            "https://evil.example/ is not on one of this connection's origins"
        );
        windows.revoke("c1");
        assert_eq!(ask(Some(&token), "{}").unwrap_err().0, 401);
    }

    #[test]
    fn keeps_the_last_requests_of_a_call() {
        let windows = Windows::default();
        windows.grant("c", json!({})).unwrap();
        let (grant, call) = windows.begin("c").unwrap();
        for i in 0..(KEPT_CALLS + 3) {
            grant.record(Some(&call), json!(i));
        }
        grant.record(None, json!("ignored"));
        let kept = Windows::end(&grant, &call);
        assert_eq!(kept.len(), KEPT_CALLS);
        assert_eq!(kept[0], json!(3));
        assert!(Windows::end(&grant, &call).is_empty());
        assert!(windows.begin("none").is_none());
    }
}

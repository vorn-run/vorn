//! A connection's signed-in profile at work: the sign-in sheet over the
//! window, and the hidden pages (runners) that answer vornd's `session:*`
//! calls from inside the profile, one per connection and origin.

use std::collections::HashMap;
use std::time::{Duration, Instant};

use serde_json::{json, Value};

use super::{App, Call};
use crate::signin::{self, From, Window};
use crate::web::WebEvent;

/// How long a call waits on a page before it is answered with an error.
const CALL_WAIT: Duration = Duration::from_secs(30);

/// A hidden page standing on one origin of a connection.
#[derive(Debug)]
struct Runner {
    origin: String,
    ready: bool,
    /// Calls waiting for the page to load.
    queue: Vec<u64>,
    busy: usize,
    used: Instant,
}

/// Who a call's answer goes to.
#[derive(Debug)]
enum Reply {
    /// vornd's `session:fetch`.
    Fetch(Value),
    /// vornd's `session:check`.
    Check { id: Value, browser: Value },
    /// The sign-in sheet's own check.
    Window,
}

#[derive(Debug)]
struct Wait {
    reply: Reply,
    key: String,
    script: String,
    until: Instant,
}

/// The runners and the calls in flight.
#[derive(Debug, Default)]
pub(super) struct Sessions {
    runners: HashMap<String, Runner>,
    waits: HashMap<u64, Wait>,
    next: u64,
}

/// What a finished sign-in leaves for whoever opened it.
pub(super) type Outcome = Result<Option<String>, String>;

impl App {
    /// vornd's `session:fetch`, `session:check` and `session:forget`.
    pub(super) fn session_bridge(&mut self, id: Value, method: &str, params: Value) {
        let conn = params["connectionId"]
            .as_str()
            .unwrap_or_default()
            .to_owned();
        match method {
            "session:forget" => {
                let conn = params.as_str().map_or(conn, str::to_owned);
                self.forget_profile(&conn);
                self.rpc.respond(id, Ok(Value::Null));
            }
            "session:fetch" => {
                let origins: Vec<String> = params["origins"]
                    .as_array()
                    .into_iter()
                    .flatten()
                    .filter_map(Value::as_str)
                    .map(str::to_owned)
                    .collect();
                let request = params["request"].clone();
                let url = request["url"].as_str().unwrap_or_default();
                if !signin::within_origins(&origins, url) {
                    let e = format!("{url} is not on one of this connection's origins");
                    return self.rpc.respond(id, Err(e));
                }
                self.session_call(&conn, &request, Reply::Fetch(id));
            }
            "session:check" => {
                let browser = params["browser"].clone();
                let request = signin::check_request(&browser);
                let origins = signin::origins(&browser);
                let url = request["url"].as_str().unwrap_or_default();
                if !signin::within_origins(&origins, url) {
                    let e = format!("{url} is not on one of this connection's origins");
                    return self.rpc.respond(id, Err(e));
                }
                self.session_call(&conn, &request, Reply::Check { id, browser });
            }
            other => self.rpc.respond(
                id,
                Err(format!("{other} is not something this app answers.")),
            ),
        }
    }

    /// Runs `request` in `conn`'s profile from a page on its own origin.
    fn session_call(&mut self, conn: &str, request: &Value, reply: Reply) {
        let url = request["url"].as_str().unwrap_or_default();
        let Some(origin) = signin::origin_of(url) else {
            return self.session_failed(reply, format!("{url} is not an https address"));
        };
        let key = signin::runner_key(conn, &origin);
        // A small same-origin page to stand on, so every call is one the site's own page could make.
        if let Err(e) = self
            .web
            .host
            .ensure(&key, &format!("{origin}/robots.txt"), Some(conn))
        {
            return self.session_failed(reply, e);
        }
        let s = &mut self.sessions;
        s.next += 1;
        let n = s.next;
        let runner = s.runners.entry(key.clone()).or_insert_with(|| Runner {
            origin: origin.clone(),
            ready: false,
            queue: Vec::new(),
            busy: 0,
            used: Instant::now(),
        });
        runner.busy += 1;
        runner.used = Instant::now();
        let script = signin::fetch_script(n, request);
        if runner.ready {
            self.web.host.run(&key, &script);
        } else {
            runner.queue.push(n);
        }
        self.sessions.waits.insert(
            n,
            Wait {
                reply,
                key,
                script,
                until: Instant::now() + CALL_WAIT,
            },
        );
    }

    fn session_failed(&mut self, reply: Reply, error: String) {
        match reply {
            Reply::Fetch(id) | Reply::Check { id, .. } => self.rpc.respond(id, Err(error)),
            Reply::Window => {
                if let Some(w) = self.st.signin.as_mut() {
                    w.checking = false;
                    w.next_check = Instant::now() + signin::CHECK_EVERY;
                }
            }
        }
    }

    fn session_answered(&mut self, n: u64, answer: Result<Value, String>) {
        let Some(wait) = self.sessions.waits.remove(&n) else {
            return;
        };
        if let Some(r) = self.sessions.runners.get_mut(&wait.key) {
            r.busy = r.busy.saturating_sub(1);
            r.used = Instant::now();
        }
        match (wait.reply, answer) {
            (reply, Err(e)) => self.session_failed(reply, e),
            (Reply::Fetch(id), Ok(a)) => self.rpc.respond(id, Ok(a)),
            (Reply::Check { id, browser }, Ok(a)) => {
                let (signed_in, identity) = signin::checked(&a, &browser);
                self.rpc.respond(
                    id,
                    Ok(json!({ "signedIn": signed_in, "identity": identity })),
                );
            }
            (Reply::Window, Ok(a)) => {
                let Some(w) = self.st.signin.as_mut() else {
                    return;
                };
                w.checking = false;
                w.next_check = Instant::now() + signin::CHECK_EVERY;
                let (signed_in, identity) = signin::checked(&a, &w.browser);
                if signed_in {
                    let conn = w.connection.clone();
                    self.call(
                        Call::SignedIn(identity.clone()),
                        "connection:signedIn",
                        json!({ "connectionId": conn, "identity": identity }),
                    );
                }
            }
        }
    }

    /// A view of a connection's profile reported something.
    pub(super) fn session_web(&mut self, event: WebEvent) {
        match event {
            WebEvent::Page { key, url, finished } if finished => {
                let Some(r) = self.sessions.runners.get_mut(&key) else {
                    return;
                };
                if signin::origin_of(&url).as_deref() != Some(r.origin.as_str()) {
                    let queued = std::mem::take(&mut r.queue);
                    let origin = r.origin.clone();
                    self.sessions.runners.remove(&key);
                    self.web.host.close(|k| k == key);
                    let e = format!("The signed-in window for {origin} ended up somewhere else");
                    for n in queued {
                        self.session_answered(n, Err(e.clone()));
                    }
                    return;
                }
                r.ready = true;
                for n in std::mem::take(&mut r.queue) {
                    if let Some(w) = self.sessions.waits.get(&n) {
                        self.web.host.run(&key, &w.script);
                    }
                }
            }
            WebEvent::Message { body, .. } => {
                let Ok(msg) = serde_json::from_str::<Value>(&body) else {
                    return;
                };
                let Some(n) = msg["n"].as_u64().filter(|_| msg["t"] == "session") else {
                    return;
                };
                let answer = if msg["ok"] == true {
                    Ok(msg["answer"].clone())
                } else {
                    Err(msg["error"]
                        .as_str()
                        .unwrap_or("The signed-in call failed")
                        .to_owned())
                };
                self.session_answered(n, answer);
            }
            // Sign-in providers open windows of their own; the sheet follows them.
            WebEvent::NewWindow { key, url } if url.starts_with("https://") => {
                self.web.host.load(&key, &url);
            }
            _ => {}
        }
    }

    /// Once a frame: the sheet's checks and deadline, late calls, idle runners.
    pub(super) fn session_tick(&mut self, now: Instant) -> Option<Instant> {
        let late: Vec<u64> = self
            .sessions
            .waits
            .iter()
            .filter(|(_, w)| w.until <= now)
            .map(|(n, _)| *n)
            .collect();
        for n in late {
            self.session_answered(n, Err("The signed-in page did not answer in time.".into()));
        }
        let idle: Vec<String> = self
            .sessions
            .runners
            .iter()
            .filter(|(_, r)| r.busy == 0 && now.duration_since(r.used) >= signin::RUNNER_IDLE)
            .map(|(k, _)| k.clone())
            .collect();
        for key in idle {
            self.sessions.runners.remove(&key);
            self.web.host.close(|k| k == key);
        }
        let mut wake = self.sessions.waits.values().map(|w| w.until).min();
        if !self.sessions.runners.is_empty() {
            let at = now + signin::RUNNER_IDLE;
            wake = Some(wake.map_or(at, |w| w.min(at)));
        }
        let Some(w) = &self.st.signin else {
            return wake;
        };
        if now.duration_since(w.opened) >= signin::SIGN_IN_TIMEOUT {
            self.finish_signin(Err(signin::TIMED_OUT.to_owned()));
            return wake;
        }
        if !w.checking && now >= w.next_check {
            let request = signin::check_request(&w.browser);
            let conn = w.connection.clone();
            if let Some(w) = self.st.signin.as_mut() {
                w.checking = true;
            }
            self.session_call(&conn, &request, Reply::Window);
        }
        let next = self.st.signin.as_ref().map(|w| w.next_check);
        match (wake, next) {
            (Some(a), Some(b)) => Some(a.min(b)),
            (a, b) => a.or(b),
        }
    }

    /// Opens the sheet on `browserAuth`'s answer, or says why it cannot.
    pub fn open_signin(&mut self, conn: &str, target: &Value, from: From) -> Option<String> {
        if self
            .st
            .signin
            .as_ref()
            .is_some_and(|w| w.connection == conn)
        {
            return None;
        }
        if self.st.signin.is_some() {
            self.finish_signin(Err(signin::CLOSED.to_owned()));
        }
        match Window::open(conn, target, None, from, Instant::now()) {
            Ok(w) => {
                self.st.signin = Some(w);
                self.st.dialog = Some(crate::state::Dialog::SignIn);
                None
            }
            Err(e) => Some(e),
        }
    }

    /// Closes the sheet and tells whoever opened it how it went.
    pub(super) fn finish_signin(&mut self, outcome: Outcome) {
        let Some(w) = self.st.signin.take() else {
            return;
        };
        if self.st.dialog == Some(crate::state::Dialog::SignIn) {
            self.st.dialog = None;
        }
        let key = signin::window_key(&w.connection);
        self.web.host.close(|k| k == key);
        match w.from {
            From::Plugins => self.plug_signed_in(&w.connection, outcome),
            From::Run => match outcome {
                Ok(_) => self.st.toast(format!("Signed in to {}", w.name), false),
                Err(e) => self.st.toast(e, true),
            },
        }
    }

    /// Drops a connection's views and what its profile keeps.
    pub(super) fn forget_profile(&mut self, conn: &str) {
        let gone: Vec<String> = self
            .sessions
            .runners
            .keys()
            .filter(|k| signin::connection_of(k) == Some(conn))
            .cloned()
            .collect();
        for key in gone {
            self.sessions.runners.remove(&key);
        }
        if self
            .st
            .signin
            .as_ref()
            .is_some_and(|w| w.connection == conn)
        {
            self.finish_signin(Err(signin::CLOSED.to_owned()));
        }
        self.web.host.forget(conn);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_window_needs_a_browser_sign_in() {
        let now = Instant::now();
        let none = Window::open("c", &json!({ "name": "X" }), None, From::Plugins, now);
        assert!(none.is_err());
        let target = json!({ "name": "Site", "browser": {
            "signInUrl": "https://site.io/login", "origins": ["https://site.io"],
            "check": { "url": "https://site.io/me" } } });
        let w = Window::open("c", &target, None, From::Plugins, now).expect("opens");
        assert_eq!(w.url, "https://site.io/login");
        assert_eq!(w.title(), "Sign in \u{b7} Site");
        let off = Window::open("c", &target, Some("https://evil.io/"), From::Run, now);
        assert_eq!(
            off.err().as_deref(),
            Some("That link is not on https://site.io.")
        );
    }
}

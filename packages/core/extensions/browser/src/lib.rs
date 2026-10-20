//! Browser: web pages in tabs beside a session's terminal, opened from its
//! card or by its agent, which reads them, finds in them and acts on them.
//!
//! Vorn ships it as a Wasm component (`vorn-extension.toml` beside this
//! crate). It keeps each session's tabs and draws them as a tree: a tab
//! strip over a `web` part the client seats a native web view in. Whatever
//! needs the live page (reading it, acting on it, its history and logs)
//! goes through Vorn's `web.call` to the client showing it, keyed by the
//! session as the view. The tabs live in its storage and rows, so a restart
//! of its instance loses nothing a person sees.

use vorn_extension_sdk::tree::{self, EventKind, Surface, Tabs, Tool};
use vorn_extension_sdk::{arg, json, serde_json, Ctx, Error, Extension, Host, Init, Op, Value};

pub mod address;
mod pane;

pub use pane::{Pane, Tab, TabArtifact};

/// The pane beside a session's terminal, drawn for the session as its scope.
pub const PANE: &str = "browser";
/// One row per session whose pane is open, its id the session's.
pub const OPEN: &str = "open";
/// What a page call says when the session has no pane to drive.
pub const NO_PANE: &str =
    "This session has no browser pane open. Open one from the session card, then retry.";
/// The tab strip's node, and its new-tab tool.
/// What the address bar says of words that are no address and no search.
pub const NOT_AN_ADDRESS: &str = "That does not look like a web address";
const TABS: &str = "tabs";
const NEW_TAB: &str = "new";
/// The page's node.
const PAGE: &str = "page";

fn pane_key(session: &str) -> String {
    format!("pane/{session}")
}

/// The tabs of a closed pane, for opening it as it was left.
fn memory_key(session: &str) -> String {
    format!("memory/{session}")
}

fn now_ms() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |d| u64::try_from(d.as_millis()).unwrap_or(u64::MAX))
}

fn failed(e: impl std::fmt::Display) -> Error {
    Error::new("failed", e.to_string())
}

/// The agent's own words for an address it may not load.
fn refused(verb: &str, url: &str) -> Error {
    failed(format!(
        "Refusing to {verb} \"{url}\" \u{2014} not an allowed web address."
    ))
}

/// The extension. It holds nothing between calls: the panes are in storage.
#[derive(Debug, Default)]
pub struct Browser;

impl Browser {
    fn load(host: &mut dyn Host, key: &str) -> Result<Option<Pane>, Error> {
        Ok(host.get(key)?.and_then(|v| serde_json::from_value(v).ok()))
    }

    fn pane(host: &mut dyn Host, session: &str) -> Result<Option<Pane>, Error> {
        Browser::load(host, &pane_key(session))
    }

    fn save(host: &mut dyn Host, session: &str, p: &Pane) -> Result<(), Error> {
        host.set(
            &pane_key(session),
            &serde_json::to_value(p).map_err(failed)?,
        )?;
        // Every change announces the session's row again, so clients draw that pane again.
        host.publish(
            OPEN,
            vec![Op::Upsert(json!({ "id": session, "at": now_ms() }))],
        );
        Ok(())
    }

    /// The pane goes; its tabs are kept for the next open unless `forget`.
    fn close(host: &mut dyn Host, session: &str, keep: Option<&Pane>) -> Result<(), Error> {
        let memory = keep.map_or(Ok(Value::Null), |p| {
            serde_json::to_value(p.clone().intent())
        });
        host.set(&memory_key(session), &memory.map_err(failed)?)?;
        host.set(&pane_key(session), &Value::Null)?;
        host.publish(OPEN, vec![Op::Delete(session.to_owned())]);
        Ok(())
    }

    /// The pane, or the agent told to open one.
    fn open_pane(host: &mut dyn Host, session: &str) -> Result<Pane, Error> {
        Browser::pane(host, session)?.ok_or_else(|| failed(NO_PANE))
    }

    /// The session's working folder, the one place its `file:` pages may come from.
    fn root(host: &mut dyn Host, session: &str) -> Result<Option<String>, Error> {
        let s = host
            .call("sessions.get", json!({ "sessionId": session }))
            .ok()
            .filter(Value::is_object)
            .ok_or_else(|| failed(format!("No session {session} in this app.")))?;
        let text = |k: &str| s.get(k).and_then(Value::as_str).filter(|p| !p.is_empty());
        Ok(text("worktreePath")
            .or(text("projectPath"))
            .map(str::to_owned))
    }

    /// Asks the client showing the session's page to `op` on it.
    fn web(host: &mut dyn Host, session: &str, op: &str, params: Value) -> Result<Value, Error> {
        host.call(
            "web.call",
            json!({ "view": session, "op": op, "params": params }),
        )
    }

    /// Opens or closes the pane from the card; open, it is as it was left.
    fn toggle(host: &mut dyn Host, session: &str) -> Result<(), Error> {
        match Browser::pane(host, session)? {
            Some(p) => Browser::close(host, session, Some(&p)),
            None => {
                let p = Browser::load(host, &memory_key(session))?.unwrap_or_default();
                Browser::save(host, session, &p)
            }
        }
    }

    /// An agent's `browser:*` call, `method` without its group.
    fn agent(&mut self, host: &mut dyn Host, method: &str, args: &Value) -> Result<Value, Error> {
        let session = arg(args, "sessionId", method)?.to_owned();
        let session = session.as_str();
        // Each call names a session of this Vorn; the pages answer for theirs only.
        let root = Browser::root(host, session)?;
        match method {
            "openPane" => Browser::agent_open(host, session, root.as_deref(), args),
            "navigate" => {
                let mut p = Browser::open_pane(host, session)?;
                let raw = args.get("url").and_then(Value::as_str).unwrap_or_default();
                let url = address::loadable_url(raw, root.as_deref())
                    .ok_or_else(|| refused("navigate to", raw))?;
                p.go(url.clone());
                Browser::save(host, session, &p)?;
                // Loaded now even when the tab was asked for this address before.
                Browser::web(host, session, "load", json!({ "url": url }))?;
                Ok(json!({ "url": url }))
            }
            "tabs" => {
                let p = Browser::open_pane(host, session)?;
                let action = args.get("action").and_then(Value::as_str).unwrap_or_default();
                let index = args
                    .get("index")
                    .and_then(Value::as_u64)
                    .and_then(|i| usize::try_from(i).ok());
                match (action, index) {
                    ("add", _) => {
                        let url = match args.get("url").and_then(Value::as_str) {
                            Some(u) => Some(
                                address::loadable_url(u, root.as_deref())
                                    .ok_or_else(|| refused("open", u))?,
                            ),
                            None => None,
                        };
                        let mut p = p;
                        p.add_tab(url);
                        Browser::save(host, session, &p)?;
                    }
                    ("close", Some(i)) => match p.close_tab(i) {
                        Some(p) => Browser::save(host, session, &p)?,
                        None => Browser::close(host, session, None)?,
                    },
                    ("select", Some(i)) => {
                        let mut p = p;
                        if p.select_tab(i) {
                            Browser::save(host, session, &p)?;
                        }
                    }
                    (action, _) => {
                        return Err(failed(format!(
                            "A tab index is required to {action} a tab."
                        )))
                    }
                }
                Ok(json!({ "ok": true }))
            }
            "listTabs" => {
                let p = Browser::open_pane(host, session)?;
                Ok(json!({ "tabs": p.tab_infos() }))
            }
            "screenshot" => Err(failed(
                "Page screenshots are not available in the native app yet. Use read_page or get_page_text.",
            )),
            _ => {
                let op = match method {
                    "history" => "history",
                    "readPage" => "read",
                    "getText" => "text",
                    "find" => "find",
                    "interact" => "act",
                    "consoleMessages" => "console",
                    "networkRequests" => "network",
                    other => return Err(Error::unknown(&format!("browser:{other}"))),
                };
                Browser::open_pane(host, session)?;
                Browser::web(host, session, op, args.clone())
            }
        }
    }

    /// `browser:openPane`: with a vetted `url` the front tab goes there, or
    /// an artifact's tab is put in front; without, the pane opens as left.
    fn agent_open(
        host: &mut dyn Host,
        session: &str,
        root: Option<&str>,
        args: &Value,
    ) -> Result<Value, Error> {
        let open = Browser::pane(host, session)?;
        let Some(raw) = args.get("url").and_then(Value::as_str) else {
            if open.is_none() {
                Browser::toggle(host, session)?;
            }
            return Ok(json!({ "url": "" }));
        };
        let url = address::loadable_url(raw, root).ok_or_else(|| refused("open", raw))?;
        let artifact = args
            .get("artifact")
            .cloned()
            .and_then(|a| serde_json::from_value::<TabArtifact>(a).ok());
        let p = match (artifact, open) {
            (Some(a), open) => {
                let mut p = open
                    .or(Browser::load(host, &memory_key(session))?)
                    .unwrap_or_else(|| Pane {
                        tabs: Vec::new(),
                        active_tab: 0,
                    });
                p.open_artifact(url.clone(), a);
                p
            }
            (None, Some(mut p)) => {
                p.go(url.clone());
                p
            }
            (None, None) => Pane::new(Tab::at(url.clone())),
        };
        Browser::save(host, session, &p)?;
        Ok(json!({ "url": url }))
    }

    /// The pane's tree: the tabs over the page in front.
    fn render_pane(host: &mut dyn Host, session: &str) -> Result<tree::Node, Error> {
        let Some(p) = Browser::pane(host, session)? else {
            return Ok(tree::sdk::col([]));
        };
        let tabs = p
            .tabs
            .iter()
            .enumerate()
            .map(|(i, t)| tree::Tab {
                id: i.to_string(),
                label: t.label(),
                active: i == p.active_tab,
                icon: t.icon().map(str::to_owned),
                ..tree::Tab::default()
            })
            .collect();
        let strip = Tabs {
            tabs,
            tools: vec![Tool {
                id: NEW_TAB.to_owned(),
                label: "New tab".to_owned(),
                icon: "plus".to_owned(),
                on: false,
            }],
            ..Tabs::default()
        };
        let url = p.active().map_or(address::DEFAULT_URL, |t| t.url.as_str());
        Ok(tree::sdk::col([
            tree::sdk::tabs(TABS, strip),
            tree::sdk::web(PAGE, session, url).grow(),
        ])
        .grow())
    }

    fn pane_event(host: &mut dyn Host, session: &str, e: &tree::Event) -> Result<Value, Error> {
        let Some(mut p) = Browser::pane(host, session)? else {
            return Ok(json!({ "same": true }));
        };
        let index = || e.value.parse::<usize>().ok();
        match (e.id.as_str(), &e.kind) {
            (TABS, EventKind::Select) => {
                if !index().is_some_and(|i| p.select_tab(i)) {
                    return Ok(json!({ "same": true }));
                }
            }
            (TABS, EventKind::Close) => {
                let Some(i) = index() else {
                    return Ok(json!({ "same": true }));
                };
                return match p.close_tab(i) {
                    Some(p) => Browser::save(host, session, &p).map(|()| Value::Null),
                    None => Browser::close(host, session, None).map(|()| Value::Null),
                };
            }
            (NEW_TAB, EventKind::Click) => p.add_tab(None),
            (PAGE, EventKind::Page) => {
                let said: Value = serde_json::from_str(&e.value)
                    .map_err(|_| Error::bad_args("a page event's value is a JSON object"))?;
                let text = |k: &str| said.get(k).and_then(Value::as_str).map(str::to_owned);
                match said.get("type").and_then(Value::as_str).unwrap_or_default() {
                    "state" => {
                        let changed = p
                            .active_mut()
                            .is_some_and(|t| t.sync(text("url"), text("title"), text("design")));
                        if !changed {
                            return Ok(json!({ "same": true }));
                        }
                    }
                    // Typed in the address bar: a page, or a search for words.
                    // Enter on the page already shown loads it again.
                    "submit" => {
                        let url = text("text")
                            .as_deref()
                            .and_then(address::address_target)
                            .ok_or_else(|| failed(NOT_AN_ADDRESS))?;
                        p.go(url.clone());
                        Browser::save(host, session, &p)?;
                        // A page that is not up yet loads from the tree instead.
                        let _ = Browser::web(host, session, "load", json!({ "url": url }));
                        return Ok(Value::Null);
                    }
                    // A page that opens a window opens a tab, when it may load there.
                    "opened" => {
                        let root = Browser::root(host, session)?;
                        let Some(url) =
                            text("url").and_then(|u| address::loadable_url(&u, root.as_deref()))
                        else {
                            return Ok(json!({ "same": true }));
                        };
                        p.add_tab(Some(url));
                    }
                    // The client's own controls moved the front tab, as to
                    // another version of its review: it keeps its artifact.
                    "go" => {
                        let (Some(url), Some(t)) = (text("url"), p.active_mut()) else {
                            return Ok(json!({ "same": true }));
                        };
                        let version = said
                            .get("version")
                            .and_then(Value::as_u64)
                            .and_then(|v| u32::try_from(v).ok());
                        let artifact = t.artifact.clone().map(|a| TabArtifact {
                            version: version.unwrap_or(a.version),
                            ..a
                        });
                        *t = Tab {
                            artifact,
                            ..Tab::at(url)
                        };
                    }
                    other => {
                        return Err(Error::bad_args(format!("no page event {other}")));
                    }
                }
            }
            _ => {
                return Err(Error::new(
                    "unknown-action",
                    format!("{PANE} has no {}", e.id),
                ))
            }
        }
        Browser::save(host, session, &p)?;
        Ok(Value::Null)
    }

    fn act(&mut self, host: &mut dyn Host, action: &str, args: &Value) -> Result<Value, Error> {
        if let Some(method) = action.strip_prefix("browser:") {
            return self.agent(host, method, args);
        }
        match action {
            "toggle" => {
                let session = arg(args, "session", action)?;
                Browser::toggle(host, session)?;
                Ok(json!({}))
            }
            // What sessions are left: the panes and memories of the rest go.
            "prune" => {
                let live: Vec<&str> = args
                    .get("live")
                    .and_then(Value::as_array)
                    .into_iter()
                    .flatten()
                    .filter_map(Value::as_str)
                    .collect();
                let open: Vec<String> = host
                    .rows(OPEN)?
                    .iter()
                    .filter_map(|r| r.get("id").and_then(Value::as_str).map(str::to_owned))
                    .collect();
                for session in open.iter().filter(|o| !live.contains(&o.as_str())) {
                    Browser::close(host, session, None)?;
                }
                Ok(Value::Null)
            }
            // What the desktop kept on its own before: its open panes and the
            // tabs of closed ones. A pane already here stays as it is.
            "import" => {
                let mut n = 0;
                for (field, open) in [("open", true), ("memory", false)] {
                    let Some(panes) = args.get(field).and_then(Value::as_object) else {
                        continue;
                    };
                    for (session, p) in panes {
                        let Ok(p) = serde_json::from_value::<Pane>(p.clone()) else {
                            continue;
                        };
                        if p.tabs.is_empty() || Browser::pane(host, session)?.is_some() {
                            continue;
                        }
                        let p = p.intent();
                        if open {
                            Browser::save(host, session, &p)?;
                        } else if Browser::load(host, &memory_key(session))?.is_none() {
                            host.set(
                                &memory_key(session),
                                &serde_json::to_value(&p).map_err(failed)?,
                            )?;
                        }
                        n += 1;
                    }
                }
                Ok(json!({ "imported": n }))
            }
            other => Err(Error::unknown(other)),
        }
    }
}

impl Extension for Browser {
    fn init(_: &Init, _: &mut dyn Host) -> Result<Self, Error> {
        Ok(Browser)
    }

    fn handle(
        &mut self,
        host: &mut dyn Host,
        method: &str,
        args: Value,
        _: &Ctx,
    ) -> Result<Value, Error> {
        self.act(host, method, &args)
    }

    fn render(
        &mut self,
        host: &mut dyn Host,
        point: &str,
        surface: Surface,
    ) -> Result<tree::Node, Error> {
        match point {
            PANE => Browser::render_pane(host, &surface.scope.unwrap_or_default()),
            _ => Err(Error::new(
                "unknown-action",
                format!("nothing draws {point}"),
            )),
        }
    }

    fn event(&mut self, host: &mut dyn Host, point: &str, e: tree::Event) -> Result<Value, Error> {
        if point != PANE {
            return Err(Error::new(
                "unknown-action",
                format!("{point} has no {}", e.id),
            ));
        }
        let session = e.scope.clone().unwrap_or_default();
        // Null: the tree is drawn again; an object is the answer as it stands.
        Ok(match Browser::pane_event(host, &session, &e)? {
            Value::Null => json!({}),
            other => other,
        })
    }
}

vorn_extension_sdk::export!(Browser);

#[cfg(test)]
mod tests;

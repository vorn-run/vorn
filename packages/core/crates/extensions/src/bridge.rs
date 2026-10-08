//! The bridge an extension calls back on: who may ask, about which session,
//! and what each method costs.
//!
//! An extension's own process proves itself with its token; a pane's page
//! with the nonce in its address, and speaks only for the session that pane
//! was opened on. A call failing a check is refused outright, so an
//! extension learns the rule once rather than reading an empty answer.

use serde_json::{Map, Value};

use crate::js;
use crate::manifest::Permission;
use crate::pack::InstalledPack;

const DEFAULT_OUTPUT_LINES: usize = 200;
const MAX_OUTPUT_LINES: f64 = 5000.0;
/// A line has no length of its own, so the output is held to a size too.
pub const MAX_OUTPUT_UNITS: usize = 256 * 1024;

/// What each method costs, the table the SDK checks against too.
pub fn permission_of(method: &str) -> Option<Permission> {
    Some(match method {
        "diff" | "status" => Permission::GitRead,
        "output" => Permission::TerminalRead,
        "selection" => Permission::TerminalSelection,
        "send" => Permission::TerminalSend,
        "rename" => Permission::CardRename,
        "usage" => Permission::AgentUsage,
        _ => return None,
    })
}

/// An answer the bridge gives without asking anyone.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Refusal {
    pub status: u16,
    pub message: String,
}

impl Refusal {
    pub fn new(status: u16, message: impl Into<String>) -> Refusal {
        Refusal {
            status,
            message: message.into(),
        }
    }
}

/// Who is calling, once proven.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Caller {
    pub extension_id: String,
    pub project_path: String,
}

/// The session a call is about, as the bridge needs to see it.
#[derive(Debug, Clone, Copy)]
pub struct SessionView<'a> {
    pub id: &'a str,
    pub project_path: &'a str,
    pub renamed_by_person: bool,
}

/// A call, admitted and read.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Request {
    Diff,
    Status,
    Output { lines: usize },
    Selection,
    Send { text: String },
    Rename { name: String },
    Usage,
}

/// The checks before a session is looked at, in the order the caller
/// learns of them: who it is, that it is installed, that the method exists,
/// that its manifest asked for it.
pub fn admit(
    caller: Option<&Caller>,
    pack: Option<&InstalledPack>,
    method: &str,
) -> Result<(), Refusal> {
    let caller =
        caller.ok_or_else(|| Refusal::new(401, "This bridge does not know that caller"))?;
    let pack = pack.filter(|p| p.is_extension()).ok_or_else(|| {
        Refusal::new(
            404,
            format!("No extension \"{}\" is installed", caller.extension_id),
        )
    })?;
    let permission = permission_of(method)
        .ok_or_else(|| Refusal::new(404, format!("The host serves no method \"{method}\"")))?;
    if !pack
        .permissions
        .as_deref()
        .unwrap_or_default()
        .contains(&permission)
    {
        return Err(Refusal::new(
            403,
            format!("this extension does not ask for {}", permission.name()),
        ));
    }
    Ok(())
}

/// The session id a call names: the pane's, for a page, else the body's.
pub fn session_named<'a>(bound: Option<&'a str>, body: &'a Map<String, Value>) -> Option<&'a str> {
    bound
        .or_else(|| body.get("sessionId").and_then(Value::as_str))
        .filter(|id| !id.is_empty())
}

/// The checks on the session, then the call read from its body. A body that
/// does not say what the method needs fails with 500, as a thrown error did.
pub fn request(
    caller: &Caller,
    method: &str,
    session: Option<SessionView<'_>>,
    body: &Map<String, Value>,
) -> Result<Request, Refusal> {
    let session = session.ok_or_else(|| Refusal::new(404, "That session is not running"))?;
    if session.project_path != caller.project_path {
        return Err(Refusal::new(403, "That session belongs to another project"));
    }
    Ok(match method {
        "diff" => Request::Diff,
        "status" => Request::Status,
        "output" => Request::Output {
            lines: output_lines(body.get("lines")),
        },
        "selection" => Request::Selection,
        "send" => match body.get("text") {
            Some(Value::String(text)) => Request::Send { text: text.clone() },
            _ => return Err(Refusal::new(500, "send takes the text to type")),
        },
        "rename" => {
            let name = body
                .get("name")
                .and_then(Value::as_str)
                .map(js::trim)
                .unwrap_or_default();
            if name.is_empty() {
                return Err(Refusal::new(500, "rename takes the name to show"));
            }
            if session.renamed_by_person {
                return Err(Refusal::new(
                    403,
                    "that card was named by the person using it",
                ));
            }
            Request::Rename {
                name: name.to_owned(),
            }
        }
        "usage" => Request::Usage,
        _ => {
            return Err(Refusal::new(
                404,
                format!("The host serves no method \"{method}\""),
            ))
        }
    })
}

/// How many lines `output` reads: what was asked, whole, within 1 to 5000.
fn output_lines(asked: Option<&Value>) -> usize {
    match asked.and_then(Value::as_f64) {
        Some(n) => n.trunc().clamp(1.0, MAX_OUTPUT_LINES) as usize,
        None => DEFAULT_OUTPUT_LINES,
    }
}

/// The lines read, joined and cut from the front: a reader wants how it ended.
pub fn output_text(lines: &[String]) -> String {
    let text = lines.join("\n");
    js::tail16(&text, MAX_OUTPUT_UNITS).to_owned()
}

/// Whether a browser says the request came from another site. An
/// extension's process sends neither header; a page sends both.
pub fn from_elsewhere(
    sec_fetch_site: Option<&str>,
    origin: Option<&str>,
    host: Option<&str>,
) -> bool {
    if sec_fetch_site.is_some_and(|s| !s.is_empty() && s != "same-origin" && s != "none") {
        return true;
    }
    origin.is_some_and(|o| !o.is_empty() && o != format!("http://{}", host.unwrap_or("undefined")))
}

/// The token in an `Authorization: Bearer` header.
pub fn bearer(header: Option<&str>) -> Option<&str> {
    let token = js::trim(header?.strip_prefix("Bearer ")?);
    (!token.is_empty()).then_some(token)
}

/// The extension paths, decoded.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Route {
    /// `POST /extensions/:id/bridge/:method`: the extension's own process.
    Bridge { id: String, method: String },
    /// `POST /extensions/:id/pane/:pane/:nonce/bridge/:method`: a pane's page.
    PaneBridge {
        id: String,
        pane: String,
        nonce: String,
        method: String,
    },
    /// `GET /extensions/:id/pane/:pane/:nonce/*`: a pane's files.
    Page {
        id: String,
        pane: String,
        nonce: String,
        rest: String,
    },
}

/// The route `path` (without its query) names, if any.
pub fn route(method: &str, path: &str) -> Option<Route> {
    let rest = path.strip_prefix("/extensions/")?;
    let parts: Vec<&str> = rest.split('/').collect();
    let seg = |i: usize| -> Option<String> {
        let raw = parts.get(i)?;
        if raw.is_empty() {
            return None;
        }
        crate::page::decode_path(raw)
    };
    match (method, parts.as_slice()) {
        ("POST", [_, "bridge", _]) => Some(Route::Bridge {
            id: seg(0)?,
            method: seg(2)?,
        }),
        ("POST", [_, "pane", _, _, "bridge", _]) => Some(Route::PaneBridge {
            id: seg(0)?,
            pane: seg(2)?,
            nonce: seg(3)?,
            method: seg(5)?,
        }),
        ("GET" | "HEAD", [_, "pane", _, _, _, ..]) => {
            let tail = parts[4..].join("/");
            let rest = crate::page::decode_path(&tail)?;
            Some(Route::Page {
                id: seg(0)?,
                pane: seg(2)?,
                nonce: seg(3)?,
                rest: rest.trim_start_matches('/').to_owned(),
            })
        }
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::pack::{fixture::install, PackStore};
    use serde_json::json;

    fn caller() -> Caller {
        Caller {
            extension_id: "x".into(),
            project_path: "/p".into(),
        }
    }

    fn pack(permissions: Value) -> (tempfile::TempDir, InstalledPack) {
        let root = tempfile::tempdir().unwrap();
        install(
            root.path(),
            "x",
            "1.0.0",
            &json!({ "id": "x", "name": "X", "kind": "extension", "permissions": permissions,
                    "contributes": { "footers": [{ "id": "f", "every": 5 }] } }),
        );
        let pack = PackStore::new(root.path()).describe("x").unwrap();
        (root, pack)
    }

    #[test]
    fn admits_in_the_order_a_caller_learns_the_rules() {
        let (_root, p) = pack(json!(["git.read"]));
        let refused =
            |c: Option<&Caller>, pk: Option<&InstalledPack>, m: &str| admit(c, pk, m).unwrap_err();
        assert_eq!(refused(None, Some(&p), "diff").status, 401);
        assert_eq!(
            refused(Some(&caller()), None, "diff").message,
            "No extension \"x\" is installed"
        );
        assert_eq!(
            refused(Some(&caller()), Some(&p), "nope").message,
            "The host serves no method \"nope\""
        );
        assert_eq!(
            refused(Some(&caller()), Some(&p), "send"),
            Refusal::new(403, "this extension does not ask for terminal.send")
        );
        assert!(admit(Some(&caller()), Some(&p), "status").is_ok());
    }

    #[test]
    fn a_connector_is_not_an_extension() {
        let root = tempfile::tempdir().unwrap();
        install(
            root.path(),
            "x",
            "1.0.0",
            &json!({ "id": "x", "name": "X", "actions": [{ "type": "a" }] }),
        );
        let p = PackStore::new(root.path()).describe("x").unwrap();
        assert_eq!(
            admit(Some(&caller()), Some(&p), "diff").unwrap_err().status,
            404
        );
    }

    #[test]
    fn reads_the_session_then_the_body() {
        let body = |v: Value| v.as_object().unwrap().clone();
        let mine = SessionView {
            id: "s",
            project_path: "/p",
            renamed_by_person: false,
        };
        let theirs = SessionView {
            project_path: "/q",
            ..mine
        };
        let named = SessionView {
            renamed_by_person: true,
            ..mine
        };
        let c = caller();
        assert_eq!(
            request(&c, "diff", None, &Map::new()).unwrap_err().status,
            404
        );
        assert_eq!(
            request(&c, "diff", Some(theirs), &Map::new())
                .unwrap_err()
                .status,
            403
        );
        assert_eq!(
            request(&c, "output", Some(mine), &body(json!({ "lines": 9e9 }))),
            Ok(Request::Output { lines: 5000 })
        );
        assert_eq!(
            request(&c, "output", Some(mine), &body(json!({ "lines": -3.7 }))),
            Ok(Request::Output { lines: 1 })
        );
        assert_eq!(
            request(&c, "output", Some(mine), &body(json!({ "lines": "5" }))),
            Ok(Request::Output { lines: 200 })
        );
        assert_eq!(
            request(&c, "send", Some(mine), &Map::new()).unwrap_err(),
            Refusal::new(500, "send takes the text to type")
        );
        assert_eq!(
            request(&c, "send", Some(mine), &body(json!({ "text": "" }))),
            Ok(Request::Send {
                text: String::new()
            })
        );
        assert_eq!(
            request(&c, "rename", Some(mine), &body(json!({ "name": "  " })))
                .unwrap_err()
                .status,
            500
        );
        assert_eq!(
            request(&c, "rename", Some(named), &body(json!({ "name": "a" })))
                .unwrap_err()
                .status,
            403
        );
        assert_eq!(
            request(&c, "rename", Some(mine), &body(json!({ "name": " a " }))),
            Ok(Request::Rename { name: "a".into() })
        );
        assert_eq!(
            session_named(Some("bound"), &body(json!({ "sessionId": "s" }))),
            Some("bound")
        );
        assert_eq!(session_named(None, &body(json!({ "sessionId": "" }))), None);
    }

    #[test]
    fn keeps_how_the_output_ended() {
        let long = vec!["a".repeat(MAX_OUTPUT_UNITS), "end".into()];
        let text = output_text(&long);
        assert_eq!(text.len(), MAX_OUTPUT_UNITS);
        assert!(text.ends_with("\nend"));
    }

    #[test]
    fn tells_a_page_from_elsewhere() {
        assert!(!from_elsewhere(None, None, Some("127.0.0.1:1")));
        assert!(!from_elsewhere(
            Some("same-origin"),
            Some("http://127.0.0.1:1"),
            Some("127.0.0.1:1")
        ));
        assert!(from_elsewhere(Some("cross-site"), None, None));
        assert!(from_elsewhere(
            None,
            Some("http://evil"),
            Some("127.0.0.1:1")
        ));
        assert_eq!(bearer(Some("Bearer  abc ")), Some("abc"));
        assert_eq!(bearer(Some("Basic abc")), None);
        assert_eq!(bearer(Some("bearer abc")), None);
    }

    #[test]
    fn reads_the_routes() {
        assert_eq!(
            route("POST", "/extensions/x/bridge/diff"),
            Some(Route::Bridge {
                id: "x".into(),
                method: "diff".into()
            })
        );
        assert_eq!(
            route("POST", "/extensions/x/pane/p/n/bridge/output"),
            Some(Route::PaneBridge {
                id: "x".into(),
                pane: "p".into(),
                nonce: "n".into(),
                method: "output".into()
            })
        );
        assert_eq!(
            route("GET", "/extensions/x/pane/p/n/"),
            Some(Route::Page {
                id: "x".into(),
                pane: "p".into(),
                nonce: "n".into(),
                rest: String::new()
            })
        );
        assert_eq!(
            route("GET", "/extensions/x/pane/p/n//lib/a%20b.js"),
            Some(Route::Page {
                id: "x".into(),
                pane: "p".into(),
                nonce: "n".into(),
                rest: "lib/a b.js".into()
            })
        );
        assert_eq!(route("GET", "/extensions/x/pane/p/n"), None);
        assert_eq!(route("GET", "/extensions/x/bridge/diff"), None);
        assert_eq!(route("POST", "/extensions//bridge/diff"), None);
    }
}

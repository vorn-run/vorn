//! Turns: counts each agent's turns from the session events Vorn sends, and
//! draws them as element trees in every place an extension can draw: the
//! sidebar, a card's badge, footer and sidebar row, a pane, the composer, a
//! settings page, a window and a dialog.
//!
//! Everything it knows is in the rows it publishes, so an instance Vorn stops
//! when idle starts again where it was; each tree is drawn from those rows.

use vorn_extension_sdk::tree::sdk::*;
use vorn_extension_sdk::tree::{self, EventKind, Node, Surface, TextStyle, Tone};
use vorn_extension_sdk::{json, Ctx, Error, Event, Extension, Host, Init, Op, Value};

/// How many finished turns the sidebar keeps.
const KEEP: usize = 8;
const SESSIONS: &str = "sessions";
const FINISHED: &str = "finished";
const PREFS: &str = "prefs";
const PLANS: &str = "plans";
const ACCOUNT: &str = "account";
const ACCOUNT_ID: &str = "turns-account";

/// The extension; it keeps nothing outside its rows.
#[derive(Debug, Default)]
pub struct Turns;

fn s<'a>(v: &'a Value, key: &str) -> &'a str {
    v[key].as_str().unwrap_or_default()
}

fn n(v: &Value, key: &str) -> u64 {
    v[key].as_u64().unwrap_or(0)
}

fn on(v: &Value, key: &str) -> bool {
    v[key].as_bool() == Some(true)
}

/// The time now as `HH:MM UTC`; a component has no time zone.
fn clock() -> String {
    let secs = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |d| d.as_secs());
    let day = secs % 86_400;
    format!("{:02}:{:02} UTC", day / 3600, day % 3600 / 60)
}

fn plans(picked: &str) -> Vec<Op> {
    [("free", "Free plan"), ("team", "Team plan")]
        .into_iter()
        .map(|(id, name)| Op::Upsert(json!({ "id": id, "name": name, "selected": id == picked })))
        .collect()
}

/// A session's status as its dot's tone.
fn tone(status: &str) -> Tone {
    match status {
        "running" => Tone::Busy,
        "waiting" => Tone::Warn,
        "error" => Tone::Bad,
        _ => Tone::Muted,
    }
}

fn quiet(t: impl Into<String>) -> Node {
    text(t).style(TextStyle::Secondary)
}

fn line(kids: impl IntoIterator<Item = Node>) -> Node {
    row(kids).gap(8.0)
}

/// `At work: 1 of 3`.
fn at_work(sessions: &[Value]) -> String {
    let running = sessions
        .iter()
        .filter(|r| s(r, "status") == "running")
        .count();
    format!("At work: {running} of {}", sessions.len())
}

impl Turns {
    /// A batch of session events: new sessions, finished turns, sessions gone.
    fn heard(host: &mut dyn Host, batch: &[Event]) -> Result<(), Error> {
        let shells = host
            .rows(PREFS)?
            .iter()
            .find(|p| s(p, "id") == "shells")
            .is_none_or(|p| on(p, "on"));
        let mut sessions = host.rows(SESSIONS)?;
        let mut finished = host.rows(FINISHED)?;
        let mut gone = Vec::new();
        for e in batch {
            let id = s(&e.data, "sessionId").to_owned();
            match e.name.as_str() {
                "session.created" => {
                    if s(&e.data, "kind") == "shell" && !shells {
                        continue;
                    }
                    let name = Some(s(&e.data, "agent")).filter(|a| !a.is_empty());
                    sessions.retain(|r| s(r, "id") != id);
                    sessions.push(json!({
                        "id": id, "name": name.unwrap_or("shell"), "status": "running",
                        "turns": 0, "last": "",
                    }));
                }
                "session.status" => {
                    let Some(r) = sessions.iter_mut().find(|r| s(r, "id") == id) else {
                        continue;
                    };
                    let now = s(&e.data, "status").to_owned();
                    if s(r, "status") == "running" && matches!(now.as_str(), "idle" | "waiting") {
                        let turn = n(r, "turns") + 1;
                        let at = clock();
                        r["turns"] = json!(turn);
                        r["last"] = json!(at);
                        let name = s(r, "name").to_owned();
                        finished.insert(
                            0,
                            json!({ "id": format!("{id}-{turn}"), "session": id, "name": name,
                                    "turn": turn, "at": at }),
                        );
                        finished.truncate(KEEP);
                    }
                    r["status"] = json!(now);
                }
                "session.exited" => {
                    sessions.retain(|r| s(r, "id") != id);
                    gone.push(id);
                }
                _ => {}
            }
        }
        // A status change carries no name; the cards' names are read again.
        if let Ok(Value::Array(listed)) = host.call("sessions.list", json!({})) {
            for r in &mut sessions {
                let named = listed.iter().find(|l| l["id"] == r["id"]);
                if let Some(name) = named.map(|l| s(l, "name")).filter(|n| !n.is_empty()) {
                    r["name"] = json!(name);
                }
            }
        }
        for f in &mut finished {
            let named = sessions.iter().find(|r| r["id"] == f["session"]);
            if let Some(name) = named.map(|r| s(r, "name").to_owned()) {
                f["name"] = json!(name);
            }
        }
        let mut ops: Vec<Op> = gone.into_iter().map(Op::Delete).collect();
        ops.extend(sessions.into_iter().map(Op::Upsert));
        host.publish(SESSIONS, ops);
        let mut ops = vec![Op::Clear];
        ops.extend(finished.into_iter().map(Op::Upsert));
        host.publish(FINISHED, ops);
        Ok(())
    }

    fn reset(host: &mut dyn Host, session: &str) -> Result<Value, Error> {
        let Some(mut r) = host
            .rows(SESSIONS)?
            .into_iter()
            .find(|r| s(r, "id") == session)
        else {
            return Ok(json!({ "toast": "That session is gone" }));
        };
        r["turns"] = json!(0);
        r["last"] = json!("");
        host.publish(SESSIONS, vec![Op::Upsert(r)]);
        Ok(Value::Null)
    }

    fn pref(host: &mut dyn Host, id: &str, value: bool) -> Result<Value, Error> {
        if let Some(mut p) = host.rows(PREFS)?.into_iter().find(|p| s(p, "id") == id) {
            p["on"] = json!(value);
            host.publish(PREFS, vec![Op::Upsert(p)]);
        }
        Ok(Value::Null)
    }

    /// Where the sign-in window opens; Vorn tells the account when it is signed in.
    fn signin() -> Value {
        json!({
            "id": ACCOUNT_ID,
            "name": "Turns account",
            "browser": { "signInUrl": "https://example.com/sign-in", "origins": ["https://example.com"] },
        })
    }

    /// An action, as a client, a palette command or a card's menu calls it.
    fn act(host: &mut dyn Host, method: &str, args: &Value) -> Result<Value, Error> {
        match method {
            "reset" => {
                let id = Some(s(args, "id"))
                    .filter(|i| !i.is_empty())
                    .unwrap_or_else(|| s(args, "sessionId"));
                Turns::reset(host, id)
            }
            "ask-clear" => Ok(json!({ "open": "clear" })),
            "clear" => {
                host.publish(FINISHED, vec![Op::Clear]);
                Ok(Value::Null)
            }
            "plan" => {
                host.publish(PLANS, plans(s(args, "id")));
                Ok(Value::Null)
            }
            "key" => Ok(Value::Null),
            "forget" => Ok(json!({ "toast": "Forgot Turns account" })),
            "signin" => Ok(Turns::signin()),
            "pref" => Turns::pref(host, s(args, "id"), args["value"] == true),
            other => Err(Error::new(
                "unknown-action",
                format!("Turns has no action {other}"),
            )),
        }
    }
}

/// The trees, one function per place.
mod draw {
    use super::*;

    pub(super) fn row_of_signin(id: &str) -> Node {
        line([button(format!("signin:{id}"), "Sign in").primary()]).py(4.0)
    }

    pub fn finished(rows: &[Value]) -> Node {
        col(rows.iter().map(|f| {
            line([
                icon("check").tone(Tone::Good),
                text(s(f, "name")).grow(),
                text(format!("#{} {}", n(f, "turn"), s(f, "at"))).style(TextStyle::Mono),
                // One per finished turn, so a session's turns each name their own button.
                button(format!("turn:{}", s(f, "id")), "Show"),
            ])
        }))
        .gap(4.0)
    }

    pub fn badges(sessions: &[Value]) -> Node {
        per_session(sessions.iter().filter(|r| n(r, "turns") > 0).map(|r| {
            (
                s(r, "id"),
                badge(format!("{} done", n(r, "turns")), Tone::Neutral),
            )
        }))
    }

    pub fn footers(sessions: &[Value]) -> Node {
        per_session(
            sessions
                .iter()
                .filter(|r| !s(r, "last").is_empty())
                .map(|r| {
                    let id = s(r, "id");
                    let said = format!("Turn {} finished at {}", n(r, "turns"), s(r, "last"));
                    let footer = line([
                        icon("clock"),
                        quiet(said).grow(),
                        button(format!("reset:{id}"), "Reset"),
                    ]);
                    (id, footer)
                }),
        )
    }

    pub fn decorations(sessions: &[Value]) -> Node {
        per_session(sessions.iter().filter(|r| n(r, "turns") > 0).map(|r| {
            let status = s(r, "status");
            let mark = line([
                dot(tone(status)).label(status),
                quiet(format!("Turns: {}", n(r, "turns"))),
            ]);
            (s(r, "id"), mark)
        }))
    }

    pub fn pane(sessions: &[Value]) -> Node {
        let entries = sessions.iter().map(|r| {
            entry(s(r, "id"), s(r, "name"))
                .tone(tone(s(r, "status")))
                .aside(n(r, "turns").to_string())
        });
        section(
            "Turns",
            [
                quiet(at_work(sessions)),
                list("sessions", entries).empty("No sessions yet"),
            ],
        )
    }

    pub fn composer(finished: &[Value]) -> Node {
        if finished.is_empty() {
            return col([]);
        }
        line([
            icon("history"),
            quiet(format!("Finished turns: {}", finished.len())).grow(),
            button("ask-clear", "Clear"),
        ])
    }

    pub fn settings(host: &mut dyn Host) -> Result<Node, Error> {
        let prefs = host.rows(PREFS)?;
        let plans = host.rows(PLANS)?;
        let account = host.rows(ACCOUNT)?;
        let finished = host.rows(FINISHED)?;
        let picked = plans
            .iter()
            .find(|p| on(p, "selected"))
            .map_or("", |p| s(p, "id"));
        let accounts = account.iter().map(|a| {
            let id = s(a, "id");
            let mut row = entry(id, s(a, "name"));
            // Under the name rather than beside it, so a phone's narrow page keeps the row's menu in view.
            if !on(a, "signed_in") {
                row = row.child(row_of_signin(id));
            }
            row.more([button(format!("forget:{id}"), "Forget account")
                .danger()
                .confirm(format!("Forget {}?", s(a, "name")))])
        });
        Ok(col([
            section(
                "Counting",
                prefs
                    .iter()
                    .map(|p| toggle(format!("pref:{}", s(p, "id")), s(p, "label"), on(p, "on"))),
            ),
            section(
                "Account",
                [
                    choice(
                        "plan",
                        plans.iter().map(|p| (s(p, "id"), s(p, "name"))),
                        picked,
                    ),
                    field("key", "").placeholder("Paste a key").secret(),
                    list("account", accounts),
                ],
            ),
            section(
                "Finished turns",
                [line([
                    quiet(format!("Kept: {}, the latest first", finished.len())).grow(),
                    button("ask-clear", "Clear").danger(),
                ])],
            ),
        ])
        .gap(24.0))
    }

    pub fn window(sessions: &[Value]) -> Node {
        let head = line([quiet(at_work(sessions)).grow(), button("vorn", "Vorn")]);
        let body = if sessions.is_empty() {
            quiet("No sessions yet")
        } else {
            col(sessions.iter().map(|r| {
                let status = s(r, "status");
                line([
                    dot(tone(status)).label(status),
                    text(s(r, "name")).grow(),
                    text(n(r, "turns").to_string()).style(TextStyle::Mono),
                    button(format!("show:{}", s(r, "id")), "Show"),
                ])
            }))
            .gap(4.0)
        };
        col([head, body]).gap(6.0)
    }

    pub fn dialog() -> Node {
        col([
            text("Clear finished turns?").style(TextStyle::Title),
            quiet("The sidebar's list empties. Each card keeps its count."),
            line([
                col([]).grow(),
                button("cancel", "Cancel"),
                button("clear", "Clear").danger(),
            ]),
        ])
        .gap(6.0)
    }
}

impl Extension for Turns {
    /// What it keeps is there from the first start on; a restart finds it as it was.
    fn init(_: &Init, host: &mut dyn Host) -> Result<Turns, Error> {
        if host.rows(PREFS)?.is_empty() {
            let shells = json!({ "id": "shells", "label": "Count shells too", "on": true });
            host.publish(PREFS, vec![Op::Upsert(shells)]);
        }
        if host.rows(PLANS)?.is_empty() {
            host.publish(PLANS, plans("free"));
        }
        if host.rows(ACCOUNT)?.is_empty() {
            let account = json!({ "id": ACCOUNT_ID, "name": "Turns account", "signed_in": false });
            host.publish(ACCOUNT, vec![Op::Upsert(account)]);
        }
        Ok(Turns)
    }

    fn handle(
        &mut self,
        host: &mut dyn Host,
        method: &str,
        args: Value,
        _: &Ctx,
    ) -> Result<Value, Error> {
        Turns::act(host, method, &args)
    }

    fn on_events(&mut self, host: &mut dyn Host, batch: Vec<Event>) {
        if let Err(e) = Turns::heard(host, &batch) {
            host.log(vorn_extension_sdk::Level::Warn, &e.message);
        }
    }

    fn render(&mut self, host: &mut dyn Host, point: &str, _: Surface) -> Result<Node, Error> {
        Ok(match point {
            "finished" => draw::finished(&host.rows(FINISHED)?),
            "count" => draw::badges(&host.rows(SESSIONS)?),
            "last" => draw::footers(&host.rows(SESSIONS)?),
            "line" => draw::decorations(&host.rows(SESSIONS)?),
            "all" => draw::pane(&host.rows(SESSIONS)?),
            "summary" => draw::composer(&host.rows(FINISHED)?),
            "prefs" => draw::settings(host)?,
            "turns" => draw::window(&host.rows(SESSIONS)?),
            "clear" => draw::dialog(),
            other => {
                return Err(Error::new(
                    "unknown-action",
                    format!("Turns draws no {other}"),
                ))
            }
        })
    }

    fn event(&mut self, host: &mut dyn Host, _: &str, e: tree::Event) -> Result<Value, Error> {
        let (does, id) = e.id.split_once(':').unwrap_or((e.id.as_str(), ""));
        match (does, e.kind) {
            ("show", _) => Ok(json!({ "focus": id })),
            ("turn", _) => {
                let finished = host.rows(FINISHED)?;
                let turn = finished.iter().find(|f| s(f, "id") == id);
                let session = turn.map(|f| s(f, "session")).unwrap_or_default();
                Ok(json!({ "focus": session }))
            }
            ("vorn", _) => Ok(json!({ "show": true })),
            ("reset", _) => Turns::reset(host, id),
            ("ask-clear", _) => Ok(json!({ "open": "clear" })),
            ("cancel", _) => Ok(json!({ "close": true })),
            ("clear", _) => {
                Turns::act(host, "clear", &Value::Null)?;
                Ok(json!({ "close": true }))
            }
            ("pref", EventKind::Toggle) => Turns::pref(host, id, e.value == "true"),
            ("plan", _) => Turns::act(host, "plan", &json!({ "id": e.value })),
            ("key", _) => Ok(Value::Null),
            ("signin", _) => Ok(json!({ "signin": Turns::signin() })),
            ("forget", _) => Turns::act(host, "forget", &Value::Null),
            _ => Err(Error::bad_args(format!("no control is '{}'", e.id))),
        }
    }
}

vorn_extension_sdk::export!(Turns);

#[cfg(test)]
mod tests;

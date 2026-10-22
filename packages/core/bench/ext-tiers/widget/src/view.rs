//! The Widget's two windows as element trees: every agent and what it asks, and the line it folds to.

use vorn_extension_sdk::tree::sdk::*;
use vorn_extension_sdk::tree::{Node, TextStyle, Tone};
use vorn_extension_sdk::Value;

/// What the agents wait on: each request, the rules it offers and its questions' options.
pub(crate) struct Asks {
    pub permissions: Vec<Value>,
    pub suggestions: Vec<Value>,
    pub options: Vec<Value>,
}

fn s<'a>(v: &'a Value, key: &str) -> &'a str {
    v[key].as_str().unwrap_or_default()
}

fn on(v: &Value, key: &str) -> bool {
    v[key].as_bool() == Some(true)
}

/// An agent's status as its dot's tone.
fn tone(status: &str) -> Tone {
    match status {
        "running" => Tone::Good,
        "waiting" => Tone::Warn,
        "error" => Tone::Bad,
        _ => Tone::Muted,
    }
}

fn quiet(t: impl Into<String>) -> Node {
    text(t).style(TextStyle::Secondary)
}

/// A line of controls, spaced as the window's rows are.
fn line(kids: impl IntoIterator<Item = Node>) -> Node {
    row(kids).gap(8.0)
}

fn count(agents: &[Value], status: &str) -> usize {
    agents.iter().filter(|a| s(a, "status") == status).count()
}

/// The folded line: how many agents run, wait and failed, and the way back.
pub(crate) fn compact(agents: &[Value]) -> Node {
    let mut kids = Vec::new();
    for status in ["running", "waiting", "error"] {
        let n = count(agents, status);
        if n > 0 {
            kids.push(line([
                dot(tone(status)).label(status),
                quiet(n.to_string()),
            ]));
        }
    }
    if agents.is_empty() {
        kids.push(quiet("No active agents"));
    }
    kids.push(col([]).grow());
    kids.push(button("expand", "Expand"));
    kids.push(button("vorn", "Vorn"));
    line(kids)
}

/// One request: the tool and agent, what it asks, and its answers.
fn request(p: &Value, asks: &Asks) -> Node {
    let id = s(p, "id");
    let question = on(p, "question");
    let mut kids = vec![line([
        badge(s(p, "tool"), Tone::Neutral),
        quiet(s(p, "agent")).grow(),
    ])];
    if !question {
        kids.push(text(s(p, "description")).style(TextStyle::Mono));
        kids.push(line([
            button(format!("allow:{id}"), "Allow").primary(),
            button(format!("deny:{id}"), "Deny"),
        ]));
    }
    let mine = |rows: &[Value]| -> Vec<Value> {
        rows.iter()
            .filter(|r| s(r, "request") == id)
            .cloned()
            .collect()
    };
    let suggestions = mine(&asks.suggestions);
    if !suggestions.is_empty() {
        kids.push(
            col(suggestions
                .iter()
                .map(|sg| button(format!("suggest:{}", s(sg, "id")), s(sg, "label"))))
            .gap(4.0),
        );
    }
    let options = mine(&asks.options);
    if !options.is_empty() {
        kids.push(
            col(options.iter().map(|o| {
                let mut pick = button(format!("pick:{}", s(o, "id")), s(o, "label"));
                if on(o, "picked") {
                    pick = pick.primary();
                }
                let ask = Some(s(o, "ask")).filter(|a| !a.is_empty()).map(quiet);
                col(ask.into_iter().chain([pick])).gap(6.0)
            }))
            .gap(4.0),
        );
    }
    if question {
        let submit = on(p, "ready").then(|| button(format!("submit:{id}"), "Submit").primary());
        kids.push(line(
            submit
                .into_iter()
                .chain([button(format!("deny:{id}"), "Skip")]),
        ));
    }
    col(kids).gap(6.0)
}

/// The window: the counts, each request waiting, then every agent.
pub(crate) fn widget(agents: &[Value], asks: &Asks) -> Node {
    let running = count(agents, "running");
    let mut page = vec![line([
        quiet(format!("{running}/{} running", agents.len())).grow(),
        button("collapse", "Collapse"),
        button("vorn", "Vorn"),
    ])];
    if !asks.permissions.is_empty() {
        page.push(col(asks.permissions.iter().map(|p| request(p, asks))).gap(4.0));
        page.push(divider());
    }
    page.push(if agents.is_empty() {
        quiet("No active agents")
    } else {
        col(agents.iter().map(|a| {
            line([
                dot(tone(s(a, "status"))).label(s(a, "status")),
                text(s(a, "name")).grow(),
                button(format!("show:{}", s(a, "id")), "Show"),
            ])
        }))
        .gap(4.0)
    });
    col(page).gap(6.0)
}

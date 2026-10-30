//! Recent sessions, the built-in: a toolbar item whose popover lists the
//! agent conversations this machine ran (`sessions.recent`) and starts one
//! again on a click (`sessions.launch` with `resume`), or shows the card
//! already running it. It holds nothing between calls.

use std::sync::Arc;

use futures_util::future::BoxFuture;
use futures_util::FutureExt;
use serde_json::{json, Value};
use vorn_extensions::api::types::Event;
use vorn_extensions::api::ExtError;
use vorn_tree::sdk::{col, entry, list, text};
use vorn_tree::{Answer, EventKind, Rendered, TextStyle};

use super::{unknown, Api, Builtin};

/// The id the built-in goes by.
pub const ID: &str = "vorn.recent";

/// The scope the popover is drawn for when no project is chosen.
pub const EVERYWHERE: &str = "*";

/// The built-in, as an entry of the built-ins' registry.
#[derive(Debug, Default)]
pub struct Recent;

/// One past conversation, as `sessions.recent` lists it.
#[derive(Debug, Clone, PartialEq)]
struct Past {
    id: String,
    agent: String,
    title: String,
    project: String,
    at_ms: f64,
    activity: String,
    running_in: Option<String>,
    resumable: bool,
}

impl Past {
    fn from_value(v: &Value) -> Option<Past> {
        let s = |k: &str| {
            v.get(k)
                .and_then(Value::as_str)
                .unwrap_or_default()
                .to_owned()
        };
        let n = v.get("activityCount").and_then(Value::as_u64).unwrap_or(0);
        let label = Some(s("activityLabel"))
            .filter(|l| !l.is_empty())
            .unwrap_or_else(|| "message".to_owned());
        Some(Past {
            id: v.get("sessionId")?.as_str()?.to_owned(),
            agent: s("agentType"),
            title: Some(s("display"))
                .filter(|d| !d.trim().is_empty())
                .unwrap_or_else(|| "Untitled session".to_owned()),
            project: s("projectPath"),
            at_ms: v.get("timestamp").and_then(Value::as_f64).unwrap_or(0.0),
            activity: format!("{n} {label}{}", if n == 1 { "" } else { "s" }),
            running_in: v
                .get("runningIn")
                .and_then(Value::as_str)
                .map(str::to_owned),
            resumable: v.get("canResumeExact").and_then(Value::as_bool) != Some(false),
        })
    }
}

/// How long ago `then_ms` was at `now_ms`, as the app's lists say it.
fn ago(then_ms: f64, now_ms: f64) -> String {
    let s = ((now_ms - then_ms) / 1000.0).max(0.0) as u64;
    match s {
        0..=59 => "just now".to_owned(),
        60..=3599 => format!("{}m ago", s / 60),
        3600..=86_399 => format!("{}h ago", s / 3600),
        _ => format!("{}d ago", s / 86_400),
    }
}

fn now_ms() -> f64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0.0, |d| d.as_secs_f64() * 1000.0)
}

fn folder(path: &str) -> &str {
    path.trim_end_matches(['/', '\\'])
        .rsplit(['/', '\\'])
        .next()
        .unwrap_or_default()
}

/// The project a tree drawn for `scope` lists, none for every project.
fn project(scope: Option<&str>) -> Option<&str> {
    scope.filter(|s| !s.is_empty() && *s != EVERYWHERE)
}

/// The popover: a heading naming where, and a row per conversation.
fn tree(past: &[Past], scope: Option<&str>, now: f64) -> Rendered {
    let place = project(scope).map_or("All Projects", folder);
    let rows = past.iter().map(|p| {
        let aside = match (&p.running_in, p.resumable) {
            (Some(_), _) => "running",
            (None, true) => "resume",
            (None, false) => "history only",
        };
        entry(format!("r:{}", p.id), p.title.as_str())
            .detail(format!(
                "{} \u{b7} {} \u{b7} {}",
                folder(&p.project),
                ago(p.at_ms, now),
                p.activity
            ))
            .icon(if p.agent == "shell" {
                "terminal"
            } else {
                "bot"
            })
            .aside(aside)
            .tip(match aside {
                "running" => "Shows the card running it",
                "resume" => "Starts this conversation again",
                _ => "Starts it again from its history",
            })
    });
    Rendered::new(
        col([
            text(format!("Recent sessions \u{b7} {place}")).style(TextStyle::Title),
            list("recent", rows).empty("No recent sessions"),
        ])
        .gap(8.0)
        .p(12.0),
    )
}

async fn listed(api: &Api, scope: Option<&str>) -> Result<Vec<Past>, ExtError> {
    let args = json!({ "project": project(scope) });
    let got = api.call("sessions.recent", args).await?;
    Ok(got
        .as_array()
        .into_iter()
        .flatten()
        .filter_map(Past::from_value)
        .collect())
}

/// What a click on row `id` asks for: the card running it, else the conversation started again.
async fn chosen(api: &Api, id: &str, scope: Option<&str>) -> Result<Answer, ExtError> {
    let Some(p) = listed(api, scope).await?.into_iter().find(|p| p.id == id) else {
        return Ok(Answer {
            error: Some("That conversation is no longer listed".to_owned()),
            ..Answer::default()
        });
    };
    let focus = match p.running_in {
        Some(running) => running,
        None => {
            let args = json!({ "project": p.project, "agent": p.agent, "resume": p.id });
            let started = api.call("sessions.launch", args).await?;
            started
                .get("sessionId")
                .and_then(Value::as_str)
                .unwrap_or_default()
                .to_owned()
        }
    };
    Ok(Answer {
        focus: Some(focus).filter(|f| !f.is_empty()),
        close: true,
        ..Answer::default()
    })
}

fn scope_of<'a>(args: &'a Value, at: &str) -> Option<&'a str> {
    args.get(at)?.get("scope")?.as_str()
}

impl Builtin for Recent {
    fn manifest(&self) -> &'static str {
        include_str!("recent.toml")
    }

    fn events(&self) -> &'static [&'static str] {
        &[]
    }

    fn on_by_default(&self) -> bool {
        true
    }

    fn start<'a>(&'a self, _api: &'a Api) -> BoxFuture<'a, ()> {
        async {}.boxed()
    }

    fn handle<'a>(
        &'a self,
        api: &'a Api,
        action: &'a str,
        args: Value,
    ) -> BoxFuture<'a, Result<Value, ExtError>> {
        async move {
            match action {
                vorn_tree::RENDER => {
                    let scope = scope_of(&args, "surface");
                    let past = listed(api, scope).await?;
                    Ok(json!(tree(&past, scope, now_ms())))
                }
                vorn_tree::EVENT => {
                    let event: vorn_tree::Event = args
                        .get("event")
                        .cloned()
                        .and_then(|e| serde_json::from_value(e).ok())
                        .ok_or_else(|| ExtError::new("bad-request", "ui.event takes an event"))?;
                    let picked = matches!(event.kind, EventKind::Click | EventKind::Open);
                    match event.id.strip_prefix("r:").filter(|_| picked) {
                        Some(id) => Ok(json!(chosen(api, id, event.scope.as_deref()).await?)),
                        None => Ok(Value::Null),
                    }
                }
                _ => Err(unknown("Recent sessions", action)),
            }
        }
        .boxed()
    }

    fn on_events<'a>(&'a self, _api: &'a Api, _batch: Vec<Arc<Event>>) -> BoxFuture<'a, ()> {
        async {}.boxed()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn its_manifest_reads_with_a_toolbar_item() {
        let m =
            vorn_extensions::manifest::file::parse_builtin(include_str!("recent.toml")).unwrap();
        let listed = vorn_extensions::points::listed(&m, &Default::default());
        assert!(
            listed
                .iter()
                .any(|p| p["kind"] == "toolbar" && p["icon"] == "clock" && p["tree"] == true),
            "{listed:?}"
        );
    }

    #[test]
    fn the_popover_lists_each_conversation_and_what_a_click_does() {
        let past: Vec<Past> = [
            json!({ "sessionId": "c1", "agentType": "acme", "display": "Fix the flaky test",
                    "projectPath": "/dev/demo", "timestamp": 0.0, "activityCount": 3,
                    "runningIn": "t1" }),
            json!({ "sessionId": "c2", "agentType": "acme", "projectPath": "/dev/demo",
                    "timestamp": 0.0, "activityCount": 1, "canResumeExact": false }),
            json!({ "agentType": "acme" }),
        ]
        .iter()
        .filter_map(Past::from_value)
        .collect();
        assert_eq!(past.len(), 2, "an entry needs an id");
        let r = tree(&past, Some("/dev/demo"), 120_000.0);
        let text = serde_json::to_string(&r).unwrap();
        for want in [
            "Recent sessions \u{b7} demo",
            "Fix the flaky test",
            "Untitled session",
            "demo \u{b7} 2m ago \u{b7} 3 messages",
            "1 message\"",
            "running",
            "history only",
            "r:c1",
        ] {
            assert!(text.contains(want), "{want} in {text}");
        }
        vorn_tree::read(&json!(r)).expect("a tree the client draws");
        let everywhere = serde_json::to_string(&tree(&[], Some(EVERYWHERE), 0.0)).unwrap();
        assert!(everywhere.contains("All Projects") && everywhere.contains("No recent sessions"));
    }
}

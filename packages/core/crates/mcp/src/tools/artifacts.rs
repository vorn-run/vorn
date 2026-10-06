//! `tools/artifacts.ts`: publishing deliverables into the session's pane, and
//! reading what the person said about them.

use serde_json::Value;

use super::browser::{page_result, with_session};
use super::{items, object, text, Args, Cx, Outcome};
use crate::json;
use crate::rpc::Rpc;

const NO_SESSION: &str = "Error: no Vorn session context (VORN_SESSION_ID is unset). Browser tools only work from a terminal session started by the Vorn app, and only when that session has a browser pane open.";

fn session<'a, R>(cx: &Cx<'a, R>) -> Option<&'a str> {
    cx.caller.session.as_deref()
}

/// `a.b.c`, throwing where JavaScript would.
fn path<'a>(value: &'a Value, keys: &[&str]) -> Result<Option<&'a Value>, String> {
    let mut at = Some(value);
    for key in keys {
        at = json::prop(at, key)?;
    }
    Ok(at)
}

pub async fn publish_artifact<R: Rpc>(cx: &Cx<'_, R>, args: &Args) -> Outcome {
    with_session(session(cx), NO_SESSION, |id| async move {
        let mut params = serde_json::Map::new();
        params.insert("sessionId".into(), Value::from(id));
        for (k, v) in args.map() {
            params.insert(k.clone(), v.clone());
        }
        let result = cx
            .call("artifact:publish", Some(Value::Object(params)))
            .await?;
        let kind = path(&result, &["artifact", "kind"])?;
        let title = path(&result, &["artifact", "title"])?;
        let version = path(&result, &["version", "version"])?;
        let artifact_id = path(&result, &["artifact", "id"])?;
        let mut lines = vec![format!(
            "Published {} \"{}\" v{} (artifactId {}).",
            json::display(kind),
            json::display(title),
            json::display(version),
            json::display(artifact_id)
        )];
        lines.push(if json::truthy(result.get("opened")) {
            "It is open in your browser pane.".to_owned()
        } else {
            format!(
                "Not opened in a pane. Served at {}",
                json::display(result.get("url"))
            )
        });
        if json::to_number(result.get("answered")) > 0.0 {
            lines.push(format!(
                "This version answers {} comments.",
                json::display(result.get("answered"))
            ));
        }
        Ok(text(lines.join("\n")))
    })
    .await
}

pub async fn list_artifacts<R: Rpc>(cx: &Cx<'_, R>, args: &Args) -> Outcome {
    with_session(session(cx), NO_SESSION, |id| async move {
        let artifacts = cx
            .call(
                "artifact:list",
                Some(object([
                    ("sessionId", Some(Value::from(id))),
                    ("limit", args.get("limit").cloned()),
                ])),
            )
            .await?;
        let list = items(Some(&artifacts));
        if list.is_empty() {
            return Ok(text("No artifacts yet."));
        }
        let lines = list
            .iter()
            .map(|a| {
                Ok(format!(
                    "{}  {}  v{}  \"{}\"  updated {}",
                    json::display(json::prop(Some(a), "id")?),
                    json::display(a.get("kind")),
                    json::display(a.get("latestVersion")),
                    json::display(a.get("title")),
                    json::display(a.get("updatedAt"))
                ))
            })
            .collect::<Result<Vec<_>, String>>()?;
        Ok(text(lines.join("\n")))
    })
    .await
}

pub async fn read_artifact_comments<R: Rpc>(cx: &Cx<'_, R>, args: &Args) -> Outcome {
    with_session(session(cx), NO_SESSION, |id| async move {
        let comments = cx
            .call(
                "artifact:readComments",
                Some(object([
                    ("sessionId", Some(Value::from(id))),
                    ("artifactId", args.get("artifactId").cloned()),
                    ("version", args.get("version").cloned()),
                ])),
            )
            .await?;
        let shown = items(Some(&comments))
            .iter()
            .map(|c| {
                Ok(object([
                    ("version", json::prop(Some(c), "version")?.cloned()),
                    ("state", c.get("state").cloned()),
                    ("anchor", c.get("anchor").cloned()),
                    ("comment", c.get("body").cloned()),
                ]))
            })
            .collect::<Result<Vec<_>, String>>()?;
        Ok(page_result(
            &Value::Array(shown),
            "ARTIFACT COMMENTS ON WEB PAGE CONTENT",
        ))
    })
    .await
}

pub async fn read_artifact<R: Rpc>(cx: &Cx<'_, R>, args: &Args) -> Outcome {
    with_session(session(cx), NO_SESSION, |id| async move {
        let found = cx
            .call(
                "artifact:readSource",
                Some(object([
                    ("sessionId", Some(Value::from(id))),
                    ("artifactId", args.get("artifactId").cloned()),
                    ("version", args.get("version").cloned()),
                ])),
            )
            .await?;
        if !json::truthy(Some(&found)) {
            return Ok(text(format!(
                "No such version of artifact {}.",
                json::display(args.get("artifactId"))
            )));
        }
        let version = path(&found, &["version", "version"])?;
        let author = path(&found, &["version", "author"])?;
        Ok(page_result(
            &object([
                ("version", version.cloned()),
                (
                    "author",
                    Some(Value::from(
                        if author.and_then(Value::as_str) == Some("user") {
                            "the person"
                        } else {
                            "an agent"
                        },
                    )),
                ),
                ("source", found.get("body").cloned()),
            ]),
            "WEB PAGE CONTENT: ARTIFACT SOURCE",
        ))
    })
    .await
}

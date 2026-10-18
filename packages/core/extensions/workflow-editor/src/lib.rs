//! The Workflow editor: the Workflows view (a tree whose `graph` is the
//! client's canvas), Run workflow in the grid's, a card's and a terminal's
//! menus (`[[contributes.menus]]` listing the workflows each offers), and
//! the approval gates waiting for a person (`[[contributes.attention]]`),
//! which the waiting pill counts and the approve and deny keys answer
//! beside agents asking.
//!
//! Vorn ships it as a Wasm component. The rows it publishes follow the
//! workflows (`workflow.changed`) and their runs' gates (`workflow.gates`);
//! off, it publishes nothing and its view, menu items and gates leave every
//! client, and the workflows still run.

use vorn_extension_sdk::tree::{self, sdk, Surface};
use vorn_extension_sdk::{arg, json, Ctx, Error, Event, Extension, Host, Init, Op, Value};

/// The workflows the grid's menu offers: every one that is on and asks for no session.
pub const MANUAL: &str = "manual";
/// The workflows a card's and a terminal's menus offer: those that run on a session.
pub const CONTEXTUAL: &str = "contextual";
/// The gates waiting for a person.
pub const GATES: &str = "gates";

/// Whether a workflow's manual trigger asks for a session.
pub fn contextual(nodes: &Value) -> bool {
    nodes.as_array().into_iter().flatten().any(|n| {
        let cfg = n.get("config");
        n.get("type").and_then(Value::as_str) == Some("trigger")
            && cfg
                .and_then(|c| c.get("triggerType"))
                .and_then(Value::as_str)
                == Some("manual")
            && cfg
                .and_then(|c| c.get("contextual"))
                .and_then(Value::as_bool)
                == Some(true)
    })
}

/// The extension; everything it shows lives in its rows.
#[derive(Debug, Default)]
pub struct Editor;

impl Editor {
    /// The menus' rows again, whole, as the workflows are now.
    fn menus(host: &mut dyn Host) -> Result<(), Error> {
        let listed = host.call("workflow.list", Value::Null)?;
        let mut manual = vec![Op::Clear];
        let mut on_session = vec![Op::Clear];
        for wf in listed
            .as_array()
            .into_iter()
            .flatten()
            .filter(|w| w.get("enabled") == Some(&Value::Bool(true)))
        {
            let row = json!({
                "id": wf.get("id"),
                "title": wf.get("name"),
                "workspace": wf.get("workspaceId").cloned().unwrap_or(json!("personal")),
            });
            if contextual(wf.get("nodes").unwrap_or(&Value::Null)) {
                on_session.push(Op::Upsert(row));
            } else {
                manual.push(Op::Upsert(row));
            }
        }
        host.publish(MANUAL, manual);
        host.publish(CONTEXTUAL, on_session);
        Ok(())
    }

    /// Every gate asking now, published whole.
    fn all_gates(host: &mut dyn Host) -> Result<(), Error> {
        let rows = host.call("workflow.gates", Value::Null)?;
        let mut ops = vec![Op::Clear];
        ops.extend(
            rows.as_array()
                .into_iter()
                .flatten()
                .cloned()
                .map(Op::Upsert),
        );
        host.publish(GATES, ops);
        Ok(())
    }

    /// Run `run`'s gates as they ask now: those that no longer ask go.
    fn gates(host: &mut dyn Host, run: &str, gates: &[Value]) {
        let shown = host.rows(GATES).unwrap_or_default();
        let ids: Vec<&str> = gates
            .iter()
            .filter_map(|g| g.get("id").and_then(Value::as_str))
            .collect();
        let mut ops: Vec<Op> = shown
            .iter()
            .filter(|r| r.get("run").and_then(Value::as_str) == Some(run))
            .filter_map(|r| r.get("id").and_then(Value::as_str))
            .filter(|id| !ids.contains(id))
            .map(|id| Op::Delete(id.to_owned()))
            .collect();
        ops.extend(gates.iter().cloned().map(Op::Upsert));
        if !ops.is_empty() {
            host.publish(GATES, ops);
        }
    }

    /// A gate row's run and step.
    fn gate_of(args: &Value, action: &str) -> Result<(String, String), Error> {
        let id = arg(args, "id", action)?;
        let (run, node) = id
            .split_once('|')
            .ok_or_else(|| Error::bad_args(format!("{action} takes a gate's id")))?;
        Ok((run.to_owned(), node.to_owned()))
    }
}

impl Extension for Editor {
    fn init(_: &Init, _: &mut dyn Host) -> Result<Self, Error> {
        Ok(Editor)
    }

    fn handle(
        &mut self,
        host: &mut dyn Host,
        method: &str,
        args: Value,
        _: &Ctx,
    ) -> Result<Value, Error> {
        match method {
            // As it is switched on: the menus and the gates as they are now.
            "refresh" => {
                Editor::menus(host)?;
                Editor::all_gates(host)?;
                Ok(Value::Null)
            }
            "run" => {
                let id = arg(&args, "id", method)?;
                let inputs = args.get("inputs").cloned().unwrap_or(Value::Null);
                host.call(
                    "workflow.run",
                    json!({ "workflowId": id, "inputs": inputs }),
                )
            }
            "approve" | "reject" => {
                let (run, node) = Editor::gate_of(&args, method)?;
                host.call(
                    "workflow.resolveGate",
                    json!({ "runId": run, "nodeId": node, "decision": method }),
                )
            }
            // Opening a card, dropping it and drawing an edge are the client's edits.
            "open" | "move" | "connect" => Ok(Value::Null),
            other => Err(Error::new(
                "unknown-action",
                format!("Workflow editor has no action {other}"),
            )),
        }
    }

    fn on_events(&mut self, host: &mut dyn Host, batch: Vec<Event>) {
        // A batch of changes to the workflows reads them once.
        if batch.iter().any(|e| e.name == "workflow.changed") {
            if let Err(e) = Editor::menus(host) {
                host.log(
                    vorn_extension_sdk::Level::Warn,
                    &format!("could not list the workflows for the menus: {e}"),
                );
            }
        }
        for e in batch.iter().filter(|e| e.name == "workflow.gates") {
            let run = e
                .data
                .get("runId")
                .and_then(Value::as_str)
                .unwrap_or_default();
            let gates = e.data["gates"].as_array().cloned().unwrap_or_default();
            Editor::gates(host, run, &gates);
        }
    }

    fn render(&mut self, _: &mut dyn Host, point: &str, _: Surface) -> Result<tree::Node, Error> {
        match point {
            "workflows" => Ok(sdk::graph()),
            other => Err(Error::new(
                "unknown-action",
                format!("nothing draws {other}"),
            )),
        }
    }
}

vorn_extension_sdk::export!(Editor);

#[cfg(test)]
mod tests {
    use vorn_extension_sdk::dispatch;
    use vorn_extension_sdk::testing::FakeHost;

    use super::*;

    fn gate(run: &str, node: &str) -> Value {
        json!({ "id": format!("{run}|{node}"), "run": run, "node": node, "title": "Approval" })
    }

    fn host() -> FakeHost {
        FakeHost::default()
            .answer("workflow.list", |_| {
                Ok(json!([
                    { "id": "a", "name": "A", "enabled": true, "nodes": [] },
                    { "id": "b", "name": "B", "enabled": true, "workspaceId": "w",
                      "nodes": [{ "type": "trigger", "config": { "triggerType": "manual", "contextual": true } }] },
                    { "id": "c", "name": "C", "enabled": false, "nodes": [] },
                ]))
            })
            .answer("workflow.gates", |_| Ok(json!([gate("r1", "g")])))
            .answer("workflow.run", |a| Ok(json!({ "runId": "r9", "asked": a })))
            .answer("workflow.resolveGate", |a| Ok(a.clone()))
    }

    fn call(h: &mut FakeHost, method: &str, args: Value) -> Result<Value, Error> {
        dispatch(&mut Editor, h, method, args, &Ctx::default())
    }

    #[test]
    fn lists_the_workflows_each_menu_offers_and_the_gates_asking() {
        let mut h = host();
        call(&mut h, "refresh", Value::Null).unwrap();
        let manual = h.rows(MANUAL).unwrap();
        assert_eq!(
            manual,
            [json!({ "id": "a", "title": "A", "workspace": "personal" })]
        );
        assert_eq!(h.rows(CONTEXTUAL).unwrap()[0]["workspace"], "w");
        assert_eq!(h.rows(GATES).unwrap(), [gate("r1", "g")]);
    }

    #[test]
    fn follows_a_runs_gates_and_answers_them() {
        let mut h = host();
        call(&mut h, "refresh", Value::Null).unwrap();
        let mut e = Editor;
        let told = |run: &str, gates: Value| Event {
            name: "workflow.gates".into(),
            data: json!({ "runId": run, "gates": gates }),
        };
        e.on_events(&mut h, vec![told("r2", json!([gate("r2", "x")]))]);
        assert_eq!(h.rows(GATES).unwrap().len(), 2);
        e.on_events(&mut h, vec![told("r1", json!([]))]);
        assert_eq!(h.rows(GATES).unwrap(), [gate("r2", "x")]);
        let answered = call(&mut h, "approve", json!({ "id": "r2|x" })).unwrap();
        assert_eq!(
            answered,
            json!({ "runId": "r2", "nodeId": "x", "decision": "approve" })
        );
        assert!(call(&mut h, "reject", json!({ "id": "nogate" })).is_err());
        let ran = call(&mut h, "run", json!({ "id": "a", "inputs": { "k": 1 } })).unwrap();
        assert_eq!(
            ran["asked"],
            json!({ "workflowId": "a", "inputs": { "k": 1 } })
        );
        assert_eq!(call(&mut h, "move", json!({})).unwrap(), Value::Null);
    }

    #[test]
    fn the_view_is_the_clients_canvas() {
        let mut h = host();
        let drawn = call(&mut h, tree::RENDER, json!({ "point": "workflows" })).unwrap();
        assert_eq!(tree::read(&drawn).unwrap().tree.kind, tree::Kind::Graph);
    }
}

//! The script step (`script`): runs a bash, python, node or powershell
//! script through the core, streaming what it prints into the step.

use serde_json::Value;
use tokio::sync::broadcast::error::RecvError;
use vorn_work::graph::{append_bounded_log, resolve_script_config};
use vorn_work::js;
use vorn_work::model::{StateExt, Status};
use vorn_workflow::{BoxFuture, Host, Note, Step, StepCx};

use crate::result_error;

/// The script step.
#[derive(Debug, Default)]
pub struct Script;

impl<H: Host> Step<H> for Script {
    fn run<'a>(&'a self, cx: StepCx<'a, H>) -> BoxFuture<'a, Result<(), String>> {
        Box::pin(async move {
            self.script(&cx).await;
            Ok(())
        })
    }
}

impl Script {
    async fn script<H: Host>(&self, cx: &StepCx<'_, H>) {
        let (node, context, outputs) = (cx.node, cx.context, cx.outputs);
        let script_run = uuid::Uuid::new_v4().to_string();
        let mut config = resolve_script_config(&node.config, context, Some(outputs));
        if let Some(map) = config.as_object_mut() {
            map.insert("runId".into(), Value::String(script_run.clone()));
        }
        let mut notes = cx.host().notes();
        let call = cx.host().call("script:execute", config);
        tokio::pin!(call);
        let mut streamed = String::new();
        let result = loop {
            tokio::select! {
                result = &mut call => break result,
                note = notes.recv() => match note {
                    Ok(Note::ScriptData { run_id, data }) if run_id == script_run => {
                        append_bounded_log(&mut streamed, &data);
                        cx.update(|s| s.logs = Some(streamed.clone()));
                        cx.publish();
                    }
                    Err(RecvError::Closed) => break (&mut call).await,
                    _ => {}
                },
            }
        };
        // Output that arrived with the answer.
        while let Ok(note) = notes.try_recv() {
            if let Note::ScriptData { run_id, data } = note {
                if run_id == script_run {
                    append_bounded_log(&mut streamed, &data);
                }
            }
        }
        match result {
            Ok(result) => {
                let success = result
                    .get("success")
                    .and_then(Value::as_bool)
                    .unwrap_or(false);
                let output = result
                    .get("output")
                    .filter(|o| !o.is_null())
                    .map(js::to_string);
                let error = result_error(&result);
                let trailer = match &error {
                    Some(e) if streamed.is_empty() => format!("\nError: {e}"),
                    _ => String::new(),
                };
                let logs = if streamed.is_empty() {
                    output.clone().unwrap_or_else(|| "undefined".into())
                } else {
                    streamed
                };
                cx.finish(|s| {
                    s.set_status(if success {
                        Status::Success
                    } else {
                        Status::Error
                    });
                    s.output = output;
                    s.logs = Some(logs + &trailer);
                    s.error = error;
                });
            }
            Err(message) => cx.finish(|s| {
                s.set_status(Status::Error);
                s.error = Some(message);
            }),
        }
    }
}

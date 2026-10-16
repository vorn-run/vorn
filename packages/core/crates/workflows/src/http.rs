//! The HTTP request step (`httpRequest`): one request through the core,
//! signed by a saved HTTP profile when the step names one.

use serde_json::{Map, Value};
use vorn_work::js;
use vorn_work::model::{StateExt, Status};
use vorn_work::template::resolve;
use vorn_workflow::{BoxFuture, Host, Step, StepCx};

use crate::{plain_object, put, result_error};

/// The HTTP request step.
#[derive(Debug, Default)]
pub struct HttpRequest;

impl<H: Host> Step<H> for HttpRequest {
    fn run<'a>(&'a self, cx: StepCx<'a, H>) -> BoxFuture<'a, Result<(), String>> {
        Box::pin(async move {
            self.http_request(&cx).await;
            Ok(())
        })
    }
}

impl HttpRequest {
    async fn http_request<H: Host>(&self, cx: &StepCx<'_, H>) {
        let (node, context, outputs) = (cx.node, cx.context, cx.outputs);
        let config = &node.config;
        let mut headers = Map::new();
        for (name, value) in config
            .get("headers")
            .and_then(Value::as_object)
            .into_iter()
            .flatten()
        {
            let name = name.trim();
            if name.is_empty() {
                continue;
            }
            let text = value
                .as_str()
                .map_or_else(|| js::to_string(value), str::to_owned);
            headers.insert(
                name.to_owned(),
                Value::String(resolve(&text, context, Some(outputs))),
            );
        }
        let mut params = Map::new();
        put(
            &mut params,
            "profileConnectionId",
            config
                .get("profileConnectionId")
                .filter(|v| vorn_work::is_truthy(Some(v)))
                .cloned(),
        );
        put(&mut params, "method", config.get("method").cloned());
        params.insert(
            "url".into(),
            Value::String(resolve(
                node.text("url").unwrap_or(""),
                context,
                Some(outputs),
            )),
        );
        params.insert("headers".into(), Value::Object(headers));
        put(
            &mut params,
            "body",
            node.text("body")
                .filter(|b| !b.is_empty())
                .map(|b| Value::String(resolve(b, context, Some(outputs)))),
        );
        match cx.host().call("http:request", Value::Object(params)).await {
            Ok(result) => {
                let success = result
                    .get("success")
                    .and_then(Value::as_bool)
                    .unwrap_or(false);
                let status = result
                    .get("output")
                    .and_then(|o| o.get("status"))
                    .map_or_else(|| "undefined".to_owned(), js::to_string);
                let structured = plain_object(&result);
                let error = result_error(&result);
                let logs = js::stringify_pretty(&result);
                cx.finish(|s| {
                    s.set_status(if success {
                        Status::Success
                    } else {
                        Status::Error
                    });
                    s.output = Some(if success {
                        format!("HTTP {status}")
                    } else {
                        "Request failed".into()
                    });
                    s.logs = Some(logs);
                    if structured.is_some() {
                        s.structured_output = structured;
                    }
                    if error.is_some() {
                        s.error = error;
                    }
                });
            }
            Err(message) => cx.finish(|s| {
                s.set_status(Status::Error);
                s.error = Some(message);
            }),
        }
    }
}

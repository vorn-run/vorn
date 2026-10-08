//! What the TypeScript connector code answered for seeded inputs, before it
//! was removed (`tests/fixtures/js-reference/connector-functions.json`),
//! replayed against the Rust that replaced it.

use std::collections::HashMap;

use serde_json::{json, Map, Value};
use vorn_connectors::{auth, catalog, connections as conns, http};

fn recorded() -> Map<String, Value> {
    let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../../../tests/fixtures/js-reference/connector-functions.json");
    let text = std::fs::read_to_string(path).expect("the recorded cases");
    let all: Value = serde_json::from_str(&text).expect("JSON");
    all["cases"].as_object().expect("cases").clone()
}

fn each(name: &str, check: impl Fn(&Value, &Value)) {
    let cases = recorded();
    let list = cases[name].as_array().unwrap_or_else(|| panic!("no cases for {name}"));
    assert!(!list.is_empty());
    for case in list {
        check(&case["input"], &case["output"]);
    }
}

#[test]
fn reads_catalog_documents_as_the_server_did() {
    each("parseCatalog", |doc, want| {
        let got = catalog::parse_catalog(doc).map_or(Value::Null, Value::Array);
        assert_eq!(&got, want, "{doc}");
    });
    each("parseTemplates", |doc, want| {
        assert_eq!(&Value::Array(catalog::parse_templates(doc)), want, "{doc}");
    });
    each("parseMcpServers", |doc, want| {
        assert_eq!(&Value::Array(catalog::parse_mcp_servers(doc)), want, "{doc}");
    });
}

#[test]
fn describes_keys_as_the_server_did() {
    each("maskSecret", |v, want| {
        assert_eq!(json!(conns::mask_secret(v.as_str().unwrap())), *want, "{v}");
    });
    each("envNamesOf", |v, want| {
        assert_eq!(json!(conns::env_names_of(v.as_str())), *want, "{v}");
    });
    each("usageCounts", |workflows, want| {
        let counts: HashMap<String, u64> = conns::usage_counts(workflows.as_array().unwrap());
        let want: HashMap<String, u64> = serde_json::from_value(want.clone()).unwrap();
        assert_eq!(counts, want, "{workflows}");
    });
    each("listKeys", |input, want| {
        let connections = input["connections"].as_array().unwrap();
        let workflows = input["workflows"].as_array().unwrap();
        let auth_of = |id: &str| {
            let key = if id == "http" { "secret" } else { "secretEnv" };
            vec![
                json!({ "key": key, "label": "L", "type": "password" }),
                json!({ "key": "other", "label": "O", "type": "text" }),
            ]
        };
        let secrets = |id: &str| -> Option<HashMap<String, String>> {
            input["secrets"].get(id).and_then(|s| serde_json::from_value(s.clone()).ok())
        };
        let got = conns::list_keys(connections, auth_of, workflows, secrets);
        assert_eq!(&Value::Array(got), want, "{input}");
    });
}

#[test]
fn reads_sign_ins_as_the_server_did() {
    each("identityFrom", |output, want| {
        let got = auth::identity_from(output.as_str().unwrap());
        assert_eq!(json!(got), *want, "{output}");
    });
    each("signInCommand", |a, want| {
        assert_eq!(json!(auth::sign_in_command(a)), *want);
    });
    each("borrowableNames", |source, want| {
        let source = auth::Source {
            auth: Some(source["auth"].clone()),
            declared: serde_json::from_value(source["declared"].clone()).unwrap(),
            trusted: source["trusted"] == true,
        };
        assert_eq!(json!(auth::borrowable_names(&source)), *want, "{source:?}");
    });
}

#[test]
fn seeds_workflows_and_draws_actions_as_the_server_did() {
    each("cronEveryMinutes", |m, want| {
        assert_eq!(json!(conns::cron_every_minutes(m.as_f64().unwrap())), *want, "{m}");
    });
    each("seededWorkflow", |input, want| {
        let got = conns::seeded_workflow(&input["conn"], &input["manifest"], &input["event"]);
        assert_eq!(&got, want, "{input}");
    });
    each("sdkActionDef", |action, want| {
        assert_eq!(&conns::sdk_action_def(action), want, "{action}");
    });
    each("scriptEnv", |fields, want| {
        let pairs: Vec<(String, String)> = fields
            .as_object()
            .unwrap()
            .iter()
            .map(|(k, v)| (k.clone(), v.as_str().unwrap().to_owned()))
            .collect();
        let got: Map<String, Value> = conns::script_env(&pairs)
            .into_iter()
            .map(|(k, v)| (k, json!(v)))
            .collect();
        assert_eq!(&Value::Object(got), want, "{fields}");
    });
}

#[test]
fn signs_requests_as_the_server_did() {
    each("httpRequest", |input, want| {
        let p = &input["profile"];
        let s = |v: &Value, k: &str| v.get(k).and_then(Value::as_str).unwrap_or("").to_owned();
        let profile = http::Profile {
            base_url: s(p, "baseUrl"),
            auth_header: s(p, "authHeader"),
            auth_query: s(p, "authQuery"),
            auth_body: s(p, "authBody"),
            secret: s(p, "secret"),
        };
        let spec = &input["spec"];
        let headers: Vec<(String, String)> = spec
            .get("headers")
            .and_then(Value::as_object)
            .into_iter()
            .flatten()
            .map(|(k, v)| (k.clone(), v.as_str().unwrap().to_owned()))
            .collect();
        let body = spec.get("body").and_then(Value::as_str).map(str::to_owned);
        match http::prepare(&profile, &s(spec, "method"), &s(spec, "url"), headers, body) {
            Ok(request) => {
                let sent = &want["sent"];
                assert_eq!(json!(request.url), sent["url"], "{input}");
                assert_eq!(json!(request.method), sent["method"], "{input}");
                let headers: Map<String, Value> =
                    request.headers.iter().map(|(k, v)| (k.clone(), json!(v))).collect();
                assert_eq!(Value::Object(headers), sent["headers"], "{input}");
                assert_eq!(json!(request.body), sent["body"], "{input}");
            }
            Err(refused) => {
                assert_eq!(refused, want["result"], "{input}");
                assert!(want["sent"].is_null());
            }
        }
    });
}

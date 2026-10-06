//! The schemas against what the SDK itself made of the same arguments.
//!
//! `cases.json` holds, for each case, the arguments and either the arguments
//! as the SDK parsed them or the message it refused them with, both taken
//! from `@modelcontextprotocol/sdk` validating against the TypeScript tools.

use serde_json::Value;

use super::*;

#[test]
fn every_tool_has_a_schema_entry_and_every_ref_resolves() {
    let listed = crate::tools_list().as_array().unwrap();
    for tool in listed {
        let name = tool["name"].as_str().unwrap();
        assert!(
            registry().tool(name).is_some(),
            "{name} has no schema entry"
        );
    }
    assert_eq!(registry().tools.len(), listed.len());
}

#[test]
fn checks_arguments_as_the_sdk_does() {
    let cases: Vec<Value> = serde_json::from_str(include_str!("cases.json")).unwrap();
    for case in cases {
        let tool = case["tool"].as_str().unwrap();
        let args = match &case["args"] {
            Value::Null => None,
            other => Some(json::js_order(other.clone())),
        };
        let schema = registry().tool(tool).unwrap().unwrap();
        match registry().parse(schema, args.as_ref()) {
            Ok(parsed) => {
                let want = case.get("data").unwrap_or_else(|| {
                    panic!(
                        "{tool} {args:?} parsed, the SDK refused it: {}",
                        case["error"]
                    )
                });
                // Compared as text, so key order counts.
                assert_eq!(
                    json::stringify(&parsed.unwrap_or(Value::Null)),
                    json::stringify(want),
                    "{tool} {args:?}"
                );
            }
            Err(issues) => {
                let want = case
                    .get("error")
                    .and_then(Value::as_str)
                    .unwrap_or_else(|| {
                        panic!(
                            "{tool} {args:?} refused, the SDK parsed it: {}",
                            error_message(&issues)
                        )
                    });
                assert_eq!(error_message(&issues), want, "{tool} {args:?}");
            }
        }
    }
}

#[test]
fn a_registry_refuses_what_it_cannot_read() {
    let unknown =
        serde_json::json!({ "defs": {}, "tools": { "x": { "t": "ref", "name": "missing" } } });
    assert_eq!(
        Registry::from_json(&unknown).unwrap_err(),
        "no definition for missing"
    );
    let refinement = serde_json::json!({ "defs": {}, "tools": { "x": { "t": "string", "checks": [{ "k": "custom", "name": "nope" }] } } });
    assert_eq!(
        Registry::from_json(&refinement).unwrap_err(),
        "a refinement this build does not know: nope"
    );
}

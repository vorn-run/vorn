//! The extension API, generated from `wit/extension.wit` by the build
//! script: the WIT's types ([`types`]), each function's params ([`params`]),
//! every function as a [`Method`], and the stdio mapping ([`STDIO`]).
//!
//! What is written by hand here is what the mapping rules need at run time:
//! the error an `ext-error` travels as, and a reader of the mapping that can
//! make a sample value of any WIT type and check a value against one, which
//! the conformance suite drives every method with.

use std::collections::HashMap;
use std::fmt;
use std::sync::LazyLock;

use serde_json::{json, Map, Value};

include!(concat!(env!("OUT_DIR"), "/api.rs"));

/// Which end of the pipe answers a method.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Side {
    /// vornd answers: what the world imports.
    Host,
    /// The extension answers: what the world exports.
    Guest,
}

/// The JSON-RPC error code every `ext-error` travels with; its own code is in `data.code`.
pub const EXT_ERROR: i64 = -32000;

impl types::ExtError {
    pub fn new(code: &str, message: impl Into<String>) -> types::ExtError {
        types::ExtError {
            code: code.to_owned(),
            message: message.into(),
        }
    }

    /// The JSON-RPC error it travels as.
    pub fn to_rpc(&self) -> (i64, String, Value) {
        (EXT_ERROR, self.message.clone(), json!({ "code": self.code }))
    }
}

impl fmt::Display for types::ExtError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.message)
    }
}

impl std::error::Error for types::ExtError {}

pub use types::ExtError;

/// One type of the mapping, as `stdio.json` describes it.
#[derive(Debug, Clone, PartialEq)]
enum Def {
    Alias(String),
    Record(Vec<(String, String)>),
    Variant(Vec<(String, Option<String>)>),
    Enum(Vec<String>),
}

/// One method of the mapping.
#[derive(Debug, Clone, PartialEq)]
pub struct MethodDoc {
    pub method: Method,
    pub params: Vec<(String, String)>,
    /// The type it answers with, `_` for nothing; `None` for a notification.
    pub result: Option<String>,
}

/// The stdio mapping, read once from [`STDIO`].
#[derive(Debug)]
pub struct Mapping {
    types: HashMap<String, Def>,
    pub methods: Vec<MethodDoc>,
}

fn strings(v: &Value) -> Vec<String> {
    v.as_array()
        .into_iter()
        .flatten()
        .filter_map(Value::as_str)
        .map(str::to_owned)
        .collect()
}

fn pairs(v: &Value) -> Vec<(String, Option<String>)> {
    v.as_array()
        .into_iter()
        .flatten()
        .filter_map(|p| {
            let name = p.get(0)?.as_str()?.to_owned();
            Some((name, p.get(1).and_then(Value::as_str).map(str::to_owned)))
        })
        .collect()
}

/// The mapping this build speaks.
pub fn mapping() -> &'static Mapping {
    static MAPPING: LazyLock<Mapping> = LazyLock::new(|| {
        let doc: Value = serde_json::from_str(STDIO).expect("the generated mapping is JSON");
        let mut types = HashMap::new();
        for (name, def) in doc["types"].as_object().into_iter().flatten() {
            let def = if let Some(a) = def.get("alias").and_then(Value::as_str) {
                Def::Alias(a.to_owned())
            } else if let Some(r) = def.get("record") {
                Def::Record(
                    pairs(r)
                        .into_iter()
                        .map(|(f, t)| (f, t.unwrap_or_default()))
                        .collect(),
                )
            } else if let Some(v) = def.get("variant") {
                Def::Variant(pairs(v))
            } else {
                Def::Enum(strings(&def["enum"]))
            };
            types.insert(name.clone(), def);
        }
        let methods = doc["methods"]
            .as_array()
            .into_iter()
            .flatten()
            .filter_map(|m| {
                Some(MethodDoc {
                    method: Method::from_name(m["method"].as_str()?)?,
                    params: pairs(&m["params"])
                        .into_iter()
                        .map(|(p, t)| (p, t.unwrap_or_default()))
                        .collect(),
                    result: m["result"].as_str().map(str::to_owned),
                })
            })
            .collect();
        Mapping { types, methods }
    });
    &MAPPING
}

/// `option<list<header>>` as its head and what it wraps.
fn wrapped<'a>(ty: &'a str, head: &str) -> Option<&'a str> {
    ty.strip_prefix(head)?.strip_prefix('<')?.strip_suffix('>')
}

impl Mapping {
    /// The documentation of `method`.
    pub fn method(&self, method: Method) -> Option<&MethodDoc> {
        self.methods.iter().find(|m| m.method == method)
    }

    /// A value of `ty` with every field filled, the first case of every choice.
    pub fn sample(&self, ty: &str) -> Value {
        if let Some(inner) = wrapped(ty, "option") {
            return self.sample(inner);
        }
        if let Some(inner) = wrapped(ty, "list") {
            return json!([self.sample(inner)]);
        }
        match ty {
            "string" => json!("x"),
            "bool" => json!(true),
            "u8" | "u16" | "u32" | "u64" | "s32" | "s64" => json!(200),
            "f64" => json!(1.5),
            "json" => json!({ "id": "x", "n": 1 }),
            "_" => Value::Null,
            name => match self.types.get(name) {
                Some(Def::Alias(t)) => self.sample(t),
                Some(Def::Record(fields)) => Value::Object(
                    fields
                        .iter()
                        .map(|(f, t)| (f.clone(), self.sample(t)))
                        .collect(),
                ),
                Some(Def::Variant(cases)) => match cases.first() {
                    Some((c, Some(t))) => json!({ c.clone(): self.sample(t) }),
                    Some((c, None)) => json!(c),
                    None => Value::Null,
                },
                Some(Def::Enum(cases)) => json!(cases.first().cloned().unwrap_or_default()),
                None => Value::Null,
            },
        }
    }

    /// The params of `method` with every one filled.
    pub fn sample_params(&self, method: Method) -> Value {
        let params: Map<String, Value> = self
            .method(method)
            .map(|m| {
                m.params
                    .iter()
                    .map(|(p, t)| (p.clone(), self.sample(t)))
                    .collect()
            })
            .unwrap_or_default();
        Value::Object(params)
    }

    /// Whether `value` is a `ty`, naming the first place it is not.
    pub fn check(&self, ty: &str, value: &Value) -> Result<(), String> {
        let fail = || Err(format!("{value} is not a {ty}"));
        if let Some(inner) = wrapped(ty, "option") {
            return if value.is_null() {
                Ok(())
            } else {
                self.check(inner, value)
            };
        }
        if let Some(inner) = wrapped(ty, "list") {
            return match value.as_array() {
                Some(items) => items.iter().try_for_each(|v| self.check(inner, v)),
                None => fail(),
            };
        }
        match ty {
            "string" if value.is_string() => Ok(()),
            "bool" if value.is_boolean() => Ok(()),
            "u8" | "u16" | "u32" | "u64" if value.is_u64() => Ok(()),
            "s32" | "s64" if value.is_i64() => Ok(()),
            "f64" if value.is_number() => Ok(()),
            "json" => Ok(()),
            "_" if value.is_null() => Ok(()),
            "string" | "bool" | "u8" | "u16" | "u32" | "u64" | "s32" | "s64" | "f64" | "_" => {
                fail()
            }
            name => match self.types.get(name) {
                Some(Def::Alias(t)) => self.check(t, value),
                Some(Def::Record(fields)) => {
                    let Some(object) = value.as_object() else {
                        return fail();
                    };
                    for (f, t) in fields {
                        let field = object.get(f).unwrap_or(&Value::Null);
                        self.check(t, field).map_err(|e| format!("{name}.{f}: {e}"))?;
                    }
                    Ok(())
                }
                Some(Def::Variant(cases)) => {
                    let found = match value {
                        Value::String(c) => cases.iter().any(|(n, t)| n == c && t.is_none()),
                        Value::Object(o) if o.len() == 1 => o.iter().next().is_some_and(|(c, v)| {
                            cases.iter().any(|(n, t)| {
                                n == c && t.as_deref().is_some_and(|t| self.check(t, v).is_ok())
                            })
                        }),
                        _ => false,
                    };
                    if found {
                        Ok(())
                    } else {
                        fail()
                    }
                }
                Some(Def::Enum(cases)) => match value.as_str() {
                    Some(c) if cases.iter().any(|n| n == c) => Ok(()),
                    _ => fail(),
                },
                None => Err(format!("{name} is not a type of the API")),
            },
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn names_every_function_of_the_world() {
        let names: Vec<&str> = Method::ALL.iter().map(|m| m.name()).collect();
        assert_eq!(
            names,
            [
                "host.call",
                "host.publish",
                "host.log",
                "net.fetch",
                "guest.init",
                "guest.handle",
                "guest.on-event"
            ]
        );
        assert_eq!(Method::GuestHandle.side(), Side::Guest);
        assert_eq!(Method::NetFetch.side(), Side::Host);
        assert!(Method::HostPublish.is_notification() && Method::GuestOnEvent.is_notification());
        assert!(!Method::GuestInit.is_notification());
        assert_eq!(API, "vorn:extension@0.9");
    }

    #[test]
    fn every_method_has_a_mapping_and_samples_that_check() {
        let m = mapping();
        for method in Method::ALL {
            let doc = m.method(method).expect("documented");
            for (p, t) in &doc.params {
                m.check(t, &m.sample(t)).unwrap_or_else(|e| panic!("{p}: {e}"));
            }
            if let Some(result) = &doc.result {
                m.check(result, &m.sample(result)).unwrap();
            }
        }
    }

    #[test]
    fn checks_values_against_the_types() {
        let m = mapping();
        assert!(m.check("row-op", &json!({ "upsert": { "id": "a" } })).is_ok());
        assert!(m.check("row-op", &json!("clear")).is_ok());
        assert!(m.check("row-op", &json!("delete")).is_err());
        assert!(m.check("level", &json!("warn")).is_ok());
        assert!(m.check("level", &json!("loud")).is_err());
        let err = m
            .check("response", &json!({ "status": "ok", "headers": [], "body": "" }))
            .unwrap_err();
        assert!(err.starts_with("response.status:"), "{err}");
        assert!(m.check("option<string>", &Value::Null).is_ok());
    }

    #[test]
    fn the_typed_params_read_what_the_mapping_samples() {
        let m = mapping();
        let fetch: params::NetFetch =
            serde_json::from_value(m.sample_params(Method::NetFetch)).unwrap();
        assert_eq!(fetch.req.headers[0].name, "x");
        let publish: params::HostPublish =
            serde_json::from_value(m.sample_params(Method::HostPublish)).unwrap();
        assert!(matches!(publish.ops[0], types::RowOp::Upsert(_)));
    }

    #[test]
    fn publishes_the_mapping_it_builds_with() {
        let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("wit/stdio.json");
        if std::env::var_os("VORN_UPDATE_WIT").is_some() {
            std::fs::write(&path, STDIO).unwrap();
        }
        let published = std::fs::read_to_string(&path).unwrap_or_default();
        assert_eq!(
            published.replace("\r\n", "\n"),
            STDIO,
            "wit/stdio.json is out of date; run the tests with VORN_UPDATE_WIT=1"
        );
    }
}

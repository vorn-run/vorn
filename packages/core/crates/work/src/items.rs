//! Lists of records, as a for-each loop walks them and a review gate draws
//! them (`item-list.ts`).
//!
//! Both take whatever a step produced: a list, the JSON text of one, or an
//! object wrapping exactly one (`{ "findings": [...] }`). Where JSON text
//! does not parse, the line and column are serde_json's, which can name a
//! different spot than V8's message for the same text.

use serde_json::Value;

/// The items of a list, and the key of the object that held it, if one did.
#[derive(Clone, Debug, PartialEq)]
pub struct ItemList {
    pub items: Vec<Value>,
    pub wrapper_key: Option<String>,
}

/// Where a JSON text failed to parse, counted from line 1, column 1.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct JsonErrorAt {
    pub line: usize,
    pub column: usize,
}

/// Parses `text` as JSON, or says where it broke, as V8's message for the
/// same text does: a token it did not expect and an early end name no
/// position, so the end of the text is reported; the rest name where.
pub fn parse_json(text: &str) -> Result<Value, JsonErrorAt> {
    serde_json::from_str(text).map_err(|err| {
        let unplaced =
            err.is_eof() || err.line() == 0 || err.to_string().starts_with("expected value");
        if unplaced {
            end_of(text)
        } else {
            JsonErrorAt {
                line: err.line(),
                column: err.column().max(1),
            }
        }
    })
}

/// The position just past the end of `text`.
fn end_of(text: &str) -> JsonErrorAt {
    let last = text.rsplit('\n').next().unwrap_or("");
    JsonErrorAt {
        line: text.split('\n').count(),
        column: crate::js::utf16_len(last) + 1,
    }
}

/// `toItemList`.
pub fn to_item_list(value: &Value) -> Result<ItemList, String> {
    let parsed;
    let data = match value {
        Value::String(text) => {
            let text = text.trim();
            if text.is_empty() {
                return Ok(ItemList {
                    items: Vec::new(),
                    wrapper_key: None,
                });
            }
            parsed = parse_json(text).map_err(|at| {
                format!(
                    "Not a list: the JSON is invalid at line {}, column {}.",
                    at.line, at.column
                )
            })?;
            &parsed
        }
        other => other,
    };
    match data {
        Value::Array(items) => Ok(ItemList {
            items: items.clone(),
            wrapper_key: None,
        }),
        Value::Object(map) => {
            let lists: Vec<(&String, &Value)> = map.iter().filter(|(_, v)| v.is_array()).collect();
            match lists.as_slice() {
                [(key, Value::Array(items))] => Ok(ItemList {
                    items: items.clone(),
                    wrapper_key: Some((*key).clone()),
                }),
                [] => Err(NOT_A_LIST.to_owned()),
                many => Err(format!(
                    "Not a list: the object holds several ({}).",
                    many.iter()
                        .map(|(k, _)| k.as_str())
                        .collect::<Vec<_>>()
                        .join(", ")
                )),
            }
        }
        _ => Err(NOT_A_LIST.to_owned()),
    }
}

const NOT_A_LIST: &str = "Not a list: expected a JSON array, or an object holding one.";

/// `isRecordList`: every item a plain object, which is what a table draws.
pub fn is_record_list(items: &[Value]) -> bool {
    !items.is_empty() && items.iter().all(Value::is_object)
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn reads_lists_from_values_text_and_wrappers() {
        assert_eq!(
            to_item_list(&json!([1, 2])).unwrap().items,
            [json!(1), json!(2)]
        );
        assert_eq!(
            to_item_list(&json!("  ")).unwrap().items,
            Vec::<Value>::new()
        );
        let wrapped = to_item_list(&json!(r#"{"findings":[{"a":1}],"n":2}"#)).unwrap();
        assert_eq!(wrapped.wrapper_key.as_deref(), Some("findings"));
        assert!(is_record_list(&wrapped.items));
        assert_eq!(
            to_item_list(&json!({ "a": [], "b": [] })).unwrap_err(),
            "Not a list: the object holds several (a, b)."
        );
        assert_eq!(to_item_list(&json!(3)).unwrap_err(), NOT_A_LIST);
        assert_eq!(to_item_list(&json!({ "a": 1 })).unwrap_err(), NOT_A_LIST);
        assert!(to_item_list(&json!("[1,"))
            .unwrap_err()
            .starts_with("Not a list: the JSON is invalid at line 1"));
        assert!(!is_record_list(&[]));
        assert!(!is_record_list(&[json!({}), json!([])]));
    }

    #[test]
    fn says_where_json_broke() {
        // V8 names no position for an unexpected token or an early end.
        assert_eq!(
            parse_json("{\n  \"a\": }").unwrap_err(),
            JsonErrorAt { line: 2, column: 9 }
        );
        assert_eq!(
            parse_json("[{\"id\":1},").unwrap_err(),
            JsonErrorAt {
                line: 1,
                column: 11
            }
        );
        assert_eq!(parse_json("[1] x").unwrap_err().column, 5);
        assert!(parse_json("[1]").is_ok());
    }
}

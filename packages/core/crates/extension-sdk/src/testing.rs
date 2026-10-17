//! A [`Host`] in memory, to test an extension's logic with `cargo test`:
//! core methods answered by closures, rows kept as Vorn keeps them, and
//! storage values, files and started sessions kept in maps. SQL is not
//! answered; an extension that keeps tables is tested against Vorn.

use std::collections::BTreeMap;

use serde_json::Value;

use crate::{Blob, Error, Host, Level, Op, SessionStart};

type Answerer = Box<dyn FnMut(&Value) -> Result<Value, Error>>;

/// Vorn in memory.
#[derive(Default)]
pub struct FakeHost {
    answers: BTreeMap<String, Answerer>,
    /// Every core method called, in order, with its args.
    pub calls: Vec<(String, Value)>,
    rows: BTreeMap<String, Vec<Value>>,
    /// What `set` kept.
    pub kv: BTreeMap<String, Value>,
    /// The extension's files.
    pub files: BTreeMap<String, Vec<u8>>,
    /// The sessions it started, in order; each was answered `s1`, `s2`, ...
    pub sessions: Vec<SessionStart>,
    /// What it logged.
    pub logs: Vec<(Level, String)>,
}

impl std::fmt::Debug for FakeHost {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("FakeHost")
            .field("calls", &self.calls)
            .field("rows", &self.rows)
            .finish_non_exhaustive()
    }
}

impl FakeHost {
    /// Answers core method `method` with `answer`, given the call's args.
    pub fn answer(
        mut self,
        method: &str,
        answer: impl FnMut(&Value) -> Result<Value, Error> + 'static,
    ) -> FakeHost {
        self.answers.insert(method.to_owned(), Box::new(answer));
        self
    }

    /// The args of every call to `method`, in order.
    pub fn asked(&self, method: &str) -> Vec<Value> {
        self.calls
            .iter()
            .filter(|(m, _)| m == method)
            .map(|(_, a)| a.clone())
            .collect()
    }
}

impl Host for FakeHost {
    fn call(&mut self, method: &str, args: Value) -> Result<Value, Error> {
        self.calls.push((method.to_owned(), args.clone()));
        match self.answers.get_mut(method) {
            Some(answer) => answer(&args),
            None => Err(Error::new(
                "unknown-method",
                format!("Vorn has no method {method}"),
            )),
        }
    }

    /// Applies `ops` as Vorn does: an upsert replaces a row in place or adds it last.
    fn publish(&mut self, collection: &str, ops: Vec<Op>) {
        let rows = self.rows.entry(collection.to_owned()).or_default();
        for op in ops {
            match op {
                Op::Upsert(row) => {
                    let Some(id) = row.get("id").and_then(Value::as_str).map(str::to_owned) else {
                        continue;
                    };
                    match rows.iter_mut().find(|r| r["id"] == id.as_str()) {
                        Some(kept) => *kept = row,
                        None => rows.push(row),
                    }
                }
                Op::Delete(id) => rows.retain(|r| r["id"] != id.as_str()),
                Op::Clear => rows.clear(),
            }
        }
    }

    fn log(&mut self, level: Level, message: &str) {
        self.logs.push((level, message.to_owned()));
    }

    fn rows(&mut self, collection: &str) -> Result<Vec<Value>, Error> {
        Ok(self.rows.get(collection).cloned().unwrap_or_default())
    }

    fn start_session(&mut self, req: &SessionStart) -> Result<String, Error> {
        self.sessions.push(req.clone());
        Ok(format!("s{}", self.sessions.len()))
    }

    fn get(&mut self, key: &str) -> Result<Option<Value>, Error> {
        Ok(self.kv.get(key).cloned())
    }

    fn set(&mut self, key: &str, value: &Value) -> Result<(), Error> {
        if value.is_null() {
            self.kv.remove(key);
        } else {
            self.kv.insert(key.to_owned(), value.clone());
        }
        Ok(())
    }

    fn put_file(&mut self, name: &str, data: &[u8]) -> Result<(), Error> {
        self.files.insert(name.to_owned(), data.to_vec());
        Ok(())
    }

    fn file(&mut self, name: &str) -> Result<Option<Vec<u8>>, Error> {
        Ok(self.files.get(name).cloned())
    }

    fn files(&mut self, prefix: &str) -> Result<Vec<Blob>, Error> {
        Ok(self
            .files
            .iter()
            .filter(|(n, _)| n.starts_with(prefix))
            .map(|(n, d)| Blob {
                name: n.clone(),
                size: u64::try_from(d.len()).unwrap_or(u64::MAX),
                modified: 0,
            })
            .collect())
    }

    fn delete_file(&mut self, name: &str) -> Result<bool, Error> {
        Ok(self.files.remove(name).is_some())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn keeps_rows_in_the_order_vorn_does() {
        let mut h = FakeHost::default();
        h.publish(
            "c",
            vec![
                Op::Upsert(json!({ "id": "a", "n": 1 })),
                Op::Upsert(json!({ "id": "b" })),
                Op::Upsert(json!({ "id": "a", "n": 2 })),
                Op::Upsert(json!({ "no": "id" })),
            ],
        );
        assert_eq!(
            h.rows("c").unwrap(),
            [json!({ "id": "a", "n": 2 }), json!({ "id": "b" })]
        );
        h.publish("c", vec![Op::Delete("a".into())]);
        assert_eq!(h.rows("c").unwrap(), [json!({ "id": "b" })]);
        h.publish("c", vec![Op::Clear]);
        assert!(h.rows("c").unwrap().is_empty());
    }

    #[test]
    fn answers_what_it_is_told_and_refuses_the_rest() {
        let mut h = FakeHost::default().answer("sessions.list", |_| Ok(json!([])));
        assert_eq!(h.call("sessions.list", Value::Null).unwrap(), json!([]));
        assert_eq!(
            h.call("git.status", json!({})).unwrap_err().code,
            "unknown-method"
        );
        assert_eq!(h.asked("git.status"), [json!({})]);
        assert_eq!(h.query("SELECT 1", &[]).unwrap_err().code, "unsupported");
    }
}

//! Two edits of one workflow made apart, an edit of its file and an edit in
//! the editor, meeting: what each changed since the version both started
//! from is kept. Steps are matched by id and config keys by name, so two
//! people changing different steps, or different keys of one step, lose
//! nothing; where both changed the same thing, the file's edit wins, since a
//! file is what a person wrote by hand.

use serde_json::{Map, Value};

/// `base` with what `ours` (the editor) and `theirs` (the file) each changed.
pub fn meet(base: &Value, ours: &Value, theirs: &Value) -> Value {
    if ours == theirs || ours == base {
        return theirs.clone();
    }
    if theirs == base {
        return ours.clone();
    }
    match (base, ours, theirs) {
        (Value::Object(b), Value::Object(o), Value::Object(t)) => Value::Object(objects(b, o, t)),
        (Value::Array(b), Value::Array(o), Value::Array(t)) if keyed(b) && keyed(o) && keyed(t) => {
            Value::Array(lists(b, o, t))
        }
        _ => theirs.clone(),
    }
}

fn objects(
    b: &Map<String, Value>,
    o: &Map<String, Value>,
    t: &Map<String, Value>,
) -> Map<String, Value> {
    let mut out = Map::new();
    let keys = t.keys().chain(o.keys().filter(|k| !t.contains_key(*k)));
    for k in keys {
        match (b.get(k), o.get(k), t.get(k)) {
            (base, Some(ov), Some(tv)) => {
                let base = base.cloned().unwrap_or(Value::Null);
                out.insert(k.clone(), meet(&base, ov, tv));
            }
            // Taken out by the file, untouched in the editor: out.
            (Some(bv), Some(ov), None) if bv == ov => {}
            (_, Some(ov), None) => {
                out.insert(k.clone(), ov.clone());
            }
            (Some(bv), None, Some(tv)) if bv == tv => {}
            (_, None, Some(tv)) => {
                out.insert(k.clone(), tv.clone());
            }
            (_, None, None) => {}
        }
    }
    out
}

/// A list of objects that each have a distinct string id: steps and edges.
fn keyed(list: &[Value]) -> bool {
    let ids: Vec<Option<&str>> = list
        .iter()
        .map(|v| v.get("id").and_then(Value::as_str))
        .collect();
    ids.iter().all(Option::is_some) && ids.iter().enumerate().all(|(i, a)| !ids[..i].contains(a))
}

fn id(v: &Value) -> &str {
    v.get("id").and_then(Value::as_str).unwrap_or_default()
}

fn lists(b: &[Value], o: &[Value], t: &[Value]) -> Vec<Value> {
    let find = |list: &[Value], key: &str| list.iter().find(|v| id(v) == key).cloned();
    let mut out = Vec::new();
    // The file's order, then what only the editor added.
    let order = t.iter().map(|v| id(v).to_owned()).chain(
        o.iter()
            .map(|v| id(v).to_owned())
            .filter(|k| find(t, k).is_none()),
    );
    for key in order {
        match (find(b, &key), find(o, &key), find(t, &key)) {
            (base, Some(ov), Some(tv)) => {
                out.push(meet(&base.unwrap_or(Value::Null), &ov, &tv));
            }
            (Some(bv), Some(ov), None) if bv == ov => {}
            (_, Some(ov), None) => out.push(ov),
            (Some(bv), None, Some(tv)) if bv == tv => {}
            (_, None, Some(tv)) => out.push(tv),
            (_, None, None) => {}
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn wf(a: &str, b: &str, extra: Option<Value>) -> Value {
        let mut nodes = vec![
            json!({ "id": "a", "type": "script", "label": "A", "config": { "scriptContent": a, "cwd": "/x" } }),
            json!({ "id": "b", "type": "script", "label": b, "config": { "scriptContent": "b" } }),
        ];
        nodes.extend(extra);
        json!({ "id": "w", "name": "W", "nodes": nodes, "edges": [] })
    }

    #[test]
    fn edits_of_different_steps_both_stay() {
        let base = wf("echo a", "B", None);
        let editor = wf("echo a", "B renamed", None);
        let file = wf("echo edited", "B", None);
        assert_eq!(
            meet(&base, &editor, &file),
            wf("echo edited", "B renamed", None)
        );
    }

    #[test]
    fn a_step_one_side_added_and_one_the_other_removed() {
        let base = wf("a", "B", Some(json!({ "id": "c", "type": "script" })));
        let editor = wf("a", "B", Some(json!({ "id": "d", "type": "script" })));
        let file = wf(
            "a",
            "B",
            Some(json!({ "id": "c", "type": "script", "label": "C" })),
        );
        let met = meet(&base, &editor, &file);
        let ids: Vec<&str> = met["nodes"].as_array().unwrap().iter().map(id).collect();
        // c: removed in the editor, but the file changed it, so it stays; d: new in the editor.
        assert_eq!(ids, ["a", "b", "c", "d"]);
    }

    #[test]
    fn the_file_wins_where_both_changed_one_thing() {
        let base = wf("a", "B", None);
        let editor = wf("from the editor", "B", None);
        let file = wf("from the file", "B", None);
        assert_eq!(
            meet(&base, &editor, &file)["nodes"][0]["config"]["scriptContent"],
            "from the file"
        );
    }
}

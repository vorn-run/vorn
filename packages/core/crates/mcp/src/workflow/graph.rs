//! The graph checks the workflow tools make, from `@vornrun/shared/workflow-graph`
//! and `tools/workflows.ts`.
//!
//! A graph reaches these as the caller sent it: `import_workflow` checks a
//! file nothing has parsed against a schema, so a node may be anything. Each
//! check reads it the way the TypeScript does, and where the TypeScript would
//! throw on a malformed node, so does the port, with V8's message.

use serde_json::Value;

use crate::json;

/// Ceiling on how many times a gate may ask, as the node reference records it.
pub fn max_gate_rounds() -> f64 {
    crate::workflow_nodes()
        .pointer("/limits/maxGateRounds")
        .and_then(Value::as_f64)
        .unwrap_or(10.0)
}

/// `a === b` for the ids and types a graph is keyed on.
fn same(a: Option<&Value>, b: Option<&Value>) -> bool {
    json::strict_equals(a, b)
}

/// `node.label || node.id`, as a template prints it.
pub fn name_of(node: &Value) -> String {
    if json::truthy(node.get("label")) {
        json::display(node.get("label"))
    } else {
        json::display(node.get("id"))
    }
}

/// `node.type === 'loop'`, throwing for a node that is not an object.
fn is_type(node: &Value, kind: &str) -> Result<bool, String> {
    Ok(json::prop(Some(node), "type")?.and_then(Value::as_str) == Some(kind))
}

/// `for (const id of value ?? [])`: what a for-of walks, and what it throws on.
fn iterate(value: Option<&Value>) -> Result<Vec<Value>, String> {
    match value {
        None | Some(Value::Null) => Ok(Vec::new()),
        Some(Value::Array(items)) => Ok(items.clone()),
        Some(Value::String(s)) => Ok(s.chars().map(|c| Value::from(c.to_string())).collect()),
        Some(Value::Object(_)) => {
            Err("object is not iterable (cannot read property Symbol(Symbol.iterator))".to_owned())
        }
        Some(other @ (Value::Number(_) | Value::Bool(_))) => Err(format!(
            "{} {} is not iterable (cannot read property Symbol(Symbol.iterator))",
            if other.is_number() {
                "number"
            } else {
                "boolean"
            },
            json::display(Some(other))
        )),
    }
}

/// `node.config.bodyNodeIds`, throwing for a loop without a config.
fn body_ids(node: &Value) -> Result<Option<&Value>, String> {
    let config = json::prop(Some(node), "config")?;
    json::prop(config, "bodyNodeIds")
}

/// `validateLoopBodies`: loop bodies name steps of the same workflow.
pub fn validate_loop_bodies(nodes: &[Value]) -> Result<Vec<String>, String> {
    let ids = nodes
        .iter()
        .map(|n| json::prop(Some(n), "id").map(|id| id.cloned()))
        .collect::<Result<Vec<_>, _>>()?;
    let mut errors = Vec::new();
    for node in nodes {
        if !is_type(node, "loop")? {
            continue;
        }
        let raw = body_ids(node)?;
        for id in iterate(raw)? {
            if !ids.iter().any(|known| same(known.as_ref(), Some(&id))) {
                errors.push(format!(
                    "loop \"{}\" references unknown body step \"{}\"",
                    name_of(node),
                    json::display(Some(&id))
                ));
            }
        }
        let lists_itself = match raw {
            // `includes` on a string looks for a substring.
            Some(Value::String(s)) => s.contains(&json::display(node.get("id"))),
            Some(Value::Array(items)) => items.iter().any(|id| same(Some(id), node.get("id"))),
            _ => false,
        };
        if lists_itself {
            errors.push(format!(
                "loop \"{}\" lists itself as a body step",
                name_of(node)
            ));
        }
    }
    Ok(errors)
}

/// `loopBodyOwners`: which loop owns each body step. The first loop to claim
/// a step keeps it, and ids of steps that do not exist own nothing.
pub fn loop_body_owners(nodes: &[Value]) -> Result<Vec<(Value, Value)>, String> {
    let exists: Vec<Option<&Value>> = nodes.iter().map(|n| n.get("id")).collect();
    let mut owners: Vec<(Value, Value)> = Vec::new();
    for n in nodes {
        if !is_type(n, "loop")? {
            continue;
        }
        let loop_id = n.get("id").cloned().unwrap_or(Value::Null);
        for id in iterate(body_ids(n)?)? {
            if exists.iter().any(|e| same(*e, Some(&id)))
                && !same(Some(&id), n.get("id"))
                && !owners.iter().any(|(k, _)| same(Some(k), Some(&id)))
            {
                owners.push((id, loop_id.clone()));
            }
        }
    }
    Ok(owners)
}

fn owner_of<'a>(owners: &'a [(Value, Value)], id: Option<&Value>) -> Option<&'a Value> {
    owners
        .iter()
        .find(|(k, _)| same(Some(k), id))
        .map(|(_, v)| v)
}

/// A loop body's members and its edges as (source, target) pairs.
type BodyGraph = (Vec<Value>, Vec<(Value, Value)>);

/// `loopBodyGraph`: a loop's members, the edges between them, chained in
/// list order when there are none.
fn loop_body_graph(
    nodes: &[Value],
    edges: &[Value],
    loop_node: &Value,
) -> Result<BodyGraph, String> {
    let owners = loop_body_owners(nodes)?;
    let listed = body_ids(loop_node)?;
    let listed = match listed {
        None | Some(Value::Null) => Vec::new(),
        Some(Value::Array(items)) => items.clone(),
        Some(_) => {
            return Err("(loop.config.bodyNodeIds ?? []).filter is not a function".to_owned())
        }
    };
    // `new Map(nodes.map((n) => [n.id, n]))`: a later node with the same id wins.
    let by_id = |id: &Value| nodes.iter().rev().find(|n| same(n.get("id"), Some(id)));
    let members: Vec<Value> = listed
        .iter()
        .filter(|id| same(owner_of(&owners, Some(id)), loop_node.get("id")))
        .filter_map(|id| by_id(id).cloned())
        .collect();
    let is_member = |id: Option<&Value>| members.iter().any(|m| same(m.get("id"), id));
    let mut inner: Vec<(Value, Value)> = edges
        .iter()
        .filter(|e| is_member(e.get("source")) && is_member(e.get("target")))
        .map(|e| {
            (
                e.get("source").cloned().unwrap_or(Value::Null),
                e.get("target").cloned().unwrap_or(Value::Null),
            )
        })
        .collect();
    if inner.is_empty() && members.len() > 1 {
        inner = members
            .windows(2)
            .map(|pair| {
                (
                    pair[0].get("id").cloned().unwrap_or(Value::Null),
                    pair[1].get("id").cloned().unwrap_or(Value::Null),
                )
            })
            .collect();
    }
    Ok((members, inner))
}

/// `loopStructureError`: why a loop's body cannot run, or `None` when it can.
pub fn loop_structure_error(
    nodes: &[Value],
    edges: &[Value],
    loop_node: &Value,
) -> Result<Option<String>, String> {
    let (members, inner) = loop_body_graph(nodes, edges, loop_node)?;
    if members.is_empty() {
        return Ok(Some(
            "Loop has no body steps. Add at least one step for it to repeat.".to_owned(),
        ));
    }
    let is_member = |id: Option<&Value>| members.iter().any(|m| same(m.get("id"), id));
    for m in &members {
        let label = json::display(m.get("label"));
        match m.get("type").and_then(Value::as_str) {
            Some("approval") => {
                return Ok(Some(format!(
                    "Loop body contains an approval gate (\"{label}\"), which is not supported."
                )))
            }
            Some("loop") => {
                return Ok(Some(format!(
                    "Loop body contains another loop (\"{label}\"), which is not supported."
                )))
            }
            Some("trigger") => {
                return Ok(Some(format!("Loop body contains a trigger (\"{label}\").")))
            }
            _ => {}
        }
    }
    for e in edges {
        let source = json::prop(Some(e), "source")?;
        let target = e.get("target");
        if is_member(target) && !is_member(source) && !same(source, loop_node.get("id")) {
            let from = nodes
                .iter()
                .find(|n| same(n.get("id"), source))
                .and_then(|n| n.get("label"))
                .filter(|l| !l.is_null())
                .or(source);
            return Ok(Some(format!(
                "\"{}\" feeds a step inside the loop from outside it. Only the loop starts its steps.",
                json::display(from)
            )));
        }
        if is_member(source) && !is_member(target) && json::truthy(e.get("conditionBranch")) {
            return Ok(Some(
                "A condition inside the loop branches to a step outside it. Keep both branches inside the loop."
                    .to_owned(),
            ));
        }
    }
    // Kahn's algorithm over the distinct members: anything left sits on a cycle.
    let mut indegree: Vec<(Value, i64)> = Vec::new();
    for m in &members {
        let id = m.get("id").cloned().unwrap_or(Value::Null);
        if !indegree.iter().any(|(k, _)| same(Some(k), Some(&id))) {
            indegree.push((id, 0));
        }
    }
    let slot = |indegree: &mut Vec<(Value, i64)>, id: &Value| -> usize {
        match indegree.iter().position(|(k, _)| same(Some(k), Some(id))) {
            Some(i) => i,
            None => {
                indegree.push((id.clone(), 0));
                indegree.len() - 1
            }
        }
    };
    for (_, target) in &inner {
        let i = slot(&mut indegree, target);
        indegree[i].1 += 1;
    }
    let mut queue: std::collections::VecDeque<Value> = indegree
        .iter()
        .filter(|(_, d)| *d == 0)
        .map(|(id, _)| id.clone())
        .collect();
    let mut visited = 0;
    while let Some(id) = queue.pop_front() {
        visited += 1;
        for (source, target) in &inner {
            if !same(Some(source), Some(&id)) {
                continue;
            }
            let i = slot(&mut indegree, target);
            indegree[i].1 -= 1;
            if indegree[i].1 == 0 {
                queue.push_back(target.clone());
            }
        }
    }
    if visited < members.len() {
        return Ok(Some("The steps inside the loop form a cycle.".to_owned()));
    }
    Ok(None)
}

/// `validateLoopStructures`.
pub fn validate_loop_structures(nodes: &[Value], edges: &[Value]) -> Result<Vec<String>, String> {
    let mut errors = Vec::new();
    for node in nodes {
        if !is_type(node, "loop")? {
            continue;
        }
        if let Some(error) = loop_structure_error(nodes, edges, node)? {
            errors.push(format!("loop \"{}\": {error}", name_of(node)));
        }
    }
    Ok(errors)
}

/// `nodesBetween(from, to, edges).size === 0`: whether `to` is below `from`.
fn leads_to(from: &Value, to: Option<&Value>, edges: &[Value]) -> Result<bool, String> {
    if same(Some(from), to) {
        return Ok(false);
    }
    let mut pairs = Vec::with_capacity(edges.len());
    for e in edges {
        pairs.push((json::prop(Some(e), "source")?, e.get("target")));
    }
    let mut seen: Vec<Option<&Value>> = vec![Some(from)];
    let mut queue = std::collections::VecDeque::from([Some(from)]);
    while let Some(at) = queue.pop_front() {
        for (source, target) in &pairs {
            if same(*source, at) && !seen.iter().any(|s| same(*s, *target)) {
                seen.push(*target);
                queue.push_back(*target);
            }
        }
    }
    // `to` below `from`; the steps between them include both ends.
    Ok(seen.iter().any(|s| same(*s, to)))
}

/// `validateGateFeedback`: a gate that takes changes names a step above it to
/// redo from, and a round count the engine allows.
pub fn validate_gate_feedback(nodes: &[Value], edges: &[Value]) -> Result<Vec<String>, String> {
    let mut errors = Vec::new();
    let owners = loop_body_owners(nodes)?;
    let max = max_gate_rounds();
    for node in nodes {
        let feedback = json::field(json::field(Some(node), "config"), "feedback");
        if !is_type(node, "approval")? || feedback.is_none() {
            continue;
        }
        let name = name_of(node);
        let from = match json::field(feedback, "from") {
            Some(Value::String(s)) => Value::from(s.as_str()),
            _ => Value::from(""),
        };
        let source = nodes.iter().find(|n| same(n.get("id"), Some(&from)));
        match source {
            None => errors.push(format!(
                "gate \"{name}\" redoes from unknown step \"{}\"",
                json::display(Some(&from))
            )),
            Some(source) if source.get("type").and_then(Value::as_str) == Some("trigger") => {
                errors.push(format!("gate \"{name}\" cannot redo from its trigger"))
            }
            Some(source) => {
                let source_name = if json::truthy(source.get("label")) {
                    json::display(source.get("label"))
                } else {
                    json::display(Some(&from))
                };
                if let Some(owner) = owner_of(&owners, Some(&from)) {
                    let loop_name = nodes
                        .iter()
                        .find(|n| same(n.get("id"), Some(owner)))
                        .and_then(|n| n.get("label"))
                        .filter(|l| json::truthy(Some(l)))
                        .unwrap_or(owner);
                    errors.push(format!(
                        "gate \"{name}\" redoes from \"{source_name}\", which is inside loop \"{}\"; redo from the loop, or a step before it",
                        json::display(Some(loop_name))
                    ));
                } else if !leads_to(&from, node.get("id"), edges)? {
                    errors.push(format!(
                        "gate \"{name}\" redoes from \"{source_name}\", which does not lead to it"
                    ));
                }
            }
        }
        let rounds = json::to_number(json::field(feedback, "maxRounds"));
        if rounds.fract() != 0.0 || !rounds.is_finite() || rounds < 2.0 || rounds > max {
            errors.push(format!(
                "gate \"{name}\" needs maxRounds between 2 and {}",
                json::number_to_string(max)
            ));
        }
    }
    Ok(errors)
}

/// `validateGraph`: everything about a graph no single node can check.
pub fn validate_graph(nodes: &[Value], edges: &[Value]) -> Result<Vec<String>, String> {
    let mut errors = validate_loop_bodies(nodes)?;
    errors.extend(validate_loop_structures(nodes, edges)?);
    errors.extend(validate_gate_feedback(nodes, edges)?);
    Ok(errors)
}

/// `isSignInWait`: a step parked until its connection signs in again.
pub fn is_sign_in_wait(state: &Value) -> bool {
    state.get("status").and_then(Value::as_str) == Some("waiting")
        && state.get("waitingFor").and_then(Value::as_str) == Some("signIn")
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn node(id: &str, kind: &str, config: Value) -> Value {
        json!({ "id": id, "type": kind, "label": id.to_uppercase(), "config": config })
    }

    fn edge(source: &str, target: &str) -> Value {
        json!({ "id": format!("{source}-{target}"), "source": source, "target": target })
    }

    #[test]
    fn a_sound_loop_passes() {
        let nodes = [
            node("t", "trigger", json!({})),
            node("l", "loop", json!({ "bodyNodeIds": ["a", "b"] })),
            node("a", "script", json!({})),
            node("b", "script", json!({})),
        ];
        let edges = [edge("t", "l"), edge("l", "a"), edge("a", "b")];
        assert_eq!(
            validate_graph(&nodes, &edges).unwrap(),
            Vec::<String>::new()
        );
    }

    #[test]
    fn names_what_is_wrong_with_a_loop() {
        let nodes = [
            node("t", "trigger", json!({})),
            node("l", "loop", json!({ "bodyNodeIds": ["a", "l", "x"] })),
            node("a", "approval", json!({})),
        ];
        let edges = [edge("t", "a")];
        assert_eq!(
            validate_graph(&nodes, &edges).unwrap(),
            [
                "loop \"L\" references unknown body step \"x\"",
                "loop \"L\" lists itself as a body step",
                "loop \"L\": Loop body contains an approval gate (\"A\"), which is not supported.",
            ]
        );
    }

    #[test]
    fn a_cycle_inside_a_loop_is_refused() {
        let nodes = [
            node("l", "loop", json!({ "bodyNodeIds": ["a", "b"] })),
            node("a", "script", json!({})),
            node("b", "script", json!({})),
        ];
        let edges = [edge("a", "b"), edge("b", "a")];
        assert_eq!(
            validate_loop_structures(&nodes, &edges).unwrap(),
            ["loop \"L\": The steps inside the loop form a cycle."]
        );
    }

    #[test]
    fn a_gate_must_redo_from_a_step_above_it() {
        let nodes = [
            node("t", "trigger", json!({})),
            node("s", "script", json!({})),
            node(
                "g",
                "approval",
                json!({ "feedback": { "from": "s", "maxRounds": 3 } }),
            ),
            node(
                "h",
                "approval",
                json!({ "feedback": { "from": "t", "maxRounds": 11 } }),
            ),
        ];
        let edges = [edge("t", "s"), edge("s", "g")];
        assert_eq!(
            validate_gate_feedback(&nodes, &edges).unwrap(),
            [
                "gate \"H\" cannot redo from its trigger",
                "gate \"H\" needs maxRounds between 2 and 10"
            ]
        );
        let edges = [edge("t", "s")];
        assert_eq!(
            validate_gate_feedback(&nodes[..3], &edges).unwrap(),
            ["gate \"G\" redoes from \"S\", which does not lead to it"]
        );
    }

    #[test]
    fn a_loop_without_a_config_throws_as_javascript_does() {
        let nodes = [json!({ "id": "l", "type": "loop" })];
        assert_eq!(
            validate_graph(&nodes, &[]).unwrap_err(),
            "Cannot read properties of undefined (reading 'bodyNodeIds')"
        );
        let nodes = [node("l", "loop", json!({ "bodyNodeIds": 5 }))];
        assert_eq!(
            validate_graph(&nodes, &[]).unwrap_err(),
            "number 5 is not iterable (cannot read property Symbol(Symbol.iterator))"
        );
    }
}

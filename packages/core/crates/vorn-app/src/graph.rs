//! A workflow's graph as the editor changes it: inserting, appending, forking,
//! looping and removing steps, and placing new steps beside arranged ones.
//! Nodes and edges stay JSON, as the definition is stored.

use std::collections::{HashMap, HashSet, VecDeque};

use serde_json::{json, Value};

use crate::runs::s;

/// A step card's width.
pub const CARD_WIDTH: f64 = 280.0;
/// A loop's width: a loop draws its body inside itself.
pub const LOOP_WIDTH: f64 = 312.0;
/// The lattice the canvas snaps drags to.
pub const GRID: f64 = 8.0;
/// One step down the trunk.
pub const PITCH: f64 = 144.0;
/// The `beforeNodeId` that names a loop's body.
pub const LOOP_BODY: &str = "__LOOP_BODY__";
/// The `beforeNodeId` that names the spot above a fork.
pub const FORK: &str = "__FORK__";

/// A new random UUID.
pub fn new_id() -> String {
    crate::tasks::uuid()
}

fn id(v: &Value) -> &str {
    s(v, "id").unwrap_or("")
}

fn kind(v: &Value) -> &str {
    s(v, "type").unwrap_or("")
}

/// An edge `source → target`, with its branch tag.
pub fn edge(source: &str, target: &str, branch: Option<&str>) -> Value {
    let mut e = json!({ "id": new_id(), "source": source, "target": target });
    if let Some(b) = branch {
        e["conditionBranch"] = json!(b);
    }
    e
}

/// Each node's targets, in edge order.
pub fn successors(edges: &[Value]) -> HashMap<String, Vec<String>> {
    let mut map: HashMap<String, Vec<String>> = HashMap::new();
    for e in edges {
        if let (Some(a), Some(b)) = (s(e, "source"), s(e, "target")) {
            map.entry(a.to_owned()).or_default().push(b.to_owned());
        }
    }
    map
}

fn reach_all(start: &str, succ: &HashMap<String, Vec<String>>) -> HashSet<String> {
    let mut seen = HashSet::new();
    let mut queue = VecDeque::from([start.to_owned()]);
    while let Some(n) = queue.pop_front() {
        if seen.insert(n.clone()) {
            queue.extend(succ.get(&n).into_iter().flatten().cloned());
        }
    }
    seen
}

/// The first node, breadth first, that every branch reaches.
pub fn find_join(children: &[String], succ: &HashMap<String, Vec<String>>) -> Option<String> {
    if children.len() <= 1 {
        return None;
    }
    let sets: Vec<HashSet<String>> = children.iter().map(|c| reach_all(c, succ)).collect();
    let mut visited = HashSet::new();
    let mut queue: VecDeque<String> = children.iter().cloned().collect();
    while let Some(cur) = queue.pop_front() {
        if !visited.insert(cur.clone()) {
            continue;
        }
        if !children.contains(&cur) && sets.iter().all(|s| s.contains(&cur)) {
            return Some(cur);
        }
        queue.extend(succ.get(&cur).into_iter().flatten().cloned());
    }
    None
}

/// Everything downstream of `start`, itself excluded.
pub fn reachable_from(start: &str, edges: &[Value]) -> HashSet<String> {
    let succ = successors(edges);
    let mut seen = HashSet::new();
    let mut queue: VecDeque<String> = succ.get(start).cloned().unwrap_or_default().into();
    while let Some(n) = queue.pop_front() {
        if seen.insert(n.clone()) {
            queue.extend(succ.get(&n).into_iter().flatten().cloned());
        }
    }
    seen
}

fn body_ids(loop_node: &Value) -> Vec<String> {
    loop_node
        .pointer("/config/bodyNodeIds")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .filter_map(Value::as_str)
        .map(str::to_owned)
        .collect()
}

/// Which loop owns each body step; the first claim wins.
pub fn loop_body_owners(nodes: &[Value]) -> HashMap<String, String> {
    let exists: HashSet<&str> = nodes.iter().map(id).collect();
    let mut owners = HashMap::new();
    for n in nodes.iter().filter(|n| kind(n) == "loop") {
        for b in body_ids(n) {
            if exists.contains(b.as_str()) && b != id(n) && !owners.contains_key(&b) {
                owners.insert(b, id(n).to_owned());
            }
        }
    }
    owners
}

/// The run graph with each loop standing for its body.
pub fn collapse_loop_bodies(nodes: &[Value], edges: &[Value]) -> Vec<Value> {
    let owners = loop_body_owners(nodes);
    let mut seen = HashSet::new();
    let mut out = Vec::new();
    for e in edges {
        let (Some(src), Some(tgt)) = (s(e, "source"), s(e, "target")) else {
            continue;
        };
        if owners.contains_key(tgt) {
            continue;
        }
        let next = match owners.get(src) {
            Some(owner) => {
                json!({ "id": format!("{}:via-loop", id(e)), "source": owner, "target": tgt })
            }
            None => e.clone(),
        };
        let (a, b) = (
            s(&next, "source").unwrap_or(""),
            s(&next, "target").unwrap_or(""),
        );
        if a == b {
            continue;
        }
        let key = format!("{a}->{b}:{}", s(&next, "conditionBranch").unwrap_or(""));
        if seen.insert(key) {
            out.push(next);
        }
    }
    out
}

/// A loop's members and the edges between them, chained in `bodyNodeIds`
/// order when it has none.
pub fn loop_body_graph<'a>(
    nodes: &'a [Value],
    edges: &[Value],
    loop_node: &Value,
) -> (Vec<&'a Value>, Vec<Value>) {
    let owners = loop_body_owners(nodes);
    let lid = id(loop_node);
    let members: Vec<&Value> = body_ids(loop_node)
        .iter()
        .filter(|b| owners.get(*b).map(String::as_str) == Some(lid))
        .filter_map(|b| nodes.iter().find(|n| id(n) == b))
        .collect();
    let ids: HashSet<&str> = members.iter().map(|m| id(m)).collect();
    let mut inner: Vec<Value> = edges
        .iter()
        .filter(|e| {
            ids.contains(s(e, "source").unwrap_or("")) && ids.contains(s(e, "target").unwrap_or(""))
        })
        .cloned()
        .collect();
    if inner.is_empty() && members.len() > 1 {
        inner = members
            .windows(2)
            .map(|w| {
                json!({ "id": format!("{}->{}:chain", id(w[0]), id(w[1])), "source": id(w[0]), "target": id(w[1]) })
            })
            .collect();
    }
    (members, inner)
}

/// One row of the flow layout.
#[derive(Debug, Clone, PartialEq)]
pub enum FlowRow {
    Node(String),
    Fork {
        from: String,
        branches: Vec<Vec<FlowRow>>,
        join: Option<String>,
    },
    Loop {
        id: String,
        body: Vec<FlowRow>,
    },
}

fn collect_ids(rows: &[FlowRow], out: &mut HashSet<String>) {
    for r in rows {
        match r {
            FlowRow::Node(n) => {
                out.insert(n.clone());
            }
            FlowRow::Loop { id, body } => {
                out.insert(id.clone());
                collect_ids(body, out);
            }
            FlowRow::Fork { branches, .. } => {
                for b in branches {
                    collect_ids(b, out);
                }
            }
        }
    }
}

/// The trigger's chain, forks with their branches side
/// by side, loops with their bodies, then whatever nothing reaches.
pub fn compute_flow_layout(nodes: &[Value], edges: &[Value]) -> Vec<FlowRow> {
    if nodes.is_empty() {
        return Vec::new();
    }
    let by_id: HashMap<&str, &Value> = nodes.iter().map(|n| (id(n), n)).collect();
    let succ = successors(edges);
    let mut rows = Vec::new();
    match nodes.iter().find(|n| kind(n) == "trigger") {
        Some(t) => rows.extend(walk(id(t), None, &by_id, &succ, &mut HashSet::new())),
        None => {
            let has_in: HashSet<&str> = edges.iter().filter_map(|e| s(e, "target")).collect();
            let mut seen = HashSet::new();
            for root in nodes.iter().filter(|n| !has_in.contains(id(n))) {
                rows.extend(walk(id(root), None, &by_id, &succ, &mut seen));
            }
        }
    }
    let mut visited = HashSet::new();
    collect_ids(&rows, &mut visited);
    for n in nodes {
        if !visited.contains(id(n)) {
            rows.push(FlowRow::Node(id(n).to_owned()));
        }
    }
    rows
}

/// The steps from `start` down to `stop`, laid out as a flow.
fn walk(
    start: &str,
    stop: Option<&str>,
    by_id: &HashMap<&str, &Value>,
    succ: &HashMap<String, Vec<String>>,
    seen: &mut HashSet<String>,
) -> Vec<FlowRow> {
    let mut rows = Vec::new();
    let mut cur = Some(start.to_owned());
    while let Some(c) = cur.take() {
        if Some(c.as_str()) == stop || seen.contains(&c) {
            break;
        }
        seen.insert(c.clone());
        let Some(node) = by_id.get(c.as_str()) else {
            break;
        };
        let next: Vec<String> = succ.get(&c).cloned().unwrap_or_default();
        if kind(node) == "loop" {
            let body: Vec<String> = body_ids(node)
                .into_iter()
                .filter(|b| by_id.contains_key(b.as_str()))
                .collect();
            for b in &body {
                seen.insert(b.clone());
            }
            rows.push(FlowRow::Loop {
                id: c.clone(),
                body: body.iter().map(|b| FlowRow::Node(b.clone())).collect(),
            });
            let mut n = next.first().cloned();
            while let Some(x) = n.clone().filter(|x| body.contains(x)) {
                n = succ.get(&x).and_then(|v| v.first()).cloned();
            }
            cur = n;
            continue;
        }
        if next.len() <= 1 {
            rows.push(FlowRow::Node(c.clone()));
            cur = next.first().cloned();
        } else {
            rows.push(FlowRow::Node(c.clone()));
            let join = find_join(&next, succ);
            let branches = next
                .iter()
                .map(|child| {
                    let mut own = seen.clone();
                    walk(child, join.as_deref(), by_id, succ, &mut own)
                })
                .collect();
            rows.push(FlowRow::Fork {
                from: c.clone(),
                branches,
                join: join.clone(),
            });
            cur = join;
        }
    }
    rows
}

/// Nobody has arranged the canvas (every x is 0).
pub fn positions_are_seed(nodes: &[Value]) -> bool {
    nodes.iter().all(|n| {
        n.pointer("/position/x")
            .and_then(Value::as_f64)
            .unwrap_or(0.0)
            == 0.0
    })
}

/// A coordinate snapped to the lattice, halves rounding up.
pub fn snap(v: f64) -> f64 {
    let r = (v / GRID + 0.5).floor() * GRID;
    if r == 0.0 {
        0.0
    } else {
        r
    }
}

/// A node's stored position.
pub fn position(n: &Value) -> (f64, f64) {
    (
        n.pointer("/position/x")
            .and_then(Value::as_f64)
            .unwrap_or(0.0),
        n.pointer("/position/y")
            .and_then(Value::as_f64)
            .unwrap_or(0.0),
    )
}

fn set_position(n: &mut Value, (x, y): (f64, f64)) {
    n["position"] = json!({ "x": num(x), "y": num(y) });
}

/// Whole numbers written as integers, as stored definitions have them.
pub fn num(v: f64) -> Value {
    if v.fract() == 0.0 && v.abs() < 9.0e15 {
        json!(v as i64)
    } else {
        json!(v)
    }
}

/// A node's width on the canvas: wider for a loop.
pub fn node_width(n: Option<&Value>) -> f64 {
    if n.is_some_and(|n| kind(n) == "loop") {
        LOOP_WIDTH
    } else {
        CARD_WIDTH
    }
}

/// Breadth first from the trigger, one pitch apart in
/// the seed column.
pub fn auto_layout(nodes: &[Value], edges: &[Value]) -> Vec<Value> {
    if nodes.is_empty() {
        return Vec::new();
    }
    let has_in: HashSet<&str> = edges.iter().filter_map(|e| s(e, "target")).collect();
    let mut ordered: Vec<String> = match nodes.iter().find(|n| kind(n) == "trigger") {
        Some(t) => vec![id(t).to_owned()],
        None => nodes
            .iter()
            .filter(|n| !has_in.contains(id(n)))
            .map(|n| id(n).to_owned())
            .collect(),
    };
    let succ = successors(edges);
    let exists: HashSet<&str> = nodes.iter().map(id).collect();
    let mut visited: HashSet<String> = ordered.iter().cloned().collect();
    let mut queue: VecDeque<String> = ordered.iter().cloned().collect();
    while let Some(cur) = queue.pop_front() {
        for child in succ.get(&cur).into_iter().flatten() {
            if visited.insert(child.clone()) {
                queue.push_back(child.clone());
                if exists.contains(child.as_str()) {
                    ordered.push(child.clone());
                }
            }
        }
    }
    for n in nodes {
        if !visited.contains(id(n)) {
            ordered.push(id(n).to_owned());
        }
    }
    ordered
        .iter()
        .enumerate()
        .filter_map(|(i, nid)| {
            let mut n = nodes.iter().find(|n| id(n) == nid)?.clone();
            set_position(&mut n, (0.0, i as f64 * PITCH));
            Some(n)
        })
        .collect()
}

/// A seed canvas reflows; an arranged one keeps its
/// positions and places only the new steps, pushing down what they cover.
pub fn place_new(prev: &[Value], next: Vec<Value>, edges: &[Value]) -> Vec<Value> {
    if positions_are_seed(prev) {
        return auto_layout(&next, edges);
    }
    let prev_ids: HashSet<&str> = prev.iter().map(id).collect();
    let mut placed: Vec<(String, (f64, f64))> = next
        .iter()
        .filter(|n| prev_ids.contains(id(n)))
        .map(|n| (id(n).to_owned(), position(n)))
        .collect();
    let get = |placed: &[(String, (f64, f64))], k: &str| {
        placed.iter().find(|(i, _)| i == k).map(|(_, p)| *p)
    };
    let incoming = |nid: &str| edges.iter().find(|e| s(e, "target") == Some(nid));
    let mut siblings: HashMap<String, usize> = HashMap::new();
    let mut remaining: Vec<&Value> = next.iter().filter(|n| !prev_ids.contains(id(n))).collect();
    let new_ids: Vec<String> = remaining.iter().map(|n| id(n).to_owned()).collect();
    while !remaining.is_empty() {
        let ready = remaining.iter().position(|n| match incoming(id(n)) {
            None => true,
            Some(e) => get(&placed, s(e, "source").unwrap_or("")).is_some(),
        });
        let node = remaining.remove(ready.unwrap_or(0));
        let inc = incoming(id(node));
        let src = inc.and_then(|e| s(e, "source"));
        let base = src.and_then(|p| get(&placed, p)).unwrap_or((0.0, 0.0));
        let sibling = match src {
            Some(p) => {
                let c = siblings.entry(p.to_owned()).or_insert(0);
                let v = *c;
                *c += 1;
                v
            }
            None => 0,
        };
        let parent = src.and_then(|p| next.iter().find(|n| id(n) == p));
        let centering = (node_width(parent) - node_width(Some(node))) / 2.0;
        let mut cand = (base.0 + centering + sibling as f64 * 320.0, base.1 + PITCH);
        let downstream = reachable_from(id(node), edges);
        let occupied = |placed: &[(String, (f64, f64))], p: (f64, f64)| {
            placed.iter().any(|(i, o)| {
                !downstream.contains(i) && (o.0 - p.0).abs() < 300.0 && (o.1 - p.1).abs() < 120.0
            })
        };
        let mut tries = 0;
        while occupied(&placed, cand) && tries < 50 {
            cand.1 += PITCH;
            tries += 1;
        }
        placed.push((id(node).to_owned(), cand));
    }
    let mut shifted = HashSet::new();
    for nid in &new_ids {
        let Some(np) = get(&placed, nid) else {
            continue;
        };
        for r in reachable_from(nid, edges) {
            if new_ids.contains(&r) {
                continue;
            }
            if get(&placed, &r).is_some_and(|p| p.1 <= np.1 + 100.0) {
                shifted.insert(r);
            }
        }
    }
    next.into_iter()
        .map(|mut n| {
            let nid = id(&n).to_owned();
            if new_ids.contains(&nid) {
                if let Some(p) = get(&placed, &nid) {
                    set_position(&mut n, p);
                }
            } else if shifted.contains(&nid) {
                let (x, y) = position(&n);
                set_position(&mut n, (x, y + PITCH));
            }
            n
        })
        .collect()
}

/// The result of a structural edit.
pub type Graph = (Vec<Value>, Vec<Value>);

/// The edge split, its branch tag on the first half.
pub fn insert_between(nodes: &[Value], edges: &[Value], edge_id: &str, new: Value) -> Graph {
    let Some(old) = edges.iter().find(|e| id(e) == edge_id) else {
        return (nodes.to_vec(), edges.to_vec());
    };
    let nid = id(&new).to_owned();
    let mut out: Vec<Value> = edges.iter().filter(|e| id(e) != edge_id).cloned().collect();
    out.push(edge(
        s(old, "source").unwrap_or(""),
        &nid,
        s(old, "conditionBranch"),
    ));
    out.push(edge(&nid, s(old, "target").unwrap_or(""), None));
    let mut next = nodes.to_vec();
    next.push(new);
    (place_new(nodes, next, &out), out)
}

/// `new` appended after `after`, joined by an edge.
pub fn append_after(nodes: &[Value], edges: &[Value], after: &str, new: Value) -> Graph {
    let mut out = edges.to_vec();
    out.push(edge(after, id(&new), None));
    let mut next = nodes.to_vec();
    next.push(new);
    (place_new(nodes, next, &out), out)
}

/// The new step takes over the fork's branches.
pub fn insert_before_fork(nodes: &[Value], edges: &[Value], fork: &str, new: Value) -> Graph {
    let nid = id(&new).to_owned();
    let mut out: Vec<Value> = edges
        .iter()
        .map(|e| {
            if s(e, "source") == Some(fork) {
                json!({ "id": new_id(), "source": nid, "target": s(e, "target").unwrap_or("") })
            } else {
                e.clone()
            }
        })
        .collect();
    out.push(edge(fork, &nid, None));
    let mut next = nodes.to_vec();
    next.push(new);
    (place_new(nodes, next, &out), out)
}

/// A sibling of `from`'s successors that rejoins
/// where they do.
pub fn add_parallel_branch(nodes: &[Value], edges: &[Value], from: &str, new: Value) -> Graph {
    let succ = successors(edges);
    let kids = succ.get(from).cloned().unwrap_or_default();
    let nid = id(&new).to_owned();
    let mut out = edges.to_vec();
    out.push(edge(from, &nid, None));
    match kids.len() {
        0 => {}
        1 => {
            let mut join_target = None;
            let mut cur = kids[0].clone();
            loop {
                let s2 = succ.get(&cur).cloned().unwrap_or_default();
                match s2.len() {
                    0 => break,
                    1 => {
                        join_target = Some(s2[0].clone());
                        break;
                    }
                    _ => match find_join(&s2, &succ) {
                        Some(j) => cur = j,
                        None => break,
                    },
                }
            }
            if let Some(j) = join_target {
                out.push(edge(&nid, &j, None));
            }
        }
        _ => {
            if let Some(j) = find_join(&kids, &succ) {
                out.push(edge(&nid, &j, None));
            }
        }
    }
    let mut next = nodes.to_vec();
    next.push(new);
    (place_new(nodes, next, &out), out)
}

/// A condition whose placeholder branches rejoin
/// at `before`.
pub fn insert_condition(
    nodes: &[Value],
    edges: &[Value],
    after: &str,
    before: Option<&str>,
    slug_taken: &HashSet<String>,
) -> Graph {
    let cond = condition_node();
    let cid = id(&cond).to_owned();
    let mut next = nodes.to_vec();
    next.push(cond);
    let mut out = edges.to_vec();
    match before {
        Some(b) => {
            let mut branch = |label: &str, content: &str| {
                let mut n = script_node(slug_taken);
                n["label"] = json!(label);
                n["slug"] = json!(crate::workflows::slugify(label));
                n["config"]["scriptContent"] = json!(content);
                let nid = id(&n).to_owned();
                next.push(n);
                nid
            };
            let t = branch("True Branch", "# True branch\nexit 0\n");
            let f = branch("False Branch", "# False branch\nexit 0\n");
            out.retain(|e| !(s(e, "source") == Some(after) && s(e, "target") == Some(b)));
            out.push(edge(after, &cid, None));
            out.push(edge(&cid, &t, Some("true")));
            out.push(edge(&cid, &f, Some("false")));
            out.push(edge(&t, b, None));
            out.push(edge(&f, b, None));
        }
        None => out.push(edge(after, &cid, None)),
    }
    (place_new(nodes, next, &out), out)
}

fn without_members(nodes: Vec<Value>, gone: &HashSet<String>) -> Vec<Value> {
    nodes
        .into_iter()
        .map(|mut n| {
            if kind(&n) == "loop" {
                if let Some(body) = n
                    .pointer_mut("/config/bodyNodeIds")
                    .and_then(Value::as_array_mut)
                {
                    body.retain(|b| !gone.contains(b.as_str().unwrap_or("")));
                }
            }
            n
        })
        .collect()
}

/// A condition takes its branches up to their join; any
/// other step is bridged.
pub fn remove(nodes: &[Value], edges: &[Value], nid: &str) -> Graph {
    let target = nodes.iter().find(|n| id(n) == nid);
    if target.is_some_and(|n| kind(n) == "condition") {
        let succ = successors(edges);
        let starts = succ.get(nid).cloned().unwrap_or_default();
        let join = find_join(&starts, &succ);
        let mut gone: HashSet<String> = HashSet::new();
        let mut queue: VecDeque<String> = starts.into();
        while let Some(cur) = queue.pop_front() {
            if Some(&cur) == join.as_ref() || !gone.insert(cur.clone()) {
                continue;
            }
            for n in succ.get(&cur).into_iter().flatten() {
                if Some(n) != join.as_ref() {
                    queue.push_back(n.clone());
                }
            }
        }
        gone.insert(nid.to_owned());
        let preds: Vec<String> = edges
            .iter()
            .filter(|e| s(e, "target") == Some(nid))
            .filter_map(|e| s(e, "source").map(str::to_owned))
            .collect();
        let mut out: Vec<Value> = edges
            .iter()
            .filter(|e| {
                !gone.contains(s(e, "source").unwrap_or(""))
                    && !gone.contains(s(e, "target").unwrap_or(""))
            })
            .cloned()
            .collect();
        if let Some(j) = &join {
            for p in &preds {
                out.push(edge(p, j, None));
            }
        }
        let rest = without_members(
            nodes
                .iter()
                .filter(|n| !gone.contains(id(n)))
                .cloned()
                .collect(),
            &gone,
        );
        return (place_new(nodes, rest, &out), out);
    }
    let ins: Vec<&Value> = edges
        .iter()
        .filter(|e| s(e, "target") == Some(nid))
        .collect();
    let outs: Vec<&Value> = edges
        .iter()
        .filter(|e| s(e, "source") == Some(nid))
        .collect();
    let mut out: Vec<Value> = edges
        .iter()
        .filter(|e| s(e, "source") != Some(nid) && s(e, "target") != Some(nid))
        .cloned()
        .collect();
    for i in &ins {
        for o in &outs {
            out.push(edge(
                s(i, "source").unwrap_or(""),
                s(o, "target").unwrap_or(""),
                None,
            ));
        }
    }
    let gone = HashSet::from([nid.to_owned()]);
    let rest = without_members(
        nodes.iter().filter(|n| id(n) != nid).cloned().collect(),
        &gone,
    );
    (place_new(nodes, rest, &out), out)
}

/// The edge and the membership written together.
pub fn append_to_loop_body(
    nodes: &[Value],
    edges: &[Value],
    loop_id: &str,
    new: Value,
    after: Option<&str>,
) -> Graph {
    let Some(lp) = nodes.iter().find(|n| id(n) == loop_id && kind(n) == "loop") else {
        return (nodes.to_vec(), edges.to_vec());
    };
    let body = body_ids(lp);
    let last = match after {
        Some(a) if body.iter().any(|b| b == a) => a.to_owned(),
        _ => body.last().cloned().unwrap_or_else(|| loop_id.to_owned()),
    };
    let nid = id(&new).to_owned();
    let onward: Vec<Value> = edges
        .iter()
        .filter(|e| s(e, "source") == Some(last.as_str()))
        .cloned()
        .collect();
    let mut out: Vec<Value> = edges
        .iter()
        .filter(|e| s(e, "source") != Some(last.as_str()))
        .cloned()
        .collect();
    out.push(edge(&last, &nid, None));
    for e in onward {
        let mut e2 = e.clone();
        e2["id"] = json!(new_id());
        e2["source"] = json!(nid);
        out.push(e2);
    }
    let mut next: Vec<Value> = nodes
        .iter()
        .map(|n| {
            let mut n = n.clone();
            if id(&n) == loop_id {
                let mut b = body.clone();
                b.push(nid.clone());
                n["config"]["bodyNodeIds"] = json!(b);
            }
            n
        })
        .collect();
    next.push(new);
    (place_new(nodes, next, &out), out)
}

/// The loop itself, or the loop a body step is in.
pub fn loop_owning<'a>(nodes: &'a [Value], after: &str) -> Option<&'a Value> {
    nodes
        .iter()
        .find(|n| id(n) == after && kind(n) == "loop")
        .or_else(|| {
            nodes
                .iter()
                .find(|n| kind(n) == "loop" && body_ids(n).iter().any(|b| b == after))
        })
}

/// Steps inserted inside a body join it.
pub fn adopt_into_loop_body(
    before: &[Value],
    result: Graph,
    after: &str,
    before_id: Option<&str>,
) -> Graph {
    let owners = loop_body_owners(before);
    let owner_of = |x: Option<&str>| x.and_then(|x| owners.get(x)).cloned();
    let after_owner = owner_of(Some(after));
    let before_owner = owner_of(before_id);
    let lp: Option<String> = if before_id == Some(LOOP_BODY) {
        loop_owning(before, after).map(|l| id(l).to_owned())
    } else if before_owner.is_some()
        && (before_owner == after_owner || before_owner.as_deref() == Some(after))
    {
        before_owner
    } else if before_id.is_none() {
        after_owner
    } else {
        None
    };
    let Some(lp) = lp else {
        return result;
    };
    let existing: HashSet<&str> = before.iter().map(id).collect();
    let added: Vec<String> = result
        .0
        .iter()
        .map(id)
        .filter(|i| !existing.contains(i))
        .map(str::to_owned)
        .collect();
    if added.is_empty() {
        return result;
    }
    let (nodes, edges) = result;
    let nodes = nodes
        .into_iter()
        .map(|mut n| {
            if id(&n) == lp {
                let mut b = body_ids(&n);
                for a in &added {
                    if !b.contains(a) {
                        b.push(a.clone());
                    }
                }
                n["config"]["bodyNodeIds"] = json!(b);
            }
            n
        })
        .collect();
    (nodes, edges)
}

/// What a loop may repeat, everything downstream of it.
pub fn nodes_after<'a>(nodes: &'a [Value], edges: &[Value], nid: &str) -> Vec<&'a Value> {
    let succ = successors(edges);
    let mut seen = HashSet::from([nid.to_owned()]);
    let mut queue: VecDeque<String> = succ.get(nid).cloned().unwrap_or_default().into();
    let mut out = Vec::new();
    while let Some(c) = queue.pop_front() {
        if !seen.insert(c.clone()) {
            continue;
        }
        if let Some(n) = nodes.iter().find(|n| id(n) == c) {
            if kind(n) != "trigger" {
                out.push(n);
            }
        }
        queue.extend(succ.get(&c).into_iter().flatten().cloned());
    }
    out
}

/// Whether a step of node type `kind` can be swapped for another type.
pub fn replaceable(kind: &str) -> bool {
    matches!(
        kind,
        "launchAgent"
            | "script"
            | "approval"
            | "createTaskFromItem"
            | "callConnectorAction"
            | "httpRequest"
    )
}

fn node(kind: &str, label: &str, config: Value) -> Value {
    json!({
        "id": new_id(),
        "type": kind,
        "label": label,
        "slug": crate::workflows::slugify(label),
        "config": config,
        "position": { "x": 0, "y": 0 },
    })
}

fn unique(mut n: Value, taken: &HashSet<String>) -> Value {
    if let Some(slug) = s(&n, "slug").map(str::to_owned) {
        n["slug"] = json!(crate::workflows::unique_slug(&slug, taken));
    }
    n
}

/// A step of node type `kind` an extension declares, its config empty until its form fills it.
pub fn contributed_node(kind: &str, taken: &HashSet<String>) -> Value {
    unique(node(kind, kind, json!({})), taken)
}

/// A new agent step in `project`, running `agent`.
pub fn agent_node(project: Option<(&str, &str)>, agent: &str, taken: &HashSet<String>) -> Value {
    let (name, path) = project.unwrap_or(("", ""));
    unique(
        node(
            "launchAgent",
            "Launch Agent",
            json!({ "agentType": agent, "projectName": name, "projectPath": path, "headless": true }),
        ),
        taken,
    )
}

/// A new script step.
pub fn script_node(taken: &HashSet<String>) -> Value {
    unique(
        node(
            "script",
            "Execute Script",
            json!({ "scriptType": "bash", "scriptContent": "# Write your script here\n", "projectName": "", "projectPath": "" }),
        ),
        taken,
    )
}

/// A new approval gate.
pub fn approval_node(taken: &HashSet<String>) -> Value {
    unique(
        node("approval", "Approval Gate", json!({ "message": "" })),
        taken,
    )
}

/// A new loop.
pub fn loop_node(taken: &HashSet<String>) -> Value {
    unique(
        node(
            "loop",
            "Repeat Steps",
            json!({ "nodeType": "loop", "bodyNodeIds": [], "maxIterations": 2 }),
        ),
        taken,
    )
}

/// A new connector action step.
pub fn connector_action_node(taken: &HashSet<String>) -> Value {
    unique(
        node(
            "callConnectorAction",
            "Connector Action",
            json!({ "nodeType": "callConnectorAction", "connectionId": "", "action": "", "args": {} }),
        ),
        taken,
    )
}

/// A new HTTP request step.
pub fn http_node(taken: &HashSet<String>) -> Value {
    unique(
        node(
            "httpRequest",
            "HTTP Request",
            json!({ "nodeType": "httpRequest", "method": "GET", "url": "", "headers": {}, "body": "" }),
        ),
        taken,
    )
}

/// A new condition (its slug made unique by the caller's next edit).
pub fn condition_node() -> Value {
    node(
        "condition",
        "Condition",
        json!({ "variable": "", "operator": "equals", "value": "" }),
    )
}

/// A new trigger, its label by kind.
pub fn trigger_node(config: Value) -> Value {
    let label = match s(&config, "triggerType").unwrap_or("") {
        "manual" => "Manual Trigger",
        "once" => "Schedule (Once)",
        "recurring" => "Schedule (Recurring)",
        "taskCreated" => "When Task Created",
        "taskStatusChanged" => "When Task Status Changes",
        "connectorPoll" => "Connector Poll",
        "webhook" => "Webhook",
        _ => "Trigger",
    };
    json!({
        "id": new_id(),
        "type": "trigger",
        "label": label,
        "config": config,
        "position": { "x": 0, "y": 0 },
    })
}

/// A trigger kind's starting config.
pub fn trigger_config(kind: &str, now_iso: &str) -> Value {
    match kind {
        "once" => json!({ "triggerType": "once", "runAt": now_iso }),
        "recurring" => json!({ "triggerType": "recurring", "cron": "0 9 * * *" }),
        "connectorPoll" => {
            json!({ "triggerType": "connectorPoll", "connectionId": "", "event": "", "cron": "*/5 * * * *" })
        }
        "webhook" => json!({ "triggerType": "webhook", "method": "POST", "token": new_id() }),
        other => json!({ "triggerType": other }),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn n(i: &str, k: &str) -> Value {
        json!({ "id": i, "type": k, "label": i, "config": {}, "position": { "x": 0, "y": 0 } })
    }

    fn e(a: &str, b: &str) -> Value {
        json!({ "id": format!("{a}-{b}"), "source": a, "target": b })
    }

    fn ends(edges: &[Value]) -> Vec<(String, String)> {
        let mut v: Vec<(String, String)> = edges
            .iter()
            .map(|e| {
                (
                    s(e, "source").unwrap().to_owned(),
                    s(e, "target").unwrap().to_owned(),
                )
            })
            .collect();
        v.sort();
        v
    }

    #[test]
    fn a_fork_lays_out_its_branches_until_they_join() {
        let nodes = [
            n("t", "trigger"),
            n("a", "script"),
            n("b", "script"),
            n("c", "script"),
            n("j", "script"),
        ];
        let edges = [
            e("t", "a"),
            e("a", "b"),
            e("a", "c"),
            e("b", "j"),
            e("c", "j"),
        ];
        let rows = compute_flow_layout(&nodes, &edges);
        assert_eq!(
            rows,
            vec![
                FlowRow::Node("t".into()),
                FlowRow::Node("a".into()),
                FlowRow::Fork {
                    from: "a".into(),
                    branches: vec![
                        vec![FlowRow::Node("b".into())],
                        vec![FlowRow::Node("c".into())]
                    ],
                    join: Some("j".into()),
                },
                FlowRow::Node("j".into()),
            ]
        );
    }

    #[test]
    fn a_loop_holds_its_body_and_the_trunk_resumes_after_it() {
        let mut lp = n("l", "loop");
        lp["config"]["bodyNodeIds"] = json!(["x", "y"]);
        let nodes = [
            n("t", "trigger"),
            lp,
            n("x", "script"),
            n("y", "script"),
            n("z", "script"),
        ];
        let edges = [e("t", "l"), e("l", "x"), e("x", "y"), e("y", "z")];
        let rows = compute_flow_layout(&nodes, &edges);
        assert_eq!(rows.len(), 3);
        assert!(matches!(&rows[1], FlowRow::Loop { id, body } if id == "l" && body.len() == 2));
        assert_eq!(rows[2], FlowRow::Node("z".into()));
        let collapsed = collapse_loop_bodies(&nodes, &edges);
        assert_eq!(
            ends(&collapsed),
            [("l".into(), "z".into()), ("t".into(), "l".into())]
        );
    }

    #[test]
    fn a_cycle_draws_as_a_truncated_chain() {
        let nodes = [n("t", "trigger"), n("a", "script")];
        let edges = [e("t", "a"), e("a", "t")];
        assert_eq!(compute_flow_layout(&nodes, &edges).len(), 2);
    }

    #[test]
    fn inserting_splits_the_edge_and_a_seed_canvas_reflows() {
        let nodes = [n("t", "trigger"), n("b", "script")];
        let edges = [e("t", "b")];
        let (nodes, edges) = insert_between(&nodes, &edges, "t-b", n("m", "script"));
        assert_eq!(
            ends(&edges),
            [("m".into(), "b".into()), ("t".into(), "m".into())]
        );
        let ys: Vec<(String, f64)> = nodes
            .iter()
            .map(|n| (id(n).to_owned(), position(n).1))
            .collect();
        // The layout hands the nodes back in walk order.
        assert_eq!(
            ys,
            [("t".into(), 0.0), ("m".into(), 144.0), ("b".into(), 288.0)]
        );
    }

    #[test]
    fn an_arranged_canvas_places_the_new_step_and_pushes_down_what_it_covers() {
        let mut t = n("t", "trigger");
        t["position"] = json!({ "x": 16, "y": 0 });
        let mut b = n("b", "script");
        b["position"] = json!({ "x": 16, "y": 144 });
        let nodes = [t, b];
        let (nodes, _) = insert_between(&nodes, &[e("t", "b")], "t-b", n("m", "script"));
        let at = |k: &str| position(nodes.iter().find(|n| id(n) == k).unwrap());
        assert_eq!(at("m"), (16.0, 144.0));
        assert_eq!(at("b"), (16.0, 288.0));
    }

    #[test]
    fn removing_a_condition_takes_its_branches_and_bridges_to_the_join() {
        let nodes = [
            n("t", "trigger"),
            n("c", "condition"),
            n("x", "script"),
            n("y", "script"),
            n("j", "script"),
        ];
        let edges = [
            e("t", "c"),
            e("c", "x"),
            e("c", "y"),
            e("x", "j"),
            e("y", "j"),
        ];
        let (nodes, edges) = remove(&nodes, &edges, "c");
        assert_eq!(nodes.iter().map(id).collect::<Vec<_>>(), ["t", "j"]);
        assert_eq!(ends(&edges), [("t".into(), "j".into())]);
    }

    #[test]
    fn a_parallel_branch_rejoins_where_the_chain_continues() {
        let nodes = [n("a", "script"), n("b", "script"), n("c", "script")];
        let edges = [e("a", "b"), e("b", "c")];
        let (_, edges) = add_parallel_branch(&nodes, &edges, "a", n("p", "script"));
        assert!(ends(&edges).contains(&("p".into(), "c".into())));
        assert!(ends(&edges).contains(&("a".into(), "p".into())));
    }

    #[test]
    fn steps_added_inside_a_loop_join_its_body() {
        let lp = n("l", "loop");
        let nodes = [n("t", "trigger"), lp, n("z", "script")];
        let edges = [e("t", "l"), e("l", "z")];
        let (nodes2, edges2) = append_to_loop_body(&nodes, &edges, "l", n("x", "script"), None);
        let l = nodes2.iter().find(|n| id(n) == "l").unwrap();
        assert_eq!(body_ids(l), ["x"]);
        assert!(ends(&edges2).contains(&("x".into(), "z".into())));
        // An insert on an edge between two body steps is adopted too.
        let result = append_after(&nodes2, &edges2, "x", n("y", "script"));
        let adopted = adopt_into_loop_body(&nodes2, result, "x", None);
        let l = adopted.0.iter().find(|n| id(n) == "l").unwrap();
        assert_eq!(body_ids(l), ["x", "y"]);
    }

    #[test]
    fn lattice_and_slugs() {
        assert_eq!(snap(-140.0), -136.0);
        assert_eq!(snap(-156.0), -152.0);
        assert_eq!(snap(3.0), 0.0);
        let taken = HashSet::from(["execute_script".to_owned()]);
        assert_eq!(s(&script_node(&taken), "slug"), Some("execute_script_2"));
    }
}

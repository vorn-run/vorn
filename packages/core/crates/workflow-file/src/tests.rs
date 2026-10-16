use serde_json::{json, Value};

use super::*;

fn none() -> Bindings<'static> {
    Bindings::default()
}

fn back(def: &Value, b: &Bindings<'_>) -> Value {
    let text = write(def, b);
    match read(&text, b) {
        Ok(r) => r.workflow,
        Err(e) => panic!("{text}\n{e:?}"),
    }
}

/// A stored workflow with the runtime fields the file leaves out removed.
fn stored(mut def: Value) -> Value {
    if let Some(map) = def.as_object_mut() {
        for k in RUNTIME_FIELDS {
            map.shift_remove(k);
        }
    }
    def
}

fn sample() -> Value {
    json!({
        "id": "review", "name": "Review a pull request", "icon": "GitPullRequest",
        "iconColor": "#6366f1", "enabled": true, "staggerDelayMs": 250, "workspaceId": "personal",
        "lastRunAt": "2026-10-01T00:00:00.000Z", "lastRunStatus": "success",
        "nodes": [
            { "id": "start", "type": "trigger", "label": "Manual", "position": { "x": 0, "y": 0 },
              "config": { "triggerType": "manual", "inputs": [{ "key": "pr", "label": "PR", "type": "text", "required": true }] } },
            { "id": "fetch", "type": "script", "label": "Fetch", "slug": "fetch", "position": { "x": 0, "y": 110.5 },
              "config": { "scriptType": "bash", "scriptContent": "gh pr diff {{inputs.pr}}\n  indented\n\n  \ttrailing  \nlast" } },
            { "id": "big", "type": "condition", "label": "Big?", "position": { "x": 0, "y": 220 },
              "config": { "variable": "{{steps.fetch.output}}", "operator": "isNotEmpty", "value": "" } },
            { "id": "review", "type": "launchAgent", "label": "Review", "slug": "review", "onError": "continue",
              "position": { "x": 0, "y": 330 }, "config": { "agentType": "claude", "headless": true, "prompt": "Review \"this\"\n{{steps.fetch.output}}\n" } },
            { "id": "skip", "type": "script", "label": "Skip", "position": { "x": 200, "y": 330 }, "config": {} },
            { "id": "each", "type": "loop", "label": "Each", "config": { "mode": "forEach", "items": "[1,2]", "bodyNodeIds": ["body"], "maxIterations": 2 } },
            { "id": "body", "type": "script", "label": "Body", "config": { "scriptType": "bash", "scriptContent": "echo {{loop.item}}" } }
        ],
        "edges": [
            { "id": "start-fetch", "source": "start", "target": "fetch" },
            { "id": "fetch-big", "source": "fetch", "target": "big" },
            { "id": "e3", "source": "big", "target": "review", "conditionBranch": "true" },
            { "id": "big-skip", "source": "big", "target": "skip", "conditionBranch": "false" },
            { "id": "review-each", "source": "review", "target": "each" },
            { "id": "each-body", "source": "each", "target": "body" }
        ]
    })
}

#[test]
fn a_workflow_reads_back_as_it_was_written() {
    let def = sample();
    assert_eq!(back(&def, &none()), stored(def));
}

#[test]
fn a_step_after_the_one_before_says_nothing_and_a_branch_says_where_from() {
    let text = write(&sample(), &none());
    assert!(text.contains("step \"script\" id=\"fetch\""), "{text}");
    assert!(
        !text.contains("id=\"fetch\" label=\"Fetch\" slug=\"fetch\" x=0 y=110.5 needs"),
        "{text}"
    );
    assert!(text.contains("needs=\"big:true#e3\""), "{text}");
    assert!(text.contains("needs=\"big:false\""), "{text}");
    assert!(text.contains("prompt \"\"\""), "{text}");
    assert!(!text.contains("lastRunAt"), "{text}");
}

#[test]
fn edges_keep_their_order_and_odd_edges_are_written_whole() {
    let mut def = sample();
    let edges = def["edges"].as_array_mut().unwrap();
    edges.reverse();
    edges.push(json!({ "id": "x y", "source": "fetch", "target": "skip", "weight": 2 }));
    edges.push(json!({ "source": "skip", "target": "body" }));
    let text = write(&def, &none());
    assert!(text.contains("edge-order"), "{text}");
    assert!(text.contains("edge from=\"skip\" to=\"body\""), "{text}");
    assert_eq!(back(&def, &none()), stored(def));
}

#[test]
fn connections_go_by_name_and_paths_by_their_place_in_the_project() {
    let conns = [
        ("c-1".to_owned(), "workspace-eng".to_owned()),
        ("c-2".to_owned(), "twin".to_owned()),
        ("c-3".to_owned(), "twin".to_owned()),
    ];
    let here = Bindings {
        connections: &conns,
        project: Some(("app", "/home/me/app")),
    };
    let def = json!({ "id": "w", "name": "W", "nodes": [
        { "id": "t", "type": "trigger", "config": { "triggerType": "connectorPoll", "connectionId": "c-1" } },
        { "id": "a", "type": "callConnectorAction", "config": { "connectionId": "c-2", "action": "x",
          "projectPath": "/home/me/app", "cwd": "/home/me/app/src", "projectName": "app", "other": "/elsewhere" } }
    ], "edges": [{ "id": "t-a", "source": "t", "target": "a" }] });
    let text = write(&def, &here);
    assert!(
        text.contains("connectionId (connection)\"workspace-eng\""),
        "{text}"
    );
    // Two connections share the name: the id stays, so nothing is lost.
    assert!(text.contains("connectionId \"c-2\""), "{text}");
    assert!(text.contains("projectPath (project)\".\""), "{text}");
    assert!(text.contains("cwd (project)\"src\""), "{text}");
    assert!(text.contains("projectName (project-name)\"app\""), "{text}");
    assert_eq!(back(&def, &here), def);

    // The same file on another machine, in another place, with its own connection of that name.
    let theirs = [("theirs".to_owned(), "workspace-eng".to_owned())];
    let there = Bindings {
        connections: &theirs,
        project: Some(("app", "/srv/app")),
    };
    let read_there = read(&text, &there).unwrap();
    assert_eq!(
        read_there.workflow["nodes"][0]["config"]["connectionId"],
        "theirs"
    );
    assert_eq!(
        read_there.workflow["nodes"][1]["config"]["cwd"],
        "/srv/app/src"
    );
    let alone = read(&text, &Bindings::default()).unwrap();
    assert_eq!(alone.warnings.len(), 1);
    assert_eq!(
        alone.warnings[0].message,
        "no connection here is called workspace-eng"
    );
}

#[test]
fn errors_name_their_line_and_column() {
    let text = "workflow \"W\" {\n    step \"script\" id=\"a\" lable=\"A\"\n    step \"x\" id=\"b\" needs=\"nowhere\"\n}\n";
    let errors = read(text, &none()).unwrap_err();
    let shown: Vec<String> = errors.iter().map(ToString::to_string).collect();
    assert_eq!(
        shown,
        [
            "2:26: unknown property lable=; did you mean label=?",
            "3:5: needs `nowhere`, which no step here is",
        ]
    );
    let bad = read("workflow \"W\" {\n  step \"a\" {\n}\n", &none()).unwrap_err();
    assert!(bad[0].at.0 >= 1);
    let not = read("flow \"W\" {}\n", &none()).unwrap_err();
    assert_eq!(
        not[0].to_string(),
        "1:1: `flow`: a workflow file starts with `workflow \"Its name\" {`"
    );
}

#[test]
fn an_edit_keeps_the_comments_of_what_did_not_change() {
    let def = sample();
    let mut text = write(&def, &none());
    text = text.replace(
        "    step \"script\" id=\"fetch\"",
        "    // Reads the diff first.\n    step \"script\" id=\"fetch\"",
    );
    text = text.replace(
        "        variable \"{{steps.fetch.output}}\"",
        "        // The diff, as fetched.\n        variable \"{{steps.fetch.output}}\"",
    );
    text = format!("// Kept on top.\n{text}// And below.\n");
    let mut changed = def.clone();
    changed["nodes"][2]["config"]["operator"] = json!("contains");
    changed["nodes"][1]["label"] = json!("Fetch the diff");
    changed["name"] = json!("Review it");
    let updated = update(&text, &changed, &none());
    for kept in [
        "// Kept on top.",
        "// Reads the diff first.",
        "// The diff, as fetched.",
        "// And below.",
    ] {
        assert!(updated.contains(kept), "{kept} lost:\n{updated}");
    }
    assert!(updated.contains("operator \"contains\""), "{updated}");
    assert_eq!(
        read(&updated, &none()).unwrap().workflow,
        stored(changed.clone())
    );

    // A step taken out takes its comment with it; one put in is written where it goes.
    let mut fewer = changed.clone();
    fewer["nodes"].as_array_mut().unwrap().remove(1);
    fewer["edges"] = json!([
        { "id": "start-big", "source": "start", "target": "big" },
        { "id": "e3", "source": "big", "target": "review", "conditionBranch": "true" },
        { "id": "big-skip", "source": "big", "target": "skip", "conditionBranch": "false" },
        { "id": "review-each", "source": "review", "target": "each" },
        { "id": "each-body", "source": "each", "target": "body" }
    ]);
    let again = update(&updated, &fewer, &none());
    assert!(!again.contains("Reads the diff first"), "{again}");
    assert!(again.contains("// Kept on top."), "{again}");
    assert_eq!(read(&again, &none()).unwrap().workflow, stored(fewer));
}

#[test]
fn file_names_come_from_the_workflow_name() {
    assert_eq!(
        file_name("Review a Pull Request!"),
        "review-a-pull-request.kdl"
    );
    assert_eq!(file_name("   "), "workflow.kdl");
}

/// A random JSON value of every shape a config can take.
fn step(rng: &mut u64) -> u64 {
    *rng ^= *rng << 13;
    *rng ^= *rng >> 7;
    *rng ^= *rng << 17;
    *rng
}

fn random(rng: &mut u64, depth: u32) -> Value {
    let next = |r: &mut u64| step(r);
    let pick = next(rng) % if depth > 2 { 6 } else { 9 };
    match pick {
        0 => Value::Null,
        1 => Value::Bool(next(rng) % 2 == 0),
        2 => json!((next(rng) % 2000) as i64 - 1000),
        3 => json!(((next(rng) % 100_000) as f64) / 7.0),
        4 | 5 => {
            const PARTS: [&str; 14] = [
                "a",
                " ",
                "\n",
                "\t",
                "\"",
                "\\",
                "{{steps.x.output}}",
                "é",
                "  ",
                "#",
                "-",
                "\r\n",
                "\"\"\"",
                "@",
            ];
            let n = next(rng) % 6;
            Value::String((0..n).map(|_| PARTS[(next(rng) % 14) as usize]).collect())
        }
        6 => Value::Array((0..next(rng) % 4).map(|_| random(rng, depth + 1)).collect()),
        _ => {
            const KEYS: [&str; 10] = [
                "a",
                "b-c",
                "-",
                "@x",
                "with space",
                "1st",
                "trigger",
                "true",
                "é",
                "x.y",
            ];
            let n = next(rng) % 4;
            let mut map = serde_json::Map::new();
            for _ in 0..n {
                let k = KEYS[(next(rng) % 10) as usize];
                map.insert(k.to_owned(), random(rng, depth + 1));
            }
            Value::Object(map)
        }
    }
}

#[test]
fn any_config_reads_back_as_written() {
    // Seeded: a failure names its seed, kept below as a case of its own.
    for seed in 1..400u64 {
        let mut rng = seed.wrapping_mul(0x9E37_79B9_7F4A_7C15) | 1;
        let config = random(&mut rng, 0);
        let extra = random(&mut rng, 1);
        let def = json!({ "id": "w", "name": "W", "nodes": [
            { "id": "t", "type": "trigger", "config": { "triggerType": "manual" } },
            { "id": "s", "type": "script", "config": config, "custom": extra }
        ], "edges": [{ "id": "t-s", "source": "t", "target": "s" }] });
        let text = write(&def, &none());
        let got = read(&text, &none()).unwrap_or_else(|e| panic!("seed {seed}: {e:?}\n{text}"));
        assert_eq!(got.workflow, def, "seed {seed}:\n{text}");
    }
}

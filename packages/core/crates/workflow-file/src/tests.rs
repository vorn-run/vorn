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
            { "id": "each", "type": "loop", "label": "Each", "config": { "mode": "forEach", "items": "{{steps.fetch.json.items}}", "bodyNodeIds": ["body"], "parallel": true, "concurrency": 4 } },
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
    if std::env::var_os("SHOW").is_some() {
        eprintln!("{}", write(&def, &none()));
    }
    assert_eq!(back(&def, &none()), stored(def));
}

#[test]
fn a_step_after_the_one_before_says_nothing_and_a_branch_says_where_from() {
    let text = write(&sample(), &none());
    assert!(
        text.starts_with("name = \"Review a pull request\"\n"),
        "{text}"
    );
    assert!(
        text.contains("\n[trigger]\nid = \"start\"\ntype = \"manual\"\n"),
        "{text}"
    );
    assert!(
        text.contains("\n[[trigger.inputs]]\nkey = \"pr\"\n"),
        "{text}"
    );
    assert!(
        text.contains("\n[[step]]\nid = \"fetch\"\ntype = \"script\"\nlabel = \"Fetch\"\nslug = \"fetch\"\nx = 0\ny = 110.5\nscriptType"),
        "{text}"
    );
    assert!(text.contains("needs = [\"big:true#e3\"]"), "{text}");
    assert!(text.contains("needs = [\"big:false\"]"), "{text}");
    assert!(
        text.contains("prompt = \"\"\"\nReview \"this\"\n{{steps.fetch.output}}\n\"\"\""),
        "{text}"
    );
    assert!(!text.contains("lastRunAt"), "{text}");
}

#[test]
fn a_script_with_backslashes_is_written_literally() {
    let def = json!({ "name": "W", "nodes": [
        { "id": "s", "type": "script", "config": { "scriptContent": "grep -E '^a' \\\n  file.txt\n" } },
        { "id": "t", "type": "script", "config": { "scriptContent": "printf \"%s\\n\" it" } }
    ], "edges": [{ "id": "s-t", "source": "s", "target": "t" }] });
    let text = write(&def, &none());
    assert!(
        text.contains("scriptContent = '''\ngrep -E '^a' \\\n  file.txt\n'''"),
        "{text}"
    );
    assert!(
        text.contains("scriptContent = 'printf \"%s\\n\" it'"),
        "{text}"
    );
    assert_eq!(back(&def, &none()), def);
}

#[test]
fn edges_keep_their_order_and_odd_edges_are_written_whole() {
    let mut def = sample();
    let edges = def["edges"].as_array_mut().unwrap();
    edges.reverse();
    edges.push(
        json!({ "id": "x y", "source": "fetch", "target": "skip", "weight": 2, "from": "odd" }),
    );
    edges.push(json!({ "source": "skip", "target": "body" }));
    let text = write(&def, &none());
    assert!(text.contains("edgeOrder = ["), "{text}");
    assert!(
        text.contains("[[edge]]\nfrom = \"skip\"\nto = \"body\""),
        "{text}"
    );
    assert!(text.contains("\"@from\" = \"odd\""), "{text}");
    assert_eq!(back(&def, &none()), stored(def));
}

#[test]
fn what_toml_has_no_word_for_still_reads_back() {
    let def = json!({ "name": "W", "nodes": [
        { "id": "t", "type": "trigger", "config": { "triggerType": "manual", "type": "odd" } },
        { "id": "a", "type": "script", "custom": { "@json": "x" }, "config": { "value": null, "big": u64::MAX, "list": [null, 1, "a"] } },
        { "id": "b", "type": "script" },
        { "id": "c", "type": 5, "label": 7, "position": { "x": 1, "y": 2, "z": 3 }, "config": "text" }
    ], "edges": [] });
    let text = write(&def, &none());
    assert!(text.contains("value = { \"@json\" = \"null\" }"), "{text}");
    assert!(
        text.contains("\"@config\" = { \"@json\" = \"\" }"),
        "{text}"
    );
    assert!(
        !text.contains("[trigger]"),
        "a trigger whose settings say type is a step: {text}"
    );
    assert_eq!(back(&def, &none()), def);
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
        text.contains("connectionId = { \"@connection\" = \"workspace-eng\" }"),
        "{text}"
    );
    // Two connections share the name: the id stays, so nothing is lost.
    assert!(text.contains("connectionId = \"c-2\""), "{text}");
    assert!(
        text.contains("projectPath = { \"@project\" = \".\" }"),
        "{text}"
    );
    assert!(text.contains("cwd = { \"@project\" = \"src\" }"), "{text}");
    assert!(
        text.contains("projectName = { \"@project-name\" = \"app\" }"),
        "{text}"
    );
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
    let text = "name = \"W\"\n\n[[step]]\nid = \"a\"\nlabel = 5\n\n[[step]]\nid = \"b\"\nneeds = [\"nowhere\"]\n";
    let errors = read(text, &none()).unwrap_err();
    let shown: Vec<String> = errors.iter().map(ToString::to_string).collect();
    assert_eq!(
        shown,
        [
            "5:1: label is a string",
            "7:1: needs `nowhere`, which no step here is",
        ]
    );
    let bad = read("name = \"W\"\n[[step]\nid = \"a\"\n", &none()).unwrap_err();
    assert_eq!(bad.len(), 1);
    assert_eq!(bad[0].at.0, 2, "{bad:?}");
    let twice = read("name = \"W\"\nname = \"V\"\n", &none()).unwrap_err();
    assert_eq!(twice[0].at, (2, 1), "{twice:?}");
    let untyped = read("[trigger]\nid = \"t\"\n", &none()).unwrap_err();
    assert_eq!(
        untyped[0].to_string(),
        "1:1: a trigger says its type, as type = \"manual\""
    );
}

#[test]
fn a_misspelt_field_is_a_setting_with_a_hint() {
    let read = read(
        "name = \"W\"\n\n[[step]]\nid = \"a\"\ntype = \"script\"\nlable = \"A\"\n",
        &none(),
    )
    .unwrap();
    assert_eq!(read.workflow["nodes"][0]["config"]["lable"], "A");
    let hints: Vec<String> = read.hints.iter().map(ToString::to_string).collect();
    assert_eq!(
        hints,
        ["6:1: `lable` is read as a setting of the step; did you mean label?"]
    );
}

#[test]
fn an_edit_keeps_the_comments_of_what_did_not_change() {
    let def = sample();
    let mut text = write(&def, &none());
    text = text.replace(
        "\n[[step]]\nid = \"fetch\"",
        "\n# Reads the diff first.\n[[step]]\nid = \"fetch\"",
    );
    text = text.replace(
        "variable = \"{{steps.fetch.output}}\"",
        "# The diff, as fetched.\nvariable = \"{{steps.fetch.output}}\"",
    );
    text = format!("# Kept on top.\n{text}# And below.\n");
    let mut changed = def.clone();
    changed["nodes"][2]["config"]["operator"] = json!("contains");
    changed["nodes"][1]["label"] = json!("Fetch the diff");
    changed["name"] = json!("Review it");
    let updated = update(&text, &changed, &none());
    for kept in [
        "# Kept on top.",
        "# Reads the diff first.",
        "# The diff, as fetched.",
        "# And below.",
    ] {
        assert!(updated.contains(kept), "{kept} lost:\n{updated}");
    }
    assert!(updated.contains("operator = \"contains\""), "{updated}");
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
    assert!(again.contains("# Kept on top."), "{again}");
    assert_eq!(read(&again, &none()).unwrap().workflow, stored(fewer));
}

#[test]
fn a_file_written_by_hand_keeps_its_layout_where_nothing_changed() {
    let text = "# Mine.\nname = \"W\"\n\n[trigger]\ntype = \"manual\" # by hand\nid = \"t\"\n\n[[step]]\nid = \"a\"\ntype = \"script\"\nheaders = { a = 1 }   # inline, as I like it\nscriptContent = 'echo hi'\n";
    let wf = read(text, &none()).unwrap().workflow;
    assert_eq!(update(text, &wf, &none()), text);
    let mut changed = wf.clone();
    changed["nodes"][1]["config"]["scriptContent"] = json!("echo bye");
    let updated = update(text, &changed, &none());
    assert!(updated.contains("type = \"manual\" # by hand"), "{updated}");
    assert!(
        updated.contains("headers = { a = 1 }   # inline, as I like it"),
        "{updated}"
    );
    assert!(
        updated.contains("scriptContent = \"echo bye\""),
        "{updated}"
    );
    assert_eq!(read(&updated, &none()).unwrap().workflow, changed);
}

#[test]
fn an_editor_edit_and_a_file_edit_meet_and_the_file_is_written_with_both() {
    let base = sample();
    let file_text = write(&base, &none()).replace(
        "label = \"Big?\"",
        "# Asked of every diff.\nlabel = \"Is it big?\"",
    );
    let theirs = read(&file_text, &none()).unwrap().workflow;
    let mut ours = stored(base.clone());
    ours["nodes"][6]["config"]["scriptContent"] = json!("echo {{loop.item.name}}");
    let met = meet(&stored(base), &ours, &theirs);
    assert_eq!(met["nodes"][2]["label"], "Is it big?");
    assert_eq!(
        met["nodes"][6]["config"]["scriptContent"],
        "echo {{loop.item.name}}"
    );
    let written = update(&file_text, &met, &none());
    assert!(written.contains("# Asked of every diff."), "{written}");
    assert_eq!(read(&written, &none()).unwrap().workflow, met);
}

#[test]
fn the_outline_names_each_step_and_where_it_is() {
    let text = write(&sample(), &none());
    let outline = outline(&text);
    assert_eq!(outline.len(), 7);
    assert!(outline[0].trigger);
    assert_eq!(outline[0].kind.as_deref(), Some("manual"));
    assert_eq!(outline[1].kind.as_deref(), Some("script"));
    let line = text.lines().position(|l| l == "[[step]]").unwrap() + 1;
    assert_eq!(outline[1].at, (line, 1));
}

#[test]
fn file_names_come_from_the_workflow_name() {
    assert_eq!(
        file_name("Review a Pull Request!"),
        "review-a-pull-request.toml"
    );
    assert_eq!(file_name("   "), "workflow.toml");
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
            const PARTS: [&str; 16] = [
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
                "'",
                "\r\n",
                "\"\"\"",
                "'''",
                "@",
                "\u{1}",
            ];
            let n = next(rng) % 6;
            Value::String((0..n).map(|_| PARTS[(next(rng) % 16) as usize]).collect())
        }
        6 => Value::Array((0..next(rng) % 4).map(|_| random(rng, depth + 1)).collect()),
        _ => {
            const KEYS: [&str; 14] = [
                "a",
                "b-c",
                "-",
                "@x",
                "@json",
                "with space",
                "1st",
                "trigger",
                "id",
                "needs",
                "true",
                "é",
                "x.y",
                "",
            ];
            let n = next(rng) % 4;
            let mut map = serde_json::Map::new();
            for _ in 0..n {
                let k = KEYS[(next(rng) % 14) as usize];
                map.insert(k.to_owned(), random(rng, depth + 1));
            }
            Value::Object(map)
        }
    }
}

#[test]
fn any_config_reads_back_as_written() {
    // Seeded: a failure names its seed, kept below as a case of its own.
    for seed in 1..600u64 {
        let mut rng = seed.wrapping_mul(0x9E37_79B9_7F4A_7C15) | 1;
        let config = random(&mut rng, 0);
        let extra = random(&mut rng, 1);
        let field = random(&mut rng, 1);
        let def = json!({ "id": "w", "name": "W", "odd": field, "nodes": [
            { "id": "t", "type": "trigger", "config": { "triggerType": "manual" } },
            { "id": "s", "type": "script", "config": config, "custom": extra }
        ], "edges": [{ "id": "t-s", "source": "t", "target": "s" }] });
        let text = write(&def, &none());
        let got = read(&text, &none()).unwrap_or_else(|e| panic!("seed {seed}: {e:?}\n{text}"));
        assert_eq!(got.workflow, def, "seed {seed}:\n{text}");
        assert_eq!(update(&text, &def, &none()), text, "seed {seed}");
    }
}

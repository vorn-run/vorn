//! Every workflow Vorn has a record of, written as a file and read back
//! unchanged: the workflows a new database is seeded with, the ones the
//! replayed runs and the recorded engine cases use, and the shapes of real
//! workflows people run (loops, gates, connector steps, long prompts).

use std::path::PathBuf;

use serde_json::Value;
use vorn_workflow_file::{read, update, write, Bindings, RUNTIME_FIELDS};

fn repo(path: &str) -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../../../..")
        .join(path)
}

fn json(path: &str) -> Value {
    let text = std::fs::read_to_string(repo(path)).unwrap_or_else(|e| panic!("{path}: {e}"));
    serde_json::from_str(&text).unwrap()
}

/// Every workflow in the fixtures.
fn corpus() -> Vec<(String, Value)> {
    let mut out = Vec::new();
    for wf in json("tests/fixtures/workflows-replay/seeded.json")
        .as_array()
        .unwrap()
    {
        out.push(("seeded".to_owned(), wf.clone()));
    }
    for wf in json("tests/fixtures/workflows-replay/final.json")["workflows"]
        .as_array()
        .unwrap()
    {
        out.push(("replay".to_owned(), wf.clone()));
    }
    for case in json("tests/fixtures/js-reference/workflow-engine.json")
        .as_array()
        .unwrap()
    {
        if let Some(wf) = case.pointer("/input/workflow") {
            out.push((case["name"].as_str().unwrap().to_owned(), wf.clone()));
        }
    }
    for wf in json("tests/fixtures/workflow-files/shapes.json")
        .as_array()
        .unwrap()
    {
        out.push(("shape".to_owned(), wf.clone()));
    }
    out
}

fn without_runtime(mut wf: Value) -> Value {
    for k in RUNTIME_FIELDS {
        wf.as_object_mut().unwrap().shift_remove(k);
    }
    wf
}

#[test]
fn every_recorded_workflow_reads_back_from_its_file_unchanged() {
    let all = corpus();
    assert!(all.len() > 80, "the corpus has {} workflows", all.len());
    let b = Bindings::default();
    for (from, wf) in all {
        let text = write(&wf, &b);
        let back = read(&text, &b).unwrap_or_else(|e| panic!("{from}: {e:?}\n{text}"));
        assert_eq!(
            back.workflow,
            without_runtime(wf.clone()),
            "{from}:\n{text}"
        );
        // Written back over itself, a file is what it was.
        assert_eq!(update(&text, &wf, &b), text, "{from}");
    }
}

/// Workflows of one's own, outside the repository: `VORN_WORKFLOW_CORPUS`
/// names a folder of exported `.json` workflows to check the same way.
#[test]
fn a_folder_of_ones_own_workflows_reads_back_unchanged() {
    let Ok(dir) = std::env::var("VORN_WORKFLOW_CORPUS") else {
        eprintln!("skipped: VORN_WORKFLOW_CORPUS names no folder of workflows to check");
        return;
    };
    let b = Bindings::default();
    for entry in std::fs::read_dir(dir).unwrap().flatten() {
        let text = std::fs::read_to_string(entry.path()).unwrap();
        let Ok(wf) = serde_json::from_str::<Value>(&text) else {
            continue;
        };
        let file = write(&wf, &b);
        let back = read(&file, &b).unwrap_or_else(|e| panic!("{:?}: {e:?}", entry.path()));
        assert_eq!(back.workflow, without_runtime(wf), "{:?}", entry.path());
    }
}

/// The workflows Vorn and its examples ship, and the one the docs show, each
/// read as written and written back the same.
#[test]
fn every_shipped_and_documented_workflow_file_reads() {
    let mut files: Vec<(String, String)> = Vec::new();
    for dir in [
        "packages/core/extensions/workflows/workflows",
        "examples/extensions/shout/workflows",
    ] {
        for entry in std::fs::read_dir(repo(dir)).unwrap().flatten() {
            let path = entry.path();
            assert_eq!(
                path.extension().and_then(|e| e.to_str()),
                Some("toml"),
                "{path:?}"
            );
            files.push((
                path.display().to_string(),
                std::fs::read_to_string(&path).unwrap(),
            ));
        }
    }
    let docs = std::fs::read_to_string(repo("docs/workflows.md")).unwrap();
    let example = docs
        .split("```toml\n")
        .nth(1)
        .and_then(|rest| rest.split("```").next())
        .unwrap();
    files.push(("docs/workflows.md".to_owned(), example.to_owned()));
    assert!(files.len() >= 4, "{files:?}");
    let b = Bindings::default();
    for (name, text) in files {
        let read = read(&text, &b).unwrap_or_else(|e| panic!("{name}: {e:?}"));
        assert!(read.hints.is_empty(), "{name}: {:?}", read.hints);
        let wf = read.workflow;
        assert!(
            wf["nodes"].as_array().is_some_and(|n| n.len() >= 2),
            "{name}"
        );
        assert_eq!(update(&text, &wf, &b), text, "{name}");
        let again = read_back(&write(&wf, &b));
        assert_eq!(again, wf, "{name}");
    }
    let wf = read(example, &b).unwrap().workflow;
    let each = &wf["nodes"][3];
    assert_eq!(each["config"]["parallel"], true);
    assert_eq!(each["config"]["concurrency"], 3);
    assert_eq!(
        wf["edges"][2],
        serde_json::json!({ "id": "any-each", "source": "any", "target": "each", "conditionBranch": "true" })
    );
}

fn read_back(text: &str) -> Value {
    read(text, &Bindings::default()).unwrap().workflow
}

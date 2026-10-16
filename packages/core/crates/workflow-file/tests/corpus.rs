//! Every workflow Vorn has a record of, written as a file and read back
//! unchanged: the workflows a new database is seeded with (as recorded before they became files), the ones the
//! replayed runs and the TypeScript reference cases use, and the shapes of
//! real workflows people run (loops, gates, connector steps, long prompts).

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

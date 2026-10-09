//! A failing project script on a real sessiond reports its exit code and output, and leaves no file.

#![cfg(all(feature = "engine", unix))]

use std::sync::Arc;
use std::time::Duration;

use serde_json::{json, Value};
use vorn_engine::Config;
use vorn_sessiond::server::{self, Sessiond};
use vornd::applink::AppLink;
use vornd::engine::{Engine, EngineHost};
use vornd::holder::{self, Holder};
use vornd::native::script::{self, Scripts};
use vornd::native::{Answer, Native};

/// A sessiond, an engine held by it, and the native side that runs scripts through them.
struct Rig {
    native: Arc<Native>,
    data: tempfile::TempDir,
    _home: tempfile::TempDir,
    _serving: tokio::task::JoinHandle<std::io::Result<()>>,
    _holding: tokio::task::JoinHandle<()>,
}

impl Rig {
    async fn start() -> Rig {
        let home = tempfile::tempdir().unwrap();
        let d = Sessiond::new(server::Config {
            home: home.path().to_path_buf(),
            instance: 0x5c41,
            build: "test".into(),
            idle_exit: Duration::from_secs(600),
            spool_cap: 64 << 20,
        });
        let listener = server::bind(&d).unwrap();
        let serving = tokio::spawn(server::serve(Arc::clone(&d), listener));
        let engine = Engine::new(Config {
            history: Some(home.path().join("history")),
            build: "test".into(),
            ..Config::default()
        });
        let holder = Holder::with_engine(Arc::clone(&engine));
        let endpoint = d.endpoint();
        let holding = tokio::spawn(async move {
            let _ = holder::connect(&endpoint, &holder).await;
        });
        let data = tempfile::tempdir().unwrap();
        let native = Native::new();
        native.set_database(data.path().join("vorn.db"));
        native.set_host(Arc::new(EngineHost::new(
            Arc::clone(&engine),
            tokio::runtime::Handle::current(),
        )));
        let link = Arc::new(AppLink::default());
        link.set_scripts(Scripts::new(&native));
        native.set_link(link);
        Rig {
            native,
            data,
            _home: home,
            _serving: serving,
            _holding: holding,
        }
    }

    /// The scripts' files left on disk.
    fn files_left(&self) -> usize {
        std::fs::read_dir(self.data.path().join("scripts"))
            .map(|d| d.count())
            .unwrap_or(0)
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn a_failing_script_reports_its_exit_code_and_output() {
    let rig = Rig::start().await;
    let answer = script::execute(
        &rig.native,
        json!({
            "scriptType": "bash",
            "scriptContent": r#"printf "out "; printf "err-$VORN_DATA_DIR" >&2; exit 3"#,
            "cwd": rig.data.path(),
        }),
    )
    .await;
    let Answer::Result(answer) = answer else {
        panic!("not run: {answer:?}");
    };
    assert_eq!(answer["success"], false, "{answer}");
    assert_eq!(answer["exitCode"], 3, "{answer}");
    // stdout and stderr are read as one stream.
    let data = rig.data.path().to_string_lossy().into_owned();
    let output = answer["output"].as_str().unwrap();
    assert!(output.contains("out "), "{output}");
    assert!(output.contains(&format!("err-{data}")), "{output}");
    assert_eq!(answer["error"], Value::String(output.to_owned()));
    assert_eq!(rig.files_left(), 0, "the script's file goes when it ends");
}

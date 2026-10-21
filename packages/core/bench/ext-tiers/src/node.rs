//! The RSS connector as Vorn ships it: the Node bundle speaking the
//! connector protocol, started as vornd starts a connector. It fetches the
//! same `https://feeds.bench/` URLs as the Rust tiers; `loopback.mjs`
//! serves them from the fixture server.

use std::net::SocketAddr;
use std::path::{Path, PathBuf};

use serde_json::{Map, Value};
use vorn_extensions::child::Launch;
use vorn_extensions::connector::{self, SdkClient, SdkLaunch, Source, Timeouts};

pub fn dir() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("rss/node")
}

/// `node` on this process's PATH.
pub fn node() -> Option<PathBuf> {
    std::env::var_os("PATH")
        .into_iter()
        .flat_map(|p| std::env::split_paths(&p).collect::<Vec<_>>())
        .map(|d| d.join("node"))
        .find(|p| p.is_file())
}

/// The connector, open.
pub struct NodeRss {
    pub client: SdkClient,
}

impl NodeRss {
    /// Starts the connector with `feeds` as its connection's feeds, its fetches served by `fixtures`.
    pub async fn open(node: &Path, fixtures: SocketAddr, feeds: &[String]) -> Result<NodeRss, String> {
        let dir = dir();
        let launch = Launch {
            program: node.to_path_buf(),
            args: vec![
                "--import".into(),
                dir.join("loopback.mjs").display().to_string(),
                dir.join("index.js").display().to_string(),
            ],
            cwd: dir.clone(),
            env: vec![
                ("BENCH_FEEDS".into(), format!("http://{fixtures}")),
                ("RSS_FEEDS".into(), feeds.join("\n")),
                ("RSS_LOOKBACK_HOURS".into(), "8760".into()),
            ],
        };
        let sdk = SdkLaunch { launch, source: Source::Command, protocol: None };
        let client = connector::open(&sdk, "bench.rss-node", "bench", Timeouts::default())
            .await
            .map_err(|e| format!("{e:?}"))?;
        Ok(NodeRss { client })
    }

    pub async fn action(&self, name: &str, args: &Value) -> Result<Value, String> {
        let args = args.as_object().cloned().unwrap_or_else(Map::new);
        self.client.action(name, &args, None).await.map(Value::Object).map_err(|e| e.to_string())
    }

    pub async fn poll(&self, cursor: Option<&str>) -> Result<Value, String> {
        self.client.poll("newItem", cursor, None).await.map(Value::Object).map_err(|e| e.to_string())
    }

    pub async fn close(&self) {
        self.client.close().await;
    }
}

/// The connector's process: this one's child running the bundle.
pub fn pid() -> Option<u32> {
    // spawn-visible: ps, run by the bench only.
    let out = std::process::Command::new("ps").args(["-A", "-o", "pid=,ppid=,command="]).output().ok()?;
    let me = std::process::id();
    String::from_utf8_lossy(&out.stdout).lines().find_map(|l| {
        let mut parts = l.split_whitespace();
        let pid: u32 = parts.next()?.parse().ok()?;
        let ppid: u32 = parts.next()?.parse().ok()?;
        (ppid == me && l.contains("rss/node/index.js")).then_some(pid)
    })
}

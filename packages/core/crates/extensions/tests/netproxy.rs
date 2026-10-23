//! A program under the sandbox profile reaches its granted hosts through the
//! proxy and nothing else, with Node's own fetch. macOS with node only.

use std::process::Command;

use vorn_extensions::child::Launch;
use vorn_extensions::netproxy::NetProxy;
use vorn_extensions::sandbox;

const PROGRAM: &str = r#"
const [allowed, refused] = process.argv.slice(2)
const out = []
for (const url of [allowed, refused]) {
  try {
    const r = await fetch(url)
    out.push(`${r.status} ${await r.text()}`)
  } catch (e) {
    out.push(`failed ${e.cause?.code ?? e.message}`)
  }
}
console.log(JSON.stringify(out))
"#;

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_sandboxed_program_reaches_its_hosts_only_through_the_proxy() {
    if sandbox::Kind::here() != sandbox::Kind::Seatbelt {
        eprintln!("skipped: the sandbox profile needs macOS");
        return;
    }
    if !Command::new("node")
        .arg("--version")
        .output()
        .is_ok_and(|o| o.status.success())
    {
        eprintln!("skipped: no node on PATH");
        return;
    }
    let server = tokio::net::TcpListener::bind(("127.0.0.1", 0))
        .await
        .unwrap();
    let port = server.local_addr().unwrap().port();
    tokio::spawn(async move {
        use tokio::io::{AsyncReadExt, AsyncWriteExt};
        while let Ok((mut s, _)) = server.accept().await {
            tokio::spawn(async move {
                let mut buf = [0u8; 2048];
                let _ = s.read(&mut buf).await;
                let _ = s
                    .write_all(
                        b"HTTP/1.1 200 OK\r\nContent-Length: 4\r\nConnection: close\r\n\r\nfeed",
                    )
                    .await;
            });
        }
    });
    let proxy = NetProxy::start().await.unwrap();
    let grant = proxy.grant("test.feeds", vec!["localhost".into()]).unwrap();
    let dir = tempfile::tempdir().unwrap();
    let script = dir.path().join("main.mjs");
    std::fs::write(&script, PROGRAM).unwrap();
    let mut env: Vec<(String, String)> = std::env::vars()
        .filter(|(k, _)| ["PATH", "HOME", "TMPDIR"].contains(&k.as_str()))
        .collect();
    env.extend(grant.env());
    let launch = Launch {
        program: "node".into(),
        args: vec![
            script.to_string_lossy().into_owned(),
            format!("http://localhost:{port}/feed"),
            format!("http://127.0.0.1:{port}/feed"),
        ],
        cwd: dir.path().to_path_buf(),
        env,
    };
    let mut reach = sandbox::Reach::of(dir.path(), None, None);
    reach.ports.push(proxy.port());
    let held = sandbox::wrap(launch, &reach, None).expect("Seatbelt confines");
    let out = tokio::process::Command::new(&held.program)
        .args(&held.args)
        .current_dir(&held.cwd)
        .env_clear()
        .envs(held.env.iter().map(|(k, v)| (k, v)))
        .output()
        .await
        .unwrap();
    let stdout = String::from_utf8_lossy(&out.stdout);
    let got: Vec<String> = serde_json::from_str(stdout.trim())
        .unwrap_or_else(|e| panic!("{e}: {stdout} {}", String::from_utf8_lossy(&out.stderr)));
    assert_eq!(got[0], "200 feed");
    // Node tunnels plain http too, so a refused host is a failed fetch.
    assert!(
        got[1].starts_with("failed") || got[1].starts_with("403"),
        "{got:?}"
    );

    // Without the proxy the same program reaches nothing at all.
    let mut bare = held.clone();
    bare.args[1] = sandbox::profile(&sandbox::Reach::of(dir.path(), None, None));
    bare.env
        .retain(|(k, _)| !k.to_lowercase().ends_with("_proxy"));
    let out = tokio::process::Command::new(&bare.program)
        .args(&bare.args)
        .env_clear()
        .envs(bare.env.iter().map(|(k, v)| (k, v)))
        .output()
        .await
        .unwrap();
    let got: Vec<String> =
        serde_json::from_str(String::from_utf8_lossy(&out.stdout).trim()).unwrap();
    assert!(got[0].starts_with("failed"), "{got:?}");
}

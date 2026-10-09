//! `vorn-sessiond [--home DIR] [--idle-exit SECS]`
//!
//! Binds a user-only endpoint under `DIR` (default `$VORN_HOME`, else
//! `~/.vorn`), announces it in `run/`, prints `listening <endpoint>` on
//! stdout for whoever runs it by hand, and serves until it has held no sessions and had no
//! vornd for `SECS` (default 60, or `VORN_SESSIOND_IDLE_EXIT`), or until
//! SIGTERM or Ctrl-C. Either way it takes its endpoint and announcement back.
//!
//! On Linux it starts its programs through a copy of itself run as
//! `vorn-sessiond --spawn-helper` ([`vorn_sessiond::spawn::helper`]).
//!
//! `VORN_SESSIOND_HANDOFF_FAULT=<step>:<fail|stall>` (Unix) makes a handoff
//! fail or hang at that step, for tests that check nothing is lost when it does.

use std::io::Write;
use std::path::PathBuf;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use vorn_sessiond::server::{self, Config, Sessiond};

fn main() {
    // Before any thread starts: nothing sessiond was handed reaches a terminal.
    #[cfg(unix)]
    {
        vorn_sessiond::pty::stdio_open();
        vorn_sessiond::pty::cloexec_inherited();
    }
    #[cfg(target_os = "linux")]
    if std::env::args_os().nth(1).as_deref() == Some(vorn_sessiond::spawn::helper::ARG.as_ref()) {
        if let Err(e) = vorn_sessiond::spawn::helper::serve() {
            eprintln!("vorn-sessiond: spawn helper: {e}");
            std::process::exit(1);
        }
        return;
    }
    let mut home = std::env::var_os("VORN_HOME").map(PathBuf::from);
    let mut idle = std::env::var("VORN_SESSIOND_IDLE_EXIT")
        .ok()
        .and_then(|v| v.parse().ok())
        .unwrap_or(60u64);
    let mut args = std::env::args().skip(1);
    while let Some(a) = args.next() {
        match a.as_str() {
            "--home" => home = args.next().map(PathBuf::from),
            "--idle-exit" => idle = args.next().and_then(|v| v.parse().ok()).unwrap_or(idle),
            "--version" => {
                println!("{}", env!("CARGO_PKG_VERSION"));
                return;
            }
            other => {
                eprintln!("vorn-sessiond: unknown argument {other}");
                std::process::exit(2);
            }
        }
    }
    // The running binary even after an upgrade replaced the file.
    #[cfg(target_os = "linux")]
    if let Err(e) = vorn_sessiond::spawn::helper::install(std::path::Path::new("/proc/self/exe")) {
        eprintln!("vorn-sessiond: no spawn helper, so programs start from here: {e}");
    }
    let home = home
        .or_else(|| home_dir().map(|h| h.join(".vorn")))
        .unwrap_or_else(|| PathBuf::from(".vorn"));
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_nanos())
        .unwrap_or(0);
    let instance = nanos ^ (u128::from(std::process::id()) << 96);
    let d = Sessiond::new(Config {
        home,
        instance,
        build: env!("CARGO_PKG_VERSION").into(),
        idle_exit: Duration::from_secs(idle),
        spool_cap: 512 << 20,
    });
    #[cfg(unix)]
    if let Some(f) = std::env::var("VORN_SESSIOND_HANDOFF_FAULT")
        .ok()
        .and_then(|v| v.parse().ok())
    {
        d.inject(f);
    }
    let rt = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .expect("tokio runtime");
    let res = rt.block_on(async {
        let listener = server::bind(&d)?;
        // Printed once the endpoint can take a connection.
        println!("listening {}", d.endpoint());
        let _ = std::io::stdout().flush();
        let stopper = std::sync::Arc::clone(&d);
        tokio::spawn(async move {
            terminated().await;
            stopper.shut_down();
        });
        server::serve(d, listener).await
    });
    if let Err(e) = res {
        eprintln!("vorn-sessiond: {e}");
        std::process::exit(1);
    }
}

/// Resolves on SIGTERM or Ctrl-C.
async fn terminated() {
    #[cfg(unix)]
    {
        use tokio::signal::unix::{signal, SignalKind};
        if let Ok(mut term) = signal(SignalKind::terminate()) {
            tokio::select! {
                _ = tokio::signal::ctrl_c() => {}
                _ = term.recv() => {}
            }
            return;
        }
    }
    let _ = tokio::signal::ctrl_c().await;
}

fn home_dir() -> Option<PathBuf> {
    std::env::var_os(if cfg!(windows) { "USERPROFILE" } else { "HOME" }).map(PathBuf::from)
}

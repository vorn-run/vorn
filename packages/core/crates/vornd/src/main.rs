//! `vornd --upstream 127.0.0.1:50091 [--listen 127.0.0.1:0] [--groups git=shadow] [--log-file PATH] [--sessiond PATH --home DIR] [--debug-spawn]`
//!
//! Prints one line of JSON, `{"port":N,"protocol":P}`, once it is listening, so
//! whoever started it knows where to connect.
//!
//! With a session holder and the server's credential in `VORND_SERVER_TOKEN`,
//! it also links to the server as its process backend
//! ([`vornd::node_link`]), and says where it listens only once the link is up
//! or [`LINK_WAIT`] has passed, so the app's first terminals already go
//! through it.

use std::fs::OpenOptions;
use std::io::Write;
use std::net::SocketAddr;
use std::path::PathBuf;
use std::process::ExitCode;
use std::sync::{Arc, Mutex};

use tokio::net::TcpListener;
use tracing::{error, info};
use tracing_subscriber::EnvFilter;
use vornd::holder::{self, Holder, HolderConfig};
use vornd::protocol::VORND_PROTOCOL;
use vornd::{proxy, Daemon, Groups};

const USAGE: &str = "usage: vornd --upstream HOST:PORT [--listen 127.0.0.1:PORT] [--groups group=mode,...] [--log-file PATH] [--exit-with-stdin] [--sessiond PATH --home DIR] [--debug-spawn]

  --upstream   the Node server to forward to
  --listen     where to listen; loopback only (default 127.0.0.1:0)
  --groups     per-group switches, forward | shadow | native (default: all forward;
               also read from VORND_GROUPS)
  --log-file   append the log here instead of stderr; VORND_LOG sets the level
  --exit-with-stdin
               stop when stdin closes, so vornd ends with whoever started it,
               even if that process is killed
  --sessiond   the vorn-sessiond binary this build ships: keep one running under
               --home (its run/ directory is where running ones are found)
  --home       the data directory, $VORN_HOME
  --debug-spawn
               answer vornd:spawn, which starts a session in the session holder;
               for tests

  VORND_SERVER_TOKEN, with --sessiond, makes vornd the server's process
  backend: it links to the server with that credential and the server starts
  its terminals and agents through it";

/// How long vornd waits for the server link before saying where it listens.
/// Under the app's own start timeout for vornd, with room to spare.
#[cfg(feature = "engine")]
const LINK_WAIT: std::time::Duration = std::time::Duration::from_secs(3);

#[derive(Debug)]
struct Args {
    upstream: SocketAddr,
    listen: SocketAddr,
    groups: Groups,
    log_file: Option<String>,
    exit_with_stdin: bool,
    holder: Option<HolderConfig>,
    debug_spawn: bool,
}

fn parse_args(mut args: impl Iterator<Item = String>) -> Result<Args, String> {
    let mut upstream = None;
    let mut listen: SocketAddr = ([127, 0, 0, 1], 0).into();
    let mut groups = std::env::var("VORND_GROUPS").ok();
    let mut log_file = None;
    let mut exit_with_stdin = false;
    let mut sessiond = None;
    let mut home = None;
    let mut debug_spawn = false;
    while let Some(flag) = args.next() {
        let mut value = |name: &str| args.next().ok_or_else(|| format!("{name} needs a value"));
        match flag.as_str() {
            "--upstream" => {
                upstream = Some(
                    value("--upstream")?
                        .parse()
                        .map_err(|e| format!("--upstream: {e}"))?,
                )
            }
            "--listen" => {
                listen = value("--listen")?
                    .parse()
                    .map_err(|e| format!("--listen: {e}"))?
            }
            "--groups" => groups = Some(value("--groups")?),
            "--log-file" => log_file = Some(value("--log-file")?),
            "--exit-with-stdin" => exit_with_stdin = true,
            "--sessiond" => sessiond = Some(PathBuf::from(value("--sessiond")?)),
            "--home" => home = Some(PathBuf::from(value("--home")?)),
            "--debug-spawn" => debug_spawn = true,
            "-h" | "--help" => return Err(String::new()),
            other => return Err(format!("unknown argument `{other}`")),
        }
    }
    let upstream = upstream.ok_or("--upstream is required")?;
    // Everything behind vornd trusts that its peers are on this machine, which
    // only stays true while vornd itself is reachable from nowhere else.
    if !listen.ip().is_loopback() {
        return Err(format!(
            "--listen must be a loopback address, not {}",
            listen.ip()
        ));
    }
    let groups = match groups {
        Some(spec) => Groups::parse(&spec).map_err(|e| format!("--groups: {e}"))?,
        None => Groups::all_forward(),
    };
    let holder = match (sessiond, home) {
        (Some(bundled), Some(home)) => Some(HolderConfig { home, bundled }),
        (None, None) => None,
        _ => return Err("--sessiond and --home go together".into()),
    };
    Ok(Args {
        upstream,
        listen,
        groups,
        log_file,
        exit_with_stdin,
        holder,
        debug_spawn,
    })
}

/// Resolves once stdin reaches its end: the parent closed the pipe or died.
/// Read on its own thread, because a blocking read is the one way to see the
/// end of a pipe on every platform.
async fn stdin_closed() {
    let (tx, rx) = tokio::sync::oneshot::channel::<()>();
    std::thread::spawn(move || {
        let mut sink = [0u8; 256];
        let mut stdin = std::io::stdin().lock();
        while matches!(std::io::Read::read(&mut stdin, &mut sink), Ok(n) if n > 0) {}
        let _ = tx.send(());
    });
    let _ = rx.await;
}

fn init_logging(log_file: Option<&str>) -> Result<(), String> {
    let filter = EnvFilter::try_from_env("VORND_LOG").unwrap_or_else(|_| EnvFilter::new("info"));
    let builder = tracing_subscriber::fmt().with_env_filter(filter);
    match log_file {
        Some(path) => {
            let file = OpenOptions::new()
                .create(true)
                .append(true)
                .open(path)
                .map_err(|e| format!("--log-file {path}: {e}"))?;
            builder
                .with_ansi(false)
                .with_writer(Mutex::new(file))
                .init();
        }
        None => builder.with_writer(std::io::stderr).init(),
    }
    Ok(())
}

async fn shutdown_signal() {
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

#[cfg(feature = "engine")]
fn new_holder(cfg: &HolderConfig) -> Holder {
    Holder::with_engine(vornd::engine::Engine::new(vorn_engine::Config {
        history: Some(cfg.home.join("vornd").join("history")),
        build: env!("CARGO_PKG_VERSION").into(),
        ..vorn_engine::Config::default()
    }))
}

#[cfg(not(feature = "engine"))]
fn new_holder(_: &HolderConfig) -> Holder {
    Holder::new()
}

/// Starts the server link and waits a little for it to come up.
#[cfg(feature = "engine")]
async fn link(engine: &Arc<vornd::engine::Engine>, upstream: SocketAddr, token: String) {
    use vornd::node_link::{self, LinkConfig};
    let (linked, mut up) = tokio::sync::watch::channel(false);
    tokio::spawn(node_link::run(
        Arc::clone(engine),
        LinkConfig { upstream, token },
        linked,
    ));
    if tokio::time::timeout(LINK_WAIT, up.wait_for(|&v| v))
        .await
        .is_err()
    {
        info!("the server link is not up yet; listening anyway");
    }
}

/// The server's credential, taken out of the environment before any thread
/// or child exists, so no session holder or program inherits it.
fn take_server_token() -> Option<String> {
    let token = std::env::var(vornd::node_link_token_env()).ok();
    // Single-threaded here: nothing else reads the environment yet.
    std::env::remove_var(vornd::node_link_token_env());
    token.filter(|t| !t.is_empty())
}

fn main() -> ExitCode {
    let server_token = take_server_token();
    let args = match parse_args(std::env::args().skip(1)) {
        Ok(args) => args,
        Err(message) => {
            if !message.is_empty() {
                eprintln!("vornd: {message}");
            }
            eprintln!("{USAGE}");
            return ExitCode::from(2);
        }
    };
    if let Err(message) = init_logging(args.log_file.as_deref()) {
        eprintln!("vornd: {message}");
        return ExitCode::from(2);
    }
    let runtime = match tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .build()
    {
        Ok(rt) => rt,
        Err(err) => {
            eprintln!("vornd: could not start: {err}");
            return ExitCode::FAILURE;
        }
    };
    runtime.block_on(async move {
        let listener = match TcpListener::bind(args.listen).await {
            Ok(l) => l,
            Err(err) => {
                error!(listen = %args.listen, %err, "could not listen");
                return ExitCode::FAILURE;
            }
        };
        let port = listener.local_addr().map(|a| a.port()).unwrap_or(0);
        for (group, mode) in args.groups.modes() {
            info!(group, %mode, "group switch");
        }
        let mut kept = None;
        let daemon = match args.holder {
            Some(cfg) => {
                let holder = Arc::new(new_holder(&cfg));
                tokio::spawn(holder::keep(cfg, holder.clone()));
                #[cfg(feature = "engine")]
                if let (Some(token), Some(engine)) = (server_token.clone(), holder.engine()) {
                    link(engine, args.upstream, token).await;
                }
                kept = Some(holder.clone());
                Daemon::with_holder(args.upstream, args.groups, holder)
            }
            None => Daemon::new(args.upstream, args.groups),
        };
        if args.debug_spawn {
            daemon.allow_spawn();
        }
        proxy::log_upstream(&daemon).await;
        info!(port, protocol = VORND_PROTOCOL, upstream = %args.upstream, "listening");
        let mut stdout = std::io::stdout().lock();
        let _ = writeln!(stdout, "{{\"port\":{port},\"protocol\":{VORND_PROTOCOL}}}");
        let _ = stdout.flush();
        drop(stdout);
        let exit_with_stdin = args.exit_with_stdin;
        let stop = async move {
            if exit_with_stdin {
                tokio::select! {
                    () = shutdown_signal() => {}
                    () = stdin_closed() => info!("stdin closed; stopping"),
                }
            } else {
                shutdown_signal().await;
            }
        };
        proxy::serve(listener, daemon, stop).await;
        // A clean stop leaves a checkpoint at the end of every session, so
        // the next vornd has nothing to replay.
        #[cfg(feature = "engine")]
        if let Some(engine) = kept.as_ref().and_then(|h| h.engine()) {
            engine.flush().await;
        }
        drop(kept);
        info!("stopped");
        ExitCode::SUCCESS
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn parse(list: &[&str]) -> Result<Args, String> {
        parse_args(list.iter().map(|s| s.to_string()))
    }

    #[test]
    fn needs_an_upstream_and_listens_on_loopback_by_default() {
        assert!(parse(&[]).is_err());
        let args = parse(&["--upstream", "127.0.0.1:50091"]).unwrap();
        assert!(args.listen.ip().is_loopback());
        assert_eq!(args.listen.port(), 0);
    }

    #[test]
    fn refuses_to_listen_beyond_this_machine() {
        let err = parse(&["--upstream", "127.0.0.1:1", "--listen", "0.0.0.0:9"]).unwrap_err();
        assert!(err.contains("loopback"), "{err}");
        assert!(parse(&["--upstream", "127.0.0.1:1", "--listen", "[::1]:9"]).is_ok());
    }

    #[test]
    fn stays_up_without_stdin_unless_asked() {
        assert!(
            !parse(&["--upstream", "127.0.0.1:1"])
                .unwrap()
                .exit_with_stdin
        );
        assert!(
            parse(&["--upstream", "127.0.0.1:1", "--exit-with-stdin"])
                .unwrap()
                .exit_with_stdin
        );
    }

    #[test]
    fn keeps_a_session_holder_only_when_told_where() {
        assert!(parse(&["--upstream", "127.0.0.1:1"])
            .unwrap()
            .holder
            .is_none());
        let args = parse(&[
            "--upstream",
            "127.0.0.1:1",
            "--sessiond",
            "/app/vorn-sessiond",
            "--home",
            "/h",
        ])
        .unwrap();
        let holder = args.holder.unwrap();
        assert_eq!(holder.home, PathBuf::from("/h"));
        assert_eq!(holder.bundled, PathBuf::from("/app/vorn-sessiond"));
        let err = parse(&["--upstream", "127.0.0.1:1", "--home", "/h"]).unwrap_err();
        assert!(err.contains("go together"), "{err}");
    }

    #[test]
    fn passes_group_errors_on() {
        let err = parse(&["--upstream", "127.0.0.1:1", "--groups", "git=native"]).unwrap_err();
        assert!(err.starts_with("--groups"), "{err}");
    }
}

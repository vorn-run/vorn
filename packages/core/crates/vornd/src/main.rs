//! `vornd --upstream 127.0.0.1:50091 [--listen 127.0.0.1:0] [--groups workflow=shadow] [--db PATH] [--log-file PATH] [--sessiond PATH --home DIR] [--debug-spawn]`
//!
//! The app passes the desktop's launch token in `VORND_DESKTOP_TOKEN`: a
//! WebSocket that opens with it is the desktop's (TP §10). It is read once
//! and taken out of the environment before anything is started, so neither
//! sessiond nor any program it runs inherits it.
//!
//! Prints one line of JSON, `{"port":N,"protocol":P,"native":[..]}`, once it is listening, so
//! whoever started it knows where to connect. With a session holder and the
//! engine, the line also names the grid endpoint, `"grid":"<socket or pipe>"`,
//! and the app's, `"app":"<socket or pipe>"`. `"native"` lists the groups
//! vornd answers itself.

use std::fs::OpenOptions;
use std::io::Write;
use std::net::SocketAddr;
use std::path::PathBuf;
use std::process::ExitCode;
use std::sync::{Arc, Mutex};

use tokio::net::TcpListener;
use tracing::{error, info};
use tracing_subscriber::EnvFilter;
use vornd::applink::AppLink;
use vornd::holder::{self, Holder, HolderConfig};
use vornd::protocol::VORND_PROTOCOL;
use vornd::{proxy, Daemon, Groups};

const USAGE: &str = "usage: vornd --upstream HOST:PORT [--listen 127.0.0.1:PORT] [--groups group=mode,...] [--db PATH] [--log-file PATH] [--exit-with-stdin] [--sessiond PATH --home DIR] [--debug-spawn]

  --upstream   the Node server to forward to
  --listen     where to listen; loopback only (default 127.0.0.1:0)
  --groups     per-group switches, forward | shadow | native, for tests
               (default: every implemented group native, the rest forwarded;
               also read from VORND_GROUPS)
  --db         the server's vorn.db, read to tell a local project from a remote
               one and to see how the agents are configured; without it those
               calls go to the server
  --log-file   append the log here instead of stderr; VORND_LOG sets the level
  --exit-with-stdin
               stop when stdin closes, so vornd ends with whoever started it,
               even if that process is killed
  --sessiond   the vorn-sessiond binary this build ships: keep one running under
               --home (its run/ directory is where running ones are found)
  --home       the data directory, $VORN_HOME
  --debug-spawn
               answer vornd:spawn from clients too, which starts a session in the
               session holder; for tests (the app's server sends it on its own
               channel)";

#[derive(Debug)]
struct Args {
    upstream: SocketAddr,
    listen: SocketAddr,
    groups: Groups,
    db: Option<PathBuf>,
    log_file: Option<String>,
    exit_with_stdin: bool,
    holder: Option<HolderConfig>,
    debug_spawn: bool,
}

fn parse_args(mut args: impl Iterator<Item = String>) -> Result<Args, String> {
    let mut upstream = None;
    let mut listen: SocketAddr = ([127, 0, 0, 1], 0).into();
    let mut groups = std::env::var("VORND_GROUPS").ok();
    let mut db = None;
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
            "--db" => db = Some(PathBuf::from(value("--db")?)),
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
    let groups = Groups::new(groups.as_deref()).map_err(|e| format!("--groups: {e}"))?;
    let holder = match (sessiond, home) {
        (Some(bundled), Some(home)) => Some(HolderConfig { home, bundled }),
        (None, None) => None,
        _ => return Err("--sessiond and --home go together".into()),
    };
    Ok(Args {
        upstream,
        listen,
        groups,
        db,
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
    // A line a gone server cannot take is dropped: reporting it would panic vornd mid-stop.
    let builder = tracing_subscriber::fmt()
        .with_env_filter(filter)
        .log_internal_errors(false);
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

/// Opens the grid endpoint for the engine's sessions, and answers where it
/// is. Without it vornd runs on; only grid clients go without.
#[cfg(feature = "engine")]
fn serve_grid(cfg: &HolderConfig, holder: &Holder) -> Option<String> {
    let engine = holder.engine()?.clone();
    let endpoint = vornd::grid::endpoint(&cfg.home);
    match vorn_sessiond::os::Listener::bind(&cfg.home, &endpoint) {
        Ok(listener) => {
            let instance = u64::from(std::process::id())
                ^ std::time::SystemTime::now()
                    .duration_since(std::time::UNIX_EPOCH)
                    .map_or(0, |d| d.as_nanos() as u64);
            tokio::spawn(vornd::grid::serve(
                listener,
                engine,
                env!("CARGO_PKG_VERSION").into(),
                instance,
            ));
            info!(%endpoint, "grid endpoint");
            Some(endpoint)
        }
        Err(err) => {
            error!(%endpoint, %err, "no grid endpoint");
            None
        }
    }
}

#[cfg(not(feature = "engine"))]
fn serve_grid(_: &HolderConfig, _: &Holder) -> Option<String> {
    None
}

/// Opens the app's channel for the engine's sessions and names it under
/// `run/`, where the app's server looks. Without it vornd runs on; the
/// server then starts its terminals itself.
#[cfg(feature = "engine")]
fn serve_app(cfg: &HolderConfig, holder: &Holder, link: &Arc<AppLink>) -> Option<String> {
    use vornd::control;
    let engine = holder.engine()?.clone();
    let endpoint = control::endpoint(&cfg.home);
    match vorn_sessiond::os::Listener::bind(&cfg.home, &endpoint) {
        Ok(listener) => {
            tokio::spawn(control::serve(listener, engine, Arc::clone(link)));
            if let Err(err) = control::announce(&cfg.home, &endpoint) {
                error!(%endpoint, %err, "could not name the app's endpoint");
                return None;
            }
            info!(%endpoint, "the app's endpoint");
            Some(endpoint)
        }
        Err(err) => {
            error!(%endpoint, %err, "no endpoint for the app");
            None
        }
    }
}

#[cfg(not(feature = "engine"))]
fn serve_app(_: &HolderConfig, _: &Holder, _: &Arc<AppLink>) -> Option<String> {
    None
}

/// With the sessions group native, the registry owns the session records
/// between runs: what the last vornd wrote down is read back and offered,
/// and this vornd writes its own down after each change. Answers the file,
/// to write once more as vornd stops.
#[cfg(feature = "engine")]
fn carry_records(
    cfg: &HolderConfig,
    holder: &Holder,
    owned: bool,
) -> Option<vornd::carry::CarryFile> {
    if !owned {
        return None;
    }
    let engine = holder.engine()?;
    let registry = engine.registry();
    registry.own_records();
    registry.expect_holder();
    let file = vornd::carry::CarryFile::in_dir(&cfg.home.join("vornd"));
    if let Some(carried) = file.load() {
        let (offered, aged) = registry.carry(carried, vornd::registry::now_ms());
        info!(offered, aged, "sessions carried over from the last run");
        for id in registry.restored_ids() {
            engine.streams().expect(&id);
        }
    }
    tokio::spawn(vornd::carry::keep(Arc::clone(registry), file.clone()));
    Some(file)
}

#[cfg(not(feature = "engine"))]
fn carry_records(_: &HolderConfig, _: &Holder, _: bool) -> Option<vornd::carry::CarryFile> {
    None
}

/// Where the app passes the desktop's launch token.
const DESKTOP_TOKEN_VAR: &str = "VORND_DESKTOP_TOKEN";

/// The desktop's launch token, taken out of the environment. Called first
/// in `main`, while vornd has one thread and has started nothing.
fn take_desktop_token() -> Option<Vec<u8>> {
    let token = std::env::var_os(DESKTOP_TOKEN_VAR)?;
    std::env::remove_var(DESKTOP_TOKEN_VAR);
    Some(token.into_encoded_bytes()).filter(|t| !t.is_empty())
}

fn main() -> ExitCode {
    let desktop_token = take_desktop_token();
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
        let mut grid: Option<String> = None;
        let mut app: Option<(PathBuf, String)> = None;
        let link = Arc::new(AppLink::default());
        let mut carry = None;
        let daemon = match args.holder {
            Some(cfg) => {
                let home = cfg.home.clone();
                match tokio::task::spawn_blocking(move || vorn_sessiond::rundir::sweep(&home)).await
                {
                    Ok(0) => {}
                    Ok(swept) => info!(swept, "removed what dead owners left in run/"),
                    Err(err) => error!(%err, "could not sweep run/"),
                }
                let holder = Arc::new(new_holder(&cfg));
                // Before the holder connects: what it holds is adopted then.
                let owned = args.groups.mode("sessions") == vornd::Mode::Native;
                carry = carry_records(&cfg, &holder, owned);
                grid = serve_grid(&cfg, &holder);
                app = serve_app(&cfg, &holder, &link).map(|e| (cfg.home.clone(), e));
                tokio::spawn(holder::keep(cfg, holder.clone()));
                kept = Some(holder.clone());
                Daemon::with_holder(args.upstream, args.groups, holder)
            }
            None => Daemon::new(args.upstream, args.groups),
        };
        if args.debug_spawn {
            daemon.allow_spawn();
        }
        if let Some(db) = args.db {
            daemon.set_database(db);
        }
        if let Some(token) = desktop_token {
            daemon.set_desktop_token(token);
        }
        if let Ok(addr) = listener.local_addr() {
            daemon.set_listen_addr(addr);
        }
        daemon.set_app_link(Arc::clone(&link));
        daemon.start_work(&link);
        proxy::log_upstream(&daemon).await;
        info!(port, protocol = VORND_PROTOCOL, upstream = %args.upstream, "listening");
        let mut stdout = std::io::stdout().lock();
        let mut ready = serde_json::json!({ "port": port, "protocol": VORND_PROTOCOL });
        if let Some(g) = &grid {
            ready["grid"] = g.as_str().into();
        }
        if let Some((_, e)) = &app {
            ready["app"] = e.as_str().into();
        }
        let native: Vec<&str> = daemon
            .groups()
            .modes()
            .filter(|(_, mode)| *mode == vornd::groups::Mode::Native)
            .map(|(group, _)| group)
            .collect();
        ready["native"] = native.into();
        let _ = writeln!(stdout, "{ready}");
        let _ = stdout.flush();
        drop(stdout);
        let exit_with_stdin = args.exit_with_stdin;
        // Saved as soon as a stop is asked for: the server kills a vornd still winding down.
        #[cfg(feature = "engine")]
        let saving = kept
            .as_ref()
            .and_then(|h| h.engine().map(Arc::clone))
            .zip(carry.clone());
        #[cfg(not(feature = "engine"))]
        drop(carry);
        let stop = async move {
            if exit_with_stdin {
                tokio::select! {
                    () = shutdown_signal() => {}
                    () = stdin_closed() => info!("stdin closed; stopping"),
                }
            } else {
                shutdown_signal().await;
            }
            #[cfg(feature = "engine")]
            if let Some((engine, file)) = saving {
                vornd::carry::save_now(engine.registry(), &file).await;
            }
        };
        proxy::serve(listener, daemon, stop).await;
        // The endpoints go first, so a kill during the flush leaves none.
        #[cfg(unix)]
        if let Some(endpoint) = &grid {
            let _ = std::fs::remove_file(endpoint);
        }
        #[cfg(feature = "engine")]
        if let Some((home, _endpoint)) = &app {
            vornd::control::withdraw(home);
            #[cfg(unix)]
            let _ = std::fs::remove_file(_endpoint);
        }
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
    fn implemented_groups_run_natively_unless_a_group_setting_says_otherwise() {
        use vornd::Mode;
        let plain = parse(&["--upstream", "127.0.0.1:1"]).unwrap();
        assert_eq!(plain.groups.mode("git"), Mode::Native);
        assert_eq!(plain.groups.mode("workflow"), Mode::Forward);
        let shadowed = parse(&[
            "--upstream",
            "127.0.0.1:1",
            "--groups",
            "git=shadow",
            "--db",
            "/h/vorn.db",
        ])
        .unwrap();
        assert_eq!(shadowed.groups.mode("git"), Mode::Shadow);
        assert_eq!(shadowed.groups.mode("file"), Mode::Native);
        assert_eq!(shadowed.db, Some(PathBuf::from("/h/vorn.db")));
    }

    #[test]
    fn passes_group_errors_on() {
        let err = parse(&["--upstream", "127.0.0.1:1", "--groups", "task=native"]).unwrap_err();
        assert!(err.starts_with("--groups"), "{err}");
    }
}

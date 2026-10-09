//! `vornd --data-dir DIR [--port N] [--host IP] [--web DIR] [--idle-exit] [--sessiond PATH] [--log-file PATH]`
//! runs vornd as the Vorn server ([`vornd::serve`]).
//!
//! The app passes the desktop's launch token in `VORND_DESKTOP_TOKEN`: a
//! WebSocket that opens with it is the desktop's (TP §10). It is read once
//! and taken out of the environment before anything is started, so neither
//! sessiond nor any program it runs inherits it.
//!
//! Prints one line of JSON, `{"port":N,"protocol":P}`, once it is listening, so
//! whoever started it knows where to connect. With a session holder and the
//! engine, the line also names the grid endpoint, `"grid":"<socket or pipe>"`.

use std::io::Write;
use std::net::SocketAddr;
use std::path::PathBuf;
use std::process::ExitCode;
use std::sync::{Arc, Mutex};

use tracing::{error, info};
use tracing_subscriber::EnvFilter;
use vorn_logfile::{LogFile, Rotation};
use vornd::applink::AppLink;
use vornd::holder::{self, Holder, HolderConfig};
use vornd::protocol::VORND_PROTOCOL;
use vornd::serve::ServeConfig;
use vornd::Daemon;

const USAGE: &str = "usage: vornd --data-dir DIR [--port N] [--host IP] [--web DIR] [--idle-exit] [--sessiond PATH] [--log-file PATH] [--exit-with-stdin] [--debug-spawn]

  --data-dir   serve as the Vorn server from this data directory, $VORN_HOME:
               its database, the port and credential it publishes, and the
               session holder's home
  --port       the port to listen on, for this run only (default: the one this
               install keeps, else 50091)
  --host       the address to listen on (default: every interface with Network
               Access on, else loopback)
  --web        the web client's build, served under /app
  --idle-exit  stop once nothing has used the server for a while
               ($VORN_IDLE_TIMEOUT_MS, default 30 minutes)
  --log-file   append the log here instead of stderr, rotated at 20 MiB and
               keeping 5 files; VORND_LOG sets the level
  --exit-with-stdin
               stop when stdin closes, so vornd ends with whoever started it,
               even if that process is killed
  --sessiond   the vorn-sessiond binary this build ships: keep one running under
               the data directory (its run/ directory is where running ones are
               found)
  --debug-spawn
               answer vornd:spawn from clients too, which starts a session in the
               session holder; for tests";

#[derive(Debug)]
struct Args {
    serve: ServeConfig,
    log_file: Option<String>,
    exit_with_stdin: bool,
    holder: Option<HolderConfig>,
    debug_spawn: bool,
}

fn parse_args(mut args: impl Iterator<Item = String>) -> Result<Args, String> {
    let mut log_file = None;
    let mut exit_with_stdin = false;
    let mut sessiond = None;
    let mut debug_spawn = false;
    let mut data_dir: Option<PathBuf> = None;
    let mut port = None;
    let mut host = None;
    let mut web = None;
    let mut idle_exit = false;
    while let Some(flag) = args.next() {
        let mut value = |name: &str| args.next().ok_or_else(|| format!("{name} needs a value"));
        match flag.as_str() {
            "--log-file" => log_file = Some(value("--log-file")?),
            "--exit-with-stdin" => exit_with_stdin = true,
            "--sessiond" => sessiond = Some(PathBuf::from(value("--sessiond")?)),
            "--debug-spawn" => debug_spawn = true,
            "--data-dir" => data_dir = Some(PathBuf::from(value("--data-dir")?)),
            "--port" => {
                port = Some(
                    value("--port")?
                        .parse::<u16>()
                        .map_err(|e| format!("--port: {e}"))?,
                )
            }
            "--host" => {
                host = Some(
                    value("--host")?
                        .parse::<std::net::IpAddr>()
                        .map_err(|e| format!("--host: {e}"))?,
                )
            }
            "--web" => web = Some(PathBuf::from(value("--web")?)),
            "--idle-exit" => idle_exit = true,
            "-h" | "--help" => return Err(String::new()),
            other => return Err(format!("unknown argument `{other}`")),
        }
    }
    let data_dir = data_dir.ok_or("give --data-dir, the data directory to serve")?;
    let allowed = std::env::var(vornd::serve::files::ALLOW_DEFAULT_VAR).is_ok_and(|v| v == "1");
    let user_home = std::env::var_os("HOME")
        .or_else(|| std::env::var_os("USERPROFILE"))
        .map(PathBuf::from);
    vornd::serve::files::refuse_default(&data_dir, user_home.as_deref(), allowed)?;
    // The data directory is the session holder's home.
    let holder = sessiond.map(|bundled| HolderConfig {
        home: data_dir.clone(),
        bundled,
    });
    let serve = ServeConfig {
        data_dir,
        port,
        host,
        web,
        idle: idle_exit.then(|| {
            vornd::serve::idle::window(std::env::var("VORN_IDLE_TIMEOUT_MS").ok().as_deref())
        }),
        build_channel: std::env::var("VORN_BUILD_CHANNEL")
            .ok()
            .filter(|c| c == "dev" || c == "packaged")
            .unwrap_or_else(|| "packaged".to_owned()),
        app_version: std::env::var("VORN_APP_VERSION").unwrap_or_else(|_| "unknown".to_owned()),
    };
    Ok(Args {
        serve,
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
            let file = LogFile::open(path, Rotation::DEFAULT)
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

/// The file the session records are carried in between runs, read back now and written on each change.
#[cfg(feature = "engine")]
fn carry_records(cfg: &HolderConfig, holder: &Holder) -> Option<vornd::carry::CarryFile> {
    let engine = holder.engine()?;
    let registry = engine.registry();
    registry.own_records();
    registry.set_boot_time(vornd::boot::time_ms());
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

/// The sessions an older server saved in its database, taken once and then
/// forgotten there, so they are offered by this vornd and never again.
#[cfg(feature = "engine")]
fn take_old_table(engine: &vornd::engine::Engine, db: &std::path::Path) {
    let mut store = match vorn_store::Store::open_beside(db) {
        Ok(Some(store)) => store,
        Ok(None) => return,
        Err(err) => {
            return tracing::warn!(%err, "could not open the database for the old session records")
        }
    };
    let old = match store.call("getPreviousSessions", serde_json::json!([])) {
        Ok(rows) => rows,
        Err(err) => return tracing::warn!(%err, "could not read the old session records"),
    };
    let terminals: Vec<vornd::registry::TerminalSession> = match serde_json::from_value(old) {
        Ok(t) => t,
        Err(err) => return tracing::warn!(%err, "the old session records do not read"),
    };
    if terminals.is_empty() {
        return;
    }
    let records = terminals.len();
    let carried = engine.carry_old(terminals);
    if let Err(err) = store.call("clearSessions", serde_json::json!([])) {
        tracing::warn!(%err, "could not forget the old session records");
    }
    info!(
        records,
        carried, "took the old session records from the database"
    );
}

#[cfg(not(feature = "engine"))]
fn carry_records(_: &HolderConfig, _: &Holder) -> Option<vornd::carry::CarryFile> {
    None
}

/// Where the app passes the desktop's launch token.
const DESKTOP_TOKEN_VAR: &str = "VORND_DESKTOP_TOKEN";

/// The name the app has always passed the server's credential under.
const BOOTSTRAP_VAR: &str = "SECRET_VORN_BOOTSTRAP_TOKEN";

/// The desktop's launch token, taken out of the environment, under either
/// name. Called first in `main`, while vornd has one thread and has started
/// nothing, so no program it runs inherits it.
fn take_desktop_token() -> Option<Vec<u8>> {
    let mut found = None;
    for var in [DESKTOP_TOKEN_VAR, BOOTSTRAP_VAR] {
        if let Some(token) = std::env::var_os(var) {
            std::env::remove_var(var);
            found = found.or(Some(token.into_encoded_bytes()).filter(|t| !t.is_empty()));
        }
    }
    found
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
    runtime.block_on(run(args, desktop_token))
}

async fn run(args: Args, desktop_token: Option<Vec<u8>>) -> ExitCode {
    // The data directory first, so a second vornd touches nothing.
    let config = args.serve;
    let held = match vornd::serve::files::Held::take(&config.data_dir) {
        Ok(held) => held,
        Err(_) => {
            error!(dir = %config.data_dir.display(), "another Vorn server is serving this data directory");
            return ExitCode::from(vornd::serve::files::EXIT_TAKEN);
        }
    };
    let db = match vornd::serve::open_database(&config.data_dir) {
        Ok(db) => db,
        Err(err) => {
            error!(%err, "could not start");
            return ExitCode::FAILURE;
        }
    };
    let credential = vornd::serve::files::credential(desktop_token);
    let mut kept = None;
    let mut grid: Option<String> = None;
    let link = Arc::new(AppLink::default());
    let mut carry = None;
    let daemon = match args.holder {
        Some(cfg) => {
            let home = cfg.home.clone();
            match tokio::task::spawn_blocking(move || vorn_sessiond::rundir::sweep(&home)).await {
                Ok(0) => {}
                Ok(swept) => info!(swept, "removed what dead owners left in run/"),
                Err(err) => error!(%err, "could not sweep run/"),
            }
            let holder = Arc::new(new_holder(&cfg));
            // Before the holder connects: what it holds is adopted then.
            carry = carry_records(&cfg, &holder);
            grid = serve_grid(&cfg, &holder);
            tokio::spawn(holder::keep(cfg, holder.clone()));
            kept = Some(holder.clone());
            Daemon::new(Some(holder))
        }
        None => Daemon::new(None),
    };
    if args.debug_spawn {
        daemon.allow_spawn();
    }
    #[cfg(feature = "engine")]
    if let Some(engine) = carry.as_ref().and(kept.as_ref()).and_then(|h| h.engine()) {
        let (engine, db) = (Arc::clone(engine), db.clone());
        tokio::task::spawn_blocking(move || take_old_table(&engine, &db));
    }
    daemon.set_database(db);
    daemon.set_desktop_token(credential.clone());
    let serving =
        match vornd::serve::Serving::start(config, held, daemon.native(), credential).await {
            Ok(serving) => {
                daemon.set_serving(Arc::clone(&serving));
                serving
            }
            Err(err) => {
                error!(%err, "could not start");
                return ExitCode::FAILURE;
            }
        };
    let addr = SocketAddr::new(std::net::Ipv4Addr::LOCALHOST.into(), serving.addr().port());
    let port = addr.port();
    daemon.set_listen_addr(addr);
    daemon.set_app_link(Arc::clone(&link));
    link.set_server_host(serving.addr().ip().to_string());
    daemon.start_connectors().await;
    daemon.start_work();
    daemon.start_widget();
    daemon.start_hooks().await;
    daemon.start_extensions().await;
    serving.publish();
    let native = Arc::downgrade(daemon.native());
    tokio::spawn(Arc::clone(&serving).follow_config(native.clone()));
    if let Some(window) = serving.idle_window() {
        tokio::spawn(Arc::clone(&serving).watch_idle(native, window));
    }
    ignore_hangups();
    info!(addr = %serving.addr(), protocol = VORND_PROTOCOL, "serving");
    let mut stdout = std::io::stdout().lock();
    let mut ready = serde_json::json!({ "port": port, "protocol": VORND_PROTOCOL });
    if let Some(g) = &grid {
        ready["grid"] = g.as_str().into();
    }
    let _ = writeln!(stdout, "{ready}");
    let _ = stdout.flush();
    drop(stdout);
    let exit_with_stdin = args.exit_with_stdin;
    // Saved as soon as a stop is asked for: whoever started vornd kills one still winding down.
    #[cfg(feature = "engine")]
    let saving = kept
        .as_ref()
        .and_then(|h| h.engine().map(Arc::clone))
        .zip(carry.clone());
    #[cfg(not(feature = "engine"))]
    drop(carry);
    let asked = Arc::clone(&serving);
    let stop = async move {
        if exit_with_stdin {
            tokio::select! {
                () = shutdown_signal() => {}
                () = stdin_closed() => info!("stdin closed; stopping"),
                () = asked.stopped() => {}
            }
        } else {
            tokio::select! {
                () = shutdown_signal() => {}
                () = asked.stopped() => {}
            }
        }
        #[cfg(feature = "engine")]
        if let Some((engine, file)) = saving {
            vornd::carry::save_now(engine.registry(), &file).await;
        }
    };
    let stopping = Arc::clone(&daemon);
    vornd::serve::accept(serving.listeners(), daemon, stop).await;
    serving.request_stop();
    serving.withdraw();
    stopping.stop_hooks();
    // The endpoints go first, so a kill during the flush leaves none.
    #[cfg(unix)]
    if let Some(endpoint) = &grid {
        let _ = std::fs::remove_file(endpoint);
    }
    // A clean stop checkpoints every session, so the next vornd replays nothing.
    #[cfg(feature = "engine")]
    if let Some(engine) = kept.as_ref().and_then(|h| h.engine()) {
        engine.flush().await;
    }
    drop(kept);
    info!("stopped");
    ExitCode::SUCCESS
}

/// A hangup is not a request to stop: the server outlives the terminal or
/// app that started it.
fn ignore_hangups() {
    #[cfg(unix)]
    {
        use tokio::signal::unix::{signal, SignalKind};
        if let Ok(mut hup) = signal(SignalKind::hangup()) {
            tokio::spawn(async move {
                while hup.recv().await.is_some() {
                    info!("ignoring SIGHUP; sessions keep running");
                }
            });
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn parse(list: &[&str]) -> Result<Args, String> {
        parse_args(list.iter().map(|s| s.to_string()))
    }

    #[test]
    fn needs_a_data_directory() {
        assert!(parse(&[]).is_err());
        let err = parse(&["--port", "9"]).unwrap_err();
        assert!(err.contains("--data-dir"), "{err}");
        let err = parse(&["--upstream", "127.0.0.1:1"]).unwrap_err();
        assert!(err.contains("unknown argument"), "{err}");
    }

    #[test]
    fn stays_up_without_stdin_unless_asked() {
        let dir = std::env::temp_dir().join("vornd-args");
        let dir = dir.to_str().unwrap();
        assert!(!parse(&["--data-dir", dir]).unwrap().exit_with_stdin);
        assert!(
            parse(&["--data-dir", dir, "--exit-with-stdin"])
                .unwrap()
                .exit_with_stdin
        );
    }

    #[test]
    fn keeps_its_session_holder_in_the_data_directory() {
        let dir = std::env::temp_dir().join("vornd-args");
        let dir_arg = dir.to_str().unwrap();
        assert!(parse(&["--data-dir", dir_arg]).unwrap().holder.is_none());
        let args = parse(&["--data-dir", dir_arg, "--sessiond", "/app/vorn-sessiond"]).unwrap();
        let holder = args.holder.unwrap();
        assert_eq!(holder.home, dir);
        assert_eq!(holder.bundled, PathBuf::from("/app/vorn-sessiond"));
    }
}

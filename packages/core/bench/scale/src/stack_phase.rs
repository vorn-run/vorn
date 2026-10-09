//! vornd with the session holder it launches, driven as the app is:
//! sessions started over its WebSocket, followed over the grid endpoint.

use std::path::Path;
use std::process::Stdio;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use serde_json::{json, Value};
use tokio::process::Command;

use crate::app_client::AppClient;
use crate::daemon::{explain, kill_tree, Daemon};
use crate::error::Error;
use crate::grid_client::GridClient;
use crate::holder_phase::settle;
use crate::procfs;
use crate::report::{StackPhase, Throughput};
use crate::spawn::{spawn_many, Spawner};
use crate::stats::Latency;
use crate::Plan;

const MARKER: &str = "FLOOD-DONE";
/// The local credential vornd is started with.
pub(crate) const CREDENTIAL: &str = "vorn-scale-bench";
const ATTACH_SAMPLES: usize = 20;
pub(crate) const WAIT: Duration = Duration::from_secs(60);
/// vornd's refusal while its holder is still starting.
const NOT_CONNECTED: &str = "no session holder connected";

pub(crate) type Argv = Box<dyn Fn(usize) -> Vec<String> + Send>;

pub(crate) struct AppSpawner<'a> {
    pub(crate) c: &'a mut AppClient,
    pub(crate) argv: Argv,
    pub(crate) cwd: String,
}

impl Spawner for AppSpawner<'_> {
    async fn request(&mut self, req: u64, i: usize) -> Result<(), Error> {
        let params = json!({
            "argv": (self.argv)(i),
            "cwd": self.cwd,
            "env": { "TERM": "xterm-256color", "PS1": "$ " },
            "cols": 80,
            "rows": 24,
        });
        self.c.call(req, "vornd:spawn", params).await
    }

    async fn answer(&mut self) -> Result<(u64, Result<String, String>), Error> {
        let (id, res) = self.c.answer().await?;
        let session = res.and_then(|v| {
            v.get("id")
                .and_then(Value::as_str)
                .map(str::to_owned)
                .ok_or_else(|| format!("no id in {v}"))
        });
        Ok((id, session))
    }
}

/// The JSON line vornd prints once it listens.
pub(crate) struct Ready {
    pub(crate) port: u16,
    pub(crate) grid: String,
}

pub(crate) fn ready(line: &str) -> Result<Ready, Error> {
    let v: Value = serde_json::from_str(line)
        .map_err(|e| Error::Protocol(format!("vornd said {line:?}: {e}")))?;
    let port = v
        .get("port")
        .and_then(Value::as_u64)
        .and_then(|p| u16::try_from(p).ok())
        .ok_or_else(|| Error::Protocol(format!("no port in {line}")))?;
    let grid = v
        .get("grid")
        .and_then(Value::as_str)
        .map(str::to_owned)
        .ok_or_else(|| Error::Protocol(format!("no grid in {line}; built without `engine`?")))?;
    Ok(Ready { port, grid })
}

/// The pid in the holder's announcement under `home/run`.
pub(crate) fn holder_pid(home: &Path) -> Option<u32> {
    std::fs::read_dir(home.join("run"))
        .ok()?
        .flatten()
        .filter(|e| e.file_name().to_string_lossy().starts_with("sessiond-"))
        .filter_map(|e| std::fs::read_to_string(e.path()).ok())
        .find_map(|text| info_pid(&text))
}

fn info_pid(text: &str) -> Option<u32> {
    text.lines()
        .find_map(|l| l.strip_prefix("pid="))
        .and_then(|p| p.trim().parse().ok())
}

/// Runs the stack phase into `out`, which keeps whatever was measured when
/// an error ends it early.
pub async fn run(plan: &Plan, out: &mut StackPhase) -> Result<(), Error> {
    let home = plan.work.join("stack");
    out.mem_available.base = procfs::mem_available().ok();
    let (mut vornd, line) = start(&plan.vornd, &plan.sessiond, &home).await?;
    let res = match measure(plan, &home, &vornd, &line, out).await {
        Err(e) => {
            let e = explain(e, std::slice::from_mut(&mut vornd));
            match holder_pid(&home).filter(|&p| procfs::usage(p).is_err()) {
                Some(_) => Err(Error::Failed(format!("{e} (the holder exited)"))),
                None => Err(e),
            }
        }
        ok => ok,
    };
    let holder = holder_pid(&home);
    vornd.stop().await;
    if let Some(pid) = holder {
        kill_tree(pid);
    }
    res
}

/// Starts vornd, with `sessiond` as its holder, in a fresh `home`.
pub(crate) async fn start(
    vornd: &Path,
    sessiond: &Path,
    home: &Path,
) -> Result<(Daemon, String), Error> {
    let _ = std::fs::remove_dir_all(home);
    std::fs::create_dir_all(home)?;
    let mut cmd = Command::new(vornd);
    cmd.arg("--data-dir")
        .arg(home)
        .args(["--port", "0", "--debug-spawn", "--sessiond"])
        .arg(sessiond)
        .arg("--log-file")
        .arg(home.join("vornd.log"))
        .env("VORND_LOG", "warn")
        .env("VORN_HOME", home)
        .env("HOME", home)
        .env("SECRET_VORN_BOOTSTRAP_TOKEN", CREDENTIAL)
        .env("VORND_KEYCHAIN", "0")
        .stdin(Stdio::null())
        .stderr(Stdio::inherit());
    Daemon::start("vornd", cmd).await
}

/// The first session, asked for again while vornd is still connecting to
/// the holder it launched, which it refuses spawns until it has. Answers
/// the session and the next free request number.
pub(crate) async fn first_spawn(s: &mut AppSpawner<'_>) -> Result<(String, u64), Error> {
    let deadline = Instant::now() + WAIT;
    let mut req = 1;
    loop {
        let mut probe = spawn_many(s, 1, 1, req).await?;
        req += 1;
        if let Some(id) = probe.ids.pop() {
            return Ok((id, req));
        }
        let why = probe.refused.unwrap_or_default();
        if !why.contains(NOT_CONNECTED) || Instant::now() > deadline {
            return Err(Error::Failed(why));
        }
        tokio::time::sleep(Duration::from_millis(200)).await;
    }
}

async fn measure(
    plan: &Plan,
    home: &Path,
    vornd: &Daemon,
    line: &str,
    out: &mut StackPhase,
) -> Result<(), Error> {
    let ready = ready(line)?;
    let mut app = AppClient::connect(ready.port, CREDENTIAL).await?;
    let mut grid = GridClient::connect(&ready.grid, WAIT).await?;
    let cwd = plan.work.display().to_string();
    let bash: Argv = Box::new(|_| {
        ["bash", "--norc", "--noprofile", "-i"]
            .map(String::from)
            .to_vec()
    });
    let mut s = AppSpawner {
        c: &mut app,
        argv: bash,
        cwd: cwd.clone(),
    };
    let (probe, _) = first_spawn(&mut s).await?;
    let holder = holder_pid(home).ok_or(Error::Failed("no holder announced".into()))?;
    out.vornd.base = procfs::usage(vornd.pid).ok();
    out.holder.base = procfs::usage(holder).ok();

    let (idle, busy) = plan.split();
    let spawned = spawn_many(&mut s, idle, 32, 1_000).await?;
    out.idle = spawned.ids.len();
    out.spawn = Some(spawned.rate);
    let idle_ids = spawned.ids;
    if let Some(why) = spawned.refused {
        return Err(Error::Failed(format!("spawning idle sessions: {why}")));
    }
    settle(plan.tier).await;
    out.vornd.idle = procfs::usage(vornd.pid).ok();
    out.holder.idle = procfs::usage(holder).ok();
    out.mem_available.idle = procfs::mem_available().ok();

    let step = (idle_ids.len() / ATTACH_SAMPLES).max(1);
    let mut took = Vec::new();
    for id in idle_ids.iter().step_by(step).take(ATTACH_SAMPLES) {
        let (sid, t) = grid.attach(id, WAIT).await?;
        took.push(t);
        grid.detach(sid);
    }
    out.attach = Some(Latency::of(&mut took));

    let (sid, _) = grid.attach(&probe, WAIT).await?;
    let deadline = Instant::now() + WAIT;
    while !grid.text(sid).contains('$') {
        grid.pump(deadline, "the probe's prompt").await?;
    }
    out.probe_idle = Some(probe_latency(&mut grid, sid, plan.probes).await?);

    let me = plan.me.display().to_string();
    let rate = plan.busy_rate.to_string();
    let mut s = AppSpawner {
        c: &mut app,
        argv: Box::new(move |i| {
            let seed = i.to_string();
            [&me, "buildlog", "--rate", &rate, "--seed", &seed]
                .map(String::from)
                .to_vec()
        }),
        cwd: cwd.clone(),
    };
    let spawned = spawn_many(&mut s, busy, 32, 1_000_000).await?;
    out.busy = spawned.ids.len();
    if let Some(why) = spawned.refused {
        return Err(Error::Failed(format!("spawning busy sessions: {why}")));
    }
    settle(plan.tier).await;
    // Read before the probe, which is what gives out first under load.
    out.vornd.busy = procfs::usage(vornd.pid).ok();
    out.holder.busy = procfs::usage(holder).ok();
    out.mem_available.busy = procfs::mem_available().ok();
    out.probe_busy = Some(probe_latency(&mut grid, sid, plan.probes).await?);
    grid.detach(sid);

    out.throughput = Some(flood(plan, &mut app, &mut grid, &cwd).await?);
    Ok(())
}

/// Types into bash and times each echo to the frame that shows it: a
/// character, then a backspace, so the line never grows.
async fn probe_latency(grid: &mut GridClient, sid: u32, n: usize) -> Result<Latency, Error> {
    let mut took = Vec::with_capacity(n);
    for i in 0..n {
        grid.drain(Duration::from_millis(20)).await?;
        let bytes: &[u8] = if i % 2 == 0 { b"x" } else { b"\x7f" };
        took.push(
            grid.type_and_wait(sid, bytes, Duration::from_secs(30))
                .await?,
        );
    }
    Ok(Latency::of(&mut took))
}

/// Sessions printing a build log as fast as they can, all followed by one
/// grid client: from their common start until the client shows each one's
/// last line.
async fn flood(
    plan: &Plan,
    app: &mut AppClient,
    grid: &mut GridClient,
    cwd: &str,
) -> Result<Throughput, Error> {
    let n = plan.flood_sessions;
    let lead = Duration::from_millis(3_000 + 200 * n as u64);
    let start_at = SystemTime::now() + lead;
    let at_ms = start_at
        .duration_since(UNIX_EPOCH)
        .map_err(|e| Error::Failed(e.to_string()))?
        .as_millis()
        .to_string();
    let me = plan.me.display().to_string();
    let total = plan.flood_bytes.to_string();
    let mut s = AppSpawner {
        c: app,
        argv: Box::new(move |i| {
            let seed = i.to_string();
            [
                &me,
                "buildlog",
                "--rate",
                "0",
                "--total",
                &total,
                "--marker",
                MARKER,
                "--start-at",
                &at_ms,
                "--seed",
                &seed,
            ]
            .map(String::from)
            .to_vec()
        }),
        cwd: cwd.to_owned(),
    };
    let spawned = spawn_many(&mut s, n, n, 2_000_000).await?;
    if let Some(why) = spawned.refused {
        return Err(Error::Failed(format!("spawning flood sessions: {why}")));
    }
    let mut sids = Vec::with_capacity(n);
    let mut before = (0, 0);
    for id in &spawned.ids {
        let (sid, _) = grid.attach(id, WAIT).await?;
        if let Some(p) = grid.pane(sid) {
            before = (before.0 + p.frames, before.1 + p.frame_bytes);
        }
        sids.push(sid);
    }
    let late = SystemTime::now().duration_since(start_at).is_ok();
    if late {
        eprintln!("vorn-scale-bench: flood attaches ran past the start");
    }
    let deadline = Instant::now() + lead + Duration::from_secs(600);
    let mut left = sids.clone();
    while !left.is_empty() {
        grid.pump(deadline, "the flood's last lines").await?;
        left.retain(|&sid| !grid.text(sid).contains(MARKER));
    }
    let secs = SystemTime::now()
        .duration_since(start_at)
        .unwrap_or_default()
        .as_secs_f64();
    let (frames, frame_bytes) = sids
        .iter()
        .filter_map(|&sid| grid.pane(sid))
        .fold((0, 0), |(f, b), p| (f + p.frames, b + p.frame_bytes));
    for sid in sids {
        grid.detach(sid);
    }
    Ok(Throughput {
        sessions: n,
        bytes: plan.flood_bytes * n as u64,
        secs,
        frames: frames - before.0,
        frame_bytes: frame_bytes - before.1,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_ready_line_names_the_port_and_the_grid_socket() {
        let r = ready(r#"{"port":1,"protocol":3,"grid":"/h/g.sock"}"#).unwrap();
        assert_eq!((r.port, r.grid.as_str()), (1, "/h/g.sock"));
        assert!(ready(r#"{"port":1}"#).is_err());
        assert!(ready("listening").is_err());
    }

    #[test]
    fn the_holder_pid_comes_from_its_announcement() {
        assert_eq!(
            info_pid("endpoint=/r/s.sock\npid=4242\nproto=3\n"),
            Some(4242)
        );
        assert_eq!(info_pid("endpoint=/r/s.sock\n"), None);
    }
}

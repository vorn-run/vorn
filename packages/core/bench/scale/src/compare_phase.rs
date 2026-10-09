//! Runs the comparison method against one build of vornd and its holder.
//! Each terminal count gets a fresh vornd; the 1,000-terminal dash one
//! then times shell commands beside busy terminals.

use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

use serde_json::json;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpStream;

use crate::app_client::AppClient;
use crate::compare::{Comparison, Memory, Parts, Response};
use crate::daemon::{explain, kill_tree, Daemon};
use crate::error::Error;
use crate::grid_client::GridClient;
use crate::procfs;
use crate::report::Limit;
use crate::spawn::spawn_many;
use crate::stack_phase::{first_spawn, holder_pid, ready, start, AppSpawner, CREDENTIAL, WAIT};
use crate::stats::Latency;

/// What each terminal prints before it waits at a prompt.
const LINES: &str = r#"i=1; while [ $i -le 1000 ]; do echo "line $i"; i=$((i+1)); done"#;
const LAST_LINE: &str = "line 1000";
const BUSY: &[u8] = b"while :; do date; sleep 0.1; done\r";
/// How long a terminal sits untouched before the idle reading.
const IDLE: Duration = Duration::from_secs(60);
/// The longest the idle reading waits, past [`IDLE`], to settle.
const IDLE_SETTLE: Duration = Duration::from_secs(180);
/// vornd and the holder count as quiet below this share of one CPU per
/// 1,000 terminals: their housekeeping grows with the count.
const QUIET_SHARE: f64 = 0.03;
/// Past this the readings go ahead, flagged as taken while not quiet.
const QUIET_WITHIN: Duration = Duration::from_secs(300);
/// How long one command may take to answer.
const ANSWER_WITHIN: Duration = Duration::from_secs(10);

/// The comparison's settings.
#[derive(Debug, Clone, PartialEq)]
pub struct Spec {
    pub sessiond: PathBuf,
    pub vornd: PathBuf,
    pub work: PathBuf,
    pub out: PathBuf,
    /// Terminal counts measured with dash.
    pub sizes: Vec<usize>,
    /// Terminals measured with bash too; 0 skips it.
    pub bash: usize,
    /// Terminals the response time is taken among.
    pub response: usize,
    /// Busy terminal counts the response time is taken beside.
    pub busy: Vec<usize>,
    pub probes: usize,
}

impl Spec {
    pub fn largest(&self) -> usize {
        self.sizes
            .iter()
            .copied()
            .chain([self.bash, self.response])
            .max()
            .unwrap_or(0)
    }
}

/// Runs every configuration, writing what it has after each so a later
/// failure keeps the earlier readings.
pub async fn run(spec: &Spec, out: &mut Comparison, save: &dyn Fn(&Comparison)) {
    let mut configs: Vec<(&str, usize)> = Vec::new();
    if spec.bash > 0 {
        configs.push(("bash", spec.bash));
    }
    configs.extend(spec.sizes.iter().map(|&n| ("dash", n)));
    if !spec.busy.is_empty() && !spec.sizes.contains(&spec.response) {
        configs.push(("dash", spec.response));
    }
    for (shell, n) in configs {
        let responds = shell == "dash" && n == spec.response;
        let mut m = Memory {
            shell: shell.to_owned(),
            terminals: n,
            ..Memory::default()
        };
        if let Err(e) = one(spec, shell, n, responds, &mut m, out).await {
            eprintln!("vorn-scale-bench: compare {shell} × {n}: {e}");
            out.limits.push(Limit {
                phase: format!("{shell} memory"),
                sessions: n,
                what: e.to_string(),
            });
        }
        if spec.sizes.contains(&n) || shell == "bash" {
            out.memory.push(m);
        }
        save(out);
    }
}

async fn one(
    spec: &Spec,
    shell: &str,
    n: usize,
    responds: bool,
    m: &mut Memory,
    out: &mut Comparison,
) -> Result<(), Error> {
    let home = spec.work.join(format!("compare-{shell}-{n}"));
    let (mut vornd, line) = start(&spec.vornd, &spec.sessiond, &home).await?;
    let res = match measure(spec, shell, n, responds, &home, &vornd, &line, m, out).await {
        Err(e) => Err(explain(e, std::slice::from_mut(&mut vornd))),
        ok => ok,
    };
    let holder = holder_pid(&home);
    vornd.stop().await;
    if let Some(pid) = holder {
        kill_tree(pid);
    }
    let _ = std::fs::remove_dir_all(&home);
    res
}

#[allow(clippy::too_many_arguments)]
async fn measure(
    spec: &Spec,
    shell: &str,
    n: usize,
    responds: bool,
    home: &Path,
    vornd: &Daemon,
    line: &str,
    m: &mut Memory,
    out: &mut Comparison,
) -> Result<(), Error> {
    let ready = ready(line)?;
    let mut app = AppClient::connect(ready.port, CREDENTIAL).await?;
    let program = match shell {
        "bash" => format!("{LINES}; exec bash --norc --noprofile -i"),
        _ => format!("{LINES}; exec dash -i"),
    };
    let shell = shell.to_owned();
    let mut s = AppSpawner {
        c: &mut app,
        argv: Box::new(move |_| vec![shell.clone(), "-c".into(), program.clone()]),
        cwd: spec.work.display().to_string(),
    };
    let (first, req) = first_spawn(&mut s).await?;
    let spawned = spawn_many(&mut s, n - 1, 32, req).await?;
    if let Some(why) = spawned.refused {
        return Err(Error::Failed(format!(
            "{} started, then: {why}",
            spawned.ids.len() + 1
        )));
    }
    let mut ids = vec![first];
    ids.extend(spawned.ids);
    let holder = holder_pid(home).ok_or(Error::Failed("no holder announced".into()))?;
    let roots = [vornd.pid, holder];

    m.quiet = quiet(&roots, n).await;
    {
        let mut grid = GridClient::connect(&ready.grid, WAIT).await?;
        for id in [&ids[0], &ids[n - 1]] {
            let (sid, _) = grid.attach(id, WAIT).await?;
            grid.drain(Duration::from_millis(300)).await?;
            if !grid.text(sid).contains(LAST_LINE) {
                return Err(Error::Failed(format!(
                    "{id} never printed {LAST_LINE:?}: {}",
                    grid.text(sid)
                )));
            }
            grid.detach(sid);
        }
    }
    let quiet_at = Instant::now();
    let (live, processes) = rss(&roots);
    m.live = Some(live.total());
    m.live_parts = Some(live);
    m.processes = processes;

    tokio::time::sleep(IDLE).await;
    let mut last = rss(&roots).0.total();
    let until = Instant::now() + IDLE_SETTLE;
    loop {
        tokio::time::sleep(Duration::from_secs(5)).await;
        let (now, _) = rss(&roots);
        if now.total().abs_diff(last) * 100 < last || Instant::now() > until {
            m.idle = Some(now.total());
            m.idle_parts = Some(now);
            break;
        }
        last = now.total();
    }
    m.idle_after_secs = quiet_at.elapsed().as_secs_f64();
    m.asleep = asleep(ready.port).await.ok();

    if responds {
        respond(spec, &mut app, &ready.grid, &ids, out).await?;
    }
    Ok(())
}

/// Times commands typed into the first terminal while more and more of
/// the others run a `date` loop.
async fn respond(
    spec: &Spec,
    app: &mut AppClient,
    endpoint: &str,
    ids: &[String],
    out: &mut Comparison,
) -> Result<(), Error> {
    let mut grid = GridClient::connect(endpoint, WAIT).await?;
    let (sid, _) = grid.attach(&ids[0], WAIT).await?;
    grid.drain(Duration::from_millis(300)).await?;
    let mut busy = 0;
    let mut req = 1_000_000;
    let mut nonce = 0u64;
    for &want in &spec.busy {
        let want = want.min(ids.len() - 1);
        while busy < want {
            busy += 1;
            write(app, req, &ids[busy], BUSY).await?;
            req += 1;
        }
        tokio::time::sleep(Duration::from_secs(2)).await;
        grid.drain(Duration::from_millis(100)).await?;
        let cpu0 = procfs::machine_ticks()?;
        let mut took = Vec::with_capacity(spec.probes);
        for _ in 0..spec.probes {
            nonce += 1;
            let cmd = format!("echo r$(({nonce}+0))e\r");
            took.push(
                grid.command(sid, cmd.as_bytes(), &format!("r{nonce}e"), ANSWER_WITHIN)
                    .await?,
            );
            grid.drain(Duration::from_millis(20)).await?;
        }
        let cpu1 = procfs::machine_ticks()?;
        out.response.push(Response {
            terminals: ids.len(),
            busy,
            latency: Some(Latency::of(&mut took)),
            cpu: share(cpu0, cpu1),
        });
    }
    Ok(())
}

async fn write(app: &mut AppClient, req: u64, id: &str, data: &[u8]) -> Result<(), Error> {
    let data = String::from_utf8_lossy(data);
    app.call(req, "terminal:write", json!({ "id": id, "data": data }))
        .await?;
    loop {
        let (got, res) = tokio::time::timeout(WAIT, app.answer())
            .await
            .map_err(|_| Error::Timeout(format!("writing to {id}")))??;
        if got == req {
            return res.map(|_| ()).map_err(Error::Failed);
        }
    }
}

/// Waits until `roots` use less than [`QUIET_SHARE`] of a CPU per 1,000
/// of the `n` terminals for three seconds running: every terminal has
/// printed and been taken in. False if that took past [`QUIET_WITHIN`].
async fn quiet(roots: &[u32], n: usize) -> bool {
    let limit = QUIET_SHARE * clock_ticks() * (n as f64 / 1000.0).max(1.0);
    let ticks = || {
        roots
            .iter()
            .filter_map(|&p| procfs::cpu_ticks(p).ok())
            .sum::<u64>()
    };
    let until = Instant::now() + QUIET_WITHIN;
    let mut last = ticks();
    let mut calm = 0;
    while calm < 3 {
        if Instant::now() > until {
            return false;
        }
        tokio::time::sleep(Duration::from_secs(1)).await;
        let now = ticks();
        calm = if (now.saturating_sub(last) as f64) < limit {
            calm + 1
        } else {
            0
        };
        last = now;
    }
    true
}

/// RssAnon of vornd (`roots[0]`), the holder (`roots[1]`) and everything
/// they started, and how many processes that was.
fn rss(roots: &[u32; 2]) -> (Parts, usize) {
    let mut rest = procfs::descendants(roots);
    rest.sort_unstable();
    rest.dedup();
    rest.retain(|p| !roots.contains(p));
    let read = |p: u32| procfs::rss_anon(p).ok();
    let (programs, n) = rest
        .iter()
        .filter_map(|&p| read(p))
        .fold((0, 0), |(sum, n), v| (sum + v, n + 1));
    let parts = Parts {
        vornd: read(roots[0]).unwrap_or(0),
        holder: read(roots[1]).unwrap_or(0),
        programs,
    };
    (parts, n + 2)
}

/// How many sessions vornd's debug report has asleep.
async fn asleep(port: u16) -> Result<usize, Error> {
    let mut conn = TcpStream::connect(("127.0.0.1", port)).await?;
    conn.write_all(b"GET /vornd/sessions HTTP/1.1\r\nHost: 127.0.0.1\r\nConnection: close\r\n\r\n")
        .await?;
    let mut body = String::new();
    tokio::time::timeout(WAIT, conn.read_to_string(&mut body))
        .await
        .map_err(|_| Error::Timeout("vornd's session report".into()))??;
    Ok(body.matches("\"asleep\":true").count())
}

/// Busy share of the machine's CPU between two `/proc/stat` readings.
fn share((busy0, total0): (u64, u64), (busy1, total1): (u64, u64)) -> Option<f64> {
    let total = total1.checked_sub(total0).filter(|&t| t > 0)?;
    Some(busy1.saturating_sub(busy0) as f64 * 100.0 / total as f64)
}

fn clock_ticks() -> f64 {
    // SAFETY: sysconf reads a constant and has no preconditions.
    let hz = unsafe { libc::sysconf(libc::_SC_CLK_TCK) };
    if hz > 0 {
        hz as f64
    } else {
        100.0
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn cpu_share_is_busy_over_total() {
        assert_eq!(share((10, 100), (60, 300)), Some(25.0));
        assert_eq!(share((10, 100), (10, 100)), None);
    }
}

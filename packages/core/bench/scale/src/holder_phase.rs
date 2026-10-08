//! The session holder on its own: the bench starts `vorn-sessiond` and
//! talks to it as vornd does, so what is measured is the holder's cost
//! alone.

use std::path::Path;
use std::process::Stdio;
use std::time::{Duration, Instant};

use tokio::process::Command;
use vorn_sessiond_wire::{Adopt, Io, Spawn, SpawnSpec, ToSessiond, ToVornd, Write};

use crate::daemon::Daemon;
use crate::error::Error;
use crate::holder_client::HolderClient;
use crate::procfs;
use crate::report::{Handoff, HolderPhase};
use crate::spawn::{spawn_many, Spawner};
use crate::stats::Latency;
use crate::Plan;

/// What a session runs, by its number.
type Argv = Box<dyn Fn(usize) -> Vec<String> + Send>;

struct HolderSpawner<'a> {
    c: &'a mut HolderClient,
    argv: Argv,
    cwd: String,
}

impl Spawner for HolderSpawner<'_> {
    async fn request(&mut self, req: u64, i: usize) -> Result<(), Error> {
        self.c.send(ToSessiond::Spawn(Spawn {
            req,
            spec: SpawnSpec {
                argv: (self.argv)(i),
                cwd: self.cwd.clone(),
                env: vec![("TERM".into(), "xterm-256color".into())],
                io: Io::Pty { cols: 80, rows: 24 },
                ring_bytes: None,
            },
        }))
    }

    async fn answer(&mut self) -> Result<(u64, Result<String, String>), Error> {
        loop {
            match self.c.next(Duration::from_secs(600)).await? {
                ToVornd::Spawned(s) => return Ok((s.req, Ok(s.session))),
                ToVornd::Failed(f) => return Ok((f.req, Err(f.error))),
                _ => {}
            }
        }
    }
}

/// Runs the holder phase into `out`, which keeps whatever was measured
/// when an error ends it early.
pub async fn run(plan: &Plan, out: &mut HolderPhase) -> Result<(), Error> {
    let home = plan.work.join("holder");
    let _ = std::fs::remove_dir_all(&home);
    std::fs::create_dir_all(&home)?;
    let (a, line) = start_holder(plan, &home).await?;
    let mut daemons = vec![a];
    let res = match measure(plan, &home, endpoint(&line)?, &mut daemons, out).await {
        Err(e) => Err(crate::daemon::explain(e, &mut daemons)),
        ok => ok,
    };
    for d in daemons {
        d.stop().await;
    }
    res
}

async fn measure(
    plan: &Plan,
    home: &Path,
    a_endpoint: String,
    daemons: &mut Vec<Daemon>,
    out: &mut HolderPhase,
) -> Result<(), Error> {
    let pid = daemons[0].pid;
    let mut c = HolderClient::connect(&a_endpoint).await?;
    out.holder.base = Some(procfs::usage(pid)?);
    let cwd = plan.work.display().to_string();

    let (idle, busy) = plan.split();
    let mut s = HolderSpawner {
        c: &mut c,
        argv: Box::new(|_| vec!["cat".into()]),
        cwd: cwd.clone(),
    };
    let probe = spawn_many(&mut s, 1, 1, 1).await?;
    let probe = probe
        .ids
        .into_iter()
        .next()
        .ok_or_else(|| Error::Failed(probe.refused.unwrap_or_default()))?;
    let spawned = spawn_many(&mut s, idle, 64, 1_000).await?;
    out.idle = spawned.ids.len();
    out.spawn = Some(spawned.rate);
    if let Some(why) = spawned.refused {
        return Err(Error::Failed(format!("spawning idle sessions: {why}")));
    }
    settle(plan.tier).await;
    out.holder.idle = Some(procfs::usage(pid)?);

    c.attach(&probe, true)?;
    out.probe_idle = Some(probe_latency(&mut c, &probe, plan.probes).await?);

    let me = plan.me.display().to_string();
    let rate = plan.busy_rate.to_string();
    let mut s = HolderSpawner {
        c: &mut c,
        argv: Box::new(move |i| {
            let seed = i.to_string();
            [&me, "buildlog", "--rate", &rate, "--seed", &seed]
                .map(String::from)
                .to_vec()
        }),
        cwd,
    };
    let spawned = spawn_many(&mut s, busy, 64, 1_000_000).await?;
    out.busy = spawned.ids.len();
    for id in &spawned.ids {
        c.attach(id, false)?;
    }
    if let Some(why) = spawned.refused {
        return Err(Error::Failed(format!("spawning busy sessions: {why}")));
    }
    settle(plan.tier).await;
    let before = c.received();
    out.probe_busy = Some(probe_latency(&mut c, &probe, plan.probes).await?);
    out.streamed_bytes = c.received() - before;
    out.holder.busy = Some(procfs::usage(pid)?);

    let (b, line) = start_holder(plan, home).await?;
    let b_pid = b.pid;
    daemons.push(b);
    let mut cb = HolderClient::connect(&endpoint(&line)?).await?;
    let t = Instant::now();
    cb.send(ToSessiond::Adopt(Adopt {
        req: 1,
        from: a_endpoint,
    }))?;
    loop {
        match cb.next(Duration::from_secs(900)).await? {
            ToVornd::Adopted(a) => {
                out.handoff = Some(Handoff {
                    sessions: a.sessions.len(),
                    ms: crate::stats::ms(t.elapsed()),
                    adopter: procfs::usage(b_pid).ok(),
                });
                return Ok(());
            }
            ToVornd::Failed(f) => return Err(Error::Failed(format!("handoff: {}", f.error))),
            _ => {}
        }
    }
}

async fn start_holder(plan: &Plan, home: &Path) -> Result<(Daemon, String), Error> {
    let mut cmd = Command::new(&plan.sessiond);
    cmd.arg("--home")
        .arg(home)
        .args(["--idle-exit", "86400"])
        .stdin(Stdio::null())
        .stderr(Stdio::inherit());
    Daemon::start("vorn-sessiond", cmd).await
}

fn endpoint(line: &str) -> Result<String, Error> {
    line.strip_prefix("listening ")
        .map(str::to_owned)
        .ok_or_else(|| Error::Protocol(format!("vorn-sessiond said {line:?}")))
}

/// Gives freshly started programs time to reach their steady state before
/// anything is read: a second per thousand sessions, three at least.
pub async fn settle(sessions: usize) {
    tokio::time::sleep(Duration::from_millis(3_000 + sessions as u64)).await;
}

/// Types into `cat` and times each echo: a character, then a backspace,
/// so the line never grows.
async fn probe_latency(c: &mut HolderClient, probe: &str, n: usize) -> Result<Latency, Error> {
    let mut took = Vec::with_capacity(n);
    for i in 0..n {
        c.drain();
        let bytes = if i % 2 == 0 {
            b"x".to_vec()
        } else {
            vec![0x7f]
        };
        let t = Instant::now();
        c.send(ToSessiond::Write(Write {
            session: probe.to_owned(),
            input_seq: i as u64 + 1,
            bytes,
        }))?;
        loop {
            match c.next(Duration::from_secs(30)).await? {
                ToVornd::Entries(e) if e.session == probe => break,
                _ => {}
            }
        }
        took.push(t.elapsed());
        tokio::time::sleep(Duration::from_millis(20)).await;
    }
    Ok(Latency::of(&mut took))
}

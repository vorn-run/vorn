//! An isolated vornd: its own vorn-sessiond under `/tmp/vorn-spike-<pid>`,
//! never the user's. Sessions are started over vornd's app channel with
//! `vornd:spawn`, the way the app's server starts them.

use std::io::{BufRead, BufReader, Read, Write};
use std::os::unix::net::UnixStream;
use std::path::PathBuf;
use std::process::{Child, Command, Stdio};
use std::time::{Duration, Instant};

use serde_json::{json, Value};

pub struct Daemon {
    pub home: PathBuf,
    pub grid: String,
    pub app: String,
    pub vornd_pid: i32,
    child: Child,
}

fn binaries() -> (PathBuf, PathBuf) {
    let t = crate::repo_root().join("packages/core/target");
    let pick = |name: &str| {
        for profile in ["ci", "release"] {
            let p = t.join(profile).join(name);
            if p.exists() {
                return p;
            }
        }
        t.join("ci").join(name)
    };
    (pick("vornd"), pick("vorn-sessiond"))
}

impl Daemon {
    pub fn start() -> Result<Daemon, String> {
        let home = PathBuf::from(format!("/tmp/vorn-spike-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&home);
        std::fs::create_dir_all(&home).map_err(|e| e.to_string())?;
        let (vornd, sessiond) = binaries();
        let mut child = Command::new(&vornd)
            // Nothing listens there: every call vornd does not answer itself
            // fails, and the spike makes none.
            .args(["--upstream", "127.0.0.1:9", "--exit-with-stdin"])
            .arg("--sessiond")
            .arg(&sessiond)
            .arg("--home")
            .arg(&home)
            .arg("--log-file")
            .arg(home.join("vornd.log"))
            .env("TMPDIR", "/tmp")
            .env("VORN_HOME", &home)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::null())
            .spawn()
            .map_err(|e| format!("{}: {e}", vornd.display()))?;
        let mut line = String::new();
        BufReader::new(child.stdout.take().unwrap())
            .read_line(&mut line)
            .map_err(|e| e.to_string())?;
        let ready: Value = serde_json::from_str(line.trim())
            .map_err(|e| format!("vornd's ready line {line:?}: {e}"))?;
        let grid = ready["grid"].as_str().ok_or("vornd has no grid endpoint")?;
        let app = ready["app"].as_str().ok_or("vornd has no app endpoint")?;
        Ok(Daemon {
            home,
            grid: grid.to_owned(),
            app: app.to_owned(),
            vornd_pid: child.id() as i32,
            child,
        })
    }

    /// vorn-sessiond under this home, found by its command line.
    pub fn sessiond_pid(&self) -> Option<i32> {
        let out = Command::new("ps")
            .args(["-A", "-o", "pid=,command="])
            .output()
            .ok()?;
        let home = self.home.to_string_lossy();
        String::from_utf8_lossy(&out.stdout).lines().find_map(|l| {
            let l = l.trim();
            let (pid, cmd) = l.split_once(' ')?;
            let exe = cmd.split_whitespace().next()?;
            (exe.ends_with("/vorn-sessiond") && cmd.contains(home.as_ref()))
                .then(|| pid.parse().ok())
                .flatten()
        })
    }

    /// Starts one session per program; answers their ids in order.
    pub fn spawn_programs(&self, progs: &[Vec<String>]) -> Result<Vec<String>, String> {
        let mut s = UnixStream::connect(&self.app).map_err(|e| format!("app channel: {e}"))?;
        s.set_read_timeout(Some(Duration::from_secs(20))).ok();
        let env = json!({
            "PATH": "/usr/bin:/bin:/usr/sbin:/sbin",
            "TERM": "xterm-256color",
            "HOME": self.home,
            "TMPDIR": "/tmp",
            "LANG": "en_US.UTF-8",
            "PS1": "$ ",
        });
        let mut ids = Vec::new();
        let mut frames = Vec::new();
        for (i, argv) in progs.iter().enumerate() {
            let req = json!({
                "jsonrpc": "2.0",
                "id": i + 1,
                "method": "vornd:spawn",
                "params": {
                    "argv": argv, "cols": 80, "rows": 24, "cwd": self.home, "env": env,
                    "name": format!("spike-{i}"),
                },
            });
            let body = req.to_string();
            let mut f = ((body.len() + 1) as u32).to_le_bytes().to_vec();
            f.push(1);
            f.extend_from_slice(body.as_bytes());
            s.write_all(&f).map_err(|e| e.to_string())?;
            let t = Instant::now();
            let id = loop {
                if t.elapsed() > Duration::from_secs(20) {
                    return Err(format!("no answer to spawn {i}"));
                }
                let v = read_json(&mut s, &mut frames)?;
                if v["id"].as_u64() == Some(i as u64 + 1) {
                    if let Some(e) = v.get("error") {
                        // The holder may still be connecting.
                        if t.elapsed() < Duration::from_secs(15) {
                            std::thread::sleep(Duration::from_millis(100));
                            s.write_all(&f).map_err(|e| e.to_string())?;
                            continue;
                        }
                        return Err(format!("spawn {i}: {e}"));
                    }
                    break v["result"]["id"]
                        .as_str()
                        .ok_or_else(|| format!("spawn {i}: {v}"))?
                        .to_owned();
                }
            };
            ids.push(id);
        }
        Ok(ids)
    }

    pub fn stop(mut self) {
        let sessiond = self.sessiond_pid();
        let doomed = sessiond.map(descendants).unwrap_or_default();
        drop(self.child.stdin.take());
        let t = Instant::now();
        while t.elapsed() < Duration::from_secs(5) {
            if let Ok(Some(_)) = self.child.try_wait() {
                break;
            }
            std::thread::sleep(Duration::from_millis(50));
        }
        let _ = self.child.kill();
        let _ = self.child.wait();
        if let Some(pid) = sessiond {
            let _ = Command::new("kill").arg(pid.to_string()).stderr(Stdio::null()).status();
        }
        for pid in doomed {
            let _ = Command::new("kill").args(["-9", &pid.to_string()]).stderr(Stdio::null()).status();
        }
        std::thread::sleep(Duration::from_millis(200));
        let _ = std::fs::remove_dir_all(&self.home);
    }
}

/// The next JSON frame on the app channel.
fn read_json(s: &mut UnixStream, buf: &mut Vec<u8>) -> Result<Value, String> {
    loop {
        if buf.len() >= 4 {
            let len = u32::from_le_bytes([buf[0], buf[1], buf[2], buf[3]]) as usize;
            if buf.len() >= 4 + len {
                let kind = buf[4];
                let payload = buf[5..4 + len].to_vec();
                buf.drain(..4 + len);
                if kind == 1 {
                    return serde_json::from_slice(&payload).map_err(|e| e.to_string());
                }
                continue;
            }
        }
        let mut chunk = [0u8; 65536];
        let n = s.read(&mut chunk).map_err(|e| format!("app channel: {e}"))?;
        if n == 0 {
            return Err("the app channel closed".into());
        }
        buf.extend_from_slice(&chunk[..n]);
    }
}

/// Every process below `root` (the sessions and what they run).
fn descendants(root: i32) -> Vec<i32> {
    let Ok(out) = Command::new("ps").args(["-A", "-o", "pid=,ppid="]).output() else {
        return Vec::new();
    };
    let pairs: Vec<(i32, i32)> = String::from_utf8_lossy(&out.stdout)
        .lines()
        .filter_map(|l| {
            let mut it = l.split_whitespace().map(|x| x.parse::<i32>());
            Some((it.next()?.ok()?, it.next()?.ok()?))
        })
        .collect();
    let mut found = vec![root];
    let mut i = 0;
    while i < found.len() {
        let p = found[i];
        found.extend(pairs.iter().filter(|(_, pp)| *pp == p).map(|(c, _)| *c));
        i += 1;
    }
    found.remove(0);
    found
}

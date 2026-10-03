//! git itself, as a child process, with the limits `execFileSync` applied.

use std::io::Read;
use std::process::{Child, Command, Stdio};
use std::sync::mpsc;
use std::thread;

use crate::{command_line, Error, Request};

pub(crate) fn run(req: &Request) -> Result<String, Error> {
    let mut cmd = Command::new(&req.bin);
    cmd.args(&req.args)
        .current_dir(&req.cwd)
        .env_clear()
        .envs(req.env.iter().map(|(k, v)| (k, v)))
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        // No console window flashing up for every call from a GUI app.
        const CREATE_NO_WINDOW: u32 = 0x0800_0000;
        cmd.creation_flags(CREATE_NO_WINDOW);
    }

    let mut child = cmd.spawn().map_err(|error| Error::Spawn {
        bin: req.bin.clone(),
        error,
    })?;

    let (stdout, stderr) = (child.stdout.take(), child.stderr.take());
    let limit = req.max_buffer;
    let (tx, rx) = mpsc::channel();
    let out_tx = tx.clone();
    thread::spawn(move || {
        let _ = out_tx.send(Stream::Out(read_capped(stdout, limit)));
    });
    // `maxBuffer` bounds stderr too, as it does for `execFileSync`.
    thread::spawn(move || {
        let _ = tx.send(Stream::Err(read_capped(stderr, limit)));
    });

    // Both pipes close when git exits; until then this thread only waits.
    let deadline = std::time::Instant::now() + req.timeout;
    let (mut out, mut err) = (None, None);
    while out.is_none() || err.is_none() {
        let left = deadline.saturating_duration_since(std::time::Instant::now());
        match rx.recv_timeout(left) {
            Ok(Stream::Out(read) | Stream::Err(read)) if read.overflowed => {
                stop(&mut child);
                return Err(Error::TooLarge {
                    bin: req.bin.clone(),
                    limit,
                });
            }
            Ok(Stream::Out(read)) => out = Some(read.bytes),
            Ok(Stream::Err(read)) => err = Some(read.bytes),
            Err(_) => {
                stop(&mut child);
                return Err(Error::TimedOut {
                    bin: req.bin.clone(),
                    after: req.timeout,
                });
            }
        }
    }

    let status = child.wait().map_err(|error| Error::Spawn {
        bin: req.bin.clone(),
        error,
    })?;
    let stdout = String::from_utf8_lossy(&out.unwrap_or_default()).into_owned();
    if status.success() {
        Ok(stdout)
    } else {
        Err(Error::Failed {
            command: command_line(req),
            stderr: String::from_utf8_lossy(&err.unwrap_or_default()).into_owned(),
        })
    }
}

enum Stream {
    Out(Capped),
    Err(Capped),
}

struct Capped {
    bytes: Vec<u8>,
    overflowed: bool,
}

/// Reads to the end, or stops at the first byte past `limit` and says so.
fn read_capped(pipe: Option<impl Read>, limit: usize) -> Capped {
    let mut bytes = Vec::new();
    let Some(mut pipe) = pipe else {
        return Capped {
            bytes,
            overflowed: false,
        };
    };
    let mut buf = [0u8; 64 * 1024];
    loop {
        let n = match pipe.read(&mut buf) {
            Ok(0) | Err(_) => break,
            Ok(n) => n,
        };
        if bytes.len() + n > limit {
            return Capped {
                bytes,
                overflowed: true,
            };
        }
        bytes.extend_from_slice(&buf[..n]);
    }
    Capped {
        bytes,
        overflowed: false,
    }
}

fn stop(child: &mut Child) {
    let _ = child.kill();
    let _ = child.wait();
}

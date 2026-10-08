//! Load producers: what a busy pane runs.
//!
//! - `yes`: `yes` through a colorizer, paced at 1 MB/s (≈18,000 lines/s,
//!   so every 8 ms frame of every pane is a new full screen). Unpaced, it
//!   measures how fast the session holder can spool to disk, not the UI.
//! - `buildlog`: a scrolling build log at a steady 256 KB/s, a compile line
//!   per module and a spinner redrawn in place every fifth line.
//! - `rec:NAME`: a recorded transcript from `crates/recovery` (vim, htop,
//!   an agent CLI) replayed at its recorded pace, looped.

use std::io::Write;
use std::time::{Duration, Instant};

pub fn main(kind: &str) {
    let r = if let Some(name) = kind.strip_prefix("rec:") {
        replay(name)
    } else if kind == "yes" {
        paced(1024.0 * 1024.0, |n, buf| {
            let _ = write!(
                buf,
                "\x1b[3{}mthe quick brown fox jumps over the lazy dog 0123456789 \x1b[1;3{}m{n}\x1b[0m\r\n",
                n % 7 + 1,
                (n + 3) % 7 + 1
            );
        })
    } else {
        paced(256.0 * 1024.0, build_line)
    };
    // A closed pty ends the producer.
    let _ = r;
}

/// Writes lines from `line(n, buf)` at `rate` bytes per second.
fn paced(rate: f64, line: impl Fn(u64, &mut Vec<u8>)) -> std::io::Result<()> {
    let mut out = std::io::stdout().lock();
    let start = Instant::now();
    let mut sent = 0usize;
    let mut buf = Vec::with_capacity(8192);
    for i in 0u64.. {
        buf.clear();
        for j in 0..32 {
            line(i * 32 + j, &mut buf);
        }
        out.write_all(&buf)?;
        out.flush()?;
        sent += buf.len();
        let due = Duration::from_secs_f64(sent as f64 / rate);
        if let Some(wait) = due.checked_sub(start.elapsed()) {
            std::thread::sleep(wait);
        }
    }
    Ok(())
}

fn build_line(n: u64, buf: &mut Vec<u8>) {
    if n % 5 == 0 {
        let _ = write!(
            buf,
            "\r\x1b[2K\x1b[36m{}\x1b[0m Thinking… \x1b[2m({}s · esc to interrupt)\x1b[0m",
            ['⠋', '⠙', '⠹', '⠸', '⠼', '⠴', '⠦', '⠧', '⠇', '⠏'][(n % 10) as usize],
            n / 40
        );
    } else {
        let _ = write!(
            buf,
            "\r\x1b[2K\x1b[32m✓\x1b[0m compiled \x1b[1msrc/module_{n}.rs\x1b[0m in {}ms\r\n",
            (n * 37) % 900
        );
    }
}

fn replay(name: &str) -> std::io::Result<()> {
    let path = crate::repo_root()
        .join("packages/core/crates/recovery/transcripts")
        .join(format!("{name}.rec"));
    let bytes = std::fs::read(&path)?;
    let mut out = std::io::stdout().lock();
    if bytes.len() < 10 || &bytes[..6] != b"VREC1\n" {
        return Ok(());
    }
    loop {
        let mut at = 10;
        while at + 9 <= bytes.len() {
            let tag = bytes[at];
            let delta = u32::from_le_bytes(bytes[at + 1..at + 5].try_into().unwrap());
            // Long pauses in a recording are someone thinking; cap them.
            std::thread::sleep(Duration::from_micros(u64::from(delta.min(50_000))));
            match tag {
                b'D' => {
                    let len =
                        u32::from_le_bytes(bytes[at + 5..at + 9].try_into().unwrap()) as usize;
                    let end = (at + 9 + len).min(bytes.len());
                    out.write_all(&bytes[at + 9..end])?;
                    out.flush()?;
                    at = end;
                }
                b'R' => at += 9,
                _ => break,
            }
        }
        out.write_all(b"\x1b[0m\x1b[2J\x1b[H")?;
    }
}

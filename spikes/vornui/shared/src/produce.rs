//! What a pane runs, started by vornd as `<prototype> produce KIND`:
//!
//! - `yes`: colored lines paced at 1 MB/s, so every pane has a new screen
//!   every frame. Unpaced it would measure how fast the holder spools to disk.
//! - `buildlog`: a few screens of a build log, then idle: the screenshot load.
//! - `echo`: echoes what is typed byte for byte, with the terminal in raw
//!   mode, so the latency probe sees its glyph as soon as the pty does. A
//!   shell would add its own line editor's work to every keystroke.

use std::io::{Read, Write};
use std::time::{Duration, Instant};

pub fn main(kind: &str) {
    let _ = match kind {
        "yes" => paced(1024.0 * 1024.0, |n, buf| {
            let _ = write!(
                buf,
                "\x1b[3{}mthe quick brown fox jumps over the lazy dog 0123456789 \x1b[1;3{}m{n}\x1b[0m\r\n",
                n % 7 + 1,
                (n + 3) % 7 + 1
            );
        }),
        "buildlog" => buildlog(),
        _ => echo(),
    };
}

/// Writes lines from `line(n, buf)` at `rate` bytes per second until the
/// pty closes.
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

fn buildlog() -> std::io::Result<()> {
    let mut out = std::io::stdout().lock();
    write!(out, "\x1b[1;36m$\x1b[0m cargo build --release\r\n")?;
    for n in 0..60u64 {
        write!(
            out,
            "\x1b[32m   Compiling\x1b[0m \x1b[1mvorn-module-{n}\x1b[0m v0.{}.{} \x1b[2m(packages/core/crates/m{n})\x1b[0m\r\n",
            n % 9,
            (n * 7) % 13
        )?;
    }
    write!(
        out,
        "\x1b[33mwarning\x1b[0m: unused variable `cols` → 日本語 ✓\r\n\x1b[32m    Finished\x1b[0m `release` profile in 41.7s\r\n\x1b[1;36m$\x1b[0m "
    )?;
    out.flush()?;
    // Idle until the pty closes, so a pane at rest still ends with its session.
    let mut sink = [0u8; 256];
    let mut input = std::io::stdin().lock();
    while input.read(&mut sink)? > 0 {}
    Ok(())
}

fn echo() -> std::io::Result<()> {
    raw_mode();
    let mut input = std::io::stdin().lock();
    let mut out = std::io::stdout().lock();
    write!(out, "\x1b[1;36m$\x1b[0m ")?;
    out.flush()?;
    let mut buf = [0u8; 4096];
    loop {
        let n = input.read(&mut buf)?;
        if n == 0 {
            return Ok(());
        }
        for &b in &buf[..n] {
            if b == b'\r' || b == b'\n' {
                out.write_all(b"\r\n\x1b[1;36m$\x1b[0m ")?;
            } else {
                out.write_all(&[b])?;
            }
        }
        out.flush()?;
    }
}

#[cfg(unix)]
fn raw_mode() {
    // SAFETY: tcgetattr fills the struct; tcsetattr reads it.
    unsafe {
        let mut t: libc::termios = std::mem::zeroed();
        if libc::tcgetattr(0, &mut t) == 0 {
            t.c_lflag &= !(libc::ICANON | libc::ECHO | libc::ISIG | libc::IEXTEN);
            t.c_iflag &= !(libc::ICRNL | libc::IXON);
            t.c_cc[libc::VMIN] = 1;
            t.c_cc[libc::VTIME] = 0;
            libc::tcsetattr(0, libc::TCSANOW, &t);
        }
    }
}

#[cfg(windows)]
fn raw_mode() {
    use windows_sys::Win32::System::Console::{
        GetStdHandle, SetConsoleMode, ENABLE_VIRTUAL_TERMINAL_INPUT, STD_INPUT_HANDLE,
    };
    // SAFETY: the handle is the process's own stdin; a pipe only fails.
    unsafe {
        SetConsoleMode(
            GetStdHandle(STD_INPUT_HANDLE),
            ENABLE_VIRTUAL_TERMINAL_INPUT,
        );
    }
}

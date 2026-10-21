//! Many panes at once: the app's frame time, input-to-pixel latency and CPU
//! per pane with N terminals streaming, against a test vornd.
//!
//! ```text
//! many_panes [--panes 8,16] [--cases mixed,agent,hidden] [--seconds 5]
//!            [--size 1600x1000] [--scale 1] [--out results.json]
//!            [--agent-rate 32768] [--log-rate 262144]
//!            [--extensions 0,20] [--reps 1] [--ext-period 2]
//!            [--idle-wait 0] [--ext-idle-ms MS] [--ext-cap N]
//! many_panes feed agent|log|redraw|echo [--rate B/s]
//! ```
//!
//! Each case starts its own vornd on a temporary HOME and data directory
//! (never `~/.vorn`), opens N shells and runs a workload in each: this
//! program again, as `feed`. One pane on screen is the probe, which echoes
//! each key it reads; the rest stream:
//!
//! - `mixed`: an agent's TUI, a build log and a full-screen redraw at 60 Hz,
//!   in turn.
//! - `agent`: every pane an agent's TUI at `--agent-rate`.
//! - `hidden`: as `agent`, with the probe's card expanded, so the other
//!   panes stream into cards that are not on screen.
//! - `resize`: as `agent`, while the window is dragged between its size and
//!   60% by 70% of it and back every two seconds, a new target each frame.
//!
//! The grid shows at most 16 cards and pages past what fits the window; every
//! pane is attached, as if the person had paged through them all, and the
//! report's `Shown` counts those drawn.
//!
//! The app is driven offscreen (the GPU when there is one, else the CPU
//! raster; `VORNUI_RENDERER` picks) on a 60 Hz clock, as a window paces its
//! frames. **Frame time** is one frame's build, layout, paint and draw, which
//! includes folding in what the terminals sent. **Input to pixel** is from a
//! key sent to the probe until a frame showing its echo has been drawn.
//! **CPU** is the app process plus vornd and its holders over the run, in per
//! cent of one core; the programs in the panes are the workload, not counted.
//! **Memory** is the resident size of the app and of vornd with its holders
//! at the end of the run.
//!
//! **Extensions**: with `--extensions N` (up to 20) each case first links N
//! test extensions (`extensions.rs`): Node, Python and Bash programs and
//! Rust Wasm components, services and one-shots. While the panes stream, a
//! connection of its own calls each one every `--ext-period` seconds, and
//! the report adds the calls' round trips, the most services seen running,
//! service restarts and cap stops from vornd's log, and the extension
//! programs' memory. `--idle-wait S` then stops calling and waits up to S
//! seconds for every service to stop as idle, and records the memory left
//! and a cold and a warm call to each service. `--ext-idle-ms` and
//! `--ext-cap` give vornd another idle time and cap; `--reps` repeats every
//! case, and the report gives the median and range of the repetitions.

#[cfg(unix)]
#[path = "../../tests/support/mod.rs"]
mod support;

#[cfg(unix)]
mod extensions;

#[cfg(not(unix))]
fn main() {
    eprintln!("many_panes: the workloads need a POSIX terminal");
    std::process::exit(1);
}

#[cfg(unix)]
fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let result = match args.first().map(String::as_str) {
        Some("feed") => feed::run(&args[1..]),
        _ => bench::run(&args),
    };
    if let Err(e) = result {
        eprintln!("many_panes: {e}");
        std::process::exit(1);
    }
}

/// The value after `--key`.
#[cfg(unix)]
fn flag<'a>(args: &'a [String], key: &str) -> Option<&'a str> {
    args.windows(2)
        .find(|w| w[0].strip_prefix("--") == Some(key))
        .map(|w| w[1].as_str())
}

/// The terminal output the workloads write, generated from fixed seeds so
/// every run feeds the same bytes.
#[cfg(unix)]
mod transcript {
    /// mulberry32.
    struct Rng(u32);

    impl Rng {
        fn next(&mut self) -> f64 {
            self.0 = self.0.wrapping_add(0x6d2b_79f5);
            let mut t = self.0;
            t = (t ^ (t >> 15)).wrapping_mul(t | 1);
            t ^= t.wrapping_add((t ^ (t >> 7)).wrapping_mul(t | 61));
            f64::from(t ^ (t >> 14)) / 4_294_967_296.0
        }

        fn below(&mut self, n: usize) -> usize {
            (self.next() * n as f64) as usize
        }
    }

    const SPINNER: [char; 10] = ['⠋', '⠙', '⠹', '⠸', '⠼', '⠴', '⠦', '⠧', '⠇', '⠏'];
    const FILES: [&str; 5] = [
        "packages/core/crates/sessiond/src/pty.rs",
        "packages/core/crates/vornd/src/terminal.rs",
        "packages/core/crates/term-proto/src/screen.rs",
        "packages/core/crates/term-proto/src/msg.rs",
        "packages/core/crates/vornd/tests/grid.rs",
    ];
    const WORDS: [&str; 7] = ["the", "flush", "buffer", "screen", "model", "per", "chunk"];

    /// An agent's TUI: a status line redrawn in place, tool calls between.
    fn spinner_frames(target: usize, seed: u32) -> Vec<String> {
        let mut rand = Rng(seed);
        let mut frames = vec!["\x1b]0;✳ agent\x07\x1b[?2004h\x1b[?25l".to_owned()];
        let mut bytes = frames[0].len();
        let mut tick = 0usize;
        while bytes < target {
            tick += 1;
            let r = rand.next();
            let frame = if r < 0.04 {
                let file = FILES[rand.below(FILES.len())];
                let lines = 50 + rand.below(900);
                format!(
                    "\r\x1b[2K\r\n\x1b[1m\x1b[38;5;114m⏺\x1b[39m Read\x1b[22m({file})\r\n  \x1b[2m⎿  Read {lines} lines\x1b[22m\r\n\r\n"
                )
            } else if r < 0.06 {
                let words = (8.0 + rand.next() * 40.0) as usize;
                let text: Vec<&str> = (0..words).map(|i| WORDS[i % WORDS.len()]).collect();
                format!("\r\x1b[2K\x1b[38;5;231m⏺\x1b[39m {}\r\n", text.join(" "))
            } else if r < 0.061 {
                "\x1b[?2004l\x1b[?25h\r\n> \x1b[?2004h\x1b[?25l".to_owned()
            } else {
                let s = tick / 10;
                // toFixed(1): rounds the f64's exact value, and exact ties (x.25, x.75) up.
                let tokens = (tick * 13) % 9000;
                let tokens = if tokens.is_multiple_of(250) && tokens % 100 == 50 {
                    format!("{}.{}", (tokens + 50) / 1000, (tokens + 50) / 100 % 10)
                } else {
                    format!("{:.1}", tokens as f64 / 1000.0)
                };
                format!(
                    "\r\x1b[2K\x1b[38;5;174m{}\x1b[39m \x1b[38;5;174mThinking…\x1b[39m \x1b[2m({s}s · ↑ {tokens}k tokens · esc to interrupt)\x1b[22m",
                    SPINNER[tick % SPINNER.len()],
                )
            };
            bytes += frame.len();
            frames.push(frame);
        }
        frames
    }

    /// Frames grouped into reads as a PTY delivers them.
    fn group_into_reads(frames: &[String], seed: u32) -> Vec<String> {
        let mut rand = Rng(seed);
        let mut reads = Vec::new();
        let mut i = 0;
        while i < frames.len() {
            let r = rand.next();
            let n = if r < 0.7 {
                1
            } else if r < 0.95 {
                2 + rand.below(9)
            } else {
                20 + rand.below(41)
            };
            reads.push(frames[i..(i + n).min(frames.len())].concat());
            i += n;
        }
        reads
    }

    /// `spinner`: 1 MB of an agent's TUI in reads of one to sixty frames.
    pub fn spinner() -> Vec<String> {
        group_into_reads(&spinner_frames(1024 * 1024, 7), 11)
    }

    /// `bulk`: 2 MB of build log in 4 KB reads.
    pub fn bulk() -> Vec<String> {
        let mut rand = Rng(3);
        let mut text = String::new();
        let mut line = 0usize;
        while text.len() < 2 * 1024 * 1024 {
            line += 1;
            let file = FILES[line % FILES.len()];
            let kind = rand.next();
            if kind < 0.1 {
                text += &format!("\x1b[33mwarning\x1b[0m: unused variable in {file}:{line}\r\n");
            } else if kind < 0.12 {
                text += &format!("\x1b[31merror\x1b[0m TS2345: {file}({line},7)\r\n");
            } else {
                let dots = ".".repeat(rand.below(60));
                text += &format!("  compiling {file} [{line}] {dots}\r\n");
            }
        }
        text.as_bytes()
            .chunks(4096)
            .map(|c| String::from_utf8_lossy(c).into_owned())
            .collect()
    }
}

/// The programs the panes run.
#[cfg(unix)]
mod feed {
    use std::io::{Read, Write};
    use std::time::{Duration, Instant};

    use super::{flag, transcript};

    /// The beat output is fed on.
    const BEAT: Duration = Duration::from_millis(8);
    const FRAME: Duration = Duration::from_micros(16_667);

    pub fn run(args: &[String]) -> Result<(), String> {
        let rate = |default: usize| {
            flag(args, "rate")
                .and_then(|r| r.parse().ok())
                .unwrap_or(default)
        };
        let out = std::io::stdout();
        let mut out = out.lock();
        // A closed terminal ends the program: the write fails.
        let _ = match args.first().map(String::as_str) {
            Some("agent") => stream(&mut out, &transcript::spinner(), rate(32 * 1024)),
            Some("log") => stream(&mut out, &transcript::bulk(), rate(256 * 1024)),
            Some("redraw") => redraw(&mut out),
            Some("echo") => echo(&mut out),
            other => return Err(format!("unknown feed {other:?}")),
        };
        Ok(())
    }

    /// `chunks` round and round at `rate` bytes a second, a beat's share
    /// at a time.
    fn stream(out: &mut impl Write, chunks: &[String], rate: usize) -> std::io::Result<()> {
        let per_beat = (rate as f64 * BEAT.as_secs_f64()).max(1.0) as usize;
        let mut at = 0;
        let mut next = Instant::now();
        let mut buf = Vec::with_capacity(per_beat * 2);
        loop {
            buf.clear();
            while buf.len() < per_beat {
                buf.extend_from_slice(chunks[at].as_bytes());
                at = (at + 1) % chunks.len();
            }
            out.write_all(&buf)?;
            out.flush()?;
            next += BEAT;
            sleep_until(&mut next, BEAT);
        }
    }

    /// Every cell rewritten 60 times a second, as a full-screen program
    /// repaints.
    fn redraw(out: &mut impl Write) -> std::io::Result<()> {
        const GLYPHS: &[u8] = b"0123456789abcdefghijklmnopqrstuvwxyzABCDEFGHIJKLMNOPQRSTUVWXYZ";
        out.write_all(b"\x1b[?1049h\x1b[?25l")?;
        let mut buf = Vec::new();
        let mut next = Instant::now();
        let mut n = 0u64;
        loop {
            let (cols, rows) = size();
            buf.clear();
            buf.extend_from_slice(b"\x1b[H");
            for row in 0..u64::from(rows) {
                write!(buf, "\x1b[38;5;{}m", 16 + (n + row) % 216)?;
                for col in 0..u64::from(cols) {
                    buf.push(GLYPHS[((n + row * 7 + col) % GLYPHS.len() as u64) as usize]);
                }
                if row + 1 < u64::from(rows) {
                    buf.extend_from_slice(b"\r\n");
                }
            }
            out.write_all(&buf)?;
            out.flush()?;
            n += 1;
            next += FRAME;
            sleep_until(&mut next, FRAME);
        }
    }

    /// The probe: raw input, each key answered on one line as `key <k>`.
    fn echo(out: &mut impl Write) -> std::io::Result<()> {
        raw_stdin();
        out.write_all(b"\x1b[?25l")?;
        out.flush()?;
        let mut input = std::io::stdin().lock();
        let mut buf = [0u8; 64];
        loop {
            let n = input.read(&mut buf)?;
            if n == 0 {
                return Ok(());
            }
            for &b in &buf[..n] {
                // Raw mode turns signals off; ^C still ends the probe.
                if b == 3 {
                    return Ok(());
                }
                if b.is_ascii_alphanumeric() {
                    write!(out, "\r\x1b[2Kkey {}", b as char)?;
                }
            }
            out.flush()?;
        }
    }

    /// Sleeps to `next`; a program that fell behind skips ahead instead of
    /// bursting to catch up.
    fn sleep_until(next: &mut Instant, period: Duration) {
        let now = Instant::now();
        if *next > now {
            std::thread::sleep(*next - now);
        } else {
            *next = now + period;
        }
    }

    /// The terminal's size, 80x24 when it does not say.
    fn size() -> (u16, u16) {
        // SAFETY: winsize is plain data, and TIOCGWINSZ only fills it.
        let ws = unsafe {
            let mut ws: libc::winsize = std::mem::zeroed();
            (libc::ioctl(libc::STDOUT_FILENO, libc::TIOCGWINSZ, &mut ws) == 0).then_some(ws)
        };
        match ws {
            Some(ws) if ws.ws_col > 0 && ws.ws_row > 0 => (ws.ws_col, ws.ws_row),
            _ => (80, 24),
        }
    }

    fn raw_stdin() {
        // SAFETY: termios is plain data; tcgetattr fills it before it is used.
        unsafe {
            let mut t: libc::termios = std::mem::zeroed();
            if libc::tcgetattr(libc::STDIN_FILENO, &mut t) == 0 {
                libc::cfmakeraw(&mut t);
                libc::tcsetattr(libc::STDIN_FILENO, libc::TCSANOW, &t);
            }
        }
    }
}

#[cfg(unix)]
mod bench {
    use std::collections::BTreeMap;
    use std::fmt::Write as _;
    use std::path::Path;
    use std::sync::Arc;
    use std::time::{Duration, Instant};

    use serde_json::{json, Value};
    use vorn_app::screen::term_id;
    use vorn_app::ui::{Offscreen, RenderMode, Ui};
    use vorn_app::{ui_config, App};

    use super::extensions;
    use super::flag;
    use super::support::{connect, wait_for, TestVornd};

    /// The frame clock: a 60 Hz display.
    const FRAME: Duration = Duration::from_micros(16_667);
    /// The pause between one probe's answer and the next key.
    const PROBE_GAP: Duration = Duration::from_millis(60);
    const PROBE_TIMEOUT: Duration = Duration::from_secs(2);
    const WARMUP: Duration = Duration::from_secs(2);

    #[derive(Clone, Copy, PartialEq)]
    enum Case {
        Mixed,
        Agent,
        Hidden,
        Resize,
    }

    impl Case {
        fn parse(s: &str) -> Result<Case, String> {
            match s {
                "mixed" => Ok(Case::Mixed),
                "agent" => Ok(Case::Agent),
                "hidden" => Ok(Case::Hidden),
                "resize" => Ok(Case::Resize),
                _ => Err(format!("unknown case {s:?} (mixed, agent, hidden, resize)")),
            }
        }

        fn name(self) -> &'static str {
            match self {
                Case::Mixed => "mixed",
                Case::Agent => "agent",
                Case::Hidden => "hidden",
                Case::Resize => "resize",
            }
        }

        /// What pane `i` runs; pane 0 is the probe.
        fn feed(self, i: usize) -> &'static str {
            match (self, i) {
                (_, 0) => "echo",
                (Case::Mixed, i) => ["agent", "log", "redraw"][(i - 1) % 3],
                _ => "agent",
            }
        }
    }

    struct Opts {
        panes: Vec<usize>,
        cases: Vec<Case>,
        seconds: f64,
        size: (f32, f32),
        scale: f32,
        out: Option<String>,
        agent_rate: usize,
        log_rate: usize,
        extensions: Vec<usize>,
        reps: usize,
        ext_period: Duration,
        idle_wait: Duration,
        ext_idle_ms: Option<u64>,
        ext_cap: Option<usize>,
    }

    impl Opts {
        fn parse(args: &[String]) -> Result<Opts, String> {
            let list = |key: &str, default: &str| -> Vec<String> {
                flag(args, key)
                    .unwrap_or(default)
                    .split(',')
                    .map(str::to_owned)
                    .collect()
            };
            let num = |key: &str, default: f64| -> Result<f64, String> {
                flag(args, key).map_or(Ok(default), |v| {
                    v.parse().map_err(|_| format!("bad --{key} {v:?}"))
                })
            };
            let panes = list("panes", "8,16")
                .iter()
                .map(|n| match n.parse::<usize>() {
                    Ok(n) if n >= 1 => Ok(n),
                    _ => Err(format!("bad --panes {n:?}")),
                })
                .collect::<Result<_, _>>()?;
            let cases = list("cases", "mixed,agent,hidden")
                .iter()
                .map(|c| Case::parse(c))
                .collect::<Result<_, _>>()?;
            let size = match flag(args, "size").map(|s| s.split_once('x')) {
                None => (1600.0, 1000.0),
                Some(Some((w, h))) => (
                    w.parse().map_err(|_| "bad --size")?,
                    h.parse().map_err(|_| "bad --size")?,
                ),
                Some(None) => return Err("--size is WxH".into()),
            };
            let extensions = list("extensions", "0")
                .iter()
                .map(|n| match n.parse::<usize>() {
                    Ok(n) if n <= extensions::MAX => Ok(n),
                    _ => Err(format!("bad --extensions {n:?} (0 to {})", extensions::MAX)),
                })
                .collect::<Result<_, _>>()?;
            let secs = |key: &str, default: f64| -> Result<Duration, String> {
                Duration::try_from_secs_f64(num(key, default)?).map_err(|_| format!("bad --{key}"))
            };
            let whole = |key: &str| -> Result<Option<u64>, String> {
                flag(args, key)
                    .map(|v| v.parse().map_err(|_| format!("bad --{key} {v:?}")))
                    .transpose()
            };
            Ok(Opts {
                panes,
                cases,
                seconds: num("seconds", 5.0)?,
                size,
                scale: num("scale", 1.0)? as f32,
                out: flag(args, "out").map(str::to_owned),
                agent_rate: num("agent-rate", 32.0 * 1024.0)? as usize,
                log_rate: num("log-rate", 256.0 * 1024.0)? as usize,
                extensions,
                reps: whole("reps")?.unwrap_or(1).max(1) as usize,
                ext_period: secs("ext-period", 2.0)?,
                idle_wait: secs("idle-wait", 0.0)?,
                ext_idle_ms: whole("ext-idle-ms")?,
                ext_cap: whole("ext-cap")?.map(|n| n as usize),
            })
        }
    }

    pub fn run(args: &[String]) -> Result<(), String> {
        let opts = Opts::parse(args)?;
        // The bench's own HOME is a temporary one too, so nothing it starts
        // reads or writes the user's.
        let home = std::env::temp_dir().join(format!("vorn-many-panes-{}", std::process::id()));
        std::fs::create_dir_all(&home).map_err(|e| e.to_string())?;
        std::env::set_var("HOME", &home);
        std::env::set_var("VORN_HOME", &home);
        std::env::remove_var("VORN_DATA_DIR");
        // vornd inherits these; unset, it runs its defaults.
        match opts.ext_idle_ms {
            Some(ms) => std::env::set_var("VORN_EXTENSION_IDLE_MS", ms.to_string()),
            None => std::env::remove_var("VORN_EXTENSION_IDLE_MS"),
        }
        match opts.ext_cap {
            Some(n) => std::env::set_var("VORN_EXTENSION_CAP", n.to_string()),
            None => std::env::remove_var("VORN_EXTENSION_CAP"),
        }
        let result = run_all(&opts);
        let _ = std::fs::remove_dir_all(&home);
        let results = result?;
        print!("{}", table(&results));
        if opts.reps > 1 || opts.extensions.len() > 1 {
            print!("{}", summary(&results));
        }
        if let Some(out) = &opts.out {
            let doc = json!({ "machine": machine(), "results": results });
            let text = serde_json::to_string_pretty(&doc).map_err(|e| e.to_string())?;
            std::fs::write(out, text + "\n").map_err(|e| format!("{out}: {e}"))?;
        }
        Ok(())
    }

    fn run_all(opts: &Opts) -> Result<Vec<Value>, String> {
        let mut results = Vec::new();
        // Repetitions go round every case, so drift over the run spreads evenly.
        for rep in 1..=opts.reps {
            for &n in &opts.panes {
                for &case in &opts.cases {
                    for &exts in &opts.extensions {
                        eprintln!(
                            "many_panes: {n} panes, {}, {exts} extensions, run {rep}/{}",
                            case.name(),
                            opts.reps
                        );
                        let mut r = run_case(opts, n, case, exts)?;
                        r["rep"] = json!(rep);
                        results.push(r);
                    }
                }
            }
        }
        Ok(results)
    }

    fn run_case(opts: &Opts, n: usize, case: Case, n_ext: usize) -> Result<Value, String> {
        let v = TestVornd::start()
            .ok_or("build vornd and vorn-sessiond into this profile's target directory first")?;
        let mut app = connect(&v);
        let exts = Arc::new(extensions::generate(&v.home.join("bench-ext"), n_ext)?);
        extensions::link(app.rpc(), &exts)?;
        let ids: Vec<String> = (0..n)
            .map(|_| {
                let info = app
                    .rpc()
                    .call("shell:create", json!(v.home))
                    .map_err(|e| format!("shell:create: {e:?}"))?;
                info["id"]
                    .as_str()
                    .map(str::to_owned)
                    .ok_or_else(|| "shell:create gave no id".to_owned())
            })
            .collect::<Result<_, _>>()?;

        let mut ui = Ui::headless(RenderMode::from_env(), &ui_config(opts.scale));
        let px = |size: (f32, f32)| {
            (
                (size.0 * opts.scale).round() as u32,
                (size.1 * opts.scale).round() as u32,
            )
        };
        let mut target = ui.offscreen(px(opts.size));
        let mut size = opts.size;
        // Cards attach their terminals when first laid out; the grid pages past
        // 16, so the rest attach here as if the person had paged through them.
        draw(&mut app, &mut ui, &target, opts.size);
        let grid = app.grid().ok_or("no grid connection")?.clone();
        for id in &ids {
            grid.attach(id, 80, 24, (0, 0), 0);
        }
        wait_for("every pane's first screen", || {
            draw(&mut app, &mut ui, &target, opts.size);
            ids.iter().all(|id| app.pane(id).is_some()).then_some(())
        });
        let on_screen = |ui: &Ui, id: &String| ui.rect_of(term_id(id)).is_some();
        // The probe is a pane on screen; it goes first so it gets the echo feed.
        let mut ids = ids;
        let first = ids.iter().position(|id| on_screen(&ui, id)).unwrap_or(0);
        ids.swap(0, first);
        let exe = std::env::current_exe().map_err(|e| e.to_string())?;
        for (i, id) in ids.iter().enumerate() {
            let feed = case.feed(i);
            let rate = if feed == "log" {
                opts.log_rate
            } else {
                opts.agent_rate
            };
            grid.text(
                id,
                &format!("exec {} feed {feed} --rate {rate}\r", quote(&exe)),
            );
        }
        if case == Case::Hidden {
            app.st.focused = Some(ids[0].clone());
        }
        let probe = &ids[0];
        let warm = Instant::now() + WARMUP;
        while Instant::now() < warm {
            draw(&mut app, &mut ui, &target, opts.size);
            std::thread::sleep(FRAME);
        }
        // The probe answers before the clock starts.
        wait_for("the probe's first answer", || {
            grid.text(probe, "z");
            draw(&mut app, &mut ui, &target, opts.size);
            std::thread::sleep(Duration::from_millis(20));
            shows(&app, probe, 'z').then_some(())
        });

        let revs = |app: &App| -> Vec<u64> {
            ids.iter()
                .map(|id| app.pane(id).map_or(0, |p| p.rev))
                .collect()
        };
        let revs_before = revs(&app);
        let pids = v.pids();
        let driver = extensions::Driver::start(&v.home, Arc::clone(&exts), opts.ext_period)?;
        let cpu_app = self_cpu();
        let cpu_daemons = procs_cpu(&pids);
        let started = Instant::now();
        let until = started + Duration::from_secs_f64(opts.seconds);

        let mut frames: Vec<f64> = Vec::new();
        let mut latencies: Vec<f64> = Vec::new();
        let mut timeouts = 0usize;
        let mut seq = 0usize;
        let mut pending: Option<(char, Instant)> = None;
        let mut probe_at = started + PROBE_GAP;
        let mut next = started;
        let mut resizes = 0usize;
        while Instant::now() < until {
            // Between frames: the probe's key goes when it is due.
            loop {
                let now = Instant::now();
                if pending.is_none() && now >= probe_at {
                    // a..y; the warm-up's z never matches a stale answer.
                    let c = char::from(b'a' + (seq % 25) as u8);
                    seq += 1;
                    grid.text(probe, &c.to_string());
                    pending = Some((c, now));
                    continue;
                }
                if now >= next {
                    break;
                }
                let wake = if pending.is_none() {
                    next.min(probe_at)
                } else {
                    next
                };
                std::thread::sleep(wake - now);
            }
            let t0 = Instant::now();
            // A late frame moves the clock rather than bursting to catch up.
            next = if next + FRAME > t0 {
                next + FRAME
            } else {
                t0 + FRAME
            };
            if case == Case::Resize {
                size = dragged(opts.size, t0 - started);
                if px(size) != target.size {
                    target = ui.offscreen(px(size));
                    resizes += 1;
                }
            }
            draw(&mut app, &mut ui, &target, size);
            let t1 = Instant::now();
            frames.push(ms(t1 - t0));
            let Some((c, sent)) = pending else {
                continue;
            };
            if shows(&app, probe, c) {
                latencies.push(ms(t1 - sent));
            } else if t1 - sent < PROBE_TIMEOUT {
                continue;
            } else {
                timeouts += 1;
            }
            pending = None;
            // Spread the keys over the frame so they land at every phase.
            let phase = Duration::from_micros((seq as u64 * 3_701) % 16_667);
            probe_at = t1 + PROBE_GAP + phase;
        }
        let wall = started.elapsed().as_secs_f64();
        let calls = driver.finish();
        let app_cpu = (self_cpu() - cpu_app) / wall * 100.0;
        let daemon_cpu = (procs_cpu(&pids) - cpu_daemons) / wall * 100.0;
        let streaming = revs(&app)
            .iter()
            .zip(&revs_before)
            .filter(|(a, b)| a != b)
            .count();
        let shown = ids
            .iter()
            .filter(|id| on_screen(&ui, id) || app.st.focused.as_ref() == Some(*id))
            .count();
        let over = frames.iter().filter(|&&f| f > ms(FRAME)).count();
        let mem_app = procs_rss(&[std::process::id()]);
        let mem_daemons = procs_rss(&pids);
        let programs = extensions::programs_rss(pids[0], &pids[1..]);
        let busy_log = extensions::read_log(&v.log());
        let idle = if opts.idle_wait.is_zero() {
            Value::Null
        } else {
            idle(opts, &mut app, &mut ui, &target, size, &v, &exts)?
        };
        Ok(json!({
            "case": case.name(),
            "panes": n,
            "extensions": n_ext,
            "shown": shown,
            "streaming": streaming,
            "renderer": ui.renderer_name(),
            "size": format!("{}x{}@{}", opts.size.0, opts.size.1, opts.scale),
            "seconds": round(wall),
            "frames": frames.len(),
            "resizes": resizes,
            "frame_ms": { "p50": pct(&frames, 50.0), "p99": pct(&frames, 99.0), "max": pct(&frames, 100.0) },
            "over_budget_pct": round(100.0 * over as f64 / frames.len().max(1) as f64),
            "input_to_pixel_ms": {
                "p50": pct(&latencies, 50.0),
                "p99": pct(&latencies, 99.0),
                "samples": latencies.len(),
                "timeouts": timeouts,
            },
            "cpu_pct": {
                "app": round(app_cpu),
                "daemons": round(daemon_cpu),
                "per_pane": round((app_cpu + daemon_cpu) / n as f64),
            },
            "rss_mb": {
                "app": round(mem_app),
                "daemons": round(mem_daemons),
                "programs": round(programs.values().map(|p| p.1).sum()),
                "programs_by_lang": programs_json(&programs),
            },
            "ext_calls": {
                "count": calls.ms.len(),
                "p50": pct(&calls.ms, 50.0),
                "p99": pct(&calls.ms, 99.0),
                "errors": calls.errors.len(),
                "first_error": calls.errors.first(),
            },
            "running_peak": { "bench": calls.peak_bench, "builtin": calls.peak_builtin },
            "lifecycle": lifecycle_json(&busy_log),
            "idle": idle,
        }))
    }

    /// The quiet after the busy window: no more calls while the panes go on
    /// streaming, until every service has stopped as idle or `--idle-wait`
    /// is up; then the memory left, and a cold and a warm call to each
    /// service, which is what stopping it costs the next use.
    #[allow(clippy::too_many_arguments)]
    fn idle(
        opts: &Opts,
        app: &mut App,
        ui: &mut Ui,
        target: &Offscreen,
        size: (f32, f32),
        v: &TestVornd,
        exts: &[extensions::Ext],
    ) -> Result<Value, String> {
        const POLL: Duration = Duration::from_millis(500);
        let quiet = Instant::now();
        let mut polled = quiet;
        let mut left = extensions::running(app.rpc())?;
        while left != (0, 0) && quiet.elapsed() < opts.idle_wait {
            draw(app, ui, target, size);
            std::thread::sleep(FRAME);
            if polled.elapsed() >= POLL {
                polled = Instant::now();
                left = extensions::running(app.rpc())?;
            }
        }
        let waited = quiet.elapsed().as_secs_f64();
        let pids = v.pids();
        let programs = extensions::programs_rss(pids[0], &pids[1..]);
        let daemons = procs_rss(&pids);
        let lifecycle = extensions::read_log(&v.log());
        let mut cold: BTreeMap<&str, Vec<f64>> = BTreeMap::new();
        let mut warm: BTreeMap<&str, Vec<f64>> = BTreeMap::new();
        for ext in exts.iter().filter(|e| e.service) {
            let lang = ext.lang.name();
            cold.entry(lang)
                .or_default()
                .push(extensions::call(app.rpc(), ext)?);
            warm.entry(lang)
                .or_default()
                .push(extensions::call(app.rpc(), ext)?);
        }
        let by_lang = |m: &BTreeMap<&str, Vec<f64>>| -> Value {
            m.iter()
                .map(|(lang, ms)| (lang.to_string(), json!(pct(ms, 50.0))))
                .collect::<serde_json::Map<_, _>>()
                .into()
        };
        Ok(json!({
            "seconds": round(waited),
            "all_stopped": left == (0, 0),
            "running": { "bench": left.0, "builtin": left.1 },
            "rss_mb": {
                "daemons": round(daemons),
                "programs": round(programs.values().map(|p| p.1).sum()),
            },
            "lifecycle": lifecycle_json(&lifecycle),
            "cold_call_ms": by_lang(&cold),
            "warm_call_ms": by_lang(&warm),
        }))
    }

    fn programs_json(programs: &BTreeMap<&str, (usize, f64)>) -> Value {
        programs
            .iter()
            .map(|(lang, (count, mb))| {
                (
                    lang.to_string(),
                    json!({ "processes": count, "mb": round(*mb) }),
                )
            })
            .collect::<serde_json::Map<_, _>>()
            .into()
    }

    fn lifecycle_json(l: &extensions::Lifecycle) -> Value {
        json!({
            "starts": l.starts,
            "restarts": l.restarts,
            "cap_stops": l.evictions,
            "crashes": l.crashes,
            "idle_stops": l.idle_stops,
        })
    }

    /// The window's size `t` into a two-second drag to 60% by 70% and back.
    fn dragged(full: (f32, f32), t: Duration) -> (f32, f32) {
        let phase = (t.as_secs_f32() / 2.0).fract();
        let f = 1.0 - (2.0 * phase - 1.0).abs();
        (
            (full.0 * (1.0 - 0.4 * f)).round(),
            (full.1 * (1.0 - 0.3 * f)).round(),
        )
    }

    /// One frame: build, lay out, paint and draw, as the window does.
    fn draw(app: &mut App, ui: &mut Ui, target: &Offscreen, size: (f32, f32)) {
        app.frame(ui, size);
        // A full atlas empties itself; the second pass fits one frame.
        if !ui.render_offscreen(target) {
            app.frame(ui, size);
            ui.render_offscreen(target);
        }
    }

    /// Whether the probe's screen shows its answer to `c`.
    fn shows(app: &App, probe: &str, c: char) -> bool {
        let want = format!("key {c}");
        app.pane(probe)
            .is_some_and(|p| p.text().lines().any(|l| l.trim_end() == want))
    }

    fn quote(p: &Path) -> String {
        format!("'{}'", p.display().to_string().replace('\'', r"'\''"))
    }

    fn ms(d: Duration) -> f64 {
        d.as_secs_f64() * 1000.0
    }

    fn round(x: f64) -> f64 {
        (x * 100.0).round() / 100.0
    }

    /// The nearest-rank percentile, 0 for no samples.
    fn pct(values: &[f64], p: f64) -> f64 {
        if values.is_empty() {
            return 0.0;
        }
        let mut v = values.to_vec();
        v.sort_by(f64::total_cmp);
        let rank = ((p / 100.0) * v.len() as f64).ceil() as usize;
        round(v[rank.clamp(1, v.len()) - 1])
    }

    /// This process's CPU time, in seconds.
    fn self_cpu() -> f64 {
        // SAFETY: rusage is plain data, and getrusage only fills it.
        let ru = unsafe {
            let mut ru: libc::rusage = std::mem::zeroed();
            libc::getrusage(libc::RUSAGE_SELF, &mut ru);
            ru
        };
        let t = |tv: libc::timeval| tv.tv_sec as f64 + tv.tv_usec as f64 / 1e6;
        t(ru.ru_utime) + t(ru.ru_stime)
    }

    /// The CPU time of `pids` together, in seconds.
    fn procs_cpu(pids: &[u32]) -> f64 {
        if cfg!(target_os = "linux") {
            // SAFETY: sysconf has no preconditions.
            let hz = unsafe { libc::sysconf(libc::_SC_CLK_TCK) }.max(1) as f64;
            return pids
                .iter()
                .filter_map(|p| std::fs::read_to_string(format!("/proc/{p}/stat")).ok())
                .filter_map(|s| {
                    // utime and stime, counted after the command, whose name may hold spaces.
                    let f: Vec<&str> = s.rsplit_once(')')?.1.split_whitespace().collect();
                    let tick = |i: usize| f.get(i)?.parse::<f64>().ok();
                    Some((tick(11)? + tick(12)?) / hz)
                })
                .sum();
        }
        let list = pids
            .iter()
            .map(u32::to_string)
            .collect::<Vec<_>>()
            .join(",");
        let Ok(out) = std::process::Command::new("ps")
            .args(["-o", "time=", "-p", &list])
            .output()
        else {
            return 0.0;
        };
        // `[[hh:]mm:]ss.cc` per process.
        String::from_utf8_lossy(&out.stdout)
            .lines()
            .map(|l| {
                l.trim().split(':').fold(0.0, |acc, part| {
                    acc * 60.0 + part.parse::<f64>().unwrap_or(0.0)
                })
            })
            .sum()
    }

    /// The resident size of `pids` together, in MB.
    fn procs_rss(pids: &[u32]) -> f64 {
        let list = pids
            .iter()
            .map(u32::to_string)
            .collect::<Vec<_>>()
            .join(",");
        let Ok(out) = std::process::Command::new("ps")
            .args(["-o", "rss=", "-p", &list])
            .output()
        else {
            return 0.0;
        };
        String::from_utf8_lossy(&out.stdout)
            .lines()
            .filter_map(|l| l.trim().parse::<f64>().ok())
            .sum::<f64>()
            / 1024.0
    }

    fn machine() -> Value {
        let sysctl = |k: &str| {
            std::process::Command::new("sysctl")
                .args(["-n", k])
                .output()
                .ok()
                .map(|o| String::from_utf8_lossy(&o.stdout).trim().to_owned())
                .filter(|s| !s.is_empty())
        };
        json!({
            "os": std::env::consts::OS,
            "arch": std::env::consts::ARCH,
            "cpu": sysctl("machdep.cpu.brand_string"),
            "cores": std::thread::available_parallelism().map_or(0, |n| n.get()),
        })
    }

    /// The results as a Markdown table.
    fn table(results: &[Value]) -> String {
        let mut s = String::from(
            "| Case | Panes | Ext | Shown | Frame p50 | Frame p99 | Over 16.7 ms | Input to pixel p50 | p99 | CPU app | CPU vornd | CPU per pane | RSS app | RSS vornd |\n\
             | ---- | ----: | --: | ----: | --------: | --------: | -----------: | -----------------: | --: | ------: | --------: | -----------: | ------: | --------: |\n",
        );
        for r in results {
            let _ = writeln!(
                s,
                "| {} | {} | {} | {} | {} ms | {} ms | {}% | {} ms | {} ms | {}% | {}% | {}% | {} MB | {} MB |",
                r["case"].as_str().unwrap_or_default(),
                r["panes"],
                r["extensions"],
                r["shown"],
                r["frame_ms"]["p50"],
                r["frame_ms"]["p99"],
                r["over_budget_pct"],
                r["input_to_pixel_ms"]["p50"],
                r["input_to_pixel_ms"]["p99"],
                r["cpu_pct"]["app"],
                r["cpu_pct"]["daemons"],
                r["cpu_pct"]["per_pane"],
                r["rss_mb"]["app"],
                r["rss_mb"]["daemons"],
            );
        }
        if let Some(r) = results.first() {
            let _ = writeln!(
                s,
                "\nRenderer: {}, {}.",
                r["renderer"].as_str().unwrap_or_default(),
                r["size"].as_str().unwrap_or_default()
            );
        }
        s
    }

    /// The measures the summary gives, as JSON pointers into a result.
    const MEASURES: [(&str, &str); 12] = [
        ("Frame p50 ms", "/frame_ms/p50"),
        ("Frame p99 ms", "/frame_ms/p99"),
        ("Input p50 ms", "/input_to_pixel_ms/p50"),
        ("Input p99 ms", "/input_to_pixel_ms/p99"),
        ("Call p50 ms", "/ext_calls/p50"),
        ("RSS vornd MB", "/rss_mb/daemons"),
        ("RSS programs MB", "/rss_mb/programs"),
        ("Running peak", "/running_peak/bench"),
        ("Restarts", "/lifecycle/restarts"),
        ("Cap stops", "/lifecycle/cap_stops"),
        ("Idle RSS vornd MB", "/idle/rss_mb/daemons"),
        ("Idle RSS programs MB", "/idle/rss_mb/programs"),
    ];
    /// The measures that must not move with extensions installed.
    const HELD: [&str; 4] = [
        "/frame_ms/p50",
        "/frame_ms/p99",
        "/input_to_pixel_ms/p50",
        "/input_to_pixel_ms/p99",
    ];

    /// The repetitions of each case, panes and extensions as the median and
    /// range of each measure, then whether each held measure with extensions
    /// is within noise of the same case without: its median no further from
    /// the baseline's than the wider of the two ranges.
    fn summary(results: &[Value]) -> String {
        let mut groups: BTreeMap<(String, u64, u64), Vec<&Value>> = BTreeMap::new();
        for r in results {
            let key = (
                r["case"].as_str().unwrap_or_default().to_owned(),
                r["panes"].as_u64().unwrap_or_default(),
                r["extensions"].as_u64().unwrap_or_default(),
            );
            groups.entry(key).or_default().push(r);
        }
        let stats = |runs: &[&Value], at: &str| -> Option<(f64, f64, f64)> {
            let mut v: Vec<f64> = runs
                .iter()
                .filter_map(|r| r.pointer(at)?.as_f64())
                .collect();
            v.sort_by(f64::total_cmp);
            let (first, last) = (*v.first()?, *v.last()?);
            let mid = v.len() / 2;
            let median = if v.len() % 2 == 1 {
                v[mid]
            } else {
                (v[mid - 1] + v[mid]) / 2.0
            };
            Some((round(median), first, last))
        };
        let mut s = String::from("\nMedian (min–max) over the repetitions:\n\n| Case | Panes | Ext | Runs |");
        for (label, _) in MEASURES {
            let _ = write!(s, " {label} |");
        }
        s += "\n| ---- | ----: | --: | ---: |";
        s += &" ---: |".repeat(MEASURES.len());
        s += "\n";
        for ((case, panes, ext), runs) in &groups {
            let _ = write!(s, "| {case} | {panes} | {ext} | {} |", runs.len());
            for (_, at) in MEASURES {
                match stats(runs, at) {
                    Some((m, lo, hi)) => {
                        let _ = write!(s, " {m} ({lo}–{hi}) |");
                    }
                    None => s += " – |",
                }
            }
            s += "\n";
        }
        s += "\n";
        for ((case, panes, ext), runs) in groups.iter().filter(|(k, _)| k.2 > 0) {
            let Some(base) = groups.get(&(case.clone(), *panes, 0)) else {
                continue;
            };
            for at in HELD {
                let (Some((m, lo, hi)), Some((m0, lo0, hi0))) = (stats(runs, at), stats(base, at))
                else {
                    continue;
                };
                let noise = (hi - lo).max(hi0 - lo0);
                let delta = round(m - m0);
                let verdict = if delta.abs() <= noise {
                    "within noise"
                } else if delta < 0.0 {
                    "better"
                } else {
                    "OUTSIDE NOISE"
                };
                let _ = writeln!(
                    s,
                    "{case}, {panes} panes, {ext} extensions, {at}: {m} vs {m0} ms, Δ {delta:+}, noise {}: {verdict}",
                    round(noise)
                );
            }
        }
        s
    }
}

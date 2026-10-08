//! The bench bookkeeping every prototype shares: what the harness asked for
//! (from the environment), the latency probe's schedule, and the raw numbers
//! written back as JSON. Times are in each UI's own clock, in milliseconds;
//! only differences are kept, so clocks never cross a process.

use std::fmt::Write as _;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use crate::Grid;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Mode {
    /// No bench: a terminal to use.
    Interactive,
    /// Type into pane 0 and time each glyph to the frame that draws it.
    Latency,
    /// Record frame times for a while (idle or under load).
    Frames,
    /// Exit once the first full frame is drawn.
    Start,
}

#[derive(Debug, Clone)]
pub struct Config {
    pub mode: Mode,
    pub settle: Duration,
    pub duration: Duration,
    pub probes: usize,
    pub out: Option<String>,
    /// The look test: draw the app screen mock instead of the pane grid,
    /// screenshot it to `shot` once settled (a `start`-mode run), then quit.
    pub look: bool,
    /// The look test's variant with each framework's platform polish.
    pub polish: bool,
    pub shot: Option<String>,
    /// Terminal font size in points.
    pub font_size: f32,
}

impl Config {
    pub fn from_env() -> Config {
        let var = |k: &str| std::env::var(k).ok();
        let ms = |k: &str, d: u64| {
            Duration::from_millis(var(k).and_then(|v| v.parse().ok()).unwrap_or(d))
        };
        Config {
            mode: match var("VORN_SPIKE_MODE").as_deref() {
                Some("latency") => Mode::Latency,
                Some("frames") => Mode::Frames,
                Some("start") => Mode::Start,
                _ => Mode::Interactive,
            },
            settle: ms("VORN_SPIKE_SETTLE_MS", 2000),
            duration: ms("VORN_SPIKE_DURATION_MS", 10_000),
            probes: var("VORN_SPIKE_PROBES")
                .and_then(|v| v.parse().ok())
                .unwrap_or(150),
            out: var("VORN_SPIKE_OUT"),
            look: var("VORN_SPIKE_LOOK").is_some_and(|v| v == "1"),
            polish: var("VORN_SPIKE_POLISH").is_some_and(|v| v == "1"),
            shot: var("VORN_SPIKE_SHOT"),
            font_size: var("VORN_SPIKE_FONT_SIZE")
                .and_then(|v| v.parse().ok())
                .unwrap_or(12.0),
        }
        .themed()
    }

    fn themed(self) -> Config {
        if self.look {
            crate::view::use_app_theme();
        }
        self
    }

    /// Takes the look test's screenshot, if this run wants one.
    pub fn shoot(&self) {
        if let Some(path) = &self.shot {
            match crate::shot::capture(path) {
                Ok(()) => eprintln!("shot: {path}"),
                Err(e) => eprintln!("shot failed: {e}"),
            }
        }
    }
}

/// Nanoseconds since the epoch: the one clock the harness and a client share.
pub fn epoch_ns() -> u128 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |d| d.as_nanos())
}

/// Tells the harness the first full frame is on screen.
pub fn print_first_frame() {
    use std::io::Write;
    let mut o = std::io::stdout().lock();
    let _ = writeln!(o, "FIRST_FRAME {}", epoch_ns());
    let _ = o.flush();
}

/// What the latency driver wants done now.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Step {
    Idle,
    /// Inject this key into pane 0 (the probe is already armed).
    Type(char),
    /// Inject Enter, unmeasured, to start a new line.
    Enter,
    /// Write the results and quit.
    Done,
}

pub struct Bench {
    pub cfg: Config,
    started: Instant,
    /// When the measured window opened (after the settle time).
    window: Option<Instant>,
    next: Instant,
    seq: u32,
    on_line: u32,
    outstanding: Option<Instant>,
    rng: u64,
    pub latency_ms: Vec<f64>,
    pub lost: u32,
    pub frame_at_ms: Vec<f64>,
    pub frame_work_ms: Vec<f64>,
    pub period_ms: f64,
    pub first_frame_ns: Option<u128>,
}

impl Bench {
    pub fn new(cfg: Config) -> Bench {
        let now = Instant::now();
        Bench {
            cfg,
            started: now,
            window: None,
            next: now,
            seq: 0,
            on_line: 0,
            outstanding: None,
            rng: 0x9e37_79b9_7f4a_7c15,
            latency_ms: Vec::new(),
            lost: 0,
            frame_at_ms: Vec::new(),
            frame_work_ms: Vec::new(),
            period_ms: 1000.0 / 60.0,
            first_frame_ns: None,
        }
    }

    fn rand_ms(&mut self, span: u64) -> u64 {
        self.rng ^= self.rng << 13;
        self.rng ^= self.rng >> 7;
        self.rng ^= self.rng << 17;
        self.rng % span
    }

    /// Whether frames are being recorded now.
    pub fn measuring(&self) -> bool {
        self.window.is_some()
    }

    /// Call once per UI tick (a timer or a frame callback).
    pub fn tick(&mut self, grid: &Grid) -> Step {
        let now = Instant::now();
        if self.first_frame_ns.is_none() {
            return Step::Idle;
        }
        if self.window.is_none() {
            if now.duration_since(self.started) < self.cfg.settle {
                return Step::Idle;
            }
            self.window = Some(now);
            self.next = now;
        }
        let window = self.window.unwrap_or(now);
        match self.cfg.mode {
            Mode::Interactive => Step::Idle,
            Mode::Start => Step::Done,
            Mode::Frames => {
                if now.duration_since(window) >= self.cfg.duration {
                    Step::Done
                } else {
                    Step::Idle
                }
            }
            Mode::Latency => {
                if let Some(since) = self.outstanding {
                    if !grid.probe_pending(0) {
                        self.outstanding = None;
                    } else if now.duration_since(since) > Duration::from_secs(1) {
                        grid.probe_cancel(0);
                        self.outstanding = None;
                        self.lost += 1;
                    } else {
                        return Step::Idle;
                    }
                }
                if now < self.next {
                    return Step::Idle;
                }
                if self.latency_ms.len() + self.lost as usize >= self.cfg.probes {
                    return Step::Done;
                }
                if self.on_line >= 30 {
                    self.on_line = 0;
                    self.next = now + Duration::from_millis(300);
                    return Step::Enter;
                }
                let ch = (b'a' + (self.seq % 26) as u8) as char;
                self.seq += 1;
                self.on_line += 1;
                self.outstanding = Some(now);
                // Jittered so typing does not lock to the display's phase.
                self.next = now + Duration::from_millis(90 + self.rand_ms(80));
                grid.probe_arm(0, ch);
                Step::Type(ch)
            }
        }
    }

    /// A frame was drawn at `at_ms` (the UI's clock), its drawing took
    /// `work_ms`.
    pub fn frame(&mut self, at_ms: f64, work_ms: f64) {
        if self.first_frame_ns.is_none() {
            return;
        }
        if self.measuring() {
            self.frame_at_ms.push(at_ms);
            self.frame_work_ms.push(work_ms);
        }
    }

    /// The first frame in which every pane has its screen.
    pub fn first_frame(&mut self) {
        if self.first_frame_ns.is_none() {
            self.first_frame_ns = Some(epoch_ns());
            print_first_frame();
        }
    }

    pub fn latency(&mut self, ms: f64) {
        self.latency_ms.push(ms);
    }

    /// The results as JSON, written to the harness's file.
    pub fn write(&self, client: &str, panes: usize, extra: &[(&str, String)]) {
        let Some(path) = &self.cfg.out else {
            return;
        };
        let list = |v: &[f64]| {
            let mut s = String::from("[");
            for (i, x) in v.iter().enumerate() {
                if i > 0 {
                    s.push(',');
                }
                let _ = write!(s, "{x:.3}");
            }
            s.push(']');
            s
        };
        let mut s = String::from("{");
        let _ = write!(
            s,
            "\"client\":\"{client}\",\"mode\":\"{:?}\",\"panes\":{panes},\"period_ms\":{:.3},\
             \"first_frame_epoch_ns\":{},\"lost\":{},\"latency_ms\":{},\"frame_at_ms\":{},\
             \"frame_work_ms\":{},\"on_screen\":{}",
            self.cfg.mode,
            self.period_ms,
            self.first_frame_ns.unwrap_or(0),
            self.lost,
            list(&self.latency_ms),
            list(&self.frame_at_ms),
            list(&self.frame_work_ms),
            crate::shot::on_screen(),
        );
        for (k, v) in extra {
            let _ = write!(s, ",\"{k}\":{v}");
        }
        s.push('}');
        let _ = std::fs::write(path, s);
    }
}

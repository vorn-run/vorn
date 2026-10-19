//! The terminal while it is drawn as pixels: finding out what it can do,
//! taking it over, reading its input raw (crossterm's mouse reader has no
//! pixel reports, and its keys are reused once parsed), and sending each
//! frame the CPU renderer draws through a [`Presenter`].

use std::collections::VecDeque;
use std::io::{self, Write};
use std::time::{Duration, Instant};

use super::input::{MouseAction, MouseReport, Parsed, Parser, Reply};
use super::kitty::{self, Wrap};
use super::present::{Frame, Placing, Presenter, Rect, Sent, Transport};
use crate::cells::term::{map_key, pointer_name, pointer_shapes_from_env, TermEvent};
use crate::cpu::{layers_bounds, CpuRaster};
use crate::interact::WHEEL_LINE;
use crate::ui::Ui;

/// Image ids the probe asks with; never kept by the terminal.
const PROBE_DIRECT: u32 = 31;
const PROBE_FILE: u32 = 32;
/// Inside tmux the device attributes come from tmux itself, before the
/// terminal's answers to what passed through: wait this much longer.
const TMUX_GRACE: Duration = Duration::from_millis(150);
/// A cell this many pixels tall is drawn at scale 1.
const CELL_H_AT_1X: f32 = 17.0;

/// What the terminal can do, as its answers to the probe said.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct Caps {
    /// It shows kitty graphics.
    pub graphics: bool,
    /// It reads images from temporary files: it runs on this machine.
    pub files: bool,
    /// A cell's size in pixels.
    pub cell: Option<(u32, u32)>,
    /// It reports the mouse in pixels (mode 1016).
    pub pixel_mouse: bool,
    /// It speaks the kitty keyboard protocol.
    pub kitty_keys: bool,
    /// It reports resizes in band, with pixel sizes (mode 2048).
    pub resize_reports: bool,
    /// It is reached through tmux.
    pub tmux: bool,
}

impl Caps {
    /// Whether the UI can be drawn as pixels here.
    pub fn pixels(&self) -> bool {
        self.graphics && self.cell.is_some_and(|(w, h)| w > 0 && h > 0)
    }
}

/// The probe's findings, and what was typed while it waited.
#[derive(Debug, Clone, Default)]
pub struct Probe {
    pub caps: Caps,
    pub typed: Vec<TermEvent>,
}

/// Asks the terminal what it can do, waiting at most `timeout` for the
/// answers. The device attributes query goes last: every terminal answers
/// it, in order, so once it arrives the rest have or never will.
pub fn probe(timeout: Duration) -> io::Result<Probe> {
    let Ok(tty) = Tty::open() else {
        return Ok(Probe::default());
    };
    let was_raw = crossterm::terminal::is_raw_mode_enabled().unwrap_or(false);
    crossterm::terminal::enable_raw_mode()?;
    let result = run_probe(&tty, timeout);
    if !was_raw {
        crossterm::terminal::disable_raw_mode()?;
    }
    result
}

fn run_probe(tty: &Tty, timeout: Duration) -> io::Result<Probe> {
    let wrap = Wrap::from_env();
    let file = std::env::temp_dir().join(format!(
        "tty-graphics-protocol-vorn-probe-{}",
        std::process::id()
    ));
    let file_ok = std::fs::write(&file, [0u8; 3]).is_ok();
    let mut q = Vec::new();
    kitty::query_direct(&mut q, wrap, PROBE_DIRECT);
    if file_ok {
        kitty::query_file(&mut q, wrap, PROBE_FILE, &file);
    }
    wrap.put(&mut q, b"\x1b[16t\x1b[14t");
    q.extend_from_slice(b"\x1b[?1016$p\x1b[?2048$p\x1b[?u\x1b[c");
    let mut out = io::stdout();
    out.write_all(&q)?;
    out.flush()?;

    let mut caps = Caps {
        tmux: wrap == Wrap::Tmux,
        ..Caps::default()
    };
    let mut typed = Vec::new();
    let mut area = None;
    let mut parser = Parser::default();
    let mut parsed = Vec::new();
    let mut buf = [0u8; 4096];
    let mut deadline = Instant::now() + timeout;
    let mut fenced = false;
    loop {
        let now = Instant::now();
        if now >= deadline {
            break;
        }
        let n = tty.read(deadline - now, &mut buf)?;
        parser.feed(&buf[..n], &mut parsed);
        for p in parsed.drain(..) {
            match p {
                Parsed::Reply(Reply::Graphics { id, ok }) if id == PROBE_DIRECT => {
                    caps.graphics = ok;
                }
                Parsed::Reply(Reply::Graphics { id, ok }) if id == PROBE_FILE => caps.files = ok,
                Parsed::Reply(Reply::CellSize(w, h)) => caps.cell = Some((w, h)),
                Parsed::Reply(Reply::TextArea(w, h)) => area = Some((w, h)),
                Parsed::Reply(Reply::Mode { mode, state }) => {
                    let known = matches!(state, 1..=3);
                    match mode {
                        1016 => caps.pixel_mouse = known,
                        2048 => caps.resize_reports = known,
                        _ => {}
                    }
                }
                Parsed::Reply(Reply::KeyboardFlags(_)) => caps.kitty_keys = true,
                Parsed::Reply(Reply::DeviceAttributes) => fenced = true,
                Parsed::Reply(_) => {}
                Parsed::Key(k) => typed.extend(map_key(&k).map(TermEvent::Input)),
                Parsed::Paste(s) => typed.push(TermEvent::Paste(s)),
                Parsed::Mouse(_) | Parsed::Focus(_) => {}
            }
        }
        if fenced {
            let waited = caps.graphics && (caps.files || !file_ok);
            if !caps.tmux || waited {
                break;
            }
            deadline = deadline.min(Instant::now() + TMUX_GRACE);
        }
    }
    // A terminal that read the file deleted it; one that did not leaves it.
    let _ = std::fs::remove_file(&file);
    if caps.tmux {
        // tmux does not pass pixel mouse reports through.
        caps.pixel_mouse = false;
    }
    if caps.cell.is_none() {
        caps.cell = cell_from_ioctl().or_else(|| {
            let (cols, rows) = crossterm::terminal::size().ok()?;
            let (w, h) = area?;
            (cols > 0 && rows > 0).then(|| (w / u32::from(cols), h / u32::from(rows)))
        });
    }
    Ok(Probe { caps, typed })
}

/// A cell's size from the window size the kernel keeps, where the
/// terminal filled in its pixels.
fn cell_from_ioctl() -> Option<(u32, u32)> {
    let ws = crossterm::terminal::window_size().ok()?;
    (ws.columns > 0 && ws.rows > 0 && ws.width > 0 && ws.height > 0).then(|| {
        (
            u32::from(ws.width) / u32::from(ws.columns),
            u32::from(ws.height) / u32::from(ws.rows),
        )
    })
}

/// The scale the UI is drawn at for cells `cell_h` pixels tall: what makes
/// its text about the size of the terminal's, in quarter steps.
pub(crate) fn scale_for(cell_h: u32) -> f32 {
    ((cell_h as f32 / CELL_H_AT_1X * 4.0).round() / 4.0).clamp(1.0, 3.0)
}

/// The terminal, owned while the UI is drawn on it as pixels; dropping it
/// gives it back as it was.
pub struct PixelTerminal {
    out: io::Stdout,
    tty: Tty,
    caps: Caps,
    wrap: Wrap,
    parser: Parser,
    parsed: Vec<Parsed>,
    events: VecDeque<TermEvent>,
    presenter: Presenter,
    /// What is under the overlays, and (while there are any) the whole frame.
    under: CpuRaster,
    all: CpuRaster,
    grid: (u16, u16),
    cell: (u32, u32),
    buf: Vec<u8>,
    read: Box<[u8]>,
    shapes: bool,
    pointer: &'static str,
    active: bool,
}

impl PixelTerminal {
    /// Takes over the terminal `caps` describes, which must show pixels.
    pub fn enter(caps: Caps) -> io::Result<PixelTerminal> {
        let cell = caps
            .cell
            .filter(|_| caps.pixels())
            .ok_or_else(|| io::Error::new(io::ErrorKind::Unsupported, "no kitty graphics"))?;
        let tty = Tty::open()?;
        crossterm::terminal::enable_raw_mode()?;
        let wrap = if caps.tmux { Wrap::Tmux } else { Wrap::Direct };
        let placing = if caps.tmux {
            Placing::Cells
        } else {
            Placing::Cursor
        };
        let transport = Transport {
            file_dir: caps.files.then(std::env::temp_dir),
            zlib: !caps.files,
        };
        let mut t = PixelTerminal {
            out: io::stdout(),
            tty,
            caps,
            wrap,
            parser: Parser::default(),
            parsed: Vec::new(),
            events: VecDeque::new(),
            presenter: Presenter::new(wrap, placing, cell, transport),
            under: CpuRaster::new(false),
            all: CpuRaster::new(false),
            grid: crossterm::terminal::size()?,
            cell,
            buf: Vec::with_capacity(256 * 1024),
            read: vec![0; 16 * 1024].into_boxed_slice(),
            shapes: pointer_shapes_from_env(),
            pointer: "default",
            active: true,
        };
        let mut seq = Vec::new();
        seq.extend_from_slice(b"\x1b[?1049h\x1b[?25l\x1b[2J");
        // SGR first: setting 1016 after it switches the reports to pixels.
        seq.extend_from_slice(b"\x1b[?1000h\x1b[?1002h\x1b[?1003h\x1b[?1006h");
        if caps.pixel_mouse {
            seq.extend_from_slice(b"\x1b[?1016h");
        }
        seq.extend_from_slice(b"\x1b[?2004h\x1b[?1004h");
        if caps.resize_reports {
            seq.extend_from_slice(b"\x1b[?2048h");
        }
        if caps.kitty_keys {
            // Disambiguated escapes and alternate keys, as the text terminal asks.
            seq.extend_from_slice(b"\x1b[>5u");
        }
        t.out.write_all(&seq)?;
        t.out.flush()?;
        Ok(t)
    }

    pub fn caps(&self) -> Caps {
        self.caps
    }

    /// The grid's size in cells.
    pub fn grid(&self) -> (u16, u16) {
        self.grid
    }

    /// The scale to lay the UI out at for this terminal's cells.
    pub fn scale(&self) -> f32 {
        scale_for(self.cell.1)
    }

    /// The frame's size in pixels: the whole grid.
    pub fn frame_px(&self) -> (u32, u32) {
        (
            u32::from(self.grid.0) * self.cell.0,
            u32::from(self.grid.1) * self.cell.1,
        )
    }

    /// The frame's size in the layout's logical pixels.
    pub fn logical_size(&self) -> (f32, f32) {
        let (w, h) = self.frame_px();
        let s = self.scale();
        (w as f32 / s, h as f32 / s)
    }

    /// Forgets what the terminal shows, so the next frame is sent whole.
    pub fn invalidate(&mut self) {
        self.presenter.invalidate();
        self.under.invalidate();
        self.all.invalidate();
    }

    /// The next event within `timeout`, if any.
    pub fn poll(&mut self, timeout: Duration) -> io::Result<Option<TermEvent>> {
        if let Some(e) = self.events.pop_front() {
            return Ok(Some(e));
        }
        if !self.caps.resize_reports {
            self.check_size();
        }
        if let Some(e) = self.events.pop_front() {
            return Ok(Some(e));
        }
        // An ESC alone waits a moment for the rest of its sequence.
        let wait = if self.parser.pending() {
            timeout.min(Duration::from_millis(10))
        } else {
            timeout
        };
        let n = self.tty.read(wait, &mut self.read)?;
        if n == 0 {
            self.parser.flush(&mut self.parsed);
        } else {
            self.parser.feed(&self.read[..n], &mut self.parsed);
        }
        let mut parsed = std::mem::take(&mut self.parsed);
        for p in parsed.drain(..) {
            self.take(p);
        }
        self.parsed = parsed;
        Ok(self.events.pop_front())
    }

    fn take(&mut self, p: Parsed) {
        let e = match p {
            Parsed::Key(k) => map_key(&k).map(TermEvent::Input),
            Parsed::Paste(s) => Some(TermEvent::Paste(s)),
            Parsed::Focus(f) => Some(TermEvent::Focus(f)),
            Parsed::Mouse(m) => self.mouse(m),
            Parsed::Reply(Reply::Resize { cols, rows, w, h }) => {
                self.grid = (cols, rows);
                if cols > 0 && rows > 0 && w > 0 && h > 0 {
                    self.set_cell((w / u32::from(cols), h / u32::from(rows)));
                }
                Some(TermEvent::Resize(cols, rows))
            }
            Parsed::Reply(Reply::CellSize(w, h)) if (w, h) != self.cell && w > 0 && h > 0 => {
                self.set_cell((w, h));
                Some(TermEvent::Resize(self.grid.0, self.grid.1))
            }
            Parsed::Reply(_) => None,
        };
        self.events.extend(e);
    }

    fn set_cell(&mut self, cell: (u32, u32)) {
        if cell != self.cell && cell.0 > 0 && cell.1 > 0 {
            self.cell = cell;
            self.presenter.set_cell(cell);
        }
    }

    /// Without in-band reports, reads the grid's size from the kernel; a
    /// change that leaves the cell size unknown asks the terminal for it.
    fn check_size(&mut self) {
        let Ok(grid) = crossterm::terminal::size() else {
            return;
        };
        if grid == self.grid {
            return;
        }
        self.grid = grid;
        match cell_from_ioctl() {
            Some(cell) => self.set_cell(cell),
            None => {
                let mut q = Vec::new();
                self.wrap.put(&mut q, b"\x1b[16t");
                let _ = self.out.write_all(&q).and_then(|()| self.out.flush());
            }
        }
        self.events.push_back(TermEvent::Resize(grid.0, grid.1));
    }

    fn mouse(&self, m: MouseReport) -> Option<TermEvent> {
        let (x, y) = mouse_point(m, self.caps.pixel_mouse, self.cell, self.scale());
        Some(match m.action()? {
            MouseAction::Down(0) => TermEvent::Down(x, y),
            MouseAction::Down(2) => TermEvent::SecondaryDown(x, y),
            MouseAction::Up(0) => TermEvent::Up(x, y),
            MouseAction::Move => TermEvent::Move(x, y),
            MouseAction::Wheel(n) => TermEvent::Wheel(x, y, f32::from(n) * WHEEL_LINE),
            MouseAction::Down(_) | MouseAction::Up(_) => return None,
        })
    }

    /// Draws the frame `ui` last laid out and sends what changed. Answers
    /// `None` when it was not drawn because the atlases filled up: lay the
    /// frame out again and call this again.
    pub fn present(&mut self, ui: &mut Ui) -> io::Result<Option<Sent>> {
        let px = self.frame_px();
        let overlays = match self.presenter.placing() {
            Placing::Cursor => ui.overlay_layers(),
            // Placeholder cells cannot stack: one image for everything.
            Placing::Cells => 0..0,
        };
        let split = !overlays.is_empty();
        let drawn = if split {
            ui.render_rasters(
                &mut [(&mut self.under, overlays.clone()), (&mut self.all, 0..0)],
                px,
            )
        } else {
            ui.render_rasters(&mut [(&mut self.under, 0..0)], px)
        };
        if !drawn {
            return Ok(None);
        }
        let rect: Option<Rect> = split
            .then(|| layers_bounds(&ui.scene, overlays, px))
            .flatten()
            .map(|b| b.map(|v| v.max(0) as u32));
        self.buf.clear();
        self.buf.extend_from_slice(b"\x1b[?2026h");
        let mark = self.buf.len();
        let under = frame_of(&self.under);
        let overlay = rect.map(|r| (frame_of(&self.all), r));
        let sent = self.presenter.present(under, overlay, &mut self.buf);
        let pointer = pointer_name(ui.cursor());
        if self.shapes && pointer != self.pointer {
            self.pointer = pointer;
            let _ = write!(self.buf, "\x1b]22;{pointer}\x1b\\");
        }
        if self.buf.len() > mark {
            self.buf.extend_from_slice(b"\x1b[?2026l");
            self.out.write_all(&self.buf)?;
            self.out.flush()?;
        }
        Ok(Some(sent))
    }

    /// Gives the terminal back.
    pub fn leave(&mut self) -> io::Result<()> {
        if !self.active {
            return Ok(());
        }
        self.active = false;
        let mut seq = Vec::new();
        self.presenter.clear(&mut seq);
        kitty::delete_all(&mut seq, self.wrap);
        if self.pointer != "default" {
            seq.extend_from_slice(b"\x1b]22;default\x1b\\");
        }
        if self.caps.kitty_keys {
            seq.extend_from_slice(b"\x1b[<u");
        }
        if self.caps.resize_reports {
            seq.extend_from_slice(b"\x1b[?2048l");
        }
        seq.extend_from_slice(b"\x1b[?1004l\x1b[?2004l");
        if self.caps.pixel_mouse {
            seq.extend_from_slice(b"\x1b[?1016l");
        }
        seq.extend_from_slice(b"\x1b[?1006l\x1b[?1003l\x1b[?1002l\x1b[?1000l");
        seq.extend_from_slice(b"\x1b[?25h\x1b[?1049l");
        self.out.write_all(&seq)?;
        self.out.flush()?;
        crossterm::terminal::disable_raw_mode()
    }
}

impl Drop for PixelTerminal {
    fn drop(&mut self) {
        let _ = self.leave();
    }
}

fn frame_of(r: &CpuRaster) -> Frame<'_> {
    Frame {
        px: r.pixels(),
        size: r.size(),
        dirty: r.dirty_tiles(),
        tiles_x: r.tiles_x(),
        tile: CpuRaster::TILE_PX,
    }
}

/// Where a mouse report points, in logical pixels: its pixel (mode 1016,
/// 0-based and negative over the padding) or its cell's centre.
fn mouse_point(m: MouseReport, pixels: bool, cell: (u32, u32), scale: f32) -> (f32, f32) {
    let (x, y) = if pixels {
        (m.x.max(0) as f32 + 0.5, m.y.max(0) as f32 + 0.5)
    } else {
        (
            ((m.x - 1).max(0) as f32 + 0.5) * cell.0 as f32,
            ((m.y - 1).max(0) as f32 + 0.5) * cell.1 as f32,
        )
    };
    (x / scale, y / scale)
}

/// The terminal's input, read with a timeout on this thread, so the text
/// terminal can take it back at any time.
#[cfg(unix)]
struct Tty {
    fd: std::os::fd::RawFd,
    /// `/dev/tty`, when standard input is not the terminal.
    _own: Option<std::fs::File>,
}

#[cfg(unix)]
impl Tty {
    fn open() -> io::Result<Tty> {
        use std::os::fd::AsRawFd;
        // SAFETY: isatty only asks about the descriptor.
        if unsafe { libc::isatty(0) } == 1 {
            return Ok(Tty { fd: 0, _own: None });
        }
        let f = std::fs::File::open("/dev/tty")?;
        Ok(Tty {
            fd: f.as_raw_fd(),
            _own: Some(f),
        })
    }

    /// Reads what arrives within `timeout` into `buf`; 0 when nothing did.
    /// `select`, since macOS's `poll` does not take terminal devices.
    fn read(&self, timeout: Duration, buf: &mut [u8]) -> io::Result<usize> {
        if self.fd < 0 || self.fd as usize >= libc::FD_SETSIZE {
            return Err(io::Error::from(io::ErrorKind::InvalidInput));
        }
        let mut tv = libc::timeval {
            tv_sec: timeout.as_secs().min(i32::MAX as u64) as libc::time_t,
            tv_usec: timeout.subsec_micros() as libc::suseconds_t,
        };
        // SAFETY: the set is zeroed then filled by FD_ZERO/FD_SET with a
        // descriptor below FD_SETSIZE; select reads and writes only it and tv.
        let ready = unsafe {
            let mut set: libc::fd_set = std::mem::zeroed();
            libc::FD_ZERO(&mut set);
            libc::FD_SET(self.fd, &mut set);
            libc::select(
                self.fd + 1,
                &mut set,
                std::ptr::null_mut(),
                std::ptr::null_mut(),
                &mut tv,
            )
        };
        if ready < 0 {
            let e = io::Error::last_os_error();
            return if e.kind() == io::ErrorKind::Interrupted {
                Ok(0)
            } else {
                Err(e)
            };
        }
        if ready == 0 {
            return Ok(0);
        }
        // SAFETY: buf is valid for writes of buf.len() bytes.
        let n = unsafe { libc::read(self.fd, buf.as_mut_ptr().cast(), buf.len()) };
        if n < 0 {
            let e = io::Error::last_os_error();
            return match e.kind() {
                io::ErrorKind::Interrupted | io::ErrorKind::WouldBlock => Ok(0),
                _ => Err(e),
            };
        }
        if n == 0 {
            return Err(io::Error::from(io::ErrorKind::UnexpectedEof));
        }
        Ok(n as usize)
    }
}

/// No raw tty here: the probe finds nothing and text cells draw.
#[cfg(not(unix))]
struct Tty;

#[cfg(not(unix))]
impl Tty {
    fn open() -> io::Result<Tty> {
        Err(io::Error::from(io::ErrorKind::Unsupported))
    }

    fn read(&self, _timeout: Duration, _buf: &mut [u8]) -> io::Result<usize> {
        Err(io::Error::from(io::ErrorKind::Unsupported))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn scale_follows_the_cell_height_in_quarter_steps() {
        assert_eq!(scale_for(17), 1.0);
        assert_eq!(scale_for(34), 2.0);
        assert_eq!(scale_for(26), 1.5);
        assert_eq!(scale_for(8), 1.0);
        assert_eq!(scale_for(90), 3.0);
    }

    #[test]
    fn mouse_reports_map_to_logical_pixels() {
        let at = |x, y| MouseReport {
            code: 0,
            x,
            y,
            release: false,
        };
        // Pixels at scale 2: the pixel's centre, halved.
        assert_eq!(mouse_point(at(0, 0), true, (16, 34), 2.0), (0.25, 0.25));
        assert_eq!(
            mouse_point(at(201, 99), true, (16, 34), 2.0),
            (100.75, 49.75)
        );
        // Over the padding the report goes negative; it is held at the edge.
        assert_eq!(mouse_point(at(-6, -3), true, (16, 34), 2.0), (0.25, 0.25));
        // Cells (1-based): the cell's centre in pixels, then logical.
        assert_eq!(mouse_point(at(1, 1), false, (16, 34), 2.0), (4.0, 8.5));
        assert_eq!(mouse_point(at(3, 2), false, (16, 34), 1.0), (40.0, 51.0));
    }
}

//! The Slint prototype: one window, a grid of panes, each drawn by the
//! `.slint` markup from runs of styled text that the shared Rust grid
//! client's views are cut into (the same code the other prototypes link).

use std::cell::RefCell;
use std::rc::Rc;
use std::sync::Arc;
use std::time::{Duration, Instant};

use slint::platform::{Key, WindowEvent};
use slint::{Color, ComponentHandle, Model, ModelRc, RenderingState, SharedString, VecModel};
use spike_core::bench::{Bench, Config, Mode, Step};
use spike_core::view::flags;
use spike_core::{Grid, PaneView};

slint::include_modules!();

struct State {
    app: slint::Weak<AppWindow>,
    grid: Arc<Grid>,
    runs: Vec<Rc<VecModel<Run>>>,
    panes: Rc<VecModel<PaneData>>,
    drawn: Vec<bool>,
    sizes: Vec<(u16, u16)>,
    cell: (f32, f32),
    bench: Bench,
    started: Instant,
    frame_start: Instant,
    typed_at: Option<Instant>,
    hit: bool,
    first: bool,
}

thread_local! {
    static ST: RefCell<Option<State>> = const { RefCell::new(None) };
}

fn with<R>(f: impl FnOnce(&mut State) -> R) -> Option<R> {
    ST.with(|s| s.borrow_mut().as_mut().map(f))
}

fn color(c: u32) -> Color {
    Color::from_rgb_u8((c >> 16) as u8, (c >> 8) as u8, c as u8)
}

/// The view as positioned runs: plain runs keep the font's advance,
/// clusters and wide cells are placed one by one.
fn runs_of(v: &PaneView, (cw, ch): (f32, f32)) -> Vec<Run> {
    let mut out = Vec::with_capacity(v.runs.len());
    for r in &v.runs {
        let text = v.run_text(r);
        let blank = text.trim().is_empty() && r.flags & flags::UNDERLINE == 0;
        if blank && r.bg == v.bg {
            continue;
        }
        let mut fg = color(r.fg);
        if r.flags & flags::FAINT != 0 {
            fg = fg.with_alpha(0.6);
        }
        let run = |col: u16, ncols: u16, t: &str| Run {
            x: col as f32 * cw,
            y: r.row as f32 * ch,
            w: ncols as f32 * cw,
            text: if blank { SharedString::new() } else { t.into() },
            fg,
            bg: color(r.bg),
            has_bg: r.bg != v.bg,
            bold: r.flags & flags::BOLD != 0,
            italic: r.flags & flags::ITALIC != 0,
            under: r.flags & flags::UNDERLINE != 0,
        };
        if r.flags & flags::WIDE != 0 && r.flags & flags::CLUSTER == 0 {
            for (k, c) in text.chars().enumerate() {
                out.push(run(r.col + 2 * k as u16, 2, c.encode_utf8(&mut [0; 4])));
            }
        } else {
            out.push(run(r.col, r.ncols, text));
        }
    }
    out
}

impl State {
    fn apply(&mut self, i: usize, v: &PaneView) {
        let (cw, ch) = self.cell;
        let model = &self.runs[i];
        let new = runs_of(v, self.cell);
        for (k, r) in new.iter().enumerate() {
            if k < model.row_count() {
                if model.row_data(k).as_ref() != Some(r) {
                    model.set_row_data(k, r.clone());
                }
            } else {
                model.push(r.clone());
            }
        }
        while model.row_count() > new.len() {
            model.remove(model.row_count() - 1);
        }
        let style = v.cursor_style;
        let (x, y) = (v.cursor_x as f32 * cw, v.cursor_y as f32 * ch);
        let (cx, cy, w, h) = match style {
            2 => (x, y, 2.0, ch),
            3 => (x, y + ch - 2.0, cw, 2.0),
            _ => (x, y, cw, ch),
        };
        self.panes.set_row_data(
            i,
            PaneData {
                bg: color(v.bg),
                runs: ModelRc::from(model.clone()),
                cursor_x: cx,
                cursor_y: cy,
                cursor_w: w,
                cursor_h: h,
                cursor_color: color(v.cursor_color),
                cursor_visible: v.cursor_visible,
                cursor_hollow: style == 1,
            },
        );
        self.drawn[i] = true;
        self.hit |= v.probe_hit;
    }

    fn update(&mut self) {
        for i in self.grid.take_dirty() {
            if let Some(v) = self.grid.view(i) {
                self.apply(i, &v);
            }
        }
    }

    fn rendered(&mut self, before: bool) {
        let now = Instant::now();
        if before {
            self.frame_start = now;
            return;
        }
        self.bench.frame(
            now.duration_since(self.started).as_secs_f64() * 1000.0,
            now.duration_since(self.frame_start).as_secs_f64() * 1000.0,
        );
        if std::mem::take(&mut self.hit) {
            if let Some(t) = self.typed_at.take() {
                self.bench.latency(now.duration_since(t).as_secs_f64() * 1000.0);
            }
        }
        if !self.first && self.drawn.iter().all(|d| *d) && self.grid.all_snapshotted() {
            self.first = true;
            self.bench.first_frame();
        }
    }
}

/// Slint's key text to the wire's (W3C `code`) names.
fn wire_key(text: &str) -> Option<String> {
    let mut chars = text.chars();
    let (Some(c), None) = (chars.next(), chars.next()) else {
        return None;
    };
    let named = [
        (Key::Return, "Enter"),
        (Key::Tab, "Tab"),
        (Key::Backtab, "Tab"),
        (Key::Backspace, "Backspace"),
        (Key::Escape, "Escape"),
        (Key::Delete, "Delete"),
        (Key::Home, "Home"),
        (Key::End, "End"),
        (Key::PageUp, "PageUp"),
        (Key::PageDown, "PageDown"),
        (Key::LeftArrow, "ArrowLeft"),
        (Key::RightArrow, "ArrowRight"),
        (Key::UpArrow, "ArrowUp"),
        (Key::DownArrow, "ArrowDown"),
        (Key::Space, "Space"),
    ];
    if let Some((_, n)) = named.iter().find(|(k, _)| char::from(k.clone()) == c) {
        return Some((*n).into());
    }
    let punct = match c {
        '-' => "Minus",
        '=' => "Equal",
        '[' => "BracketLeft",
        ']' => "BracketRight",
        '\\' => "Backslash",
        ';' => "Semicolon",
        '\'' => "Quote",
        ',' => "Comma",
        '.' => "Period",
        '/' => "Slash",
        '`' => "Backquote",
        _ => "",
    };
    if !punct.is_empty() {
        return Some(punct.into());
    }
    if c.is_ascii_alphabetic() {
        return Some(c.to_ascii_uppercase().to_string());
    }
    if c.is_ascii_digit() {
        return Some(format!("Digit{c}"));
    }
    let f = c as u32;
    let f1 = char::from(Key::F1) as u32;
    (f1..f1 + 24).contains(&f).then(|| format!("F{}", f - f1 + 1))
}

fn on_key(text: &str, shift: bool, alt: bool, ctrl: bool, meta: bool) -> bool {
    let printable = text.chars().next().is_some_and(|c| {
        !c.is_control() && !('\u{f700}'..='\u{f8ff}').contains(&c)
    });
    if printable && !ctrl && !alt && !meta {
        // Text goes through the hidden input, so IME composition works.
        return false;
    }
    let Some(code) = wire_key(text) else {
        return false;
    };
    let mods = (shift as u16) | (alt as u16) << 1 | (ctrl as u16) << 2 | (meta as u16) << 3;
    with(|s| {
        let pane = s.app.upgrade().map_or(0, |a| a.get_focused() as usize);
        s.grid.key(pane, &code, mods, printable.then_some(text));
    });
    true
}

fn bench_tick() {
    let Some((step, app)) = with(|s| (s.bench.tick(&s.grid), s.app.clone())) else {
        return;
    };
    let Some(app) = app.upgrade() else { return };
    let press = |text: SharedString| {
        app.window().dispatch_event(WindowEvent::KeyPressed { text: text.clone() });
        app.window().dispatch_event(WindowEvent::KeyReleased { text });
    };
    match step {
        Step::Idle => {}
        Step::Type(c) => {
            with(|s| s.typed_at = Some(Instant::now()));
            press(c.to_string().into());
        }
        Step::Enter => press(Key::Return.into()),
        Step::Done => {
            with(|s| {
                let errs: Vec<String> = s.grid.errors().iter().map(|e| format!("{e:?}")).collect();
                s.bench.cfg.shoot();
                s.bench.write("slint", s.grid.panes(), &[("errors", format!("[{}]", errs.join(",")))]);
            });
            let _ = slint::quit_event_loop();
        }
    }
}

/// AppKit fits a titled window into the screen's visible frame (above the
/// dock); the look test wants the whole 1440x900, so the window keeps the
/// size asked for, as the other prototypes' windows do.
#[cfg(target_os = "macos")]
fn full_size(app: &AppWindow, w: f64, h: f64) {
    use objc2::encode::{Encode, Encoding, RefEncode};
    use objc2::runtime::{AnyClass, AnyObject, Sel};
    use objc2::{msg_send, sel};
    use slint::winit_030::winit::raw_window_handle::{HasWindowHandle, RawWindowHandle};
    use slint::winit_030::WinitWindowAccessor;
    #[repr(C)]
    #[derive(Clone, Copy)]
    struct Rect(f64, f64, f64, f64);
    unsafe impl Encode for Rect {
        const ENCODING: Encoding = Encoding::Struct(
            "CGRect",
            &[
                Encoding::Struct("CGPoint", &[f64::ENCODING, f64::ENCODING]),
                Encoding::Struct("CGSize", &[f64::ENCODING, f64::ENCODING]),
            ],
        );
    }
    unsafe impl RefEncode for Rect {
        const ENCODING_REF: Encoding = Encoding::Pointer(&<Self as Encode>::ENCODING);
    }
    extern "C" fn keep(_: *mut AnyObject, _: Sel, r: Rect, _: *mut AnyObject) -> Rect {
        r
    }
    extern "C" {
        fn object_getClass(o: *mut AnyObject) -> *const AnyClass;
        fn class_replaceMethod(
            c: *const AnyClass,
            s: Sel,
            imp: *const std::ffi::c_void,
            types: *const std::ffi::c_char,
        ) -> *const std::ffi::c_void;
    }
    app.window().with_winit_window(|win| {
        let Ok(handle) = win.window_handle() else { return };
        let RawWindowHandle::AppKit(kit) = handle.as_raw() else { return };
        // SAFETY: the view is live and this is the main thread.
        unsafe {
            let view = kit.ns_view.as_ptr().cast::<AnyObject>();
            let win: *mut AnyObject = msg_send![view, window];
            if win.is_null() {
                return;
            }
            class_replaceMethod(
                object_getClass(win),
                sel!(constrainFrameRect:toScreen:),
                keep as *const std::ffi::c_void,
                c"{CGRect={CGPoint=dd}{CGSize=dd}}@:{CGRect={CGPoint=dd}{CGSize=dd}}@".as_ptr(),
            );
            let screen: *mut AnyObject = msg_send![win, screen];
            if screen.is_null() {
                return;
            }
            let sf: Rect = msg_send![screen, frame];
            let frame = Rect(40.0, sf.3 - 40.0 - h, w, h);
            let _: () = msg_send![win, setFrame: frame, display: true];
        }
    });
}

fn main() {
    let Some(socket) = spike_core::env::grid() else {
        eprintln!("no grid endpoint (VORN_SPIKE_GRID/VORN_SPIKE_SESSIONS)");
        std::process::exit(1);
    };
    let sessions = spike_core::env::sessions();
    let cfg = Config::from_env();
    let interactive = cfg.mode == Mode::Interactive;
    let (look, polish) = (cfg.look, cfg.polish);

    slint::BackendSelector::new()
        .with_winit_window_attributes_hook(move |attrs| {
            use slint::winit_030::winit::dpi::{LogicalPosition, LogicalSize};
            let mut attrs = attrs
                .with_inner_size(LogicalSize::new(1440.0, 900.0))
                .with_position(LogicalPosition::new(40.0, 40.0))
                .with_active(interactive);
            #[cfg(target_os = "macos")]
            if look {
                use slint::winit_030::winit::platform::macos::WindowAttributesExtMacOS;
                attrs = attrs
                    .with_titlebar_transparent(true)
                    .with_fullsize_content_view(true)
                    .with_title_hidden(true);
            }
            if polish {
                attrs = attrs.with_transparent(true).with_blur(true);
            }
            attrs
        })
        .select()
        .expect("winit backend");

    let app = AppWindow::new().expect("window");
    app.set_look(look);
    app.set_polish(polish);
    app.set_font_size(cfg.font_size);
    let n = sessions.len();
    let (cols, rows) = spike_core::layout(n);
    app.set_cols(cols as i32);
    app.set_rows(rows as i32);
    let cell = (app.get_cell_w(), app.get_cell_h());
    let (pw, ph) = if look {
        // The session card's body: 744 wide less its borders, the window
        // less the card header.
        (742.0, 900.0 - 41.0 - 3.0)
    } else {
        (
            (1440.0 - 2.0 * (cols as f32 - 1.0)) / cols as f32,
            (900.0 - 2.0 * (rows as f32 - 1.0)) / rows as f32,
        )
    };
    let start = ((pw / cell.0).floor() as u16, (ph / cell.1).floor() as u16);
    let grid = match Grid::connect(&socket, sessions, start.0, start.1) {
        Ok(g) => g,
        Err(e) => {
            eprintln!("grid: {e}");
            std::process::exit(1);
        }
    };

    let blank = PaneData { bg: color(0x1e1e1e), ..Default::default() };
    let runs: Vec<Rc<VecModel<Run>>> = (0..n).map(|_| Rc::new(VecModel::default())).collect();
    let panes = Rc::new(VecModel::from(
        runs.iter()
            .map(|r| PaneData { runs: ModelRc::from(r.clone()), ..blank.clone() })
            .collect::<Vec<_>>(),
    ));
    app.set_panes(ModelRc::from(panes.clone()));
    let mut bench = Bench::new(cfg);
    bench.period_ms = 1000.0 / 120.0;
    let now = Instant::now();
    ST.with(|s| {
        *s.borrow_mut() = Some(State {
            app: app.as_weak(),
            grid: grid.clone(),
            runs,
            panes,
            drawn: vec![false; n],
            sizes: vec![start; n],
            cell,
            bench,
            started: now,
            frame_start: now,
            typed_at: None,
            hit: false,
            first: false,
        })
    });

    app.on_resized(|i, w, h| {
        with(|s| {
            let i = i as usize;
            // Panes report 0x0 before their first layout.
            if w < s.cell.0 * 2.0 || h < s.cell.1 {
                return;
            }
            let cols = (w / s.cell.0).floor() as u16;
            let rows = (h / s.cell.1).floor() as u16;
            if s.sizes.get(i).is_some_and(|z| *z != (cols, rows)) {
                s.sizes[i] = (cols, rows);
                s.grid.resize(i, cols, rows);
            }
        });
    });
    app.on_key(|t, sh, alt, ctrl, meta| on_key(&t, sh, alt, ctrl, meta));
    app.on_text(|t| {
        with(|s| {
            let pane = s.app.upgrade().map_or(0, |a| a.get_focused() as usize);
            s.grid.text(pane, &t);
        });
    });
    app.window()
        .set_rendering_notifier(|state, _| match state {
            RenderingState::BeforeRendering => {
                with(|s| s.rendered(true));
            }
            RenderingState::AfterRendering => {
                with(|s| s.rendered(false));
            }
            _ => {}
        })
        .expect("rendering notifier");

    // The reader thread wakes the event loop; the grid coalesces wakes
    // until the changes are taken.
    grid.set_waker(|| {
        let _ = slint::invoke_from_event_loop(|| {
            with(State::update);
        });
    });

    let timer = slint::Timer::default();
    if !interactive {
        timer.start(slint::TimerMode::Repeated, Duration::from_millis(4), bench_tick);
    }
    app.show().expect("show");
    // winit creates the window once the event loop runs.
    #[cfg(target_os = "macos")]
    {
        let weak = app.as_weak();
        slint::Timer::single_shot(Duration::from_millis(50), move || {
            if let Some(app) = weak.upgrade() {
                full_size(&app, 1440.0, 900.0);
            }
        });
    }
    slint::run_event_loop().expect("event loop");
}

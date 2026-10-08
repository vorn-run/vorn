//! The GPUI prototype: one window, a grid of panes, each painted on a
//! canvas from the view the shared Rust grid client hands it (the same
//! code the other two prototypes link). GPUI shapes and caches the text
//! and draws everything on the GPU.

use std::cell::{Cell, RefCell};
use std::ops::Range;
use std::rc::Rc;
use std::sync::Arc;
use std::time::{Duration, Instant};

use futures::StreamExt;
use gpui::{
    canvas, div, fill, font, point, prelude::*, px, rgb, size, App, Bounds, Context,
    ElementInputHandler, EntityInputHandler, FocusHandle, Font, FontStyle, FontWeight, Hsla,
    KeyDownEvent, Keystroke, MouseDownEvent, Pixels, SharedString, StrikethroughStyle, TextAlign,
    TextRun, UTF16Selection, UnderlineStyle, Window, WindowBounds, WindowKind, WindowOptions,
};
use spike_core::bench::{Bench, Config, Mode, Step};
use spike_core::view::flags;
use spike_core::{Grid, PaneView};

const FONT_SIZE: f32 = 12.0;
const GAP: f32 = 2.0;

struct Metrics {
    cell: gpui::Size<Pixels>,
    ascent: Pixels,
    font: Font,
}

struct Root {
    grid: Arc<Grid>,
    views: Vec<Option<Rc<PaneView>>>,
    sizes: Vec<Rc<Cell<(u16, u16)>>>,
    focus: Vec<FocusHandle>,
    focused: usize,
    marked: String,
    metrics: Rc<Metrics>,
    bench: Rc<RefCell<Bench>>,
    started: Instant,
    typed_at: Rc<Cell<Option<Instant>>>,
    first_frame: Rc<Cell<bool>>,
    hit: Rc<Cell<bool>>,
    frame_start: Instant,
}

fn color(c: u32) -> Hsla {
    rgb(c).into()
}

/// GPUI's key names to the wire's (W3C `code`) names.
fn wire_key(key: &str) -> Option<String> {
    let named = match key {
        "enter" => "Enter",
        "tab" => "Tab",
        "space" => "Space",
        "backspace" => "Backspace",
        "escape" => "Escape",
        "delete" => "Delete",
        "home" => "Home",
        "end" => "End",
        "pageup" => "PageUp",
        "pagedown" => "PageDown",
        "left" => "ArrowLeft",
        "right" => "ArrowRight",
        "up" => "ArrowUp",
        "down" => "ArrowDown",
        "-" => "Minus",
        "=" => "Equal",
        "[" => "BracketLeft",
        "]" => "BracketRight",
        "\\" => "Backslash",
        ";" => "Semicolon",
        "'" => "Quote",
        "," => "Comma",
        "." => "Period",
        "/" => "Slash",
        "`" => "Backquote",
        _ => "",
    };
    if !named.is_empty() {
        return Some(named.into());
    }
    let mut chars = key.chars();
    match (chars.next(), chars.next()) {
        (Some(c), None) if c.is_ascii_alphabetic() => Some(c.to_ascii_uppercase().to_string()),
        (Some(c), None) if c.is_ascii_digit() => Some(format!("Digit{c}")),
        _ if key.starts_with('f') && key[1..].parse::<u8>().is_ok() => Some(key.to_uppercase()),
        _ => None,
    }
}

impl Root {
    fn new(grid: Arc<Grid>, metrics: Metrics, window: &mut Window, cx: &mut Context<Self>) -> Self {
        let n = grid.panes();
        let focus: Vec<FocusHandle> = (0..n).map(|_| cx.focus_handle()).collect();
        window.focus(&focus[0], cx);
        let bench = Rc::new(RefCell::new(Bench::new(Config::from_env())));
        bench.borrow_mut().period_ms = 1000.0 / 120.0;

        // The reader thread wakes the window through a channel.
        let (tx, mut rx) = futures::channel::mpsc::unbounded::<()>();
        grid.set_waker(move || {
            let _ = tx.unbounded_send(());
        });
        cx.spawn_in(window, async move |this, cx| {
            while rx.next().await.is_some() {
                if this.update(cx, |_, cx| cx.notify()).is_err() {
                    break;
                }
            }
        })
        .detach();

        if bench.borrow().cfg.mode != Mode::Interactive {
            cx.spawn_in(window, async move |this, cx| loop {
                cx.background_executor().timer(Duration::from_millis(4)).await;
                let Ok(key) = this.update(cx, |this, cx| this.bench_tick(cx)) else {
                    break;
                };
                match key {
                    Some(Some(k)) => {
                        let _ = cx.update(|window, cx| window.dispatch_keystroke(k, cx));
                    }
                    Some(None) => {}
                    None => break,
                }
            })
            .detach();
        }

        Root {
            views: vec![None; n],
            sizes: (0..n).map(|_| Rc::new(Cell::new((0, 0)))).collect(),
            focus,
            focused: 0,
            marked: String::new(),
            metrics: Rc::new(metrics),
            bench,
            started: Instant::now(),
            typed_at: Rc::new(Cell::new(None)),
            first_frame: Rc::new(Cell::new(false)),
            hit: Rc::new(Cell::new(false)),
            frame_start: Instant::now(),
            grid,
        }
    }

    /// Returns the keystroke to inject, or None when the bench is over.
    fn bench_tick(&mut self, cx: &mut Context<Self>) -> Option<Option<Keystroke>> {
        let step = self.bench.borrow_mut().tick(&self.grid);
        match step {
            Step::Idle => Some(None),
            Step::Type(c) => {
                self.typed_at.set(Some(Instant::now()));
                Some(Keystroke::parse(&c.to_string()).ok())
            }
            Step::Enter => Some(Keystroke::parse("enter").ok()),
            Step::Done => {
                let errs: Vec<String> =
                    self.grid.errors().iter().map(|e| format!("{e:?}")).collect();
                self.bench.borrow().write(
                    "gpui",
                    self.grid.panes(),
                    &[("errors", format!("[{}]", errs.join(",")))],
                );
                cx.quit();
                None
            }
        }
    }

    fn on_key(&mut self, pane: usize, ev: &KeyDownEvent, cx: &mut Context<Self>) {
        let k = &ev.keystroke;
        let m = &k.modifiers;
        let mods = (m.shift as u16) | (m.alt as u16) << 1 | (m.control as u16) << 2 | (m.platform as u16) << 3;
        let plain_text = k.key_char.is_some() && !m.control && !m.alt && !m.platform && k.key != "enter"
            && k.key != "tab" && k.key != "escape" && k.key != "backspace";
        if plain_text || !self.marked.is_empty() {
            // Text goes through the input handler, so IME composition works.
            return;
        }
        if let Some(code) = wire_key(&k.key) {
            self.grid.key(pane, &code, mods, k.key_char.as_deref());
            cx.stop_propagation();
        }
    }

    fn pane(&self, i: usize, cx: &mut Context<Self>) -> impl IntoElement {
        let view = self.views[i].clone();
        let metrics = self.metrics.clone();
        let size = self.sizes[i].clone();
        let grid = self.grid.clone();
        let focus = self.focus[i].clone();
        let entity = cx.entity();
        let focused = self.focused == i;
        let marked = if focused { self.marked.clone() } else { String::new() };
        let hit = self.hit.clone();
        div()
            .flex_1()
            .h_full()
            .overflow_hidden()
            .track_focus(&self.focus[i])
            .on_key_down(cx.listener(move |this, ev: &KeyDownEvent, _, cx| this.on_key(i, ev, cx)))
            .on_mouse_down(
                gpui::MouseButton::Left,
                cx.listener(move |this, _: &MouseDownEvent, window, cx| {
                    this.focused = i;
                    window.focus(&this.focus[i], cx);
                    cx.notify();
                }),
            )
            .child(
                canvas(
                    move |bounds, _, _| {
                        let cols = ((bounds.size.width / metrics.cell.width).floor() as u16).max(2);
                        let rows = ((bounds.size.height / metrics.cell.height).floor() as u16).max(1);
                        if size.get() != (cols, rows) {
                            size.set((cols, rows));
                            grid.resize(i, cols, rows);
                        }
                        (view, metrics)
                    },
                    move |bounds, (view, m), window, cx| {
                        window.handle_input(&focus, ElementInputHandler::new(bounds, entity), cx);
                        paint_pane(bounds, view.as_deref(), &m, focused, &marked, window, cx);
                        if view.is_some_and(|v| v.probe_hit) {
                            hit.set(true);
                        }
                    },
                )
                .size_full(),
            )
    }
}

fn paint_pane(
    bounds: Bounds<Pixels>,
    view: Option<&PaneView>,
    m: &Metrics,
    focused: bool,
    marked: &str,
    window: &mut Window,
    cx: &mut App,
) {
    let Some(v) = view else {
        window.paint_quad(fill(bounds, color(0x1e1e1e)));
        return;
    };
    window.paint_quad(fill(bounds, color(v.bg)));
    let (cw, ch) = (m.cell.width, m.cell.height);
    let origin = bounds.origin;
    let at = |col: u16, row: u16| point(origin.x + cw * col as f32, origin.y + ch * row as f32);
    for r in &v.runs {
        if r.bg != v.bg {
            window.paint_quad(fill(
                Bounds::new(at(r.col, r.row), size(cw * r.ncols as f32, ch)),
                color(r.bg),
            ));
        }
    }
    let ts = window.text_system().clone();
    for r in &v.runs {
        let text = v.run_text(r);
        if text.trim().is_empty() && r.flags & (flags::UNDERLINE | flags::STRIKE) == 0 {
            continue;
        }
        let mut f = m.font.clone();
        if r.flags & flags::BOLD != 0 {
            f.weight = FontWeight::BOLD;
        }
        if r.flags & flags::ITALIC != 0 {
            f.style = FontStyle::Italic;
        }
        let mut fg = color(r.fg);
        if r.flags & flags::FAINT != 0 {
            fg.a = 0.6;
        }
        let run = TextRun {
            len: text.len(),
            font: f,
            color: fg,
            background_color: None,
            underline: (r.flags & flags::UNDERLINE != 0).then(|| UnderlineStyle {
                thickness: px(1.0),
                color: Some(fg),
                wavy: false,
            }),
            strikethrough: (r.flags & flags::STRIKE != 0).then(|| StrikethroughStyle {
                thickness: px(1.0),
                color: Some(fg),
            }),
        };
        // Plain runs keep the grid's advance; clusters are drawn alone.
        let force = (r.flags & flags::CLUSTER == 0).then_some(cw);
        let line = ts.shape_line(SharedString::from(text.to_owned()), px(FONT_SIZE), &[run], force);
        let _ = line.paint(at(r.col, r.row), ch, TextAlign::Left, None, window, cx);
    }
    let c = at(v.cursor_x, v.cursor_y);
    if !marked.is_empty() {
        let run = TextRun {
            len: marked.len(),
            font: m.font.clone(),
            color: color(v.fg),
            background_color: Some(color(v.bg)),
            underline: Some(UnderlineStyle { thickness: px(1.0), color: Some(color(v.fg)), wavy: false }),
            strikethrough: None,
        };
        let line = ts.shape_line(SharedString::from(marked.to_owned()), px(FONT_SIZE), &[run], None);
        let _ = line.paint(c, ch, TextAlign::Left, None, window, cx);
    } else if v.cursor_visible {
        let mut cc = color(v.cursor_color);
        cc.a = 0.75;
        let b = match v.cursor_style {
            2 => Bounds::new(c, size(px(2.0), ch)),
            3 => Bounds::new(point(c.x, c.y + ch - px(2.0)), size(cw, px(2.0))),
            _ => Bounds::new(c, size(cw, ch)),
        };
        if focused || v.cursor_style >= 2 {
            window.paint_quad(fill(b, cc));
        } else {
            window.paint_quad(gpui::outline(b, cc, gpui::BorderStyle::Solid));
        }
    }
}

impl Render for Root {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        self.frame_start = Instant::now();
        for i in self.grid.take_dirty() {
            if let Some(v) = self.grid.view(i) {
                self.views[i] = Some(Rc::new(v));
            }
        }
        let n = self.views.len();
        let (cols, rows) = spike_core::layout(n);
        let mut column = div().size_full().flex().flex_col().gap(px(GAP)).bg(rgb(0x404040));
        for r in 0..rows {
            let mut row = div().flex_1().w_full().flex().flex_row().gap(px(GAP));
            for c in 0..cols {
                let i = r * cols + c;
                row = if i < n {
                    row.child(self.pane(i, cx))
                } else {
                    row.child(div().flex_1())
                };
            }
            column = column.child(row);
        }
        // Painted last: the frame's work is done here.
        let bench = self.bench.clone();
        let (started, frame_start) = (self.started, self.frame_start);
        let typed_at = self.typed_at.clone();
        let first = self.first_frame.clone();
        let hit = self.hit.clone();
        let all_drawn = self.views.iter().all(Option::is_some) && self.grid.all_snapshotted();
        column.child(
            canvas(|_, _, _| (), move |_, _, _, _| {
                let now = Instant::now();
                let mut b = bench.borrow_mut();
                b.frame(
                    now.duration_since(started).as_secs_f64() * 1000.0,
                    now.duration_since(frame_start).as_secs_f64() * 1000.0,
                );
                if hit.replace(false) {
                    if let Some(t) = typed_at.take() {
                        b.latency(now.duration_since(t).as_secs_f64() * 1000.0);
                    }
                }
                if all_drawn && !first.get() {
                    first.set(true);
                    b.first_frame();
                }
            })
            .absolute()
            .size_0(),
        )
    }
}

impl EntityInputHandler for Root {
    fn text_for_range(&mut self, _: Range<usize>, _: &mut Option<Range<usize>>, _: &mut Window, _: &mut Context<Self>) -> Option<String> {
        None
    }
    fn selected_text_range(&mut self, _: bool, _: &mut Window, _: &mut Context<Self>) -> Option<UTF16Selection> {
        Some(UTF16Selection { range: 0..0, reversed: false })
    }
    fn marked_text_range(&self, _: &mut Window, _: &mut Context<Self>) -> Option<Range<usize>> {
        (!self.marked.is_empty()).then(|| 0..self.marked.encode_utf16().count())
    }
    fn unmark_text(&mut self, _: &mut Window, cx: &mut Context<Self>) {
        self.marked.clear();
        cx.notify();
    }
    fn replace_text_in_range(&mut self, _: Option<Range<usize>>, text: &str, _: &mut Window, cx: &mut Context<Self>) {
        self.marked.clear();
        self.grid.text(self.focused, text);
        cx.notify();
    }
    fn replace_and_mark_text_in_range(&mut self, _: Option<Range<usize>>, text: &str, _: Option<Range<usize>>, _: &mut Window, cx: &mut Context<Self>) {
        self.marked = text.to_owned();
        cx.notify();
    }
    fn bounds_for_range(&mut self, _: Range<usize>, element: Bounds<Pixels>, _: &mut Window, _: &mut Context<Self>) -> Option<Bounds<Pixels>> {
        let v = self.views.get(self.focused)?.as_ref()?;
        let m = &self.metrics;
        Some(Bounds::new(
            point(element.origin.x + m.cell.width * v.cursor_x as f32, element.origin.y + m.cell.height * v.cursor_y as f32),
            m.cell,
        ))
    }
    fn character_index_for_point(&mut self, _: gpui::Point<Pixels>, _: &mut Window, _: &mut Context<Self>) -> Option<usize> {
        None
    }
}

fn main() {
    let Some(socket) = spike_core::env::grid() else {
        eprintln!("no grid endpoint (VORN_SPIKE_GRID/VORN_SPIKE_SESSIONS)");
        std::process::exit(1);
    };
    let sessions = spike_core::env::sessions();
    let interactive = Config::from_env().mode == Mode::Interactive;
    gpui_platform::application().run(move |cx: &mut App| {
        let f = font("Menlo");
        let ts = cx.text_system().clone();
        let id = ts.resolve_font(&f);
        let cw = ts.advance(id, px(FONT_SIZE), 'M').map(|a| a.width).unwrap_or(px(7.2));
        let ascent = ts.ascent(id, px(FONT_SIZE));
        let descent = ts.descent(id, px(FONT_SIZE));
        let ch = (ascent.ceil() + descent.abs().ceil()).max(px(FONT_SIZE));
        let metrics = Metrics { cell: size(cw, ch), ascent, font: f };

        let win = size(px(1440.0), px(900.0));
        let (cols, rows) = spike_core::layout(sessions.len());
        let pw = (win.width - px(GAP) * (cols as f32 - 1.0)) / cols as f32;
        let ph = (win.height - px(GAP) * (rows as f32 - 1.0)) / rows as f32;
        let grid = match Grid::connect(
            &socket,
            sessions.clone(),
            (pw / cw).floor() as u16,
            (ph / ch).floor() as u16,
        ) {
            Ok(g) => g,
            Err(e) => {
                eprintln!("grid: {e}");
                std::process::exit(1);
            }
        };
        let bounds = Bounds::new(point(px(40.0), px(40.0)), win);
        cx.open_window(
            WindowOptions {
                window_bounds: Some(WindowBounds::Windowed(bounds)),
                focus: interactive,
                kind: if interactive { WindowKind::Normal } else { WindowKind::PopUp },
                // The bench window is not key; it must not be throttled.
                inactive_frame_interval: None,
                titlebar: Some(gpui::TitlebarOptions {
                    title: Some("Vorn spike: GPUI".into()),
                    ..Default::default()
                }),
                ..Default::default()
            },
            |window, cx| cx.new(|cx| Root::new(grid, metrics, window, cx)),
        )
        .expect("window");
        if interactive {
            cx.activate(true);
        }
    });
}

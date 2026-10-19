//! Frame times and bytes a frame for the pixel terminal: a sidebar and a
//! terminal card full of text, drawn by the CPU renderer and turned into
//! kitty images, as typing, scrolling and full repaints change it.
//!
//! `cargo run --release --example pixel_frames`

use std::time::{Duration, Instant};

use vornui::backend::{Backend, Cells};
use vornui::theme::color;
use vornui::tty::kitty::Wrap;
use vornui::tty::present::{Placing, Sent, Transport};
use vornui::tty::PixelFrames;
use vornui::{custom, div, text, El, Rect, RenderMode, Rgba, Ui, UiConfig};

const FRAMES: usize = 120;

#[derive(Clone, Copy)]
struct State {
    typed: usize,
    scroll: usize,
    clear: Rgba,
}

fn scene(size: (f32, f32), s: State) -> El {
    let mut side = div()
        .col()
        .w(220.0)
        .h(size.1)
        .bg(color::SURFACE_SUNKEN)
        .gap(6.0);
    for i in 0..30 {
        side = side.child(text(format!("Session {i}"), 13.0, color::INK));
    }
    let card = div()
        .flex(1.0)
        .h(size.1 - 16.0)
        .rounded(10.0)
        .bg(Rgba::hex(0x101014))
        .child(custom(0).flex(1.0).h(size.1 - 16.0));
    div()
        .row()
        .w(size.0)
        .h(size.1)
        .bg(s.clear)
        .gap(8.0)
        .child(side)
        .child(card)
}

/// A shell's screen: lines of text that scroll by `s.scroll` and a prompt
/// line `s.typed` characters long.
fn paint_card(ui: &mut Ui, r: Rect, s: State) {
    let cell = ui.text.cell();
    let cols = (r.w / cell.w) as usize;
    let lines = (r.h / cell.h) as usize;
    let fg = Rgba::hex(0xd4d4d8);
    let mut buf = [0u8; 4];
    for y in 0..lines {
        let last = y + 1 == lines;
        let n = if last {
            s.typed.min(cols)
        } else {
            cols * 3 / 4
        };
        for x in 0..n {
            let seed = x * 7 + (y + s.scroll) * 13;
            let ch = if last {
                'a'
            } else {
                char::from(b'!' + (seed % 90) as u8)
            };
            let at = (r.x + x as f32 * cell.w, r.y + y as f32 * cell.h);
            ui.text.draw_cell(
                &mut ui.scene,
                &mut ui.atlases,
                ch.encode_utf8(&mut buf),
                0,
                at,
                fg,
            );
        }
    }
}

struct Run {
    times: Vec<Duration>,
    sent: Vec<Sent>,
}

fn run(
    cells: (u32, u32),
    cell: (u32, u32),
    transport: &Transport,
    step: fn(usize, State) -> State,
) -> Run {
    let px = (cells.0 * cell.0, cells.1 * cell.1);
    let scale = ((cell.1 as f32 / 17.0 * 4.0).round() / 4.0).clamp(1.0, 3.0);
    let size = (px.0 as f32 / scale, px.1 as f32 / scale);
    let mut ui = Ui::headless(RenderMode::Cpu, &UiConfig::system(scale, 13.0));
    let mut frames = PixelFrames::new(Wrap::Direct, Placing::Cursor, cell, transport.clone());
    let mut out = Vec::with_capacity(1 << 20);
    let mut s = State {
        typed: 10,
        scroll: 0,
        clear: color::SURFACE_BASE,
    };
    let mut frame = |ui: &mut Ui, s: State, out: &mut Vec<u8>| {
        ui.begin(color::SURFACE_BASE);
        let laid = ui.layout(scene(size, s), size);
        for &(_, r) in &laid.customs {
            paint_card(ui, r, s);
        }
        out.clear();
        frames.draw(ui, px, out).expect("atlas room")
    };
    frame(&mut ui, s, &mut out);
    let mut r = Run {
        times: Vec::new(),
        sent: Vec::new(),
    };
    for i in 0..FRAMES {
        s = step(i, s);
        let t = Instant::now();
        let sent = frame(&mut ui, s, &mut out);
        r.times.push(t.elapsed());
        r.sent.push(sent);
    }
    r
}

/// Today's text drawing of the same typing, for comparison: layout and
/// the cell grid (the escapes it writes are left out).
fn text_typing(cells: (u16, u16)) -> Vec<Duration> {
    let size = vornui::cells::units::px(cells.0, cells.1);
    let mut ui = Ui::cells(&UiConfig::system(1.0, 13.0));
    let mut out = Cells::offscreen(cells.0, cells.1);
    let mut s = State {
        typed: 10,
        scroll: 0,
        clear: color::SURFACE_BASE,
    };
    let mut times = Vec::new();
    for i in 0..=FRAMES {
        s.typed = 10 + i % 60;
        let t = Instant::now();
        ui.begin(color::SURFACE_BASE);
        let laid = ui.layout(scene(size, s), size);
        for &(_, r) in &laid.customs {
            paint_card(&mut ui, r, s);
        }
        out.present(&mut ui).expect("cells ui");
        if i > 0 {
            times.push(t.elapsed());
        }
    }
    times
}

fn pct(v: &mut [Duration], p: f64) -> f64 {
    v.sort();
    v[((v.len() - 1) as f64 * p).round() as usize].as_secs_f64() * 1e3
}

fn main() {
    let typing = |i: usize, s: State| State {
        typed: 10 + i % 60,
        ..s
    };
    let scrolling = |_: usize, s: State| State {
        scroll: s.scroll + 1,
        ..s
    };
    let repaint = |i: usize, s: State| State {
        clear: if i.is_multiple_of(2) {
            color::SURFACE_SUNKEN
        } else {
            color::SURFACE_BASE
        },
        ..s
    };
    let idle = |_: usize, s: State| s;
    let dir = std::env::temp_dir().join(format!("vorn-pixel-frames-{}", std::process::id()));
    std::fs::create_dir_all(&dir).expect("temp dir");
    let transports = [
        (
            "local, files",
            Transport {
                file_dir: Some(dir.clone()),
                zlib: false,
            },
        ),
        (
            "ssh, inline+zlib",
            Transport {
                file_dir: None,
                zlib: true,
            },
        ),
        ("inline, raw", Transport::default()),
    ];
    let screens = [
        ("120x40 at 16x32 (1920x1280)", (120, 40), (16, 32)),
        (
            "Retina full screen 216x63 at 16x34 (3456x2142)",
            (216, 63),
            (16, 34),
        ),
    ];
    type Step = fn(usize, State) -> State;
    let steps: [(&str, Step); 4] = [
        ("typing", typing),
        ("scrolling", scrolling),
        ("full repaint", repaint),
        ("idle", idle),
    ];
    for (screen, cells, cell) in screens {
        println!("\n## {screen}");
        let mut t = text_typing((cells.0 as u16, cells.1 as u16));
        println!(
            "text cells, typing: p50 {:.2} ms, p95 {:.2} ms\n",
            pct(&mut t, 0.5),
            pct(&mut t, 0.95)
        );
        println!("| transport | change | p50 ms | p95 ms | bytes/frame (median) | file bytes/frame | images/frame | MB/s at p50 |");
        println!("|---|---|---|---|---|---|---|---|");
        for (tname, t) in &transports {
            for (sname, step) in &steps {
                let mut r = run(cells, cell, t, *step);
                let (p50, p95) = (pct(&mut r.times, 0.5), pct(&mut r.times, 0.95));
                let mut bytes: Vec<usize> = r.sent.iter().map(|s| s.bytes).collect();
                bytes.sort_unstable();
                let mid = bytes[bytes.len() / 2];
                let files = r.sent.iter().map(|s| s.file_bytes).sum::<usize>() / FRAMES;
                let images = r.sent.iter().map(|s| s.images).sum::<usize>() as f64 / FRAMES as f64;
                let rate = if p50 > 0.0 {
                    mid as f64 / (p50 / 1e3) / 1e6
                } else {
                    0.0
                };
                println!(
                    "| {tname} | {sname} | {p50:.2} | {p95:.2} | {mid} | {files} | {images:.1} | {rate:.1} |"
                );
            }
        }
    }
    let _ = std::fs::remove_dir_all(&dir);
}

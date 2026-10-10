//! The CPU renderer on the 8-pane terminal grid at 2x: a frame where every
//! pixel changes, one where a single pane changes, and one where nothing does.

use criterion::{criterion_group, criterion_main, Criterion};
use vornui::theme::color;
use vornui::{custom, div, El, Rect, RenderMode, Rgba, Ui, UiConfig};

const SIZE: (f32, f32) = (1440.0, 900.0);
const PANES: u64 = 8;

fn grid() -> El {
    let mut rows = div().col().w(SIZE.0).h(SIZE.1).gap(1.0);
    for r in 0..2 {
        let mut row = div().row().flex(1.0).gap(1.0);
        for c in 0..PANES / 2 {
            row = row.child(custom(r * (PANES / 2) + c).flex(1.0));
        }
        rows = rows.child(row);
    }
    rows
}

/// Fills a pane with cells; `tick` changes pane 0's text.
fn paint_pane(ui: &mut Ui, id: u64, r: Rect, tick: u32) {
    let cell = ui.text.cell();
    let cols = (r.w / cell.w) as u32;
    let lines = (r.h / cell.h) as u32;
    let fg = Rgba::hex(0xd4d4d8);
    let mut buf = [0u8; 4];
    for y in 0..lines {
        for x in 0..cols {
            let seed = x * 7 + y * 13 + if id == 0 { tick } else { 0 };
            let ch = char::from(b'!' + (seed % 90) as u8);
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

fn frame(ui: &mut Ui, clear: Rgba, tick: u32) {
    ui.begin(clear);
    let laid = ui.layout(grid(), SIZE);
    for &(id, r) in &laid.customs {
        paint_pane(ui, id, r, tick);
    }
}

fn bench(c: &mut Criterion) {
    let mut ui = Ui::headless(RenderMode::Cpu, &UiConfig::system(2.0, 13.0));
    let target = ui.offscreen(((SIZE.0 * 2.0) as u32, (SIZE.1 * 2.0) as u32));
    frame(&mut ui, color::SURFACE_BASE, 0);
    ui.render_offscreen(&target);
    let mut g = c.benchmark_group("cpu raster, 8 panes at 2x");
    g.sample_size(30);
    let mut tick = 0u32;
    g.bench_function("every pixel changes", |b| {
        b.iter(|| {
            tick += 1;
            let clear = if tick.is_multiple_of(2) {
                color::SURFACE_BASE
            } else {
                color::SURFACE_SUNKEN
            };
            frame(&mut ui, clear, tick);
            ui.render_offscreen(&target)
        })
    });
    g.bench_function("one pane changes", |b| {
        b.iter(|| {
            tick += 1;
            frame(&mut ui, color::SURFACE_BASE, tick);
            ui.render_offscreen(&target)
        })
    });
    g.bench_function("nothing changes", |b| {
        b.iter(|| {
            frame(&mut ui, color::SURFACE_BASE, tick);
            ui.render_offscreen(&target)
        })
    });
    g.finish();
}

criterion_group!(benches, bench);
criterion_main!(benches);

//! Frame time with a menu over the 8-pane terminal grid at 2x, Solid
//! against Native materials: Solid must cost what it did before materials,
//! and Native adds the backdrop blur under the menu.

use criterion::{criterion_group, criterion_main, Criterion};
use vorn_theme::Materials;
use vornui::palette::{self, Palette};
use vornui::theme::color;
use vornui::{custom, div, text, El, Place, Rect, RenderMode, Ui, UiConfig};

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
    let mut menu = div().col().w(220.0).py(4.0).rounded(8.0).floating();
    for item in [
        "New Session",
        "Split Right",
        "Split Down",
        "Rename",
        "Close",
    ] {
        menu = menu.child(div().px(12.0).py(6.0).child(text(item, 13.0, color::INK)));
    }
    rows.child(
        div()
            .absolute(600.0, 300.0)
            .size(0.0, 0.0)
            .overlay(menu, Place::BelowStart, 0.0),
    )
}

/// Fills a pane with cells; `tick` changes pane 0's text.
fn paint_pane(ui: &mut Ui, id: u64, r: Rect, tick: u32) {
    let cell = ui.text.cell();
    let cols = (r.w / cell.w) as u32;
    let lines = (r.h / cell.h) as u32;
    let fg = color::GRAY_300.rgba();
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

fn frame(ui: &mut Ui, tick: u32) {
    ui.begin(color::SURFACE_BASE);
    let laid = ui.layout(grid(), SIZE);
    for &(id, r) in &laid.customs {
        paint_pane(ui, id, r, tick);
    }
}

fn bench(c: &mut Criterion) {
    for mode in [RenderMode::Gpu, RenderMode::Cpu] {
        let mut ui = Ui::headless(mode, &UiConfig::system(2.0, 13.0));
        if mode == RenderMode::Gpu && ui.is_cpu() {
            eprintln!("no GPU adapter; the GPU group is skipped");
            continue;
        }
        let target = ui.offscreen(((SIZE.0 * 2.0) as u32, (SIZE.1 * 2.0) as u32));
        let mut g = c.benchmark_group(format!("menu over 8 panes at 2x, {mode:?}"));
        g.sample_size(30);
        let mut tick = 0u32;
        for materials in [Materials::Solid, Materials::Native] {
            palette::install(Palette::new(vorn_theme::vorn_dark().clone(), materials));
            frame(&mut ui, tick);
            ui.render_offscreen(&target);
            g.bench_function(materials.key(), |b| {
                b.iter(|| {
                    tick += 1;
                    frame(&mut ui, tick);
                    ui.render_offscreen(&target)
                })
            });
        }
        g.finish();
    }
    palette::install(Palette::default());
}

criterion_group!(benches, bench);
criterion_main!(benches);

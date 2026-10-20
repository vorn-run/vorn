//! Every bundled theme draws, on the window renderer and in a terminal,
//! with its own surfaces and text that reads: ink on the base surface and
//! on a panel at 4.5:1 or more, as the theme's tokens promise.
//!
//! `VORN_THEME_PNG_DIR` saves each window frame as a PNG.

use std::path::PathBuf;

use vorn_theme::{contrast, Materials, Rgba8, Theme, Token};
use vornui::palette::{self, Palette};
use vornui::theme::color;
use vornui::{div, text, El, RenderMode, Ui, UiConfig};

const SIZE: (u32, u32) = (320, 160);
const WORDS: &str = "The quick brown fox";

/// The base surface, a panel and a floating menu, each with a line of ink.
fn scene(size: (f32, f32)) -> El {
    let line = || text(WORDS, 13.0, color::INK);
    div()
        .w(size.0)
        .h(size.1)
        .row()
        .bg(color::SURFACE_BASE)
        .child(
            div()
                .w(size.0 / 2.0)
                .h_full()
                .p(16.0)
                .bg(color::SURFACE_PANEL)
                .child(line()),
        )
        .child(
            div()
                .col()
                .p(16.0)
                .gap(16.0)
                .child(line())
                .child(div().p(8.0).rounded(4.0).floating().child(line())),
        )
}

fn rgb(p: &[u8]) -> Rgba8 {
    Rgba8::rgb(u32::from(p[0]) << 16 | u32::from(p[1]) << 8 | u32::from(p[2]))
}

/// The pixel of the frame furthest in contrast from `bg` within `(x, y, w, h)`.
fn strongest(rgba: &[u8], (x, y, w, h): (u32, u32, u32, u32), bg: Rgba8) -> f32 {
    let mut best = 1.0f32;
    for row in y..y + h {
        for col in x..x + w {
            let i = ((row * SIZE.0 + col) * 4) as usize;
            best = best.max(contrast::ratio(rgb(&rgba[i..i + 4]), bg));
        }
    }
    best
}

fn save(theme: &Theme, rgba: &[u8]) {
    let Some(dir) = std::env::var_os("VORN_THEME_PNG_DIR") else {
        return;
    };
    let path = PathBuf::from(dir).join(format!("{}.png", theme.id));
    let file = std::io::BufWriter::new(std::fs::File::create(path).unwrap());
    let mut enc = png::Encoder::new(file, SIZE.0, SIZE.1);
    enc.set_color(png::ColorType::Rgba);
    enc.set_depth(png::BitDepth::Eight);
    enc.write_header().unwrap().write_image_data(rgba).unwrap();
}

#[test]
fn each_bundled_theme_draws_its_surfaces_and_readable_ink_in_a_window() {
    for theme in vorn_theme::bundled() {
        palette::install(Palette::new(theme.clone(), Materials::Solid));
        let mut ui = Ui::headless(RenderMode::Cpu, &UiConfig::system(1.0, 13.0));
        let target = ui.offscreen(SIZE);
        let logical = (SIZE.0 as f32, SIZE.1 as f32);
        loop {
            ui.begin(color::SURFACE_BASE);
            ui.layout(scene(logical), logical);
            if ui.render_offscreen(&target) {
                break;
            }
        }
        let rgba = ui.read(&target).expect("cpu frame");
        save(theme, &rgba);
        let at = |x: u32, y: u32| {
            let i = ((y * SIZE.0 + x) * 4) as usize;
            rgb(&rgba[i..i + 4])
        };
        let (base, panel) = (theme.get(Token::SurfaceBase), theme.get(Token::SurfacePanel));
        assert_eq!(at(SIZE.0 - 2, SIZE.1 - 2), base, "{}: base", theme.id);
        assert_eq!(at(2, SIZE.1 - 2), panel, "{}: panel", theme.id);
        let on_panel = strongest(&rgba, (16, 16, 140, 16), panel);
        let on_base = strongest(&rgba, (176, 16, 140, 16), base);
        assert!(on_panel >= 4.5, "{}: ink on panel {on_panel:.2}", theme.id);
        assert!(on_base >= 4.5, "{}: ink on base {on_base:.2}", theme.id);
    }
    palette::install(Palette::default());
}

#[test]
fn each_bundled_theme_reads_in_a_terminal() {
    for theme in vorn_theme::bundled() {
        palette::install(Palette::new(theme.clone(), Materials::Native));
        let mut ui = Ui::cells(&UiConfig::system(1.0, 13.0));
        let (cols, rows) = (60u16, 8u16);
        let size = vornui::cells::units::px(cols, rows);
        ui.begin(color::SURFACE_BASE);
        ui.layout(scene(size), size);
        let mut buf = vornui::cells::Buffer::new(cols, rows);
        buf.draw(ui.paint_list().expect("cells"));
        let mut seen = 0;
        for row in 0..rows {
            for col in 0..cols {
                let cell = buf.get(col, row).unwrap();
                if !cell.sym.as_str().chars().any(char::is_alphanumeric) {
                    continue;
                }
                let px = |c: [u8; 3]| {
                    Rgba8::rgb(u32::from(c[0]) << 16 | u32::from(c[1]) << 8 | u32::from(c[2]))
                };
                let r = contrast::ratio(px(cell.fg), px(cell.bg));
                assert!(r >= 4.5, "{}: {col},{row} reads at {r:.2}", theme.id);
                seen += 1;
            }
        }
        assert!(seen >= 3 * 10, "{}: the three lines show ({seen} cells)", theme.id);
        let corner = buf.get(cols - 1, rows - 1).unwrap().bg;
        let base = theme.get(Token::SurfaceBase);
        assert_eq!(
            corner,
            [base.r, base.g, base.b],
            "{}: the terminal fills with the theme's base",
            theme.id
        );
    }
    palette::install(Palette::default());
}

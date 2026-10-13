//! The GPU and CPU renderers draw the same frame. Edges are antialiased by
//! different rasterizers, so the check is per pixel within a tolerance and
//! a small share of edge pixels, not byte equality.

use vornui::theme::{self, color};
use vornui::widgets::{self, Choice, PickerLook, Pill, Tab};
use vornui::{div, text, El, Id, RenderMode, Rgba, Ui, UiConfig};

const SIZE: (u32, u32) = (640, 400);

fn frame(ui: &Ui) -> El {
    let pills = [Pill::new("Terminals", None), Pill::new("Tasks", None)];
    let choices = [
        Choice {
            label: "Opus",
            hint: None,
        },
        Choice {
            label: "Sonnet",
            hint: None,
        },
    ];
    let tabs = [
        Tab {
            label: "zsh",
            closable: false,
        },
        Tab {
            label: "build",
            closable: true,
        },
    ];
    div()
        .w(SIZE.0 as f32)
        .h(SIZE.1 as f32)
        .col()
        .p(16.0)
        .gap(12.0)
        .child(widgets::tab_strip(ui, Id::new("tabs"), &tabs, 0))
        .child(widgets::pills(ui, Id::new("view"), "View", &pills, 1))
        .child(
            div()
                .row()
                .gap(8.0)
                .child(widgets::button(ui, Id::new("go"), "Run"))
                .child(widgets::icon_button(
                    ui,
                    Id::new("x"),
                    widgets::icons::X,
                    14.0,
                    "Close",
                ))
                .child(widgets::picker(
                    ui,
                    Id::new("m"),
                    "Model",
                    PickerLook::Chip,
                    &choices,
                    0,
                )),
        )
        .child(widgets::text_input(
            ui,
            Id::new("q"),
            "Search",
            "Search sessions",
        ))
}

fn draw(mode: RenderMode, scale: f32, frame: fn(&Ui) -> El) -> Option<Vec<u8>> {
    let mut ui = Ui::headless(mode, &UiConfig::system(scale, 13.0));
    if mode == RenderMode::Gpu && ui.is_cpu() {
        return None;
    }
    ui.set_edit_text(Id::new("q"), "hello world");
    let px = |v: u32| (v as f32 * scale).round() as u32;
    let target = ui.offscreen((px(SIZE.0), px(SIZE.1)));
    loop {
        ui.begin(color::SURFACE_BASE);
        let root = frame(&ui);
        ui.layout(root, (SIZE.0 as f32, SIZE.1 as f32));
        if ui.render_offscreen(&target) {
            break;
        }
    }
    ui.read(&target).ok()
}

/// A striped field under a glass panel (backdrop blur), cards with the
/// theme's shadows at several radii, hairline borders and text in light
/// and dark ink.
fn effects(_ui: &Ui) -> El {
    let stripes = (0..16).map(|i| {
        let c = [0xc9972a, 0x6f8faf, 0x7d9471, 0xd4623f][i % 4];
        div().w(40.0).h(SIZE.1 as f32).bg(Rgba::hex(c))
    });
    let card = |r: f32, sh: &[(f32, f32, f32, f32)]| {
        div()
            .w(120.0)
            .h(70.0)
            .rounded(r)
            .bg(color::SURFACE_OVERLAY)
            .border(1.0, Rgba::white(0.1))
            .shadow(sh)
            .p(8.0)
            .child(text("Shadow", 13.0, color::INK))
    };
    div()
        .w(SIZE.0 as f32)
        .h(SIZE.1 as f32)
        .bg(color::GRAY_100)
        .child(div().row().children(stripes))
        .child(
            div()
                .absolute(24.0, 24.0)
                .w(300.0)
                .h(150.0)
                .rounded(theme::radius::XL)
                .bg(Rgba::hexa(0x1c1c20, 0.55))
                .backdrop_blur(theme::blur::GLASS)
                .p(16.0)
                .child(text("Frosted glass", 14.0, color::INK))
                .child(text("dark ink", 13.0, Rgba::hex(0x101012))),
        )
        .child(
            div()
                .absolute(360.0, 40.0)
                .col()
                .gap(40.0)
                .child(card(theme::radius::LG, &theme::shadow::XL))
                .child(card(0.0, &theme::shadow::XL2))
                .child(card(theme::radius::FULL, &theme::shadow::GLASS)),
        )
}

fn compare(frame: fn(&Ui) -> El, scale: f32) {
    let Some(gpu) = draw(RenderMode::Gpu, scale, frame) else {
        eprintln!("no GPU adapter; skipped");
        return;
    };
    let cpu = draw(RenderMode::Cpu, scale, frame).expect("cpu frame");
    assert_eq!(gpu.len(), cpu.len());
    let px = gpu.len() / 4;
    let (mut off, mut total) = (0usize, 0u64);
    for (g, c) in gpu.chunks_exact(4).zip(cpu.chunks_exact(4)) {
        let d = (0..3).map(|i| g[i].abs_diff(c[i])).max().unwrap_or(0);
        total += u64::from(d);
        if d > 32 {
            off += 1;
        }
    }
    let mean = total as f64 / px as f64;
    let share = off as f64 / px as f64;
    eprintln!("@{scale}x: mean {mean:.3}, {:.3}% beyond 32", share * 100.0);
    assert!(
        share < 0.01,
        "@{scale}x: {:.3}% of pixels differ by more than 32",
        share * 100.0
    );
    assert!(mean < 1.0, "@{scale}x: mean difference {mean:.3}");
}

#[test]
fn gpu_and_cpu_draw_the_same_widgets() {
    compare(frame, 2.0);
}

#[test]
fn gpu_and_cpu_draw_the_same_shadows_blur_and_text() {
    for scale in [1.0, 1.5, 2.0] {
        compare(effects, scale);
    }
}

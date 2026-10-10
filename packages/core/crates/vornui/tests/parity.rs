//! The GPU and CPU renderers draw the same frame. Edges are antialiased by
//! different rasterizers, so the check is per pixel within a tolerance and
//! a small share of edge pixels, not byte equality.

use vornui::theme::color;
use vornui::widgets::{self, Choice, PickerLook, Pill, Tab};
use vornui::{div, El, Id, RenderMode, Ui, UiConfig};

const SIZE: (u32, u32) = (640, 400);

fn frame(ui: &Ui) -> El {
    let pills = [
        Pill {
            label: "Terminals",
            icon: None,
        },
        Pill {
            label: "Tasks",
            icon: None,
        },
    ];
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

fn draw(mode: RenderMode) -> Option<Vec<u8>> {
    let mut ui = Ui::headless(mode, &UiConfig::system(2.0, 13.0));
    if mode == RenderMode::Gpu && ui.is_cpu() {
        return None;
    }
    ui.set_edit_text(Id::new("q"), "hello world");
    let target = ui.offscreen((SIZE.0 * 2, SIZE.1 * 2));
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

#[test]
fn gpu_and_cpu_draw_the_same_widgets() {
    let Some(gpu) = draw(RenderMode::Gpu) else {
        eprintln!("no GPU adapter; skipped");
        return;
    };
    let cpu = draw(RenderMode::Cpu).expect("cpu frame");
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
    eprintln!("mean {mean:.3}, {:.3}% beyond 32", share * 100.0);
    assert!(
        share < 0.01,
        "{:.3}% of pixels differ by more than 32",
        share * 100.0
    );
    assert!(mean < 1.0, "mean difference {mean:.3}");
}

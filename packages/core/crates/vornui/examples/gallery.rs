//! The renderer's showcase: soft shadows on floating surfaces, backdrop
//! blur, crisp hairline borders, text, and motion.
//!
//! `cargo run -p vornui --example gallery` opens it in a window (click
//! "Animate" to replay the motion row). With `--render <dir>` it draws
//! offscreen instead and writes `gallery-<renderer>-<scale>x.png` and a
//! motion filmstrip; `--renderer cpu|gpu`, `--scale 1,1.5,2` pick what is
//! drawn, `--before` turns shadows, blur and text gamma off for a
//! comparison, and `--reduce-motion` draws the filmstrip with reduced
//! motion.

use std::time::{Duration, Instant};

use vornui::theme::{self, color};
use vornui::widgets;
use vornui::{
    div, text, write_png, App, El, Event, Id, Offscreen, RenderMode, Rgba, TextGamma, Transform,
    Transition, Ui, UiConfig,
};

const SIZE: (f32, f32) = (1040.0, 760.0);
const ANIMATE: &str = "animate";
/// The motion row's place in the frame, logical pixels: top, height, width.
const MOTION_ROWS: (usize, usize, usize) = (505, 110, 560);

struct Look {
    effects: bool,
}

fn heading(s: &str) -> El {
    text(s, 12.0, color::INK_SECONDARY).weight(600)
}

type Layers = [(f32, f32, f32, f32)];

fn shadows<'a>(look: &Look, list: &'a Layers) -> &'a Layers {
    if look.effects {
        list
    } else {
        &[]
    }
}

fn menu(look: &Look) -> El {
    let item = |label: &str, hot: bool| {
        div()
            .row()
            .items_center()
            .h(28.0)
            .px(8.0)
            .rounded(theme::radius::SM)
            .bg(if hot {
                Rgba::white(0.06)
            } else {
                Rgba::TRANSPARENT
            })
            .child(text(label, 13.0, color::INK))
    };
    div()
        .col()
        .w(180.0)
        .p(4.0)
        .rounded(theme::radius::LG)
        .bg(color::SURFACE_OVERLAY)
        .border(1.0, Rgba::white(0.08))
        .shadow(shadows(look, &theme::shadow::XL))
        .child(item("New terminal", false))
        .child(item("Split right", true))
        .child(item("Rename…", false))
        .child(item("Close", false))
}

fn tooltip(look: &Look) -> El {
    div()
        .px(8.0)
        .py(4.0)
        .rounded(theme::radius::MD)
        .bg(color::GRAY_700)
        .shadow(shadows(look, &theme::shadow::MD))
        .child(text("Open settings  Ctrl+,", 12.0, color::INK))
}

fn dialog(look: &Look) -> El {
    div()
        .col()
        .gap(10.0)
        .w(280.0)
        .p(16.0)
        .rounded(theme::radius::XL)
        .bg(color::SURFACE_OVERLAY)
        .border(1.0, Rgba::white(0.08))
        .shadow(shadows(look, &theme::shadow::XL2))
        .child(text("Discard changes?", 14.0, color::INK).weight(600))
        .child(text(
            "The session's edits will be lost.",
            13.0,
            color::INK_SECONDARY,
        ))
}

fn floating(look: &Look) -> El {
    div()
        .col()
        .gap(12.0)
        .child(heading("FLOATING SURFACES"))
        .child(
            div()
                .col()
                .gap(32.0)
                .p(24.0)
                .rounded(theme::radius::XL)
                .bg(color::SURFACE_PANEL)
                .child(
                    div()
                        .row()
                        .gap(24.0)
                        .items_center()
                        .child(menu(look))
                        .child(tooltip(look)),
                )
                .child(dialog(look)),
        )
}

fn shadow_scale(look: &Look) -> El {
    let card = |name: &str, list: &Layers, r: f32| {
        div()
            .w(76.0)
            .h(60.0)
            .rounded(r)
            .bg(color::WHITE)
            .shadow(shadows(look, list))
            .center()
            .child(text(name, 12.0, color::GRAY_600))
    };
    div().col().gap(12.0).child(heading("SHADOW SCALE")).child(
        div()
            .row()
            .gap(16.0)
            .p(24.0)
            .rounded(theme::radius::XL)
            .bg(color::GRAY_100)
            .child(card("sm", &theme::shadow::SM, theme::radius::MD))
            .child(card("md", &theme::shadow::MD, theme::radius::MD))
            .child(card("lg", &theme::shadow::LG, theme::radius::LG))
            .child(card("xl", &theme::shadow::XL, theme::radius::XL))
            .child(card("2xl", &theme::shadow::XL2, 0.0))
            .child(card("glass", &theme::shadow::GLASS, theme::radius::FULL)),
    )
}

fn blur(look: &Look) -> El {
    let stripes = (0..30).map(|i| {
        let c = [
            color::BRONZO,
            color::STATUS_BLUE,
            color::STATUS_SAGE,
            color::DANGER,
        ][i % 4];
        div().w(20.0).h(150.0).bg(c)
    });
    let glass = |x: f32, sigma: f32, label: &str| {
        let panel = div()
            .absolute(x, 40.0)
            .w(220.0)
            .h(90.0)
            .rounded(theme::radius::XL)
            .bg(Rgba::hexa(0x1c1c20, 0.45))
            .border(1.0, Rgba::white(0.1))
            .p(12.0)
            .child(text(label, 13.0, color::INK).weight(500));
        if look.effects {
            panel.backdrop_blur(sigma)
        } else {
            panel
        }
    };
    div().col().gap(12.0).child(heading("BACKDROP BLUR")).child(
        div()
            .w(600.0)
            .h(150.0)
            .rounded(theme::radius::XL)
            .clip()
            .child(div().row().children(stripes))
            .child(
                div()
                    .absolute(16.0, 8.0)
                    .child(text("Vorn · terminals that think", 24.0, color::INK).weight(700)),
            )
            .child(glass(40.0, theme::blur::SM, "blur-sm (4px)"))
            .child(glass(320.0, theme::blur::GLASS, "glass (16px)")),
    )
}

fn borders() -> El {
    let boxes = [0.0, 4.0, 6.0, 8.0, 12.0, 9999.0].map(|r| {
        div()
            .w(52.0)
            .h(32.0)
            .rounded(r)
            .border(1.0, Rgba::white(0.35))
    });
    let rules = div()
        .col()
        .w(120.0)
        .children((0..4).map(|_| div().h(9.0).border_bottom(1.0, Rgba::white(0.25))));
    div().col().gap(12.0).child(heading("HAIRLINES")).child(
        div()
            .row()
            .gap(12.0)
            .items_center()
            .p(16.0)
            .rounded(theme::radius::XL)
            .bg(color::SURFACE_PANEL)
            .border(1.0, Rgba::white(0.08))
            .children(boxes)
            .child(rules),
    )
}

fn type_specimen() -> El {
    let lines = |ink: Rgba| {
        div()
            .col()
            .gap(4.0)
            .child(text(
                "The quick brown fox jumps over the lazy dog",
                12.0,
                ink,
            ))
            .child(text("The quick brown fox jumps over the lazy dog", 13.0, ink).weight(500))
            .child(text(
                "The quick brown fox jumps over the lazy dog",
                14.0,
                ink,
            ))
            .child(text("Session 42 · 3 agents running", 16.0, ink).weight(600))
    };
    div().col().gap(12.0).child(heading("TEXT")).child(
        div()
            .col()
            .rounded(theme::radius::XL)
            .clip()
            .child(
                div()
                    .p(16.0)
                    .bg(color::SURFACE_SUNKEN)
                    .child(lines(color::INK)),
            )
            .child(div().p(16.0).bg(color::WHITE).child(lines(color::GRAY_700))),
    )
}

/// Every animated property in one row: an entrance (opacity, scale,
/// offset), a colour change and a size change.
fn motion(ui: &Ui, on: bool) -> El {
    let key = Id::new("gallery-motion");
    let hidden = Transform {
        scale: 0.95,
        offset: (0.0, -8.0),
    };
    let xf = ui.animate_from(
        key.child("xf"),
        hidden,
        if on { Transform::IDENTITY } else { hidden },
        Transition::POPOVER,
    );
    let fade = ui.animate_from(
        key.child("fade"),
        0.0f32,
        if on { 1.0 } else { 0.0 },
        Transition::POPOVER,
    );
    let tint = ui.animate(
        key.child("tint"),
        if on { color::BRONZO } else { color::GRAY_600 },
        Transition::COLORS,
    );
    let size = ui.animate(
        key.child("size"),
        if on { (200.0, 32.0) } else { (60.0, 32.0) },
        Transition::spring(Duration::from_millis(350), 0.3),
    );
    div()
        .col()
        .gap(12.0)
        .child(
            div()
                .row()
                .gap(12.0)
                .items_center()
                .child(heading("MOTION"))
                .child(widgets::button(ui, Id::new(ANIMATE), "Animate")),
        )
        .child(
            div()
                .row()
                .gap(24.0)
                .items_center()
                .h(72.0)
                .child(
                    div()
                        .w(140.0)
                        .h(60.0)
                        .rounded(theme::radius::LG)
                        .bg(color::SURFACE_OVERLAY)
                        .border(1.0, Rgba::white(0.08))
                        .opacity(fade)
                        .transform(xf.scale, xf.offset)
                        .center()
                        .child(text("popover", 13.0, color::INK)),
                )
                .child(div().w(60.0).h(32.0).rounded(theme::radius::MD).bg(tint))
                .child(
                    div()
                        .w(size.0)
                        .h(size.1)
                        .rounded(theme::radius::FULL)
                        .bg(color::STATUS_BLUE),
                ),
        )
}

fn gallery(ui: &Ui, look: &Look, on: bool) -> El {
    div()
        .w(SIZE.0)
        .h(SIZE.1)
        .col()
        .gap(24.0)
        .p(24.0)
        .bg(color::SURFACE_BASE)
        .child(
            div().row().gap(24.0).child(floating(look)).child(
                div()
                    .col()
                    .gap(24.0)
                    .child(shadow_scale(look))
                    .child(blur(look)),
            ),
        )
        .child(
            div()
                .row()
                .gap(24.0)
                .child(div().col().gap(24.0).child(borders()).child(motion(ui, on)))
                .child(type_specimen()),
        )
}

struct Gallery {
    on: bool,
}

impl App for Gallery {
    fn view(&mut self, ui: &mut Ui) -> El {
        gallery(ui, &Look { effects: true }, self.on)
    }

    fn event(&mut self, _ui: &mut Ui, event: Event) {
        if matches!(event, Event::Click(id) if id == Id::new(ANIMATE)) {
            self.on = !self.on;
        }
    }
}

struct Args {
    out: Option<String>,
    mode: RenderMode,
    scales: Vec<f32>,
    before: bool,
    reduce: bool,
}

fn args() -> Result<Args, String> {
    let mut a = Args {
        out: None,
        mode: RenderMode::Gpu,
        scales: vec![1.0, 1.5, 2.0],
        before: false,
        reduce: false,
    };
    let mut it = std::env::args().skip(1);
    while let Some(arg) = it.next() {
        let mut value = || it.next().ok_or(format!("{arg} needs a value"));
        match arg.as_str() {
            "--render" => a.out = Some(value()?),
            "--renderer" => {
                a.mode = match value()?.as_str() {
                    "cpu" => RenderMode::Cpu,
                    "gpu" => RenderMode::Gpu,
                    other => return Err(format!("unknown renderer {other}")),
                }
            }
            "--scale" => {
                a.scales = value()?
                    .split(',')
                    .map(|s| s.parse().map_err(|_| format!("bad scale {s}")))
                    .collect::<Result<_, _>>()?;
            }
            "--before" => a.before = true,
            "--reduce-motion" => a.reduce = true,
            other => return Err(format!("unknown argument {other}")),
        }
    }
    Ok(a)
}

/// Draws one frame of `build` at `now` and reads it back.
fn shoot(
    ui: &mut Ui,
    target: &Offscreen,
    now: Instant,
    build: &dyn Fn(&Ui) -> El,
) -> Result<Vec<u8>, String> {
    loop {
        ui.begin_at(color::SURFACE_BASE, now);
        let root = build(ui);
        ui.layout(root, SIZE);
        if ui.render_offscreen(target) {
            break;
        }
    }
    ui.read(target)
}

fn render(a: &Args, dir: &str) -> Result<(), String> {
    let look = Look { effects: !a.before };
    let renderer = match a.mode {
        RenderMode::Cpu => "cpu",
        _ => "gpu",
    };
    let suffix = if a.before { "-before" } else { "" };
    for &scale in &a.scales {
        let mut ui = Ui::headless(a.mode, &UiConfig::system(scale, 13.0));
        if a.before {
            ui.text.gamma = TextGamma::OFF;
        }
        let px = |v: f32| (v * scale).round() as u32;
        let size = (px(SIZE.0), px(SIZE.1));
        let target = ui.offscreen(size);
        // The second frame, once every entrance has settled.
        let t0 = Instant::now();
        let build = |ui: &Ui| gallery(ui, &look, true);
        shoot(&mut ui, &target, t0, &build)?;
        let frame = shoot(&mut ui, &target, t0 + Duration::from_secs(3), &build)?;
        let path = format!("{dir}/gallery-{renderer}-{scale}x{suffix}.png");
        write_png(&path, size, &frame)?;
        println!("{path} ({})", if ui.is_cpu() { "cpu" } else { "gpu" });
    }
    if a.before {
        return Ok(());
    }
    // The filmstrip: the motion row every 40 ms after "Animate".
    let mut ui = Ui::headless(a.mode, &UiConfig::system(1.0, 13.0));
    ui.set_reduced_motion(a.reduce);
    let size = (SIZE.0 as u32, SIZE.1 as u32);
    let target = ui.offscreen(size);
    let t0 = Instant::now();
    shoot(&mut ui, &target, t0, &|ui| gallery(ui, &look, false))?;
    let (top, rows, width) = MOTION_ROWS;
    let row = size.0 as usize * 4;
    let mut strip = Vec::new();
    let mut frames = 0;
    for i in 1..=12 {
        let now = t0 + Duration::from_millis(40 * i);
        let frame = shoot(&mut ui, &target, now, &|ui| gallery(ui, &look, true))?;
        for y in top..top + rows {
            strip.extend_from_slice(&frame[y * row..y * row + width * 4]);
        }
        frames += 1;
        if !ui.animating() {
            break;
        }
    }
    let name = if a.reduce {
        "gallery-motion-reduced.png"
    } else {
        "gallery-motion.png"
    };
    let path = format!("{dir}/{name}");
    write_png(&path, (width as u32, (rows * frames) as u32), &strip)?;
    println!("{path} ({frames} frames, 40 ms apart)");
    Ok(())
}

fn main() {
    let a = match args() {
        Ok(a) => a,
        Err(e) => {
            eprintln!("gallery: {e}");
            std::process::exit(2);
        }
    };
    let result = match &a.out {
        Some(dir) => render(&a, dir),
        None => vornui::run(
            "vornui gallery",
            SIZE,
            UiConfig::system(1.0, 13.0),
            |_, _| Gallery { on: true },
        ),
    };
    if let Err(e) = result {
        eprintln!("gallery: {e}");
        std::process::exit(1);
    }
}

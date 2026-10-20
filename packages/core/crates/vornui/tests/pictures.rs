//! Pictures drawn with [`Ui::picture`] on the CPU renderer: scaled into their rect, cut to their
//! source rect, and a large one shrunk to fit the atlas but still drawn whole.

use vornui::{Rect, RenderMode, Rgba, Ui, UiConfig};

const RED: [u8; 4] = [255, 0, 0, 255];
const GREEN: [u8; 4] = [0, 255, 0, 255];
const BLUE: [u8; 4] = [0, 0, 255, 255];
const WHITE: [u8; 4] = [255, 255, 255, 255];

/// A `w` by `h` picture of four quadrants: red, green over blue, white.
fn quadrants(w: u32, h: u32) -> Vec<u8> {
    let mut px = Vec::with_capacity((w * h * 4) as usize);
    for y in 0..h {
        for x in 0..w {
            let c = match (x < w / 2, y < h / 2) {
                (true, true) => RED,
                (false, true) => GREEN,
                (true, false) => BLUE,
                (false, false) => WHITE,
            };
            px.extend_from_slice(&c);
        }
    }
    px
}

/// Draws the pictures `draw` puts on a black 64 by 64 frame and answers its RGBA pixels.
fn frame(draw: impl Fn(&mut Ui)) -> Vec<u8> {
    let mut ui = Ui::headless(RenderMode::Cpu, &UiConfig::system(1.0, 13.0));
    let target = ui.offscreen((64, 64));
    // The first frame may fill the atlas and ask to be drawn again.
    for _ in 0..2 {
        ui.begin(Rgba::hex(0x000000));
        draw(&mut ui);
        if ui.render_offscreen(&target) {
            return ui.read(&target).expect("a frame");
        }
    }
    panic!("the frame never fit the atlas");
}

fn at(px: &[u8], x: usize, y: usize) -> [u8; 4] {
    let i = (y * 64 + x) * 4;
    [px[i], px[i + 1], px[i + 2], px[i + 3]]
}

fn near(a: [u8; 4], b: [u8; 4]) -> bool {
    a.iter().zip(b).all(|(a, b)| a.abs_diff(b) < 24)
}

#[test]
fn a_picture_is_scaled_into_its_rect() {
    let rgba = quadrants(4, 4);
    let px = frame(|ui| {
        ui.picture(
            1,
            (4, 4),
            &rgba,
            [0.0, 0.0, 4.0, 4.0],
            Rect::new(8.0, 8.0, 40.0, 40.0),
        )
    });
    assert!(near(at(&px, 16, 16), RED));
    assert!(near(at(&px, 40, 16), GREEN));
    assert!(near(at(&px, 16, 40), BLUE));
    assert!(near(at(&px, 40, 40), WHITE));
    assert!(
        near(at(&px, 4, 4), [0, 0, 0, 255]),
        "nothing outside the rect"
    );
    assert!(near(at(&px, 56, 56), [0, 0, 0, 255]));
}

#[test]
fn only_the_source_rect_is_drawn() {
    let rgba = quadrants(64, 64);
    // The right half: green over white.
    let px = frame(|ui| {
        ui.picture(
            1,
            (64, 64),
            &rgba,
            [32.0, 0.0, 32.0, 64.0],
            Rect::new(0.0, 0.0, 64.0, 64.0),
        )
    });
    assert!(near(at(&px, 8, 8), GREEN));
    assert!(near(at(&px, 56, 8), GREEN));
    assert!(near(at(&px, 8, 56), WHITE));
    assert!(near(at(&px, 56, 56), WHITE));
}

#[test]
fn a_picture_larger_than_the_atlas_takes_is_shrunk_not_dropped() {
    let (w, h) = (4000, 2000);
    let rgba = quadrants(w, h);
    let px = frame(|ui| {
        let src = [0.0, 0.0, w as f32, h as f32];
        ui.picture(9, (w, h), &rgba, src, Rect::new(0.0, 0.0, 64.0, 32.0));
    });
    assert!(near(at(&px, 8, 4), RED));
    assert!(near(at(&px, 56, 4), GREEN));
    assert!(near(at(&px, 8, 28), BLUE));
    assert!(near(at(&px, 56, 28), WHITE));
}

#[test]
fn bad_sizes_draw_nothing() {
    let px = frame(|ui| {
        ui.picture(1, (0, 4), &[], [0.0; 4], Rect::new(0.0, 0.0, 64.0, 64.0));
        // Fewer bytes than the size says.
        ui.picture(
            2,
            (4, 4),
            &[255; 8],
            [0.0, 0.0, 4.0, 4.0],
            Rect::new(0.0, 0.0, 64.0, 64.0),
        );
    });
    assert!(near(at(&px, 32, 32), [0, 0, 0, 255]));
}

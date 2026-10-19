//! A known scene drawn for a pixel terminal and played back on a model of
//! one: what the terminal ends up showing must be, pixel for pixel, what
//! the CPU renderer drew, frame after frame, whichever way the images went.
//! With `VORN_PIXELS_PNG_DIR` set, each frame the model shows is saved there.

use std::collections::HashMap;
use std::io::Read;
use std::path::PathBuf;

use vornui::theme::color;
use vornui::tty::kitty::Wrap;
use vornui::tty::present::{Placing, Sent, Transport};
use vornui::tty::PixelFrames;
use vornui::{div, text, El, Place, RenderMode, Rgba, Ui, UiConfig};

const SCALE: f32 = 2.0;
const CELL: (u32, u32) = (16, 32);
/// 120 by 40 cells.
const PX: (u32, u32) = (120 * CELL.0, 40 * CELL.1);

fn logical() -> (f32, f32) {
    (PX.0 as f32 / SCALE, PX.1 as f32 / SCALE)
}

/// A sidebar of rows, a card holding `typed`, and a menu when `menu`.
fn scene(typed: &str, scrolled: usize, menu: bool) -> El {
    let (w, h) = logical();
    let mut side = div().col().w(220.0).h(h).bg(color::SURFACE_SUNKEN).gap(4.0);
    for i in scrolled..scrolled + 24 {
        side = side.child(text(format!("Session {i}"), 13.0, color::INK));
    }
    let mut button = div()
        .size(120.0, 28.0)
        .rounded(6.0)
        .bg(Rgba::hex(0x3b82f6))
        .child(text("Open", 13.0, Rgba::hex(0xffffff)));
    if menu {
        let panel = div()
            .col()
            .w(180.0)
            .rounded(8.0)
            .bg(Rgba::hex(0x34343c))
            .child(text("Rename", 13.0, color::INK))
            .child(text("Close", 13.0, color::INK));
        button = button.overlay(panel, Place::BelowStart, 4.0);
    }
    let card = div()
        .col()
        .w(500.0)
        .h(200.0)
        .rounded(12.0)
        .bg(color::SURFACE_RAISED)
        .gap(8.0)
        .child(text(typed, 15.0, color::INK))
        .child(button);
    div()
        .row()
        .w(w)
        .h(h)
        .bg(color::SURFACE_BASE)
        .gap(24.0)
        .child(side)
        .child(card)
}

/// What a kitty-graphics terminal keeps: images, where each is placed and
/// at what depth, and the cursor the next placement starts from.
#[derive(Default)]
struct Model {
    images: HashMap<u32, Image>,
    cursor: (u32, u32),
    /// Chunks of an image still arriving: its keys and its payload so far.
    pending: Option<(HashMap<String, String>, Vec<u8>)>,
    order: u64,
}

struct Image {
    size: (u32, u32),
    rgb: Vec<u8>,
    at: (u32, u32),
    z: i64,
    order: u64,
}

impl Model {
    fn feed(&mut self, bytes: &[u8]) {
        let mut i = 0;
        while i < bytes.len() {
            assert_eq!(bytes[i], 0x1b, "only escapes are sent, at {i}");
            match bytes[i + 1] {
                b'[' => {
                    let end = i
                        + 2
                        + bytes[i + 2..]
                            .iter()
                            .position(|b| b.is_ascii_alphabetic())
                            .unwrap();
                    assert_eq!(bytes[end], b'H', "only the cursor moves");
                    let args = std::str::from_utf8(&bytes[i + 2..end]).unwrap();
                    let (row, col) = args.split_once(';').unwrap();
                    self.cursor = (
                        (col.parse::<u32>().unwrap() - 1) * CELL.0,
                        (row.parse::<u32>().unwrap() - 1) * CELL.1,
                    );
                    i = end + 1;
                }
                b'_' => {
                    let end = i + bytes[i..].windows(2).position(|w| w == b"\x1b\\").unwrap();
                    self.graphics(&bytes[i + 3..end]);
                    i = end + 2;
                }
                other => panic!("unexpected escape {:?}", other as char),
            }
        }
    }

    fn graphics(&mut self, body: &[u8]) {
        let body = std::str::from_utf8(body).unwrap();
        let (keys, payload) = body.split_once(';').unwrap_or((body, ""));
        let keys: HashMap<String, String> = keys
            .split(',')
            .filter_map(|kv| kv.split_once('='))
            .map(|(k, v)| (k.to_owned(), v.to_owned()))
            .collect();
        let data = base64(payload);
        let (mut head, mut buf) = self.pending.take().unwrap_or_default();
        if head.is_empty() {
            head = keys.clone();
        }
        buf.extend_from_slice(&data);
        if keys.get("m").is_some_and(|m| m == "1") {
            self.pending = Some((head, buf));
            return;
        }
        let num = |k: &str| head.get(k).map(|v| v.parse::<i64>().unwrap()).unwrap_or(0);
        let id = num("i") as u32;
        match head.get("a").map(String::as_str) {
            Some("d") => {
                assert_eq!(head.get("d").map(String::as_str), Some("I"));
                assert!(
                    self.images.remove(&id).is_some(),
                    "image {id} deleted twice"
                );
            }
            Some("T") => {
                assert_eq!(num("f"), 24);
                assert_eq!(num("C"), 1, "the cursor stays put");
                let rgb = match head.get("t").map(String::as_str) {
                    Some("t") => {
                        let path = PathBuf::from(String::from_utf8(buf).unwrap());
                        let rgb = std::fs::read(&path).unwrap();
                        std::fs::remove_file(&path).unwrap();
                        rgb
                    }
                    None if head.get("o").is_some_and(|o| o == "z") => {
                        let mut rgb = Vec::new();
                        flate2::read::ZlibDecoder::new(&buf[..])
                            .read_to_end(&mut rgb)
                            .unwrap();
                        rgb
                    }
                    None => buf,
                    Some(t) => panic!("medium {t}"),
                };
                let size = (num("s") as u32, num("v") as u32);
                assert_eq!(rgb.len(), (size.0 * size.1 * 3) as usize);
                let at = (
                    self.cursor.0 + num("X") as u32,
                    self.cursor.1 + num("Y") as u32,
                );
                self.order += 1;
                let image = Image {
                    size,
                    rgb,
                    at,
                    z: num("z"),
                    order: self.order,
                };
                assert!(self.images.insert(id, image).is_none(), "id {id} reused");
            }
            a => panic!("action {a:?}"),
        }
    }

    /// The screen: every image, deepest first, as RGB.
    fn screen(&self) -> Vec<u8> {
        let mut out = vec![0u8; (PX.0 * PX.1 * 3) as usize];
        let mut list: Vec<&Image> = self.images.values().collect();
        list.sort_by_key(|im| (im.z, im.order));
        for im in list {
            for y in 0..im.size.1 {
                let row = (im.at.1 + y) as usize;
                let src = &im.rgb[(y * im.size.0 * 3) as usize..][..(im.size.0 * 3) as usize];
                let dst = (row * PX.0 as usize + im.at.0 as usize) * 3;
                out[dst..dst + src.len()].copy_from_slice(src);
            }
        }
        out
    }
}

fn base64(s: &str) -> Vec<u8> {
    const ABC: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
    let mut out = Vec::new();
    let (mut acc, mut bits) = (0u32, 0);
    for c in s.bytes().filter(|&c| c != b'=') {
        acc = (acc << 6) | ABC.iter().position(|&a| a == c).unwrap() as u32;
        bits += 6;
        if bits >= 8 {
            bits -= 8;
            out.push((acc >> bits) as u8);
        }
    }
    out
}

struct Rig {
    ui: Ui,
    frames: PixelFrames,
    model: Model,
    name: &'static str,
    n: usize,
}

impl Rig {
    fn new(name: &'static str, transport: Transport) -> Rig {
        Rig {
            ui: Ui::headless(RenderMode::Cpu, &UiConfig::system(SCALE, 13.0)),
            frames: PixelFrames::new(Wrap::Direct, Placing::Cursor, CELL, transport),
            model: Model::default(),
            name,
            n: 0,
        }
    }

    /// Draws `el`, plays what was sent, and checks the model shows the frame.
    fn frame(&mut self, el: El) -> Sent {
        self.ui.begin(color::SURFACE_BASE);
        self.ui.layout(el, logical());
        let mut out = Vec::new();
        let sent = self
            .frames
            .draw(&mut self.ui, PX, &mut out)
            .expect("atlas room");
        assert_eq!(sent.bytes, out.len());
        self.model.feed(&out);
        let (rgba, size) = self.frames.pixels();
        assert_eq!(size, PX);
        let want: Vec<u8> = rgba
            .chunks_exact(4)
            .flat_map(|p| [p[0], p[1], p[2]])
            .collect();
        let shown = self.model.screen();
        self.save(&shown);
        let wrong = shown
            .chunks(3)
            .zip(want.chunks(3))
            .filter(|(a, b)| a != b)
            .count();
        assert_eq!(
            wrong, 0,
            "{} frame {}: pixels differ from the render",
            self.name, self.n
        );
        self.n += 1;
        sent
    }

    fn save(&self, rgb: &[u8]) {
        let Some(dir) = std::env::var_os("VORN_PIXELS_PNG_DIR") else {
            return;
        };
        let path = PathBuf::from(dir).join(format!("{}-{}.png", self.name, self.n));
        let file = std::io::BufWriter::new(std::fs::File::create(path).unwrap());
        let mut enc = png::Encoder::new(file, PX.0, PX.1);
        enc.set_color(png::ColorType::Rgb);
        enc.set_depth(png::BitDepth::Eight);
        enc.write_header().unwrap().write_image_data(rgb).unwrap();
    }
}

/// Typing, a menu opening and closing, and scrolling, through `transport`.
fn play(name: &'static str, transport: Transport) -> Vec<Sent> {
    let mut r = Rig::new(name, transport);
    let mut sent = vec![r.frame(scene("hello", 0, false))];
    sent.push(r.frame(scene("hello!", 0, false)));
    let overlays = |m: &Model| m.images.values().filter(|i| i.z >= 1 << 30).count();
    sent.push(r.frame(scene("hello!", 0, true)));
    assert_eq!(overlays(&r.model), 1, "the menu is an image above the rest");
    sent.push(r.frame(scene("hello!", 0, false)));
    assert_eq!(overlays(&r.model), 0, "and goes when it closes");
    sent.push(r.frame(scene("hello!", 3, false)));
    // Nothing changed: nothing is sent.
    sent.push(r.frame(scene("hello!", 3, false)));
    sent
}

#[test]
fn the_terminal_shows_each_frame_exactly_inline() {
    let sent = play("inline", Transport::default());
    assert!(sent[0].whole);
    let typed = sent[1];
    assert!(
        !typed.whole && typed.images == 1,
        "a keystroke is one patch: {typed:?}"
    );
    assert!(
        typed.bytes * 20 < sent[0].bytes,
        "a keystroke sends little: {typed:?}"
    );
    assert!(
        sent[2].images >= 1 && !sent[2].whole,
        "the menu is its own layer"
    );
    assert_eq!(
        sent[5],
        Sent {
            whole: false,
            ..Sent::default()
        }
    );
}

#[test]
fn the_terminal_shows_each_frame_exactly_deflated() {
    let sent = play(
        "zlib",
        Transport {
            file_dir: None,
            zlib: true,
        },
    );
    let raw = (PX.0 * PX.1 * 3) as usize;
    assert!(
        sent[0].bytes * 4 < raw,
        "a flat UI deflates well: {:?}",
        sent[0]
    );
}

#[test]
fn the_terminal_shows_each_frame_exactly_through_files() {
    let dir = std::env::temp_dir().join(format!("vorn-pixels-test-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let sent = play(
        "files",
        Transport {
            file_dir: Some(dir.clone()),
            zlib: false,
        },
    );
    assert!(
        sent[0].file_bytes > 0 && sent[0].bytes < 512,
        "{:?}",
        sent[0]
    );
    let left = std::fs::read_dir(&dir).unwrap().count();
    std::fs::remove_dir_all(&dir).unwrap();
    assert_eq!(left, 0, "the terminal read every file");
}

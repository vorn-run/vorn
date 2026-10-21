//! Kitty graphics through an emulator: what a program transmits, places and deletes is what a
//! renderer reads back, placements follow the text through scrolls and resizes, PNG loads only
//! with images on and files only with image files on, every refused file reads the same, and
//! size reports carry the cell pixels a card was given.

use vorn_screen::{Effect, Emulator, Placed};

/// A 2×2 red RGB PNG.
const PNG: &str =
    "iVBORw0KGgoAAAANSUhEUgAAAAIAAAACCAIAAAD91JpzAAAAEElEQVR4nGP4z8AARAwQCgAf7gP9i18U1AAAAABJRU5ErkJggg==";

fn b64(bytes: &[u8]) -> String {
    const A: &[u8] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
    let mut out = String::new();
    for c in bytes.chunks(3) {
        let n = c
            .iter()
            .enumerate()
            .fold(0u32, |n, (i, &b)| n | u32::from(b) << (16 - 8 * i));
        for i in 0..4 {
            out.push(if i <= c.len() {
                A[(n >> (18 - 6 * i) & 63) as usize] as char
            } else {
                '='
            });
        }
    }
    out
}

fn apc(keys: &str, payload: &str) -> Vec<u8> {
    format!("\x1b_G{keys};{payload}\x1b\\").into_bytes()
}

/// A `w`×`h` RGBA image of one colour.
fn rgba(w: u32, h: u32, px: [u8; 4]) -> Vec<u8> {
    px.repeat((w * h) as usize)
}

/// An 80×24 card of 10×20 pixel cells, with images on.
fn card() -> Emulator {
    let mut em = Emulator::with_scrollback(80, 24, 1 << 20).unwrap();
    em.resize_px(80, 24, 800, 480, &mut Vec::new()).unwrap();
    em.set_images(true);
    em
}

fn feed(em: &mut Emulator, bytes: &[u8]) -> Vec<String> {
    let mut fx = Vec::new();
    em.feed(bytes, &mut fx);
    fx.into_iter()
        .filter_map(|e| match e {
            Effect::Reply(r) => Some(String::from_utf8_lossy(&r).into_owned()),
            _ => None,
        })
        .collect()
}

fn placed(em: &Emulator) -> Vec<Placed> {
    let (mut p, mut v) = (Vec::new(), Vec::new());
    em.placements(&mut p, &mut v);
    p.sort_by_key(|p| (p.image, p.row, p.col));
    p
}

fn transmit(em: &mut Emulator, id: u32, keys: &str) {
    let data = b64(&rgba(20, 40, [1, 2, 3, 255]));
    feed(
        em,
        &apc(&format!("a=T,f=32,s=20,v=40,i={id},q=2{keys}"), &data),
    );
}

#[test]
fn a_transmitted_and_put_image_reads_back_where_the_cursor_was() {
    let mut em = card();
    feed(&mut em, b"\x1b[3;5H");
    let data = b64(&rgba(20, 40, [9, 8, 7, 255]));
    let reply = feed(&mut em, &apc("a=T,f=32,s=20,v=40,i=7", &data));
    assert_eq!(reply, ["\x1b_Gi=7;OK\x1b\\"]);
    let p = placed(&em);
    assert_eq!(p.len(), 1);
    let p = p[0];
    assert_eq!((p.image, p.row, p.col), (7, 2, 4));
    assert_eq!((p.width, p.height, p.src), (20, 40, [0, 0, 20, 40]));
    let pic = em.picture(7, p.gen).unwrap();
    assert_eq!((pic.width, pic.height), (20, 40));
    assert_eq!(&pic.rgba[..4], &[9, 8, 7, 255]);
    assert!(em.picture(7, p.gen + 1).is_none());
    assert!(em.image_bytes() >= 20 * 40 * 4);
}

#[test]
fn chunked_rgb_and_sized_crops_come_out_as_placed() {
    let mut em = card();
    let data = b64(&[200u8, 100, 50].repeat(30 * 20));
    let (a, b) = data.split_at(40);
    feed(&mut em, &apc("a=t,f=24,s=30,v=20,i=3,q=2,m=1", a));
    assert!(placed(&em).is_empty());
    feed(&mut em, &apc("m=0", b));
    feed(
        &mut em,
        &apc(
            "a=p,i=3,p=1,x=10,y=5,w=10,h=10,c=4,r=2,X=3,Y=4,z=-5,q=2",
            "",
        ),
    );
    let p = placed(&em);
    assert_eq!(p.len(), 1);
    let p = p[0];
    assert_eq!((p.src, p.z, p.x_off, p.y_off), ([10, 5, 10, 10], -5, 3, 4));
    // The drawn size is the cells it covers less its offset into the first one.
    assert_eq!((p.width, p.height), (37, 36));
    let pic = em.picture(3, p.gen).unwrap();
    assert_eq!(&pic.rgba[..8], &[200, 100, 50, 255, 200, 100, 50, 255]);
}

#[test]
fn deletes_remove_what_they_name() {
    let mut em = card();
    for id in 1..=3 {
        feed(&mut em, format!("\x1b[{};1H", id * 5).as_bytes());
        transmit(&mut em, id, &format!(",z={id}"));
    }
    assert_eq!(placed(&em).len(), 3);
    // By z-index.
    feed(&mut em, &apc("a=d,d=z,z=2,q=2", ""));
    assert_eq!(
        placed(&em).iter().map(|p| p.image).collect::<Vec<_>>(),
        [1, 3]
    );
    // By id, freeing the image too.
    feed(&mut em, &apc("a=d,d=I,i=3,q=2", ""));
    assert_eq!(placed(&em).iter().map(|p| p.image).collect::<Vec<_>>(), [1]);
    assert!(em.picture(3, 0).is_none());
    // At the cursor's cell.
    feed(&mut em, b"\x1b[6;1H");
    feed(&mut em, &apc("a=d,d=c,q=2", ""));
    assert!(placed(&em).is_empty());
    // Everything visible.
    transmit(&mut em, 4, "");
    feed(&mut em, &apc("a=d,d=a,q=2", ""));
    assert!(placed(&em).is_empty());
}

#[test]
fn erasing_the_screen_drops_its_placements() {
    let mut em = card();
    transmit(&mut em, 5, "");
    feed(&mut em, b"\x1b[2J");
    assert!(placed(&em).is_empty());
    transmit(&mut em, 6, "");
    feed(&mut em, b"\x1bc");
    assert!(placed(&em).is_empty());
}

#[test]
fn a_placement_scrolls_with_its_text_and_leaves_with_its_history() {
    let mut em = Emulator::with_scrollback(80, 24, 64 * 1024).unwrap();
    em.resize_px(80, 24, 800, 480, &mut Vec::new()).unwrap();
    em.set_images(true);
    feed(&mut em, b"\x1b[10;1H");
    transmit(&mut em, 1, ",C=1");
    let gen = em.images_gen();
    feed(&mut em, b"\x1b[24;1H\n\n\n");
    assert_eq!(placed(&em)[0].row, 6);
    // Scrolling moves placements without touching the storage.
    assert_eq!(em.images_gen(), gen);
    feed(&mut em, &b"\n".repeat(7));
    assert_eq!(placed(&em)[0].row, -1);
    // It leaves once the history its rows were in is pruned.
    feed(&mut em, &b"line\n".repeat(20_000));
    assert!(placed(&em).is_empty());
}

#[test]
fn a_placement_kept_in_history_reads_above_the_viewport() {
    let mut em = card();
    transmit(&mut em, 1, ",C=1");
    feed(&mut em, b"\x1b[24;1H\n\n\n");
    let p = placed(&em);
    assert_eq!(p[0].row, -3);
}

#[test]
fn a_placement_survives_a_resize() {
    let mut em = card();
    feed(&mut em, b"\x1b[5;10H");
    transmit(&mut em, 1, ",C=1");
    em.resize_px(100, 30, 2000, 1200, &mut Vec::new()).unwrap();
    let p = placed(&em);
    assert_eq!((p[0].row, p[0].col), (4, 9));
    // Pixel sizes stay the image's own; cells grew.
    assert_eq!((p[0].width, p[0].height), (20, 40));
    em.resize_px(40, 24, 400, 480, &mut Vec::new()).unwrap();
    assert_eq!(placed(&em).len(), 1);
}

#[test]
fn png_decodes_only_with_images_on() {
    let mut em = card();
    em.set_images(false);
    let refused = feed(&mut em, &apc("a=t,f=100,i=1", PNG));
    assert!(refused[0].starts_with("\x1b_Gi=1;E"), "{refused:?}");
    em.set_images(true);
    let ok = feed(&mut em, &apc("a=T,f=100,i=2", PNG));
    assert_eq!(ok, ["\x1b_Gi=2;OK\x1b\\"]);
    let p = placed(&em)[0];
    let pic = em.picture(2, p.gen).unwrap();
    assert_eq!((pic.width, pic.height), (2, 2));
    assert_eq!(pic.rgba, [255, 0, 0, 255].repeat(4));
}

/// A refusal's reply with its image id taken out, so refusals compare whole.
fn refusal(reply: &[String]) -> String {
    let [reply] = reply else {
        panic!("one reply: {reply:?}");
    };
    let (_, rest) = reply.split_once(';').unwrap();
    rest.to_owned()
}

#[test]
fn files_load_only_with_image_files_on() {
    let dir = std::env::temp_dir().join(format!("vorn-kitty-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let file = dir.join("img.rgba");
    std::fs::write(&file, rgba(4, 4, [5, 6, 7, 255])).unwrap();
    let path = b64(file.to_str().unwrap().as_bytes());
    let mut em = card();
    assert!(!em.image_files(), "files start off");
    let refused = feed(&mut em, &apc("a=t,t=f,f=32,s=4,v=4,i=1", &path));
    assert_eq!(refused, ["\x1b_Gi=1;EINVAL: unsupported medium\x1b\\"]);
    // Sent in the stream, an image loads either way.
    transmit(&mut em, 9, "");
    assert_eq!(placed(&em).len(), 1);
    em.set_image_files(true);
    let ok = feed(&mut em, &apc("a=t,t=f,f=32,s=4,v=4,i=2", &path));
    assert_eq!(ok, ["\x1b_Gi=2;OK\x1b\\"]);
    // A temporary file must be named as one, and is deleted once read.
    let tmp = dir.join("tty-graphics-protocol-x.rgba");
    std::fs::write(&tmp, rgba(4, 4, [5, 6, 7, 255])).unwrap();
    let ok = feed(
        &mut em,
        &apc(
            "a=t,t=t,f=32,s=4,v=4,i=3",
            &b64(tmp.to_str().unwrap().as_bytes()),
        ),
    );
    assert_eq!(ok, ["\x1b_Gi=3;OK\x1b\\"]);
    assert!(!tmp.exists());
    em.set_image_files(false);
    let off = refusal(&feed(&mut em, &apc("a=t,t=f,f=32,s=4,v=4,i=4", &path)));
    assert_eq!(off, "EINVAL: unsupported medium\x1b\\");
    std::fs::remove_dir_all(&dir).unwrap();
}

#[cfg(unix)]
#[test]
fn every_refused_file_reads_the_same_whatever_the_reason() {
    let dir = std::env::temp_dir().join(format!("vorn-kitty-same-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let misnamed = dir.join("plain.rgba");
    std::fs::write(&misnamed, rgba(4, 4, [5, 6, 7, 255])).unwrap();
    let path = |p: &std::path::Path| b64(p.to_str().unwrap().as_bytes());
    let mut em = card();
    em.set_image_files(true);
    let mut refusals = Vec::new();
    for (id, keys, data) in [
        // Not a regular file.
        (1, "t=f", path(&dir)),
        // Not there.
        (2, "t=f", path(&dir.join("missing.rgba"))),
        // A temporary file not named as one.
        (3, "t=t", path(&misnamed)),
        // A temporary file outside the temp dir.
        (4, "t=t", path(std::path::Path::new("/etc/tty-graphics-protocol"))),
        // A shared memory object that is not there.
        (5, "t=s", b64(b"/vorn-no-such-shm")),
        // A device.
        (6, "t=f", path(std::path::Path::new("/dev/zero"))),
    ] {
        let reply = feed(&mut em, &apc(&format!("a=t,{keys},f=32,s=4,v=4,i={id}"), &data));
        assert!(reply[0].starts_with(&format!("\x1b_Gi={id};")), "{reply:?}");
        refusals.push(refusal(&reply));
    }
    assert!(misnamed.exists(), "a refused temporary file is left alone");
    assert!(
        refusals.iter().all(|r| r == "EINVAL: unsupported medium\x1b\\"),
        "{refusals:?}"
    );
    std::fs::remove_dir_all(&dir).unwrap();
}

#[test]
fn a_checkpoint_keeps_a_terminal_holding_images_live() {
    let mut em = card();
    transmit(&mut em, 1, "");
    em.checkpoint().unwrap();
    assert_eq!(placed(&em).len(), 1);
    assert!(em.images());
    feed(&mut em, &apc("a=d,d=A,q=2", ""));
    assert_eq!(em.image_bytes(), 0);
    em.set_image_files(true);
    em.checkpoint().unwrap();
    assert!(em.images(), "the switch carries across a rebuild");
    assert!(em.image_files(), "so does the one for files");
}

#[test]
fn virtual_placements_resolve_their_grid_from_the_cell_size() {
    let mut em = card();
    transmit(&mut em, 9, ",U=1,p=4");
    let (mut p, mut v) = (Vec::new(), Vec::new());
    em.placements(&mut p, &mut v);
    assert!(p.is_empty());
    assert_eq!(v.len(), 1);
    assert_eq!(
        (v[0].image, v[0].placement, v[0].cols, v[0].rows),
        (9, 4, 2, 2)
    );
}

#[test]
fn size_reports_follow_the_cell_pixels_at_any_scale() {
    let mut em = Emulator::new(80, 24).unwrap();
    // A 2x display: the same 80×24 card in twice the pixels.
    for (scale, w, h) in [(1, 8, 17), (2, 16, 34)] {
        em.resize_px(80, 24, 80 * w, 24 * h, &mut Vec::new())
            .unwrap();
        let replies = feed(&mut em, b"\x1b[14t\x1b[16t\x1b[18t");
        assert_eq!(
            replies,
            [
                format!("\x1b[4;{};{}t", 24 * h, 80 * w),
                format!("\x1b[6;{h};{w}t"),
                "\x1b[8;24;80t".to_owned(),
            ],
            "scale {scale}"
        );
    }
    // A resize to fewer cells in the same window keeps the reply in step.
    em.resize_px(100, 30, 1600, 1020, &mut Vec::new()).unwrap();
    let replies = feed(&mut em, b"\x1b[14t\x1b[16t\x1b[18t");
    assert_eq!(
        replies,
        ["\x1b[4;1020;1600t", "\x1b[6;34;16t", "\x1b[8;30;100t"]
    );
}

#[test]
fn many_images_stay_within_the_storage_limit() {
    let mut em = card();
    // 400 images of 100×100 RGBA, 16 MB sent: the oldest are evicted past the 10 MB a screen keeps.
    let data = b64(&rgba(100, 100, [5, 6, 7, 255]));
    let start = std::time::Instant::now();
    for id in 1..=400u32 {
        feed(
            &mut em,
            &apc(&format!("a=T,f=32,s=100,v=100,i={id},q=2"), &data),
        );
        feed(&mut em, b"\r\n");
    }
    let held = em.image_bytes();
    eprintln!(
        "400 images of 40 kB: {} kB held, {:?} to parse",
        held / 1000,
        start.elapsed()
    );
    assert!(held <= 10 << 20, "{held} bytes held");
    assert!(held >= 100 * 100 * 4);
}

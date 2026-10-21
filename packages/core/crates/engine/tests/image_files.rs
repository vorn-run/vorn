//! A session's programs load kitty images from files only once the host says
//! the session runs on this machine; sent in the stream, images load
//! everywhere, and a refused file reads the same as any other refusal.

use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Arc;
use std::time::{Duration, Instant};

mod common;

use common::Sessiond;
use vorn_engine::{Config, Input, Local, Open, Out, Session};
use vorn_recovery::{LogBuilder, Size};
use vorn_term_proto::Cursor;

const SIZE: (u16, u16) = (20, 4);

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

/// What the program is sent back, in order.
fn writes(outs: &[Out]) -> Vec<String> {
    outs.iter()
        .filter_map(|o| match o {
            Out::Write(w) => Some(String::from_utf8_lossy(w).into_owned()),
            _ => None,
        })
        .collect()
}

/// A live session over `sequences`, upto `keep` of them applied, and what
/// it wrote back.
fn run(local: Option<Local>, sequences: &[String], keep: u64) -> (Sessiond, Session, Vec<String>) {
    let mut b = LogBuilder::new(Size::new(SIZE.0, SIZE.1));
    for seq in sequences {
        b.data(seq.as_str());
    }
    let mut d = Sessiond::new(0, SIZE);
    d.append(&b.build().entries);
    let cfg = Arc::new(Config {
        local,
        ..Config::default()
    });
    let mut first = Vec::new();
    let mut s = Session::open(
        "s",
        cfg,
        Open::spawned(Cursor::start(0), Some(SIZE)),
        Instant::now(),
        &mut first,
    );
    let mut seen = Vec::new();
    d.serve(&mut s, first, &mut seen, keep.checked_sub(1));
    (d, s, writes(&seen))
}

fn file_image(name: &str) -> (std::path::PathBuf, String) {
    let dir = std::env::temp_dir().join(format!("vorn-{name}-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let file = dir.join("img.rgba");
    std::fs::write(&file, [5u8, 6, 7, 255].repeat(4)).unwrap();
    let path = b64(file.to_str().unwrap().as_bytes());
    (dir, path)
}

#[test]
fn only_a_session_running_here_loads_images_from_files() {
    let (dir, path) = file_image("engine-kitty-here");
    let file = format!("\x1b_Ga=t,t=f,f=32,s=2,v=2,i=1;{path}\x1b\\");
    let direct = format!(
        "\x1b_Ga=t,f=32,s=2,v=2,i=2;{}\x1b\\",
        b64(&[1, 2, 3, 255].repeat(4))
    );
    let both = [file, direct];
    let refused = "\x1b_Gi=1;EINVAL: unsupported medium\x1b\\";

    let remote = Local(Arc::new(|_: &str| Some(false)));
    let (_, _, w) = run(Some(remote), &both, 2);
    assert_eq!(w, [refused, "\x1b_Gi=2;OK\x1b\\"]);

    let local = Local(Arc::new(|_: &str| Some(true)));
    assert_eq!(run(Some(local), &both[..1], 1).2, ["\x1b_Gi=1;OK\x1b\\"]);

    // A host that says nothing lets no session load files.
    assert_eq!(run(None, &both[..1], 1).2, [refused]);
    std::fs::remove_dir_all(&dir).unwrap();
}

#[test]
fn a_session_not_placed_yet_loads_no_files_until_the_host_knows() {
    let (dir, path) = file_image("engine-kitty-later");
    let file = |id: u32| format!("\x1b_Ga=t,t=f,f=32,s=2,v=2,i={id};{path}\x1b\\");
    let asked = Arc::new(AtomicUsize::new(0));
    let count = Arc::clone(&asked);
    let local = Local(Arc::new(move |id: &str| {
        assert_eq!(id, "s");
        (count.fetch_add(1, Ordering::SeqCst) > 0).then_some(true)
    }));
    let (mut d, mut s, w) = run(Some(local), &[file(1), file(2)], 1);
    assert_eq!(w, ["\x1b_Gi=1;EINVAL: unsupported medium\x1b\\"]);
    let rest: Vec<_> = d.records().into_iter().skip(1).collect();
    let mut out = Vec::new();
    s.input(
        Input::Entries(rest),
        Instant::now() + Duration::from_secs(2),
        &mut out,
    );
    assert_eq!(writes(&out), ["\x1b_Gi=2;OK\x1b\\"]);
    assert_eq!(asked.load(Ordering::SeqCst), 2);
    std::fs::remove_dir_all(&dir).unwrap();
}

#[test]
fn asks_again_once_the_answer_is_old_and_never_for_images_sent_in_the_stream() {
    let (dir, path) = file_image("engine-kitty-again");
    let file = |id: u32| format!("\x1b_Ga=t,t=f,f=32,s=2,v=2,i={id};{path}\x1b\\");
    let direct = format!(
        "\x1b_Ga=t,f=32,s=2,v=2,i=3;{}\x1b\\",
        b64(&[1, 2, 3, 255].repeat(4))
    );
    let asked = Arc::new(AtomicUsize::new(0));
    let count = Arc::clone(&asked);
    // Here at first; a remote login started under it later.
    let local = Local(Arc::new(move |_: &str| {
        Some(count.fetch_add(1, Ordering::SeqCst) == 0)
    }));
    let (mut d, mut s, w) = run(Some(local), &[direct, file(1), file(2)], 1);
    assert_eq!(w, ["\x1b_Gi=3;OK\x1b\\"]);
    assert_eq!(
        asked.load(Ordering::SeqCst),
        0,
        "a direct image asks nothing"
    );

    let records = d.records();
    let mut out = Vec::new();
    let start = Instant::now();
    s.input(Input::Entries(records[1..2].to_vec()), start, &mut out);
    assert_eq!(writes(&out), ["\x1b_Gi=1;OK\x1b\\"]);
    out.clear();
    s.input(
        Input::Entries(records[2..].to_vec()),
        start + Duration::from_secs(2),
        &mut out,
    );
    assert_eq!(writes(&out), ["\x1b_Gi=2;EINVAL: unsupported medium\x1b\\"]);
    assert_eq!(asked.load(Ordering::SeqCst), 2);
    std::fs::remove_dir_all(&dir).unwrap();
}

#[test]
fn a_readable_file_holding_the_wrong_data_reads_as_any_refusal() {
    let dir = std::env::temp_dir().join(format!("vorn-engine-kitty-bad-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let short = dir.join("short.rgba");
    std::fs::write(&short, [1u8, 2, 3]).unwrap();
    let packed = dir.join("packed.rgba");
    std::fs::write(&packed, b"not zlib at all").unwrap();
    let path = |p: &std::path::Path| b64(p.to_str().unwrap().as_bytes());
    let sequences = [
        format!("\x1b_Ga=t,t=f,f=32,s=2,v=2,i=1;{}\x1b\\", path(&short)),
        format!("\x1b_Ga=t,t=f,f=32,o=z,s=2,v=2,i=2;{}\x1b\\", path(&packed)),
        format!("\x1b_Ga=t,t=f,f=100,i=3;{}\x1b\\", path(&packed)),
    ];
    let local = Local(Arc::new(|_: &str| Some(true)));
    let (_, _, w) = run(Some(local), &sequences, 3);
    assert_eq!(
        w,
        [1, 2, 3].map(|i| format!("\x1b_Gi={i};EINVAL: unsupported medium\x1b\\"))
    );
    std::fs::remove_dir_all(&dir).unwrap();
}

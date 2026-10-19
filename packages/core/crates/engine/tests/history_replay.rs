//! Replays a session's history log (`VORN_HISTORY_LOG`, a vornd
//! `history/<id>.log` file) through the engine, record by record, checking
//! the grid client's screen against the emulator at every step and printing
//! the screen around each resize, so a garbled live session can be followed.
//!
//! With `VORN_HISTORY_CLEAN` set it also checks the end is clean: the
//! program started its screen over after the last resize, and what the
//! session shows is what that repaint and the output after it draw on a
//! fresh terminal of the final size. A frame drawn for another size, a row
//! clamped to the bottom or columns written over one another all differ.

mod gridrig;

use gridrig::{assert_same, Rig, SCROLLBACK};
use vorn_term_mirror::row_text;
use vorn_term_proto::{Entry, Record, RecordHeader};

/// What starts a screen over: ED 2 and the alternate screen's entry.
const REPAINTS: [&[u8]; 2] = [b"\x1b[2J", b"\x1b[?1049h"];

/// Where the last repaint in `bytes` starts.
fn last_repaint(bytes: &[u8]) -> Option<usize> {
    REPAINTS
        .iter()
        .filter_map(|p| bytes.windows(p.len()).rposition(|w| w == *p))
        .max()
}

/// `bytes` drawn on a fresh terminal of `size`, on the alternate screen if
/// `alt`, as rows of text.
fn drawn_fresh(bytes: &[u8], (cols, rows): (u16, u16), alt: bool) -> String {
    let mut em = vorn_screen::Emulator::new(u32::from(cols), u32::from(rows)).expect("a terminal");
    let mut fx = Vec::new();
    if alt {
        em.feed(b"\x1b[?1049h", &mut fx);
    }
    em.feed(bytes, &mut fx);
    let mut out = Vec::new();
    em.rows_text(0, rows.saturating_sub(1), &mut out);
    out.join("\n").trim_end().to_owned()
}

fn screen(m: &vorn_term_mirror::Mirror) -> String {
    m.rows()
        .iter()
        .map(|r| row_text(r).trim_end().to_owned())
        .collect::<Vec<_>>()
        .join("\n")
}

#[test]
fn a_history_log_replays_cleanly() {
    let Ok(path) = std::env::var("VORN_HISTORY_LOG") else {
        eprintln!("skipped: VORN_HISTORY_LOG names no log");
        return;
    };
    // Segments in order, separated by ':' (`<name>.log.1:<name>.log`).
    let mut frames = Vec::new();
    let mut start = None;
    for file in path.split(':') {
        let bytes = std::fs::read(file).expect("the log reads");
        let (_, at, part, _) = vorn_pipeline::history::parse(&bytes).expect("a history log");
        start.get_or_insert(at);
        frames.extend(part);
    }
    let start = start.expect("one segment");
    let size = std::env::var("VORN_HISTORY_SIZE")
        .ok()
        .and_then(|s| {
            let (c, r) = s.split_once('x')?;
            Some((c.parse().ok()?, r.parse().ok()?))
        })
        .unwrap_or((80u16, 24u16));
    let mut rig = Rig::new(size, SCROLLBACK);
    rig.attach(1, 1, true, None);
    let around = std::env::var("VORN_HISTORY_AROUND")
        .ok()
        .and_then(|n| n.parse::<usize>().ok())
        .unwrap_or(2);
    let mut show_until = 0usize;
    let n = frames.len();
    // Records fed per update, as a live engine folds many into one.
    let batch: usize = std::env::var("VORN_HISTORY_BATCH")
        .ok()
        .and_then(|b| b.parse().ok())
        .unwrap_or(1);
    let mut pending: Vec<Entry> = Vec::new();
    // The output with a marker at each resize, as the capture fixtures hold it.
    let mut raw = std::env::var_os("VORN_HISTORY_RAW").map(|p| (p, Vec::new()));
    // The output since the last resize, for the clean check.
    let mut since_resize: Vec<u8> = Vec::new();
    for (i, f) in frames.into_iter().enumerate() {
        match &f.record {
            Record::Data { bytes, .. } => since_resize.extend_from_slice(bytes),
            Record::Resize { .. } => since_resize.clear(),
            _ => {}
        }
        if let Some((_, out)) = &mut raw {
            match &f.record {
                Record::Data { bytes, .. } => out.extend_from_slice(bytes),
                Record::Resize { cols, rows, .. } => {
                    out.extend_from_slice(format!("\n<<<RESIZE {cols}x{rows}>>>\n").as_bytes())
                }
                _ => {}
            }
        }
        let is_resize = matches!(f.record, Record::Resize { .. });
        if let Record::Resize { cols, rows, .. } = f.record {
            eprintln!(
                "### frame {i}: resize {cols}x{rows} at offset {}",
                f.start_offset
            );
            show_until = i + around;
        }
        if std::env::var_os("VORN_HISTORY_DUMP").is_some() && i <= show_until && !is_resize {
            if let Record::Data { bytes, .. } = &f.record {
                let shown: String = String::from_utf8_lossy(&bytes[..bytes.len().min(700)])
                    .replace('\x1b', "\\e")
                    .replace('\r', "\\r")
                    .replace('\n', "\\n\n");
                eprintln!("### frame {i}: data {} bytes:\n{shown}", bytes.len());
            }
        }
        pending.push(Entry {
            hdr: RecordHeader {
                epoch: start.epoch,
                rseq: f.rseq,
                start_offset: f.start_offset,
            },
            at_ns: 0,
            rec: f.record,
        });
        if pending.len() < batch && i + 1 < n {
            continue;
        }
        rig.feed(&std::mem::take(&mut pending));
        rig.settle();
        let m = rig.mirror(1, 1);
        assert_same(rig.em(), m, &format!("frame {i}"));
        if is_resize || i <= show_until || i + 1 == n {
            let t = m.term();
            eprintln!(
                "--- after frame {i} ({}x{}, {:?}, {} history):\n{}",
                t.cols,
                t.rows,
                t.screen,
                t.history_lines,
                screen(m)
            );
        }
    }
    if let Some((path, out)) = raw {
        std::fs::write(path, out).expect("the raw output writes");
    }
    if std::env::var_os("VORN_HISTORY_CLEAN").is_none() {
        return;
    }
    let m = rig.mirror(1, 1);
    let t = m.term();
    let Some(at) = last_repaint(&since_resize) else {
        panic!(
            "the program never started its screen over after the last resize ({}x{}):\n{}",
            t.cols,
            t.rows,
            screen(m)
        );
    };
    let alt = t.screen == vorn_term_proto::screen::Screen::Alternate;
    let want = drawn_fresh(&since_resize[at..], (t.cols, t.rows), alt);
    let got: Vec<String> = screen(m).lines().map(str::to_owned).collect();
    let got = got.join("\n").trim_end().to_owned();
    assert_eq!(
        got, want,
        "the final screen is not the program's last repaint at {}x{}",
        t.cols, t.rows
    );
}

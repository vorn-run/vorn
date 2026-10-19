//! Replays a session's history log (`VORN_HISTORY_LOG`, a vornd
//! `history/<id>.log` file) through the engine, record by record, checking
//! the grid client's screen against the emulator at every step and printing
//! the screen around each resize, so a garbled live session can be followed.

mod gridrig;

use gridrig::{assert_same, Rig, SCROLLBACK};
use vorn_term_mirror::row_text;
use vorn_term_proto::{Entry, Record, RecordHeader};

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
    for (i, f) in frames.into_iter().enumerate() {
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
                "--- after frame {i} ({}x{}):\n{}",
                t.cols,
                t.rows,
                screen(m)
            );
        }
    }
}

//! A full-screen agent's output around resizes, replayed from a capture
//! (`fixtures/claude-2.1.303-resize.bin`, Claude Code 2.1.303 in a plain
//! PTY at 120x40, resized to 100x40, 80x30 and back to 120x40): the screen
//! every grid client holds matches a terminal fed the same bytes, and reads
//! as the program drew it, at every size.

mod gridrig;

use gridrig::{assert_same, Rig, SCROLLBACK};
use vorn_engine::GridIn;
use vorn_recovery::{LogBuilder, Size};
use vorn_term_mirror::row_text;

const CAPTURE: &[u8] = include_bytes!("fixtures/claude-2.1.303-resize.bin");

/// Bytes, then the size the program was resized to after them, if it was.
type Piece = (Vec<u8>, Option<(u16, u16)>);

/// The capture's pieces: the bytes before each resize marker, and the size
/// the marker names.
fn pieces() -> Vec<Piece> {
    let mut out = Vec::new();
    let mut rest = CAPTURE;
    loop {
        let Some(at) = rest.windows(10).position(|w| w == b"\n<<<RESIZE") else {
            out.push((rest.to_vec(), None));
            return out;
        };
        let (before, after) = rest.split_at(at);
        let after = &after[10..];
        let end = after.iter().position(|&b| b == b'\n').expect("marker ends");
        let marker = std::str::from_utf8(&after[..end]).expect("ascii marker");
        let (c, r) = marker
            .trim()
            .trim_end_matches(">>>")
            .split_once('x')
            .expect("cols x rows");
        out.push((
            before.to_vec(),
            Some((c.parse().expect("cols"), r.parse().expect("rows"))),
        ));
        rest = &after[end + 1..];
    }
}

fn screen(m: &vorn_term_mirror::Mirror) -> String {
    m.rows()
        .iter()
        .map(|r| row_text(r).trim_end().to_owned())
        .collect::<Vec<_>>()
        .join("\n")
}

#[test]
fn a_full_screen_agents_resizes_replay_exactly() {
    let mut b = LogBuilder::new(Size::new(120, 40));
    for (bytes, size) in pieces() {
        b.data(bytes);
        if let Some((cols, rows)) = size {
            b.resize(Size::new(cols, rows));
        }
    }
    let entries = b.build().entries;
    let mut rig = Rig::new((120, 40), SCROLLBACK);
    rig.attach(1, 1, true, None);
    for (i, e) in entries.iter().enumerate() {
        rig.feed(std::slice::from_ref(e));
        rig.settle();
        let m = rig.mirror(1, 1);
        assert_same(rig.em(), m, &format!("entry {i}"));
        let t = rig.mirror(1, 1).term().clone();
        rig.grid(GridIn::FetchHistory {
            peer: vorn_engine::Peer { conn: 1, sid: 1 },
            req: 1,
            sb_epoch: t.sb_epoch,
            from_line: t.top_line - t.history_lines,
            count: t.history_lines as u16,
        });
        rig.settle();
        // The repaint after each resize reads as the program drew it: the
        // model line on its own row above the input box, and the rules
        // across the whole width. The last piece also holds the exit.
        let repainted = matches!(
            entries.get(i - 1).map(|p| &p.rec),
            Some(vorn_term_proto::Record::Resize { .. })
        ) && i + 1 < entries.len();
        if repainted {
            let text = screen(rig.mirror(1, 1));
            assert!(
                text.lines().any(|l| l.trim_end().ends_with("/effort")),
                "entry {i}: the model line is on its own row:\n{text}"
            );
            let rules = text
                .lines()
                .filter(|l| l.chars().all(|c| c == '─') && !l.is_empty())
                .count();
            assert_eq!(rules, 2, "entry {i}: two rules span the width:\n{text}");
            assert!(
                text.lines()
                    .all(|l| l.chars().count() <= usize::from(t.cols)),
                "entry {i}: no row is wider than {}:\n{text}",
                t.cols
            );
        }
    }
}

/// Output the program drew for the old size can already sit in the PTY when
/// it is resized, so it is recorded after the Resize and parsed at the new
/// size, as any terminal would. On the alternate screen the program's
/// repaint (clear, then every row) must leave nothing of it behind.
#[test]
fn old_width_output_after_a_resize_is_cleared_by_the_repaint() {
    let pieces = pieces();
    let (first, _) = &pieces[0];
    let (repaint_100, _) = &pieces[1];
    let mut b = LogBuilder::new(Size::new(120, 40));
    b.data(first.clone());
    b.resize(Size::new(100, 40));
    // The 120-column frame again: in flight when the PTY shrank.
    b.data(first.clone());
    b.data(repaint_100.clone());
    let entries = b.build().entries;
    let mut rig = Rig::new((120, 40), SCROLLBACK);
    rig.attach(1, 1, true, None);
    rig.feed(&entries);
    rig.settle();
    let m = rig.mirror(1, 1);
    assert_same(rig.em(), m, "after the repaint");
    let text = screen(m);
    assert!(
        text.lines().all(|l| l.chars().count() <= 100),
        "no row is wider than 100:\n{text}"
    );
    let rules = text
        .lines()
        .filter(|l| !l.is_empty() && l.chars().all(|c| c == '─'))
        .count();
    assert_eq!(rules, 2, "two rules span the width:\n{text}");
    assert!(
        text.lines().any(|l| l.trim_end().ends_with("/effort")),
        "the model line is on its own row:\n{text}"
    );
}

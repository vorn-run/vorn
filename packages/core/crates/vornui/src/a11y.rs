//! The accessibility side of text fields: AccessKit wants a field's text
//! as `TextRun` children, one per visual line, with each character's
//! length, position and width, so a screen reader can read by character,
//! word and line and follow the caret. Characters here are grapheme
//! clusters, the unit the caret moves by.

use accesskit::{Node, NodeId, Rect as AkRect, Role, TextPosition, TextSelection};
use unicode_segmentation::UnicodeSegmentation;

use crate::element::Id;
use crate::text::Shaped;

/// The node of line `line` of field `field`.
pub(crate) fn run_id(field: Id, line: usize) -> NodeId {
    NodeId(field.child(("run", line)).0)
}

pub(crate) fn run_start(sh: &Shaped, line: usize) -> usize {
    sh.lines.get(line).map_or(0, |l| l.start)
}

/// Where line `line`'s run ends: where the next starts, so the runs cover
/// the text, newlines and wrap spaces included.
pub(crate) fn run_end(sh: &Shaped, line: usize, len: usize) -> usize {
    sh.lines
        .get(line + 1)
        .map_or(len, |l| l.start)
        .max(run_start(sh, line))
}

fn units(s: &str) -> impl Iterator<Item = (usize, &str)> {
    s.grapheme_indices(true)
}

/// The byte offset of character `index` in `run`.
pub(crate) fn char_to_byte(run: &str, index: usize) -> usize {
    units(run).nth(index).map_or(run.len(), |(i, _)| i)
}

fn byte_to_char(run: &str, byte: usize) -> usize {
    units(run).take_while(|(i, _)| *i < byte).count()
}

/// The runs of a field showing `text` shaped as `sh` with its top-left at
/// `origin` (physical pixels), and the caret as a selection on them.
pub(crate) fn text_runs(
    field: Id,
    text: &str,
    sh: &Shaped,
    origin: (f32, f32),
    (anchor, caret): (usize, usize),
) -> (Vec<(NodeId, Node)>, TextSelection) {
    let lines = sh.lines.len().max(1);
    let mut nodes = Vec::with_capacity(lines);
    let pos = |b: usize| -> TextPosition {
        let line = (0..lines)
            .find(|&l| b < run_end(sh, l, text.len()))
            .unwrap_or(lines - 1);
        let start = run_start(sh, line).min(text.len());
        let end = run_end(sh, line, text.len()).min(text.len());
        TextPosition {
            node: run_id(field, line),
            character_index: byte_to_char(&text[start..end], b.saturating_sub(start)),
        }
    };
    let selection = TextSelection {
        anchor: pos(anchor),
        focus: pos(caret),
    };
    for line in 0..lines {
        let start = run_start(sh, line).min(text.len());
        let end = run_end(sh, line, text.len()).min(text.len());
        let run = &text[start..end];
        let x0 = sh.caret(start).0;
        let mut lens = Vec::new();
        let mut xs = Vec::new();
        let mut ws = Vec::new();
        let mut words = Vec::new();
        let word_bytes: Vec<usize> = run
            .split_word_bound_indices()
            .filter(|(_, w)| w.chars().any(char::is_alphanumeric))
            .map(|(i, _)| i)
            .collect();
        for (n, (i, g)) in units(run).enumerate() {
            // AccessKit counts lengths in u8; a cluster longer than that is
            // not text anyone types, so it is cut rather than refused.
            lens.push(u8::try_from(g.len()).unwrap_or(u8::MAX));
            let (a, la) = sh.caret(start + i);
            let (b, lb) = sh.caret(start + i + g.len());
            xs.push((a - x0).max(0.0));
            ws.push(if la == lb { (b - a).max(0.0) } else { 0.0 });
            if word_bytes.contains(&i) {
                words.push(u8::try_from(n).unwrap_or(u8::MAX));
            }
        }
        let mut node = Node::new(Role::TextRun);
        if lens.iter().map(|l| usize::from(*l)).sum::<usize>() == run.len() {
            node.set_value(run);
            node.set_character_lengths(lens);
            node.set_character_positions(xs);
            node.set_character_widths(ws);
            node.set_word_starts(words);
        }
        if let Some(l) = sh.lines.get(line) {
            node.set_bounds(AkRect::new(
                f64::from(origin.0 + x0),
                f64::from(origin.1 + l.top),
                f64::from(origin.0 + x0 + l.w),
                f64::from(origin.1 + l.top + l.height),
            ));
        }
        nodes.push((run_id(field, line), node));
    }
    (nodes, selection)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn chars_are_grapheme_clusters() {
        let run = "ae\u{301}🇯🇵!";
        assert_eq!(char_to_byte(run, 0), 0);
        assert_eq!(char_to_byte(run, 2), 4);
        assert_eq!(char_to_byte(run, 3), 12);
        assert_eq!(char_to_byte(run, 9), run.len());
        assert_eq!(byte_to_char(run, 12), 3);
    }
}

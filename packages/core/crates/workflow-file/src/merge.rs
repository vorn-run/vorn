//! A workflow's changes written into the file it came from: whatever did
//! not change keeps its text as the person left it, comments and layout
//! included; what changed is written fresh in its place; what is new comes
//! where the workflow has it.
//!
//! Each node of the old file is matched to the new one by what it is (a
//! field by its name, a step by its id, a config key by its key, a list's
//! item by its place) and kept when it says the same as before, judged by
//! writing both from the workflow they read as. A step that changed keeps
//! the config keys that did not, and the comments before them.

use kdl::{KdlDocument, KdlNode};
use serde_json::Value;

use crate::read::read;
use crate::write::write;
use crate::Bindings;

/// `old`, a workflow file, changed to say `definition`. A file that does not
/// read is written fresh.
pub fn update(old: &str, definition: &Value, bindings: &Bindings<'_>) -> String {
    let fresh = write(definition, bindings);
    let Ok(was) = read(old, bindings) else {
        return fresh;
    };
    let canon = write(&was.workflow, bindings);
    let (Ok(o), Ok(c), Ok(n)) = (
        KdlDocument::parse_v2(old),
        KdlDocument::parse_v2(&canon),
        KdlDocument::parse_v2(&fresh),
    ) else {
        return fresh;
    };
    let (Some(ow), Some(cw), Some(nw)) = (o.nodes().first(), c.nodes().first(), n.nodes().first())
    else {
        return fresh;
    };
    let m = Merge {
        old,
        canon: &canon,
        fresh: &fresh,
    };
    let (start, end) = range(ow);
    let mut out = String::with_capacity(old.len() + fresh.len() / 4);
    out.push_str(&old[..start]);
    out.push_str(&m.node(ow, cw, nw));
    out.push_str(&old[end..]);
    out
}

/// Where a node's text starts and ends: its name to its block's close.
fn range(node: &KdlNode) -> (usize, usize) {
    let span = node.span();
    (span.offset(), span.offset() + span.len())
}

/// Where a node's block opens and closes, `{` and `}`, when it has one.
fn braces(src: &str, node: &KdlNode) -> Option<(usize, usize)> {
    node.children()?;
    let (start, end) = range(node);
    let after = node
        .entries()
        .iter()
        .map(|e| e.span().offset() + e.span().len())
        .max()
        .unwrap_or(start + node.name().span().len());
    let open = after + src[after..end].find('{')?;
    let close = src[..end].rfind('}')?;
    (open < close).then_some((open, close))
}

/// What a node is, to find it again: its name and id, or its place in a list.
fn key(node: &KdlNode, index: usize) -> (String, String) {
    let name = node.name().value().to_owned();
    if name == "-" {
        return (name, index.to_string());
    }
    let id = node
        .entry("id")
        .and_then(|e| e.value().as_string())
        .map(str::to_owned)
        .unwrap_or_default();
    (name, id)
}

struct Merge<'a> {
    old: &'a str,
    canon: &'a str,
    fresh: &'a str,
}

fn text<'s>(src: &'s str, node: &KdlNode) -> &'s str {
    let (a, b) = range(node);
    &src[a..b]
}

impl Merge<'_> {
    /// New node `n`, kept as old node `o` where it says what `c` (`o` as
    /// written fresh) says, or with its block merged.
    fn node(&self, o: &KdlNode, c: &KdlNode, n: &KdlNode) -> String {
        if text(self.canon, c) == text(self.fresh, n) {
            return text(self.old, o).to_owned();
        }
        let (Some((oo, oc)), Some((co, _)), Some((no, nc))) = (
            braces(self.old, o),
            braces(self.canon, c),
            braces(self.fresh, n),
        ) else {
            return text(self.fresh, n).to_owned();
        };
        let (os, cs, ns) = (range(o).0, range(c).0, range(n).0);
        let mut out = String::new();
        // The node's own line: the old one where it says the same.
        if self.canon[cs..co] == self.fresh[ns..no] {
            out.push_str(&self.old[os..=oo]);
        } else {
            out.push_str(&self.fresh[ns..=no]);
        }
        let (Some(ob), Some(cb), Some(nb)) = (o.children(), c.children(), n.children()) else {
            return text(self.fresh, n).to_owned();
        };
        if ob.nodes().is_empty() || nb.nodes().is_empty() {
            out.push_str(&self.fresh[no + 1..nc]);
        } else {
            out.push_str(&self.block(ob, cb, nb, oo + 1, oc));
        }
        out.push('}');
        out
    }

    /// The inside of a block, between `{` at `open - 1` and `}` at `close`:
    /// each new child as the old child it matches, after the comments that
    /// came before that child, or written fresh on a line of its own.
    fn block(
        &self,
        o: &KdlDocument,
        c: &KdlDocument,
        n: &KdlDocument,
        open: usize,
        close: usize,
    ) -> String {
        let olds: Vec<(String, String)> = o
            .nodes()
            .iter()
            .enumerate()
            .map(|(i, x)| key(x, i))
            .collect();
        let canons: Vec<(String, String)> = c
            .nodes()
            .iter()
            .enumerate()
            .map(|(i, x)| key(x, i))
            .collect();
        let indent = {
            let first = range(&n.nodes()[0]).0;
            let line = self.fresh[..first].rfind('\n').map_or(0, |i| i + 1);
            self.fresh[line..first].to_owned()
        };
        let mut out = String::new();
        for (i, nn) in n.nodes().iter().enumerate() {
            let k = key(nn, i);
            let old_at = olds.iter().position(|x| *x == k);
            let canon_at = canons.iter().position(|x| *x == k);
            match (old_at, canon_at) {
                (Some(oi), Some(ci)) => {
                    let gap_start = if oi == 0 {
                        open
                    } else {
                        range(&o.nodes()[oi - 1]).1
                    };
                    let on = &o.nodes()[oi];
                    out.push_str(&self.old[gap_start..range(on).0]);
                    out.push_str(&self.node(on, &c.nodes()[ci], nn));
                }
                _ => {
                    out.push('\n');
                    out.push_str(&indent);
                    out.push_str(text(self.fresh, nn));
                }
            }
        }
        let last = o.nodes().last().map_or(open, |x| range(x).1);
        out.push_str(&self.old[last..close]);
        out
    }
}

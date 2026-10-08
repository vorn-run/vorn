//! What vornd relies on, probed per engine with small fixed inputs: a
//! snapshot that rebuilds the screen, the alternate screen, history by
//! line, query replies, OSC 8 links and the title.

use vt_api::Engine;

use crate::corpus;
use crate::json::Obj;
use crate::{play, SCROLLBACK};

/// The queries a shell or full-screen program sends and waits on.
pub const QUERIES: &[(&str, &[u8])] = &[
    ("cursor position (CSI 6n)", b"\x1b[6n"),
    ("status (CSI 5n)", b"\x1b[5n"),
    ("primary DA (CSI c)", b"\x1b[c"),
    ("secondary DA (CSI > c)", b"\x1b[>c"),
    ("DECRQM bracketed paste (CSI ? 2004 $ p)", b"\x1b[?2004$p"),
    ("XTVERSION (CSI > q)", b"\x1b[>q"),
    ("background colour (OSC 11 ?)", b"\x1b]11;?\x1b\\"),
    ("kitty keyboard flags (CSI ? u)", b"\x1b[?u"),
];

/// Printable form of a reply, escapes spelled out.
pub fn visible(bytes: &[u8]) -> String {
    bytes.escape_ascii().to_string()
}

fn alt_screen<E: Engine>() -> bool {
    let mut e = E::new(20, 4, SCROLLBACK);
    e.feed(b"primary\x1b[?1049h\x1b[Halt");
    let entered = e.alt_screen() && e.grid().row_text(0) == "alt";
    e.feed(b"\x1b[?1049l");
    entered && !e.alt_screen() && e.grid().row_text(0) == "primary"
}

/// Whether history line `n` reads back as the `n`th line written, with and
/// without the cap evicting the oldest lines.
fn history<E: Engine>() -> (bool, usize) {
    let lines = |n: usize| -> Vec<u8> {
        (0..n)
            .map(|i| format!("L{i}"))
            .collect::<Vec<_>>()
            .join("\r\n")
            .into_bytes()
    };
    let mut e = E::new(20, 5, 100);
    e.feed(&lines(30));
    let ok = e.scrollback_lines() == 25
        && e.history_line(0).as_deref() == Some("L0")
        && e.history_line(24).as_deref() == Some("L24")
        && e.history_line(25).is_none();
    // Past the cap: which line is now the oldest?
    let mut e = E::new(20, 5, 100);
    e.feed(&lines(1000));
    let kept = e.scrollback_lines();
    let oldest_ok = e.history_line(0) == Some(format!("L{}", 995 - kept));
    (ok && oldest_ok, kept)
}

fn link<E: Engine>() -> bool {
    let mut e = E::new(20, 2, SCROLLBACK);
    e.feed(b"\x1b]8;;https://example.com/\x1b\\link\x1b]8;;\x1b\\ x");
    let g = e.grid();
    let row = g.row(0);
    row[0].link.as_deref() == Some("https://example.com/") && row[5].link.is_none()
}

fn titles<E: Engine>() -> (bool, bool) {
    let mut e = E::new(20, 2, SCROLLBACK);
    e.feed(b"\x1b]0;zero\x07");
    let osc0 = e.title().as_deref() == Some("zero");
    e.feed(b"\x1b]2;two\x1b\\");
    (osc0, e.title().as_deref() == Some("two"))
}

/// Feeds each corpus, rebuilds a fresh engine from the snapshot, and says
/// per corpus whether the grid and cursor came back, or why there was no
/// snapshot to rebuild from.
fn snapshot<E: Engine>() -> (Obj, usize) {
    let mut out = Obj::new();
    let mut largest = 0;
    for name in corpus::NAMES {
        let c = corpus::load_once(name, 256 << 10);
        let mut e = E::new(c.cols, c.rows, SCROLLBACK);
        play(&mut e, &c.ops);
        let verdict = match e.rebuild() {
            Ok((f, len)) => {
                largest = largest.max(len);
                let (a, b) = (e.cursor(), f.cursor());
                if e.grid() == f.grid() && (a.x, a.y) == (b.x, b.y) {
                    "same"
                } else {
                    "differs"
                }
            }
            Err(why) => why,
        };
        out = out.str(name, verdict);
    }
    (out, largest)
}

pub fn probe<E: Engine>() -> String {
    let mut o = Obj::new()
        .str("engine", E::NAME)
        .bool("alt_screen", alt_screen::<E>());
    let (hist, kept) = history::<E>();
    o = o
        .bool("history_by_line", hist)
        .num("history_kept_at_cap_100", kept as f64)
        .bool("osc8_links", link::<E>());
    let (osc0, osc2) = titles::<E>();
    o = o.bool("title_osc0", osc0).bool("title_osc2", osc2);
    let (snap, largest) = snapshot::<E>();
    o = o
        .raw("snapshot", &snap.to_string())
        .num("snapshot_max_bytes", largest as f64);
    let mut replies = Obj::new();
    for (name, q) in QUERIES {
        let mut e = E::new(20, 4, SCROLLBACK);
        e.feed(b"ab");
        e.feed(q);
        replies = replies.str(name, &visible(&e.take_replies()));
    }
    o.raw("replies", &replies.to_string()).to_string()
}

#[cfg(test)]
mod tests {
    use super::*;
    use vt_ghostty::Ghostty;

    #[test]
    fn the_baseline_passes_every_probe() {
        assert!(alt_screen::<Ghostty>());
        assert!(history::<Ghostty>().0);
        assert!(link::<Ghostty>());
        assert_eq!(titles::<Ghostty>(), (true, true));
        let snap = snapshot::<Ghostty>().0.to_string();
        assert!(!snap.contains("differs"), "{snap}");
        assert!(snap.contains(r#""agent":"same""#), "{snap}");
    }

    #[test]
    fn replies_print_their_escapes() {
        assert_eq!(visible(b"\x1b[1;3R"), "\\x1b[1;3R");
    }
}

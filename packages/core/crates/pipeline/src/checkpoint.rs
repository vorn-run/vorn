//! A checkpoint's file body, as `packages/server/src/history/checkpoint.ts`
//! reads it: one JSON object holding the screen, the scrollback behind it and
//! where in the record stream they end.
//!
//! Built here because the scrollback is a quarter of a megabyte of escape
//! sequences, and `JSON.stringify` over it -- every ESC becoming six characters
//! -- held the event loop for up to three milliseconds per checkpoint, which a
//! busy terminal takes several of a second.

use vorn_screen::Snapshot;

/// The Session Recovery Contract's cursor: the first record and byte a state
/// does not include.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Cursor {
    pub epoch: u32,
    pub next_rseq: u64,
    pub next_offset: u64,
}

/// What the writer knows about a checkpoint that the stream does not.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Meta {
    pub generation: u32,
    pub resume: Cursor,
    /// Written by a shutdown rather than by the clock.
    pub closed_cleanly: bool,
}

pub fn encode(snapshot: &Snapshot, scrollback: &str, meta: Meta) -> Vec<u8> {
    let mut out = Vec::with_capacity(snapshot.screen.len() + scrollback.len() * 2 + 256);
    out.extend_from_slice(b"{\"screen\":");
    string(&mut out, &snapshot.screen);
    out.extend_from_slice(b",\"scrollback\":");
    string(&mut out, scrollback);
    out.extend_from_slice(
        format!(
            ",\"cols\":{},\"rows\":{},\"title\":",
            snapshot.cols, snapshot.rows
        )
        .as_bytes(),
    );
    string(&mut out, &snapshot.title);
    out.extend_from_slice(b",\"cwd\":");
    string(&mut out, &snapshot.cwd);
    out.extend_from_slice(
        format!(
            ",\"generation\":{},\"resume\":{{\"epoch\":{},\"nextRseq\":{},\"nextOffset\":{}}}",
            meta.generation, meta.resume.epoch, meta.resume.next_rseq, meta.resume.next_offset
        )
        .as_bytes(),
    );
    if meta.closed_cleanly {
        out.extend_from_slice(b",\"closedCleanly\":true");
    }
    out.push(b'}');
    out
}

/// A JSON string. Only `"`, `\` and control characters need escaping; the rest
/// of a Rust string is valid UTF-8 and goes through as it is.
fn string(out: &mut Vec<u8>, s: &str) {
    const HEX: &[u8; 16] = b"0123456789abcdef";
    out.push(b'"');
    let bytes = s.as_bytes();
    let mut start = 0;
    for (i, &b) in bytes.iter().enumerate() {
        let escape: &[u8] = match b {
            b'"' => b"\\\"",
            b'\\' => b"\\\\",
            b'\n' => b"\\n",
            b'\r' => b"\\r",
            b'\t' => b"\\t",
            0x00..=0x1f => b"",
            _ => continue,
        };
        out.extend_from_slice(&bytes[start..i]);
        if escape.is_empty() {
            out.extend_from_slice(&[
                b'\\',
                b'u',
                b'0',
                b'0',
                HEX[(b >> 4) as usize],
                HEX[(b & 0xf) as usize],
            ]);
        } else {
            out.extend_from_slice(escape);
        }
        start = i + 1;
    }
    out.extend_from_slice(&bytes[start..]);
    out.push(b'"');
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::{json, Value};

    fn snapshot(screen: &str) -> Snapshot {
        Snapshot {
            screen: screen.into(),
            cols: 120,
            rows: 40,
            title: "a \"quoted\" title".into(),
            cwd: "/tmp/日本".into(),
        }
    }

    const META: Meta = Meta {
        generation: 3,
        resume: Cursor {
            epoch: 0xfffffffe,
            next_rseq: 1 << 40,
            next_offset: (1 << 52) + 3,
        },
        closed_cleanly: false,
    };

    #[test]
    fn is_the_object_checkpoint_ts_reads() {
        let screen = "\x1b[31mred\x1b[0m\r\n\ttab \\ back \u{1}\u{7f} 🙂";
        let body = encode(&snapshot(screen), "scroll\x1b[K\u{0}back", META);
        let value: Value = serde_json::from_slice(&body).unwrap();
        assert_eq!(
            value,
            json!({
                "screen": screen,
                "scrollback": "scroll\x1b[K\u{0}back",
                "cols": 120,
                "rows": 40,
                "title": "a \"quoted\" title",
                "cwd": "/tmp/日本",
                "generation": 3,
                "resume": { "epoch": 0xfffffffeu32, "nextRseq": 1u64 << 40, "nextOffset": (1u64 << 52) + 3 }
            })
        );
    }

    #[test]
    fn says_it_closed_cleanly_only_when_it_did() {
        let clean = Meta {
            closed_cleanly: true,
            ..META
        };
        let value: Value = serde_json::from_slice(&encode(&snapshot(""), "", clean)).unwrap();
        assert_eq!(value["closedCleanly"], json!(true));
        let value: Value = serde_json::from_slice(&encode(&snapshot(""), "", META)).unwrap();
        assert!(value.get("closedCleanly").is_none());
    }

    #[test]
    fn escapes_every_control_character() {
        let all: String = (0u8..0x20).map(char::from).collect();
        let body = encode(&snapshot(&all), "", META);
        let value: Value = serde_json::from_slice(&body).unwrap();
        assert_eq!(value["screen"], json!(all));
    }
}

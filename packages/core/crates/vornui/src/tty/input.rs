//! What a terminal sends, read from raw bytes: keys (legacy and the kitty
//! keyboard protocol), SGR mouse reports in cells or pixels, bracketed
//! paste, focus, in-band resizes and the answers to the queries that find
//! out what the terminal can do. Bytes may arrive split anywhere; an
//! unfinished sequence waits for the rest.

use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};

/// The longest unfinished control sequence kept waiting for its end;
/// past it the bytes are not one and are dropped.
const MAX_PENDING: usize = 64 * 1024;

/// One thing read.
#[derive(Debug, Clone, PartialEq)]
pub enum Parsed {
    Key(KeyEvent),
    Mouse(MouseReport),
    Paste(String),
    Focus(bool),
    Reply(Reply),
}

/// An SGR mouse report: the button code as sent, and where, in cells
/// (1-based) or, with mode 1016, in pixels from the grid's top-left (which
/// may be negative over the padding).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct MouseReport {
    pub code: u16,
    pub x: i32,
    pub y: i32,
    pub release: bool,
}

/// What a mouse report means, apart from where it is.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MouseAction {
    Down(u8),
    Up(u8),
    Move,
    /// Wheel notches; positive scrolls down.
    Wheel(i8),
}

impl MouseReport {
    /// The action, or `None` for one vornui has no use for (horizontal
    /// wheels).
    pub fn action(self) -> Option<MouseAction> {
        let button = (self.code & 0b11) as u8;
        if self.code & 64 != 0 {
            return match button {
                0 => Some(MouseAction::Wheel(-1)),
                1 => Some(MouseAction::Wheel(1)),
                _ => None,
            };
        }
        if self.code & 32 != 0 {
            return Some(MouseAction::Move);
        }
        Some(if self.release {
            MouseAction::Up(button)
        } else {
            MouseAction::Down(button)
        })
    }
}

/// An answer to a query.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Reply {
    /// A graphics command's answer: `ok` unless the terminal said why not.
    Graphics { id: u32, ok: bool },
    /// The primary device attributes, which every terminal answers: the
    /// fence after the other queries.
    DeviceAttributes,
    /// A cell's size in pixels, `(w, h)`.
    CellSize(u32, u32),
    /// The text area's size in pixels, `(w, h)`.
    TextArea(u32, u32),
    /// Whether private mode `mode` is known (1, 2 set or reset; 3, 4
    /// permanently) or not (0).
    Mode { mode: u32, state: u32 },
    /// The kitty keyboard protocol's flags in force.
    KeyboardFlags(u32),
    /// The window's new size, sent unasked with mode 2048.
    Resize {
        cols: u16,
        rows: u16,
        w: u32,
        h: u32,
    },
}

/// Reads events out of the bytes a terminal sends.
#[derive(Debug, Default)]
pub struct Parser {
    buf: Vec<u8>,
    /// Pasted bytes so far, between the paste's start and end.
    paste: Option<Vec<u8>>,
}

impl Parser {
    /// Reads what `bytes` complete, leaving an unfinished sequence for the
    /// next call.
    pub fn feed(&mut self, bytes: &[u8], out: &mut Vec<Parsed>) {
        self.buf.extend_from_slice(bytes);
        let mut at = 0;
        while at < self.buf.len() {
            if let Some(paste) = &mut self.paste {
                const END: &[u8] = b"\x1b[201~";
                let rest = &self.buf[at..];
                match find(rest, END) {
                    Some(n) => {
                        paste.extend_from_slice(&rest[..n]);
                        let text = String::from_utf8_lossy(paste).into_owned();
                        self.paste = None;
                        out.push(Parsed::Paste(text));
                        at += n + END.len();
                        continue;
                    }
                    None => {
                        // Keeps what could be the start of the end marker.
                        let keep = (1..END.len())
                            .rev()
                            .find(|k| rest.ends_with(&END[..*k]))
                            .unwrap_or(0);
                        paste.extend_from_slice(&rest[..rest.len() - keep]);
                        at = self.buf.len() - keep;
                        break;
                    }
                }
            }
            match self.one(at, out) {
                Some(n) => at += n,
                None => break,
            }
        }
        self.buf.drain(..at);
        if self.buf.len() > MAX_PENDING {
            self.buf.clear();
        }
    }

    /// What is left once no more bytes come: a lone ESC is the Escape key,
    /// and ESC with `[` or `O` the Alt chords that start like a sequence.
    pub fn flush(&mut self, out: &mut Vec<Parsed>) {
        let key = match self.buf.as_slice() {
            [0x1b] => Some(KeyEvent::new(KeyCode::Esc, KeyModifiers::NONE)),
            [0x1b, b'['] => Some(KeyEvent::new(KeyCode::Char('['), KeyModifiers::ALT)),
            [0x1b, b'O'] => Some(KeyEvent::new(KeyCode::Char('O'), KeyModifiers::ALT)),
            _ => None,
        };
        if let Some(k) = key {
            self.buf.clear();
            out.push(Parsed::Key(k));
        }
    }

    /// Whether bytes wait for more.
    pub fn pending(&self) -> bool {
        !self.buf.is_empty()
    }

    /// Reads one event at `at`; answers how many bytes it took, or `None`
    /// when they end before it does.
    fn one(&mut self, at: usize, out: &mut Vec<Parsed>) -> Option<usize> {
        let b = &self.buf[at..];
        if b[0] != 0x1b {
            let (key, n) = legacy(b)?;
            out.extend(key.map(Parsed::Key));
            return Some(n);
        }
        let &next = b.get(1)?;
        match next {
            b'[' => {
                let end = b[2..].iter().position(|c| (0x40..=0x7e).contains(c));
                let Some(end) = end else {
                    return too_long(b);
                };
                let params = &b[2..2 + end];
                let fin = b[2 + end];
                if fin == b'~' && params == b"200" {
                    self.paste = Some(Vec::new());
                } else {
                    out.extend(csi(params, fin));
                }
                Some(3 + end)
            }
            b'O' => {
                let fin = *b.get(2)?;
                out.extend(ss3(fin).map(|c| Parsed::Key(KeyEvent::new(c, KeyModifiers::NONE))));
                Some(3)
            }
            b'_' | b'P' | b']' => {
                // A string ends at ST (ESC \), or BEL for OSC.
                let body = &b[2..];
                let st = find(body, b"\x1b\\").map(|n| (n, 2));
                let bel = (next == b']')
                    .then(|| body.iter().position(|c| *c == 0x07).map(|n| (n, 1)))
                    .flatten();
                let Some((n, term)) = [st, bel].into_iter().flatten().min() else {
                    return too_long(b);
                };
                if next == b'_' {
                    out.extend(apc(&body[..n]).map(Parsed::Reply));
                }
                Some(2 + n + term)
            }
            0x1b => {
                out.push(Parsed::Key(KeyEvent::new(KeyCode::Esc, KeyModifiers::ALT)));
                Some(2)
            }
            _ => {
                let (key, n) = legacy(&b[1..])?;
                out.extend(key.map(|mut k| {
                    k.modifiers |= KeyModifiers::ALT;
                    Parsed::Key(k)
                }));
                Some(1 + n)
            }
        }
    }
}

/// `None` (wait for more) unless the sequence has run past any real one,
/// when its ESC is dropped.
fn too_long(b: &[u8]) -> Option<usize> {
    (b.len() > MAX_PENDING).then_some(1)
}

fn find(hay: &[u8], needle: &[u8]) -> Option<usize> {
    hay.windows(needle.len()).position(|w| w == needle)
}

/// A key typed without escape: a character or a control byte. Answers
/// the key (`None` for a byte that is none) and its length, or `None`
/// when a character's bytes are not all there.
fn legacy(b: &[u8]) -> Option<(Option<KeyEvent>, usize)> {
    let ctrl = |c: char| Some(KeyEvent::new(KeyCode::Char(c), KeyModifiers::CONTROL));
    let plain = |c: KeyCode| Some(KeyEvent::new(c, KeyModifiers::NONE));
    let key = match b[0] {
        b'\r' => plain(KeyCode::Enter),
        b'\t' => plain(KeyCode::Tab),
        0x7f | 0x08 => plain(KeyCode::Backspace),
        0x00 => ctrl(' '),
        c @ 0x01..=0x1a => ctrl(char::from(b'a' + c - 1)),
        0x1c => ctrl('\\'),
        0x1d => ctrl(']'),
        0x1e => ctrl('^'),
        0x1f => ctrl('/'),
        0x1b => plain(KeyCode::Esc),
        c if c < 0x80 => plain(KeyCode::Char(char::from(c))),
        c => {
            let len = match c {
                0xc0..=0xdf => 2,
                0xe0..=0xef => 3,
                0xf0..=0xf7 => 4,
                _ => return Some((None, 1)),
            };
            if b.len() < len {
                return None;
            }
            return Some(match std::str::from_utf8(&b[..len]) {
                Ok(s) => (s.chars().next().and_then(|c| plain(KeyCode::Char(c))), len),
                Err(_) => (None, 1),
            });
        }
    };
    Some((key, 1))
}

fn ss3(fin: u8) -> Option<KeyCode> {
    Some(match fin {
        b'A' => KeyCode::Up,
        b'B' => KeyCode::Down,
        b'C' => KeyCode::Right,
        b'D' => KeyCode::Left,
        b'H' => KeyCode::Home,
        b'F' => KeyCode::End,
        b'P' => KeyCode::F(1),
        b'Q' => KeyCode::F(2),
        b'R' => KeyCode::F(3),
        b'S' => KeyCode::F(4),
        _ => return None,
    })
}

/// The modifiers a `1 + bits` parameter names.
fn modifiers(param: Option<u32>) -> KeyModifiers {
    let bits = param.unwrap_or(1).saturating_sub(1);
    let mut m = KeyModifiers::NONE;
    for (bit, flag) in [
        (1, KeyModifiers::SHIFT),
        (2, KeyModifiers::ALT),
        (4, KeyModifiers::CONTROL),
        (8, KeyModifiers::SUPER),
        (16, KeyModifiers::HYPER),
        (32, KeyModifiers::META),
    ] {
        if bits & bit != 0 {
            m |= flag;
        }
    }
    m
}

/// A parameter's numbers: `a:b:c` as `[a, b, c]`, an empty one as `None`.
fn subparams(p: &str) -> Vec<Option<u32>> {
    p.split(':').map(|n| n.parse().ok()).collect()
}

fn csi(params: &[u8], fin: u8) -> Option<Parsed> {
    let s = std::str::from_utf8(params).ok()?;
    if let Some(rest) = s.strip_prefix('<') {
        if fin != b'M' && fin != b'm' {
            return None;
        }
        let mut n = rest.split(';');
        let code = n.next()?.parse().ok()?;
        let x = n.next()?.parse().ok()?;
        let y = n.next()?.parse().ok()?;
        return Some(Parsed::Mouse(MouseReport {
            code,
            x,
            y,
            release: fin == b'm',
        }));
    }
    if let Some(rest) = s.strip_prefix('?') {
        return match fin {
            b'u' => Some(Parsed::Reply(Reply::KeyboardFlags(rest.parse().ok()?))),
            b'c' => Some(Parsed::Reply(Reply::DeviceAttributes)),
            b'y' => {
                let (mode, state) = rest.strip_suffix('$')?.split_once(';')?;
                Some(Parsed::Reply(Reply::Mode {
                    mode: mode.parse().ok()?,
                    state: state.parse().ok()?,
                }))
            }
            _ => None,
        };
    }
    let nums: Vec<&str> = s.split(';').collect();
    let num = |i: usize| nums.get(i).and_then(|n| n.parse::<u32>().ok());
    let key = |code: KeyCode, m: Option<u32>| Some(Parsed::Key(KeyEvent::new(code, modifiers(m))));
    match fin {
        b't' => {
            let reply = match num(0)? {
                4 => Reply::TextArea(num(2)?, num(1)?),
                6 => Reply::CellSize(num(2)?, num(1)?),
                48 => Reply::Resize {
                    rows: u16::try_from(num(1)?).ok()?,
                    cols: u16::try_from(num(2)?).ok()?,
                    h: num(3)?,
                    w: num(4)?,
                },
                _ => return None,
            };
            Some(Parsed::Reply(reply))
        }
        b'I' if s.is_empty() => Some(Parsed::Focus(true)),
        b'O' if s.is_empty() => Some(Parsed::Focus(false)),
        b'Z' => {
            let m = modifiers(num(1)) | KeyModifiers::SHIFT;
            Some(Parsed::Key(KeyEvent::new(KeyCode::BackTab, m)))
        }
        b'~' => {
            let code = match num(0)? {
                1 | 7 => KeyCode::Home,
                2 => KeyCode::Insert,
                3 => KeyCode::Delete,
                4 | 8 => KeyCode::End,
                5 => KeyCode::PageUp,
                6 => KeyCode::PageDown,
                n @ 11..=15 => KeyCode::F((n - 10) as u8),
                n @ 17..=21 => KeyCode::F((n - 11) as u8),
                n @ 23..=24 => KeyCode::F((n - 12) as u8),
                _ => return None,
            };
            key(code, num(1))
        }
        b'u' => kitty_key(&nums),
        _ => key(ss3(fin)?, num(1)),
    }
}

/// A key in the kitty keyboard protocol: `code[:shifted];mods[:event]`.
fn kitty_key(nums: &[&str]) -> Option<Parsed> {
    let codes = subparams(nums.first()?);
    let mods = nums.get(1).map(|m| subparams(m)).unwrap_or_default();
    if mods.get(1).copied().flatten() == Some(3) {
        return None;
    }
    let m = modifiers(mods.first().copied().flatten());
    let code = codes.first().copied().flatten()?;
    let shifted = codes.get(1).copied().flatten();
    let key = match code {
        27 => KeyCode::Esc,
        13 | 57414 => KeyCode::Enter,
        9 => KeyCode::Tab,
        127 => KeyCode::Backspace,
        // The keypad, media and modifier keys alone.
        57344..=63743 => return None,
        _ => {
            let c = match shifted {
                Some(s) if m.contains(KeyModifiers::SHIFT) => s,
                _ => code,
            };
            KeyCode::Char(char::from_u32(c)?)
        }
    };
    Some(Parsed::Key(KeyEvent::new(key, m)))
}

/// A graphics answer: `G` keys, `;`, then `OK` or why not.
fn apc(body: &[u8]) -> Option<Reply> {
    let s = std::str::from_utf8(body.strip_prefix(b"G")?).ok()?;
    let (keys, msg) = s.split_once(';')?;
    let id = keys
        .split(',')
        .find_map(|kv| kv.strip_prefix("i="))?
        .parse()
        .ok()?;
    Some(Reply::Graphics {
        id,
        ok: msg == "OK",
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn parse(bytes: &[u8]) -> Vec<Parsed> {
        let mut p = Parser::default();
        let mut out = Vec::new();
        p.feed(bytes, &mut out);
        p.flush(&mut out);
        out
    }

    fn key(code: KeyCode, m: KeyModifiers) -> Parsed {
        Parsed::Key(KeyEvent::new(code, m))
    }

    #[test]
    fn reads_typed_characters_and_control_bytes() {
        assert_eq!(
            parse("aé\r\x03\x1c\x7f".as_bytes()),
            vec![
                key(KeyCode::Char('a'), KeyModifiers::NONE),
                key(KeyCode::Char('é'), KeyModifiers::NONE),
                key(KeyCode::Enter, KeyModifiers::NONE),
                key(KeyCode::Char('c'), KeyModifiers::CONTROL),
                key(KeyCode::Char('\\'), KeyModifiers::CONTROL),
                key(KeyCode::Backspace, KeyModifiers::NONE),
            ]
        );
    }

    #[test]
    fn escape_alone_alt_chords_and_split_sequences() {
        assert_eq!(parse(b"\x1b"), vec![key(KeyCode::Esc, KeyModifiers::NONE)]);
        assert_eq!(
            parse(b"\x1bb"),
            vec![key(KeyCode::Char('b'), KeyModifiers::ALT)]
        );
        assert_eq!(
            parse(b"\x1b["),
            vec![key(KeyCode::Char('['), KeyModifiers::ALT)]
        );
        let mut p = Parser::default();
        let mut out = Vec::new();
        p.feed(b"\x1b[1;", &mut out);
        assert!(out.is_empty() && p.pending(), "waits for the rest");
        p.feed(b"5A", &mut out);
        assert_eq!(out, vec![key(KeyCode::Up, KeyModifiers::CONTROL)]);
        assert!(!p.pending());
    }

    #[test]
    fn reads_named_keys_with_modifiers() {
        assert_eq!(
            parse(b"\x1b[3~\x1b[5;2~\x1bOP\x1b[Z\x1b[15~\x1b[1;3H"),
            vec![
                key(KeyCode::Delete, KeyModifiers::NONE),
                key(KeyCode::PageUp, KeyModifiers::SHIFT),
                key(KeyCode::F(1), KeyModifiers::NONE),
                key(KeyCode::BackTab, KeyModifiers::SHIFT),
                key(KeyCode::F(5), KeyModifiers::NONE),
                key(KeyCode::Home, KeyModifiers::ALT),
            ]
        );
    }

    #[test]
    fn reads_kitty_keyboard_keys() {
        assert_eq!(
            parse(b"\x1b[99;5u\x1b[97:65;6u\x1b[27u\x1b[91;3u\x1b[57441;2u\x1b[97;1:3u"),
            vec![
                key(KeyCode::Char('c'), KeyModifiers::CONTROL),
                key(
                    KeyCode::Char('A'),
                    KeyModifiers::CONTROL | KeyModifiers::SHIFT
                ),
                key(KeyCode::Esc, KeyModifiers::NONE),
                key(KeyCode::Char('['), KeyModifiers::ALT),
            ],
            "a lone modifier key and a release are dropped"
        );
    }

    #[test]
    fn reads_mouse_reports_in_cells_and_pixels() {
        let got = parse(b"\x1b[<0;10;5M\x1b[<0;-3;812m\x1b[<35;640;400M\x1b[<65;1;1M");
        let reports: Vec<MouseReport> = got
            .into_iter()
            .map(|p| match p {
                Parsed::Mouse(m) => m,
                other => panic!("not a mouse report: {other:?}"),
            })
            .collect();
        assert_eq!(
            reports[0],
            MouseReport {
                code: 0,
                x: 10,
                y: 5,
                release: false
            }
        );
        assert_eq!((reports[1].x, reports[1].y), (-3, 812));
        assert_eq!(reports[0].action(), Some(MouseAction::Down(0)));
        assert_eq!(reports[1].action(), Some(MouseAction::Up(0)));
        assert_eq!(reports[2].action(), Some(MouseAction::Move));
        assert_eq!(reports[3].action(), Some(MouseAction::Wheel(1)));
        assert_eq!(
            MouseReport {
                code: 66,
                x: 0,
                y: 0,
                release: false
            }
            .action(),
            None
        );
    }

    #[test]
    fn reads_a_paste_split_across_reads() {
        let mut p = Parser::default();
        let mut out = Vec::new();
        p.feed(b"\x1b[200~hello \x1b[", &mut out);
        assert!(out.is_empty());
        p.feed(b"A wor", &mut out);
        p.feed(b"ld\x1b[20", &mut out);
        p.feed(b"1~x", &mut out);
        assert_eq!(
            out,
            vec![
                Parsed::Paste("hello \x1b[A world".into()),
                key(KeyCode::Char('x'), KeyModifiers::NONE)
            ]
        );
    }

    #[test]
    fn reads_the_answers_to_queries() {
        let got = parse(
            b"\x1b_Gi=31;OK\x1b\\\x1b_Gi=32;ENOENT:no such file\x1b\\\x1b[6;34;16t\
              \x1b[4;1360;1920t\x1b[?1016;2$y\x1b[?2048;0$y\x1b[?1u\x1b[48;40;120;1360;1920t\
              \x1b[I\x1b[O\x1b[?62;22;52c",
        );
        assert_eq!(
            got,
            vec![
                Parsed::Reply(Reply::Graphics { id: 31, ok: true }),
                Parsed::Reply(Reply::Graphics { id: 32, ok: false }),
                Parsed::Reply(Reply::CellSize(16, 34)),
                Parsed::Reply(Reply::TextArea(1920, 1360)),
                Parsed::Reply(Reply::Mode {
                    mode: 1016,
                    state: 2
                }),
                Parsed::Reply(Reply::Mode {
                    mode: 2048,
                    state: 0
                }),
                Parsed::Reply(Reply::KeyboardFlags(1)),
                Parsed::Reply(Reply::Resize {
                    cols: 120,
                    rows: 40,
                    w: 1920,
                    h: 1360
                }),
                Parsed::Focus(true),
                Parsed::Focus(false),
                Parsed::Reply(Reply::DeviceAttributes),
            ]
        );
    }

    #[test]
    fn strings_it_does_not_know_are_skipped() {
        assert_eq!(
            parse(b"\x1b]11;rgb:0/0/0\x07a\x1bP>|kitty\x1b\\b"),
            vec![
                key(KeyCode::Char('a'), KeyModifiers::NONE),
                key(KeyCode::Char('b'), KeyModifiers::NONE)
            ]
        );
    }

    #[test]
    fn garbage_never_panics() {
        let mut p = Parser::default();
        let mut out = Vec::new();
        let mut seed = 0x2545_f491_4f6c_dd1du64;
        for _ in 0..2000 {
            let mut bytes = Vec::new();
            for _ in 0..(seed % 17) {
                seed ^= seed << 13;
                seed ^= seed >> 7;
                seed ^= seed << 17;
                let pick = b"\x1b[<;0123456789~uMm?$ytOP_G\\\x07\xc3\xa9\xffa";
                bytes.push(pick[(seed % pick.len() as u64) as usize]);
            }
            p.feed(&bytes, &mut out);
        }
        p.flush(&mut out);
    }
}

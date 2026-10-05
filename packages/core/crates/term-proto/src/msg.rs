//! Grid mode's messages and their framing (Terminal State Protocol §7): what
//! a grid client and vornd say to each other over a local socket.
//!
//! A frame is a 4-byte little-endian length, a 1-byte kind and a CBOR
//! payload; the length counts the kind byte and the payload, as on
//! sessiond's socket. Payloads are CBOR maps keyed by small integers, so a
//! field can be added without breaking an older reader, which skips keys it
//! does not know. Kinds with the high bit set are optional: a reader that
//! does not know one ignores it. Any other unknown kind is an error.
//!
//! The field numbers of each message are listed on its type. The screen
//! types reuse [`crate::screen`]; `TermState` keeps the numbers TP §6 gives
//! it. Two details are this crate's, where the design leaves them open:
//!
//! - a session is named by sessiond's id string, not a UUID, because that is
//!   the name every other part of Vorn uses for it;
//! - `History` carries the style and link definitions past the client's
//!   marks (keys 7 and 8), because history rows are encoded when they are
//!   fetched and may use styles minted since the client's last frame.
//!
//! Size policy (`Viewport`, `Presence`, `TakeSize`, `LockSize`) and
//! `SetDefaults` have kinds reserved here and decode to
//! [`ClientMsg::Unhandled`]; the bytes mode's messages are the WebSocket's.

use std::fmt;

use crate::cbor::{self, CborError, Fields, Value, Writer};
use crate::position::Cursor;
use crate::screen::{
    Color, ColorsDelta, CursorState, CursorStyle, Delta, LinkDef, MouseMode, Row, Screen, Snapshot,
    StyleDef, TermDelta, TermState, Underline,
};

/// The protocol major this build speaks. vornd also accepts the previous one
/// (TP §13), so an app one release ahead or behind still attaches.
pub const PROTO_MAJOR: u16 = 1;
pub const PROTO_MINOR: u16 = 0;

/// The largest frame either side accepts: a snapshot of a large viewport
/// whose every cell holds a long cluster, with its history tail, fits.
pub const MAX_FRAME: usize = 64 << 20;

/// Whether a peer speaking `major` can be served: this build's major or the
/// one before it.
pub fn major_supported(major: u16) -> bool {
    major == PROTO_MAJOR || major.checked_add(1) == Some(PROTO_MAJOR)
}

/// Capability bits of `Hello` and `Welcome`; both sides use the
/// intersection.
pub mod caps {
    /// Grid mode itself.
    pub const GRID: u64 = 1 << 0;
    /// `History` carries table definitions (keys 7 and 8).
    pub const HISTORY_TABLES: u64 = 1 << 1;
    /// Reserved for Kitty graphics placements.
    pub const IMAGES: u64 = 1 << 63;
    /// What this build implements.
    pub const ALL: u64 = GRID | HISTORY_TABLES;
}

/// Message kinds. Client to vornd below 0x20, vornd to client from 0x21.
pub mod kind {
    pub const HELLO: u8 = 0x01;
    pub const ATTACH: u8 = 0x02;
    pub const DETACH: u8 = 0x03;
    pub const SET_VISIBLE: u8 = 0x04;
    pub const ACK: u8 = 0x05;
    pub const INPUT: u8 = 0x06;
    pub const VIEWPORT: u8 = 0x07;
    pub const PRESENCE: u8 = 0x08;
    pub const TAKE_SIZE: u8 = 0x09;
    pub const LOCK_SIZE: u8 = 0x0a;
    pub const FETCH_HISTORY: u8 = 0x0b;
    pub const SELECT_AT: u8 = 0x0c;
    pub const COPY: u8 = 0x0d;
    pub const SEARCH: u8 = 0x0e;
    pub const SET_DEFAULTS: u8 = 0x0f;

    pub const WELCOME: u8 = 0x21;
    pub const ATTACHED: u8 = 0x22;
    pub const SNAPSHOT: u8 = 0x23;
    pub const DELTA: u8 = 0x24;
    pub const RESET_TABLES: u8 = 0x25;
    pub const BYTES: u8 = 0x26;
    pub const VT_SNAPSHOT: u8 = 0x27;
    pub const RESIZED: u8 = 0x28;
    pub const INPUT_ACK: u8 = 0x29;
    pub const PASTE_CONFIRM: u8 = 0x2a;
    pub const HISTORY: u8 = 0x2b;
    pub const SELECTION: u8 = 0x2c;
    pub const COPIED: u8 = 0x2d;
    pub const SEARCH_HITS: u8 = 0x2e;
    pub const EVENT: u8 = 0x2f;
    pub const RESYNC: u8 = 0x30;
    pub const ERROR: u8 = 0x31;

    /// Kinds with this bit set may be ignored by a reader that does not
    /// know them.
    pub const OPTIONAL: u8 = 0x80;
}

/// Why a frame could not be read. Each one closes the connection.
#[derive(Debug, Clone, PartialEq)]
pub enum MsgError {
    /// A frame longer than [`MAX_FRAME`], or with no kind byte.
    Frame(usize),
    Cbor(CborError),
    /// A kind this build does not know and may not ignore.
    UnknownKind(u8),
    /// The payload is not a map.
    NotAMap(u8),
    /// A required field is missing or of the wrong type.
    Missing {
        kind: u8,
        field: u64,
    },
    /// A field holds a value out of range.
    Invalid {
        kind: u8,
        field: u64,
    },
}

impl fmt::Display for MsgError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            MsgError::Frame(n) => write!(f, "bad frame length {n}"),
            MsgError::Cbor(e) => write!(f, "{e}"),
            MsgError::UnknownKind(k) => write!(f, "unknown message kind 0x{k:02x}"),
            MsgError::NotAMap(k) => write!(f, "message 0x{k:02x} is not a CBOR map"),
            MsgError::Missing { kind, field } => {
                write!(f, "message 0x{kind:02x} lacks field {field}")
            }
            MsgError::Invalid { kind, field } => {
                write!(f, "message 0x{kind:02x} has field {field} out of range")
            }
        }
    }
}

impl std::error::Error for MsgError {}

impl From<CborError> for MsgError {
    fn from(e: CborError) -> Self {
        MsgError::Cbor(e)
    }
}

// ---------------------------------------------------------------------------
// Client to vornd
// ---------------------------------------------------------------------------

/// What kind of client says hello. vornd decides who is a desktop from the
/// connection, never from this (TP §10).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum ClientKind {
    #[default]
    Native,
    Electron,
    Web,
    Cli,
    /// A kind a newer client names.
    Unknown,
}

/// `{1 proto_major, 2 proto_minor, 3 caps, 4 client, 5 build}`
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct Hello {
    pub proto_major: u16,
    pub proto_minor: u16,
    pub caps: u64,
    pub client: ClientKind,
    pub build: String,
}

/// Frames of the screen, or the output's bytes.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum AttachMode {
    #[default]
    Grid,
    Bytes,
}

/// `{1 cols, 2 rows, 3 px_w, 4 px_h}`
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct Size {
    pub cols: u16,
    pub rows: u16,
    pub px_w: u32,
    pub px_h: u32,
}

/// A grid client's resume token: `{1 state_gen, 2 rev, 3 table_gen,
/// 4 style_mark, 5 link_mark}`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct GridResume {
    pub state_gen: u64,
    pub rev: u64,
    pub table_gen: u32,
    pub style_mark: u32,
    pub link_mark: u32,
}

/// What a reconnecting client holds.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Resume {
    Grid(GridResume),
    /// `{6 cursor}`: an offset alone is never a resume token.
    Bytes(Cursor),
}

/// `{1 session, 2 mode, 3 view, 4 visible, 5 resume, 6 history_tail}`
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct Attach {
    pub session: String,
    pub mode: AttachMode,
    pub view: Size,
    /// False: events only, no frames.
    pub visible: bool,
    pub resume: Option<Resume>,
    /// Lines of scrollback to include in the snapshot.
    pub history_tail: u16,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum KeyAction {
    Press,
    Repeat,
    Release,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MouseAction {
    Press,
    Release,
    Motion,
}

/// Bits of an input event's `mods`.
pub mod mods {
    pub const SHIFT: u16 = 1 << 0;
    pub const ALT: u16 = 1 << 1;
    pub const CTRL: u16 = 1 << 2;
    pub const SUPER: u16 = 1 << 3;
    pub const CAPS_LOCK: u16 = 1 << 4;
    pub const NUM_LOCK: u16 = 1 << 5;
    /// The right-hand key of the modifier; only meaningful with it set.
    pub const SHIFT_RIGHT: u16 = 1 << 6;
    pub const ALT_RIGHT: u16 = 1 << 7;
    pub const CTRL_RIGHT: u16 = 1 << 8;
    pub const SUPER_RIGHT: u16 = 1 << 9;
}

/// A physical key, by its W3C UI Events `code` in [`KEY_NAMES`] order. A
/// code past the table is a key a newer client knows; vornd treats it as
/// `Unidentified`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub struct KeyCode(pub u16);

impl KeyCode {
    pub fn name(self) -> &'static str {
        KEY_NAMES
            .get(usize::from(self.0))
            .copied()
            .unwrap_or("Unidentified")
    }

    pub fn from_name(name: &str) -> Option<KeyCode> {
        KEY_NAMES
            .iter()
            .position(|n| *n == name)
            .and_then(|i| u16::try_from(i).ok())
            .map(KeyCode)
    }
}

/// The W3C UI Events key codes, in wire order.
#[rustfmt::skip]
pub const KEY_NAMES: [&str; 176] = [
    "Unidentified", "Backquote", "Backslash", "BracketLeft", "BracketRight", "Comma", "Digit0",
    "Digit1", "Digit2", "Digit3", "Digit4", "Digit5", "Digit6", "Digit7", "Digit8", "Digit9",
    "Equal", "IntlBackslash", "IntlRo", "IntlYen", "A", "B", "C", "D", "E", "F", "G", "H", "I",
    "J", "K", "L", "M", "N", "O", "P", "Q", "R", "S", "T", "U", "V", "W", "X", "Y", "Z", "Minus",
    "Period", "Quote", "Semicolon", "Slash", "AltLeft", "AltRight", "Backspace", "CapsLock",
    "ContextMenu", "ControlLeft", "ControlRight", "Enter", "MetaLeft", "MetaRight", "ShiftLeft",
    "ShiftRight", "Space", "Tab", "Convert", "KanaMode", "NonConvert", "Delete", "End", "Help",
    "Home", "Insert", "PageDown", "PageUp", "ArrowDown", "ArrowLeft", "ArrowRight", "ArrowUp",
    "NumLock", "Numpad0", "Numpad1", "Numpad2", "Numpad3", "Numpad4", "Numpad5", "Numpad6",
    "Numpad7", "Numpad8", "Numpad9", "NumpadAdd", "NumpadBackspace", "NumpadClear",
    "NumpadClearEntry", "NumpadComma", "NumpadDecimal", "NumpadDivide", "NumpadEnter",
    "NumpadEqual", "NumpadMemoryAdd", "NumpadMemoryClear", "NumpadMemoryRecall",
    "NumpadMemoryStore", "NumpadMemorySubtract", "NumpadMultiply", "NumpadParenLeft",
    "NumpadParenRight", "NumpadSubtract", "NumpadSeparator", "NumpadUp", "NumpadDown",
    "NumpadRight", "NumpadLeft", "NumpadBegin", "NumpadHome", "NumpadEnd", "NumpadInsert",
    "NumpadDelete", "NumpadPageUp", "NumpadPageDown", "Escape", "F1", "F2", "F3", "F4", "F5", "F6",
    "F7", "F8", "F9", "F10", "F11", "F12", "F13", "F14", "F15", "F16", "F17", "F18", "F19", "F20",
    "F21", "F22", "F23", "F24", "F25", "Fn", "FnLock", "PrintScreen", "ScrollLock", "Pause",
    "BrowserBack", "BrowserFavorites", "BrowserForward", "BrowserHome", "BrowserRefresh",
    "BrowserSearch", "BrowserStop", "Eject", "LaunchApp1", "LaunchApp2", "LaunchMail",
    "MediaPlayPause", "MediaSelect", "MediaStop", "MediaTrackNext", "MediaTrackPrevious", "Power",
    "Sleep", "AudioVolumeDown", "AudioVolumeMute", "AudioVolumeUp", "WakeUp", "Copy", "Cut",
    "Paste",
];

/// One input event, encoded by vornd against the modes in effect when it is
/// dequeued (TP §10). On the wire: `{1 type, ...}` with the fields of each
/// variant numbered from 2 in the order below.
///
/// Equality compares positions bit for bit, so it is an equivalence and
/// messages holding events can be `Eq`.
#[derive(Debug, Clone)]
pub enum InputEvent {
    Key {
        action: KeyAction,
        key: KeyCode,
        mods: u16,
        consumed_mods: u16,
        text: Option<String>,
        unshifted: Option<char>,
        composing: bool,
    },
    /// Committed IME text.
    Text {
        utf8: String,
    },
    Paste {
        utf8: String,
        confirmed: bool,
    },
    /// `x` and `y` in cells, fractional.
    Mouse {
        action: MouseAction,
        button: Option<u8>,
        mods: u16,
        x: f32,
        y: f32,
    },
    /// `x` and `y` in cells, where the pointer is: keys 5 and 6, added to
    /// TP §7's `Wheel` so a program tracking the mouse scrolls the pane
    /// under it.
    Wheel {
        dx: f32,
        dy: f32,
        mods: u16,
        x: f32,
        y: f32,
    },
    Focus {
        focused: bool,
    },
    /// Bytes as they are: what a bytes client sends.
    Raw {
        bytes: Vec<u8>,
    },
}

impl PartialEq for InputEvent {
    fn eq(&self, other: &Self) -> bool {
        use InputEvent::*;
        match (self, other) {
            (
                Key {
                    action: a,
                    key: k,
                    mods: m,
                    consumed_mods: c,
                    text: t,
                    unshifted: u,
                    composing: p,
                },
                Key {
                    action: a2,
                    key: k2,
                    mods: m2,
                    consumed_mods: c2,
                    text: t2,
                    unshifted: u2,
                    composing: p2,
                },
            ) => a == a2 && k == k2 && m == m2 && c == c2 && t == t2 && u == u2 && p == p2,
            (Text { utf8: a }, Text { utf8: b }) => a == b,
            (
                Paste {
                    utf8: a,
                    confirmed: c,
                },
                Paste {
                    utf8: b,
                    confirmed: d,
                },
            ) => a == b && c == d,
            (
                Mouse {
                    action: a,
                    button: b,
                    mods: m,
                    x,
                    y,
                },
                Mouse {
                    action: a2,
                    button: b2,
                    mods: m2,
                    x: x2,
                    y: y2,
                },
            ) => {
                a == a2
                    && b == b2
                    && m == m2
                    && x.to_bits() == x2.to_bits()
                    && y.to_bits() == y2.to_bits()
            }
            (
                Wheel { dx, dy, mods, x, y },
                Wheel {
                    dx: dx2,
                    dy: dy2,
                    mods: m2,
                    x: x2,
                    y: y2,
                },
            ) => {
                dx.to_bits() == dx2.to_bits()
                    && dy.to_bits() == dy2.to_bits()
                    && mods == m2
                    && x.to_bits() == x2.to_bits()
                    && y.to_bits() == y2.to_bits()
            }
            (Focus { focused: a }, Focus { focused: b }) => a == b,
            (Raw { bytes: a }, Raw { bytes: b }) => a == b,
            _ => false,
        }
    }
}

impl Eq for InputEvent {}

/// `{1 line, 2 col, 3 sb_epoch}`. On the alternate screen, which has no
/// line numbers, `line` is the viewport row.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct GridPoint {
    pub line: u64,
    pub col: u16,
    pub sb_epoch: u32,
}

/// What `SelectAt` selects around its point.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SelectKind {
    Word,
    Line,
    /// One command's output, by its OSC 133 marks.
    Output,
}

/// How `Copy` formats the text.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CopyFormat {
    Plain,
    Vt,
    Html,
}

/// Everything a grid client says to vornd.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ClientMsg {
    Hello(Hello),
    Attach(Attach),
    /// `{1 sid}`
    Detach {
        sid: u32,
    },
    /// `{1 sid, 2 visible}`
    SetVisible {
        sid: u32,
        visible: bool,
    },
    /// `{1 sid, 2 rev}`: returns one credit.
    Ack {
        sid: u32,
        rev: u64,
    },
    /// `{1 sid, 2 input_seq, 3 event}`
    Input {
        sid: u32,
        input_seq: u64,
        event: InputEvent,
    },
    /// `{1 sid, 2 req, 3 sb_epoch, 4 from_line, 5 count}`
    FetchHistory {
        sid: u32,
        req: u32,
        sb_epoch: u32,
        from_line: u64,
        count: u16,
    },
    /// `{1 sid, 2 req, 3 at, 4 kind}`
    SelectAt {
        sid: u32,
        req: u32,
        at: GridPoint,
        kind: SelectKind,
    },
    /// `{1 sid, 2 req, 3 from, 4 to, 5 rect, 6 format}`
    Copy {
        sid: u32,
        req: u32,
        from: GridPoint,
        to: GridPoint,
        rect: bool,
        format: CopyFormat,
    },
    /// `{1 sid, 2 req, 3 query, 4 regex, 5 case, 6 from_line}`
    Search {
        sid: u32,
        req: u32,
        query: String,
        regex: bool,
        case: bool,
        from_line: Option<u64>,
    },
    /// A kind this build reserves but does not act on yet.
    Unhandled(u8),
}

// ---------------------------------------------------------------------------
// vornd to client
// ---------------------------------------------------------------------------

/// `{1 proto_major, 2 proto_minor, 3 caps, 4 vornd_build, 5 vornd_instance}`
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct Welcome {
    pub proto_major: u16,
    pub proto_minor: u16,
    pub caps: u64,
    pub vornd_build: String,
    pub vornd_instance: u64,
}

/// Whether a session is what a vornd that never died would have (TP §12).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Fidelity {
    Exact,
    Approximate,
}

/// `{1 sid, 2 session, 3 epoch, 4 owner, 5 fidelity}`
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Attached {
    pub sid: u32,
    pub session: String,
    pub epoch: u32,
    pub owner: bool,
    pub fidelity: Fidelity,
}

/// An effect's name: the record that caused it and its place in that
/// record, the same after a replay. `{1 epoch, 2 rseq, 3 index}`
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct EventId {
    pub epoch: u32,
    pub rseq: u64,
    pub index: u32,
}

/// `{1 type, ...}`, fields numbered from 2.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum EventKind {
    Bell,
    Clipboard {
        text: String,
    },
    Notify {
        title: String,
        body: String,
    },
    Status {
        state: u32,
    },
    Exit {
        code: Option<i32>,
        signal: Option<i32>,
    },
}

/// Why a client gets a snapshot rather than a delta.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ResyncReason {
    Gap,
    TableCompacted,
    BaseMismatch,
    Restarted,
    NotRetained,
}

/// `{1 from, 2 to}`, both inclusive.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Hit {
    pub from: GridPoint,
    pub to: GridPoint,
}

/// Everything vornd says to a grid client.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ServerMsg {
    Welcome(Welcome),
    Attached(Attached),
    /// `{1 sid, 2 state_gen, 3 rev, 4 resume, 5 table_gen, 6 row_fmt,
    /// 7 term, 8 styles, 9 links, 10 rows, 11 history}`
    Snapshot {
        sid: u32,
        snap: Box<Snapshot>,
    },
    /// `{1 sid, 2 state_gen, 3 table_gen, 4 base_rev, 5 rev, 6 resume,
    /// 7 scrolled, 8 term, 9 styles, 10 links, 11 rows}`
    Delta {
        sid: u32,
        delta: Box<Delta>,
    },
    /// `{1 sid, 2 table_gen}`: drop every definition; a snapshot follows.
    ResetTables {
        sid: u32,
        table_gen: u32,
    },
    /// `{1 sid, 2 input_seq}`
    InputAck {
        sid: u32,
        input_seq: u64,
    },
    /// `{1 sid, 2 input_seq}`: the paste is held until resent confirmed.
    PasteConfirm {
        sid: u32,
        input_seq: u64,
    },
    /// `{1 sid, 2 req, 3 sb_epoch, 4 from_line, 5 rows, 6 oldest_line,
    /// 7 styles, 8 links}`
    History {
        sid: u32,
        req: u32,
        sb_epoch: u32,
        from_line: u64,
        rows: Vec<Row>,
        oldest_line: u64,
        styles: Vec<StyleDef>,
        links: Vec<LinkDef>,
    },
    /// `{1 sid, 2 req, 3 from, 4 to}`; no range when nothing is there.
    Selection {
        sid: u32,
        req: u32,
        range: Option<(GridPoint, GridPoint)>,
    },
    /// `{1 sid, 2 req, 3 text}`
    Copied {
        sid: u32,
        req: u32,
        text: String,
    },
    /// `{1 sid, 2 req, 3 hits, 4 done}`
    SearchHits {
        sid: u32,
        req: u32,
        hits: Vec<Hit>,
        done: bool,
    },
    /// `{1 sid, 2 effect_id, 3 kind}`
    Event {
        sid: u32,
        id: EventId,
        kind: EventKind,
    },
    /// `{1 sid, 2 reason}`: a snapshot follows.
    Resync {
        sid: u32,
        reason: ResyncReason,
    },
    /// `{1 code, 2 message}`
    Error {
        code: u16,
        message: String,
    },
}

// ---------------------------------------------------------------------------
// Framing
// ---------------------------------------------------------------------------

/// Appends one frame: the length, `kind` and the payload `body` writes.
fn frame(out: &mut Vec<u8>, kind: u8, body: impl FnOnce(&mut Writer<'_>)) {
    let start = out.len();
    out.extend_from_slice(&[0; 4]);
    out.push(kind);
    body(&mut Writer::new(out));
    let len = (out.len() - start - 4) as u32;
    out[start..start + 4].copy_from_slice(&len.to_le_bytes());
}

/// Splits a byte stream into frames. Feed it whatever a read returned.
#[derive(Debug, Default)]
pub struct FrameReader {
    buf: Vec<u8>,
    at: usize,
}

impl FrameReader {
    pub fn new() -> Self {
        Self::default()
    }

    /// Bytes as a read returned them.
    pub fn push(&mut self, bytes: &[u8]) {
        if self.at > 0 && self.at == self.buf.len() {
            self.buf.clear();
            self.at = 0;
        }
        self.buf.extend_from_slice(bytes);
    }

    /// The next whole frame, `(kind, payload)`, if one has arrived.
    pub fn next_frame(&mut self) -> Result<Option<(u8, &[u8])>, MsgError> {
        let rest = &self.buf[self.at..];
        let Some(len) = rest.get(..4) else {
            return Ok(None);
        };
        let len = u32::from_le_bytes([len[0], len[1], len[2], len[3]]) as usize;
        if len == 0 || len > MAX_FRAME {
            return Err(MsgError::Frame(len));
        }
        if rest.len() < 4 + len {
            // Compact before the rest arrives, so a long-lived reader does
            // not keep every frame it has read.
            if self.at > 0 {
                self.buf.drain(..self.at);
                self.at = 0;
            }
            return Ok(None);
        }
        let start = self.at + 4;
        self.at = start + len;
        Ok(Some((self.buf[start], &self.buf[start + 1..start + len])))
    }
}

// ---------------------------------------------------------------------------
// Decoding helpers
// ---------------------------------------------------------------------------

struct Dec {
    kind: u8,
    f: Fields,
}

impl Dec {
    fn new(kind: u8, payload: &[u8]) -> Result<Dec, MsgError> {
        let f = cbor::decode(payload)?
            .into_fields()
            .ok_or(MsgError::NotAMap(kind))?;
        Ok(Dec { kind, f })
    }

    fn sub(kind: u8, v: Value) -> Result<Dec, MsgError> {
        Ok(Dec {
            kind,
            f: v.into_fields().ok_or(MsgError::NotAMap(kind))?,
        })
    }

    fn missing(&self, field: u64) -> MsgError {
        MsgError::Missing {
            kind: self.kind,
            field,
        }
    }

    fn invalid(&self, field: u64) -> MsgError {
        MsgError::Invalid {
            kind: self.kind,
            field,
        }
    }

    fn opt_num<T: TryFrom<u64>>(&mut self, key: u64) -> Result<Option<T>, MsgError> {
        match self.f.take(key) {
            None => Ok(None),
            Some(v) => {
                let n = v.as_u64().ok_or(self.missing(key))?;
                T::try_from(n).map(Some).map_err(|_| self.invalid(key))
            }
        }
    }

    fn num<T: TryFrom<u64>>(&mut self, key: u64) -> Result<T, MsgError> {
        self.opt_num(key)?.ok_or(self.missing(key))
    }

    /// A number that may be left out when it is zero.
    fn num_or_zero<T: TryFrom<u64> + Default>(&mut self, key: u64) -> Result<T, MsgError> {
        Ok(self.opt_num(key)?.unwrap_or_default())
    }

    fn opt_int(&mut self, key: u64) -> Result<Option<i32>, MsgError> {
        match self.f.take(key) {
            None | Some(Value::Null) => Ok(None),
            Some(v) => {
                let n = v.as_i64().ok_or(self.missing(key))?;
                i32::try_from(n).map(Some).map_err(|_| self.invalid(key))
            }
        }
    }

    fn flag(&mut self, key: u64) -> Result<bool, MsgError> {
        match self.f.take(key) {
            None => Ok(false),
            Some(v) => v.as_bool().ok_or(self.missing(key)),
        }
    }

    fn opt_text(&mut self, key: u64) -> Result<Option<String>, MsgError> {
        match self.f.take(key) {
            None | Some(Value::Null) => Ok(None),
            Some(v) => v.into_text().map(Some).ok_or(self.missing(key)),
        }
    }

    fn text(&mut self, key: u64) -> Result<String, MsgError> {
        Ok(self.opt_text(key)?.unwrap_or_default())
    }

    fn bytes(&mut self, key: u64) -> Result<Vec<u8>, MsgError> {
        match self.f.take(key) {
            None => Ok(Vec::new()),
            Some(v) => v.into_bytes().ok_or(self.missing(key)),
        }
    }

    fn float(&mut self, key: u64) -> Result<f32, MsgError> {
        match self.f.take(key) {
            None => Ok(0.0),
            Some(v) => Ok(v.as_f64().ok_or(self.missing(key))? as f32),
        }
    }

    fn list(&mut self, key: u64) -> Result<Vec<Value>, MsgError> {
        match self.f.take(key) {
            None => Ok(Vec::new()),
            Some(v) => v.into_array().ok_or(self.missing(key)),
        }
    }

    fn map(&mut self, key: u64) -> Result<Option<Dec>, MsgError> {
        match self.f.take(key) {
            None | Some(Value::Null) => Ok(None),
            Some(v) => Dec::sub(self.kind, v).map(Some),
        }
    }

    fn need_map(&mut self, key: u64) -> Result<Dec, MsgError> {
        self.map(key)?.ok_or(self.missing(key))
    }

    /// A small enum by its number; `None` for one this build does not know.
    fn variant(&mut self, key: u64) -> Result<Option<u64>, MsgError> {
        self.opt_num::<u64>(key)
    }
}

// ---------------------------------------------------------------------------
// Shared types on the wire
// ---------------------------------------------------------------------------

fn put_cursor(w: &mut Writer<'_>, c: &Cursor) {
    w.map();
    w.key(1).uint(u64::from(c.epoch));
    w.key(2).uint(c.next_rseq);
    w.key(3).uint(c.next_offset);
    w.end();
}

fn get_cursor(mut d: Dec) -> Result<Cursor, MsgError> {
    Ok(Cursor {
        epoch: d.num(1)?,
        next_rseq: d.num(2)?,
        next_offset: d.num(3)?,
    })
}

/// `Default` is a key left out; a palette index is its number; an RGB colour
/// is `1 << 24 | rgb`.
fn color_num(c: Color) -> Option<u64> {
    match c {
        Color::Default => None,
        Color::Palette(i) => Some(u64::from(i)),
        Color::Rgb(r, g, b) => Some(1 << 24 | rgb_num((r, g, b))),
    }
}

fn num_color(n: Option<u64>) -> Color {
    match n {
        None => Color::Default,
        Some(n) if n < 256 => Color::Palette(n as u8),
        Some(n) => {
            let (r, g, b) = num_rgb(n);
            Color::Rgb(r, g, b)
        }
    }
}

fn rgb_num((r, g, b): (u8, u8, u8)) -> u64 {
    u64::from(r) << 16 | u64::from(g) << 8 | u64::from(b)
}

fn num_rgb(n: u64) -> (u8, u8, u8) {
    ((n >> 16) as u8, (n >> 8) as u8, n as u8)
}

fn put_style(w: &mut Writer<'_>, s: &StyleDef) {
    w.map();
    w.key(1).uint(u64::from(s.id));
    for (k, c) in [(2, s.fg), (3, s.bg), (4, s.underline_color)] {
        if let Some(n) = color_num(c) {
            w.key(k).uint(n);
        }
    }
    if s.attrs != 0 {
        w.key(5).uint(u64::from(s.attrs));
    }
    if s.underline != Underline::None {
        w.key(6).uint(underline_num(s.underline));
    }
    w.end();
}

fn get_style(mut d: Dec) -> Result<StyleDef, MsgError> {
    Ok(StyleDef {
        id: d.num(1)?,
        fg: num_color(d.opt_num(2)?),
        bg: num_color(d.opt_num(3)?),
        underline_color: num_color(d.opt_num(4)?),
        attrs: d.num_or_zero(5)?,
        underline: num_underline(d.variant(6)?.unwrap_or(0)),
    })
}

fn underline_num(u: Underline) -> u64 {
    match u {
        Underline::None => 0,
        Underline::Single => 1,
        Underline::Double => 2,
        Underline::Curly => 3,
        Underline::Dotted => 4,
        Underline::Dashed => 5,
        Underline::Unknown => 6,
    }
}

fn num_underline(n: u64) -> Underline {
    match n {
        0 => Underline::None,
        1 => Underline::Single,
        2 => Underline::Double,
        3 => Underline::Curly,
        4 => Underline::Dotted,
        5 => Underline::Dashed,
        _ => Underline::Unknown,
    }
}

fn put_link(w: &mut Writer<'_>, l: &LinkDef) {
    w.map();
    w.key(1).uint(u64::from(l.id));
    w.key(2).text(&l.uri);
    if let Some(id) = &l.osc8_id {
        w.key(3).text(id);
    }
    w.end();
}

fn get_link(mut d: Dec) -> Result<LinkDef, MsgError> {
    Ok(LinkDef {
        id: d.num(1)?,
        uri: d.text(2)?,
        osc8_id: d.opt_text(3)?,
    })
}

fn put_row(w: &mut Writer<'_>, r: &Row) {
    w.map();
    w.key(1).uint(u64::from(r.y));
    if r.line != 0 {
        w.key(2).uint(r.line);
    }
    if r.flags != 0 {
        w.key(3).uint(u64::from(r.flags));
    }
    w.key(4).bytes(&r.cells);
    w.end();
}

fn get_row(mut d: Dec) -> Result<Row, MsgError> {
    Ok(Row {
        y: d.num(1)?,
        line: d.num_or_zero(2)?,
        flags: d.num_or_zero(3)?,
        cells: d.bytes(4)?,
    })
}

fn put_list<T>(w: &mut Writer<'_>, key: u64, items: &[T], put: fn(&mut Writer<'_>, &T)) {
    if items.is_empty() {
        return;
    }
    w.key(key).array(items.len());
    for i in items {
        put(w, i);
    }
}

fn get_list<T>(
    d: &mut Dec,
    key: u64,
    get: fn(Dec) -> Result<T, MsgError>,
) -> Result<Vec<T>, MsgError> {
    let kind = d.kind;
    d.list(key)?
        .into_iter()
        .map(|v| get(Dec::sub(kind, v)?))
        .collect()
}

fn put_cursor_state(w: &mut Writer<'_>, c: &CursorState) {
    w.map();
    w.key(1).uint(u64::from(c.x));
    w.key(2).uint(u64::from(c.y));
    w.key(3).bool(c.visible);
    w.key(4).bool(c.blinking);
    w.key(5).uint(match c.style {
        CursorStyle::Block => 0,
        CursorStyle::BlockHollow => 1,
        CursorStyle::Bar => 2,
        CursorStyle::Underline => 3,
        CursorStyle::Unknown => 4,
    });
    w.key(6).bool(c.wide_tail);
    w.end();
}

fn get_cursor_state(mut d: Dec) -> Result<CursorState, MsgError> {
    Ok(CursorState {
        x: d.num_or_zero(1)?,
        y: d.num_or_zero(2)?,
        visible: d.flag(3)?,
        blinking: d.flag(4)?,
        style: match d.variant(5)?.unwrap_or(0) {
            0 => CursorStyle::Block,
            1 => CursorStyle::BlockHollow,
            2 => CursorStyle::Bar,
            3 => CursorStyle::Underline,
            _ => CursorStyle::Unknown,
        },
        wide_tail: d.flag(6)?,
    })
}

fn put_colors(w: &mut Writer<'_>, c: &ColorsDelta) {
    w.map();
    for (k, v) in [(1, c.fg), (2, c.bg), (3, c.cursor)] {
        if let Some(rgb) = v {
            w.key(k).uint(rgb_num(rgb));
        }
    }
    if !c.palette.is_empty() {
        w.key(4).array(c.palette.len() * 2);
        for &(i, rgb) in &c.palette {
            w.uint(u64::from(i));
            w.uint(rgb_num(rgb));
        }
    }
    w.end();
}

fn get_colors(mut d: Dec) -> Result<ColorsDelta, MsgError> {
    let fg = d.opt_num::<u64>(1)?.map(num_rgb);
    let bg = d.opt_num::<u64>(2)?.map(num_rgb);
    let cursor = d.opt_num::<u64>(3)?.map(num_rgb);
    let flat = d.list(4)?;
    let mut palette = Vec::with_capacity(flat.len() / 2);
    for pair in flat.chunks(2) {
        let [i, rgb] = pair else {
            return Err(d.invalid(4));
        };
        let i = i.as_u64().and_then(|i| u8::try_from(i).ok());
        let rgb = rgb.as_u64();
        match (i, rgb) {
            (Some(i), Some(rgb)) => palette.push((i, num_rgb(rgb))),
            _ => return Err(d.invalid(4)),
        }
    }
    Ok(ColorsDelta {
        fg,
        bg,
        cursor,
        palette,
    })
}

fn screen_num(s: Screen) -> u64 {
    match s {
        Screen::Primary => 0,
        Screen::Alternate => 1,
    }
}

fn num_screen(n: u64) -> Screen {
    if n == 1 {
        Screen::Alternate
    } else {
        Screen::Primary
    }
}

fn mouse_num(m: MouseMode) -> u64 {
    match m {
        MouseMode::None => 0,
        MouseMode::X10 => 1,
        MouseMode::Normal => 2,
        MouseMode::Button => 3,
        MouseMode::Any => 4,
    }
}

fn num_mouse(n: u64) -> MouseMode {
    match n {
        1 => MouseMode::X10,
        2 => MouseMode::Normal,
        3 => MouseMode::Button,
        4 => MouseMode::Any,
        _ => MouseMode::None,
    }
}

fn put_term(w: &mut Writer<'_>, t: &TermState) {
    w.map();
    w.key(1).uint(u64::from(t.cols));
    w.key(2).uint(u64::from(t.rows));
    w.key(3).uint(screen_num(t.screen));
    w.key(4);
    put_cursor_state(w, &t.cursor);
    w.key(5);
    put_colors(w, &t.colors);
    w.key(6).uint(mouse_num(t.mouse));
    w.key(7).uint(u64::from(t.flags));
    w.key(8).text(&t.title);
    w.key(9).text(&t.cwd);
    w.key(10).uint(u64::from(t.sb_epoch));
    w.key(11).uint(t.history_lines);
    w.key(12).uint(t.top_line);
    w.end();
}

fn get_term(mut d: Dec) -> Result<TermState, MsgError> {
    Ok(TermState {
        cols: d.num(1)?,
        rows: d.num(2)?,
        screen: num_screen(d.variant(3)?.unwrap_or(0)),
        cursor: d
            .map(4)?
            .map(get_cursor_state)
            .transpose()?
            .unwrap_or_default(),
        colors: d.map(5)?.map(get_colors).transpose()?.unwrap_or_default(),
        mouse: num_mouse(d.variant(6)?.unwrap_or(0)),
        flags: d.num_or_zero(7)?,
        title: d.text(8)?,
        cwd: d.text(9)?,
        sb_epoch: d.num_or_zero(10)?,
        history_lines: d.num_or_zero(11)?,
        top_line: d.num_or_zero(12)?,
    })
}

fn put_term_delta(w: &mut Writer<'_>, t: &TermDelta) {
    w.map();
    if let Some((cols, rows)) = t.size {
        w.key(1).uint(u64::from(cols));
        w.key(2).uint(u64::from(rows));
    }
    if let Some(s) = t.screen {
        w.key(3).uint(screen_num(s));
    }
    if let Some(c) = &t.cursor {
        w.key(4);
        put_cursor_state(w, c);
    }
    if let Some(c) = &t.colors {
        w.key(5);
        put_colors(w, c);
    }
    if let Some(m) = t.mouse {
        w.key(6).uint(mouse_num(m));
    }
    if let Some(f) = t.flags {
        w.key(7).uint(u64::from(f));
    }
    if let Some(s) = &t.title {
        w.key(8).text(s);
    }
    if let Some(s) = &t.cwd {
        w.key(9).text(s);
    }
    if let Some(v) = t.sb_epoch {
        w.key(10).uint(u64::from(v));
    }
    if let Some(v) = t.history_lines {
        w.key(11).uint(v);
    }
    if let Some(v) = t.top_line {
        w.key(12).uint(v);
    }
    w.end();
}

fn get_term_delta(mut d: Dec) -> Result<TermDelta, MsgError> {
    let cols: Option<u16> = d.opt_num(1)?;
    let rows: Option<u16> = d.opt_num(2)?;
    let size = match (cols, rows) {
        (Some(c), Some(r)) => Some((c, r)),
        (None, None) => None,
        _ => return Err(d.missing(if cols.is_none() { 1 } else { 2 })),
    };
    Ok(TermDelta {
        size,
        screen: d.variant(3)?.map(num_screen),
        cursor: d.map(4)?.map(get_cursor_state).transpose()?,
        colors: d.map(5)?.map(get_colors).transpose()?,
        mouse: d.variant(6)?.map(num_mouse),
        flags: d.opt_num(7)?,
        title: d.opt_text(8)?,
        cwd: d.opt_text(9)?,
        sb_epoch: d.opt_num(10)?,
        history_lines: d.opt_num(11)?,
        top_line: d.opt_num(12)?,
    })
}

fn put_point(w: &mut Writer<'_>, p: &GridPoint) {
    w.map();
    w.key(1).uint(p.line);
    w.key(2).uint(u64::from(p.col));
    w.key(3).uint(u64::from(p.sb_epoch));
    w.end();
}

fn get_point(mut d: Dec) -> Result<GridPoint, MsgError> {
    Ok(GridPoint {
        line: d.num_or_zero(1)?,
        col: d.num_or_zero(2)?,
        sb_epoch: d.num_or_zero(3)?,
    })
}

fn need_point(d: &mut Dec, key: u64) -> Result<GridPoint, MsgError> {
    get_point(d.need_map(key)?)
}

// ---------------------------------------------------------------------------
// Client messages
// ---------------------------------------------------------------------------

impl ClientMsg {
    pub fn kind(&self) -> u8 {
        match self {
            ClientMsg::Hello(_) => kind::HELLO,
            ClientMsg::Attach(_) => kind::ATTACH,
            ClientMsg::Detach { .. } => kind::DETACH,
            ClientMsg::SetVisible { .. } => kind::SET_VISIBLE,
            ClientMsg::Ack { .. } => kind::ACK,
            ClientMsg::Input { .. } => kind::INPUT,
            ClientMsg::FetchHistory { .. } => kind::FETCH_HISTORY,
            ClientMsg::SelectAt { .. } => kind::SELECT_AT,
            ClientMsg::Copy { .. } => kind::COPY,
            ClientMsg::Search { .. } => kind::SEARCH,
            ClientMsg::Unhandled(k) => *k,
        }
    }

    /// Appends this message as one frame.
    pub fn encode(&self, out: &mut Vec<u8>) {
        frame(out, self.kind(), |w| {
            w.map();
            match self {
                ClientMsg::Hello(h) => {
                    w.key(1).uint(u64::from(h.proto_major));
                    w.key(2).uint(u64::from(h.proto_minor));
                    w.key(3).uint(h.caps);
                    w.key(4).uint(match h.client {
                        ClientKind::Native => 0,
                        ClientKind::Electron => 1,
                        ClientKind::Web => 2,
                        ClientKind::Cli => 3,
                        ClientKind::Unknown => 255,
                    });
                    w.key(5).text(&h.build);
                }
                ClientMsg::Attach(a) => {
                    w.key(1).text(&a.session);
                    w.key(2).uint(match a.mode {
                        AttachMode::Grid => 0,
                        AttachMode::Bytes => 1,
                    });
                    w.key(3).map();
                    w.key(1).uint(u64::from(a.view.cols));
                    w.key(2).uint(u64::from(a.view.rows));
                    w.key(3).uint(u64::from(a.view.px_w));
                    w.key(4).uint(u64::from(a.view.px_h));
                    w.end();
                    w.key(4).bool(a.visible);
                    match &a.resume {
                        None => {}
                        Some(Resume::Grid(r)) => {
                            w.key(5).map();
                            w.key(1).uint(r.state_gen);
                            w.key(2).uint(r.rev);
                            w.key(3).uint(u64::from(r.table_gen));
                            w.key(4).uint(u64::from(r.style_mark));
                            w.key(5).uint(u64::from(r.link_mark));
                            w.end();
                        }
                        Some(Resume::Bytes(c)) => {
                            w.key(5).map();
                            w.key(6);
                            put_cursor(w, c);
                            w.end();
                        }
                    }
                    w.key(6).uint(u64::from(a.history_tail));
                }
                ClientMsg::Detach { sid } => {
                    w.key(1).uint(u64::from(*sid));
                }
                ClientMsg::SetVisible { sid, visible } => {
                    w.key(1).uint(u64::from(*sid));
                    w.key(2).bool(*visible);
                }
                ClientMsg::Ack { sid, rev } => {
                    w.key(1).uint(u64::from(*sid));
                    w.key(2).uint(*rev);
                }
                ClientMsg::Input {
                    sid,
                    input_seq,
                    event,
                } => {
                    w.key(1).uint(u64::from(*sid));
                    w.key(2).uint(*input_seq);
                    w.key(3);
                    put_event(w, event);
                }
                ClientMsg::FetchHistory {
                    sid,
                    req,
                    sb_epoch,
                    from_line,
                    count,
                } => {
                    w.key(1).uint(u64::from(*sid));
                    w.key(2).uint(u64::from(*req));
                    w.key(3).uint(u64::from(*sb_epoch));
                    w.key(4).uint(*from_line);
                    w.key(5).uint(u64::from(*count));
                }
                ClientMsg::SelectAt { sid, req, at, kind } => {
                    w.key(1).uint(u64::from(*sid));
                    w.key(2).uint(u64::from(*req));
                    w.key(3);
                    put_point(w, at);
                    w.key(4).uint(match kind {
                        SelectKind::Word => 0,
                        SelectKind::Line => 1,
                        SelectKind::Output => 2,
                    });
                }
                ClientMsg::Copy {
                    sid,
                    req,
                    from,
                    to,
                    rect,
                    format,
                } => {
                    w.key(1).uint(u64::from(*sid));
                    w.key(2).uint(u64::from(*req));
                    w.key(3);
                    put_point(w, from);
                    w.key(4);
                    put_point(w, to);
                    w.key(5).bool(*rect);
                    w.key(6).uint(match format {
                        CopyFormat::Plain => 0,
                        CopyFormat::Vt => 1,
                        CopyFormat::Html => 2,
                    });
                }
                ClientMsg::Search {
                    sid,
                    req,
                    query,
                    regex,
                    case,
                    from_line,
                } => {
                    w.key(1).uint(u64::from(*sid));
                    w.key(2).uint(u64::from(*req));
                    w.key(3).text(query);
                    w.key(4).bool(*regex);
                    w.key(5).bool(*case);
                    if let Some(l) = from_line {
                        w.key(6).uint(*l);
                    }
                }
                ClientMsg::Unhandled(_) => {}
            }
            w.end();
        });
    }

    /// Reads one frame's message. `Ok(None)` for an optional kind this
    /// build does not know, which the reader skips.
    pub fn decode(k: u8, payload: &[u8]) -> Result<Option<ClientMsg>, MsgError> {
        let known = matches!(k, kind::HELLO..=kind::SET_DEFAULTS);
        if !known {
            return if k & kind::OPTIONAL != 0 {
                Ok(None)
            } else {
                Err(MsgError::UnknownKind(k))
            };
        }
        let mut d = Dec::new(k, payload)?;
        let msg = match k {
            kind::HELLO => ClientMsg::Hello(Hello {
                proto_major: d.num(1)?,
                proto_minor: d.num_or_zero(2)?,
                caps: d.num_or_zero(3)?,
                client: match d.variant(4)?.unwrap_or(0) {
                    0 => ClientKind::Native,
                    1 => ClientKind::Electron,
                    2 => ClientKind::Web,
                    3 => ClientKind::Cli,
                    _ => ClientKind::Unknown,
                },
                build: d.text(5)?,
            }),
            kind::ATTACH => {
                let session = d.opt_text(1)?.ok_or(d.missing(1))?;
                let mode = match d.variant(2)?.unwrap_or(0) {
                    0 => AttachMode::Grid,
                    1 => AttachMode::Bytes,
                    _ => return Err(d.invalid(2)),
                };
                let view = match d.map(3)? {
                    None => Size::default(),
                    Some(mut v) => Size {
                        cols: v.num_or_zero(1)?,
                        rows: v.num_or_zero(2)?,
                        px_w: v.num_or_zero(3)?,
                        px_h: v.num_or_zero(4)?,
                    },
                };
                let visible = d.flag(4)?;
                let resume = match d.map(5)? {
                    None => None,
                    Some(mut r) => match mode {
                        AttachMode::Grid => Some(Resume::Grid(GridResume {
                            state_gen: r.num(1)?,
                            rev: r.num(2)?,
                            table_gen: r.num_or_zero(3)?,
                            style_mark: r.num_or_zero(4)?,
                            link_mark: r.num_or_zero(5)?,
                        })),
                        AttachMode::Bytes => match r.map(6)? {
                            Some(c) => Some(Resume::Bytes(get_cursor(c)?)),
                            None => None,
                        },
                    },
                };
                ClientMsg::Attach(Attach {
                    session,
                    mode,
                    view,
                    visible,
                    resume,
                    history_tail: d.num_or_zero(6)?,
                })
            }
            kind::DETACH => ClientMsg::Detach { sid: d.num(1)? },
            kind::SET_VISIBLE => ClientMsg::SetVisible {
                sid: d.num(1)?,
                visible: d.flag(2)?,
            },
            kind::ACK => ClientMsg::Ack {
                sid: d.num(1)?,
                rev: d.num(2)?,
            },
            kind::INPUT => {
                let sid = d.num(1)?;
                let input_seq = d.num(2)?;
                let event = get_event(d.need_map(3)?)?;
                ClientMsg::Input {
                    sid,
                    input_seq,
                    event,
                }
            }
            kind::FETCH_HISTORY => ClientMsg::FetchHistory {
                sid: d.num(1)?,
                req: d.num_or_zero(2)?,
                sb_epoch: d.num_or_zero(3)?,
                from_line: d.num_or_zero(4)?,
                count: d.num_or_zero(5)?,
            },
            kind::SELECT_AT => ClientMsg::SelectAt {
                sid: d.num(1)?,
                req: d.num_or_zero(2)?,
                at: need_point(&mut d, 3)?,
                kind: match d.variant(4)?.unwrap_or(0) {
                    0 => SelectKind::Word,
                    1 => SelectKind::Line,
                    2 => SelectKind::Output,
                    _ => return Err(d.invalid(4)),
                },
            },
            kind::COPY => ClientMsg::Copy {
                sid: d.num(1)?,
                req: d.num_or_zero(2)?,
                from: need_point(&mut d, 3)?,
                to: need_point(&mut d, 4)?,
                rect: d.flag(5)?,
                format: match d.variant(6)?.unwrap_or(0) {
                    0 => CopyFormat::Plain,
                    1 => CopyFormat::Vt,
                    2 => CopyFormat::Html,
                    _ => return Err(d.invalid(6)),
                },
            },
            kind::SEARCH => ClientMsg::Search {
                sid: d.num(1)?,
                req: d.num_or_zero(2)?,
                query: d.text(3)?,
                regex: d.flag(4)?,
                case: d.flag(5)?,
                from_line: d.opt_num(6)?,
            },
            other => ClientMsg::Unhandled(other),
        };
        Ok(Some(msg))
    }
}

fn put_event(w: &mut Writer<'_>, e: &InputEvent) {
    w.map();
    match e {
        InputEvent::Key {
            action,
            key,
            mods,
            consumed_mods,
            text,
            unshifted,
            composing,
        } => {
            w.key(1).uint(0);
            w.key(2).uint(match action {
                KeyAction::Press => 0,
                KeyAction::Repeat => 1,
                KeyAction::Release => 2,
            });
            w.key(3).uint(u64::from(key.0));
            w.key(4).uint(u64::from(*mods));
            w.key(5).uint(u64::from(*consumed_mods));
            if let Some(t) = text {
                w.key(6).text(t);
            }
            if let Some(c) = unshifted {
                w.key(7).uint(u64::from(u32::from(*c)));
            }
            w.key(8).bool(*composing);
        }
        InputEvent::Text { utf8 } => {
            w.key(1).uint(1);
            w.key(2).text(utf8);
        }
        InputEvent::Paste { utf8, confirmed } => {
            w.key(1).uint(2);
            w.key(2).text(utf8);
            w.key(3).bool(*confirmed);
        }
        InputEvent::Mouse {
            action,
            button,
            mods,
            x,
            y,
        } => {
            w.key(1).uint(3);
            w.key(2).uint(match action {
                MouseAction::Press => 0,
                MouseAction::Release => 1,
                MouseAction::Motion => 2,
            });
            if let Some(b) = button {
                w.key(3).uint(u64::from(*b));
            }
            w.key(4).uint(u64::from(*mods));
            w.key(5).f32(*x);
            w.key(6).f32(*y);
        }
        InputEvent::Wheel { dx, dy, mods, x, y } => {
            w.key(1).uint(4);
            w.key(2).f32(*dx);
            w.key(3).f32(*dy);
            w.key(4).uint(u64::from(*mods));
            w.key(5).f32(*x);
            w.key(6).f32(*y);
        }
        InputEvent::Focus { focused } => {
            w.key(1).uint(5);
            w.key(2).bool(*focused);
        }
        InputEvent::Raw { bytes } => {
            w.key(1).uint(6);
            w.key(2).bytes(bytes);
        }
    }
    w.end();
}

fn get_event(mut d: Dec) -> Result<InputEvent, MsgError> {
    Ok(match d.variant(1)?.ok_or(d.missing(1))? {
        0 => InputEvent::Key {
            action: match d.variant(2)?.unwrap_or(0) {
                0 => KeyAction::Press,
                1 => KeyAction::Repeat,
                2 => KeyAction::Release,
                _ => return Err(d.invalid(2)),
            },
            key: KeyCode(d.num_or_zero(3)?),
            mods: d.num_or_zero(4)?,
            consumed_mods: d.num_or_zero(5)?,
            text: d.opt_text(6)?,
            unshifted: match d.opt_num::<u32>(7)? {
                None => None,
                Some(c) => Some(char::from_u32(c).ok_or(d.invalid(7))?),
            },
            composing: d.flag(8)?,
        },
        1 => InputEvent::Text { utf8: d.text(2)? },
        2 => InputEvent::Paste {
            utf8: d.text(2)?,
            confirmed: d.flag(3)?,
        },
        3 => InputEvent::Mouse {
            action: match d.variant(2)?.unwrap_or(0) {
                0 => MouseAction::Press,
                1 => MouseAction::Release,
                2 => MouseAction::Motion,
                _ => return Err(d.invalid(2)),
            },
            button: d.opt_num(3)?,
            mods: d.num_or_zero(4)?,
            x: d.float(5)?,
            y: d.float(6)?,
        },
        4 => InputEvent::Wheel {
            dx: d.float(2)?,
            dy: d.float(3)?,
            mods: d.num_or_zero(4)?,
            x: d.float(5)?,
            y: d.float(6)?,
        },
        5 => InputEvent::Focus {
            focused: d.flag(2)?,
        },
        6 => InputEvent::Raw { bytes: d.bytes(2)? },
        _ => return Err(d.invalid(1)),
    })
}

// ---------------------------------------------------------------------------
// vornd messages
// ---------------------------------------------------------------------------

impl ServerMsg {
    pub fn kind(&self) -> u8 {
        match self {
            ServerMsg::Welcome(_) => kind::WELCOME,
            ServerMsg::Attached(_) => kind::ATTACHED,
            ServerMsg::Snapshot { .. } => kind::SNAPSHOT,
            ServerMsg::Delta { .. } => kind::DELTA,
            ServerMsg::ResetTables { .. } => kind::RESET_TABLES,
            ServerMsg::InputAck { .. } => kind::INPUT_ACK,
            ServerMsg::PasteConfirm { .. } => kind::PASTE_CONFIRM,
            ServerMsg::History { .. } => kind::HISTORY,
            ServerMsg::Selection { .. } => kind::SELECTION,
            ServerMsg::Copied { .. } => kind::COPIED,
            ServerMsg::SearchHits { .. } => kind::SEARCH_HITS,
            ServerMsg::Event { .. } => kind::EVENT,
            ServerMsg::Resync { .. } => kind::RESYNC,
            ServerMsg::Error { .. } => kind::ERROR,
        }
    }

    /// The attachment the message is for; `None` for the connection's own.
    pub fn sid(&self) -> Option<u32> {
        match self {
            ServerMsg::Welcome(_) | ServerMsg::Error { .. } => None,
            ServerMsg::Attached(a) => Some(a.sid),
            ServerMsg::Snapshot { sid, .. }
            | ServerMsg::Delta { sid, .. }
            | ServerMsg::ResetTables { sid, .. }
            | ServerMsg::InputAck { sid, .. }
            | ServerMsg::PasteConfirm { sid, .. }
            | ServerMsg::History { sid, .. }
            | ServerMsg::Selection { sid, .. }
            | ServerMsg::Copied { sid, .. }
            | ServerMsg::SearchHits { sid, .. }
            | ServerMsg::Event { sid, .. }
            | ServerMsg::Resync { sid, .. } => Some(*sid),
        }
    }

    /// Appends this message as one frame.
    pub fn encode(&self, out: &mut Vec<u8>) {
        frame(out, self.kind(), |w| {
            w.map();
            match self {
                ServerMsg::Welcome(m) => {
                    w.key(1).uint(u64::from(m.proto_major));
                    w.key(2).uint(u64::from(m.proto_minor));
                    w.key(3).uint(m.caps);
                    w.key(4).text(&m.vornd_build);
                    w.key(5).uint(m.vornd_instance);
                }
                ServerMsg::Attached(a) => {
                    w.key(1).uint(u64::from(a.sid));
                    w.key(2).text(&a.session);
                    w.key(3).uint(u64::from(a.epoch));
                    w.key(4).bool(a.owner);
                    w.key(5).uint(match a.fidelity {
                        Fidelity::Exact => 0,
                        Fidelity::Approximate => 1,
                    });
                }
                ServerMsg::Snapshot { sid, snap } => {
                    w.key(1).uint(u64::from(*sid));
                    w.key(2).uint(snap.state_gen);
                    w.key(3).uint(snap.rev);
                    w.key(4);
                    put_cursor(w, &snap.resume);
                    w.key(5).uint(u64::from(snap.table_gen));
                    w.key(6).uint(u64::from(snap.row_fmt));
                    w.key(7);
                    put_term(w, &snap.term);
                    put_list(w, 8, &snap.styles, put_style);
                    put_list(w, 9, &snap.links, put_link);
                    put_list(w, 10, &snap.rows, put_row);
                    put_list(w, 11, &snap.history, put_row);
                }
                ServerMsg::Delta { sid, delta } => {
                    w.key(1).uint(u64::from(*sid));
                    w.key(2).uint(delta.state_gen);
                    w.key(3).uint(u64::from(delta.table_gen));
                    w.key(4).uint(delta.base_rev);
                    w.key(5).uint(delta.rev);
                    w.key(6);
                    put_cursor(w, &delta.resume);
                    if delta.scrolled != 0 {
                        w.key(7).uint(u64::from(delta.scrolled));
                    }
                    if let Some(t) = &delta.term {
                        w.key(8);
                        put_term_delta(w, t);
                    }
                    put_list(w, 9, &delta.styles, put_style);
                    put_list(w, 10, &delta.links, put_link);
                    put_list(w, 11, &delta.rows, put_row);
                }
                ServerMsg::ResetTables { sid, table_gen } => {
                    w.key(1).uint(u64::from(*sid));
                    w.key(2).uint(u64::from(*table_gen));
                }
                ServerMsg::InputAck { sid, input_seq }
                | ServerMsg::PasteConfirm { sid, input_seq } => {
                    w.key(1).uint(u64::from(*sid));
                    w.key(2).uint(*input_seq);
                }
                ServerMsg::History {
                    sid,
                    req,
                    sb_epoch,
                    from_line,
                    rows,
                    oldest_line,
                    styles,
                    links,
                } => {
                    w.key(1).uint(u64::from(*sid));
                    w.key(2).uint(u64::from(*req));
                    w.key(3).uint(u64::from(*sb_epoch));
                    w.key(4).uint(*from_line);
                    put_list(w, 5, rows, put_row);
                    w.key(6).uint(*oldest_line);
                    put_list(w, 7, styles, put_style);
                    put_list(w, 8, links, put_link);
                }
                ServerMsg::Selection { sid, req, range } => {
                    w.key(1).uint(u64::from(*sid));
                    w.key(2).uint(u64::from(*req));
                    if let Some((from, to)) = range {
                        w.key(3);
                        put_point(w, from);
                        w.key(4);
                        put_point(w, to);
                    }
                }
                ServerMsg::Copied { sid, req, text } => {
                    w.key(1).uint(u64::from(*sid));
                    w.key(2).uint(u64::from(*req));
                    w.key(3).text(text);
                }
                ServerMsg::SearchHits {
                    sid,
                    req,
                    hits,
                    done,
                } => {
                    w.key(1).uint(u64::from(*sid));
                    w.key(2).uint(u64::from(*req));
                    w.key(3).array(hits.len());
                    for h in hits {
                        w.map();
                        w.key(1);
                        put_point(w, &h.from);
                        w.key(2);
                        put_point(w, &h.to);
                        w.end();
                    }
                    w.key(4).bool(*done);
                }
                ServerMsg::Event { sid, id, kind } => {
                    w.key(1).uint(u64::from(*sid));
                    w.key(2).map();
                    w.key(1).uint(u64::from(id.epoch));
                    w.key(2).uint(id.rseq);
                    w.key(3).uint(u64::from(id.index));
                    w.end();
                    w.key(3).map();
                    match kind {
                        EventKind::Bell => {
                            w.key(1).uint(0);
                        }
                        EventKind::Clipboard { text } => {
                            w.key(1).uint(1);
                            w.key(2).text(text);
                        }
                        EventKind::Notify { title, body } => {
                            w.key(1).uint(2);
                            w.key(2).text(title);
                            w.key(3).text(body);
                        }
                        EventKind::Status { state } => {
                            w.key(1).uint(3);
                            w.key(2).uint(u64::from(*state));
                        }
                        EventKind::Exit { code, signal } => {
                            w.key(1).uint(4);
                            if let Some(c) = code {
                                w.key(2).int(i64::from(*c));
                            }
                            if let Some(s) = signal {
                                w.key(3).int(i64::from(*s));
                            }
                        }
                    }
                    w.end();
                }
                ServerMsg::Resync { sid, reason } => {
                    w.key(1).uint(u64::from(*sid));
                    w.key(2).uint(match reason {
                        ResyncReason::Gap => 0,
                        ResyncReason::TableCompacted => 1,
                        ResyncReason::BaseMismatch => 2,
                        ResyncReason::Restarted => 3,
                        ResyncReason::NotRetained => 4,
                    });
                }
                ServerMsg::Error { code, message } => {
                    w.key(1).uint(u64::from(*code));
                    w.key(2).text(message);
                }
            }
            w.end();
        });
    }

    /// Reads one frame's message. `Ok(None)` for a kind the client may skip:
    /// an optional one it does not know, or one of the bytes mode's.
    pub fn decode(k: u8, payload: &[u8]) -> Result<Option<ServerMsg>, MsgError> {
        match k {
            kind::BYTES | kind::VT_SNAPSHOT | kind::RESIZED => return Ok(None),
            kind::WELCOME..=kind::ERROR => {}
            _ if k & kind::OPTIONAL != 0 => return Ok(None),
            _ => return Err(MsgError::UnknownKind(k)),
        }
        let mut d = Dec::new(k, payload)?;
        let msg = match k {
            kind::WELCOME => ServerMsg::Welcome(Welcome {
                proto_major: d.num(1)?,
                proto_minor: d.num_or_zero(2)?,
                caps: d.num_or_zero(3)?,
                vornd_build: d.text(4)?,
                vornd_instance: d.num_or_zero(5)?,
            }),
            kind::ATTACHED => ServerMsg::Attached(Attached {
                sid: d.num(1)?,
                session: d.text(2)?,
                epoch: d.num_or_zero(3)?,
                owner: d.flag(4)?,
                fidelity: match d.variant(5)?.unwrap_or(1) {
                    0 => Fidelity::Exact,
                    _ => Fidelity::Approximate,
                },
            }),
            kind::SNAPSHOT => {
                let sid = d.num(1)?;
                let snap = Snapshot {
                    state_gen: d.num(2)?,
                    rev: d.num(3)?,
                    resume: get_cursor(d.need_map(4)?)?,
                    table_gen: d.num_or_zero(5)?,
                    row_fmt: d.num(6)?,
                    term: get_term(d.need_map(7)?)?,
                    styles: get_list(&mut d, 8, get_style)?,
                    links: get_list(&mut d, 9, get_link)?,
                    rows: get_list(&mut d, 10, get_row)?,
                    history: get_list(&mut d, 11, get_row)?,
                };
                ServerMsg::Snapshot {
                    sid,
                    snap: Box::new(snap),
                }
            }
            kind::DELTA => {
                let sid = d.num(1)?;
                let delta = Delta {
                    state_gen: d.num(2)?,
                    table_gen: d.num_or_zero(3)?,
                    base_rev: d.num(4)?,
                    rev: d.num(5)?,
                    resume: get_cursor(d.need_map(6)?)?,
                    scrolled: d.num_or_zero(7)?,
                    term: d.map(8)?.map(get_term_delta).transpose()?,
                    styles: get_list(&mut d, 9, get_style)?,
                    links: get_list(&mut d, 10, get_link)?,
                    rows: get_list(&mut d, 11, get_row)?,
                };
                ServerMsg::Delta {
                    sid,
                    delta: Box::new(delta),
                }
            }
            kind::RESET_TABLES => ServerMsg::ResetTables {
                sid: d.num(1)?,
                table_gen: d.num_or_zero(2)?,
            },
            kind::INPUT_ACK => ServerMsg::InputAck {
                sid: d.num(1)?,
                input_seq: d.num(2)?,
            },
            kind::PASTE_CONFIRM => ServerMsg::PasteConfirm {
                sid: d.num(1)?,
                input_seq: d.num(2)?,
            },
            kind::HISTORY => ServerMsg::History {
                sid: d.num(1)?,
                req: d.num_or_zero(2)?,
                sb_epoch: d.num_or_zero(3)?,
                from_line: d.num_or_zero(4)?,
                rows: get_list(&mut d, 5, get_row)?,
                oldest_line: d.num_or_zero(6)?,
                styles: get_list(&mut d, 7, get_style)?,
                links: get_list(&mut d, 8, get_link)?,
            },
            kind::SELECTION => {
                let sid = d.num(1)?;
                let req = d.num_or_zero(2)?;
                let from = d.map(3)?.map(get_point).transpose()?;
                let to = d.map(4)?.map(get_point).transpose()?;
                ServerMsg::Selection {
                    sid,
                    req,
                    range: from.zip(to),
                }
            }
            kind::COPIED => ServerMsg::Copied {
                sid: d.num(1)?,
                req: d.num_or_zero(2)?,
                text: d.text(3)?,
            },
            kind::SEARCH_HITS => {
                let sid = d.num(1)?;
                let req = d.num_or_zero(2)?;
                let hits = get_list(&mut d, 3, |mut h| {
                    Ok(Hit {
                        from: need_point(&mut h, 1)?,
                        to: need_point(&mut h, 2)?,
                    })
                })?;
                ServerMsg::SearchHits {
                    sid,
                    req,
                    hits,
                    done: d.flag(4)?,
                }
            }
            kind::EVENT => {
                let sid = d.num(1)?;
                let mut id = d.need_map(2)?;
                let id = EventId {
                    epoch: id.num_or_zero(1)?,
                    rseq: id.num_or_zero(2)?,
                    index: id.num_or_zero(3)?,
                };
                let mut e = d.need_map(3)?;
                let kind = match e.variant(1)?.ok_or(e.missing(1))? {
                    0 => EventKind::Bell,
                    1 => EventKind::Clipboard { text: e.text(2)? },
                    2 => EventKind::Notify {
                        title: e.text(2)?,
                        body: e.text(3)?,
                    },
                    3 => EventKind::Status {
                        state: e.num_or_zero(2)?,
                    },
                    4 => EventKind::Exit {
                        code: e.opt_int(2)?,
                        signal: e.opt_int(3)?,
                    },
                    _ => return Err(e.invalid(1)),
                };
                ServerMsg::Event { sid, id, kind }
            }
            kind::RESYNC => ServerMsg::Resync {
                sid: d.num(1)?,
                reason: match d.variant(2)?.unwrap_or(0) {
                    0 => ResyncReason::Gap,
                    1 => ResyncReason::TableCompacted,
                    2 => ResyncReason::BaseMismatch,
                    3 => ResyncReason::Restarted,
                    _ => ResyncReason::NotRetained,
                },
            },
            kind::ERROR => ServerMsg::Error {
                code: d.num_or_zero(1)?,
                message: d.text(2)?,
            },
            _ => return Err(MsgError::UnknownKind(k)),
        };
        Ok(Some(msg))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::screen::{attrs, flags, row_flags};

    fn round_client(m: ClientMsg) {
        let mut out = Vec::new();
        m.encode(&mut out);
        let mut r = FrameReader::new();
        r.push(&out);
        let (k, p) = r.next_frame().unwrap().unwrap();
        assert_eq!(ClientMsg::decode(k, p).unwrap(), Some(m));
        assert_eq!(r.next_frame().unwrap(), None);
    }

    fn round_server(m: ServerMsg) {
        let mut out = Vec::new();
        m.encode(&mut out);
        let mut r = FrameReader::new();
        // Byte by byte: a frame arrives in pieces.
        let mut got = None;
        for b in &out {
            r.push(std::slice::from_ref(b));
            if let Some((k, p)) = r.next_frame().unwrap() {
                got = Some(ServerMsg::decode(k, p).unwrap().unwrap());
            }
        }
        assert_eq!(got, Some(m));
    }

    fn term() -> TermState {
        TermState {
            cols: 80,
            rows: 24,
            screen: Screen::Alternate,
            cursor: CursorState {
                x: 3,
                y: 4,
                visible: true,
                blinking: false,
                style: CursorStyle::Bar,
                wide_tail: true,
            },
            colors: ColorsDelta {
                fg: Some((1, 2, 3)),
                bg: None,
                cursor: Some((9, 9, 9)),
                palette: vec![(0, (0, 0, 0)), (255, (255, 255, 255))],
            },
            mouse: MouseMode::Button,
            flags: flags::BRACKETED_PASTE | flags::SYNC_OUTPUT_ACTIVE,
            title: "vim".into(),
            cwd: "/tmp".into(),
            sb_epoch: 3,
            history_lines: 1000,
            top_line: 123_456_789_012,
        }
    }

    #[test]
    fn every_client_message_round_trips() {
        round_client(ClientMsg::Hello(Hello {
            proto_major: 1,
            proto_minor: 7,
            caps: caps::ALL,
            client: ClientKind::Cli,
            build: "test".into(),
        }));
        round_client(ClientMsg::Attach(Attach {
            session: "s-1".into(),
            mode: AttachMode::Grid,
            view: Size {
                cols: 200,
                rows: 50,
                px_w: 1600,
                px_h: 900,
            },
            visible: true,
            resume: Some(Resume::Grid(GridResume {
                state_gen: u64::MAX,
                rev: 412,
                table_gen: 2,
                style_mark: 30,
                link_mark: 4,
            })),
            history_tail: 200,
        }));
        round_client(ClientMsg::Attach(Attach {
            session: "b".into(),
            mode: AttachMode::Bytes,
            resume: Some(Resume::Bytes(Cursor {
                epoch: 1,
                next_rseq: 8,
                next_offset: 120,
            })),
            ..Attach::default()
        }));
        round_client(ClientMsg::Detach { sid: 7 });
        round_client(ClientMsg::SetVisible {
            sid: 7,
            visible: true,
        });
        round_client(ClientMsg::Ack { sid: 7, rev: 99 });
        for event in [
            InputEvent::Key {
                action: KeyAction::Repeat,
                key: KeyCode::from_name("ArrowUp").unwrap(),
                mods: mods::CTRL | mods::SHIFT,
                consumed_mods: mods::SHIFT,
                text: Some("A".into()),
                unshifted: Some('a'),
                composing: false,
            },
            InputEvent::Text {
                utf8: "日本".into(),
            },
            InputEvent::Paste {
                utf8: "ls\n".into(),
                confirmed: true,
            },
            InputEvent::Mouse {
                action: MouseAction::Motion,
                button: Some(1),
                mods: 0,
                x: 10.5,
                y: 3.25,
            },
            InputEvent::Wheel {
                dx: 0.0,
                dy: -3.0,
                mods: mods::ALT,
                x: 12.0,
                y: 7.5,
            },
            InputEvent::Focus { focused: true },
            InputEvent::Raw {
                bytes: vec![0x1b, b'[', b'A'],
            },
        ] {
            round_client(ClientMsg::Input {
                sid: 1,
                input_seq: 5,
                event,
            });
        }
        round_client(ClientMsg::FetchHistory {
            sid: 1,
            req: 2,
            sb_epoch: 3,
            from_line: 4,
            count: 200,
        });
        let p = GridPoint {
            line: 10,
            col: 4,
            sb_epoch: 1,
        };
        round_client(ClientMsg::SelectAt {
            sid: 1,
            req: 2,
            at: p,
            kind: SelectKind::Output,
        });
        round_client(ClientMsg::Copy {
            sid: 1,
            req: 2,
            from: p,
            to: GridPoint { col: 9, ..p },
            rect: true,
            format: CopyFormat::Html,
        });
        round_client(ClientMsg::Search {
            sid: 1,
            req: 2,
            query: "err(or)?".into(),
            regex: true,
            case: false,
            from_line: Some(5),
        });
        round_client(ClientMsg::Unhandled(kind::VIEWPORT));
    }

    #[test]
    fn every_server_message_round_trips() {
        round_server(ServerMsg::Welcome(Welcome {
            proto_major: 1,
            proto_minor: 0,
            caps: caps::GRID,
            vornd_build: "0.8".into(),
            vornd_instance: 42,
        }));
        round_server(ServerMsg::Attached(Attached {
            sid: 1,
            session: "s".into(),
            epoch: 2,
            owner: false,
            fidelity: Fidelity::Exact,
        }));
        let row = Row {
            y: 3,
            line: 1003,
            flags: row_flags::WRAPPED | row_flags::PROMPT,
            cells: vec![0, 0, 4, b'a', b'b'],
        };
        let style = StyleDef {
            id: 1,
            fg: Color::Palette(196),
            bg: Color::Rgb(1, 2, 3),
            underline_color: Color::Default,
            attrs: attrs::BOLD | attrs::OVERLINE,
            underline: Underline::Curly,
        };
        let link = LinkDef {
            id: 1,
            uri: "https://example.com".into(),
            osc8_id: Some("x".into()),
        };
        let resume = Cursor {
            epoch: 1,
            next_rseq: 88_121,
            next_offset: 1 << 40,
        };
        round_server(ServerMsg::Snapshot {
            sid: 1,
            snap: Box::new(Snapshot {
                state_gen: 9,
                rev: 412,
                resume,
                table_gen: 1,
                row_fmt: 1,
                term: term(),
                styles: vec![StyleDef::default(), style.clone()],
                links: vec![LinkDef::default(), link.clone()],
                rows: vec![row.clone(), Row::default()],
                history: vec![row.clone()],
            }),
        });
        round_server(ServerMsg::Delta {
            sid: 1,
            delta: Box::new(Delta {
                state_gen: 9,
                table_gen: 1,
                base_rev: 412,
                rev: 431,
                resume,
                scrolled: 120,
                term: Some(TermDelta {
                    size: Some((100, 30)),
                    title: Some(String::new()),
                    sb_epoch: Some(4),
                    ..TermDelta::default()
                }),
                styles: vec![style],
                links: vec![link.clone()],
                rows: vec![row.clone()],
            }),
        });
        round_server(ServerMsg::Delta {
            sid: 1,
            delta: Box::new(Delta::default()),
        });
        round_server(ServerMsg::ResetTables {
            sid: 1,
            table_gen: 3,
        });
        round_server(ServerMsg::InputAck {
            sid: 1,
            input_seq: 4,
        });
        round_server(ServerMsg::PasteConfirm {
            sid: 1,
            input_seq: 4,
        });
        round_server(ServerMsg::History {
            sid: 1,
            req: 2,
            sb_epoch: 3,
            from_line: 800,
            rows: vec![row],
            oldest_line: 10,
            styles: Vec::new(),
            links: vec![link],
        });
        let p = GridPoint {
            line: 1,
            col: 2,
            sb_epoch: 3,
        };
        round_server(ServerMsg::Selection {
            sid: 1,
            req: 2,
            range: Some((p, p)),
        });
        round_server(ServerMsg::Selection {
            sid: 1,
            req: 2,
            range: None,
        });
        round_server(ServerMsg::Copied {
            sid: 1,
            req: 2,
            text: "hi".into(),
        });
        round_server(ServerMsg::SearchHits {
            sid: 1,
            req: 2,
            hits: vec![Hit { from: p, to: p }],
            done: true,
        });
        for kind in [
            EventKind::Bell,
            EventKind::Clipboard { text: "x".into() },
            EventKind::Notify {
                title: "t".into(),
                body: "b".into(),
            },
            EventKind::Status { state: 2 },
            EventKind::Exit {
                code: Some(-1),
                signal: None,
            },
        ] {
            round_server(ServerMsg::Event {
                sid: 1,
                id: EventId {
                    epoch: 1,
                    rseq: 2,
                    index: 3,
                },
                kind,
            });
        }
        round_server(ServerMsg::Resync {
            sid: 1,
            reason: ResyncReason::Restarted,
        });
        round_server(ServerMsg::Error {
            code: 426,
            message: "upgrade".into(),
        });
    }

    /// TP-T18's reader half: unknown fields and unknown optional kinds are
    /// skipped; an unknown required kind is an error.
    #[test]
    fn newer_peers_are_read_by_older_readers() {
        // An Ack from a newer client, with a field this build does not know.
        let mut out = Vec::new();
        frame(&mut out, kind::ACK, |w| {
            w.map();
            w.key(1).uint(3);
            w.key(2).uint(10);
            w.key(40).text("from the future");
            w.end();
        });
        let mut r = FrameReader::new();
        r.push(&out);
        let (k, p) = r.next_frame().unwrap().unwrap();
        assert_eq!(
            ClientMsg::decode(k, p).unwrap(),
            Some(ClientMsg::Ack { sid: 3, rev: 10 })
        );
        assert_eq!(ClientMsg::decode(0x9a, &[0xa0]).unwrap(), None);
        assert_eq!(ServerMsg::decode(0xc0, &[0xa0]).unwrap(), None);
        assert_eq!(
            ClientMsg::decode(0x1a, &[0xa0]),
            Err(MsgError::UnknownKind(0x1a))
        );
        assert_eq!(
            ServerMsg::decode(0x40, &[0xa0]),
            Err(MsgError::UnknownKind(0x40))
        );
        // A known message missing a required field.
        assert_eq!(
            ClientMsg::decode(kind::ACK, &[0xa1, 0x01, 0x01]),
            Err(MsgError::Missing {
                kind: kind::ACK,
                field: 2
            })
        );
        assert!(major_supported(PROTO_MAJOR));
        assert!(major_supported(PROTO_MAJOR - 1));
        assert!(!major_supported(PROTO_MAJOR + 1));
    }

    #[test]
    fn frames_are_bounded() {
        let mut r = FrameReader::new();
        r.push(&((MAX_FRAME as u32) + 1).to_le_bytes());
        assert_eq!(r.next_frame(), Err(MsgError::Frame(MAX_FRAME + 1)));
        let mut r = FrameReader::new();
        r.push(&0u32.to_le_bytes());
        assert_eq!(r.next_frame(), Err(MsgError::Frame(0)));
    }

    #[test]
    fn key_names_are_unique() {
        for (i, n) in KEY_NAMES.iter().enumerate() {
            assert_eq!(KeyCode::from_name(n), Some(KeyCode(i as u16)));
            assert_eq!(KeyCode(i as u16).name(), *n);
        }
        assert_eq!(KeyCode(9999).name(), "Unidentified");
    }
}

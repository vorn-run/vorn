//! The screen mirror's types (Terminal State Protocol §5 and §6): what vornd
//! sends a grid client in place of bytes. libghostty-vt's types never appear
//! here, so a Ghostty upgrade cannot change the wire.
//!
//! Types only for now. The CBOR framing (§7) lands with vornd, when there is
//! a second side to agree with.

use crate::position::Cursor;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Screen {
    #[default]
    Primary,
    Alternate,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum CursorStyle {
    #[default]
    Block,
    BlockHollow,
    Bar,
    Underline,
    /// A style a newer Ghostty added and this build does not know.
    Unknown,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct CursorState {
    pub x: u16,
    pub y: u16,
    pub visible: bool,
    pub blinking: bool,
    pub style: CursorStyle,
    pub wide_tail: bool,
}

/// Unresolved, so a palette change resends no rows.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Color {
    #[default]
    Default,
    Palette(u8),
    Rgb(u8, u8, u8),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum MouseMode {
    #[default]
    None,
    X10,
    Normal,
    Button,
    Any,
}

/// Bits of `TermState::flags`.
pub mod flags {
    pub const BRACKETED_PASTE: u32 = 1 << 0;
    pub const FOCUS_EVENTS: u32 = 1 << 1;
    pub const SYNC_OUTPUT_ACTIVE: u32 = 1 << 2;
    pub const PASSWORD_INPUT: u32 = 1 << 3;
    pub const REVERSE_VIDEO: u32 = 1 << 4;
}

/// Bits of `StyleDef::attrs`.
pub mod attrs {
    pub const BOLD: u16 = 1 << 0;
    pub const ITALIC: u16 = 1 << 1;
    pub const FAINT: u16 = 1 << 2;
    pub const BLINK: u16 = 1 << 3;
    pub const INVERSE: u16 = 1 << 4;
    pub const INVISIBLE: u16 = 1 << 5;
    pub const STRIKE: u16 = 1 << 6;
    pub const OVERLINE: u16 = 1 << 7;
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Underline {
    #[default]
    None,
    Single,
    Double,
    Curly,
    Dotted,
    Dashed,
    Unknown,
}

/// The palette's defaults and the entries that changed.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct ColorsDelta {
    pub fg: Option<(u8, u8, u8)>,
    pub bg: Option<(u8, u8, u8)>,
    pub cursor: Option<(u8, u8, u8)>,
    pub palette: Vec<(u8, (u8, u8, u8))>,
}

/// Sent whole in a snapshot; a delta carries the same fields, each only when
/// it changed.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct TermState {
    pub cols: u16,
    pub rows: u16,
    pub screen: Screen,
    pub cursor: CursorState,
    pub colors: ColorsDelta,
    pub mouse: MouseMode,
    pub flags: u32,
    pub title: String,
    pub cwd: String,
    /// Raised when absolute line numbers stop being valid.
    pub sb_epoch: u32,
    /// Lines currently held in scrollback.
    pub history_lines: u64,
    /// Absolute line of active row 0, primary screen only.
    pub top_line: u64,
}

/// A delta's changes to `TermState`.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct TermDelta {
    pub size: Option<(u16, u16)>,
    pub screen: Option<Screen>,
    pub cursor: Option<CursorState>,
    pub colors: Option<ColorsDelta>,
    pub mouse: Option<MouseMode>,
    pub flags: Option<u32>,
    pub title: Option<String>,
    pub cwd: Option<String>,
    pub sb_epoch: Option<u32>,
    pub history_lines: Option<u64>,
    pub top_line: Option<u64>,
}

#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct StyleDef {
    pub id: u32,
    pub fg: Color,
    pub bg: Color,
    pub underline_color: Color,
    pub attrs: u16,
    pub underline: Underline,
}

#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct LinkDef {
    pub id: u32,
    pub uri: String,
    pub osc8_id: Option<String>,
}

/// Bits of `Row::flags`.
pub mod row_flags {
    pub const WRAPPED: u8 = 1 << 0;
    pub const WRAP_CONTINUATION: u8 = 1 << 1;
    pub const PROMPT: u8 = 1 << 2;
    pub const PROMPT_CONTINUATION: u8 = 1 << 3;
}

#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct Row {
    /// Index in the viewport.
    pub y: u16,
    /// Absolute line on the primary screen; 0 on the alternate screen.
    pub line: u64,
    pub flags: u8,
    /// `row_fmt` 1, see [`crate::row`].
    pub cells: Vec<u8>,
}

/// What a grid client gets on attach or after a resync: complete tables and
/// the whole viewport.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct Snapshot {
    pub state_gen: u64,
    pub rev: u64,
    pub resume: Cursor,
    pub table_gen: u32,
    pub row_fmt: u8,
    pub term: TermState,
    pub styles: Vec<StyleDef>,
    pub links: Vec<LinkDef>,
    pub rows: Vec<Row>,
    pub history: Vec<Row>,
}

/// The rows that changed since `base_rev`, and every table definition past
/// the client's marks.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct Delta {
    pub state_gen: u64,
    pub table_gen: u32,
    pub base_rev: u64,
    pub rev: u64,
    pub resume: Cursor,
    /// Lines pushed into history since `base_rev`.
    pub scrolled: u32,
    pub term: Option<TermDelta>,
    pub styles: Vec<StyleDef>,
    pub links: Vec<LinkDef>,
    pub rows: Vec<Row>,
}

/// Stable name of a side effect: the record that caused it and its index in
/// that record, the same after a replay.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct EffectId {
    pub session: u128,
    pub epoch: u32,
    pub rseq: u64,
    pub index: u32,
}

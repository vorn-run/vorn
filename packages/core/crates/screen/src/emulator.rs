//! A terminal that can be checkpointed exactly: a libghostty-vt terminal fed
//! in step with a model of Ghostty's parser ([`crate::vtparse`]) and a model
//! of the state Ghostty keeps but does not expose.
//!
//! A checkpoint is the VT that rebuilds a terminal from blank
//! ([`crate::checkpoint`]). Most of what it has to rebuild can be read back
//! from Ghostty: cells, rows, cursor, modes, colors. The rest can only be
//! followed as the stream passes, and that is what [`Tracker`] does: saved
//! cursors, the Kitty keyboard stack, the OSC 133 state of the cursor, the
//! character sets, the screen that is not showing, and what REP would repeat.
//! Where it cannot follow something (XTSAVE'd modes, a title stack, Kitty
//! graphics) it marks the terminal so no checkpoint is cut until a full reset
//! clears it.
//!
//! The terminal also reports what a session engine acts on as effects, in
//! stream order: replies to queries, bells, clipboard writes and desktop
//! notifications.

use std::cell::RefCell;
use std::collections::HashMap;
use std::rc::Rc;

use libghostty_vt::screen::Screen as GhosttyScreen;
use libghostty_vt::style::Style;
use libghostty_vt::terminal::{
    ConformanceLevel, DeviceAttributeFeature, DeviceAttributes, DeviceType, Mode, Options, Point,
    PointCoordinate, PointSpace, PrimaryDeviceAttributes, SecondaryDeviceAttributes,
    SizeReportSize, Terminal, TertiaryDeviceAttributes,
};

use crate::vtparse::{Event, Parser};
use crate::{clip_units, dimension, is_plausible_path, percent_decode, Result, OSC_PRIVATE};

/// What the terminal asks its host to do, in the order the stream asked.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Effect {
    /// Bytes answering a query (device attributes, cursor position, a mode
    /// report), for the program's input. Only a live terminal sends them.
    Reply(Vec<u8>),
    Bell,
    /// An OSC 52 (or iTerm2) clipboard write, decoded. Empty contents clear.
    Clipboard {
        location: ClipboardTarget,
        contents: Vec<(String, String)>,
    },
    /// An OSC 9 or OSC 777 desktop notification.
    Notify {
        title: String,
        body: String,
    },
    /// The cwd an OSC 5522 moved to, once per move.
    Cwd(String),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ClipboardTarget {
    Standard,
    Selection,
    Primary,
}

/// Which of the two screens.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Which {
    Primary,
    Alternate,
}

impl Which {
    fn index(self) -> usize {
        match self {
            Which::Primary => 0,
            Which::Alternate => 1,
        }
    }
}

/// A character set Ghostty can designate. `Utf8` is the initial value,
/// which no sequence designates.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub(crate) enum Cs {
    #[default]
    Utf8,
    Ascii,
    British,
    DecSpecial,
}

impl Cs {
    pub(crate) fn final_byte(self) -> Option<u8> {
        match self {
            Cs::Utf8 => None,
            Cs::Ascii => Some(b'B'),
            Cs::British => Some(b'A'),
            Cs::DecSpecial => Some(b'0'),
        }
    }
}

/// A screen's character set state: four slots, what GL and GR invoke, and
/// a pending single shift.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct Charset {
    pub g: [Cs; 4],
    pub gl: u8,
    pub gr: u8,
    pub single_shift: Option<u8>,
}

impl Default for Charset {
    fn default() -> Self {
        Self {
            g: [Cs::Utf8; 4],
            gl: 0,
            gr: 2,
            single_shift: None,
        }
    }
}

/// What text the cursor writes, per OSC 133.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub(crate) enum Sem {
    #[default]
    Output,
    Input,
    Prompt,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub(crate) struct Semantic {
    pub content: Sem,
    /// Input that ends at the end of the line (OSC 133;I).
    pub clear_eol: bool,
}

/// The kind of character protection last selected; Ghostty never resets it
/// to off, because erasing depends on the last kind used.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub(crate) enum Protect {
    #[default]
    Off,
    Iso,
    Dec,
}

/// Ghostty's Kitty keyboard flag stack: a ring of eight with a cursor.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub(crate) struct Kitty {
    pub flags: [u8; 8],
    pub idx: u8,
}

impl Kitty {
    fn push(&mut self, f: u8) {
        self.idx = (self.idx + 1) % 8;
        self.flags[usize::from(self.idx)] = f;
    }

    fn pop(&mut self, n: u16) {
        if n >= 8 {
            *self = Kitty::default();
            return;
        }
        for _ in 0..n {
            self.flags[usize::from(self.idx)] = 0;
            self.idx = (self.idx + 7) % 8;
        }
    }

    fn set(&mut self, mode: u16, f: u8) {
        let top = &mut self.flags[usize::from(self.idx)];
        match mode {
            1 => *top = f,
            2 => *top |= f,
            3 => *top &= !f,
            _ => {}
        }
    }
}

/// A DECSC: everything Ghostty's `saveCursor` keeps.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct Saved {
    pub x: u16,
    pub y: u16,
    pub style: Style,
    pub protected: bool,
    pub pending_wrap: bool,
    pub origin: bool,
    pub charset: Charset,
}

/// The state of one screen Ghostty keeps but does not expose.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub(crate) struct ScreenModel {
    /// `None` until a DECSC; restoring then resets to the origin, which is
    /// the same as a save of the initial state there.
    pub saved: Option<Saved>,
    pub kitty: Kitty,
    pub semantic: Semantic,
    pub protect: Protect,
    pub protected: bool,
    pub charset: Charset,
}

/// The screen that is not showing, drawn as it was when it was left, with
/// what has happened to it since.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct Capture {
    pub cols: u16,
    pub rows: u16,
    /// The VT that draws it on a blank screen of `cols` x `rows`, or why it
    /// could not be drawn.
    pub part: std::result::Result<Vec<u8>, &'static str>,
    /// The sequence that left it, replayed as it was.
    pub op: Vec<u8>,
    /// Mode 2027 when it was drawn, which decides how its text clusters.
    pub grapheme: bool,
    /// Resizes since: columns, rows, and whether wraparound (which decides
    /// whether the primary screen reflows) was on.
    pub resizes: Vec<(u16, u16, bool)>,
    /// The character sets the showing screen had right after the switch,
    /// which a rebuild starts drawing it with.
    pub entry_charset: Charset,
}

/// Whether the shell redraws its prompt after a resize (OSC 133 `redraw`),
/// which decides what a resize clears. A new terminal from the C API starts
/// at `No`; a full reset puts back Ghostty's own default, `All`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Redraw {
    All,
    No,
    Last,
}

/// Everything the emulator follows beside Ghostty.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct Tracker {
    pub active: Which,
    pub screens: [ScreenModel; 2],
    pub inactive: Option<Capture>,
    /// Why no checkpoint can be cut until a full reset.
    pub taint: Option<&'static str>,
    /// `parser.printed` when a single shift was invoked, and whether a
    /// codepoint that might not consume it was printed since.
    pub shift_at: u64,
    pub redraw: Redraw,
}

impl Tracker {
    /// The state of a terminal just created, which a full reset does not
    /// quite return to (see [`Redraw`]).
    fn new() -> Self {
        Self {
            active: Which::Primary,
            screens: [ScreenModel::default(); 2],
            inactive: None,
            taint: None,
            shift_at: 0,
            redraw: Redraw::No,
        }
    }
}

impl Tracker {
    pub(crate) fn screen(&self) -> &ScreenModel {
        &self.screens[self.active.index()]
    }

    fn screen_mut(&mut self) -> &mut ScreenModel {
        &mut self.screens[self.active.index()]
    }

    fn taint(&mut self, why: &'static str) {
        self.taint.get_or_insert(why);
    }
}

/// What one dispatched sequence asks of the emulator.
enum Hook {
    None,
    Index,
    Invoke {
        gr: bool,
        slot: u8,
    },
    SingleShift(u8),
    Designate {
        slot: u8,
        cs: Cs,
    },
    SaveCursor,
    /// CSI s without parameters: DECSLRM with mode 69, else a save.
    SaveOrMargins,
    RestoreCursor,
    Switch {
        mode: u16,
        set: bool,
    },
    Reset,
    Protect(Option<Protect>),
    KittyPush(u8),
    KittyPop(u16),
    KittySet(u8, u16),
    Semantic(SemOp),
    /// OSC 133;L: a line feed unless the cursor is at the left margin.
    FreshLine,
    /// OSC 133;A or N: a prompt, and maybe a new `redraw` setting.
    PromptStart(Option<Redraw>),
    Notify {
        title: String,
        body: String,
    },
    Label {
        num: u32,
        payload: Vec<u8>,
    },
    Taint(&'static str),
}

/// The OSC 133 commands that move the cursor's semantic state.
enum SemOp {
    Prompt,
    Input { clear_eol: bool },
    Output,
}

/// Which hooks need Ghostty's state around the byte that ends them.
enum Needs {
    /// Only the model changes.
    Model,
    /// Ghostty's state after the byte, or the effect's place in the stream.
    After,
    /// Ghostty's state before the byte as well.
    Around,
    /// Whether the byte moved the cursor.
    Moved,
}

impl Hook {
    fn needs(&self) -> Needs {
        match self {
            Hook::Switch { .. } => Needs::Around,
            Hook::FreshLine => Needs::Moved,
            Hook::SaveCursor
            | Hook::SaveOrMargins
            | Hook::RestoreCursor
            | Hook::Notify { .. }
            | Hook::Label { .. } => Needs::After,
            _ => Needs::Model,
        }
    }
}

/// Answers a device attributes query as Ghostty itself does.
const PRIMARY_DA: PrimaryDeviceAttributes = PrimaryDeviceAttributes::new(
    ConformanceLevel::VT220,
    &[DeviceAttributeFeature::ANSI_COLOR],
);

pub struct Emulator {
    pub(crate) term: Terminal<'static, 'static>,
    effects: Rc<RefCell<Vec<Effect>>>,
    size: Rc<std::cell::Cell<(u16, u16)>>,
    pub(crate) parser: Parser,
    pub(crate) track: Tracker,
    pub(crate) cols: u16,
    pub(crate) rows: u16,
    /// Ghostty's history limit, in bytes of page memory.
    pub(crate) scrollback: usize,
    /// Title and cwd as the session record keeps them (see [`crate::Screen`]).
    pub(crate) title: String,
    pub(crate) cwd: String,
    /// Whether each codepoint above U+00FF sets what REP repeats, which
    /// depends on its width in Ghostty's own tables; found by asking a
    /// scratch terminal once per codepoint.
    counted: HashMap<u32, bool>,
    /// Output bytes handed to Ghostty's parser by [`Emulator::feed`].
    pub(crate) parsed: u64,
    /// Scrollback clears (ED 3) and full resets (RIS) parsed.
    pub(crate) history_clears: u64,
    /// Feeds that printed a zero-width codepoint (see
    /// [`Emulator::joiners_printed`]).
    joiners: u64,
}

impl Emulator {
    /// A terminal that keeps no history, as a session engine runs it.
    pub fn new(cols: u32, rows: u32) -> Result<Self> {
        Self::with_scrollback(cols, rows, 0)
    }

    /// A terminal that keeps history above the screen, up to
    /// `max_scrollback` bytes of Ghostty's page memory (see
    /// [`crate::Screen::with_scrollback`]). Checkpoints carry the history.
    pub fn with_scrollback(cols: u32, rows: u32, max_scrollback: usize) -> Result<Self> {
        let (cols, rows) = (dimension(cols)?, dimension(rows)?);
        let mut term = Terminal::new(Options {
            cols,
            rows,
            max_scrollback,
        })?;
        let effects = Rc::new(RefCell::new(Vec::new()));
        let size = Rc::new(std::cell::Cell::new((cols, rows)));
        {
            let sink = Rc::clone(&effects);
            term.on_pty_write(move |_, bytes| {
                sink.borrow_mut().push(Effect::Reply(bytes.to_vec()))
            })?;
            let sink = Rc::clone(&effects);
            term.on_bell(move |_| sink.borrow_mut().push(Effect::Bell))?;
            let sink = Rc::clone(&effects);
            term.on_clipboard_write(move |_, write| {
                use libghostty_vt::terminal::ClipboardLocation as L;
                let location = match write.location() {
                    L::Selection => ClipboardTarget::Selection,
                    L::Primary => ClipboardTarget::Primary,
                    _ => ClipboardTarget::Standard,
                };
                let contents = write
                    .contents()
                    .map(|c| (c.mime.to_owned(), c.data.to_owned()))
                    .collect();
                sink.borrow_mut()
                    .push(Effect::Clipboard { location, contents });
                Ok(())
            })?;
            term.on_device_attributes(|_| {
                Some(DeviceAttributes {
                    primary: PRIMARY_DA,
                    secondary: SecondaryDeviceAttributes {
                        device_type: DeviceType::VT220,
                        firmware_version: 1,
                        rom_cartridge: 0,
                    },
                    tertiary: TertiaryDeviceAttributes { unit_id: 0 },
                })
            })?;
            let dims = Rc::clone(&size);
            term.on_size(move |_| {
                let (columns, rows) = dims.get();
                Some(SizeReportSize {
                    rows,
                    columns,
                    cell_width: 1,
                    cell_height: 1,
                })
            })?;
            term.on_xtversion(|_| Some("vorn"))?;
        }
        Ok(Self {
            term,
            effects,
            size,
            parser: Parser::default(),
            track: Tracker::new(),
            cols,
            rows,
            scrollback: max_scrollback,
            title: String::new(),
            cwd: String::new(),
            counted: HashMap::new(),
            parsed: 0,
            history_clears: 0,
            joiners: 0,
        })
    }

    pub fn cols(&self) -> u16 {
        self.cols
    }

    pub fn rows(&self) -> u16 {
        self.rows
    }

    /// Ghostty's history limit this terminal was made with, in bytes.
    pub fn scrollback_limit(&self) -> usize {
        self.scrollback
    }

    /// The terminal itself, read only, for comparing states.
    pub fn terminal(&self) -> &Terminal<'static, 'static> {
        &self.term
    }

    /// The last OSC 0/2 title, as the session record keeps it.
    pub fn title(&self) -> &str {
        &self.title
    }

    /// The last cwd from OSC 7 or OSC 5522.
    pub fn cwd(&self) -> &str {
        &self.cwd
    }

    /// How many bytes of output [`Emulator::feed`] has passed to Ghostty's
    /// parser, carried across checkpoint cuts: the one-parse counter (TP-T1).
    /// Each byte fed is passed exactly once, so it equals the bytes fed.
    pub fn parsed_bytes(&self) -> u64 {
        self.parsed
    }

    /// How many feeds printed a codepoint that joined the cell before it
    /// rather than taking its own. Ghostty does not mark a row dirty when a
    /// codepoint joins a cell outside grapheme clustering mode (2027), so a
    /// renderer that trusts dirty rows re-reads every row when this moved.
    pub fn joiners_printed(&self) -> u64 {
        self.joiners
    }

    /// How many times the output cleared the scrollback (ED 3) or reset the
    /// terminal (RIS): a renderer numbering history lines starts again when
    /// this moves.
    pub fn history_clears(&self) -> u64 {
        self.history_clears
    }

    fn write(&mut self, bytes: &[u8]) {
        self.parsed += bytes.len() as u64;
        self.term.vt_write(bytes);
    }

    /// The colours a program sees as the defaults, as `(fg, bg)` RGB: what
    /// an OSC 10 or 11 query is answered with. Ghostty answers neither query
    /// until they are set; `None` unsets them.
    pub fn set_default_colors(&mut self, colors: Option<([u8; 3], [u8; 3])>) -> Result<()> {
        let rgb = |c: [u8; 3]| libghostty_vt::style::RgbColor {
            r: c[0],
            g: c[1],
            b: c[2],
        };
        self.term
            .set_default_fg_color(colors.map(|(fg, _)| rgb(fg)))?;
        self.term
            .set_default_bg_color(colors.map(|(_, bg)| rgb(bg)))?;
        Ok(())
    }

    /// Ghostty's parser is in its ground state with no UTF-8 sequence open.
    pub fn at_ground(&self) -> bool {
        self.parser.at_ground()
    }

    /// Feeds output, appending what it asks of the host to `out`.
    pub fn feed(&mut self, bytes: &[u8], out: &mut Vec<Effect>) {
        let mut pos = 0;
        let mut flushed = 0;
        while pos < bytes.len() {
            let step = self.parser.step(&bytes[pos..]);
            let end = pos + step.consumed;
            let hook = match step.event {
                Some(e) => {
                    if clears_history(&e) {
                        self.history_clears += 1;
                    }
                    hook_of(&e)
                }
                None => Hook::None,
            };
            pos = end;
            if matches!(hook, Hook::None) {
                continue;
            }
            match hook.needs() {
                Needs::Model => self.model(hook),
                Needs::After => {
                    self.write(&bytes[flushed..end]);
                    flushed = end;
                    self.after(hook);
                }
                Needs::Moved => {
                    self.write(&bytes[flushed..end - 1]);
                    let before = self.term.cursor_x().ok();
                    self.write(&bytes[end - 1..end]);
                    flushed = end;
                    if before.is_none() || before != self.term.cursor_x().ok() {
                        self.model(Hook::Index);
                    }
                }
                Needs::Around => {
                    self.write(&bytes[flushed..end - 1]);
                    let before = self.before(&hook);
                    self.write(&bytes[end - 1..end]);
                    flushed = end;
                    self.switched(hook, before);
                }
            }
        }
        self.write(&bytes[flushed..]);
        out.append(&mut self.effects.borrow_mut());
        self.count_joiners();
    }

    /// Whether this feed printed a codepoint Ghostty gives no width, which
    /// joins the cell before it. Each candidate is asked of Ghostty's own
    /// tables once ([`Emulator::counted`]); too many distinct ones in one
    /// feed count as a join without asking.
    fn count_joiners(&mut self) {
        if self.parser.join_candidates.is_empty() && !self.parser.join_overflow {
            return;
        }
        let candidates = std::mem::take(&mut self.parser.join_candidates);
        let joined = std::mem::take(&mut self.parser.join_overflow)
            || candidates.iter().any(|&cp| !self.counted(cp));
        if joined {
            self.joiners += 1;
        }
        // Keep the allocation for the next feed.
        let mut candidates = candidates;
        candidates.clear();
        self.parser.join_candidates = candidates;
    }

    /// Resizes, appending any in-band size report to `out`.
    pub fn resize(&mut self, cols: u32, rows: u32, out: &mut Vec<Effect>) -> Result<()> {
        let (c, r) = (dimension(cols)?, dimension(rows)?);
        if (c, r) == (self.cols, self.rows) {
            return Ok(());
        }
        let wrap = self.term.mode(Mode::WRAPAROUND)?;
        // Ghostty moves the active screen's saved cursor with the text under
        // it, through a tracked pin; a tracked reference follows the same pin.
        let saved = self.track.screen().saved;
        let pin = saved.and_then(|sc| {
            self.term
                .track_grid_ref(Point::Active(PointCoordinate {
                    x: sc.x,
                    y: u32::from(sc.y),
                }))
                .ok()
        });
        self.term.resize(c, r, 1, 1)?;
        self.cols = c;
        self.rows = r;
        self.size.set((c, r));
        if let (Some(mut sc), Some(pin)) = (saved, pin) {
            match pin.point(PointSpace::Active)? {
                Some(pt) => {
                    sc.x = pt.x;
                    sc.y = u16::try_from(pt.y).unwrap_or(u16::MAX);
                    if sc.pending_wrap && sc.x != c - 1 {
                        sc.pending_wrap = false;
                        sc.x += 1;
                    }
                }
                None => {
                    sc.x = 0;
                    sc.y = 0;
                    sc.pending_wrap = false;
                }
            }
            self.track.screen_mut().saved = Some(sc);
        }
        if let Some(cap) = &mut self.track.inactive {
            cap.resizes.push((c, r, wrap));
        }
        out.append(&mut self.effects.borrow_mut());
        Ok(())
    }

    /// Why no checkpoint can be cut at this point, if one cannot.
    pub fn uncuttable(&mut self) -> Option<&'static str> {
        if !self.parser.at_ground() {
            return Some("inside a sequence");
        }
        self.settle_single_shift();
        if let Some(why) = self.track.taint {
            return Some(why);
        }
        if let Some(cap) = &self.track.inactive {
            if let Err(why) = &cap.part {
                return Some(why);
            }
        }
        None
    }

    /// Whether codepoint `cp`, printed with grapheme clustering off, set
    /// what REP repeats: it did unless Ghostty gave it no width.
    pub(crate) fn counted(&mut self, cp: u32) -> bool {
        if cp <= 0xff {
            return true;
        }
        if let Some(&known) = self.counted.get(&cp) {
            return known;
        }
        let known = probe_counted(cp).unwrap_or(false);
        self.counted.insert(cp, known);
        known
    }

    /// A single shift is consumed by the next codepoint printed through a
    /// cell; one above U+00FF may instead have joined the previous cell.
    fn settle_single_shift(&mut self) {
        let at = self.track.shift_at;
        let screen = &mut self.track.screens[self.track.active.index()];
        if screen.charset.single_shift.is_none() || self.parser.printed == at {
            return;
        }
        if self.parser.printed_max <= 0xff {
            screen.charset.single_shift = None;
        } else {
            self.track.taint("single shift unresolved");
        }
    }

    fn model(&mut self, hook: Hook) {
        let t = &mut self.track;
        match hook {
            Hook::Index => {
                let s = &mut t.screen_mut().semantic;
                if s.clear_eol {
                    *s = Semantic::default();
                }
            }
            Hook::Invoke { gr, slot } => {
                let cs = &mut t.screen_mut().charset;
                if gr {
                    cs.gr = slot;
                } else {
                    cs.gl = slot;
                }
            }
            Hook::SingleShift(slot) => {
                t.screen_mut().charset.single_shift = Some(slot);
                t.shift_at = self.parser.printed;
                self.parser.printed_max = 0;
            }
            Hook::Designate { slot, cs } => t.screen_mut().charset.g[usize::from(slot)] = cs,
            Hook::Reset => {
                *t = Tracker {
                    redraw: Redraw::All,
                    ..Tracker::new()
                };
                self.parser.recent.clear();
            }
            Hook::Protect(p) => {
                let s = t.screen_mut();
                match p {
                    Some(kind) => {
                        s.protected = true;
                        s.protect = kind;
                    }
                    None => s.protected = false,
                }
            }
            Hook::KittyPush(f) => t.screen_mut().kitty.push(f),
            Hook::KittyPop(n) => t.screen_mut().kitty.pop(n),
            Hook::KittySet(f, mode) => t.screen_mut().kitty.set(mode, f),
            Hook::Semantic(op) => {
                t.screen_mut().semantic = match op {
                    SemOp::Prompt => Semantic {
                        content: Sem::Prompt,
                        clear_eol: false,
                    },
                    SemOp::Input { clear_eol } => Semantic {
                        content: Sem::Input,
                        clear_eol,
                    },
                    SemOp::Output => Semantic::default(),
                }
            }
            Hook::PromptStart(redraw) => {
                t.screen_mut().semantic = Semantic {
                    content: Sem::Prompt,
                    clear_eol: false,
                };
                if let Some(r) = redraw {
                    t.redraw = r;
                }
            }
            Hook::Taint(why) => t.taint(why),
            Hook::None
            | Hook::SaveCursor
            | Hook::SaveOrMargins
            | Hook::RestoreCursor
            | Hook::Switch { .. }
            | Hook::FreshLine
            | Hook::Notify { .. }
            | Hook::Label { .. } => {}
        }
    }

    fn after(&mut self, hook: Hook) {
        match hook {
            Hook::SaveCursor => self.save_cursor(),
            Hook::SaveOrMargins => {
                if !self.term.mode(Mode::LEFT_RIGHT_MARGIN).unwrap_or(false) {
                    self.save_cursor();
                }
            }
            Hook::RestoreCursor => self.restore_cursor(),
            Hook::Notify { title, body } => self
                .effects
                .borrow_mut()
                .push(Effect::Notify { title, body }),
            Hook::Label { num, payload } => self.label(num, &payload),
            other => self.model(other),
        }
    }

    fn save_cursor(&mut self) {
        self.settle_single_shift();
        let saved = self.cursor_state();
        self.track.screen_mut().saved = saved;
        if saved.is_none() {
            self.track.taint("cursor unreadable");
        }
    }

    pub(crate) fn cursor_state(&self) -> Option<Saved> {
        let s = self.track.screen();
        Some(Saved {
            x: self.term.cursor_x().ok()?,
            y: self.term.cursor_y().ok()?,
            style: self.term.cursor_style().ok()?,
            protected: s.protected,
            pending_wrap: self.term.is_cursor_pending_wrap().ok()?,
            origin: self.term.mode(Mode::ORIGIN).ok()?,
            charset: s.charset,
        })
    }

    fn restore_cursor(&mut self) {
        let s = self.track.screen_mut();
        let saved = s.saved;
        s.charset = saved.map(|v| v.charset).unwrap_or_default();
        s.protected = saved.is_some_and(|v| v.protected);
    }

    /// Before an alternate-screen mode's final byte: the screen about to be
    /// left, drawn as it is, if it will be left.
    fn before(&mut self, hook: &Hook) -> Option<(Option<Saved>, Capture)> {
        let &Hook::Switch { mode, set } = hook else {
            return None;
        };
        self.settle_single_shift();
        let leaving = matches!(
            (set, self.track.active),
            (true, Which::Primary) | (false, Which::Alternate)
        );
        // 1049 saves the cursor even when no switch happens.
        let saved = (mode == 1049 && set).then(|| self.cursor_state()).flatten();
        if !leaving {
            return Some((saved, Capture::empty()));
        }
        let part = if self.term.mode(Mode::ORIGIN).unwrap_or(true) {
            Err("origin mode at a screen switch")
        } else {
            crate::checkpoint::screen_part(self)
        };
        let capture = Capture {
            cols: self.cols,
            rows: self.rows,
            part,
            op: format!("\x1b[?{mode}{}", if set { 'h' } else { 'l' }).into_bytes(),
            grapheme: self.term.mode(Mode::GRAPHEME_CLUSTER).unwrap_or(false),
            resizes: Vec::new(),
            entry_charset: Charset::default(),
        };
        Some((saved, capture))
    }

    /// After an alternate-screen mode: the model follows what Ghostty did.
    fn switched(&mut self, hook: Hook, before: Option<(Option<Saved>, Capture)>) {
        let (Hook::Switch { mode, set }, Some((saved, capture))) = (hook, before) else {
            return;
        };
        let was = self.track.active;
        if mode == 1049 && set {
            self.track.screen_mut().saved = saved;
        }
        let now = match self.term.active_screen() {
            Ok(GhosttyScreen::Alternate) => Which::Alternate,
            Ok(_) => Which::Primary,
            Err(_) => {
                self.track.taint("screen unreadable");
                return;
            }
        };
        if now == was {
            // Leaving 1049 restores the primary cursor even from the primary.
            if mode == 1049 && !set {
                self.restore_cursor();
            }
            return;
        }
        let t = &mut self.track;
        let old = t.screens[was.index()];
        let new = &mut t.screens[now.index()];
        // switchScreen brings the character sets along; every mode but
        // leaving 1049 also copies the cursor, and with it the semantic
        // state and protection.
        new.charset = old.charset;
        if !(mode == 1049 && !set) {
            new.semantic = old.semantic;
            new.protected = old.protected;
        }
        t.active = now;
        t.inactive = Some(capture);
        if mode == 1049 && !set {
            self.restore_cursor();
        }
        let charset = self.track.screen().charset;
        if let Some(cap) = &mut self.track.inactive {
            cap.entry_charset = charset;
        }
    }

    fn label(&mut self, num: u32, payload: &[u8]) {
        match num {
            0 | 2 => self.title = clip_units(&String::from_utf8_lossy(payload)),
            7 => {
                let raw = String::from_utf8_lossy(payload);
                let path = match raw.strip_prefix("file://") {
                    Some(rest) => rest.find('/').map_or("", |at| &rest[at..]),
                    None => &raw,
                };
                if !path.is_empty() {
                    self.cwd = clip_units(&percent_decode(path).unwrap_or_else(|| path.to_owned()));
                }
            }
            OSC_PRIVATE => {
                if let Some(rest) = payload.strip_prefix(b"cwd;") {
                    let next = clip_units(&String::from_utf8_lossy(rest));
                    if !next.is_empty() && is_plausible_path(&next) && next != self.cwd {
                        self.cwd = next.clone();
                        self.effects.borrow_mut().push(Effect::Cwd(next));
                    }
                }
            }
            _ => {}
        }
    }
}

impl Capture {
    fn empty() -> Self {
        Capture {
            cols: 0,
            rows: 0,
            part: Err("no capture"),
            op: Vec::new(),
            grapheme: false,
            resizes: Vec::new(),
            entry_charset: Charset::default(),
        }
    }
}

/// Prints `cp` after a narrow character on a scratch terminal, then REP:
/// the cursor ends past column 2 only if `cp` became what REP repeats.
fn probe_counted(cp: u32) -> Option<bool> {
    let ch = char::from_u32(cp)?;
    let mut t = Terminal::new(Options {
        cols: 16,
        rows: 1,
        max_scrollback: 0,
    })
    .ok()?;
    let mut bytes = b"a".to_vec();
    let mut buf = [0; 4];
    bytes.extend_from_slice(ch.encode_utf8(&mut buf).as_bytes());
    bytes.extend_from_slice(b"\x1b[b");
    t.vt_write(&bytes);
    Some(t.cursor_x().ok()? >= 3)
}

/// What a dispatched sequence means to the emulator.
/// ED 3, which erases the scrollback, or RIS.
fn clears_history(e: &Event<'_>) -> bool {
    match *e {
        Event::Esc { inter, fin } => inter.is_empty() && fin == b'c',
        Event::Csi { inter, params, fin } => {
            fin == b'J' && inter.is_empty() && params.first() == Some(&3)
        }
        _ => false,
    }
}

fn hook_of(e: &Event<'_>) -> Hook {
    match *e {
        Event::Execute(c) => match c {
            0x0a..=0x0c => Hook::Index,
            0x0e => Hook::Invoke { gr: false, slot: 1 },
            0x0f => Hook::Invoke { gr: false, slot: 0 },
            _ => Hook::None,
        },
        Event::Esc { inter, fin } => esc_hook(inter, fin),
        Event::Csi { inter, params, fin } => csi_hook(inter, params, fin),
        Event::Osc(payload) => osc_hook(payload),
        Event::ApcStart => Hook::Taint("APC string"),
        Event::Dcs { params, fin, .. } => {
            if fin == b'p' && params.first() == Some(&1000) {
                Hook::Taint("tmux control mode")
            } else {
                Hook::None
            }
        }
    }
}

fn esc_hook(inter: &[u8], fin: u8) -> Hook {
    if let [i] = inter {
        let slot = match i {
            b'(' => 0,
            b')' => 1,
            b'*' => 2,
            b'+' => 3,
            _ => return Hook::None,
        };
        let cs = match fin {
            b'B' => Cs::Ascii,
            b'A' => Cs::British,
            b'0' => Cs::DecSpecial,
            _ => return Hook::None,
        };
        return Hook::Designate { slot, cs };
    }
    if !inter.is_empty() {
        return Hook::None;
    }
    match fin {
        b'7' => Hook::SaveCursor,
        b'8' => Hook::RestoreCursor,
        b'D' | b'E' => Hook::Index,
        b'N' => Hook::SingleShift(2),
        b'O' => Hook::SingleShift(3),
        b'V' => Hook::Protect(Some(Protect::Iso)),
        b'W' => Hook::Protect(None),
        b'c' => Hook::Reset,
        b'n' => Hook::Invoke { gr: false, slot: 2 },
        b'o' => Hook::Invoke { gr: false, slot: 3 },
        b'~' => Hook::Invoke { gr: true, slot: 1 },
        b'}' => Hook::Invoke { gr: true, slot: 2 },
        b'|' => Hook::Invoke { gr: true, slot: 3 },
        _ => Hook::None,
    }
}

fn csi_hook(inter: &[u8], params: &[u16], fin: u8) -> Hook {
    match (inter, fin) {
        (b"?", b'h' | b'l') => {
            let set = fin == b'h';
            let tracked = |p: &u16| matches!(p, 47 | 1047 | 1048 | 1049);
            match params {
                [m @ (47 | 1047 | 1049)] => Hook::Switch { mode: *m, set },
                [1048] if set => Hook::SaveCursor,
                [1048] => Hook::RestoreCursor,
                _ if params.iter().any(tracked) => Hook::Taint("screen modes combined"),
                _ => Hook::None,
            }
        }
        (b"", b's') if params.is_empty() => Hook::SaveOrMargins,
        (b"", b'u') => Hook::RestoreCursor,
        (b"?", b's') => Hook::Taint("XTSAVE"),
        (b"\"", b'q') => match params {
            [] | [0] | [2] => Hook::Protect(None),
            [1] => Hook::Protect(Some(Protect::Dec)),
            _ => Hook::None,
        },
        (b">", b'u') => {
            let f = match params {
                [f] => *f,
                _ => 0,
            };
            match u8::try_from(f) {
                Ok(f) if f < 32 => Hook::KittyPush(f),
                _ => Hook::None,
            }
        }
        (b"<", b'u') => Hook::KittyPop(match params {
            [n] => *n,
            _ => 1,
        }),
        (b"=", b'u') => {
            let f = params.first().copied().unwrap_or(0);
            let mode = params.get(1).copied().unwrap_or(1);
            match u8::try_from(f) {
                Ok(f) if f < 32 && (1..=3).contains(&mode) => Hook::KittySet(f, mode),
                _ => Hook::None,
            }
        }
        (b"", b't') => match params {
            [22, 0 | 2] | [22, 0 | 2, _] => Hook::Taint("title stack"),
            _ => Hook::None,
        },
        (b"$", b'}') if params != [0] => Hook::Taint("status display"),
        _ => Hook::None,
    }
}

fn osc_hook(payload: &[u8]) -> Hook {
    let (num, rest) = match payload.iter().position(|&b| b == b';') {
        Some(at) => (&payload[..at], &payload[at + 1..]),
        None => (payload, &[][..]),
    };
    let Some(num) = std::str::from_utf8(num)
        .ok()
        .and_then(|n| n.parse::<u32>().ok())
    else {
        return Hook::None;
    };
    // An OSC with no `;` and no payload carries nothing a caller tracks.
    match num {
        0 | 2 | 7 | OSC_PRIVATE if payload.len() > num_len(num) => Hook::Label {
            num,
            payload: rest.to_vec(),
        },
        9 => notify_9(rest),
        777 => notify_777(rest),
        133 => semantic_hook(rest),
        _ => Hook::None,
    }
}

fn num_len(n: u32) -> usize {
    n.to_string().len()
}

/// OSC 9 is a notification unless it is one of ConEmu's numbered commands.
fn notify_9(rest: &[u8]) -> Hook {
    let digits = rest.iter().take_while(|b| b.is_ascii_digit()).count();
    if digits > 0 && (digits == rest.len() || rest[digits] == b';') {
        return Hook::None;
    }
    Hook::Notify {
        title: String::new(),
        body: String::from_utf8_lossy(rest).into_owned(),
    }
}

/// `OSC 777;notify;title;body`.
fn notify_777(rest: &[u8]) -> Hook {
    let Some(rest) = rest.strip_prefix(b"notify;") else {
        return Hook::None;
    };
    let (title, body) = match rest.iter().position(|&b| b == b';') {
        Some(at) => (&rest[..at], &rest[at + 1..]),
        None => (rest, &[][..]),
    };
    Hook::Notify {
        title: String::from_utf8_lossy(title).into_owned(),
        body: String::from_utf8_lossy(body).into_owned(),
    }
}

/// OSC 133 as Ghostty parses it: an action letter, then `;`-separated
/// options, of which only `redraw` is state the model follows.
fn semantic_hook(data: &[u8]) -> Hook {
    // Ghostty captures the payload in a 2 KiB buffer and drops longer ones.
    if data.is_empty() || data.len() > 2048 {
        return Hook::None;
    }
    let (action, opts) = (data[0], &data[1..]);
    if action == b'L' {
        return if opts.is_empty() {
            Hook::FreshLine
        } else {
            Hook::None
        };
    }
    let opts = match opts {
        [] => &[][..],
        [b';', rest @ ..] => rest,
        _ => return Hook::None,
    };
    let op = match action {
        b'A' | b'N' => {
            let redraw = opts
                .split(|&b| b == b';')
                .find_map(|o| o.strip_prefix(b"redraw="))
                .and_then(|v| match v {
                    b"0" => Some(Redraw::No),
                    b"1" => Some(Redraw::All),
                    b"last" => Some(Redraw::Last),
                    _ => None,
                });
            return Hook::PromptStart(redraw);
        }
        b'P' => SemOp::Prompt,
        b'B' => SemOp::Input { clear_eol: false },
        b'I' => SemOp::Input { clear_eol: true },
        b'C' | b'D' => SemOp::Output,
        _ => return Hook::None,
    };
    Hook::Semantic(op)
}

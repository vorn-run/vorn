//! Interaction: what the pointer is over, what has focus, which menu is
//! open, and how pointer, keyboard and screen-reader input turn into
//! [`Event`]s for the app.
//!
//! Hit-testing uses the boxes the last [`Ui::layout`] recorded, the way a
//! browser tests against the last layout: input arrives between frames.
//! Widgets read this state while building the next frame (`ui.hover(id)`,
//! `ui.menu_open(id)`), so there are no callbacks and no retained widgets.

use std::collections::HashMap;
use std::rc::Rc;
use std::time::{Duration, Instant};

use accesskit::{ActionData, ActionRequest};

use crate::edit::{Outcome, TextEdit};
use crate::element::{Action, Cursor, Id, Sense};
use crate::input::Input;
use crate::scene::Rect;
use crate::text::Shaped;
use crate::theme;
use crate::Ui;

/// Something the app should act on, reported by [`Ui::take_events`].
#[derive(Debug, Clone, PartialEq)]
pub enum Event {
    /// An element with [`Action::Click`] was activated.
    Click(Id),
    /// Item `index` of `group` was chosen (a tab, a pill, a menu entry).
    Select { group: Id, index: usize },
    /// The text of an edit field changed.
    Changed(Id),
    /// Enter in an edit field (Shift+Enter in a composer adds a line).
    Submit(Id),
    /// A split divider moved; `ratio` is the new share of the first side.
    Resized { id: Id, ratio: f32 },
    /// The drag of a split divider ended at `ratio`: the time to save it.
    ResizeEnd { id: Id, ratio: f32 },
    /// A menu closed without a choice (Escape, a click outside).
    Dismiss(Id),
}

/// How long the pointer rests on an element before its tooltip shows.
pub const TOOLTIP_DELAY: Duration = Duration::from_millis(400);
/// How long a menu takes to grow in.
pub const MENU_IN: Duration = Duration::from_millis(200);
/// Half a caret blink.
pub const BLINK: Duration = Duration::from_millis(500);
const DOUBLE_CLICK: Duration = Duration::from_millis(500);
/// Smallest split share either side may shrink to.
pub const MIN_SPLIT: f32 = 0.15;

/// A box that takes input, as the last layout placed it (logical pixels,
/// already clipped by its scrolling ancestors).
#[derive(Debug, Clone)]
pub(crate) struct Hit {
    pub id: Id,
    pub rect: Rect,
    pub sense: Sense,
    pub action: Option<Action>,
    pub cursor: Option<Cursor>,
    /// 0 for the window's tree, 1.. for each overlay, in paint order.
    pub layer: u32,
    pub tip: bool,
    /// The parent's box: what a split divider's ratio is measured against.
    pub parent: Rect,
    pub vertical: bool,
}

/// A hover fade: CSS `transition-colors` for one element.
#[derive(Debug, Clone, Copy)]
pub(crate) struct Fade {
    on: bool,
    /// Linear progress (before easing) when `on` last changed.
    from: f32,
    at: Instant,
}

impl Fade {
    fn linear(&self, now: Instant) -> f32 {
        let dt =
            now.saturating_duration_since(self.at).as_secs_f32() / theme::TRANSITION.as_secs_f32();
        if self.on {
            (self.from + dt).min(1.0)
        } else {
            (self.from - dt).max(0.0)
        }
    }

    fn settled(&self, now: Instant) -> bool {
        let v = self.linear(now);
        v <= 0.0 || v >= 1.0
    }
}

#[derive(Debug, Clone, Copy)]
pub(crate) struct Menu {
    pub id: Id,
    pub trigger: Id,
    pub opened: Instant,
    /// Opened from the keyboard: focus moves into it once it is laid out.
    pub focus_in: bool,
}

#[derive(Debug, Clone, Copy, Default)]
pub(crate) struct Scroll {
    pub offset: f32,
    pub max: f32,
}

/// Where an edit field's text was drawn, to place clicks and the caret.
#[derive(Clone)]
pub(crate) struct EditGeom {
    pub shaped: Rc<Shaped>,
    /// Physical pixels of the text's top-left, after scrolling.
    pub origin: (f32, f32),
}

pub(crate) struct State {
    pub now: Instant,
    pub pointer: Option<(f32, f32)>,
    /// Ids under the pointer in its topmost layer, outermost first.
    pub under: Vec<Id>,
    pub pressed: Option<Id>,
    pub focus: Option<Id>,
    pub focus_visible: bool,
    pub fades: HashMap<Id, Fade>,
    /// The tooltip owner under the pointer, and since when.
    pub tip: Option<(Id, Instant)>,
    pub menu: Option<Menu>,
    pub scroll: HashMap<Id, Scroll>,
    pub drag: Option<Id>,
    pub selecting: Option<Id>,
    pub last_click: Option<(Id, Instant)>,
    pub edits: HashMap<Id, TextEdit>,
    pub geoms: HashMap<Id, EditGeom>,
    pub ratios: HashMap<Id, f32>,
    /// When the caret last moved: it stays lit for a blink after.
    pub blink_from: Instant,
    pub events: Vec<Event>,
    pub hits: Vec<Hit>,
    pub focus_order: Vec<Id>,
    /// Each select group's items in order, and whether each is selected.
    pub groups: HashMap<Id, Vec<(Id, bool)>>,
    pub cursor: Cursor,
    pub ime: Option<Rect>,
}

impl State {
    pub fn new(now: Instant) -> State {
        State {
            now,
            pointer: None,
            under: Vec::new(),
            pressed: None,
            focus: None,
            focus_visible: false,
            fades: HashMap::new(),
            tip: None,
            menu: None,
            scroll: HashMap::new(),
            drag: None,
            selecting: None,
            last_click: None,
            edits: HashMap::new(),
            geoms: HashMap::new(),
            ratios: HashMap::new(),
            blink_from: now,
            events: Vec::new(),
            hits: Vec::new(),
            focus_order: Vec::new(),
            groups: HashMap::new(),
            cursor: Cursor::Default,
            ime: None,
        }
    }

    pub fn hit(&self, id: Id) -> Option<&Hit> {
        self.hits.iter().rev().find(|h| h.id == id)
    }

    /// Whether anything is mid-animation at `now`.
    pub fn animating(&self, now: Instant) -> bool {
        self.fades.values().any(|f| !f.settled(now))
            || self
                .menu
                .is_some_and(|m| now.saturating_duration_since(m.opened) < MENU_IN)
    }

    fn focused_edit(&self) -> Option<Id> {
        let id = self.focus?;
        let h = self.hit(id)?;
        (h.sense.has(Sense::TEXT) && self.edits.contains_key(&id)).then_some(id)
    }

    /// The hits in the topmost layer that holds `p`, in paint order.
    fn stack(&self, p: (f32, f32)) -> impl Iterator<Item = &Hit> {
        let top = self
            .hits
            .iter()
            .filter(|h| h.rect.contains(p.0, p.1))
            .map(|h| h.layer)
            .max();
        self.hits
            .iter()
            .filter(move |h| Some(h.layer) == top && h.rect.contains(p.0, p.1))
    }

    /// Recomputes what the pointer is over, after it moved or the layout
    /// under it changed.
    pub fn rehover(&mut self) {
        let now = self.now;
        let (under, cursor, tip): (Vec<Id>, Cursor, Option<Id>) = match self.pointer {
            None => (Vec::new(), Cursor::Default, None),
            Some(p) => {
                let stack: Vec<&Hit> = self.stack(p).collect();
                let under = stack
                    .iter()
                    .filter(|h| h.sense.has(Sense::HOVER))
                    .map(|h| h.id)
                    .collect();
                let cursor = stack
                    .iter()
                    .rev()
                    .find_map(|h| h.cursor.or_else(|| default_cursor(h)))
                    .unwrap_or_default();
                let tip = stack.iter().rev().find(|h| h.tip).map(|h| h.id);
                (under, cursor, tip)
            }
        };
        for id in &under {
            if !self.under.contains(id) {
                self.fade(*id, true, now);
            }
        }
        for id in std::mem::take(&mut self.under) {
            if !under.contains(&id) {
                self.fade(id, false, now);
            }
        }
        self.under = under;
        self.cursor = match self.drag.and_then(|d| self.hit(d)) {
            Some(h) => h.cursor.or_else(|| default_cursor(h)).unwrap_or_default(),
            None => cursor,
        };
        if self.tip.map(|t| t.0) != tip {
            self.tip = tip.map(|t| (t, now));
        }
    }

    fn fade(&mut self, id: Id, on: bool, now: Instant) {
        let from = self.fades.get(&id).map_or(0.0, |f| f.linear(now));
        self.fades.insert(id, Fade { on, from, at: now });
    }

    /// Drops fades that finished fading out, so the map does not grow with
    /// every element the pointer ever crossed.
    pub fn gc(&mut self) {
        let now = self.now;
        self.fades.retain(|_, f| f.on || f.linear(now) > 0.0);
    }
}

fn default_cursor(h: &Hit) -> Option<Cursor> {
    if h.sense.has(Sense::TEXT) {
        Some(Cursor::Text)
    } else if h.sense.has(Sense::DRAG) {
        Some(if h.vertical {
            Cursor::ColResize
        } else {
            Cursor::RowResize
        })
    } else {
        None
    }
}

/// The platform's shortcut set: macOS or everything else.
fn mac() -> bool {
    cfg!(target_os = "macos")
}

impl Ui {
    /// Sets the clock input is stamped with. A window calls it as events
    /// arrive; tests drive it with [`Ui::begin_at`] instead.
    pub fn set_now(&mut self, now: Instant) {
        self.st.now = now;
    }

    /// Whether `id` is under the pointer (CSS `:hover`, ancestors included).
    pub fn hovered(&self, id: Id) -> bool {
        self.st.under.contains(&id)
    }

    /// How far `id`'s hover transition has run, eased: 0 at rest, 1 hovered.
    pub fn hover(&self, id: Id) -> f32 {
        self.st
            .fades
            .get(&id)
            .map_or(0.0, |f| theme::ease_in_out(f.linear(self.st.now)))
    }

    pub fn pressed(&self, id: Id) -> bool {
        self.st.pressed == Some(id)
    }

    pub fn focused(&self, id: Id) -> bool {
        self.st.focus == Some(id)
    }

    /// The focused element, if any.
    pub fn focus(&self) -> Option<Id> {
        self.st.focus
    }

    /// Moves keyboard focus to `id` (or nowhere), as a click would.
    pub fn set_focus(&mut self, id: Option<Id>) {
        if self.st.focus != id {
            self.st.focus = id;
            self.st.blink_from = self.st.now;
        }
    }

    pub fn menu_open(&self, menu: Id) -> bool {
        self.st.menu.is_some_and(|m| m.id == menu)
    }

    /// How far the open menu's entrance has run, eased: 0 just opened, 1 done.
    pub fn menu_progress(&self, menu: Id) -> f32 {
        match self.st.menu {
            Some(m) if m.id == menu => {
                let t = self
                    .st
                    .now
                    .saturating_duration_since(m.opened)
                    .as_secs_f32()
                    / MENU_IN.as_secs_f32();
                ease_out(t.min(1.0))
            }
            _ => 0.0,
        }
    }

    /// Opens `menu` from `trigger`, closing any other.
    pub fn open_menu(&mut self, menu: Id, trigger: Id) {
        self.st.menu = Some(Menu {
            id: menu,
            trigger,
            opened: self.st.now,
            focus_in: false,
        });
    }

    /// Closes the open menu; focus goes back to its trigger if it was inside.
    pub fn close_menu(&mut self) {
        if let Some(m) = self.st.menu.take() {
            let inside = self.st.focus.is_some_and(|f| {
                self.st
                    .groups
                    .get(&m.id)
                    .is_some_and(|g| g.iter().any(|i| i.0 == f))
            });
            if inside {
                self.st.focus = Some(m.trigger);
            }
        }
    }

    /// Whether `id`'s tooltip should show: the pointer has rested on it for
    /// [`TOOLTIP_DELAY`] and no menu is open.
    pub fn tooltip_shown(&self, id: Id) -> bool {
        self.st.menu.is_none()
            && self.st.pressed.is_none()
            && self
                .st
                .tip
                .is_some_and(|(t, since)| t == id && self.st.now >= since + TOOLTIP_DELAY)
    }

    /// The edit field `id`, made empty if it does not exist yet.
    pub fn edit_mut(&mut self, id: Id) -> &mut TextEdit {
        self.st.edits.entry(id).or_default()
    }

    /// The text of edit field `id` ("" before it exists).
    pub fn edit_text(&self, id: Id) -> &str {
        self.st.edits.get(&id).map_or("", TextEdit::text)
    }

    pub fn set_edit_text(&mut self, id: Id, text: &str) {
        self.edit_mut(id).set_text(text);
    }

    /// The share of split `id` taken by its first side.
    pub fn split_ratio(&self, id: Id, default: f32) -> f32 {
        self.st.ratios.get(&id).copied().unwrap_or(default)
    }

    pub fn set_split_ratio(&mut self, id: Id, ratio: f32) {
        self.st.ratios.insert(id, clamp_ratio(ratio));
    }

    /// How far scroll view `id` is scrolled, logical pixels.
    pub fn scroll_offset(&self, id: Id) -> f32 {
        self.st.scroll.get(&id).map_or(0.0, |s| s.offset)
    }

    /// The events since the last call, oldest first.
    pub fn take_events(&mut self) -> Vec<Event> {
        std::mem::take(&mut self.st.events)
    }

    /// The pointer shape for where the pointer is.
    pub fn cursor(&self) -> Cursor {
        self.st.cursor
    }

    /// Whether something is animating, so the next frame should come on
    /// the display's beat.
    pub fn animating(&self) -> bool {
        self.st.animating(self.st.now)
    }

    /// When a frame is next needed without input: a tooltip's delay
    /// running out, the caret blinking. `None` when nothing is pending.
    pub fn next_wake(&self) -> Option<Instant> {
        let now = self.st.now;
        if self.animating() {
            return Some(now);
        }
        let tip = self
            .st
            .tip
            .map(|(_, since)| since + TOOLTIP_DELAY)
            .filter(|t| *t > now);
        let blink = self.st.focused_edit().map(|_| {
            let n = now
                .saturating_duration_since(self.st.blink_from)
                .as_millis()
                / BLINK.as_millis();
            self.st.blink_from + BLINK * (n as u32 + 1)
        });
        tip.into_iter().chain(blink).min()
    }

    /// Whether keys should go through the IME: a text field has focus.
    pub fn wants_ime(&self) -> bool {
        self.st.focused_edit().is_some()
    }

    /// The caret of the focused text field, logical pixels, for the IME's
    /// candidate window.
    pub fn ime_area(&self) -> Option<Rect> {
        self.st.ime
    }

    pub(crate) fn caret_lit(&self) -> bool {
        let n = self
            .st
            .now
            .saturating_duration_since(self.st.blink_from)
            .as_millis()
            / BLINK.as_millis();
        n.is_multiple_of(2)
    }

    /// The pointer moved to `(x, y)`, logical pixels.
    pub fn pointer_move(&mut self, x: f32, y: f32) {
        self.st.pointer = Some((x, y));
        if let Some(id) = self.st.drag {
            if let Some(h) = self.st.hit(id) {
                let (pos, start, extent) = if h.vertical {
                    (x, h.parent.x, h.parent.w)
                } else {
                    (y, h.parent.y, h.parent.h)
                };
                if extent > 0.0 {
                    let ratio = clamp_ratio((pos - start) / extent);
                    self.st.ratios.insert(id, ratio);
                    self.st.events.push(Event::Resized { id, ratio });
                }
            }
        } else if let Some(id) = self.st.selecting {
            if let Some(i) = self.edit_index_at(id, (x, y)) {
                self.edit_mut(id).move_to(i, true);
                self.st.blink_from = self.st.now;
            }
        }
        self.st.rehover();
    }

    /// The pointer left the window.
    pub fn pointer_leave(&mut self) {
        if self.st.drag.is_none() {
            self.st.pointer = None;
            self.st.rehover();
        }
    }

    /// The primary button went down at the pointer.
    pub fn pointer_down(&mut self) {
        let Some(p) = self.st.pointer else {
            return;
        };
        let now = self.st.now;
        let top = self
            .st
            .stack(p)
            .filter(|h| {
                h.action.is_some()
                    || h.sense.has(Sense::CLICK)
                    || h.sense.has(Sense::FOCUS)
                    || h.sense.has(Sense::DRAG)
            })
            .last()
            .cloned();
        if let Some(m) = self.st.menu {
            // A press outside the menu closes it and still lands, as a
            // document mousedown listener would let it.
            let inside = top
                .as_ref()
                .is_some_and(|h| h.layer > 0 || h.id == m.trigger);
            if !inside {
                self.st.menu = None;
                self.st.events.push(Event::Dismiss(m.id));
            }
        }
        self.st.focus_visible = false;
        let Some(h) = top else {
            self.set_focus(None);
            return;
        };
        if h.sense.has(Sense::FOCUS) {
            self.set_focus(Some(h.id));
        }
        if h.sense.has(Sense::TEXT) {
            let double = self.st.last_click.is_some_and(|(id, at)| {
                id == h.id && now.saturating_duration_since(at) < DOUBLE_CLICK
            });
            self.st.last_click = Some((h.id, now));
            if let Some(i) = self.edit_index_at(h.id, p) {
                let e = self.edit_mut(h.id);
                if double {
                    e.select_word(i);
                } else {
                    e.move_to(i, false);
                }
            }
            self.st.selecting = (!double).then_some(h.id);
            self.st.blink_from = now;
            return;
        }
        if h.sense.has(Sense::DRAG) {
            self.st.drag = Some(h.id);
            self.st.pressed = Some(h.id);
            return;
        }
        // Menu entries choose on press, so a press-drag-release works.
        if matches!(h.action, Some(Action::Select { .. })) && h.layer > 0 {
            self.activate(h.id);
            return;
        }
        if h.action.is_some() {
            self.st.pressed = Some(h.id);
        }
    }

    /// The primary button went up.
    pub fn pointer_up(&mut self) {
        self.st.selecting = None;
        if let Some(id) = self.st.drag.take() {
            self.st.pressed = None;
            let ratio = self.split_ratio(id, 0.5);
            self.st.events.push(Event::ResizeEnd { id, ratio });
            self.st.rehover();
            return;
        }
        let Some(pressed) = self.st.pressed.take() else {
            return;
        };
        let Some(p) = self.st.pointer else {
            return;
        };
        let released_on = self.st.stack(p).any(|h| h.id == pressed);
        if released_on {
            self.activate(pressed);
        }
    }

    /// The wheel or trackpad scrolled by `dy` logical pixels (positive
    /// moves the content up, showing what is below).
    pub fn wheel(&mut self, dy: f32) {
        let Some(p) = self.st.pointer else {
            return;
        };
        let target = self
            .st
            .stack(p)
            .filter(|h| h.sense.has(Sense::SCROLL))
            .filter(|h| {
                let s = self.st.scroll.get(&h.id).copied().unwrap_or_default();
                // Chains outward when this one is already at its end.
                (dy > 0.0 && s.offset < s.max) || (dy < 0.0 && s.offset > 0.0)
            })
            .last()
            .map(|h| h.id);
        if let Some(id) = target {
            let s = self.st.scroll.entry(id).or_default();
            s.offset = (s.offset + dy).clamp(0.0, s.max);
            // A tooltip anchored to something that moved would float loose.
            self.st.tip = None;
        }
    }

    /// A key or IME event. Returns whether the UI used it; if not, the app
    /// may (a terminal takes everything a text field did not).
    pub fn key(&mut self, input: &Input) -> bool {
        if let Some(id) = self.st.focused_edit() {
            if self.edit_key(id, input) {
                return true;
            }
        }
        let Input::Key { code, mods, .. } = input else {
            return false;
        };
        let shift = mods & crate::input::mods::SHIFT != 0;
        if let Some(m) = self.st.menu {
            match code.as_str() {
                "Escape" => {
                    self.close_menu();
                    self.st.events.push(Event::Dismiss(m.id));
                    return true;
                }
                "ArrowDown" | "ArrowUp" | "Home" | "End" => {
                    self.move_in_group(m.id, code);
                    self.st.focus_visible = true;
                    return true;
                }
                "Tab" => {
                    self.close_menu();
                }
                _ => {}
            }
        }
        match code.as_str() {
            "Tab" if *mods & !crate::input::mods::SHIFT == 0 => {
                self.tab(shift);
                true
            }
            "Enter" | "NumpadEnter" | "Space" => {
                let Some(id) = self.st.focus else {
                    return false;
                };
                if self.st.hit(id).and_then(|h| h.action).is_none() {
                    return false;
                }
                self.st.focus_visible = true;
                self.activate_from_keys(id);
                true
            }
            "ArrowLeft" | "ArrowRight" | "ArrowUp" | "ArrowDown" => {
                let Some(id) = self.st.focus else {
                    return false;
                };
                match self.st.hit(id).and_then(|h| h.action) {
                    Some(Action::Select { group, .. }) => {
                        self.move_in_group(group, code);
                        self.st.focus_visible = true;
                        true
                    }
                    Some(Action::ToggleMenu(_)) if code == "ArrowDown" => {
                        self.activate_from_keys(id);
                        true
                    }
                    _ => false,
                }
            }
            _ => false,
        }
    }

    /// A screen reader asked for something.
    pub fn access(&mut self, req: &ActionRequest) {
        use accesskit::Action as A;
        let id = Id(req.target_node.0);
        match req.action {
            A::Click => {
                if self.st.hit(id).is_some_and(|h| h.action.is_some()) {
                    self.activate(id);
                }
            }
            A::Focus => {
                self.set_focus(Some(id));
                self.st.focus_visible = true;
            }
            A::Blur => {
                if self.st.focus == Some(id) {
                    self.set_focus(None);
                }
            }
            A::Expand | A::Collapse => {
                if let Some(Action::ToggleMenu(m)) = self.st.hit(id).and_then(|h| h.action) {
                    if self.menu_open(m) != (req.action == A::Expand) {
                        self.activate(id);
                    }
                }
            }
            A::SetValue => {
                if let Some(ActionData::Value(v)) = &req.data {
                    if self.st.edits.contains_key(&id) {
                        self.set_edit_text(id, v);
                        self.st.events.push(Event::Changed(id));
                    }
                }
            }
            A::SetTextSelection => {
                if let Some(ActionData::SetTextSelection(sel)) = &req.data {
                    self.select_from_access(sel);
                }
            }
            A::ScrollDown | A::ScrollUp => {
                if let Some(s) = self.st.scroll.get_mut(&id) {
                    let step = if req.action == A::ScrollDown {
                        40.0
                    } else {
                        -40.0
                    };
                    s.offset = (s.offset + step).clamp(0.0, s.max);
                }
            }
            _ => {}
        }
    }

    fn activate_from_keys(&mut self, id: Id) {
        let opens = matches!(self.st.hit(id).and_then(|h| h.action), Some(Action::ToggleMenu(m)) if !self.menu_open(m));
        self.activate(id);
        if opens {
            if let Some(m) = &mut self.st.menu {
                m.focus_in = true;
            }
        }
    }

    /// Does what activating `id` means.
    pub(crate) fn activate(&mut self, id: Id) {
        let Some(action) = self.st.hit(id).and_then(|h| h.action) else {
            return;
        };
        match action {
            Action::Click => self.st.events.push(Event::Click(id)),
            Action::Select { group, index } => {
                self.st.events.push(Event::Select { group, index });
                if self.menu_open(group) {
                    self.close_menu();
                }
            }
            Action::ToggleMenu(m) => {
                if self.menu_open(m) {
                    self.close_menu();
                } else {
                    self.open_menu(m, id);
                }
            }
        }
    }

    fn tab(&mut self, back: bool) {
        let order = &self.st.focus_order;
        if order.is_empty() {
            return;
        }
        let at = self
            .st
            .focus
            .and_then(|f| order.iter().position(|o| *o == f));
        let n = order.len();
        let next = match (at, back) {
            (None, false) => 0,
            (None, true) => n - 1,
            (Some(i), false) => (i + 1) % n,
            (Some(i), true) => (i + n - 1) % n,
        };
        self.st.focus_visible = true;
        self.set_focus(Some(order[next]));
    }

    fn move_in_group(&mut self, group: Id, code: &str) {
        let Some(items) = self.st.groups.get(&group) else {
            return;
        };
        if items.is_empty() {
            return;
        }
        let n = items.len();
        let at = self
            .st
            .focus
            .and_then(|f| items.iter().position(|i| i.0 == f));
        let next = match code {
            "Home" => 0,
            "End" => n - 1,
            "ArrowUp" | "ArrowLeft" => at.map_or(n - 1, |i| (i + n - 1) % n),
            _ => at.map_or(0, |i| (i + 1) % n),
        };
        let id = items[next].0;
        self.set_focus(Some(id));
    }

    /// Runs `input` through the focused field; whether it used it.
    fn edit_key(&mut self, id: Id, input: &Input) -> bool {
        let Some(mut e) = self.st.edits.remove(&id) else {
            return false;
        };
        let out = e.key(input, mac(), self.clipboard.as_mut());
        let used = match out {
            Outcome::Unhandled => false,
            Outcome::Moved => true,
            Outcome::Changed => {
                self.st.events.push(Event::Changed(id));
                true
            }
            Outcome::Submit => {
                self.st.events.push(Event::Submit(id));
                true
            }
            Outcome::Vertical { dir, extend } => {
                if let Some(g) = self.st.geoms.get(&id) {
                    vertical(&mut e, &g.shaped, dir, extend);
                }
                true
            }
        };
        if used {
            self.st.blink_from = self.st.now;
        }
        self.st.edits.insert(id, e);
        used
    }

    fn edit_index_at(&self, id: Id, (x, y): (f32, f32)) -> Option<usize> {
        let g = self.st.geoms.get(&id)?;
        let k = self.scale();
        let i = g.shaped.index_at(x * k - g.origin.0, y * k - g.origin.1);
        Some(i.min(self.edit_text(id).len()))
    }

    fn select_from_access(&mut self, sel: &accesskit::TextSelection) {
        let byte = |ui: &Ui, pos: &accesskit::TextPosition| -> Option<(Id, usize)> {
            ui.st.geoms.iter().find_map(|(id, g)| {
                let line = (0..g.shaped.lines.len())
                    .find(|l| crate::a11y::run_id(*id, *l).0 == pos.node.0)?;
                let start = crate::a11y::run_start(&g.shaped, line);
                let text = ui.edit_text(*id);
                let end = crate::a11y::run_end(&g.shaped, line, text.len());
                let i = crate::a11y::char_to_byte(&text[start..end], pos.character_index);
                Some((*id, start + i))
            })
        };
        let (Some((a_id, a)), Some((f_id, f))) = (byte(self, &sel.anchor), byte(self, &sel.focus))
        else {
            return;
        };
        if a_id == f_id {
            self.edit_mut(a_id).set_selection(a, f);
            self.set_focus(Some(a_id));
        }
    }
}

/// Moves the caret a visual line up or down, keeping its column.
fn vertical(e: &mut TextEdit, sh: &Shaped, dir: i32, extend: bool) {
    let (x, line) = sh.caret(e.caret());
    let goal = *e.goal_x.get_or_insert(x);
    let target = line as i64 + i64::from(dir);
    let to = if target < 0 {
        0
    } else if target as usize >= sh.lines.len() {
        e.text().len()
    } else {
        let l = sh.lines[target as usize];
        sh.index_at(goal, l.top + l.height / 2.0)
    };
    e.move_to(to, extend);
    e.goal_x = Some(goal);
}

fn clamp_ratio(r: f32) -> f32 {
    if r.is_finite() {
        r.clamp(MIN_SPLIT, 1.0 - MIN_SPLIT)
    } else {
        0.5
    }
}

/// A short ease-out, standing in for the menu's spring (0.2 s, bounce 0.1).
fn ease_out(t: f32) -> f32 {
    1.0 - (1.0 - t).powi(3)
}

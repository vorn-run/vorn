//! The phone app's state and input: what is open, the terminal's scroll, and touches as taps and drags.

use std::collections::HashMap;

use vorn_app::paint::{self, Caret};
use vorn_app::scroll::{self, Route, Rows, Scroll};
use vorn_app::view::PaneView;
use vorn_components::agent::AgentMarks;
use vornui::input::mods;
use vornui::{El, Event, Id, Input, Laid, Rect, Ui, WheelPhase, MOVE_SLOP};
use web_time::Instant;

use crate::model::{Model, Target};
use crate::screens::{self, id, key_id, row_id, settings_id, tab_id, KEYS, TERMINAL};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Tab {
    Sessions,
    Tasks,
    Workflows,
}

/// The safe area, logical pixels.
#[derive(Debug, Clone, Copy, Default)]
pub struct Insets {
    pub top: f32,
    pub bottom: f32,
    pub left: f32,
    pub right: f32,
}

/// A finger on the glass: a tap until it moves past the slop, then a drag.
struct Touch {
    start: (f32, f32),
    last: (f32, f32),
    at: Instant,
    dragging: bool,
    /// Points per second, smoothed over the last moves.
    vel: f32,
}

/// A drag's momentum after the finger lifts, decaying as the system's does.
struct Fling {
    at: (f32, f32),
    vel: f32,
    last: Instant,
}

/// The fraction of speed a fling keeps each millisecond (the system's normal rate).
const DECELERATION: f32 = 0.998;

pub struct Phone {
    pub model: Model,
    pub tab: Tab,
    pub session: Option<String>,
    /// Extension settings over the screen: `Some(None)` lists the pages.
    pub settings: Option<Option<String>>,
    /// Logical size and the safe area around it.
    pub size: (f32, f32),
    pub insets: Insets,
    pub keyboard: f32,
    pub marks: AgentMarks,
    /// Only the open session's terminal: the native host draws the rest.
    pub terminal_only: bool,
    pub ctrl: bool,
    preedit: String,
    scrolls: HashMap<String, Scroll>,
    /// Where the current terminal gesture goes, fixed when it begins.
    wheel: Option<(Route, Rows)>,
    term: Option<Rect>,
    touch: Option<Touch>,
    fling: Option<Fling>,
    /// Whether the keyboard should be up: a field or the terminal was tapped.
    pub typing: bool,
    pub dirty: bool,
}

impl Phone {
    pub fn new(model: Model) -> Phone {
        Phone {
            model,
            tab: Tab::Sessions,
            session: None,
            settings: None,
            size: (390.0, 844.0),
            insets: Insets::default(),
            keyboard: 0.0,
            marks: AgentMarks::default(),
            terminal_only: false,
            ctrl: false,
            preedit: String::new(),
            scrolls: HashMap::new(),
            wheel: None,
            term: None,
            touch: None,
            fling: None,
            typing: false,
            dirty: true,
        }
    }

    pub fn connect(&mut self, t: Target) {
        self.model.connect(t);
        self.dirty = true;
    }

    /// Opens `session`'s terminal, or goes back to the list.
    pub fn open(&mut self, session: Option<String>) {
        if let (Some(old), Some(g)) = (&self.session, &self.model.grid) {
            if session.as_ref() != Some(old) {
                g.detach(old);
            }
        }
        self.session = session;
        self.typing = self.session.is_some() && self.terminal_only;
        self.dirty = true;
    }

    /// Whether a frame is due: something changed, or something moves.
    pub fn due(&mut self, ui: &Ui) -> bool {
        if self.model.pump() {
            self.dirty = true;
        }
        let now = Instant::now();
        self.dirty
            || self.fling.is_some()
            || ui.next_wake().is_some_and(|w| w <= now)
            || self.scrolls.values().any(|s| !s.resting())
    }

    // Touches arrive in logical pixels.

    pub fn touch_down(&mut self, ui: &mut Ui, x: f32, y: f32) {
        self.fling = None;
        self.touch = Some(Touch {
            start: (x, y),
            last: (x, y),
            at: Instant::now(),
            dragging: false,
            vel: 0.0,
        });
        ui.pointer_move(x, y);
        self.dirty = true;
    }

    pub fn touch_move(&mut self, ui: &mut Ui, x: f32, y: f32) {
        let Some(t) = &mut self.touch else { return };
        let now = Instant::now();
        if !t.dragging {
            let (dx, dy) = (x - t.start.0, y - t.start.1);
            if dx.hypot(dy) < MOVE_SLOP * 2.0 {
                return;
            }
            t.dragging = true;
            ui.wheel_phase(0.0, WheelPhase::Began);
        }
        let dy = y - t.last.1;
        let dt = now.saturating_duration_since(t.at).as_secs_f32().max(1e-3);
        t.vel = 0.6 * (dy / dt) + 0.4 * t.vel;
        t.last = (x, y);
        t.at = now;
        // Content follows the finger: a finger moving up shows what is below.
        ui.wheel_phase(-dy, WheelPhase::Moved);
        self.after_input(ui);
    }

    pub fn touch_up(&mut self, ui: &mut Ui, x: f32, y: f32) {
        let Some(t) = self.touch.take() else { return };
        if t.dragging {
            ui.wheel_phase(0.0, WheelPhase::Ended);
            let idle = t.at.elapsed().as_secs_f32();
            if t.vel.abs() > 60.0 && idle < 0.05 {
                ui.wheel_phase(0.0, WheelPhase::Began);
                self.fling = Some(Fling {
                    at: t.last,
                    vel: t.vel,
                    last: Instant::now(),
                });
            }
        } else {
            ui.pointer_move(x, y);
            ui.pointer_down();
            ui.pointer_up();
        }
        self.after_input(ui);
    }

    pub fn touch_cancel(&mut self, ui: &mut Ui) {
        if self.touch.take().is_some_and(|t| t.dragging) {
            ui.wheel_phase(0.0, WheelPhase::Ended);
        }
        self.dirty = true;
    }

    fn step_fling(&mut self, ui: &mut Ui) {
        let Some(f) = &mut self.fling else { return };
        let now = Instant::now();
        let ms = now.saturating_duration_since(f.last).as_secs_f32() * 1000.0;
        f.last = now;
        let dy = f.vel * ms / 1000.0;
        f.vel *= DECELERATION.powf(ms);
        let (at, done) = (f.at, f.vel.abs() < 20.0);
        ui.pointer_move(at.0, at.1);
        ui.wheel_phase(-dy, WheelPhase::Moved);
        if done {
            ui.wheel_phase(0.0, WheelPhase::Ended);
            self.fling = None;
        }
        self.after_input(ui);
    }

    /// Text from the system keyboard: dictation, autocorrect and IME commits included.
    pub fn text(&mut self, ui: &mut Ui, s: &str) {
        self.preedit.clear();
        if ui.focus().is_some() {
            for ch in s.chars() {
                let input = if ch == '\n' {
                    key("Enter", 0)
                } else {
                    Input::Commit(ch.to_string())
                };
                ui.key(&input);
            }
        } else if let Some(sid) = self.session.clone() {
            self.back_to_live(&sid);
            if self.ctrl && s.chars().count() == 1 {
                self.ctrl = false;
                let ch = s.chars().next().unwrap_or(' ').to_ascii_uppercase();
                self.model.key(&sid, &format!("Key{ch}"), mods::CTRL, None);
            } else if s == "\n" {
                self.model.key(&sid, "Enter", 0, None);
            } else {
                self.model.type_text(&sid, s);
            }
        }
        self.after_input(ui);
    }

    pub fn preedit(&mut self, ui: &mut Ui, s: &str) {
        self.preedit = s.to_owned();
        if ui.focus().is_some() {
            ui.key(&Input::Preedit(s.to_owned()));
        }
        self.dirty = true;
    }

    /// A key the system keyboard has no text for (Backspace), as a DOM code.
    pub fn key(&mut self, ui: &mut Ui, code: &str, m: u16) {
        if ui.focus().is_some() {
            ui.key(&key(code, m));
        } else if let Some(sid) = self.session.clone() {
            self.back_to_live(&sid);
            let m = if std::mem::take(&mut self.ctrl) {
                m | mods::CTRL
            } else {
                m
            };
            self.model.key(&sid, code, m, None);
        }
        self.after_input(ui);
    }

    /// The composer's text, sent as typed and then Return.
    pub fn send(&mut self, text: &str) {
        if let Some(sid) = self.session.clone() {
            self.back_to_live(&sid);
            if !text.is_empty() {
                self.model.type_text(&sid, text);
            }
            self.model.key(&sid, "Enter", 0, None);
        }
    }

    fn back_to_live(&mut self, sid: &str) {
        if let Some(s) = self.scrolls.get_mut(sid) {
            s.reset();
        }
        if let Some(g) = &self.model.grid {
            if g.scroll_to(sid, 0) {
                if let Some(v) = g.view(sid) {
                    self.model.views.insert(sid.to_owned(), v);
                }
            }
        }
    }

    /// Hands what the last input did to the app.
    pub fn after_input(&mut self, ui: &mut Ui) {
        for e in ui.take_events() {
            self.event(ui, e);
        }
        self.dirty = true;
    }

    fn event(&mut self, ui: &mut Ui, e: Event) {
        match e {
            Event::Click(id) => self.click(ui, id),
            Event::Submit(i) if i == id(screens::COMPOSER) => self.submit_composer(ui),
            Event::Submit(i) => self.model.submit(ui, i),
            Event::Wheel {
                id,
                dy,
                x,
                y,
                phase,
            } if id == Id(TERMINAL) => {
                if let Some(sid) = self.session.clone() {
                    self.wheel_terminal(ui, &sid, dy, (x, y), phase);
                }
            }
            _ => {}
        }
    }

    fn submit_composer(&mut self, ui: &mut Ui) {
        let c = id(screens::COMPOSER);
        let text = ui.edit_text(c).to_owned();
        self.send(&text);
        ui.set_edit_text(c, "");
    }

    fn click(&mut self, ui: &mut Ui, i: Id) {
        if i == Id(TERMINAL) {
            ui.set_focus(None);
            self.typing = true;
            return;
        }
        if i == id(screens::COMPOSER) {
            self.typing = true;
            return;
        }
        if i == id(screens::SEND) {
            self.submit_composer(ui);
            return;
        }
        for t in [Tab::Sessions, Tab::Tasks, Tab::Workflows] {
            if i == tab_id(t) {
                self.tab = t;
                return;
            }
        }
        if i == id("back") {
            ui.set_focus(None);
            self.typing = false;
            self.open(None);
            return;
        }
        if i == id("hide-keyboard") {
            ui.set_focus(None);
            self.typing = false;
            return;
        }
        if i == id("open-settings") {
            self.settings = Some(None);
            return;
        }
        if i == id("sheet-done") || i == id("sheet-scrim") {
            self.settings = None;
            self.typing = false;
            ui.set_focus(None);
            return;
        }
        if i == id("sheet-back") {
            self.settings = Some(None);
            return;
        }
        for (k, (label, _, code, ctrl)) in KEYS.iter().enumerate() {
            if i == key_id(k) {
                let Some(sid) = self.session.clone() else {
                    return;
                };
                if *label == "ctrl" {
                    self.ctrl = !self.ctrl;
                } else {
                    let m = if *ctrl || std::mem::take(&mut self.ctrl) {
                        mods::CTRL
                    } else {
                        0
                    };
                    self.back_to_live(&sid);
                    self.model.key(&sid, code, m, None);
                }
                return;
            }
        }
        let hit = self
            .model
            .store
            .sessions
            .iter()
            .find(|s| row_id(&s.info.id) == i)
            .map(|s| s.info.id.clone());
        if let Some(sid) = hit {
            self.open(Some(sid));
            return;
        }
        let page = self
            .model
            .settings()
            .find(|p| settings_id(&p.key()) == i)
            .map(|p| p.key());
        if let Some(key) = page {
            self.settings = Some(Some(key));
            return;
        }
        self.model.click(i);
        self.typing = ui.focus().is_some();
    }

    /// As on the desktop: wheel reports when the program tracks the mouse, arrows on the alternate screen, else history.
    fn wheel_terminal(
        &mut self,
        ui: &mut Ui,
        sid: &str,
        dy: f32,
        (x, y): (f32, f32),
        phase: WheelPhase,
    ) {
        let Some(v) = self.model.views.get(sid) else {
            return;
        };
        if phase == WheelPhase::Began || self.wheel.is_none() {
            self.wheel = Some((scroll::route(v.mouse, !v.primary), Rows::default()));
        }
        let Some((route, rows)) = &mut self.wheel else {
            return;
        };
        if *route == Route::History {
            self.scroll_terminal(ui, sid, dy, phase);
            return;
        }
        let (Some(r), Some(g)) = (self.term, &self.model.grid) else {
            return;
        };
        let k = ui.scale();
        let cell = ui.text.cell();
        let n = rows.take(dy * k, phase, cell.h, *route);
        let col = ((x - r.x) * k / cell.w).floor().max(0.0);
        let row = ((y - r.y) * k / cell.h).floor().max(0.0);
        g.wheel_rows(sid, n, *route, col, row);
    }

    fn scroll_terminal(&mut self, ui: &mut Ui, sid: &str, dy: f32, phase: WheelPhase) {
        let (Some(r), Some(v)) = (self.term, self.model.views.get(sid)) else {
            return;
        };
        let k = ui.scale();
        let row = ui.text.cell().h;
        let s = self.scrolls.entry(sid.to_owned()).or_default();
        s.set_metrics(row, r.h * k, v.history);
        s.wheel(dy * k, phase, Instant::now());
    }

    /// Lines the terminal's view up with its scroll; the pixels to draw it down by.
    fn scroll_frame(&mut self, ui: &Ui, sid: &str, h: f32) -> f32 {
        let (Some(s), Some(v), Some(g)) = (
            self.scrolls.get_mut(sid),
            self.model.views.get(sid),
            &self.model.grid,
        ) else {
            return 0.0;
        };
        let row = ui.text.cell().h;
        s.set_metrics(row, h, v.history);
        if let Some(at) = s.tick(Instant::now()) {
            ui.wake_at(at);
        }
        let (back, _) = s.split();
        if back != v.back() && g.scroll_to(sid, back) {
            if let Some(nv) = g.view(sid) {
                self.model.views.insert(sid.to_owned(), nv);
            }
        }
        let back = self.model.views.get(sid).map_or(0, PaneView::back);
        let dy = (s.pos() - back as f32 * row).round();
        if s.resting() && !s.scrolled() {
            self.scrolls.remove(sid);
        }
        dy
    }

    /// Whether the system keyboard should be up.
    pub fn wants_keyboard(&self, ui: &Ui) -> bool {
        ui.focus().is_some() || (self.typing && self.session.is_some())
    }
}

fn key(code: &str, m: u16) -> Input {
    Input::Key {
        code: code.to_owned(),
        mods: m,
        text: None,
    }
}

impl vornui::App for Phone {
    fn view(&mut self, ui: &mut Ui) -> El {
        self.step_fling(ui);
        self.marks.load(ui);
        self.model.acts.clear();
        self.dirty = false;
        screens::root(self, ui)
    }

    fn paint(&mut self, ui: &mut Ui, laid: &Laid) {
        self.term = None;
        let Some(sid) = self.session.clone() else {
            return;
        };
        let Some(&(_, r)) = laid.customs.iter().find(|(id, _)| *id == TERMINAL) else {
            return;
        };
        let k = ui.scale();
        self.term = Some(Rect {
            x: r.x / k,
            y: r.y / k,
            w: r.w / k,
            h: r.h / k,
        });
        let cell = ui.text.cell();
        if let Some(g) = &self.model.grid {
            let cols = (r.w / cell.w).floor() as u16;
            let rows = (r.h / cell.h).floor() as u16;
            let shell = self.model.store.session(&sid).is_some_and(|s| s.is_shell());
            g.attach(
                &sid,
                cols,
                rows,
                if shell {
                    vorn_app::grid::SHELL_HISTORY
                } else {
                    0
                },
            );
            g.resize(&sid, cols.max(2), rows.max(1));
        }
        let dy = self.scroll_frame(ui, &sid, r.h);
        let live = ui.focus().is_none() && self.typing;
        let caret = if live { Caret::Live } else { Caret::Hollow };
        let pre = if live { self.preedit.as_str() } else { "" };
        if let Some(v) = self.model.views.get(&sid) {
            ui.scene.push_clip(r);
            paint::pane(ui, v, r, caret, pre, &[], dy);
            ui.scene.pop_clip();
        }
    }

    fn background(&self) -> vornui::Rgba {
        vornui::theme::color::SURFACE_BASE
    }
}

//! Model B: Worktree cleanup's UI as vornui code in a component; each client runs it and paints its frames.

use std::collections::BTreeSet;
use std::time::Instant;

use serde_json::Value;
use vornui::cells::{Buffer, PaintList};
use vornui::theme::{color, radius, text as size};
use vornui::widgets::{self, Tone};
use vornui::{
    div, text, Action, El, Event, Id, Input as Key, Offscreen, Rgba, Sense, Ui, UiConfig,
};

wit_bindgen::generate!({ world: "ui", path: "../wit" });

use vorn::ext_ui_poc::surface::{Cell, Input as In};

struct Row {
    id: String,
    name: String,
    branch: String,
    status: String,
    size: String,
    removable: bool,
}

#[derive(Default)]
struct Screen {
    rows: Vec<Row>,
    selected: BTreeSet<String>,
    anchor: Option<usize>,
    confirming: bool,
    status: String,
}

const FILTER: Id = Id(0x46_49_4c);
const LIST: Id = Id(0x4c_49_53);
const REMOVE: Id = Id(0x52_45_4d);
const OK: Id = Id(0x4f_4b);
const CANCEL: Id = Id(0x43_41_4e);

fn row_id(i: usize) -> Id {
    LIST.child(i)
}

impl Screen {
    fn visible<'a>(&'a self, ui: &Ui) -> Vec<(usize, &'a Row)> {
        let q = ui.edit_text(FILTER).to_lowercase();
        self.rows
            .iter()
            .enumerate()
            .filter(|(_, r)| {
                q.is_empty()
                    || r.name.to_lowercase().contains(&q)
                    || r.branch.to_lowercase().contains(&q)
            })
            .collect()
    }

    fn view(&self, ui: &Ui) -> El {
        let removable = self.rows.iter().filter(|r| r.removable).count();
        let rows = self
            .visible(ui)
            .into_iter()
            .map(|(i, r)| self.row(ui, i, r));
        let n = self.selected.len();
        let label = format!("Remove {n}");
        let mut remove = widgets::button(ui, REMOVE, &label);
        if n == 0 {
            remove = remove.disabled(true).opacity(0.4);
        }
        if self.confirming {
            let msg = format!("Remove {n} worktrees? Their branches stay.");
            remove = widgets::confirm(ui, remove, &msg, (OK, "Remove"), CANCEL, Tone::Danger);
        }
        div()
            .col()
            .gap(12.0)
            .p(16.0)
            .w_full()
            .h_full()
            .bg(color::SURFACE_BASE)
            .child(
                div()
                    .row()
                    .justify_between()
                    .items_center()
                    .child(
                        div()
                            .col()
                            .gap(2.0)
                            .child(text("Worktree cleanup", size::SM.0, color::INK).weight(600))
                            .child(text(
                                format!("{} worktrees · {removable} can go", self.rows.len()),
                                size::XS.0,
                                color::INK_SECONDARY,
                            )),
                    )
                    .child(
                        div()
                            .row()
                            .items_center()
                            .gap(8.0)
                            .child(widgets::text_input(ui, FILTER, "Filter", "Filter").w(200.0))
                            .child(remove),
                    ),
            )
            .child(widgets::list(LIST, "Worktrees", rows).grow().min_h(0.0))
            .child(text(self.status.clone(), size::XS.0, color::INK_SECONDARY))
    }

    fn row(&self, ui: &Ui, i: usize, r: &Row) -> El {
        let on = self.selected.contains(&r.id);
        let id = row_id(i);
        let (fg, bg) = match r.status.as_str() {
            "merged" => (
                color::GREEN_400,
                Rgba::hexa(vornui::theme::rgb::GREEN_500, 0.15),
            ),
            "stale" => (
                color::YELLOW_500,
                Rgba::hexa(vornui::theme::rgb::YELLOW_500, 0.15),
            ),
            "dirty" => (
                color::RED_400,
                Rgba::hexa(vornui::theme::rgb::RED_500, 0.15),
            ),
            "active" => (color::STATUS_BLUE, Rgba::white(0.08)),
            _ => (color::GRAY_400, Rgba::white(0.06)),
        };
        let check = div()
            .size(14.0, 14.0)
            .rounded(radius::SM)
            .border(1.0, Rgba::white(if on { 0.6 } else { 0.25 }))
            .bg(if on { color::BRONZO } else { Rgba::TRANSPARENT })
            .glyph(if on { "☑" } else { "☐" });
        let wash = if on {
            Rgba::white(0.06)
        } else {
            Rgba::TRANSPARENT.mix(Rgba::white(0.04), ui.hover(id))
        };
        div()
            .id(id)
            .row()
            .items_center()
            .gap(12.0)
            .px(8.0)
            .py(6.0)
            .rounded(radius::MD)
            .bg(wash)
            .on(Action::Click)
            .sense(Sense::MOVE)
            .opacity(if r.removable { 1.0 } else { 0.55 })
            .child(check)
            .child(
                text(r.name.clone(), size::XS.0, color::INK)
                    .w(160.0)
                    .truncate(),
            )
            .child(
                text(r.branch.clone(), size::XS.0, color::GRAY_300)
                    .mono()
                    .grow()
                    .truncate(),
            )
            .child(widgets::badge(&r.status, fg, bg))
            .child(text(r.size.clone(), size::XS.0, color::INK_SECONDARY).w(64.0))
    }

    fn index_at(&self, ui: &Ui, y: f32) -> Option<usize> {
        self.visible(ui).into_iter().map(|(i, _)| i).find(|&i| {
            ui.rect_of(row_id(i))
                .is_some_and(|r| y >= r.y && y < r.y + r.h)
        })
    }

    fn toggle(&mut self, i: usize) {
        let Some(r) = self.rows.get(i) else { return };
        if !r.removable {
            return;
        }
        if !self.selected.remove(&r.id) {
            self.selected.insert(r.id.clone());
        }
        self.anchor = Some(i);
    }

    fn event(&mut self, ui: &Ui, e: Event) {
        match e {
            Event::Click(id) if id == REMOVE && !self.selected.is_empty() => self.confirming = true,
            Event::Click(id) if id == CANCEL => self.confirming = false,
            Event::Click(id) if id == OK => {
                self.confirming = false;
                let ids: Vec<String> = self.selected.iter().cloned().collect();
                let args = serde_json::to_string(&ids).unwrap_or_default();
                self.status = match call("remove", &args) {
                    Ok(done) => done,
                    Err(e) => e,
                };
                self.selected.clear();
            }
            Event::Click(id) => {
                if let Some(i) = (0..self.rows.len()).find(|&i| row_id(i) == id) {
                    self.toggle(i);
                }
            }
            Event::Move { y, .. } => {
                let (Some(a), Some(b)) = (self.anchor, self.index_at(ui, y)) else {
                    return;
                };
                let (lo, hi) = (a.min(b), a.max(b));
                for r in &self.rows[lo..=hi] {
                    if r.removable {
                        self.selected.insert(r.id.clone());
                    }
                }
            }
            _ => {}
        }
    }
}

struct State {
    ui: Ui,
    screen: Screen,
    target: Option<Offscreen>,
    t0: Instant,
}

thread_local! {
    static STATE: std::cell::RefCell<Option<State>> = const { std::cell::RefCell::new(None) };
}

struct Component;

impl Guest for Component {
    fn init(cells: bool, scale: f32) {
        let cfg = UiConfig {
            scale,
            ui_font: "System Font",
            mono_font: "Menlo",
            term_size: 13.0,
        };
        let ui = if cells {
            Ui::cells(&cfg)
        } else {
            Ui::headless(vornui::RenderMode::Cpu, &cfg)
        };
        STATE.with(|s| {
            *s.borrow_mut() = Some(State {
                ui,
                screen: Screen::default(),
                target: None,
                t0: Instant::now(),
            })
        });
    }

    fn data(rows: String) {
        let v: Value = serde_json::from_str(&rows).unwrap_or(Value::Null);
        let s = |r: &Value, k: &str| r[k].as_str().unwrap_or("").to_owned();
        let rows = v
            .as_array()
            .into_iter()
            .flatten()
            .map(|r| Row {
                id: s(r, "id"),
                name: s(r, "name"),
                branch: s(r, "branch"),
                status: s(r, "status"),
                size: s(r, "size"),
                removable: r["removable"].as_bool().unwrap_or(false),
            })
            .collect();
        STATE.with(|st| {
            if let Some(st) = st.borrow_mut().as_mut() {
                let live: BTreeSet<String> = st.screen.rows.iter().map(|r| r.id.clone()).collect();
                st.screen.rows = rows;
                st.screen.selected.retain(|id| live.contains(id));
            }
        });
    }

    fn frame(w: u32, h: u32, inputs: Vec<In>) -> Frame {
        STATE.with(|st| {
            let mut st = st.borrow_mut();
            let st = st.as_mut().expect("init first");
            for i in inputs {
                match i {
                    In::PointerMove((x, y)) => st.ui.pointer_move(x, y),
                    In::PointerDown => st.ui.pointer_down(),
                    In::PointerUp => st.ui.pointer_up(),
                    In::Wheel(dy) => st.ui.wheel(dy),
                    In::Key(k) => {
                        st.ui.key(&Key::Key {
                            code: k.code,
                            mods: k.mods,
                            text: k.text,
                        });
                    }
                }
                for e in st.ui.take_events() {
                    st.screen.event(&st.ui, e);
                }
            }
            let now = st.t0 + std::time::Duration::from_secs(3600);
            let size = if st.ui.is_cells() {
                vornui::cells::units::px(w as u16, h as u16)
            } else {
                (w as f32 / st.ui.scale(), h as f32 / st.ui.scale())
            };
            st.ui.begin_at(color::SURFACE_BASE, now);
            let root = st.screen.view(&st.ui);
            st.ui.layout(root, size);
            if st.ui.is_cells() {
                let mut buf = Buffer::new(w as u16, h as u16);
                buf.draw(st.ui.paint_list().unwrap_or(&PaintList::default()));
                let rgb =
                    |c: [u8; 3]| u32::from(c[0]) << 16 | u32::from(c[1]) << 8 | u32::from(c[2]);
                return Frame::Cells(
                    buf.cells
                        .iter()
                        .map(|c| Cell {
                            sym: c.sym.as_str().to_owned(),
                            fg: rgb(c.fg),
                            bg: rgb(c.bg),
                            attr: c.attr,
                        })
                        .collect(),
                );
            }
            if st.target.as_ref().is_none_or(|t| t.size != (w, h)) {
                st.target = Some(st.ui.offscreen((w, h)));
            }
            let target = st.target.as_ref().unwrap();
            st.ui.render_offscreen(target);
            Frame::Pixels(st.ui.read(target).unwrap_or_default())
        })
    }
}

export!(Component);

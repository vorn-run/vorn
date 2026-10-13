//! Cascading menus driven the way a window drives them: the submenu sits
//! beside its entry, flips left at the window's edge, holds while the
//! pointer heads for it, toggles on click and follows the arrow keys.

use std::time::{Duration, Instant};

use vornui::menu::{self, Cascade, Item, HOLD, OVERLAP};
use vornui::theme::color;
use vornui::widgets::item_id;
use vornui::{div, Action, Event, Id, Input, Place, Rect, RenderMode, Ui, UiConfig};

/// Alpha and Gamma have submenus; Beta and Delta do not.
const SUBS: [bool; 4] = [true, false, true, false];

fn menu_id() -> Id {
    Id::new("menu")
}

fn sub_id() -> Id {
    Id::new("sub")
}

fn trigger() -> Id {
    Id::new("trigger")
}

fn items() -> Vec<Item<u32>> {
    let leaf = |l: &str, a| Item::option(l, false, a);
    vec![
        leaf("Alpha", 0).submenu(vec![leaf("A1", 10), leaf("A2", 11), leaf("A3", 12)]),
        leaf("Beta", 1),
        leaf("Gamma", 2).submenu(vec![leaf("G1", 20), leaf("G2", 21)]),
        leaf("Delta", 3),
    ]
}

struct Rig {
    ui: Ui,
    now: Instant,
    c: Cascade,
    at: (f32, f32),
    size: (f32, f32),
    chosen: Vec<u32>,
}

impl Rig {
    /// The menu open with its top-left at `at` in a window `size`.
    fn new(at: (f32, f32), size: (f32, f32)) -> Rig {
        let mut r = Rig {
            ui: Ui::headless(RenderMode::Cpu, &UiConfig::system(1.0, 13.0)),
            now: Instant::now(),
            c: Cascade::default(),
            at,
            size,
            chosen: Vec::new(),
        };
        r.ui.set_now(r.now);
        r.frame();
        r.ui.open_menu(menu_id(), trigger());
        r.frame();
        r
    }

    fn frame(&mut self) {
        menu::track(&mut self.ui, &mut self.c, menu_id(), sub_id(), &SUBS);
        self.ui.begin_at(color::SURFACE_BASE, self.now);
        let mut root = div().w(self.size.0).h(self.size.1).child(
            div()
                .id(trigger())
                .absolute(0.0, 0.0)
                .size(10.0, 10.0)
                .on(Action::ToggleMenu(menu_id())),
        );
        if self.ui.menu_open(menu_id()) {
            let panel = menu::panel(
                &self.ui,
                menu_id(),
                menu_id(),
                sub_id(),
                200.0,
                "Menu",
                items(),
                self.c,
            );
            root = root.child(div().absolute(self.at.0, self.at.1).size(0.0, 0.0).overlay(
                panel,
                Place::BelowStart,
                0.0,
            ));
        }
        self.ui.layout(root, self.size);
    }

    fn wait(&mut self, ms: u64) {
        self.now += Duration::from_millis(ms);
        self.ui.set_now(self.now);
    }

    /// What the app does with a choice: a submenu's entry toggles it.
    fn events(&mut self) {
        for e in self.ui.take_events() {
            if let Event::Select { group, index } = e {
                if group == menu_id() && SUBS[index] {
                    self.c.click(index, self.now);
                    self.ui.reopen_menu();
                } else if group == menu_id() {
                    self.chosen.push(index as u32);
                } else if group == sub_id() {
                    self.chosen.push(100 + index as u32);
                }
            }
        }
    }

    fn rect(&self, id: Id) -> Rect {
        self.ui
            .rect_of(id)
            .unwrap_or_else(|| panic!("{id:?} is laid out"))
    }

    fn row(&self, i: usize) -> Rect {
        self.rect(item_id(menu_id(), i))
    }

    fn sub(&self) -> Rect {
        self.rect(menu::sub_panel_id(sub_id()))
    }

    /// The pointer to `p`, a frame later.
    fn move_to(&mut self, p: (f32, f32)) {
        self.ui.pointer_move(p.0, p.1);
        self.wait(16);
        self.frame();
        self.frame();
    }

    /// Steps the pointer to `p` in `n` moves, a frame each.
    fn glide(&mut self, from: (f32, f32), to: (f32, f32), n: u32) {
        for k in 1..=n {
            let t = k as f32 / n as f32;
            self.move_to((from.0 + (to.0 - from.0) * t, from.1 + (to.1 - from.1) * t));
        }
    }

    fn click_row(&mut self, i: usize) {
        let r = self.row(i);
        self.ui.pointer_move(r.x + 20.0, r.y + r.h / 2.0);
        self.ui.pointer_down();
        self.ui.pointer_up();
        self.events();
        self.frame();
        self.frame();
    }

    fn key(&mut self, code: &str) -> bool {
        let used = menu::key(&mut self.ui, &mut self.c, menu_id(), sub_id(), &SUBS, code)
            || self.ui.key(&Input::named(code));
        self.events();
        self.frame();
        self.frame();
        used
    }

    fn focused(&self, id: Id) -> bool {
        self.ui.focused(id)
    }
}

fn mid(r: Rect) -> (f32, f32) {
    (r.x + r.w / 2.0, r.y + r.h / 2.0)
}

#[test]
fn a_submenu_opens_beside_its_entry_level_with_it() {
    let mut r = Rig::new((40.0, 40.0), (800.0, 600.0));
    let row = r.row(0);
    r.move_to((row.x + 20.0, row.y + row.h / 2.0));
    assert_eq!(r.c.open, Some(0), "hovering Alpha opens its submenu");
    let (row, sub) = (r.row(0), r.sub());
    assert!(
        sub.x < row.right() && sub.x >= row.right() - OVERLAP - 1.5,
        "to the right, a little over the edge: {sub:?} beside {row:?}"
    );
    let first = r.rect(item_id(sub_id(), 0));
    assert!(
        (first.y - row.y).abs() <= 1.0,
        "its first entry level with Alpha"
    );
    // The next entry stays reachable: the submenu covers none of it.
    let beta = r.row(1);
    assert!(sub.x >= beta.right() - OVERLAP - 1.5);
}

#[test]
fn a_submenu_flips_left_at_the_window_edge() {
    let mut r = Rig::new((800.0 - 210.0, 40.0), (800.0, 600.0));
    let row = r.row(0);
    r.move_to((row.x + 20.0, row.y + row.h / 2.0));
    let (row, sub) = (r.row(0), r.sub());
    assert!(
        sub.right() > row.x && sub.right() <= row.x + OVERLAP + 1.5,
        "to the left, a little over the edge: {sub:?} beside {row:?}"
    );
    assert!((r.rect(item_id(sub_id(), 0)).y - row.y).abs() <= 1.0);
}

#[test]
fn a_submenu_slides_up_to_fit_above_the_window_bottom() {
    let mut r = Rig::new((40.0, 600.0 - 140.0), (800.0, 600.0));
    let row = r.row(2);
    r.move_to((row.x + 20.0, row.y + row.h / 2.0));
    assert_eq!(r.c.open, Some(2));
    let sub = r.sub();
    assert!(sub.bottom() <= 600.0 - 3.5, "on screen: {sub:?}");
    assert!(
        sub.y < row.y && sub.bottom() >= row.bottom(),
        "still beside Gamma"
    );
    assert!(
        sub.x >= row.right() - OVERLAP - 1.5,
        "beside, not over, the menu"
    );
}

#[test]
fn heading_for_the_submenu_across_other_entries_keeps_it_open() {
    let mut r = Rig::new((40.0, 40.0), (800.0, 600.0));
    let a = r.row(0);
    let start = (a.x + 20.0, a.y + a.h / 2.0);
    r.move_to(start);
    assert_eq!(r.c.open, Some(0));
    // Diagonally down and right, over Beta, towards Alpha's submenu.
    let beta = r.row(1);
    let on_beta = (beta.right() - 24.0, beta.y + beta.h / 2.0);
    r.glide(start, on_beta, 4);
    assert!(
        r.ui.hovered(item_id(menu_id(), 1)),
        "the pointer is on Beta"
    );
    assert_eq!(r.c.open, Some(0), "Alpha's submenu holds");
    // On into the submenu: it stays.
    let a2 = mid(r.rect(item_id(sub_id(), 1)));
    r.glide(on_beta, a2, 2);
    assert_eq!(r.c.open, Some(0));
    assert!(r.ui.hovered(item_id(sub_id(), 1)));

    // Resting on Beta instead lets the held change land after `HOLD`.
    r.glide(a2, start, 2);
    r.glide(start, on_beta, 4);
    assert_eq!(r.c.open, Some(0));
    r.wait(HOLD.as_millis() as u64 + 20);
    r.frame();
    assert_eq!(r.c.open, None, "Beta has no submenu");
}

#[test]
fn moving_away_from_the_submenu_switches_at_once() {
    let mut r = Rig::new((40.0, 40.0), (800.0, 600.0));
    let a = r.row(0);
    r.move_to((a.x + 20.0, a.y + a.h / 2.0));
    assert_eq!(r.c.open, Some(0));
    // Straight down onto Gamma: not towards the submenu.
    let g = r.row(2);
    r.glide(
        (a.x + 20.0, a.y + a.h / 2.0),
        (g.x + 20.0, g.y + g.h / 2.0),
        3,
    );
    assert_eq!(r.c.open, Some(2), "Gamma's submenu replaces Alpha's");
    let d = r.row(3);
    r.move_to((d.x + 20.0, d.y + d.h / 2.0));
    assert_eq!(r.c.open, None);
}

#[test]
fn clicking_an_entry_with_a_submenu_toggles_it() {
    let mut r = Rig::new((40.0, 40.0), (800.0, 600.0));
    let a = r.row(0);
    r.move_to((a.x + 20.0, a.y + a.h / 2.0));
    assert_eq!(r.c.open, Some(0), "hovering opens it");
    r.click_row(0);
    assert_eq!(r.c.open, None, "the click closes it");
    assert!(r.ui.menu_open(menu_id()), "the menu stays");
    r.wait(400);
    r.frame();
    assert_eq!(r.c.open, None, "resting on it does not reopen it");
    r.click_row(0);
    assert_eq!(r.c.open, Some(0));
    assert!(r.chosen.is_empty(), "nothing chosen");
    // An entry in the submenu chooses.
    let a3 = mid(r.rect(item_id(sub_id(), 2)));
    r.move_to(a3);
    r.ui.pointer_down();
    r.ui.pointer_up();
    r.events();
    assert_eq!(r.chosen, vec![102]);
}

#[test]
fn keys_open_walk_and_close_submenus() {
    let mut r = Rig::new((40.0, 40.0), (800.0, 600.0));
    assert!(r.key("ArrowDown"));
    assert!(r.focused(item_id(menu_id(), 0)));
    assert!(r.key("ArrowRight"));
    assert_eq!(r.c.open, Some(0));
    assert!(
        r.focused(item_id(sub_id(), 0)),
        "focus goes into the submenu"
    );
    assert!(r.key("ArrowDown"));
    assert!(r.focused(item_id(sub_id(), 1)), "Down walks the submenu");
    assert!(r.key("ArrowLeft"));
    assert_eq!(r.c.open, None);
    assert!(r.focused(item_id(menu_id(), 0)), "Left returns to Alpha");
    assert!(r.key("ArrowDown"));
    assert!(r.focused(item_id(menu_id(), 1)));
    assert!(r.key("ArrowRight"), "Right on Beta does nothing");
    assert_eq!(r.c.open, None);
    assert!(r.focused(item_id(menu_id(), 1)));
    assert!(r.key("ArrowDown"));
    assert!(r.key("Enter"));
    assert_eq!(r.c.open, Some(2), "Enter opens Gamma's submenu");
    assert!(r.focused(item_id(sub_id(), 0)));
    assert!(r.key("Escape"));
    assert_eq!(r.c.open, None, "Escape closes one level");
    assert!(r.ui.menu_open(menu_id()));
    assert!(r.focused(item_id(menu_id(), 2)));
    assert!(r.key("Escape"));
    assert!(!r.ui.menu_open(menu_id()), "and then the menu");
    assert!(r.chosen.is_empty());
}

#[test]
fn entries_of_a_menu_and_its_submenu_never_share_an_id() {
    // Groups an app names by hand, one bit apart.
    let (menu, sub) = (Id(0x6d65_6e75_0000_0001), Id(0x6d65_6e75_0000_0003));
    for i in 0..64 {
        for j in 0..64 {
            assert_ne!(
                item_id(menu, i),
                item_id(sub, j),
                "entry {i} and submenu entry {j}"
            );
        }
    }
}

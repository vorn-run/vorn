//! Widgets driven the way a window drives them: frames at a fixed clock,
//! pointer and keys between frames, drawn by the CPU renderer offscreen.

use std::time::{Duration, Instant};

use vornui::accesskit::{Node, NodeId, Role, Toggled};
use vornui::input::mods;
use vornui::theme::color;
use vornui::widgets::{self, Choice, PickerLook, Pill, Tab};
use vornui::{div, El, Event, Id, Input, Laid, RenderMode, Rgba, Ui, UiConfig};

const SIZE: (f32, f32) = (480.0, 360.0);

struct Rig {
    ui: Ui,
    now: Instant,
    laid: Option<Laid>,
}

impl Rig {
    fn new() -> Rig {
        Rig {
            ui: Ui::headless(RenderMode::Cpu, &UiConfig::system(1.0, 13.0)),
            now: Instant::now(),
            laid: None,
        }
    }

    fn frame(&mut self, build: &dyn Fn(&Ui) -> El) -> &Laid {
        self.ui.begin_at(color::SURFACE_BASE, self.now);
        let root = div()
            .w(SIZE.0)
            .h(SIZE.1)
            .col()
            .p(16.0)
            .gap(8.0)
            .child(build(&self.ui));
        self.laid = Some(self.ui.layout(root, SIZE));
        self.laid.as_ref().expect("just laid out")
    }

    fn wait(&mut self, ms: u64) {
        self.now += Duration::from_millis(ms);
        self.ui.set_now(self.now);
    }

    fn node(&self, id: Id) -> Option<&Node> {
        self.laid
            .as_ref()?
            .tree
            .nodes
            .iter()
            .find(|(n, _)| n.0 == id.0)
            .map(|(_, n)| n)
    }

    fn nodes(&self, role: Role) -> Vec<&Node> {
        let laid = self.laid.as_ref().expect("a frame");
        laid.tree
            .nodes
            .iter()
            .filter(|(_, n)| n.role() == role)
            .map(|(_, n)| n)
            .collect()
    }

    fn center(&self, id: Id) -> (f32, f32) {
        let b = self
            .node(id)
            .and_then(Node::bounds)
            .unwrap_or_else(|| panic!("{id:?} not laid out"));
        (((b.x0 + b.x1) / 2.0) as f32, ((b.y0 + b.y1) / 2.0) as f32)
    }

    fn click(&mut self, id: Id) {
        let (x, y) = self.center(id);
        self.ui.pointer_move(x, y);
        self.ui.pointer_down();
        self.ui.pointer_up();
    }

    fn key(&mut self, code: &str) -> bool {
        self.ui.key(&Input::named(code))
    }

    fn typed(&mut self, s: &str) {
        for ch in s.chars() {
            assert!(self.ui.key(&Input::char(ch)));
        }
    }

    fn pixel(&mut self, (x, y): (f32, f32)) -> [u8; 4] {
        let size = (SIZE.0 as u32, SIZE.1 as u32);
        let target = self.ui.offscreen(size);
        assert!(self.ui.render_offscreen(&target));
        let px = self.ui.read(&target).expect("cpu frame");
        let i = (y as usize * size.0 as usize + x as usize) * 4;
        [px[i], px[i + 1], px[i + 2], px[i + 3]]
    }
}

/// `c` painted over `base`.
fn over(base: Rgba, c: Rgba) -> Rgba {
    let a = c.0[3];
    Rgba([0, 1, 2, 3].map(|i| {
        if i == 3 {
            1.0
        } else {
            base.0[i] * (1.0 - a) + c.0[i] * a
        }
    }))
}

fn near(px: [u8; 4], c: Rgba) {
    for i in 0..3 {
        let want = (c.0[i] * 255.0).round() as i32;
        assert!((i32::from(px[i]) - want).abs() <= 2, "{px:?} is not {c:?}");
    }
}

#[test]
fn pills_choose_on_click_and_arrows() {
    let g = Id::new("view");
    let items = [
        Pill {
            label: "Terminals",
            icon: None,
        },
        Pill {
            label: "Tasks",
            icon: None,
        },
    ];
    let build = |ui: &Ui| widgets::pills(ui, g, "View", &items, 0);
    let mut r = Rig::new();
    r.frame(&build);
    let radios = r.nodes(Role::RadioButton);
    assert_eq!(radios.len(), 2);
    assert_eq!(radios[0].toggled(), Some(Toggled::True));
    assert_eq!(r.nodes(Role::RadioGroup)[0].label(), Some("View"));
    r.click(widgets::item_id(g, 1));
    assert_eq!(
        r.ui.take_events(),
        vec![Event::Select { group: g, index: 1 }]
    );
    r.frame(&build);
    assert!(r.ui.focused(widgets::item_id(g, 1)));
    assert!(r.key("ArrowLeft"));
    assert!(r.ui.focused(widgets::item_id(g, 0)));
    assert!(r.key("Enter"));
    assert_eq!(
        r.ui.take_events(),
        vec![Event::Select { group: g, index: 0 }]
    );
}

#[test]
fn picker_menu_opens_chooses_on_press_and_dismisses() {
    let p = Id::new("model");
    let menu = widgets::menu_id(p);
    let choices = [
        Choice {
            label: "Opus",
            hint: None,
        },
        Choice {
            label: "Sonnet",
            hint: Some("fast"),
        },
        Choice {
            label: "Haiku",
            hint: None,
        },
    ];
    let build = |ui: &Ui| widgets::picker(ui, p, "Model", PickerLook::Chip, &choices, 1);
    let mut r = Rig::new();
    r.frame(&build);
    let combo = r.node(p).expect("trigger");
    assert_eq!(combo.role(), Role::ComboBox);
    assert_eq!(combo.value(), Some("Sonnet"));
    assert_eq!(combo.is_expanded(), Some(false));
    r.click(p);
    assert!(r.ui.menu_open(menu));
    r.frame(&build);
    assert_eq!(r.node(p).and_then(Node::is_expanded), Some(true));
    let options = r.nodes(Role::ListBoxOption);
    assert_eq!(options.len(), 3);
    assert!(options[1].is_selected() == Some(true));
    // Below the trigger, at least 180 wide.
    let (tb, mb) = (
        r.node(p).and_then(Node::bounds).unwrap(),
        r.node(menu).and_then(Node::bounds).unwrap(),
    );
    assert!(mb.y0 >= tb.y1 + 3.0 && mb.x1 - mb.x0 >= 180.0);

    // A press on an entry chooses it at once.
    let (x, y) = r.center(widgets::item_id(menu, 2));
    r.ui.pointer_move(x, y);
    r.ui.pointer_down();
    assert_eq!(
        r.ui.take_events(),
        vec![Event::Select {
            group: menu,
            index: 2
        }]
    );
    assert!(!r.ui.menu_open(menu));
    r.ui.pointer_up();

    // Escape and a press elsewhere close it without a choice.
    r.frame(&build);
    r.click(p);
    r.frame(&build);
    assert!(r.key("Escape"));
    assert_eq!(r.ui.take_events(), vec![Event::Dismiss(menu)]);
    r.click(p);
    r.frame(&build);
    r.ui.pointer_move(SIZE.0 - 5.0, SIZE.1 - 5.0);
    r.ui.pointer_down();
    assert!(!r.ui.menu_open(menu));
    assert_eq!(r.ui.take_events(), vec![Event::Dismiss(menu)]);
}

#[test]
fn picker_keyboard_opens_on_the_chosen_entry() {
    let p = Id::new("branch");
    let menu = widgets::menu_id(p);
    let choices = [
        Choice {
            label: "main",
            hint: None,
        },
        Choice {
            label: "dev",
            hint: None,
        },
    ];
    let build = |ui: &Ui| widgets::picker(ui, p, "Branch", PickerLook::Plain, &choices, 1);
    let mut r = Rig::new();
    r.frame(&build);
    assert!(r.key("Tab"));
    assert!(r.ui.focused(p));
    assert!(r.key("ArrowDown"));
    r.frame(&build);
    assert!(
        r.ui.focused(widgets::item_id(menu, 1)),
        "focus lands on the chosen entry"
    );
    assert_eq!(
        r.laid.as_ref().unwrap().tree.focus.0,
        widgets::item_id(menu, 1).0
    );
    assert!(r.key("ArrowDown"));
    assert!(r.ui.focused(widgets::item_id(menu, 0)), "wraps around");
    assert!(r.key("Enter"));
    assert_eq!(
        r.ui.take_events(),
        vec![Event::Select {
            group: menu,
            index: 0
        }]
    );
    assert!(r.ui.focused(p), "focus returns to the trigger");
}

#[test]
fn menu_grows_in_then_settles() {
    let p = Id::new("p");
    let menu = widgets::menu_id(p);
    let choices = [Choice {
        label: "a",
        hint: None,
    }];
    let build = |ui: &Ui| widgets::picker(ui, p, "P", PickerLook::Compact, &choices, 0);
    let mut r = Rig::new();
    r.frame(&build);
    r.click(p);
    assert_eq!(r.ui.menu_progress(menu), 0.0);
    assert!(r.ui.animating());
    r.wait(100);
    let mid = r.ui.menu_progress(menu);
    assert!(mid > 0.5 && mid < 1.0, "{mid}");
    r.wait(150);
    assert_eq!(r.ui.menu_progress(menu), 1.0);
    r.frame(&build);
    assert!(!r.ui.animating());
}

#[test]
fn tooltip_shows_after_the_delay_and_hides_on_press() {
    let b = Id::new("settings");
    let build = |ui: &Ui| {
        div().row().pt(60.0).pl(200.0).child(widgets::icon_button(
            ui,
            b,
            widgets::icons::X,
            14.0,
            "Settings",
        ))
    };
    let mut r = Rig::new();
    r.frame(&build);
    let (x, y) = r.center(b);
    r.ui.pointer_move(x, y);
    r.frame(&build);
    assert!(r.nodes(Role::Tooltip).is_empty());
    assert_eq!(r.ui.next_wake(), Some(r.now + vornui::TOOLTIP_DELAY));
    r.wait(399);
    assert!(!r.ui.tooltip_shown(b));
    r.wait(1);
    r.frame(&build);
    let tip = r.nodes(Role::Tooltip);
    assert_eq!(tip.len(), 1);
    assert_eq!(tip[0].label(), Some("Settings"));
    // Centred above the button.
    let (tb, bb) = (
        tip[0].bounds().unwrap(),
        r.node(b).and_then(Node::bounds).unwrap(),
    );
    assert!((tb.y1 + 6.0 - bb.y0).abs() < 0.5, "{tb:?} {bb:?}");
    assert!((((tb.x0 + tb.x1) - (bb.x0 + bb.x1)) / 2.0).abs() < 1.0);
    r.ui.pointer_down();
    assert!(!r.ui.tooltip_shown(b));
    r.ui.pointer_up();
    assert_eq!(r.ui.take_events(), vec![Event::Click(b)]);
}

#[test]
fn hover_transitions_over_150ms() {
    let b = Id::new("b");
    let build = |ui: &Ui| widgets::button(ui, b, "Run");
    let mut r = Rig::new();
    r.frame(&build);
    let (x, y) = r.center(b);
    r.ui.pointer_move(x, y);
    assert_eq!(r.ui.hover(b), 0.0);
    r.wait(75);
    let half = r.ui.hover(b);
    assert!(half > 0.3 && half < 0.9, "{half}");
    r.wait(75);
    assert_eq!(r.ui.hover(b), 1.0);
    r.frame(&build);
    near(
        r.pixel((x - 15.0, y)),
        over(color::SURFACE_BASE, Rgba::white(0.06)),
    );
}

#[test]
fn text_input_types_composes_and_reports() {
    let f = Id::new("name");
    let build = |ui: &Ui| widgets::text_input(ui, f, "Name", "Branch name").w(200.0);
    let mut r = Rig::new();
    r.frame(&build);
    assert!(!r.ui.wants_ime());
    r.click(f);
    assert!(r.ui.wants_ime());
    r.typed("héllo");
    assert_eq!(r.ui.edit_text(f), "héllo");
    assert!(r.ui.take_events().iter().all(|e| *e == Event::Changed(f)));
    r.ui.key(&Input::Preedit("にほ".into()));
    r.frame(&build);
    assert_eq!(r.ui.edit_text(f), "héllo", "composition is not text yet");
    let ime = r.ui.ime_area().expect("caret for the candidate window");
    let field = r.node(f).and_then(Node::bounds).unwrap();
    assert!(f64::from(ime.x) > field.x0 + 30.0 && f64::from(ime.x) < field.x1);
    r.ui.key(&Input::Commit("日本".into()));
    assert_eq!(r.ui.edit_text(f), "héllo日本");
    assert!(r.key("Enter"));
    assert_eq!(r.ui.take_events().last(), Some(&Event::Submit(f)));

    r.frame(&build);
    let node = r.node(f).unwrap();
    assert_eq!(node.role(), Role::TextInput);
    assert_eq!(node.label(), Some("Name"));
    assert_eq!(node.value(), Some("héllo日本"));
    let runs = r.nodes(Role::TextRun);
    assert_eq!(runs.len(), 1);
    assert_eq!(runs[0].value(), Some("héllo日本"));
    assert_eq!(runs[0].character_lengths(), &[1, 2, 1, 1, 1, 3, 3]);
    let sel = node.text_selection().expect("caret");
    assert_eq!(sel.focus.character_index, 7);
    assert_eq!(r.laid.as_ref().unwrap().tree.focus.0, f.0);
}

#[test]
fn composer_submits_on_enter_and_breaks_lines_on_shift_enter() {
    let c = Id::new("composer");
    let build = |ui: &Ui| {
        let _ = ui;
        widgets::composer(c, "Describe your task...", [div().w(20.0).h(20.0)]).w(400.0)
    };
    let mut r = Rig::new();
    r.frame(&build);
    let node = r.node(c).unwrap();
    assert_eq!(node.role(), Role::MultilineTextInput);
    assert_eq!(node.bounds().map(|b| b.y1 - b.y0), Some(124.0));
    r.click(c);
    r.typed("fix it");
    r.ui.key(&Input::Key {
        code: "Enter".into(),
        mods: mods::SHIFT,
        text: Some("\r".into()),
    });
    r.typed("now");
    assert_eq!(r.ui.edit_text(c), "fix it\nnow");
    r.ui.take_events();
    assert!(r.key("Enter"));
    assert_eq!(r.ui.take_events(), vec![Event::Submit(c)]);
    r.frame(&build);
    assert_eq!(r.nodes(Role::TextRun).len(), 2, "a run per line");
}

#[test]
fn split_drags_within_bounds_and_commits_on_release() {
    let s = Id::new("split");
    let build = |ui: &Ui| widgets::split(ui, s, true, 0.5, div(), div()).h(200.0);
    let mut r = Rig::new();
    r.frame(&build);
    let node = r.node(s).unwrap();
    assert_eq!(node.role(), Role::Splitter);
    let (x, y) = r.center(s);
    // Grabbable 3px off the line.
    r.ui.pointer_move(x + 3.0, y);
    assert_eq!(r.ui.cursor(), vornui::Cursor::ColResize);
    r.ui.pointer_down();
    r.ui.pointer_move(16.0 + 448.0 * 0.25, y);
    r.ui.pointer_move(16.0, y);
    r.ui.pointer_up();
    let ev = r.ui.take_events();
    assert!(
        matches!(ev[0], Event::Resized { id, ratio } if id == s && (ratio - 0.25).abs() < 0.01)
    );
    assert!(
        matches!(ev.last(), Some(Event::ResizeEnd { ratio, .. }) if (*ratio - vornui::MIN_SPLIT).abs() < 1e-6)
    );
    r.frame(&build);
    let (x2, _) = r.center(s);
    assert!(
        (x2 - (16.0 + 447.0 * vornui::MIN_SPLIT)).abs() < 1.5,
        "{x2}"
    );
}

#[test]
fn list_scrolls_with_the_wheel_and_screen_readers() {
    let l = Id::new("list");
    let g = Id::new("rows");
    let build = |ui: &Ui| {
        widgets::list(
            l,
            "Sessions",
            (0..40).map(|i| {
                widgets::option(
                    ui,
                    g,
                    i,
                    i == 3,
                    Choice {
                        label: "row",
                        hint: None,
                    },
                )
            }),
        )
        .h(150.0)
    };
    let mut r = Rig::new();
    r.frame(&build);
    let (x, y) = r.center(l);
    r.ui.pointer_move(x, y);
    r.ui.wheel(100.0);
    assert_eq!(r.ui.scroll_offset(l), 100.0);
    r.ui.wheel(1e6);
    r.frame(&build);
    let max = 40.0 * 30.0 - 150.0;
    assert_eq!(r.ui.scroll_offset(l), max);
    let node = r.node(l).unwrap();
    assert_eq!(node.role(), Role::ListBox);
    assert_eq!(node.scroll_y_max(), Some(f64::from(max)));
    r.ui.access(&vornui::accesskit::ActionRequest {
        action: vornui::accesskit::Action::ScrollUp,
        target_tree: vornui::accesskit::TreeId::ROOT,
        target_node: NodeId(l.0),
        data: None,
    });
    assert_eq!(r.ui.scroll_offset(l), max - 40.0);
    // Rows scrolled out are not clickable through the list's edge.
    r.ui.pointer_move(x, SIZE.1 - 2.0);
    r.ui.pointer_down();
    r.ui.pointer_up();
    assert!(r.ui.take_events().is_empty());
}

#[test]
fn tabs_select_close_and_add() {
    let s = Id::new("strip");
    let tabs = [
        Tab {
            label: "zsh",
            closable: true,
        },
        Tab {
            label: "a very long tab label that has to be cut",
            closable: true,
        },
    ];
    let build = |ui: &Ui| widgets::tab_strip(ui, s, &tabs, 0);
    let mut r = Rig::new();
    r.frame(&build);
    let t = r.nodes(Role::Tab);
    assert_eq!(t.len(), 2);
    assert_eq!(t[0].is_selected(), Some(true));
    let b = t[1].bounds().unwrap();
    assert!(b.x1 - b.x0 <= 170.0 + 0.5);
    r.click(widgets::tab_id(s, 1));
    assert_eq!(
        r.ui.take_events(),
        vec![Event::Select { group: s, index: 1 }]
    );
    r.frame(&build);
    r.click(widgets::tab_close_id(s, 1));
    assert_eq!(
        r.ui.take_events(),
        vec![Event::Click(widgets::tab_close_id(s, 1))]
    );
    r.frame(&build);
    r.click(widgets::tab_add_id(s));
    assert_eq!(
        r.ui.take_events(),
        vec![Event::Click(widgets::tab_add_id(s))]
    );
}

#[test]
fn tab_walks_focus_and_shows_the_ring() {
    let a = Id::new("a");
    let b = Id::new("b");
    let build = |ui: &Ui| {
        div()
            .row()
            .gap(8.0)
            .child(widgets::button(ui, a, "One"))
            .child(widgets::button(ui, b, "Two"))
    };
    let mut r = Rig::new();
    r.frame(&build);
    assert!(r.key("Tab"));
    assert!(r.ui.focused(a));
    assert!(r.key("Tab"));
    assert!(r.ui.focused(b));
    r.frame(&build);
    let bb = r.node(b).and_then(Node::bounds).unwrap();
    // The ring sits 1px outside the box, at 45% white.
    let ring = r.pixel(((bb.x0 + bb.x1) as f32 / 2.0, bb.y0 as f32 - 1.5));
    near(ring, over(color::SURFACE_BASE, Rgba::white(0.45)));
    assert!(r.key("Space"));
    assert_eq!(r.ui.take_events(), vec![Event::Click(b)]);
}

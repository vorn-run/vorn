//! The component gallery: every vornui primitive and every composed Vorn
//! component in each of its states.
//!
//! `cargo run -p vorn-components --example gallery` opens it in a window;
//! `-- --render DIR` draws each case offscreen to `DIR/<case>.png` and
//! writes `DIR/manifest.json`, which the fidelity harness replays against
//! the renderer's components (same size, same pointer, same clock).

use std::cell::Cell;
use std::fmt::Write as _;
use std::time::{Duration, Instant};

use vorn_components::card::{card_actions, CardActions, CardVariant};
use vorn_components::dock::{
    approval_pill, dock, minimized_pill, Approval, ApprovalIds, DockIds, DockKind, DockState,
    Minimized,
};
use vorn_components::sidebar::{session_row, SessionRow, SessionRowIds};
use vorn_components::{agent_icon, status_dot, status_icon, AgentMarks, Status};
use vornui::theme::color;
use vornui::widgets::{
    confirm, icon_button, item_id, kbd, picker, pills, toast, toggle, Choice, PickerLook, Pill,
    ToastKind, Tone,
};
use vornui::{div, text, Action, El, Event, Id, Input, Laid, RenderMode, Ui, UiConfig};

/// What the pointer or keyboard does before a case is drawn.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Act {
    Rest,
    /// Past the hover fade, short of the tooltip delay.
    Hover,
    /// Resting long enough for the tooltip.
    Tip,
    /// The button held down over the target.
    Press,
    /// Tab from nothing focused.
    Focus,
    /// A click, then the open animation run out.
    Click,
}

impl Act {
    fn name(self) -> &'static str {
        match self {
            Act::Rest => "rest",
            Act::Hover => "hover",
            Act::Tip => "tip",
            Act::Press => "press",
            Act::Focus => "focus",
            Act::Click => "click",
        }
    }
}

/// What a view sees: the agent marks, and whether its target was clicked
/// (for what the host opens, such as a confirmation).
struct Ctx {
    marks: AgentMarks,
    clicked: Cell<bool>,
}

type View = Box<dyn Fn(&Ui, &Ctx) -> El>;

struct Case {
    layer: &'static str,
    component: &'static str,
    state: &'static str,
    /// The renderer's component the case is compared with.
    source: &'static str,
    size: (f32, f32),
    /// Where the component's top-left sits in the case.
    at: (f32, f32),
    act: Act,
    target: Option<Id>,
    /// The CSS selector of the same target in the renderer's DOM.
    selector: &'static str,
    /// Seconds into the shared animation clock.
    clock: f32,
    view: View,
}

impl Case {
    fn id(&self) -> String {
        format!("{}--{}", self.component, self.state)
    }
}

/// A case's place in the gallery and what it is compared with.
struct Meta {
    layer: &'static str,
    component: &'static str,
    state: &'static str,
    source: &'static str,
    size: (f32, f32),
}

#[derive(Default)]
struct Builder(Vec<Case>);

impl Builder {
    fn add(
        &mut self,
        m: Meta,
        act: Act,
        target: Option<(Id, &'static str)>,
        view: impl Fn(&Ui, &Ctx) -> El + 'static,
    ) -> &mut Case {
        self.0.push(Case {
            layer: m.layer,
            component: m.component,
            state: m.state,
            source: m.source,
            size: m.size,
            at: (16.0, 16.0),
            act,
            target: target.map(|t| t.0),
            selector: target.map_or("", |t| t.1),
            clock: 0.0,
            view: Box::new(view),
        });
        self.0.last_mut().expect("just pushed")
    }
}

const P: &str = "primitives";
const C: &str = "composed";

fn meta(
    layer: &'static str,
    component: &'static str,
    state: &'static str,
    source: &'static str,
    size: (f32, f32),
) -> Meta {
    Meta {
        layer,
        component,
        state,
        source,
        size,
    }
}

fn lucide(name: &str) -> &'static str {
    // The gallery lives for the process; its few icons can too.
    vornui::icons::lucide(name, 2.0).unwrap_or_default().leak()
}

fn primitives(b: &mut Builder) {
    let btn = Id::new("icon-button");
    let sel = "button[aria-label='Settings']";
    let gear = lucide("settings");
    for (state, act) in [
        ("rest", Act::Rest),
        ("hover", Act::Hover),
        ("tooltip", Act::Tip),
        ("pressed", Act::Press),
        ("focus", Act::Focus),
        ("disabled", Act::Hover),
    ] {
        let off = state == "disabled";
        b.add(
            meta(P, "icon-button", state, "IconButton.tsx", (160.0, 90.0)),
            act,
            Some((btn, sel)),
            move |ui, _| {
                let el = icon_button(ui, btn, gear, 14.0, "Settings");
                if off {
                    el.disabled(true).opacity(0.4)
                } else {
                    el
                }
            },
        )
        .at = (64.0, 48.0);
    }

    for (state, on, off) in [
        ("on", true, false),
        ("off", false, false),
        ("disabled", true, true),
    ] {
        b.add(
            meta(
                P,
                "toggle",
                state,
                "settings/ToggleSwitch.tsx",
                (100.0, 56.0),
            ),
            Act::Rest,
            None,
            move |_, _| toggle(Id::new("toggle"), "Toggle", on, off),
        );
    }

    b.add(
        meta(P, "kbd", "rest", "KbdHint.tsx", (100.0, 48.0)),
        Act::Rest,
        None,
        |_, _| kbd("\u{2318}N", (10.0, 10.0), color::GRAY_500),
    );

    let pick = Id::new("picker");
    let choices = [
        Choice {
            label: "Newest first",
            hint: None,
        },
        Choice {
            label: "Oldest first",
            hint: None,
        },
        Choice {
            label: "By name",
            hint: None,
        },
    ];
    for (state, act) in [
        ("rest", Act::Rest),
        ("hover", Act::Hover),
        ("open", Act::Click),
    ] {
        b.add(
            meta(
                P,
                "picker",
                state,
                "SelectPicker.tsx (compact)",
                (240.0, 180.0),
            ),
            act,
            Some((pick, "button[aria-haspopup='listbox']")),
            move |ui, _| picker(ui, pick, "Sort", PickerLook::Plain, &choices, 0),
        )
        .at = (24.0, 16.0);
    }

    let group = Id::new("view-pills");
    let items = [
        Pill::icon_only("Sessions", lucide("monitor"), "\u{2318}S"),
        Pill::icon_only("Tasks", lucide("list-todo"), "\u{2318}T"),
        Pill::icon_only("Workflows", lucide("workflow"), "\u{2318}\u{21e7}W"),
    ];
    for (state, act) in [
        ("rest", Act::Rest),
        ("hover", Act::Hover),
        ("tooltip", Act::Tip),
    ] {
        b.add(
            meta(P, "segmented", state, "MainViewPills.tsx", (200.0, 90.0)),
            act,
            Some((item_id(group, 1), "button[aria-label='Tasks']")),
            move |ui, _| pills(ui, group, "View", &items, 0),
        );
    }

    let trigger = Id::new("confirm-trigger");
    for (state, tone, ok) in [
        ("danger", Tone::Danger, "Close"),
        ("neutral", Tone::Neutral, "Apply"),
    ] {
        b.add(
            meta(P, "confirm", state, "ConfirmPopover.tsx", (320.0, 150.0)),
            Act::Click,
            Some((trigger, "button[data-g='trigger']")),
            move |ui, ctx| {
                let t = div()
                    .id(trigger)
                    .on(Action::Click)
                    .px(10.0)
                    .py(4.0)
                    .child(text("Open", 12.0, color::GRAY_300).line_height(18.0));
                if !ctx.clicked.get() {
                    return t;
                }
                confirm(
                    ui,
                    t,
                    "Close this session?",
                    (Id::new("ok"), ok),
                    Id::new("cancel"),
                    tone,
                )
            },
        )
        .at = (130.0, 16.0);
    }

    for (state, kind, msg) in [
        ("success", ToastKind::Success, "Session \"api\" closed"),
        ("error", ToastKind::Error, "Could not start the agent"),
        (
            "warning",
            ToastKind::Warning,
            "Worktree has uncommitted changes",
        ),
        ("info", ToastKind::Info, "Copied to clipboard"),
    ] {
        b.add(
            meta(P, "toast", state, "Toast.tsx", (400.0, 76.0)),
            Act::Rest,
            None,
            move |ui, _| toast(ui, Id::new("toast-close"), msg, kind),
        );
    }
}

const AGENTS: [&str; 6] = ["claude", "copilot", "codex", "opencode", "gemini", "shell"];

fn composed(b: &mut Builder) {
    for agent in AGENTS {
        b.add(
            meta(C, "agent-icon", agent, "AgentIcon.tsx", (48.0, 48.0)),
            Act::Rest,
            None,
            move |_, ctx| agent_icon(&ctx.marks, agent, 16.0),
        );
    }
    for (state, agent, status, clock) in [
        ("running-0s", "claude", Status::Running, 0.0),
        ("running-0.7s", "claude", Status::Running, 0.7),
        ("running-1.6s", "codex", Status::Running, 1.6),
        ("waiting", "claude", Status::Waiting, 0.0),
        ("idle", "copilot", Status::Idle, 0.0),
        ("shell-running", "shell", Status::Running, 0.0),
    ] {
        b.add(
            meta(C, "status-icon", state, "AgentStatusIcon.tsx", (48.0, 48.0)),
            Act::Rest,
            None,
            move |ui, ctx| status_icon(ui, &ctx.marks, agent, status, 14.0, 1.0),
        )
        .clock = clock;
    }
    for (state, status, clock) in [
        ("running-0s", Status::Running, 0.0),
        ("running-1s", Status::Running, 1.0),
        ("waiting", Status::Waiting, 0.0),
        ("idle", Status::Idle, 0.0),
        ("error", Status::Error, 0.0),
    ] {
        b.add(
            meta(
                C,
                "status-dot",
                state,
                "STATUS_DOT (status-colors.ts)",
                (40.0, 40.0),
            ),
            Act::Rest,
            None,
            move |ui, _| status_dot(ui, status),
        )
        .clock = clock;
    }

    card_cases(b);
    dock_cases(b);

    let row = SessionRowIds {
        row: Id::new("row"),
        more: None,
        close: Id::new("row").child("close"),
    };
    for (state, act, active) in [
        ("rest", Act::Rest, false),
        ("hover", Act::Hover, false),
        ("active", Act::Rest, true),
    ] {
        b.add(
            meta(
                C,
                "session-row",
                state,
                "project-sidebar/SessionItem.tsx",
                (260.0, 60.0),
            ),
            act,
            Some((row.row, "div[data-g='row'] > div")),
            move |ui, ctx| {
                div().w(228.0).col().child(session_row(
                    ui,
                    &ctx.marks,
                    row,
                    &SessionRow {
                        name: "api-server",
                        branch: None,
                        agent: "claude",
                        status: Status::Waiting,
                        active,
                        detail: None,
                        steps: &[],
                    },
                ))
            },
        );
    }
}

fn card_ids(variant: CardVariant) -> CardActions<'static> {
    let (name, key) = match variant {
        CardVariant::Mini => ("mini", "\u{2318}O"),
        CardVariant::Focused => ("focused", "\u{2318}W"),
    };
    let base = Id::new("card").child(name);
    CardActions {
        variant,
        more: base.child("more"),
        panes: &[],
        minimize: base.child("min"),
        toggle: base.child("toggle"),
        close: base.child("close"),
        toggle_key: key,
        confirming: None,
    }
}

fn card_cases(b: &mut Builder) {
    let mini = card_ids(CardVariant::Mini);
    let expand = (mini.toggle, "button[aria-label='Expand session']");
    let close = (mini.close, "button[aria-label='Close session']");
    for (state, act, target) in [
        ("rest", Act::Rest, None),
        ("hover-expand", Act::Hover, Some(expand)),
        ("tooltip-expand", Act::Tip, Some(expand)),
        ("hover-close", Act::Hover, Some(close)),
        ("confirm-close", Act::Click, Some(close)),
    ] {
        b.add(
            meta(
                C,
                "card-actions",
                state,
                "card/CardActionCluster.tsx (mini)",
                (300.0, 170.0),
            ),
            act,
            target,
            move |ui, ctx| {
                let a = card_ids(CardVariant::Mini);
                let confirming = ctx
                    .clicked
                    .get()
                    .then(|| (a.close.child("ok"), a.close.child("cancel")));
                card_actions(ui, &CardActions { confirming, ..a })
            },
        )
        .at = (150.0, 48.0);
    }
    let collapse = (
        card_ids(CardVariant::Focused).toggle,
        "button[aria-label='Collapse session']",
    );
    for (state, act) in [
        ("focused-rest", Act::Rest),
        ("focused-tooltip-collapse", Act::Tip),
    ] {
        b.add(
            meta(
                C,
                "card-actions",
                state,
                "card/CardActionCluster.tsx (focused)",
                (300.0, 90.0),
            ),
            act,
            Some(collapse),
            move |ui, _| card_actions(ui, &card_ids(CardVariant::Focused)),
        )
        .at = (150.0, 16.0);
    }
}

/// The gate the harness seeds: the dock's says less than the lone pill's.
fn approval(message: &str) -> Approval<'_> {
    Approval {
        workflow: "Release notes",
        icon: "workflow",
        icon_color: None,
        message: Some(message),
    }
}

fn approval_ids(name: &str) -> ApprovalIds {
    let base = Id::new(name);
    ApprovalIds {
        open: base,
        approve: base.child("ok"),
        reject: base.child("no"),
    }
}

fn dock_cases(b: &mut Builder) {
    let pill = Id::new("minimized");
    for (state, act, branch, worktree) in [
        ("rest", Act::Rest, Some("main"), false),
        ("hover", Act::Hover, Some("main"), false),
        ("worktree", Act::Rest, Some("fix-login"), true),
        ("no-branch", Act::Rest, None, false),
    ] {
        b.add(
            meta(
                C,
                "minimized-pill",
                state,
                "MinimizedPill.tsx",
                (320.0, 56.0),
            ),
            act,
            Some((pill, "[data-g='pill'] > button")),
            move |ui, ctx| {
                minimized_pill(
                    ui,
                    &ctx.marks,
                    pill,
                    &Minimized {
                        name: "api-server",
                        agent: "claude",
                        status: Status::Waiting,
                        branch,
                        worktree,
                    },
                )
            },
        );
    }

    let ids = approval_ids("approval");
    for (state, act, target) in [
        ("rest", Act::Rest, None),
        (
            "hover",
            Act::Hover,
            Some((ids.open, "[data-g='approval'] > div")),
        ),
        (
            "hover-approve",
            Act::Hover,
            Some((ids.approve, "button[aria-label='Approve']")),
        ),
    ] {
        b.add(
            meta(
                C,
                "approval-pill",
                state,
                "WaitingApprovalPill.tsx",
                (520.0, 56.0),
            ),
            act,
            target,
            move |ui, _| approval_pill(ui, ids, &approval("Review the draft before publishing")),
        );
    }

    let dock_ids = DockIds {
        chip: Id::new("dock-chip"),
        overflow: Id::new("dock-more"),
        collapse: Id::new("dock-collapse"),
        expand: Id::new("dock-expand"),
    };
    for (state, collapsed, n, act, target) in [
        ("expanded", false, 3, Act::Rest, None),
        (
            "overflow-open",
            false,
            6,
            Act::Click,
            Some((dock_ids.overflow, "[data-g='dock'] button[aria-haspopup]")),
        ),
        ("collapsed", true, 3, Act::Rest, None),
        (
            "collapsed-open",
            true,
            3,
            Act::Click,
            Some((dock_ids.chip, "[data-g='dock'] button[aria-haspopup]")),
        ),
    ] {
        let h = if act == Act::Click { 320.0 } else { 56.0 };
        b.add(
            meta(C, "dock", state, "SessionDock.tsx", (1100.0, h)),
            act,
            target,
            move |ui, ctx| {
                let names = ["api-server", "web", "docs", "infra", "mobile", "billing"];
                let mut items = vec![(
                    DockKind::Waiting,
                    approval_pill(
                        ui,
                        approval_ids("dock-approval"),
                        &approval("Review the draft"),
                    ),
                )];
                for (i, name) in names.iter().take(n - 1).enumerate() {
                    let m = Minimized {
                        name,
                        agent: AGENTS[i % 5],
                        status: Status::Idle,
                        branch: Some("main"),
                        worktree: false,
                    };
                    let id = Id::new("dock-pill").child(i);
                    items.push((DockKind::Minimized, minimized_pill(ui, &ctx.marks, id, &m)));
                }
                let state = DockState {
                    collapsed,
                    open: ctx.clicked.get(),
                };
                dock(ui, dock_ids, state, items).unwrap_or_else(div)
            },
        );
    }
}

fn cases() -> Vec<Case> {
    let mut b = Builder::default();
    primitives(&mut b);
    composed(&mut b);
    b.0
}

/// One case laid out and ready to draw, the clock at `now`.
fn lay(ui: &mut Ui, case: &Case, ctx: &Ctx, now: Instant) -> Laid {
    ui.begin_at(color::SURFACE_BASE, now);
    let el = (case.view)(ui, ctx);
    let root = div()
        .size(case.size.0, case.size.1)
        .row()
        .items_start()
        .pl(case.at.0)
        .pt(case.at.1)
        .child(el);
    ui.layout(root, case.size)
}

/// Drives `case` through its act on `ui`, the clock starting at `now`.
fn stage(ui: &mut Ui, case: &Case, ctx: &Ctx, mut now: Instant) {
    ctx.clicked.set(false);
    ui.set_now(now);
    lay(ui, case, ctx, now);
    let centre = case
        .target
        .and_then(|t| ui.rect_of(t))
        .map(|r| (r.x + r.w / 2.0, r.y + r.h / 2.0));
    let wait = |now: &mut Instant, ms| {
        *now += Duration::from_millis(ms);
        *now
    };
    match (case.act, centre) {
        (Act::Rest, _) => {}
        (Act::Focus, _) => {
            ui.key(&Input::named("Tab"));
        }
        (_, Some((x, y))) => {
            ui.pointer_move(x, y);
            lay(ui, case, ctx, now);
            let ms = if case.act == Act::Tip { 600 } else { 250 };
            ui.set_now(wait(&mut now, ms));
            lay(ui, case, ctx, now);
            if matches!(case.act, Act::Press | Act::Click) {
                ui.pointer_down();
                lay(ui, case, ctx, now);
            }
            if case.act == Act::Click {
                ui.pointer_up();
                let clicked = ui
                    .take_events()
                    .iter()
                    .any(|e| matches!(e, Event::Click(id) if Some(*id) == case.target));
                ctx.clicked.set(clicked);
                // Away, so nothing is drawn hovered; then past the entrance.
                ui.pointer_move(1.0, 1.0);
                lay(ui, case, ctx, now);
                ui.set_now(wait(&mut now, 400));
            }
        }
        (_, None) => eprintln!("{}: target not on screen", case.id()),
    }
    lay(ui, case, ctx, now);
}

/// Leaves `ui` with nothing hovered, held, focused or open.
fn reset(ui: &mut Ui) {
    ui.pointer_up();
    ui.pointer_leave();
    ui.close_menu();
    ui.set_focus(None);
    ui.take_events();
}

fn json(s: &str) -> String {
    s.replace('\\', "\\\\").replace('"', "\\\"")
}

fn manifest(cases: &[Case], scale: f32) -> String {
    let mut s = format!("{{\n  \"scale\": {scale},\n  \"cases\": [\n");
    for (i, c) in cases.iter().enumerate() {
        let comma = if i + 1 == cases.len() { "" } else { "," };
        let _ = writeln!(
            s,
            "    {{\"id\": \"{}\", \"layer\": \"{}\", \"component\": \"{}\", \"state\": \"{}\", \
             \"source\": \"{}\", \"w\": {}, \"h\": {}, \"x\": {}, \"y\": {}, \"act\": \"{}\", \
             \"selector\": \"{}\", \"clock\": {}}}{comma}",
            c.id(),
            c.layer,
            c.component,
            c.state,
            json(c.source),
            c.size.0,
            c.size.1,
            c.at.0,
            c.at.1,
            c.act.name(),
            json(c.selector),
            c.clock,
        );
    }
    s.push_str("  ]\n}\n");
    s
}

fn render(dir: &str, only: Option<&str>) -> Result<(), String> {
    const SCALE: f32 = 2.0;
    std::fs::create_dir_all(dir).map_err(|e| e.to_string())?;
    let mut ui = Ui::headless(RenderMode::Auto, &UiConfig::system(SCALE, 13.0));
    let mut ctx = Ctx {
        marks: AgentMarks::default(),
        clicked: Cell::new(false),
    };
    ctx.marks.load(&mut ui);
    let cases = cases();
    let epoch = Instant::now();
    for (i, case) in cases.iter().enumerate() {
        if only.is_some_and(|o| !case.id().contains(o)) {
            continue;
        }
        reset(&mut ui);
        // Far enough apart that the case before has faded out; each case's
        // animations count from its own start, as the harness's do.
        let start = epoch + Duration::from_secs(1000 * (i as u64 + 1));
        ui.set_anim_epoch(start);
        let start = start + Duration::from_secs_f32(case.clock);
        let px = ((case.size.0 * SCALE) as u32, (case.size.1 * SCALE) as u32);
        let target = ui.offscreen(px);
        // A full atlas empties itself; the second pass fits one case.
        for _ in 0..2 {
            stage(&mut ui, case, &ctx, start);
            if ui.render_offscreen(&target) {
                break;
            }
        }
        ui.save_png(&target, &format!("{dir}/{}.png", case.id()))?;
    }
    std::fs::write(format!("{dir}/manifest.json"), manifest(&cases, SCALE))
        .map_err(|e| e.to_string())
}

/// The gallery in a window: every case that needs no pointer, by layer.
struct Gallery {
    cases: Vec<Case>,
    ctx: Ctx,
}

impl vornui::App for Gallery {
    fn view(&mut self, ui: &mut Ui) -> El {
        self.ctx.marks.load(ui);
        let mut col = div()
            .id(Id::new("gallery"))
            .col()
            .scroll_y()
            .p(24.0)
            .gap(12.0)
            .bg(color::SURFACE_BASE);
        let mut last = ("", "");
        for c in &self.cases {
            if c.layer != last.0 {
                col = col.child(text(c.layer, 18.0, color::WHITE).weight(600).mt(12.0));
            }
            if c.component != last.1 {
                col = col.child(text(c.component, 13.0, color::GRAY_400).mt(8.0));
            }
            last = (c.layer, c.component);
            if c.act != Act::Rest {
                continue;
            }
            col = col.child(
                div()
                    .row()
                    .items_start()
                    .gap(16.0)
                    .child(text(c.state, 11.0, color::GRAY_500).w(120.0))
                    .child((c.view)(ui, &self.ctx)),
            );
        }
        col
    }
}

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let flag = |name: &str| {
        args.iter()
            .position(|a| a == name)
            .map(|i| args.get(i + 1).map(String::as_str))
    };
    let result = match flag("--render") {
        Some(dir) => render(dir.unwrap_or("gallery"), flag("--only").flatten()),
        None => vornui::run(
            "vornui gallery",
            (960.0, 900.0),
            UiConfig::system(1.0, 13.0),
            |_, _| Gallery {
                cases: cases(),
                ctx: Ctx {
                    marks: AgentMarks::default(),
                    clicked: Cell::new(false),
                },
            },
        ),
    };
    if let Err(e) = result {
        eprintln!("gallery: {e}");
        std::process::exit(1);
    }
}

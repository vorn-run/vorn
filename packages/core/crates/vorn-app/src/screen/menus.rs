//! The app's menus (`GridContextMenu`, `CardContextMenu`,
//! `TerminalContextMenu`, the toolbar's popovers and the launcher's
//! pickers): their entries as data, and one panel that draws any of them
//! at a point, with a submenu beside the hovered entry.

use super::{c, ico, white, Screen};
use crate::act::Act;
use crate::look::{self, Rgb};
use crate::state::{Filter, LayoutMode, MenuKind, Sort};
use crate::ui::{div, item_id, text, Action, El, Id, Place, Rgba, Role, Ui};

/// The open menu's choice group; entries report `Select` in it.
pub const MENU: Id = Id(0x6d65_6e75_0000_0001);
/// The open submenu's choice group.
pub const SUB: Id = Id(0x6d65_6e75_0000_0003);

/// One entry of a menu.
#[derive(Default)]
pub struct Item {
    pub icon: Option<(&'static str, Rgb)>,
    /// A coloured dot instead of an icon (the status filter).
    pub dot: Option<Rgb>,
    pub label: String,
    pub detail: Option<String>,
    pub shortcut: Option<String>,
    pub act: Option<Act>,
    pub sub: Vec<Item>,
    /// A rule above it.
    pub separator: bool,
    /// A section title rather than a choice.
    pub header: bool,
    pub disabled: bool,
    pub checked: bool,
    pub danger: bool,
    pub mono: bool,
    /// A row drawn by the caller (the layout buttons); not a choice.
    pub row: Option<El>,
    /// A `SelectPicker` row: the check on the left, 30 px tall.
    pub picker: bool,
    /// A glyph drawn by the caller (a connector's or a project's mark).
    pub mark: Option<El>,
    /// A plain `px-3 py-2` button with a 12 px icon in its text colour.
    pub plain: bool,
    /// An `OptionRow` menu's entry: check on the left, sections of `py-1.5`.
    pub option_row: bool,
    /// A `KanbanCardMenu` entry: 28 px, the label always secondary.
    pub card_row: bool,
}

impl Item {
    pub fn new(icon: &'static str, label: impl Into<String>, act: Act) -> Item {
        Item {
            icon: Some((icon, look::GRAY_500)),
            label: label.into(),
            act: Some(act),
            ..Item::default()
        }
    }

    /// Drawn as a plain menu button (`plain_row`).
    pub fn plain(mut self) -> Item {
        self.plain = true;
        self
    }

    pub fn header(label: impl Into<String>) -> Item {
        Item {
            label: label.into(),
            header: true,
            ..Item::default()
        }
    }

    pub fn option(label: impl Into<String>, checked: bool, act: Act) -> Item {
        Item {
            label: label.into(),
            act: Some(act),
            checked,
            ..Item::default()
        }
    }

    /// A `SelectPicker` option.
    pub fn pick(label: impl Into<String>, checked: bool, act: Act) -> Item {
        Item {
            label: label.into(),
            act: Some(act),
            checked,
            picker: true,
            ..Item::default()
        }
    }

    pub fn mark(mut self, el: El) -> Item {
        self.mark = Some(el);
        self
    }

    pub fn sep(mut self) -> Item {
        self.separator = true;
        self
    }

    pub fn submenu(mut self, items: Vec<Item>) -> Item {
        self.sub = items;
        self
    }

    pub fn detail(mut self, d: impl Into<String>) -> Item {
        self.detail = Some(d.into());
        self
    }

    pub fn tint(mut self, color: Rgb) -> Item {
        if let Some((name, _)) = self.icon {
            self.icon = Some((name, color));
        }
        self
    }

    pub fn disabled(mut self, d: bool) -> Item {
        self.disabled = d;
        self
    }

    fn choosable(&self) -> bool {
        !self.header && self.row.is_none()
    }
}

/// How wide each menu is, as the renderer sizes them.
pub fn width(kind: &MenuKind) -> f32 {
    if let Some(w) = super::workflows::menu_width(kind) {
        return w;
    }
    match kind {
        MenuKind::Grid | MenuKind::Card(_) => 220.0,
        MenuKind::Terminal(_) => 180.0,
        MenuKind::ViewOptions => 200.0,
        MenuKind::Project(_) => 140.0,
        MenuKind::LaunchAgent => 160.0,
        MenuKind::LaunchProject | MenuKind::LaunchWorktree => 240.0,
        MenuKind::LaunchBranch => 220.0,
        MenuKind::Recent | MenuKind::Headless => 280.0,
        MenuKind::TaskOptions | MenuKind::TaskStatus(_) | MenuKind::TaskAgent(_) => 200.0,
        MenuKind::TaskCard(_) | MenuKind::TaskProject(_) => 180.0,
        MenuKind::SetShell => 288.0,
        MenuKind::SetSelect(_) => 128.0,
        MenuKind::SshKey(_) => 300.0,
        MenuKind::OpenIn(_) => 180.0,
        MenuKind::Branch(_) | MenuKind::FileTabs(_) => 220.0,
        _ => 200.0,
    }
}

fn row(ui: &Ui, group: Id, i: usize, item: Item, sub_open: Option<El>) -> El {
    if let Some(r) = item.row {
        return r;
    }
    if item.header && item.option_row {
        return div().px(12.0).py(4.0).child(
            text(item.label.to_uppercase(), 10.0, c(look::GRAY_500))
                .tracking(0.05)
                .line_height(15.0)
                .role(Role::Label, item.label.as_str()),
        );
    }
    if item.header {
        return div().px(12.0).pt(6.0).pb(4.0).child(
            text(item.label.to_uppercase(), 10.0, c(look::GRAY_500))
                .line_height(14.0)
                .role(Role::Label, item.label.as_str()),
        );
    }
    let id = item_id(group, i);
    if item.picker {
        return picker_row(ui, group, i, id, item);
    }
    if item.plain {
        return plain_row(ui, group, i, id, item);
    }
    let hot = ui.hover(id).max(if sub_open.is_some() { 1.0 } else { 0.0 });
    if item.card_row {
        return card_row(group, i, id, hot, item);
    }
    let fg = if item.danger {
        c(look::DANGER)
    } else if item.checked {
        c(look::WHITE)
    } else {
        c(look::GRAY_300).mix(c(look::WHITE), hot)
    };
    let bg = if item.checked {
        white(0.06)
    } else {
        white(0.0).mix(white(0.06), hot)
    };
    let mut r = div()
        .id(id)
        .row()
        .items_center()
        .gap(10.0)
        .px(12.0)
        .h(32.0)
        .bg(bg)
        .role(Role::MenuItem, item.label.as_str())
        .selected(item.checked);
    if item.disabled {
        r = r.opacity(0.4).disabled(true);
    } else {
        r = r.on(Action::Select { group, index: i });
    }
    if let Some(dot) = item.dot {
        r = r.child(div().size(8.0, 8.0).rounded(4.0).bg(c(dot)).shrink0());
    }
    if let Some((name, color)) = item.icon {
        let color = if item.danger { look::DANGER } else { color };
        r = r.child(ico(name, 14.0, 1.5, c(color)).shrink0());
    }
    let mut label = text(&item.label, 12.0, fg)
        .line_height(16.0)
        .truncate()
        .silent();
    if item.mono {
        label = label.mono();
    }
    r = r.child(div().row().grow().min_w(0.0).child(label));
    if let Some(d) = &item.detail {
        r = r.child(text(d, 10.0, c(look::GRAY_600)).line_height(14.0).silent());
    }
    if let Some(s) = &item.shortcut {
        r = r.child(
            text(s, 10.0, c(look::GRAY_600))
                .mono()
                .line_height(14.0)
                .silent(),
        );
    }
    if !item.sub.is_empty() {
        r = r.has_popup().expanded(sub_open.is_some()).child(ico(
            "chevron-right",
            11.0,
            2.0,
            c(look::GRAY_500),
        ));
    }
    if item.checked {
        r = r.child(ico("check", 11.0, 3.0, fg));
    }
    match sub_open {
        Some(panel) => r.overlay(panel, Place::Right, 4.0),
        None => r,
    }
}

/// `KanbanCardMenu`'s button: `px-3 py-1.5 text-xs text-ink-secondary`,
/// only the icon in the action's colour.
fn card_row(group: Id, i: usize, id: Id, hot: f32, item: Item) -> El {
    let mut r = div()
        .id(id)
        .row()
        .items_center()
        .gap(10.0)
        .px(12.0)
        .py(6.0)
        .bg(white(0.0).mix(white(0.06), hot))
        .role(Role::MenuItem, item.label.as_str())
        .on(Action::Select { group, index: i });
    if let Some((name, color)) = item.icon {
        let color = if item.danger { look::DANGER } else { color };
        r = r.child(ico(name, 14.0, 2.0, c(color)).shrink0());
    }
    r.child(
        text(
            &item.label,
            12.0,
            Rgba::hexa(look::INK, look::INK_SECONDARY),
        )
        .line_height(16.0)
        .silent(),
    )
}

/// A plain menu button: `text-[12px] text-gray-300` (danger at 80 %), the
/// icon in the text's colour.
fn plain_row(ui: &Ui, group: Id, i: usize, id: Id, item: Item) -> El {
    let hot = ui.hover(id);
    let fg = if item.danger {
        Rgba::hexa(look::DANGER, 0.8).mix(c(look::DANGER), hot)
    } else {
        c(look::GRAY_300).mix(c(look::WHITE), hot)
    };
    let mut r = div()
        .id(id)
        .row()
        .items_center()
        .gap(8.0)
        .px(12.0)
        .py(8.0)
        .bg(white(0.0).mix(white(0.06), hot))
        .role(Role::MenuItem, item.label.as_str());
    if item.disabled {
        r = r.opacity(0.4).disabled(true);
    } else {
        r = r.on(Action::Select { group, index: i });
    }
    if let Some((name, _)) = item.icon {
        r = r.child(ico(name, 12.0, 1.5, fg).shrink0());
    }
    r.child(
        div().row().grow().min_w(0.0).child(
            text(&item.label, 12.0, fg)
                .line_height(18.0)
                .truncate()
                .silent(),
        ),
    )
}

/// `SelectPicker`'s option: the check (or its room), the mark, the label
/// and a hint.
fn picker_row(ui: &Ui, group: Id, i: usize, id: Id, item: Item) -> El {
    let hot = ui.hover(id);
    let fg = if item.checked {
        c(look::WHITE)
    } else {
        c(look::GRAY_300).mix(c(look::WHITE), hot)
    };
    let bg = if item.checked {
        white(0.06)
    } else {
        white(0.0).mix(white(0.04), hot)
    };
    let mut r = div()
        .id(id)
        .row()
        .items_center()
        .gap(8.0)
        .px(12.0)
        .py(6.0)
        .bg(bg)
        .role(Role::MenuItem, item.label.as_str())
        .selected(item.checked)
        .child(if item.checked {
            ico("check", 11.0, 3.0, fg)
        } else {
            div().w(11.0).shrink0()
        });
    if let Some(d) = item.dot.filter(|_| item.option_row) {
        r = r.child(div().size(6.0, 6.0).rounded(3.0).bg(c(d)).shrink0());
    }
    if item.disabled {
        r = r.opacity(0.4).disabled(true);
    } else {
        r = r.on(Action::Select { group, index: i });
    }
    if let Some(m) = item.mark {
        r = r.child(div().shrink0().child(m));
    } else if let Some((name, color)) = item.icon {
        r = r.child(ico(name, 12.0, 2.0, c(color)).shrink0());
    }
    r = r.child(
        div().row().grow().min_w(0.0).child(
            text(&item.label, 12.0, fg)
                .line_height(18.0)
                .truncate()
                .silent(),
        ),
    );
    if let Some(d) = &item.detail {
        r = r.child(text(d, 10.0, c(look::GRAY_600)).line_height(15.0).silent());
    }
    r
}

/// A panel of `items`: `bg-surface-overlay border-white/10 rounded-lg
/// py-1`, rows of 32 px, separators as rules.
pub fn panel(ui: &Ui, group: Id, w: f32, name: &str, items: Vec<Item>, sub: Option<usize>) -> El {
    let p = ui.menu_progress(MENU);
    let edge = if items.iter().any(|i| i.plain || i.option_row) {
        0.08
    } else {
        0.10
    };
    // An `OptionRow` menu is sections of `py-1.5` with a rule between.
    let pad = if items.iter().any(|i| i.option_row) {
        6.0
    } else {
        4.0
    };
    let mut col = div()
        .col()
        .w(w)
        .py(pad)
        .bg(c(look::SURFACE_OVERLAY))
        .border(1.0, white(edge))
        .rounded(8.0)
        .opacity(p)
        .transform(0.96 + 0.04 * p, (0.0, -4.0 * (1.0 - p)))
        .role(Role::Menu, name);
    for (i, mut item) in items.into_iter().enumerate() {
        if item.separator && i > 0 {
            col = col.child(super::rule(pad));
        }
        let sub_items = std::mem::take(&mut item.sub);
        let open = sub == Some(i) && !sub_items.is_empty();
        let has_sub = !sub_items.is_empty();
        let sub_panel = open.then(|| {
            div()
                .id(SUB.child("scroll"))
                .col()
                .scroll_y()
                .max_h(320.0)
                .child(panel_inner(ui, SUB, sub_items))
        });
        if has_sub {
            // Keeps the chevron while the entries live in the submenu.
            item.sub.push(Item::default());
        }
        col = col.child(row(ui, group, i, item, sub_panel));
    }
    col
}

fn panel_inner(ui: &Ui, group: Id, items: Vec<Item>) -> El {
    let mut col = div()
        .col()
        .w(200.0)
        .py(4.0)
        .bg(c(look::SURFACE_OVERLAY))
        .border(1.0, white(0.10))
        .rounded(8.0)
        .role(Role::Menu, "Submenu");
    for (i, item) in items.into_iter().enumerate() {
        if item.separator && i > 0 {
            col = col.child(super::rule(4.0));
        }
        col = col.child(row(ui, group, i, item, None));
    }
    col
}

/// The entry `index` of a menu's items, or of the open submenu's.
pub fn chosen(mut items: Vec<Item>, sub: Option<usize>, group: Id, index: usize) -> Option<Item> {
    if group == SUB {
        let parent = items.get_mut(sub?)?;
        let mut subs = std::mem::take(&mut parent.sub);
        return (index < subs.len()).then(|| subs.swap_remove(index));
    }
    (index < items.len())
        .then(|| items.swap_remove(index))
        .filter(Item::choosable)
}

/// The entries of the menu `kind`, from what the screen shows.
pub fn items(s: &Screen<'_>, kind: &MenuKind) -> Vec<Item> {
    let mut items = kind_items(s, kind);
    // These menus are `OptionRow`s under small uppercase section titles.
    if matches!(
        kind,
        MenuKind::ViewOptions | MenuKind::TaskOptions | MenuKind::WorkflowFilter
    ) {
        for it in &mut items {
            it.option_row = true;
            if !it.header && it.row.is_none() {
                it.picker = true;
                it.icon = None;
            }
        }
    }
    if matches!(kind, MenuKind::TaskCard(_)) {
        for it in &mut items {
            it.card_row = true;
        }
    }
    items
}

fn kind_items(s: &Screen<'_>, kind: &MenuKind) -> Vec<Item> {
    match kind {
        MenuKind::Grid => grid(s),
        MenuKind::Card(id) => card(s, id),
        MenuKind::Terminal(id) => terminal(s, id),
        MenuKind::ViewOptions => view_options(s),
        MenuKind::Project(name) => project(s, name),
        MenuKind::LaunchProject => launch_projects(s),
        MenuKind::LaunchAgent => launch_agents(s),
        MenuKind::LaunchWorktree => launch_worktrees(s),
        MenuKind::LaunchBranch => launch_branches(s),
        MenuKind::Recent | MenuKind::Headless | MenuKind::WfIcon | MenuKind::SetShell => Vec::new(),
        MenuKind::SetSelect(key) => super::settings::select_items(s, key),
        MenuKind::SshKey(host) => super::settings::ssh_key_items(s, host),
        MenuKind::OpenIn(session) => super::git::open_in_items(s, session),
        MenuKind::FileTabs(owner) => super::panes::tab_items(s, owner),
        MenuKind::Branch(_) => Vec::new(),
        MenuKind::RunStatus
        | MenuKind::WorkflowFilter
        | MenuKind::WorkflowItem(_)
        | MenuKind::EditorMore
        | MenuKind::WfChoice(_)
        | MenuKind::PromptProject
        | MenuKind::NodeMore(_) => super::workflows::menu_items(s, kind),
        MenuKind::TaskOptions => super::tasks::options_items(s),
        MenuKind::TaskCard(id) => super::tasks::card_items(s, id),
        MenuKind::TaskStatus(f) => super::tasks::status_items(s, *f),
        MenuKind::TaskProject(f) => super::tasks::project_items(s, *f),
        MenuKind::TaskAgent(f) => super::tasks::agent_items(s, *f),
    }
}

/// The agents a session can start, in `AGENT_LIST`'s order.
pub const AGENTS: [(&str, &str); 5] = [
    ("claude", "Claude Code"),
    ("copilot", "GitHub Copilot"),
    ("codex", "Codex"),
    ("opencode", "OpenCode"),
    ("gemini", "Gemini"),
];

pub fn agent_name(agent: &str) -> &str {
    AGENTS
        .iter()
        .find(|(k, _)| *k == agent)
        .map_or(agent, |(_, n)| n)
}

fn installed<'a>(
    s: &'a Screen<'_>,
) -> impl Iterator<Item = &'static (&'static str, &'static str)> + 'a {
    AGENTS.iter().filter(|(k, _)| {
        s.st.installed
            .as_ref()
            .is_none_or(|i| i.iter().any(|a| a == k))
    })
}

/// `workflow-menu-items`: the workflows a menu offers, contextual ones only
/// for a session.
fn workflows(s: &Screen<'_>, source: Option<&str>) -> Vec<Item> {
    s.store
        .workflows
        .iter()
        .filter(|w| w.enabled)
        .filter(|w| contextual(&w.nodes) == source.is_some())
        .map(|w| {
            Item::new(
                "workflow",
                w.name.as_str(),
                Act::RunWorkflow {
                    id: w.id.clone(),
                    source: source.map(str::to_owned),
                },
            )
        })
        .collect()
}

/// Whether a workflow's manual trigger asks for a session.
pub fn contextual(nodes: &serde_json::Value) -> bool {
    nodes.as_array().into_iter().flatten().any(|n| {
        let cfg = n.pointer("/config");
        n.get("type").and_then(|t| t.as_str()) == Some("trigger")
            && cfg
                .and_then(|c| c.get("triggerType"))
                .and_then(|t| t.as_str())
                == Some("manual")
            && cfg
                .and_then(|c| c.get("contextual"))
                .and_then(serde_json::Value::as_bool)
                == Some(true)
    })
}

fn scoped(s: &Screen<'_>, shell: bool) -> Vec<Item> {
    let mut out = Vec::new();
    for p in &s.store.projects {
        let wts = s.st.worktrees.get(&p.path).cloned().unwrap_or_default();
        let count = |path: &str| {
            s.store
                .sessions
                .iter()
                .filter(|x| {
                    x.info
                        .worktree_path
                        .as_deref()
                        .unwrap_or(&x.info.project_path)
                        == path
                })
                .count()
        };
        let new = |worktree: Option<String>| {
            if shell {
                Act::NewShellIn(worktree.unwrap_or_else(|| p.path.clone()))
            } else {
                Act::NewSessionIn {
                    project: p.name.clone(),
                    worktree,
                    agent: None,
                }
            }
        };
        if wts.is_empty() {
            let mut it = Item::new("folder", p.name.as_str(), new(None)).tint(look::GRAY_400);
            it.separator = !out.is_empty();
            out.push(it);
            continue;
        }
        let mut h = Item::header(p.name.as_str());
        h.separator = !out.is_empty();
        out.push(h);
        for wt in &wts {
            let n = count(&wt.path);
            let detail = format!("{n} session{}", if n == 1 { "" } else { "s" });
            let (icon, label, tint) = if wt.main {
                ("git-branch", wt.branch.as_str(), look::GRAY_400)
            } else {
                ("folder-git-2", wt.name.as_str(), look::AMBER_400)
            };
            out.push(
                Item::new(icon, label, new((!wt.main).then(|| wt.path.clone())))
                    .tint(tint)
                    .detail(detail),
            );
        }
        if !shell {
            // An empty worktree path asks for a new worktree.
            out.push(Item::new(
                "plus",
                "New worktree",
                Act::NewSessionIn {
                    project: p.name.clone(),
                    worktree: Some(String::new()),
                    agent: None,
                },
            ));
        }
    }
    out
}

fn grid(s: &Screen<'_>) -> Vec<Item> {
    let mut items = Vec::new();
    if !s.store.projects.is_empty() {
        items.push(
            Item::new("bot", "New session in\u{2026}", Act::Nothing)
                .tint(look::GRAY_400)
                .submenu(scoped(s, false)),
        );
        items.push(
            Item::new("terminal", "New terminal in\u{2026}", Act::Nothing)
                .tint(look::GRAY_400)
                .submenu(scoped(s, true)),
        );
    }
    let mut new = Item::new("plus", "New session...", Act::NewSession);
    new.shortcut = Some(format!("{}N", super::MOD_LABEL));
    new.separator = !items.is_empty();
    items.push(new);
    let flows = workflows(s, None);
    if !flows.is_empty() {
        items.push(Item::new("workflow", "Run workflow", Act::Nothing).submenu(flows));
    }
    items
}

fn card(s: &Screen<'_>, id: &str) -> Vec<Item> {
    let Some(session) = s.store.session(id) else {
        return Vec::new();
    };
    let info = &session.info;
    let worktree = info
        .worktree_path
        .clone()
        .filter(|_| info.is_worktree == Some(true));
    let mut items = vec![
        Item::new(
            "bot",
            "New session",
            Act::NewSessionIn {
                project: info.project_name.clone(),
                worktree: worktree.clone(),
                agent: None,
            },
        )
        .tint(look::GRAY_400),
        Item::new(
            "terminal",
            "New terminal",
            Act::NewShellIn(worktree.unwrap_or_else(|| info.project_path.clone())),
        ),
    ];
    let agents: Vec<Item> = installed(s)
        .map(|(k, name)| {
            Item::new(
                "bot",
                *name,
                Act::NewSessionIn {
                    project: info.project_name.clone(),
                    worktree: info
                        .worktree_path
                        .clone()
                        .filter(|_| info.is_worktree == Some(true)),
                    agent: Some((*k).to_owned()),
                },
            )
            .tint(look::GRAY_400)
        })
        .collect();
    if agents.len() > 1 {
        items.push(Item::new("bot", "New session with\u{2026}", Act::Nothing).submenu(agents));
    }
    let flows = workflows(s, Some(id));
    if !flows.is_empty() {
        items.push(Item::new("workflow", "Run workflow", Act::Nothing).submenu(flows));
    }
    items
}

fn terminal(s: &Screen<'_>, id: &str) -> Vec<Item> {
    let selection = false;
    let mut items = vec![
        Item::new("copy", "Copy", Act::Nothing).disabled(!selection),
        Item::new("clipboard-paste", "Paste", Act::Paste(id.to_owned())),
    ];
    let locked = s.st.locked.contains(id);
    items.push(
        Item::new(
            "maximize-2",
            "Fit to this device",
            Act::FitToDevice(id.to_owned()),
        )
        .sep(),
    );
    items.push(Item::new(
        if locked { "lock-open" } else { "lock" },
        if locked { "Unlock size" } else { "Lock size" },
        Act::LockSize(id.to_owned(), !locked),
    ));
    let flows = workflows(s, Some(id));
    if !flows.is_empty() {
        items.push(
            Item::new("workflow", "Run workflow", Act::Nothing)
                .sep()
                .submenu(flows),
        );
    }
    items
}

fn view_options(s: &Screen<'_>) -> Vec<Item> {
    const DOTS: [Rgb; 5] = [
        look::GRAY_400,
        look::GREEN_500,
        look::YELLOW_500,
        look::GRAY_500,
        look::RED_500,
    ];
    let mut items = vec![Item::header("Status")];
    for (f, dot) in Filter::ALL.into_iter().zip(DOTS) {
        let mut it = Item::option(f.label(), s.st.filter == f, Act::Filter(f));
        it.dot = Some(dot);
        items.push(it);
    }
    items.push(Item::header("Sort").sep());
    for o in Sort::ALL {
        items.push(Item::option(o.label(), s.st.sort == o, Act::Sort(o)));
    }
    items.push(Item::header("Layout").sep());
    let seg = |mode: LayoutMode, label: &str, icon: &str| {
        let on = s.st.layout == mode;
        let id = s.acts.on("layout", Act::Layout(mode));
        let hot = s.ui.hover(id);
        let fg = if on {
            c(look::WHITE)
        } else {
            c(look::GRAY_500).mix(c(look::GRAY_300), hot)
        };
        div()
            .id(id)
            .on(Action::Click)
            .row()
            .items_center()
            .gap(6.0)
            .px(10.0)
            .py(4.0)
            .rounded(6.0)
            .bg(if on { white(0.10) } else { white(0.04) })
            .role(Role::Button, label)
            .selected(on)
            .child(ico(icon, 12.0, 2.0, fg))
            .child(text(label, 11.0, fg).line_height(14.0).silent())
    };
    items.push(Item {
        row: Some(
            div()
                .row()
                .gap(4.0)
                .px(12.0)
                .py(4.0)
                .child(seg(LayoutMode::Grid, "Grid", "grid-4"))
                .child(seg(LayoutMode::Tabs, "Tabs", "tab-window")),
        ),
        ..Item::default()
    });
    if s.st.layout == LayoutMode::Grid {
        items.push(Item::header("Columns").sep());
        for (n, label) in [
            (0, "Auto"),
            (1, "1 Column"),
            (2, "2 Columns"),
            (3, "3 Columns"),
            (4, "4 Columns"),
            (-1, "Flexible"),
        ] {
            items.push(Item::option(label, s.st.columns == n, Act::Columns(n)));
        }
    }
    items
}

fn project(s: &Screen<'_>, name: &str) -> Vec<Item> {
    let confirm = s.st.confirm_delete.as_deref() == Some(name);
    let mut del = if confirm {
        Item::new(
            "trash-2",
            "Confirm delete?",
            Act::DeleteProject(name.to_owned()),
        )
    } else {
        Item::new(
            "trash-2",
            "Delete Project",
            Act::AskDeleteProject(name.to_owned()),
        )
    };
    del.danger = true;
    vec![
        Item::new("pencil", "Edit Project", Act::EditProject(name.to_owned())),
        del,
    ]
}

fn launch_projects(s: &Screen<'_>) -> Vec<Item> {
    if s.store.projects.is_empty() {
        return vec![Item::header("No projects")];
    }
    s.store
        .projects
        .iter()
        .map(|p| {
            let mut it = Item::new("folder", p.name.as_str(), Act::PickProject(p.name.clone()))
                .tint(look::GRAY_400);
            it.checked = s.st.composer.project.as_deref() == Some(p.name.as_str());
            it.detail = Some(super::truncate(&p.path, 28));
            it
        })
        .collect()
}

fn launch_agents(s: &Screen<'_>) -> Vec<Item> {
    let current =
        s.st.launch
            .agent
            .as_deref()
            .unwrap_or(s.store.default_agent());
    AGENTS
        .iter()
        .map(|(k, name)| {
            let ok =
                s.st.installed
                    .as_ref()
                    .is_none_or(|i| i.iter().any(|a| a == k));
            let mut it = Item::new("bot", *name, Act::PickAgent((*k).to_owned()))
                .tint(look::GRAY_400)
                .disabled(!ok);
            it.checked = current == *k;
            if !ok {
                it.detail = Some("Not installed".to_owned());
            }
            it
        })
        .collect()
}

fn launch_worktrees(s: &Screen<'_>) -> Vec<Item> {
    let l = &s.st.launch;
    let mut root = Item::option(
        "Project root",
        l.worktree.is_none() && !l.new_worktree,
        Act::PickWorktree(None),
    );
    root.icon = None;
    let mut new =
        Item::new("folder-git-2", "New worktree", Act::NewWorktree(true)).tint(look::AMBER_400);
    new.checked = l.new_worktree;
    let mut items = vec![root, new];
    let path =
        s.st.composer
            .project
            .as_deref()
            .and_then(|n| s.store.projects.iter().find(|p| p.name == n))
            .map(|p| p.path.clone())
            .unwrap_or_default();
    let mut first = true;
    for wt in
        s.st.worktrees
            .get(&path)
            .into_iter()
            .flatten()
            .filter(|w| !w.main)
    {
        let n = s
            .store
            .sessions
            .iter()
            .filter(|x| x.info.worktree_path.as_deref() == Some(wt.path.as_str()))
            .count();
        let mut it = Item::new(
            "folder-git-2",
            wt.name.as_str(),
            Act::PickWorktree(Some(wt.path.clone())),
        )
        .detail(if n == 0 {
            format!("{} \u{b7} idle", wt.branch)
        } else {
            format!(
                "{} \u{b7} {n} session{}",
                wt.branch,
                if n == 1 { "" } else { "s" }
            )
        });
        it.mono = true;
        it.checked = l.worktree.as_deref() == Some(wt.path.as_str());
        it.separator = first;
        first = false;
        items.push(it);
    }
    items
}

fn launch_branches(s: &Screen<'_>) -> Vec<Item> {
    let path =
        s.st.composer
            .project
            .as_deref()
            .and_then(|n| s.store.projects.iter().find(|p| p.name == n))
            .map(|p| p.path.clone())
            .unwrap_or_default();
    let Some(branches) = s.st.branches.get(&path) else {
        return vec![Item::header("Loading...")];
    };
    if branches.is_empty() {
        return vec![Item::header("No branches found")];
    }
    branches
        .iter()
        .map(|b| {
            let mut it = Item::new("git-branch", b.as_str(), Act::PickBranch(Some(b.clone())));
            it.checked = s.st.launch.branch.as_deref() == Some(b.as_str());
            it
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn contextual_workflows_have_a_manual_contextual_trigger() {
        let yes = json!([{ "type": "trigger", "config": { "triggerType": "manual", "contextual": true } }]);
        let no = json!([{ "type": "trigger", "config": { "triggerType": "manual" } }]);
        assert!(contextual(&yes));
        assert!(!contextual(&no));
        assert!(!contextual(&json!(null)));
    }

    #[test]
    fn choosing_skips_headers_and_reaches_submenus() {
        let items = || {
            vec![
                Item::header("Status"),
                Item::option("All", true, Act::Nothing),
                Item::new("bot", "More", Act::Nothing).submenu(vec![Item::option(
                    "x",
                    false,
                    Act::Collapse,
                )]),
            ]
        };
        assert!(chosen(items(), None, MENU, 0).is_none());
        assert_eq!(
            chosen(items(), None, MENU, 1).map(|i| i.label),
            Some("All".to_owned())
        );
        let sub = chosen(items(), Some(2), SUB, 0).and_then(|i| i.act);
        assert_eq!(sub, Some(Act::Collapse));
    }
}

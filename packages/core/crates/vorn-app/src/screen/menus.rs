//! The app's menus (`GridContextMenu`, `CardContextMenu`,
//! `TerminalContextMenu`, the toolbar's popovers and the launcher's
//! pickers): their entries as data, and one panel that draws any of them
//! at a point, with a submenu beside the hovered entry.

use super::{c, ico, white, Screen};
use crate::act::Act;
use crate::look::{self, Rgb};
use crate::state::{Filter, LayoutMode, MenuKind, SidebarMode, Sort};
use crate::ui::menu::Inset;
use crate::ui::{div, text, Action, El, Id, Role, Ui};

/// The open menu's choice group; entries report `Select` in it.
pub const MENU: Id = Id(0x6d65_6e75_0000_0001);
/// The open submenu's choice group.
pub const SUB: Id = Id(0x6d65_6e75_0000_0003);

/// One entry of a menu, its choice an [`Act`].
pub type Item = crate::ui::menu::Item<Act>;

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
        MenuKind::Group(_) | MenuKind::SessionGroups(_) => 176.0,
        MenuKind::GroupIcon(_) => 248.0,
        MenuKind::LaunchAgent => 160.0,
        MenuKind::LaunchModel => 288.0,
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

/// A panel of `items` in the open menu's group, its submenu in [`SUB`].
pub fn panel(ui: &Ui, group: Id, w: f32, name: &str, items: Vec<Item>, sub: Option<usize>) -> El {
    crate::ui::menu::panel(ui, MENU, group, SUB, w, name, items, sub)
}

/// The entry `index` of a menu's items, or of the open submenu's.
pub fn chosen(items: Vec<Item>, sub: Option<usize>, group: Id, index: usize) -> Option<Item> {
    crate::ui::menu::chosen(items, sub, group == SUB, index)
}

/// The entries of the menu `kind`, from what the screen shows.
pub fn items(s: &Screen<'_>, kind: &MenuKind) -> Vec<Item> {
    let mut items = kind_items(s, kind);
    // These menus are `OptionRow`s under small uppercase section titles.
    if matches!(
        kind,
        MenuKind::ViewOptions
            | MenuKind::TaskOptions
            | MenuKind::WorkflowFilter
            | MenuKind::SidebarOptions
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
        MenuKind::SidebarOptions => sidebar_options(s),
        MenuKind::Group(id) => group(s, id),
        MenuKind::SessionGroups(id) => session_groups(s, id),
        MenuKind::GroupIcon(_) => Vec::new(),
        MenuKind::LaunchProject => launch_projects(s),
        MenuKind::LaunchAgent => launch_agents(s),
        MenuKind::LaunchWorktree => launch_worktrees(s),
        MenuKind::LaunchBranch => launch_branches(s),
        MenuKind::Recent | MenuKind::Headless | MenuKind::WfIcon | MenuKind::SetShell => Vec::new(),
        MenuKind::SetSelect(key) => super::settings::select_items(s, key),
        MenuKind::SshKey(host) => super::settings::ssh_key_items(s, host),
        MenuKind::FileTabs(owner) => super::side::tab_items(s, owner),
        MenuKind::Branch(_) | MenuKind::OpenIn(_) | MenuKind::LaunchModel => Vec::new(),
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

/// The launcher's name for an agent (`AGENT_LIST`'s `displayName`).
pub fn launch_name(agent: &str) -> &str {
    match agent {
        "codex" => "Codex CLI",
        "gemini" => "Gemini CLI",
        _ => agent_name(agent),
    }
}

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

/// A delete entry that arms on the first choice: "Confirm delete?" on the
/// danger wash until chosen again.
fn delete_item(armed: bool, label: &str, ask: Act, delete: Act) -> Item {
    let mut del = if armed {
        Item::new("trash-2", "Confirm delete?", delete)
    } else {
        Item::new("trash-2", label, ask)
    };
    del.danger = true;
    del.checked = armed;
    del.side(Inset::MENU)
}

/// `ProjectContextMenu`.
fn project(s: &Screen<'_>, name: &str) -> Vec<Item> {
    let armed = s.st.confirm_delete.as_deref() == Some(name);
    vec![
        Item::new("pencil", "Edit Project", Act::EditProject(name.to_owned())).side(Inset::MENU),
        delete_item(
            armed,
            "Delete Project",
            Act::AskDeleteProject(name.to_owned()),
            Act::DeleteProject(name.to_owned()),
        ),
    ]
}

/// `ProjectsSectionToolbar`'s Show section; the worktree levels are not
/// drawn by this sidebar, so their modes are not offered.
fn sidebar_options(s: &Screen<'_>) -> Vec<Item> {
    let mut items = vec![Item::header("Show")];
    for m in SidebarMode::ALL {
        items.push(Item::option(
            m.label(),
            s.st.sidebar_mode == m,
            Act::SidebarMode(m),
        ));
    }
    items
}

/// `GroupContextMenu`.
fn group(s: &Screen<'_>, id: &str) -> Vec<Item> {
    let armed = s.st.confirm_delete.as_deref() == Some(id);
    let id = id.to_owned();
    vec![
        Item::new("pencil", "Rename group", Act::RenameGroup(id.clone())).side(Inset::MENU),
        Item::new("palette", "Change icon", Act::GroupIcon(id.clone())).side(Inset::MENU),
        Item::new(
            "ungroup",
            "Remove all sessions",
            Act::UngroupAll(id.clone()),
        )
        .side(Inset::MENU),
        delete_item(
            armed,
            "Delete group",
            Act::AskDeleteGroup(id.clone()),
            Act::DeleteGroup(id),
        ),
    ]
}

/// `SessionContextMenu`: the groups a session can move to, choosing its
/// own taking it out.
fn session_groups(s: &Screen<'_>, sid: &str) -> Vec<Item> {
    let current = s.store.session(sid).and_then(|x| x.info.group_id.clone());
    let mut items = vec![Item::header("Move to group").side(Inset::MENU)];
    for g in &s.store.groups {
        let on = current.as_deref() == Some(g.id.as_str());
        let mut it = Item::option(
            g.name.as_str(),
            on,
            Act::MoveToGroup {
                session: sid.to_owned(),
                group: (!on).then(|| g.id.clone()),
            },
        )
        .side(Inset::NESTED)
        .mark(super::sidebar::stored_icon(
            g.icon.as_deref(),
            g.icon_color.as_deref(),
            12.0,
        ));
        it.checked = on;
        items.push(it);
    }
    let mut new = Item::new(
        "plus",
        "New group\u{2026}",
        Act::NewGroup(Some(sid.to_owned())),
    )
    .side(Inset::NESTED);
    new.icon = Some(("plus", look::GRAY_300));
    items.push(new);
    if current.is_some() {
        items.push(
            Item::option(
                "Remove from group",
                false,
                Act::MoveToGroup {
                    session: sid.to_owned(),
                    group: None,
                },
            )
            .side(Inset::NESTED),
        );
    }
    items
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
        .map(|(k, _)| {
            let ok =
                s.st.installed
                    .as_ref()
                    .is_none_or(|i| i.iter().any(|a| a == k));
            let mut it = Item::option(
                launch_name(k),
                current == *k,
                Act::PickAgent((*k).to_owned()),
            )
            .plain()
            .mark(super::agent_icon(s, k, 14.0))
            .disabled(!ok);
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

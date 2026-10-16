//! The command palette (`CommandPalette`): every command the app offers,
//! by category, matched against what is typed. The commands are rebuilt
//! from the store each frame, so the list is never stale.

use serde_json::Value;

use crate::act::Act;
use crate::client::Store;
use crate::screen::MainView;
use crate::state::{basename, now_ms, time_ago, Filter, LayoutMode, Sort, State};
use crate::tasks::TaskAct;
use crate::workflows::WfAct;

/// What the MCP server answers on, as the renderer copies it.
pub const MCP_URL: &str = "http://localhost:56433/mcp";

/// What a palette control does.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub enum PalAct {
    /// Runs the command at this index of the shown list.
    Execute(usize),
}

/// The palette's own state: what is typed and which row Enter runs.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Palette {
    pub query: String,
    pub active: usize,
}

impl Palette {
    /// Moves the active row by one, wrapping as the renderer does.
    pub fn step(&mut self, n: usize, forward: bool) {
        let n = n.max(1);
        self.active = if forward {
            (self.active + 1) % n
        } else {
            (self.active + n - 1) % n
        };
    }
}

/// `CATEGORY_ORDER`, with `CATEGORY_LABELS` as the headers.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum Cat {
    Actions,
    Settings,
    Terminals,
    Recent,
    Projects,
    Workflows,
    Layouts,
    QuickLaunch,
    Filter,
}

impl Cat {
    pub fn label(self) -> &'static str {
        match self {
            Cat::Actions => "Actions",
            Cat::Settings => "Settings",
            Cat::Terminals => "Terminals",
            Cat::Recent => "Recent Sessions",
            Cat::Projects => "Projects",
            Cat::Workflows => "Workflows",
            Cat::Layouts => "Layouts",
            Cat::QuickLaunch => "Quick Launch",
            Cat::Filter => "Filter & Sort",
        }
    }
}

/// A row's leading glyph.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Glyph {
    Icon(&'static str),
    /// An extension's icon, by its lucide name.
    Named(String),
    Agent(String),
    None,
}

/// One command.
#[derive(Debug, Clone, PartialEq)]
pub struct Entry {
    pub cat: Cat,
    pub label: String,
    pub sublabel: Option<String>,
    pub glyph: Glyph,
    pub shortcut: Option<String>,
    pub keywords: Vec<String>,
    pub act: Act,
}

impl Entry {
    fn new(cat: Cat, label: impl Into<String>, glyph: Glyph, act: Act) -> Entry {
        Entry {
            cat,
            label: label.into(),
            sublabel: None,
            glyph,
            shortcut: None,
            keywords: Vec::new(),
            act,
        }
    }

    fn keys(mut self, words: &[&str]) -> Entry {
        self.keywords.extend(words.iter().map(|w| (*w).to_owned()));
        self
    }

    fn sub(mut self, text: impl Into<String>) -> Entry {
        self.sublabel = Some(text.into());
        self
    }

    fn shortcut(mut self, keys: impl Into<String>) -> Entry {
        self.shortcut = Some(keys.into());
        self
    }
}

/// `scoreMatch`: an exact label, a prefix, a word's prefix, anywhere in
/// the label, then the keywords; 0 is no match.
pub fn score(query: &str, cmd: &Entry) -> u32 {
    let q = query.to_lowercase();
    let label = cmd.label.to_lowercase();
    if label == q {
        100
    } else if label.starts_with(&q) {
        80
    } else if label.split_whitespace().any(|w| w.starts_with(&q)) {
        60
    } else if label.contains(&q) {
        40
    } else if cmd.keywords.join(" ").to_lowercase().contains(&q) {
        20
    } else {
        0
    }
}

/// The rows the palette shows, in order: by category with nothing typed
/// (Quick Launch left out), else every match, best first.
pub fn shown(commands: Vec<Entry>, query: &str) -> Vec<Entry> {
    // Inside "Open Layout ›" only the layouts are listed, matched on the rest of what is typed.
    let (commands, query) = match query.strip_prefix(LAYOUT_MENU) {
        Some(rest) => (
            commands
                .into_iter()
                .filter(|c| c.cat == Cat::Layouts)
                .collect(),
            rest,
        ),
        None => (commands, query),
    };
    let q = query.trim();
    if q.is_empty() {
        let in_menu = commands.iter().all(|c| c.cat == Cat::Layouts) && !commands.is_empty();
        let mut list: Vec<Entry> = commands
            .into_iter()
            .filter(|c| in_menu || !matches!(c.cat, Cat::QuickLaunch | Cat::Layouts))
            .collect();
        list.sort_by_key(|c| c.cat);
        return list;
    }
    let mut scored: Vec<(u32, Entry)> = commands
        .into_iter()
        .map(|c| (score(q, &c), c))
        .filter(|(s, _)| *s > 0)
        .collect();
    scored.sort_by_key(|(s, _)| std::cmp::Reverse(*s));
    scored.into_iter().map(|(_, c)| c).collect()
}

/// `getProjectHostIds(p).includes('local')`: a project names its hosts,
/// else it is this machine's.
fn is_local(hosts: Option<&Value>) -> bool {
    match hosts.and_then(Value::as_array) {
        Some(ids) if !ids.is_empty() => ids.iter().any(|h| h.as_str() == Some("local")),
        _ => true,
    }
}

/// Extensions and Connections, Install and where from, and each built-in's switch.
fn extensions(st: &State) -> Vec<Entry> {
    use crate::plugins::{Origin, PlugAct};
    use crate::settings::{Category, SetAct};
    let mut out = vec![
        Entry::new(
            Cat::Settings,
            "Open Extensions",
            Glyph::Icon("puzzle"),
            Act::Set(SetAct::Open(Category::Connectors)),
        )
        .keys(&["plugins", "connectors", "extensions", "add-ons"]),
        Entry::new(
            Cat::Settings,
            "Open Connections",
            Glyph::Icon("plug"),
            Act::Set(SetAct::Open(Category::Connections)),
        )
        .keys(&["connectors", "accounts", "sign-in", "extensions"]),
        Entry::new(
            Cat::Settings,
            "Install Extension\u{2026}",
            Glyph::Icon("plus"),
            Act::Plug(PlugAct::InstallMenu),
        )
        .keys(&["extensions", "plugins", "add", "git", "folder", "link"]),
    ];
    for (origin, title, keys) in [
        (
            Origin::Git,
            "Install Extension from Git\u{2026}",
            &["extensions", "plugins", "repository", "clone"][..],
        ),
        (
            Origin::Folder,
            "Install Extension from a Folder\u{2026}",
            &["extensions", "plugins", "directory", "copy", "local"][..],
        ),
        (
            Origin::Link,
            "Link an Extension Folder\u{2026}",
            &[
                "extensions",
                "plugins",
                "directory",
                "develop",
                "live",
                "local",
            ][..],
        ),
    ] {
        out.push(
            Entry::new(
                Cat::Settings,
                title,
                Glyph::Icon(origin.icon()),
                Act::Plug(PlugAct::InstallFrom(origin)),
            )
            .keys(keys),
        );
    }
    for b in &st.points.builtins {
        let (Some(id), Some(name)) = (
            b.get("id").and_then(Value::as_str),
            b.get("name").and_then(Value::as_str),
        ) else {
            continue;
        };
        let on = b.get("on").and_then(Value::as_bool) == Some(true);
        let verb = if on { "Off" } else { "On" };
        out.push(
            Entry::new(
                Cat::Actions,
                format!("Turn {name} {verb}"),
                Glyph::Icon(if on { "toggle-right" } else { "toggle-left" }),
                Act::Point(crate::points::PointAct::Builtin {
                    id: id.to_owned(),
                    on: !on,
                }),
            )
            .sub("Built-in extension")
            .keys(&["extension", "enable", "disable", name]),
        );
    }
    out
}

/// `useCommands`: every command, in the renderer's order.
/// `m` and `alt` are the modifier labels; the status filters show the
/// keys that really pick them (Alt), not the renderer's ⌘ digits, which
/// jump to a card.
pub fn commands(store: &Store, st: &State, m: &str, alt: &str) -> Vec<Entry> {
    let mut out = actions(store, m);
    for p in st.points.at(crate::points::kind::COMMANDS) {
        let mut e = Entry::new(
            Cat::Actions,
            &p.title,
            p.icon.clone().map_or(Glyph::Icon("puzzle"), Glyph::Named),
            Act::Point(crate::points::PointAct::Choose {
                key: p.key(),
                args: "{}".to_owned(),
            }),
        )
        .sub(&p.extension_name)
        .keys(&["extension", &p.extension_name]);
        if let Some(k) = &p.key {
            e = e.shortcut(k.replace("Mod+", m).replace('+', ""));
        }
        out.push(e);
    }
    out.extend(extensions(st));
    for p in st.points.at(crate::points::kind::SETTINGS) {
        out.push(
            Entry::new(
                Cat::Actions,
                format!("{} Settings", p.title),
                p.icon.clone().map_or(Glyph::Icon("settings"), Glyph::Named),
                Act::Point(crate::points::PointAct::Settings(p.key())),
            )
            .sub(&p.extension_name)
            .keys(&["settings", "extension", &p.extension_name]),
        );
    }
    // The web client already runs in the browser.
    if cfg!(target_arch = "wasm32") {
        out.retain(|e| e.act != Act::OpenWebClient);
    }
    for s in &store.sessions {
        let kind = if s.is_shell() {
            "Shell"
        } else {
            store.agents.label(s.info.agent_type.as_str())
        };
        out.push(
            Entry::new(
                Cat::Terminals,
                s.title(),
                Glyph::Agent(s.info.agent_type.as_str().to_owned()),
                Act::Expand(s.info.id.clone()),
            )
            .keys(&[kind, &s.info.project_name, s.status()]),
        );
    }
    let now = now_ms();
    for (i, r) in st.recent.iter().enumerate() {
        if !r.can_resume {
            continue;
        }
        let project = store
            .projects
            .iter()
            .find(|p| p.path == r.project_path)
            .map_or_else(|| basename(&r.project_path).to_owned(), |p| p.name.clone());
        let label = if r.display.is_empty() {
            "Untitled session"
        } else {
            r.display.as_str()
        };
        let sub = format!(
            "{project} \u{b7} {} \u{b7} {}",
            time_ago(r.timestamp, now),
            r.activity_text()
        );
        out.push(
            Entry::new(
                Cat::Recent,
                label,
                Glyph::Agent(r.agent.clone()),
                Act::ResumeRecent(i),
            )
            .sub(sub)
            .keys(&["resume", "recent", "history", &project, &r.agent]),
        );
    }
    out.push(
        Entry::new(
            Cat::Projects,
            "All Projects",
            Glyph::Icon("monitor"),
            Act::Project(None),
        )
        .keys(&["show all", "clear filter"]),
    );
    for p in &store.projects {
        out.push(
            Entry::new(
                Cat::Projects,
                &p.name,
                Glyph::None,
                Act::Project(Some(p.name.clone())),
            )
            .keys(&[&p.path]),
        );
    }
    for wf in &store.workflows {
        let projects: Vec<&str> = wf
            .nodes
            .as_array()
            .into_iter()
            .flatten()
            .filter(|n| n.get("type").and_then(Value::as_str) == Some("launchAgent"))
            .filter_map(|n| n.pointer("/config/projectName").and_then(Value::as_str))
            .filter(|s| !s.is_empty())
            .collect();
        out.push(
            Entry::new(
                Cat::Workflows,
                &wf.name,
                Glyph::Icon("workflow"),
                Act::RunWorkflow {
                    id: wf.id.clone(),
                    source: None,
                },
            )
            .keys(&projects),
        );
    }
    layouts(st, &mut out);
    quick_launch(store, st, &mut out);
    filters(alt, &mut out);
    out
}

/// The text the palette narrows to for "Open Layout ›".
pub const LAYOUT_MENU: &str = "Open Layout \u{203a} ";

/// "Open Layout ›", which lists the layouts, and a row for each.
fn layouts(st: &State, out: &mut Vec<Entry>) {
    if st.layouts.is_empty() {
        return;
    }
    out.push(
        Entry::new(
            Cat::Actions,
            "Open Layout \u{203a}",
            Glyph::Icon("layout-grid"),
            Act::OpenLayoutMenu,
        )
        .keys(&["layouts", "arrangement", "sessions", "workspace"]),
    );
    for l in &st.layouts {
        let sessions = if l.sessions == 1 {
            "1 session".to_owned()
        } else {
            format!("{} sessions", l.sessions)
        };
        let sub = if l.source == "user" {
            sessions
        } else {
            format!("{sessions} \u{b7} {}", l.source)
        };
        out.push(
            Entry::new(
                Cat::Layouts,
                format!("{LAYOUT_MENU}{}", l.name),
                Glyph::Icon("layout-grid"),
                Act::OpenLayout {
                    name: l.name.clone(),
                    source: l.source.clone(),
                },
            )
            .sub(sub)
            .keys(&["layout", "open"]),
        );
    }
}

fn actions(store: &Store, m: &str) -> Vec<Entry> {
    let running = store.sessions.len();
    let mut stop = Entry::new(
        Cat::Actions,
        "Stop Sessions and Server",
        Glyph::Icon("power"),
        Act::StopServer,
    )
    .keys(&["stop", "quit", "shutdown", "server", "end", "kill"]);
    if running > 0 {
        stop = stop.sub(format!("{running} running"));
    }
    let next_layout = match LayoutMode::from_config(&store.config) {
        LayoutMode::Grid => LayoutMode::Tabs,
        LayoutMode::Tabs => LayoutMode::Grid,
    };
    vec![
        Entry::new(
            Cat::Actions,
            "New Session",
            Glyph::Icon("plus"),
            Act::NewSession,
        )
        .shortcut(format!("{m}N"))
        .keys(&["create", "add", "agent", "terminal", "launch"]),
        Entry::new(
            Cat::Actions,
            "Open Settings",
            Glyph::Icon("settings"),
            Act::Settings,
        )
        .shortcut(format!("{m},"))
        .keys(&["preferences", "config", "options"]),
        Entry::new(
            Cat::Actions,
            "Open in Browser",
            Glyph::Icon("globe"),
            Act::OpenWebClient,
        )
        .keys(&["web", "browser", "url", "remote", "phone", "app"]),
        Entry::new(
            Cat::Actions,
            "Toggle Sidebar",
            Glyph::Icon("panel-left"),
            Act::ToggleSidebar,
        )
        .shortcut(format!("{m}B"))
        .keys(&["hide", "show", "panel"]),
        Entry::new(
            Cat::Actions,
            "Keyboard Shortcuts",
            Glyph::Icon("keyboard"),
            Act::Shortcuts,
        )
        .shortcut(format!("{m}?"))
        .keys(&[
            "shortcuts",
            "keys",
            "help",
            "reference",
            "bindings",
            "hotkeys",
        ]),
        Entry::new(
            Cat::Actions,
            "Add Project",
            Glyph::Icon("folder-plus"),
            Act::AddProject,
        )
        .keys(&["new project", "create project"]),
        Entry::new(
            Cat::Actions,
            "New Group",
            Glyph::Icon("group"),
            Act::AddGroup,
        )
        .keys(&["new group", "create group", "sidebar"]),
        Entry::new(
            Cat::Actions,
            "Add Workflow",
            Glyph::Icon("workflow"),
            Act::Workflow(WfAct::New),
        )
        .keys(&["new workflow", "create workflow", "schedule"]),
        Entry::new(
            Cat::Actions,
            "Open Tasks View",
            Glyph::Icon("list-todo"),
            Act::View(MainView::Tasks),
        )
        .keys(&["task", "kanban", "queue", "todo", "board"]),
        Entry::new(
            Cat::Actions,
            "Create Task",
            Glyph::Icon("plus"),
            Act::Task(TaskAct::New("todo".into())),
        )
        .keys(&["new task", "add task", "todo"]),
        stop,
        Entry::new(
            Cat::Actions,
            "Toggle Layout (Grid/Tabs)",
            Glyph::Icon("layout-dashboard"),
            Act::Layout(next_layout),
        )
        .keys(&["grid", "tabs", "layout", "switch", "view"]),
        Entry::new(
            Cat::Actions,
            "Show Welcome Guide",
            Glyph::Icon("book-open"),
            Act::Welcome,
        )
        .keys(&["welcome", "guide", "help", "onboarding", "tour"]),
        Entry::new(
            Cat::Actions,
            "New Terminal Session",
            Glyph::Icon("terminal"),
            Act::NewShellIn(String::new()),
        )
        .shortcut("Ctrl+`")
        .keys(&["shell", "terminal", "zsh", "bash"]),
        Entry::new(
            Cat::Actions,
            "Copy MCP Server URL",
            Glyph::Icon("plug"),
            Act::Copy {
                text: MCP_URL.to_owned(),
                toast: String::new(),
            },
        )
        .keys(&["mcp", "api", "integration", "cursor", "server", "url"])
        .keys(
            &store
                .agents
                .iter()
                .map(|a| a.key.as_str())
                .collect::<Vec<_>>(),
        ),
        Entry::new(
            Cat::Actions,
            "SSH & Hosts",
            Glyph::Icon("server"),
            Act::Set(crate::settings::SetAct::Open(
                crate::settings::Category::Ssh,
            )),
        )
        .keys(&[
            "ssh",
            "remote",
            "host",
            "server",
            "key",
            "credential",
            "vault",
        ]),
    ]
}

/// Each installed agent on each local project, its worktrees and a new
/// one; then a shell in each.
fn quick_launch(store: &Store, st: &State, out: &mut Vec<Entry>) {
    let local: Vec<_> = store
        .projects
        .iter()
        .filter(|p| is_local(p.host_ids.as_ref()))
        .collect();
    // A project is a repository when vornd listed worktrees for it.
    let worktrees = |path: &str| {
        st.worktrees
            .get(path)
            .filter(|w| !w.is_empty())
            .map(|w| w.iter().filter(|w| !w.main).collect::<Vec<_>>())
    };
    let wt_sub = |w: &crate::state::Worktree| {
        if w.branch == w.name {
            "Existing worktree".to_owned()
        } else {
            format!("Branch: {}", w.branch)
        }
    };
    let installed = store
        .agents
        .iter()
        .filter(|a| st.installed.as_ref().is_some_and(|i| i.contains(&a.key)));
    for a in installed {
        let (agent, name) = (a.key.as_str(), a.label.as_str());
        for p in &local {
            let launch = |worktree: Option<String>| Act::NewSessionIn {
                project: p.name.clone(),
                worktree,
                agent: Some(agent.to_owned()),
            };
            let glyph = Glyph::Agent(agent.to_owned());
            out.push(
                Entry::new(
                    Cat::QuickLaunch,
                    format!("{name} on {}", p.name),
                    glyph.clone(),
                    launch(None),
                )
                .keys(&["launch", "start", "run"]),
            );
            let Some(wts) = worktrees(&p.path) else {
                continue;
            };
            for w in wts {
                out.push(
                    Entry::new(
                        Cat::QuickLaunch,
                        format!("{name} on {} \u{203a} {}", p.name, w.name),
                        glyph.clone(),
                        launch(Some(w.path.clone())),
                    )
                    .sub(wt_sub(w))
                    .keys(&[
                        "launch", "start", "run", "worktree", "branch", &w.name, &w.branch,
                    ]),
                );
            }
            out.push(
                Entry::new(
                    Cat::QuickLaunch,
                    format!("{name} on {} (new worktree)", p.name),
                    glyph,
                    launch(Some(String::new())),
                )
                .sub("Isolated worktree from current branch")
                .keys(&[
                    "launch", "start", "run", "worktree", "branch", "isolated", "fork", "new",
                ]),
            );
        }
    }
    for p in &local {
        out.push(
            Entry::new(
                Cat::QuickLaunch,
                format!("Terminal in {}", p.name),
                Glyph::Icon("terminal"),
                Act::NewShellIn(p.path.clone()),
            )
            .keys(&["terminal", "shell", "launch", &p.name]),
        );
        for w in worktrees(&p.path).unwrap_or_default() {
            out.push(
                Entry::new(
                    Cat::QuickLaunch,
                    format!("Terminal in {} \u{203a} {}", p.name, w.name),
                    Glyph::Icon("terminal"),
                    Act::NewShellIn(w.path.clone()),
                )
                .sub(wt_sub(w))
                .keys(&[
                    "terminal", "shell", "launch", "worktree", &w.name, &w.branch,
                ]),
            );
        }
    }
}

fn filters(alt: &str, out: &mut Vec<Entry>) {
    let shows = [
        (Filter::All, "Show All"),
        (Filter::Running, "Show Running"),
        (Filter::Waiting, "Show Waiting"),
        (Filter::Idle, "Show Idle"),
        (Filter::Error, "Show Errors"),
    ];
    for (i, (f, label)) in shows.into_iter().enumerate() {
        out.push(
            Entry::new(Cat::Filter, label, Glyph::Icon("funnel"), Act::Filter(f))
                .shortcut(format!("{alt}{}", i + 1))
                .keys(&["filter", "status"]),
        );
    }
    let sorts = [
        (Sort::Manual, "Sort: Manual"),
        (Sort::Created, "Sort: Created"),
        (Sort::Recent, "Sort: Recent Activity"),
    ];
    for (s, label) in sorts {
        out.push(
            Entry::new(
                Cat::Filter,
                label,
                Glyph::Icon("arrow-up-down"),
                Act::Sort(s),
            )
            .keys(&["sort", "order"]),
        );
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::state::LayoutRow;

    fn cmd(label: &str, keys: &[&str]) -> Entry {
        Entry::new(Cat::Actions, label, Glyph::None, Act::Nothing).keys(keys)
    }

    #[test]
    fn scores_rank_the_label_before_the_keywords() {
        let c = cmd("New Terminal Session", &["shell", "zsh"]);
        assert_eq!(score("new terminal session", &c), 100);
        assert_eq!(score("new t", &c), 80);
        assert_eq!(score("term", &c), 60);
        assert_eq!(score("minal", &c), 40);
        assert_eq!(score("zsh", &c), 20);
        assert_eq!(score("fish", &c), 0);
    }

    #[test]
    fn an_empty_query_groups_by_category_without_quick_launch() {
        let mut quick = cmd("Acme Code on p", &[]);
        quick.cat = Cat::QuickLaunch;
        let mut filter = cmd("Show All", &[]);
        filter.cat = Cat::Filter;
        let mut project = cmd("All Projects", &[]);
        project.cat = Cat::Projects;
        let list = shown(
            vec![filter, quick.clone(), project, cmd("New Session", &[])],
            " ",
        );
        let labels: Vec<_> = list.iter().map(|c| c.label.as_str()).collect();
        assert_eq!(labels, ["New Session", "All Projects", "Show All"]);
        // Typed, quick launch takes part and the best match comes first.
        let list = shown(vec![cmd("Add Acme", &[]), quick], "acme");
        assert_eq!(list[0].label, "Acme Code on p");
    }

    #[test]
    fn open_layout_narrows_the_palette_to_the_layouts() {
        let st = State {
            layouts: vec![
                LayoutRow {
                    name: "dev".into(),
                    source: "user".into(),
                    sessions: 3,
                },
                LayoutRow {
                    name: "docs".into(),
                    source: "acme.tools".into(),
                    sessions: 1,
                },
            ],
            ..State::default()
        };
        let mut all = Vec::new();
        layouts(&st, &mut all);
        // Nothing typed: the menu entry only, not every layout.
        let rows = shown(all.clone(), "");
        let labels: Vec<_> = rows.iter().map(|c| c.label.as_str()).collect();
        assert_eq!(labels, ["Open Layout \u{203a}"]);
        assert_eq!(rows[0].act, Act::OpenLayoutMenu);
        // The menu's text lists the layouts and not the entry itself.
        let rows = shown(all, LAYOUT_MENU);
        let labels: Vec<_> = rows.iter().map(|c| c.label.as_str()).collect();
        assert_eq!(
            labels,
            ["Open Layout \u{203a} dev", "Open Layout \u{203a} docs"]
        );
        assert_eq!(
            rows[1].sublabel.as_deref(),
            Some("1 session \u{b7} acme.tools")
        );
    }

    #[test]
    fn a_tie_keeps_the_renderer_order() {
        let list = shown(vec![cmd("Show Idle", &[]), cmd("Show All", &[])], "show");
        assert_eq!(list[0].label, "Show Idle");
    }

    #[test]
    fn the_active_row_wraps() {
        let mut p = Palette::default();
        p.step(3, false);
        assert_eq!(p.active, 2);
        p.step(3, true);
        assert_eq!(p.active, 0);
        p.step(0, true);
        assert_eq!(p.active, 0);
    }

    #[test]
    fn a_project_without_hosts_is_local() {
        use serde_json::json;
        assert!(is_local(None));
        assert!(is_local(Some(&json!([]))));
        assert!(is_local(Some(&json!(["local", "h1"]))));
        assert!(!is_local(Some(&json!(["h1"]))));
    }
}

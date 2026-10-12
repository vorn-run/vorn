//! The command palette (`CommandPalette`): every command the app offers,
//! by category, matched against what is typed. The commands are rebuilt
//! from the store each frame, so the list is never stale.

use serde_json::Value;

use crate::act::Act;
use crate::client::Store;
use crate::screen::menus::AGENTS;
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
    Terminals,
    Recent,
    Projects,
    Workflows,
    QuickLaunch,
    Filter,
}

impl Cat {
    pub fn label(self) -> &'static str {
        match self {
            Cat::Actions => "Actions",
            Cat::Terminals => "Terminals",
            Cat::Recent => "Recent Sessions",
            Cat::Projects => "Projects",
            Cat::Workflows => "Workflows",
            Cat::QuickLaunch => "Quick Launch",
            Cat::Filter => "Filter & Sort",
        }
    }
}

/// A row's leading glyph.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Glyph {
    Icon(&'static str),
    Agent(String),
    None,
}

/// One command.
#[derive(Debug, Clone, PartialEq)]
pub struct Command {
    pub cat: Cat,
    pub label: String,
    pub sublabel: Option<String>,
    pub glyph: Glyph,
    pub shortcut: Option<String>,
    pub keywords: Vec<String>,
    pub act: Act,
}

impl Command {
    fn new(cat: Cat, label: impl Into<String>, glyph: Glyph, act: Act) -> Command {
        Command {
            cat,
            label: label.into(),
            sublabel: None,
            glyph,
            shortcut: None,
            keywords: Vec::new(),
            act,
        }
    }

    fn keys(mut self, words: &[&str]) -> Command {
        self.keywords.extend(words.iter().map(|w| (*w).to_owned()));
        self
    }

    fn sub(mut self, text: impl Into<String>) -> Command {
        self.sublabel = Some(text.into());
        self
    }

    fn shortcut(mut self, keys: impl Into<String>) -> Command {
        self.shortcut = Some(keys.into());
        self
    }
}

/// `scoreMatch`: an exact label, a prefix, a word's prefix, anywhere in
/// the label, then the keywords; 0 is no match.
pub fn score(query: &str, cmd: &Command) -> u32 {
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
pub fn shown(commands: Vec<Command>, query: &str) -> Vec<Command> {
    let q = query.trim();
    if q.is_empty() {
        let mut list: Vec<Command> = commands
            .into_iter()
            .filter(|c| c.cat != Cat::QuickLaunch)
            .collect();
        list.sort_by_key(|c| c.cat);
        return list;
    }
    let mut scored: Vec<(u32, Command)> = commands
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

/// `useCommands`: every command, in the renderer's order.
/// `m` and `alt` are the modifier labels; the status filters show the
/// keys that really pick them (Alt), not the renderer's ⌘ digits, which
/// jump to a card.
pub fn commands(store: &Store, st: &State, m: &str, alt: &str) -> Vec<Command> {
    let mut out = actions(store, m);
    for s in &store.sessions {
        let kind = if s.is_shell() {
            "Shell"
        } else {
            crate::screen::menus::agent_name(s.info.agent_type.as_str())
        };
        out.push(
            Command::new(
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
            Command::new(
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
        Command::new(
            Cat::Projects,
            "All Projects",
            Glyph::Icon("monitor"),
            Act::Project(None),
        )
        .keys(&["show all", "clear filter"]),
    );
    for p in &store.projects {
        out.push(
            Command::new(
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
            Command::new(
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
    quick_launch(store, st, &mut out);
    filters(alt, &mut out);
    out
}

fn actions(store: &Store, m: &str) -> Vec<Command> {
    let running = store.sessions.len();
    let mut stop = Command::new(
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
        Command::new(
            Cat::Actions,
            "New Session",
            Glyph::Icon("plus"),
            Act::NewSession,
        )
        .shortcut(format!("{m}N"))
        .keys(&["create", "add", "agent", "terminal", "launch"]),
        Command::new(
            Cat::Actions,
            "Open Settings",
            Glyph::Icon("settings"),
            Act::Settings,
        )
        .shortcut(format!("{m},"))
        .keys(&["preferences", "config", "options"]),
        Command::new(
            Cat::Actions,
            "Toggle Sidebar",
            Glyph::Icon("panel-left"),
            Act::ToggleSidebar,
        )
        .shortcut(format!("{m}B"))
        .keys(&["hide", "show", "panel"]),
        Command::new(
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
        Command::new(
            Cat::Actions,
            "Add Project",
            Glyph::Icon("folder-plus"),
            Act::AddProject,
        )
        .keys(&["new project", "create project"]),
        Command::new(
            Cat::Actions,
            "Add Workflow",
            Glyph::Icon("workflow"),
            Act::Workflow(WfAct::New),
        )
        .keys(&["new workflow", "create workflow", "schedule"]),
        Command::new(
            Cat::Actions,
            "Open Tasks View",
            Glyph::Icon("list-todo"),
            Act::View(MainView::Tasks),
        )
        .keys(&["task", "kanban", "queue", "todo", "board"]),
        Command::new(
            Cat::Actions,
            "Create Task",
            Glyph::Icon("plus"),
            Act::Task(TaskAct::New("todo".into())),
        )
        .keys(&["new task", "add task", "todo"]),
        stop,
        Command::new(
            Cat::Actions,
            "Toggle Layout (Grid/Tabs)",
            Glyph::Icon("layout-dashboard"),
            Act::Layout(next_layout),
        )
        .keys(&["grid", "tabs", "layout", "switch", "view"]),
        Command::new(
            Cat::Actions,
            "Show Welcome Guide",
            Glyph::Icon("book-open"),
            Act::Welcome,
        )
        .keys(&["welcome", "guide", "help", "onboarding", "tour"]),
        Command::new(
            Cat::Actions,
            "New Terminal Session",
            Glyph::Icon("terminal"),
            Act::NewShellIn(String::new()),
        )
        .shortcut("Ctrl+`")
        .keys(&["shell", "terminal", "zsh", "bash"]),
        Command::new(
            Cat::Actions,
            "Copy MCP Server URL",
            Glyph::Icon("plug"),
            Act::Copy {
                text: MCP_URL.to_owned(),
                toast: String::new(),
            },
        )
        .keys(&[
            "mcp",
            "api",
            "integration",
            "claude",
            "cursor",
            "server",
            "url",
        ]),
        Command::new(
            Cat::Actions,
            "Manage Worktrees",
            Glyph::Icon("hard-drive"),
            Act::Settings,
        )
        .keys(&[
            "worktree",
            "disk",
            "space",
            "cleanup",
            "clean",
            "prune",
            "size",
            "node_modules",
        ]),
        Command::new(
            Cat::Actions,
            "SSH & Hosts",
            Glyph::Icon("server"),
            Act::Settings,
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
fn quick_launch(store: &Store, st: &State, out: &mut Vec<Command>) {
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
    let installed = AGENTS.iter().filter(|(k, _)| {
        st.installed
            .as_ref()
            .is_some_and(|i| i.iter().any(|a| a == k))
    });
    for &(agent, name) in installed {
        for p in &local {
            let launch = |worktree: Option<String>| Act::NewSessionIn {
                project: p.name.clone(),
                worktree,
                agent: Some(agent.to_owned()),
            };
            let glyph = Glyph::Agent(agent.to_owned());
            out.push(
                Command::new(
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
                    Command::new(
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
                Command::new(
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
            Command::new(
                Cat::QuickLaunch,
                format!("Terminal in {}", p.name),
                Glyph::Icon("terminal"),
                Act::NewShellIn(p.path.clone()),
            )
            .keys(&["terminal", "shell", "launch", &p.name]),
        );
        for w in worktrees(&p.path).unwrap_or_default() {
            out.push(
                Command::new(
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

fn filters(alt: &str, out: &mut Vec<Command>) {
    let shows = [
        (Filter::All, "Show All"),
        (Filter::Running, "Show Running"),
        (Filter::Waiting, "Show Waiting"),
        (Filter::Idle, "Show Idle"),
        (Filter::Error, "Show Errors"),
    ];
    for (i, (f, label)) in shows.into_iter().enumerate() {
        out.push(
            Command::new(Cat::Filter, label, Glyph::Icon("funnel"), Act::Filter(f))
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
            Command::new(
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

    fn cmd(label: &str, keys: &[&str]) -> Command {
        Command::new(Cat::Actions, label, Glyph::None, Act::Nothing).keys(keys)
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
        let mut quick = cmd("Claude Code on p", &[]);
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
        let list = shown(vec![cmd("Add Claude", &[]), quick], "claude");
        assert_eq!(list[0].label, "Claude Code on p");
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

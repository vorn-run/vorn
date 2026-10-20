//! The welcome guide's modal: a header with the
//! progress bars and the step's title, the step's body, and a footer
//! that moves between steps.

use super::{agent_icon, alt_label, c, ico, mod_label, white, Screen};
use crate::act::Act;
use crate::look;
use crate::onboarding::{self, OnbAct, Then, EXPLORE, MCP_CAPS, STEPS};
use crate::ui::{div, text, Action, El, Id, Rgba, Role};

/// Tailwind v4's `emerald-400`, `cyan-400` and `violet-400`: the guide's
/// status and demo tints.
const EMERALD_400: u32 = 0x00d492;
const CYAN_400: u32 = 0x00d3f2;
const VIOLET_400: u32 = 0xa684ff;

const W: f32 = 520.0;
/// The header's and footer's heights with their hairlines, for the body's
/// share of `max-h-[85vh]`.
const HEADER_H: f32 = 97.0;
const FOOTER_H: f32 = 61.0;
/// How long a copied command shows its check.
const COPIED_FOR: web_time::Duration = web_time::Duration::from_millis(1500);

fn onb(a: OnbAct) -> Act {
    Act::Onboard(a)
}

/// A line of text that takes no pointer.
fn t(s: impl Into<String>, size: f32, line: f32, color: Rgba) -> El {
    text(s, size, color).line_height(line).silent()
}

/// A piece of a paragraph: plain, highlighted, or a key.
enum Seg<'a> {
    P(&'a str),
    Em(&'a str),
    Key(&'a str),
}

/// `Kbd`: a key in a mono box.
fn kbd(keys: &str) -> El {
    div()
        .px(6.0)
        .py(2.0)
        .rounded(4.0)
        .bg(white(0.06))
        .border(1.0, white(0.08))
        .shrink0()
        .child(t(keys, 11.0, 16.0, c(look::GRAY_400)).mono())
}

/// A paragraph laid word by word, so highlighted words and keys wrap
/// with the rest.
fn para(segs: &[Seg<'_>], size: f32, line: f32, color: u32) -> El {
    let mut row = div().row().wrap_children().items_center().w_full();
    for seg in segs {
        let (s, col) = match seg {
            Seg::P(s) => (*s, color),
            Seg::Em(s) => (*s, look::GRAY_200),
            Seg::Key(k) => {
                row = row.child(kbd(k));
                continue;
            }
        };
        for word in s.split_inclusive(' ') {
            row = row.child(t(word, size, line, c(col)));
        }
    }
    row
}

/// The steps' lead paragraph: `text-[13px] text-gray-400 leading-relaxed`.
fn lead(segs: &[Seg<'_>]) -> El {
    para(segs, 13.0, 21.0, look::GRAY_400)
}

/// A footnote: `text-[11px] text-gray-600`.
fn foot(segs: &[Seg<'_>]) -> El {
    para(segs, 11.0, 16.0, look::GRAY_600)
}

/// A sunken card: `rounded-lg border border-white/[0.06] bg-surface-sunken`.
fn card() -> El {
    div()
        .col()
        .rounded(8.0)
        .border(1.0, white(0.06))
        .bg(c(look::SURFACE_SUNKEN))
}

/// A bronze label on a hairline, which closes the guide
/// and opens what it names.
fn accent(s: &Screen<'_>, glyph: &str, label: &str, then: Then) -> El {
    let id = s.acts.on("onboard", onb(OnbAct::Then(then)));
    let h = s.ui.hover(id);
    div()
        .id(id)
        .on(Action::Click)
        .row()
        .items_center()
        .gap(8.0)
        .px(14.0)
        .py(6.0)
        .rounded(8.0)
        .bg(Rgba::hexa(look::BRONZO, 0.04))
        .border(1.0, white(0.08).mix(white(0.14), h))
        .ring(8.0)
        .role(Role::Button, label)
        .child(ico(glyph, 13.0, 2.0, c(look::BRONZO)))
        .child(t(label, 12.0, 16.0, c(look::BRONZO)).weight(500))
}

/// A gray text link that brightens on hover.
fn link(s: &Screen<'_>, act: Act, glyph: &str, size: f32, label: &str, enabled: bool) -> El {
    let id = s.acts.on("onboard", act);
    let col = c(look::GRAY_500).mix(
        c(look::GRAY_300),
        if enabled { s.ui.hover(id) } else { 0.0 },
    );
    let el = div()
        .row()
        .items_center()
        .gap(6.0)
        .opacity(if enabled { 1.0 } else { 0.4 })
        .role(Role::Button, label)
        .disabled(!enabled)
        .child(ico(glyph, size, 2.0, col))
        .child(t(label, 12.0, 16.0, col));
    if enabled {
        el.id(id).on(Action::Click).ring(4.0)
    } else {
        el
    }
}

/// The tile behind an agent's mark.
fn tile(s: &Screen<'_>, agent: &str, box_: f32, glyph: f32, r: f32, bg: Rgba) -> El {
    div()
        .size(box_, box_)
        .rounded(r)
        .bg(bg)
        .center()
        .shrink0()
        .child(agent_icon(s, agent, glyph))
}

/// Copies a command, then shows a check for a moment.
fn copy_btn(s: &Screen<'_>, command: &str) -> El {
    let id = s
        .acts
        .on("onboard-copy", onb(OnbAct::Copy(command.to_owned())));
    let h = s.ui.hover(id);
    let copied =
        s.st.onboard
            .copied
            .as_ref()
            .is_some_and(|(c, at)| c == command && at.elapsed() < COPIED_FOR);
    let glyph = if copied {
        ico("circle-check", 12.0, 2.0, c(EMERALD_400))
    } else {
        ico("copy", 12.0, 2.0, c(look::GRAY_500).mix(c(look::WHITE), h))
    };
    div()
        .id(id)
        .on(Action::Click)
        .p(6.0)
        .rounded(6.0)
        .bg(white(0.04).mix(white(0.08), h))
        .shrink0()
        .ring(6.0)
        .role(Role::Button, "Copy")
        .child(glyph)
}

fn caps_label(s: &str, size: f32) -> El {
    t(s.to_uppercase(), size, size + 5.0, c(look::GRAY_600))
        .weight(500)
        .tracking(0.05)
}

fn step_agents(s: &Screen<'_>) -> El {
    let loading = s.st.installed.is_none();
    let known =
        s.st.installed
            .as_deref()
            .or(s.st.onboard.detected.as_deref());
    let (ready, missing): (Vec<_>, Vec<_>) = s
        .store
        .agents
        .iter()
        .partition(|a| onboarding::is_installed(known, &a.key));
    let mut body = div().col().gap(12.0).child(lead(&[Seg::P(
        "Vorn works with multiple AI coding agents. We detected which ones you have installed.",
    )]));
    if !ready.is_empty() {
        let mut list = div().col().gap(6.0);
        for a in ready {
            list = list.child(
                div()
                    .row()
                    .items_center()
                    .gap(12.0)
                    .px(12.0)
                    .py(8.0)
                    .rounded(8.0)
                    .border(1.0, white(0.06))
                    .bg(c(look::SURFACE_SUNKEN))
                    .child(tile(
                        s,
                        &a.key,
                        28.0,
                        14.0,
                        6.0,
                        Rgba::hexa(a.color, a.bg_alpha),
                    ))
                    .child(t(&a.name, 14.0, 20.0, c(look::GRAY_200)).grow())
                    .child(
                        div()
                            .row()
                            .items_center()
                            .gap(4.0)
                            .child(ico("check", 10.0, 2.0, c(EMERALD_400)))
                            .child(t("Ready", 11.0, 16.0, c(EMERALD_400))),
                    ),
            );
        }
        body = body.child(list);
    }
    if !missing.is_empty() {
        let mut list = div().col().gap(6.0).child(caps_label("Not detected", 11.0));
        for a in missing {
            list = list.child(
                div()
                    .row()
                    .items_center()
                    .gap(12.0)
                    .px(12.0)
                    .py(8.0)
                    .rounded(8.0)
                    .border(1.0, white(0.04))
                    .bg(c(look::SURFACE_SUNKEN))
                    .child(tile(s, &a.key, 28.0, 14.0, 6.0, white(0.03)).opacity(0.4))
                    .child(t(&a.name, 14.0, 20.0, c(look::GRAY_500)).grow().min_w0())
                    .child(
                        div().max_w(180.0).min_w0().child(
                            t(&a.install, 10.0, 14.0, c(look::GRAY_600))
                                .mono()
                                .truncate(),
                        ),
                    ),
            );
        }
        body = body.child(list);
    }
    let label = if loading {
        "Scanning..."
    } else {
        "Re-detect agents"
    };
    body = body.child(div().row().child(link(
        s,
        onb(OnbAct::Redetect),
        "refresh-cw",
        11.0,
        label,
        !loading,
    )));
    body.child(mcp(s, known))
}

/// The collapsible skills and MCP setup.
fn mcp(s: &Screen<'_>, known: Option<&[String]>) -> El {
    let open = s.st.onboard.mcp_open;
    let id = s.acts.on("onboard", onb(OnbAct::ToggleMcp));
    let head = div()
        .id(id)
        .on(Action::Click)
        .row()
        .items_center()
        .gap(8.0)
        .px(12.0)
        .py(10.0)
        .w_full()
        .bg(white(0.0).mix(white(0.02), s.ui.hover(id)))
        .role(Role::Button, "Install skills & MCP server")
        .expanded(open)
        .child(ico("plug", 13.0, 2.0, c(look::BRONZO)))
        .child(
            t("Install skills & MCP server", 12.0, 16.0, c(look::GRAY_200))
                .weight(500)
                .grow(),
        )
        .child(
            div()
                .px(6.0)
                .py(2.0)
                .rounded(4.0)
                .bg(Rgba::hexa(look::BRONZO, 0.08))
                .child(t("Recommended", 10.0, 14.0, c(look::BRONZO)).weight(500)),
        )
        .child(ico(
            if open {
                "chevron-down"
            } else {
                "chevron-right"
            },
            12.0,
            2.0,
            c(look::GRAY_600),
        ));
    let mut box_ = card().clip().child(head);
    if !open {
        return box_;
    }
    let mut caps = div().row().wrap_children().gap(8.0);
    for cap in MCP_CAPS {
        caps = caps.child(
            div()
                .row()
                .items_center()
                .gap(4.0)
                .px(8.0)
                .py(2.0)
                .rounded(4.0)
                .bg(white(0.03))
                .child(ico("check", 8.0, 2.0, Rgba::hexa(EMERALD_400, 0.5)))
                .child(t(cap, 10.0, 14.0, c(look::GRAY_400))),
        );
    }
    let mut setups = div().col().gap(10.0);
    for def in onboarding::mcp_to_show(&s.store.agents, known) {
        let Some(m) = &def.mcp else {
            continue;
        };
        let title = if m.in_agent {
            format!("Run in {}", def.name)
        } else {
            "Run in your terminal".to_owned()
        };
        let mut one = div().col().gap(4.0).child(
            div()
                .row()
                .items_center()
                .gap(8.0)
                .child(tile(
                    s,
                    &def.key,
                    20.0,
                    10.0,
                    4.0,
                    Rgba::hexa(def.color, def.bg_alpha),
                ))
                .child(caps_label(&title, 10.0)),
        );
        for command in &m.commands {
            one = one.child(
                div()
                    .row()
                    .items_center()
                    .gap(8.0)
                    .pl(28.0)
                    .child(
                        div()
                            .grow()
                            .min_w0()
                            .child(t(command, 11.0, 16.0, c(look::GRAY_400)).mono().truncate()),
                    )
                    .child(copy_btn(s, command)),
            );
        }
        if let Some(note) = &m.note {
            one = one.child(
                div()
                    .pl(28.0)
                    .child(t(note, 10.0, 14.0, c(look::GRAY_600)).wrapping()),
            );
        }
        setups = setups.child(one);
    }
    box_ = box_.child(
        div()
            .col()
            .gap(10.0)
            .px(12.0)
            .pb(12.0)
            .border_top(1.0, white(0.04))
            .child(div().pt(8.0).child(para(
                &[Seg::P(
                    "Agents get full context about your projects and tasks \u{2014} they can launch other agents, pick up tasks, read terminal output, and trigger workflows autonomously.",
                )],
                12.0,
                19.5,
                look::GRAY_400,
            )))
            .child(caps)
            .child(setups),
    );
    box_
}

/// A small tag: `text-[9px] px-1.5 py-0.5 rounded`.
fn tag(label: &str, fg: Rgba, bg: Rgba) -> El {
    div()
        .px(6.0)
        .py(2.0)
        .rounded(4.0)
        .bg(bg)
        .child(t(label, 9.0, 12.0, fg))
}

fn step_projects(s: &Screen<'_>) -> El {
    let branch = |name: &str, col: u32, tag_el: El| {
        div()
            .row()
            .items_center()
            .gap(8.0)
            .child(ico("git-branch", 11.0, 2.0, c(col)))
            .child(t(name, 11.0, 16.0, c(look::GRAY_400)))
            .child(tag_el)
    };
    let tree = card()
        .gap(10.0)
        .p(14.0)
        .child(
            div()
                .row()
                .items_center()
                .gap(12.0)
                .child(ico("folder-git-2", 15.0, 2.0, c(CYAN_400)))
                .child(t("my-app/", 12.0, 16.0, c(look::GRAY_200)).weight(500)),
        )
        .child(
            div()
                .col()
                .gap(6.0)
                .ml(16.0)
                .pl(16.0)
                .border_left(1.0, white(0.06))
                .child(branch(
                    "main",
                    look::GRAY_500,
                    tag("default", c(look::GRAY_600), white(0.04)),
                ))
                .child(branch(
                    "feat/auth",
                    VIOLET_400,
                    tag(
                        "worktree",
                        Rgba::hexa(VIOLET_400, 0.6),
                        Rgba::hexa(VIOLET_400, 0.06),
                    ),
                ))
                .child(branch(
                    "fix/api-bug",
                    look::AMBER_400,
                    tag(
                        "worktree",
                        Rgba::hexa(look::AMBER_400, 0.6),
                        Rgba::hexa(look::AMBER_400, 0.06),
                    ),
                )),
        );
    let feature = |glyph: &str, label: &str| {
        div()
            .flex(1.0)
            .row()
            .items_center()
            .gap(8.0)
            .px(10.0)
            .py(6.0)
            .rounded(6.0)
            .border(1.0, white(0.04))
            .bg(c(look::SURFACE_SUNKEN))
            .child(ico(glyph, 11.0, 2.0, c(look::GRAY_500)))
            .child(t(label, 11.0, 16.0, c(look::GRAY_400)))
    };
    let features = div()
        .col()
        .gap(6.0)
        .child(
            div()
                .row()
                .gap(6.0)
                .child(feature("square-kanban", "Kanban board"))
                .child(feature("git-branch", "Auto branch & worktree")),
        )
        .child(
            div()
                .row()
                .gap(6.0)
                .child(feature("bot", "Launch agents from tasks"))
                .child(feature("list-todo", "Full task lifecycle")),
        );
    let n = s.store.projects.len();
    let first = if n > 0 {
        t(
            format!("{n} project{} configured", if n > 1 { "s" } else { "" }),
            12.0,
            16.0,
            c(look::GRAY_500),
        )
    } else {
        accent(s, "folder-plus", "Add project", Then::AddProject)
    };
    let actions = div()
        .row()
        .items_center()
        .gap(12.0)
        .child(first)
        .child(link(
            s,
            onb(OnbAct::Then(Then::OpenTasks)),
            "square-kanban",
            11.0,
            "Open Tasks",
            true,
        ));
    div()
        .col()
        .gap(12.0)
        .child(lead(&[
            Seg::P("A "),
            Seg::Em("project "),
            Seg::P("is a Git repo. "),
            Seg::Em("Worktrees "),
            Seg::P(
                "let multiple agents work on different branches simultaneously without conflicts.",
            ),
        ]))
        .child(tree)
        .child(features)
        .child(actions)
}

fn step_sessions(s: &Screen<'_>) -> El {
    let key = format!("{m}N", m = mod_label());
    let mut pickers = div().row().items_center().gap(8.0);
    for (glyph, label) in [
        ("bot", "Agent"),
        ("folder-git-2", "Project"),
        ("git-branch", "Branch"),
    ] {
        pickers = pickers.child(
            div()
                .row()
                .items_center()
                .gap(6.0)
                .px(10.0)
                .py(6.0)
                .rounded(6.0)
                .bg(white(0.04))
                .border(1.0, white(0.06))
                .child(ico(glyph, 12.0, 2.0, c(look::GRAY_400)))
                .child(t(label, 11.0, 16.0, c(look::GRAY_300)))
                .child(ico("chevron-down", 10.0, 2.0, c(look::GRAY_600))),
        );
    }
    let demo = card()
        .gap(12.0)
        .p(16.0)
        .child(pickers)
        .child(
            div()
                .w_full()
                .px(12.0)
                .py(8.0)
                .rounded(6.0)
                .bg(Rgba::hexa(0, 0.3))
                .border(1.0, white(0.06))
                .child(t(
                    "Type a prompt to direct the agent...",
                    11.0,
                    16.0,
                    c(look::GRAY_600),
                )),
        )
        .child(
            div()
                .row()
                .items_center()
                .justify_between()
                .child(t(
                    "Sessions can also run on remote hosts via SSH",
                    10.0,
                    14.0,
                    c(look::GRAY_600),
                ))
                .child(
                    div()
                        .row()
                        .items_center()
                        .gap(4.0)
                        .px(12.0)
                        .py(4.0)
                        .rounded(6.0)
                        .bg(white(0.06))
                        .child(ico("rocket", 10.0, 2.0, c(look::GRAY_400)))
                        .child(t("Launch", 11.0, 16.0, c(look::GRAY_400))),
                ),
        );
    div()
        .col()
        .gap(16.0)
        .child(lead(&[
            Seg::P("Press "),
            Seg::Key(&key),
            Seg::P(" to open the session launcher. Pick an agent, project, and branch \u{2014} then type an optional prompt to direct the agent."),
        ]))
        .child(demo)
        .child(div().row().child(accent(s, "rocket", "Try it now", Then::NewSession)))
}

fn step_workspace() -> El {
    let key = format!("{m}J", m = mod_label());
    let slot = |h: f32| {
        div()
            .h(h)
            .rounded(4.0)
            .bg(white(0.04))
            .border(1.0, white(0.06))
    };
    let title = |glyph: &str, col: u32, label: &str| {
        div()
            .row()
            .items_center()
            .gap(6.0)
            .mb(4.0)
            .child(ico(glyph, 11.0, 2.0, c(col)))
            .child(t(label, 11.0, 16.0, c(look::GRAY_300)).weight(500))
    };
    let caption = |s: &str| t(s, 10.0, 14.0, c(look::GRAY_600));
    let pair = || {
        div()
            .row()
            .gap(6.0)
            .child(slot(32.0).flex(1.0))
            .child(slot(32.0).flex(1.0))
    };
    let grid = card()
        .flex(1.0)
        .gap(8.0)
        .p(12.0)
        .child(title("layout-grid", CYAN_400, "Grid"))
        .child(div().col().gap(6.0).child(pair()).child(pair()))
        .child(caption("Side-by-side comparison"));
    let mut tabs = div().row().gap(4.0);
    for (i, name) in ["Agent 1", "Agent 2", "Agent 3"].into_iter().enumerate() {
        let (fg, bg) = if i == 0 {
            (c(look::BRONZO), Rgba::hexa(look::BRONZO, 0.08))
        } else {
            (c(0x555555), white(0.03))
        };
        tabs = tabs.child(tag(name, fg, bg));
    }
    let tabbed = card()
        .flex(1.0)
        .gap(8.0)
        .p(12.0)
        .child(title("columns-3", VIOLET_400, "Tabs"))
        .child(div().col().gap(6.0).child(tabs).child(slot(48.0)))
        .child(caption("Maximum terminal space"));
    let feature = |glyph: &str, label: &str| {
        div()
            .row()
            .items_center()
            .gap(4.0)
            .child(ico(glyph, 10.0, 2.0, c(look::GRAY_500)))
            .child(t(label, 11.0, 16.0, c(look::GRAY_500)))
    };
    let review = card()
        .p(12.0)
        .child(
            div()
                .row()
                .items_center()
                .gap(8.0)
                .mb(8.0)
                .child(ico("panel-right", 12.0, 2.0, c(look::GRAY_400)))
                .child(t("Review Panel", 11.0, 16.0, c(look::GRAY_300)).weight(500))
                .child(t(
                    "\u{2014} click the diff badge on any card",
                    10.0,
                    14.0,
                    c(look::GRAY_600),
                )),
        )
        .child(
            div()
                .row()
                .items_center()
                .gap(12.0)
                .child(feature("file-code", "File tree"))
                .child(feature("file-diff", "Syntax diffs"))
                .child(feature("git-commit-horizontal", "Commit & push")),
        );
    div()
        .col()
        .gap(16.0)
        .child(lead(&[
            Seg::P("View sessions in a multi-agent "),
            Seg::Em("Grid "),
            Seg::P("or focused "),
            Seg::Em("Tabs "),
            Seg::P("layout. Press "),
            Seg::Key(&key),
            Seg::P(" to toggle."),
        ]))
        .child(div().row().gap(12.0).child(grid).child(tabbed))
        .child(review)
        .child(foot(&[Seg::P(
            "Headless agents appear as compact pills above the grid for background tasks.",
        )]))
}

fn step_workflows() -> El {
    let mut triggers = card()
        .gap(10.0)
        .p(16.0)
        .child(div().mb(8.0).child(caps_label("Trigger types", 11.0)));
    for (glyph, label, desc) in [
        ("zap", "Manual", "Run on demand from sidebar"),
        ("clock", "Scheduled", "Cron or one-time schedule"),
        ("square-kanban", "Task event", "When a task status changes"),
    ] {
        triggers = triggers.child(
            div()
                .row()
                .items_center()
                .gap(12.0)
                .child(
                    div()
                        .size(24.0, 24.0)
                        .rounded(4.0)
                        .bg(white(0.04))
                        .center()
                        .child(ico(glyph, 12.0, 2.0, c(look::GRAY_400))),
                )
                .child(
                    div()
                        .col()
                        .child(t(label, 12.0, 16.0, c(look::GRAY_300)))
                        .child(t(desc, 11.0, 16.0, c(look::GRAY_600))),
                ),
        );
    }
    let mut nodes = div().row().wrap_children().items_center().gap(8.0);
    for n in ["Launch Agent", "Run Script", "Condition", "Parallel"] {
        nodes = nodes.child(
            div()
                .px(10.0)
                .py(4.0)
                .rounded(6.0)
                .border(1.0, white(0.06))
                .bg(c(look::SURFACE_SUNKEN))
                .child(t(n, 11.0, 16.0, c(look::GRAY_400))),
        );
    }
    div()
        .col()
        .gap(16.0)
        .child(lead(&[Seg::P(
            "Build multi-step workflows that launch agents, run scripts, and branch based on conditions. Trigger them manually, on a schedule, or when a task changes status.",
        )]))
        .child(triggers)
        .child(nodes)
        .child(foot(&[Seg::P("Create workflows from the sidebar or command palette.")]))
}

fn step_shortcuts(s: &Screen<'_>) -> El {
    let keys = onboarding::essential_shortcuts(mod_label(), alt_label());
    let row = |(desc, k): &(&str, String)| {
        div()
            .flex(1.0)
            .row()
            .items_center()
            .justify_between()
            .gap(8.0)
            .child(t(*desc, 12.0, 16.0, c(look::GRAY_400)))
            .child(kbd(k))
    };
    let mut grid = card().gap(8.0).p(12.0);
    for pair in keys.chunks(2) {
        let mut r = div().row().gap(16.0);
        for k in pair {
            r = r.child(row(k));
        }
        grid = grid.child(r);
    }
    let palette = format!("{m}K", m = mod_label());
    let panel = format!("{m}/", m = mod_label());
    div()
        .col()
        .gap(16.0)
        .child(lead(&[
            Seg::P("Press "),
            Seg::Key(&palette),
            Seg::P(" to open the command palette \u{2014} it searches sessions, projects, workflows, and every action in one place."),
        ]))
        .child(grid)
        .child(div().row().child(accent(s, "search", "Try Command Palette", Then::Palette)))
        .child(foot(&[Seg::P("View all shortcuts anytime with "), Seg::Key(&panel)]))
}

fn step_ready(s: &Screen<'_>) -> El {
    let mut list = div().col().gap(6.0);
    // An extension's page shows while the extension contributes it.
    let explore = EXPLORE.into_iter().filter(|(_, _, to)| {
        crate::settings::Category::from_id(to).is_some() || s.st.points.get(to).is_some()
    });
    for (glyph, label, category) in explore {
        let id = s
            .acts
            .on("onboard", onb(OnbAct::Then(Then::Settings(category))));
        let h = s.ui.hover(id);
        // `hover:bg-white/[0.02]` over the sunken fill: a second layer inside.
        let row = div()
            .row()
            .items_center()
            .gap(12.0)
            .px(12.0)
            .py(10.0)
            .bg(white(0.02 * h));
        list = list.child(
            div()
                .id(id)
                .on(Action::Click)
                .rounded(8.0)
                .clip()
                .border(1.0, white(0.04).mix(white(0.08), h))
                .bg(c(look::SURFACE_SUNKEN))
                .ring(8.0)
                .role(Role::Button, label)
                .child(
                    row.child(
                        div()
                            .size(28.0, 28.0)
                            .rounded(6.0)
                            .bg(white(0.04).mix(white(0.06), h))
                            .center()
                            .child(ico(glyph, 14.0, 2.0, c(look::GRAY_400))),
                    )
                    .child(t(label, 13.0, 18.0, c(look::GRAY_300)).grow())
                    .child(ico(
                        "arrow-right",
                        12.0,
                        2.0,
                        c(look::GRAY_600).mix(c(look::GRAY_400), h),
                    )),
                ),
        );
    }
    div()
        .col()
        .gap(16.0)
        .child(lead(&[Seg::P(
            "You are all set. Here are a few more things to explore when you are ready.",
        )]))
        .child(list)
        .child(foot(&[Seg::P(
            "You can reopen this guide anytime from the sidebar or command palette.",
        )]))
}

fn header(s: &Screen<'_>) -> El {
    let o = &s.st.onboard;
    let mut bars = div().row().items_center().gap(6.0).mb(16.0);
    for (i, step) in STEPS.iter().enumerate() {
        let id = s.acts.on("onboard", onb(OnbAct::Go(i)));
        let fill = if i <= o.step {
            c(look::BRONZO)
        } else {
            white(0.08)
        };
        bars = bars.child(
            div()
                .id(id)
                .on(Action::Click)
                .size(if i == o.step { 32.0 } else { 16.0 }, 4.0)
                .rounded(2.0)
                .bg(fill)
                .hit_slop(0.0, 10.0)
                .role(
                    Role::Button,
                    format!("Go to step {}: {}", i + 1, step.title),
                ),
        );
    }
    bars = bars.child(div().grow()).child(t(
        format!("{}/{}", o.step + 1, STEPS.len()),
        11.0,
        16.0,
        c(look::GRAY_600),
    ));
    let step = o.current();
    let title = div()
        .row()
        .items_center()
        .gap(12.0)
        .child(
            div()
                .size(40.0, 40.0)
                .rounded(12.0)
                .bg(Rgba::hexa(look::BRONZO, 0.06))
                .center()
                .shrink0()
                .child(ico(step.icon, 20.0, 2.0, c(look::BRONZO))),
        )
        .child(
            div()
                .col()
                .child(
                    text(step.title, 16.0, c(look::WHITE))
                        .weight(600)
                        .line_height(24.0)
                        .role(Role::Heading, step.title),
                )
                .child(t(step.subtitle, 12.0, 16.0, c(look::GRAY_500))),
        );
    div()
        .col()
        .px(24.0)
        .pt(20.0)
        .pb(16.0)
        .border_bottom(1.0, white(0.06))
        .child(bars)
        .child(title)
}

fn footer(s: &Screen<'_>) -> El {
    let o = &s.st.onboard;
    let skip = s.acts.on("onboard-skip", onb(OnbAct::Close));
    let mut row = div()
        .row()
        .items_center()
        .gap(12.0)
        .px(24.0)
        .py(14.0)
        .border_top(1.0, white(0.06))
        .child(
            div()
                .id(skip)
                .on(Action::Click)
                .ring(4.0)
                .role(Role::Button, "Skip guide")
                .child(t(
                    "Skip guide",
                    12.0,
                    16.0,
                    c(look::GRAY_600).mix(c(look::GRAY_400), s.ui.hover(skip)),
                )),
        )
        .child(div().grow());
    if o.step > 0 {
        let back = s.acts.on("onboard", onb(OnbAct::Back));
        let h = s.ui.hover(back);
        let col = c(look::GRAY_400).mix(c(look::WHITE), h);
        row = row.child(
            div()
                .id(back)
                .on(Action::Click)
                .row()
                .items_center()
                .gap(6.0)
                .px(12.0)
                .py(6.0)
                .rounded(8.0)
                .bg(white(0.0).mix(white(0.04), h))
                .ring(8.0)
                .role(Role::Button, "Back")
                .child(ico("arrow-left", 13.0, 2.0, col))
                .child(t("Back", 13.0, 20.0, col)),
        );
    }
    let next = s.acts.on("onboard", onb(OnbAct::Next));
    let label = if o.is_last() { "Get Started" } else { "Next" };
    let fill = c(look::BRONZO).mix(c(crate::ui::theme::rgb::BRONZO_DARK), s.ui.hover(next));
    row.child(
        div()
            .id(next)
            .on(Action::Click)
            .row()
            .items_center()
            .gap(6.0)
            .px(16.0)
            .py(6.0)
            .rounded(8.0)
            .bg(fill)
            .ring(8.0)
            .role(Role::Button, label)
            .child(t(label, 13.0, 20.0, c(0)).weight(500))
            .child(ico("arrow-right", 13.0, 2.0, c(0))),
    )
}

pub(super) fn modal(s: &Screen<'_>) -> Option<El> {
    let o = &s.st.onboard;
    let body_max = (s.size.1 * 0.85 - HEADER_H - FOOTER_H).max(120.0);
    let body = match o.current().id {
        "agents" => step_agents(s),
        "projects" => step_projects(s),
        "sessions" => step_sessions(s),
        "workspace" => step_workspace(),
        "workflows" => step_workflows(),
        "shortcuts" => step_shortcuts(s),
        _ => step_ready(s),
    };
    let x = s.acts.on("onboard-x", onb(OnbAct::Close));
    let close = div()
        .id(x)
        .on(Action::Click)
        .absolute_right(14.0, 14.0)
        .p(4.0)
        .rounded(4.0)
        .ring(4.0)
        .role(Role::Button, "Close welcome guide")
        .child(ico(
            "x",
            16.0,
            2.0,
            c(look::GRAY_600).mix(c(look::GRAY_300), s.ui.hover(x)),
        ));
    let panel = div()
        .col()
        .w(W)
        .rounded(16.0)
        .border(1.0, white(0.08))
        .bg(c(look::SURFACE_OVERLAY))
        .clip()
        .role(Role::Dialog, "Welcome guide")
        .child(header(s))
        .child(
            div()
                .id(Id::new("onboard-body"))
                .col()
                .px(24.0)
                .py(20.0)
                .max_h(body_max)
                .scroll_y()
                .child(body),
        )
        .child(footer(s))
        .child(close);
    let panel = s.acts.click(panel, "dialog", Act::Nothing);
    let scrim = s.acts.on("onboard-scrim", onb(OnbAct::Close));
    Some(
        crate::ui::widgets::scrim(scrim, s.size, 0.6)
            .center()
            .child(panel),
    )
}

//! Settings › Agents: the agents on this machine, the ones that need
//! setup, and one agent opened in place with its Vorn tools, default,
//! command and arguments.

use serde_json::Value;

use super::pages::{default_act, default_agent, glyph, glyph_icon, pill_button, tip};
use super::{card, header, label, section, set};
use crate::act::Act;
use crate::look;
use crate::screen::{agent_icon, c, white, Screen};
use crate::settings::{self, args_text, command_of, modified, Agent, AgentField, SetAct};
use crate::ui::{div, field, text, tooltip, Action, El, Id, Role, Sense, TextStyle};

/// The text field for an agent's `field`.
pub fn agent_field_id(agent: &str, f: AgentField) -> Id {
    Id::new("agent-field").child((agent, f))
}

/// The field the Agents page filters its list with.
pub const AGENTS_FILTER: Id = Id(0x6167_6e74_7366_6c74);

/// Where an agent's program was found, home-relative.
fn found_at<'a>(s: &Screen<'a>, a: &Agent) -> Option<String> {
    s.st.agent_paths
        .get(a.key.as_str())
        .map(|p| settings::home_relative(p))
}

pub(super) fn page(s: &Screen<'_>) -> El {
    let open = s.st.settings.agent.as_deref();
    match open.and_then(|k| s.store.agents.get(k)) {
        Some(a) => detail(s, a),
        None => list(s),
    }
}

fn list(s: &Screen<'_>) -> El {
    let query = s.ui.edit_text(AGENTS_FILTER).trim().to_lowercase();
    let shown = s
        .store
        .agents
        .iter()
        .filter(|a| query.is_empty() || a.name.to_lowercase().contains(&query));
    let (ready, setup) = settings::partition_agents(shown, s.st.installed.as_deref());
    let mut col = div()
        .col()
        .child(header("Agents", "", Some(header_tools(s))));
    if !ready.is_empty() {
        col = col.child(card(ready.iter().map(|a| ready_row(s, a)).collect()));
    }
    if !setup.is_empty() {
        col = col
            .child(section("Needs setup"))
            .child(card(setup.iter().map(|a| setup_row(s, a)).collect()));
    }
    if ready.is_empty() && setup.is_empty() {
        col = col.child(
            text("No agents match", 13.0, c(look::GRAY_500))
                .line_height(18.0)
                .px(4.0)
                .py(8.0),
        );
    }
    let more_id = s.acts.on(
        "agents-more",
        set(SetAct::Browse(Some(crate::plugins::list::Tag::Agent), None)),
    );
    let more_ink = c(look::BRONZO).mix(c(look::WHITE), s.ui.hover(more_id) * 0.3);
    let more = div()
        .id(more_id)
        .on(Action::Click)
        .role(Role::Link, "Get more agents")
        .ring(2.0)
        .child(
            text("Get more agents\u{2026}", 13.0, more_ink)
                .line_height(18.0)
                .silent(),
        );
    col.child(div().row().px(4.0).mt(12.0).child(tooltip(
        s.ui,
        more,
        more_id,
        tip::MORE_AGENTS,
        None,
    )))
}

/// The header's search field and refresh button.
fn header_tools(s: &Screen<'_>) -> El {
    let focus = if s.ui.focused(AGENTS_FILTER) {
        1.0
    } else {
        0.0
    };
    let style = TextStyle {
        line_height: 16.0,
        ..TextStyle::ui(12.0)
    };
    let search = div()
        .row()
        .items_center()
        .gap(8.0)
        .w(180.0)
        .px(10.0)
        .py(6.0)
        .rounded(4.0)
        .bg(white(0.04))
        .border(1.0, white(0.06).mix(white(0.2), focus))
        .frames(AGENTS_FILTER)
        .child(glyph_icon(glyph::SEARCH, 13.0, c(look::GRAY_500)))
        .child(
            field(
                AGENTS_FILTER,
                "Search agents",
                "Search agents",
                style,
                look::GRAY_200,
            )
            .grow()
            .min_w0(),
        );
    let busy = s.st.settings.detecting;
    let re = s.acts.on("agents-redetect", set(SetAct::Redetect));
    let h = s.ui.hover(re);
    let mut refresh = div()
        .id(re)
        .size(28.0, 28.0)
        .center()
        .rounded(4.0)
        .bg(white(0.0).mix(white(0.06), h))
        .role(Role::Button, "Look for agents again")
        .ring(4.0)
        .child(glyph_icon(
            glyph::REFRESH,
            14.0,
            c(look::GRAY_500).mix(c(look::WHITE), h),
        ));
    refresh = if busy {
        refresh.opacity(0.4).disabled(true)
    } else {
        refresh.on(Action::Click)
    };
    div()
        .row()
        .items_center()
        .gap(8.0)
        .child(search)
        .child(tooltip(s.ui, refresh, re, tip::REDETECT, None))
}

/// The `Default` tag beside the default agent's name.
fn default_tag(s: &Screen<'_>, agent: &str) -> El {
    let id = Id::new("agent-default-tag").child(agent);
    let tag = div()
        .px(7.0)
        .py(1.0)
        .rounded(4.0)
        .bg(white(0.08))
        .sense(Sense::HOVER)
        .child(
            text("Default", 10.0, c(look::GRAY_300))
                .line_height(14.0)
                .role(Role::Label, "Default"),
        );
    tooltip(s.ui, tag, id, tip::DEFAULT_TAG, None)
}

/// Vorn tools ✓ in a muted green, else a neutral Connect tools.
fn tools_chip(s: &Screen<'_>, a: &Agent, open: &Act) -> Option<El> {
    a.mcp.as_ref()?;
    let on = settings::tools_connected(&s.store.config, &a.key);
    let id = s.acts.on(&format!("agent-tools:{}", a.key), open.clone());
    let (bg, fg, words) = if on {
        (
            c(look::STATUS_SAGE).alpha(0.2),
            c(look::STATUS_SAGE).mix(c(look::WHITE), 0.45),
            "Vorn tools \u{2713}",
        )
    } else {
        (
            white(0.08).mix(white(0.12), s.ui.hover(id)),
            c(look::GRAY_200),
            "Connect tools",
        )
    };
    let chip = div()
        .id(id)
        .on(Action::Click)
        .shrink0()
        .px(9.0)
        .py(3.0)
        .rounded(4.0)
        .bg(bg)
        .role(Role::Button, words)
        .child(text(words, 11.0, fg).line_height(14.0).silent());
    Some(tooltip(s.ui, chip, id, tip::TOOLS, None))
}

fn ready_row(s: &Screen<'_>, a: &Agent) -> El {
    let open = set(SetAct::OpenAgent(Some(a.key.clone())));
    let id = s.acts.on(&format!("agent-row:{}", a.key), open.clone());
    let sub = match found_at(s, a) {
        Some(p) => format!("Ready \u{b7} {p}"),
        None => "Ready".to_owned(),
    };
    let is_default = default_agent(s) == a.key;
    let row = div()
        .id(id)
        .on(Action::Click)
        .row()
        .items_center()
        .gap(12.0)
        .px(16.0)
        .py(11.0)
        .bg(white(0.0).mix(white(0.03), s.ui.hover(id)))
        .role(Role::Button, a.name.as_str())
        .child(div().shrink0().child(agent_icon(s, &a.key, 24.0)))
        .child(
            div()
                .col()
                .gap(2.0)
                .grow()
                .min_w0()
                .child(
                    div()
                        .row()
                        .items_center()
                        .gap(8.0)
                        .child(
                            text(a.name.as_str(), 14.0, c(look::GRAY_100))
                                .line_height(20.0)
                                .silent(),
                        )
                        .child_if(is_default.then(|| default_tag(s, &a.key))),
                )
                .child(
                    text(sub, 11.0, c(look::GRAY_500))
                        .line_height(15.0)
                        .truncate()
                        .silent(),
                ),
        )
        .child_if(tools_chip(s, a, &open))
        .child(glyph_icon(glyph::CHEVRON_RIGHT, 12.0, c(look::GRAY_600)));
    tooltip(s.ui, row, id, tip::AGENT_ROW, None)
}

/// Not on this machine.
const NOT_HERE: &str = if cfg!(target_os = "macos") {
    "Not on this Mac"
} else {
    "Not on this computer"
};

fn setup_row(s: &Screen<'_>, a: &Agent) -> El {
    div()
        .row()
        .items_center()
        .gap(12.0)
        .px(16.0)
        .py(11.0)
        .role(Role::Group, a.name.as_str())
        .child(
            div()
                .shrink0()
                .opacity(0.5)
                .child(agent_icon(s, &a.key, 24.0)),
        )
        .child(
            div()
                .col()
                .gap(2.0)
                .grow()
                .min_w0()
                .child(
                    text(a.name.as_str(), 14.0, c(look::GRAY_300))
                        .line_height(20.0)
                        .silent(),
                )
                .child(
                    text(NOT_HERE, 11.0, c(look::GRAY_500))
                        .line_height(15.0)
                        .silent(),
                ),
        )
        .child_if((!a.install.is_empty()).then(|| {
            pill_button(
                s,
                &format!("agent-install:{}", a.key),
                "Install",
                SetAct::InstallAgent(a.key.clone()),
                tip::INSTALL_AGENT,
                false,
                true,
            )
        }))
}

/// The adapter's version, once the installed packs are known.
fn adapter_version<'a>(s: &Screen<'a>, a: &Agent) -> Option<&'a str> {
    s.st.plugins
        .packs
        .iter()
        .find(|p| p.get("id").and_then(Value::as_str) == Some(a.extension.as_str()))
        .and_then(|p| p.get("version"))
        .and_then(Value::as_str)
}

fn detail(s: &Screen<'_>, a: &Agent) -> El {
    let back = s.acts.on("agent-back", set(SetAct::OpenAgent(None)));
    let ink = c(look::GRAY_500).mix(c(look::GRAY_200), s.ui.hover(back));
    let back = div()
        .id(back)
        .on(Action::Click)
        .row()
        .items_center()
        .gap(2.0)
        .mb(12.0)
        .role(Role::Link, "Agents")
        .ring(2.0)
        .child(glyph_icon(glyph::CHEVRON_LEFT, 12.0, ink))
        .child(text("Agents", 12.0, ink).line_height(16.0).silent());
    let mut sub = vec![if settings::is_ready(s.st.installed.as_deref(), &a.key) {
        "Ready".to_owned()
    } else {
        NOT_HERE.to_owned()
    }];
    sub.extend(found_at(s, a));
    sub.extend(adapter_version(s, a).map(|v| format!("adapter {v}")));
    let is_default = default_agent(s) == a.key;
    let make_default = if is_default {
        default_tag(s, &a.key)
    } else {
        pill_button(
            s,
            "agent-make-default",
            "Make default",
            default_act("defaultAgent", format!("\"{}\"", a.key)),
            tip::MAKE_DEFAULT,
            false,
            true,
        )
    };
    let head = div()
        .row()
        .items_center()
        .gap(14.0)
        .p(16.0)
        .child(div().shrink0().child(agent_icon(s, &a.key, 40.0)))
        .child(
            div()
                .col()
                .gap(3.0)
                .grow()
                .min_w0()
                .child(
                    text(a.name.as_str(), 17.0, c(look::WHITE))
                        .weight(600)
                        .line_height(22.0)
                        .role(Role::Heading, a.name.as_str()),
                )
                .child(
                    text(sub.join(" \u{b7} "), 12.0, c(look::GRAY_500))
                        .line_height(16.0)
                        .truncate()
                        .silent(),
                ),
        )
        .child(make_default);
    let mut col = div().col().child(back).child(card(vec![head]));
    if let Some(tools) = tools_card(s, a) {
        col = col.child(section("Vorn tools")).child(tools);
    }
    col.child(div().col().mt(16.0).child(card(more_rows(s, a))))
}

/// Not connected or Connected, Connect, and what Connect runs.
fn tools_card(s: &Screen<'_>, a: &Agent) -> Option<El> {
    let setup = a.mcp.as_ref()?;
    let on = settings::tools_connected(&s.store.config, &a.key);
    let short = s.store.agents.short_name(&a.key);
    let ready = settings::is_ready(s.st.installed.as_deref(), &a.key);
    let connect = pill_button(
        s,
        "agent-connect",
        if on { "Connect again" } else { "Connect" },
        SetAct::ConnectAgent(a.key.clone()),
        tip::CONNECT,
        !on,
        ready,
    );
    let state = div()
        .row()
        .items_center()
        .justify_between()
        .gap(16.0)
        .px(16.0)
        .py(14.0)
        .child(
            div()
                .col()
                .gap(3.0)
                .min_w0()
                .child(
                    text(
                        if on { "Connected" } else { "Not connected" },
                        14.0,
                        c(look::GRAY_100),
                    )
                    .line_height(20.0)
                    .role(Role::Label, "Vorn tools"),
                )
                .child(
                    text(
                        format!("Lets {short} open panes, read sessions and run workflows in Vorn"),
                        12.0,
                        c(look::GRAY_500),
                    )
                    .line_height(16.0)
                    .wrapping()
                    .silent(),
                ),
        )
        .child(connect);
    let runs_in = if setup.in_agent {
        format!("Vorn runs this in a new {short} session for you")
    } else {
        "Vorn runs this in a terminal for you".to_owned()
    };
    let mut how = div().col().gap(6.0).px(16.0).py(12.0).child(
        text(runs_in, 11.0, c(look::GRAY_500))
            .line_height(15.0)
            .silent(),
    );
    for cmd in &setup.commands {
        how = how.child(
            div().px(10.0).py(7.0).rounded(4.0).bg(white(0.04)).child(
                text(cmd.as_str(), 12.0, c(look::GRAY_300))
                    .mono()
                    .line_height(16.0)
                    .wrapping()
                    .role(Role::Label, cmd.as_str()),
            ),
        );
    }
    if let Some(n) = &setup.note {
        how = how.child(
            text(n.as_str(), 11.0, c(look::GRAY_500))
                .line_height(15.0)
                .wrapping()
                .silent(),
        );
    }
    Some(card(vec![state, how]))
}

/// A row of the last card: a label at the left, `right` at the right.
fn link_row(
    s: &Screen<'_>,
    place: &str,
    act: Act,
    label: &str,
    ink: crate::ui::Rgba,
    right: Option<El>,
    tip_text: &str,
) -> El {
    let id = s.acts.on(place, act);
    let row = div()
        .id(id)
        .on(Action::Click)
        .row()
        .items_center()
        .justify_between()
        .px(16.0)
        .py(12.0)
        .bg(white(0.0).mix(white(0.03), s.ui.hover(id)))
        .role(Role::Button, label)
        .child(text(label, 14.0, ink).line_height(20.0).silent())
        .child_if(right);
    tooltip(s.ui, row, id, tip_text, None)
}

/// Advanced (and its fields when open), Documentation, Remove agent.
fn more_rows(s: &Screen<'_>, a: &Agent) -> Vec<El> {
    let open = s.st.settings.agents_open.contains(a.key.as_str());
    let hint = div()
        .row()
        .items_center()
        .gap(4.0)
        .child(
            text("Command, arguments", 12.0, c(look::GRAY_500))
                .line_height(16.0)
                .silent(),
        )
        .child(glyph_icon(
            if open {
                glyph::CHEVRON_DOWN
            } else {
                glyph::CHEVRON_RIGHT
            },
            12.0,
            c(look::GRAY_500),
        ));
    let mut rows = vec![link_row(
        s,
        "agent-advanced",
        set(SetAct::ExpandAgent(a.key.clone())),
        "Advanced",
        c(look::GRAY_100),
        Some(hint),
        tip::ADVANCED,
    )
    .expanded(open)];
    if open {
        rows.push(advanced(s, a));
    }
    if !a.url.is_empty() {
        rows.push(link_row(
            s,
            "agent-docs",
            Act::OpenUrl(a.url.clone()),
            "Documentation",
            c(look::GRAY_100),
            Some(glyph_icon(glyph::ARROW_UP_RIGHT, 12.0, c(look::GRAY_500))),
            tip::DOCS,
        ));
    }
    rows.push(remove_row(s, a));
    rows
}

/// Remove agent, red; once pressed it asks in the row.
fn remove_row(s: &Screen<'_>, a: &Agent) -> El {
    let act = set(SetAct::RemoveAgent(a.key.clone()));
    if s.st.settings.confirm_remove.as_deref() != Some(a.key.as_str()) {
        return link_row(
            s,
            "agent-remove",
            act,
            "Remove agent",
            c(look::RED_400),
            None,
            tip::REMOVE_AGENT,
        );
    }
    let cancel = super::button(s, "agent-keep", "Cancel", SetAct::KeepAgent, true);
    let remove_id = s.acts.on("agent-remove-ok", act);
    let remove = div()
        .id(remove_id)
        .on(Action::Click)
        .row()
        .items_center()
        .h(28.0)
        .px(12.0)
        .rounded(4.0)
        .bg(white(0.06).mix(white(0.1), s.ui.hover(remove_id)))
        .role(Role::Button, "Remove")
        .ring(4.0)
        .child(
            text("Remove", 12.0, c(look::RED_400))
                .weight(500)
                .line_height(16.0)
                .silent(),
        );
    div()
        .row()
        .items_center()
        .justify_between()
        .gap(12.0)
        .px(16.0)
        .py(10.0)
        .child(
            text(
                format!("Remove {}'s adapter from Vorn?", a.name),
                14.0,
                c(look::GRAY_100),
            )
            .line_height(20.0)
            .truncate()
            .silent(),
        )
        .child(div().row().gap(8.0).child(cancel).child(remove))
}

/// A mono input bound to an agent's command field.
fn input(s: &Screen<'_>, agent: &str, f: AgentField, value: String, placeholder: &str) -> El {
    let id = agent_field_id(agent, f);
    s.acts
        .text(id, set(SetAct::AgentText(agent.to_owned(), f)), value);
    let focus = s.ui.focused(id);
    div()
        .w_full()
        .px(10.0)
        .py(6.0)
        .rounded(4.0)
        .bg(white(0.04))
        .border(1.0, if focus { white(0.15) } else { white(0.06) })
        .frames(id)
        .child(
            field(
                id,
                placeholder,
                placeholder,
                TextStyle {
                    line_height: 16.0,
                    ..TextStyle::mono(12.0)
                },
                c(look::GRAY_200),
            )
            .placeholder_color(c(look::GRAY_700))
            .w_full(),
        )
}

/// Command, Arguments, Headless arguments, and Reset when changed.
fn advanced(s: &Screen<'_>, a: &Agent) -> El {
    let cmd = command_of(&s.store.config, a);
    let typed = |f: AgentField, headless: bool| {
        s.st.settings
            .args
            .get(&(a.key.to_owned(), f))
            .cloned()
            .unwrap_or_else(|| args_text(&cmd, headless))
    };
    let command = cmd
        .get("command")
        .and_then(Value::as_str)
        .unwrap_or("")
        .to_owned();
    let fields = div()
        .row()
        .gap(8.0)
        .child(
            div()
                .col()
                .w(140.0)
                .shrink0()
                .child(label("Command", 4.0))
                .child(input(s, &a.key, AgentField::Command, command, "Command")),
        )
        .child(
            div()
                .col()
                .grow()
                .min_w0()
                .child(label("Arguments", 4.0))
                .child(input(
                    s,
                    &a.key,
                    AgentField::Args,
                    typed(AgentField::Args, false),
                    "e.g. --dangerously-skip-permissions",
                )),
        );
    let headless = div()
        .col()
        .child(label("Headless arguments", 4.0))
        .child(input(
            s,
            &a.key,
            AgentField::Headless,
            typed(AgentField::Headless, true),
            "Used for workflow runs",
        ));
    let reset = modified(&cmd, a).then(|| {
        div().row().justify_end().child(super::button(
            s,
            "agent-reset",
            "Reset",
            SetAct::ResetAgent(a.key.clone()),
            true,
        ))
    });
    div()
        .col()
        .gap(8.0)
        .px(16.0)
        .pt(4.0)
        .pb(14.0)
        .child(fields)
        .child(headless)
        .child_if(reset)
}

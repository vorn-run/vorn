//! `AgentSettings`: which agents are installed, each installed agent's
//! command and arguments, and how to install the rest.

use super::{label, set};
use crate::act::Act;
use crate::look;
use crate::screen::{agent_icon, c, ico, white, Screen};
use crate::settings::{args_text, command_of, modified, AgentField, SetAct, AGENTS};
use crate::ui::{div, field, text, Action, El, Id, Rgba, Role, TextStyle};

/// The text field for an agent's `field`.
pub fn agent_field_id(agent: &str, f: AgentField) -> Id {
    Id::new("agent-field").child((agent, f))
}

/// A `bg-black/30` mono input bound to an agent's command field.
fn input(s: &Screen<'_>, agent: &str, f: AgentField, value: String, placeholder: &str) -> El {
    let id = agent_field_id(agent, f);
    s.acts
        .text(id, set(SetAct::AgentText(agent.to_owned(), f)), value);
    let focus = s.ui.focused(id);
    div()
        .w_full()
        .px(10.0)
        .py(6.0)
        .rounded(6.0)
        .bg(Rgba::hexa(0, 0.3))
        .border(1.0, if focus { white(0.15) } else { white(0.06) })
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

fn installed_card(s: &Screen<'_>, a: &'static crate::settings::Agent) -> El {
    let cmd = command_of(&s.store.config, a);
    let typed = |f: AgentField, headless: bool| {
        s.st.settings
            .args
            .get(&(a.key.to_owned(), f))
            .cloned()
            .unwrap_or_else(|| args_text(&cmd, headless))
    };
    let card_id = Id::new("agent-card").child(a.key);
    let hot = s.ui.hover(card_id) > 0.0;
    let mut head = div()
        .row()
        .items_center()
        .gap(12.0)
        .px(16.0)
        .py(12.0)
        .border_left(2.0, Rgba::hexa(a.color, 0x30 as f32 / 255.0))
        .child(
            div()
                .size(36.0, 36.0)
                .center()
                .shrink0()
                .rounded(8.0)
                .bg(Rgba::hexa(a.color, a.bg_alpha))
                .child(agent_icon(s, a.key, 18.0)),
        )
        .child(
            div()
                .col()
                .grow()
                .min_w0()
                .child(
                    div()
                        .row()
                        .items_center()
                        .gap(8.0)
                        .child(
                            text(a.name, 14.0, c(0xf3f4f6))
                                .weight(500)
                                .line_height(20.0)
                                .silent(),
                        )
                        .child(
                            div()
                                .row()
                                .items_center()
                                .gap(4.0)
                                .px(6.0)
                                .py(2.0)
                                .rounded(999.0)
                                .bg(Rgba::hexa(a.color, 0x12 as f32 / 255.0))
                                .child(ico("terminal", 8.0, 2.0, c(a.color)))
                                .child(
                                    text("Ready", 10.0, c(a.color))
                                        .weight(500)
                                        .line_height(15.0)
                                        .silent(),
                                ),
                        ),
                )
                .child(
                    text(a.description, 11.0, c(look::GRAY_500))
                        .line_height(16.5)
                        .silent(),
                ),
        );
    if modified(&cmd, a) {
        let id = s
            .acts
            .on("agent-reset", set(SetAct::ResetAgent(a.key.to_owned())));
        let h = s.ui.hover(id);
        head = head.child(
            div()
                .id(id)
                .on(Action::Click)
                .row()
                .items_center()
                .gap(4.0)
                .px(8.0)
                .py(4.0)
                .rounded(6.0)
                .bg(white(0.03).mix(white(0.06), h))
                .border(1.0, white(0.04).mix(white(0.08), h))
                .opacity(if hot || h > 0.0 { 1.0 } else { 0.0 })
                .role(Role::Button, "Reset")
                .child(ico("rotate-ccw", 10.0, 2.0, c(look::GRAY_500)))
                .child(
                    text("Reset", 11.0, c(look::GRAY_500).mix(c(look::GRAY_300), h))
                        .line_height(16.5)
                        .silent(),
                ),
        );
    }
    let command = cmd
        .get("command")
        .and_then(|v| v.as_str())
        .unwrap_or("")
        .to_owned();
    let body = div()
        .col()
        .gap(8.0)
        .px(16.0)
        .pb(12.0)
        .pt(4.0)
        .child(
            div()
                .row()
                .gap(8.0)
                .child(
                    div()
                        .col()
                        .w(140.0)
                        .shrink0()
                        .child(label("Command", 4.0))
                        .child(input(s, a.key, AgentField::Command, command, "Command")),
                )
                .child(
                    div()
                        .col()
                        .grow()
                        .min_w0()
                        .child(label("Arguments", 4.0))
                        .child(input(
                            s,
                            a.key,
                            AgentField::Args,
                            typed(AgentField::Args, false),
                            "e.g. --dangerously-skip-permissions",
                        )),
                ),
        )
        .child(
            div()
                .col()
                .child(label("Headless Arguments", 4.0))
                .child(input(
                    s,
                    a.key,
                    AgentField::Headless,
                    typed(AgentField::Headless, true),
                    "e.g. --dangerously-skip-permissions (used for headless/workflow runs)",
                )),
        );
    div()
        .id(card_id)
        .sense(crate::ui::Sense::HOVER)
        .col()
        .rounded(12.0)
        .clip()
        .bg(c(look::SURFACE_SUNKEN))
        .border(1.0, if hot { white(0.1) } else { white(0.06) })
        .role(Role::Group, a.name)
        .child(head)
        .child(body)
}

fn available_card(s: &Screen<'_>, a: &'static crate::settings::Agent) -> El {
    let open = s.st.settings.agents_open.contains(a.key);
    let toggle = s
        .acts
        .on("agent-expand", set(SetAct::ExpandAgent(a.key.to_owned())));
    let head = div()
        .id(toggle)
        .on(Action::Click)
        .row()
        .items_center()
        .gap(12.0)
        .px(16.0)
        .py(12.0)
        .role(Role::Button, a.name)
        .expanded(open)
        .child(
            div()
                .size(36.0, 36.0)
                .center()
                .shrink0()
                .rounded(8.0)
                .bg(white(0.03))
                .opacity(0.5)
                .child(agent_icon(s, a.key, 18.0)),
        )
        .child(
            div()
                .row()
                .items_end()
                .grow()
                .min_w0()
                .child(
                    text(a.name, 14.0, c(look::GRAY_400))
                        .weight(500)
                        .line_height(20.0)
                        .silent(),
                )
                .child(
                    text(a.description, 11.0, c(look::GRAY_600))
                        .line_height(18.0)
                        .ml(8.0)
                        .silent(),
                ),
        )
        .child(ico(
            if open {
                "chevron-down"
            } else {
                "chevron-right"
            },
            14.0,
            2.0,
            c(look::GRAY_600),
        ));
    let hot = s.ui.hover(toggle) > 0.0;
    let mut card = div()
        .col()
        .rounded(12.0)
        .clip()
        .bg(c(look::SURFACE_SUNKEN))
        .border(1.0, if hot { white(0.06) } else { white(0.04) })
        .child(head);
    if open {
        let copy = s
            .acts
            .on("agent-copy", set(SetAct::CopyInstall(a.key.to_owned())));
        let docs = s.acts.on("agent-docs", Act::OpenUrl(a.url.to_owned()));
        let copied =
            s.st.settings
                .copied
                .as_ref()
                .is_some_and(|(k, _)| k == a.key);
        let square = |id: Id, el: El, label: &str| {
            let h = s.ui.hover(id);
            let b = div()
                .id(id)
                .on(Action::Click)
                .p(8.0)
                .rounded(8.0)
                .shrink0()
                .bg(white(0.04).mix(white(0.08), h))
                .border(1.0, white(0.04).mix(white(0.08), h))
                .role(Role::Button, label)
                .child(el);
            crate::ui::tooltip(s.ui, b, id, label, None)
        };
        let copy_icon = if copied {
            ico("circle-check-big", 14.0, 2.0, c(0x00d492))
        } else {
            ico("copy", 14.0, 2.0, c(look::GRAY_500))
        };
        card = card.child(
            div().px(16.0).pb(16.0).child(
                div()
                    .col()
                    .p(12.0)
                    .rounded(8.0)
                    .bg(Rgba::hexa(0, 0.3))
                    .border(1.0, white(0.06))
                    .child(
                        div()
                            .row()
                            .items_center()
                            .gap(8.0)
                            .mb(8.0)
                            .child(ico("terminal", 10.0, 2.0, c(look::GRAY_500)))
                            .child(
                                text("INSTALL COMMAND", 10.0, c(look::GRAY_500))
                                    .weight(500)
                                    .tracking(0.05)
                                    .line_height(15.0)
                                    .silent(),
                            ),
                    )
                    .child(
                        div()
                            .row()
                            .items_center()
                            .gap(8.0)
                            .child(
                                div()
                                    .row()
                                    .items_center()
                                    .gap(8.0)
                                    .grow()
                                    .min_w0()
                                    .px(12.0)
                                    .py(8.0)
                                    .rounded(6.0)
                                    .bg(white(0.03))
                                    .border(1.0, white(0.04))
                                    .child(
                                        text("$", 12.0, c(look::GRAY_600))
                                            .line_height(16.0)
                                            .silent(),
                                    )
                                    .child(
                                        text(a.install, 12.0, c(look::GRAY_300))
                                            .mono()
                                            .line_height(16.0)
                                            .silent(),
                                    ),
                            )
                            .child(square(copy, copy_icon, "Copy install command"))
                            .child(square(
                                docs,
                                ico("external-link", 14.0, 2.0, c(look::GRAY_500)),
                                "View documentation",
                            )),
                    )
                    .child(
                        text(
                            "After installing, click Re-detect above to enable this agent.",
                            10.0,
                            c(look::GRAY_600),
                        )
                        .line_height(15.0)
                        .mt(8.0)
                        .silent(),
                    ),
            ),
        );
    }
    card
}

pub(super) fn page(s: &Screen<'_>) -> El {
    let installed = |k: &str| {
        s.st.installed
            .as_ref()
            .is_none_or(|l| l.iter().any(|x| x == k))
    };
    let have: Vec<_> = AGENTS.iter().filter(|a| installed(a.key)).collect();
    let missing: Vec<_> = AGENTS.iter().filter(|a| !installed(a.key)).collect();
    let busy = s.st.settings.detecting;
    let re = s.acts.on("agents-redetect", set(SetAct::Redetect));
    let h = s.ui.hover(re);
    let mut redetect = div()
        .id(re)
        .row()
        .items_center()
        .gap(6.0)
        .px(12.0)
        .py(6.0)
        .rounded(8.0)
        .bg(white(0.04).mix(white(0.08), h))
        .border(1.0, white(0.06).mix(white(0.1), h))
        .role(Role::Button, "Re-detect installed agents")
        .child(ico(
            if busy { "loader-circle" } else { "refresh-cw" },
            11.0,
            2.0,
            c(look::GRAY_400).mix(c(look::WHITE), h),
        ))
        .child(
            text(
                if busy { "Scanning..." } else { "Re-detect" },
                12.0,
                c(look::GRAY_400).mix(c(look::WHITE), h),
            )
            .line_height(16.0)
            .silent(),
        );
    redetect = if busy {
        redetect.opacity(0.4)
    } else {
        redetect.on(Action::Click)
    };
    let redetect = crate::ui::tooltip(s.ui, redetect, re, "Re-detect installed agents", None);
    let mut status = div()
        .row()
        .items_center()
        .gap(16.0)
        .px(16.0)
        .py(10.0)
        .mb(24.0)
        .rounded(8.0)
        .border(1.0, white(0.04))
        .bg(c(look::SURFACE_SUNKEN))
        .child(
            div()
                .row()
                .items_center()
                .gap(8.0)
                .child(div().size(8.0, 8.0).rounded(4.0).bg(c(0x00d492)))
                .child(
                    text(have.len().to_string(), 12.0, c(look::GRAY_200))
                        .weight(500)
                        .line_height(16.0)
                        .silent(),
                )
                .child(
                    text("installed", 12.0, c(look::GRAY_400))
                        .line_height(16.0)
                        .silent(),
                ),
        );
    if !missing.is_empty() {
        status = status.child(
            div()
                .row()
                .items_center()
                .gap(8.0)
                .child(div().size(8.0, 8.0).rounded(4.0).bg(c(look::GRAY_600)))
                .child(
                    text(missing.len().to_string(), 12.0, c(look::GRAY_400))
                        .weight(500)
                        .line_height(16.0)
                        .silent(),
                )
                .child(
                    text("available", 12.0, c(look::GRAY_400))
                        .line_height(16.0)
                        .silent(),
                ),
        );
    }
    status = status.child(div().grow()).child(
        text(format!("{} AGENTS", AGENTS.len()), 10.0, c(look::GRAY_600))
            .tracking(0.05)
            .line_height(15.0)
            .silent(),
    );
    let mut col = div()
        .col()
        .child(
            div()
                .row()
                .items_center()
                .justify_between()
                .mb(4.0)
                .child(
                    text("Coding Agents", 20.0, c(look::WHITE))
                        .weight(600)
                        .tracking(-0.025)
                        .line_height(28.0)
                        .role(Role::Heading, "Coding Agents"),
                )
                .child(redetect),
        )
        .child(
            text(
                "Configure the command and arguments for each coding agent",
                14.0,
                c(look::GRAY_500),
            )
            .line_height(20.0)
            .mb(24.0)
            .silent(),
        )
        .child(status);
    if !have.is_empty() {
        let mut list = div().col().gap(8.0).mb(32.0);
        for a in have {
            list = list.child(installed_card(s, a));
        }
        col = col.child(list);
    }
    if !missing.is_empty() {
        let mut list = div().col().gap(6.0);
        for a in missing {
            list = list.child(available_card(s, a));
        }
        col = col
            .child(
                div()
                    .row()
                    .items_center()
                    .gap(8.0)
                    .mb(12.0)
                    .child(ico("download", 12.0, 2.0, c(look::GRAY_600)))
                    .child(
                        text("AVAILABLE TO INSTALL", 12.0, c(look::GRAY_500))
                            .weight(500)
                            .tracking(0.05)
                            .line_height(16.0)
                            .role(Role::Heading, "Available to Install"),
                    ),
            )
            .child(list);
    }
    col
}

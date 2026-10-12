//! `WorktreeSettings`: what every worktree costs on disk, the summary card,
//! each project's worktrees and stale branches, and the selection bar.

use super::{header, set};
use crate::look;
use crate::screen::{c, ico, white, Screen};
use crate::settings::{
    format_bytes, selection_warnings, totals, verdict_style, SetAct, WtEntry, WtProject,
};
use crate::ui::{div, text, Action, El, Rgba, Role};

fn small(t: impl Into<String>, color: Rgba) -> El {
    text(t, 11.0, color).line_height(16.5).silent()
}

/// A text button that turns lighter on hover.
fn link(s: &Screen<'_>, place: &str, label: &str, act: SetAct, enabled: bool) -> El {
    let id = s.acts.on(place, set(act));
    let hot = s.ui.hover(id);
    let el = div()
        .id(id)
        .role(Role::Button, label)
        .child(small(label, c(look::GRAY_500).mix(c(look::GRAY_300), hot)));
    if enabled {
        el.on(Action::Click)
    } else {
        el.opacity(0.4)
    }
}

fn rescan(s: &Screen<'_>) -> El {
    let page = &s.st.settings.worktrees;
    let enabled = !page.loading && !page.busy;
    let id = s.acts.on("wt-rescan", set(SetAct::Rescan));
    let hot = s.ui.hover(id);
    let el = div()
        .id(id)
        .row()
        .items_center()
        .gap(6.0)
        .px(12.0)
        .py(6.0)
        .rounded(8.0)
        .shrink0()
        .bg(white(0.04).mix(white(0.08), hot))
        .role(Role::Button, "Rescan")
        .child(ico(
            if page.loading {
                "loader-circle"
            } else {
                "refresh-cw"
            },
            13.0,
            2.0,
            c(look::GRAY_300),
        ))
        .child(small("Rescan", c(look::GRAY_300)));
    if enabled {
        el.on(Action::Click)
    } else {
        el.opacity(0.5)
    }
}

fn summary(s: &Screen<'_>, projects: &[WtProject]) -> El {
    let page = &s.st.settings.worktrees;
    let (disk, artifacts, removable) = totals(projects);
    let art_pct = if disk > 0.0 {
        (artifacts / disk).min(1.0)
    } else {
        0.0
    };
    let rem_pct = if disk > 0.0 {
        (removable / disk).min(1.0 - art_pct)
    } else {
        0.0
    };
    let mut card = div()
        .col()
        .p(20.0)
        .mb(24.0)
        .rounded(12.0)
        .border(1.0, white(0.06))
        .bg(white(0.02))
        .child(
            div()
                .row()
                .items_end()
                .justify_between()
                .gap(24.0)
                .child(
                    div()
                        .col()
                        .child(
                            div()
                                .row()
                                .items_center()
                                .gap(6.0)
                                .child(ico("hard-drive", 12.0, 2.0, c(look::GRAY_500)))
                                .child(
                                    text("ON DISK", 11.0, c(look::GRAY_500))
                                        .tracking(0.05)
                                        .line_height(16.5)
                                        .silent(),
                                ),
                        )
                        .child(
                            text(format_bytes(disk), 30.0, c(look::WHITE))
                                .weight(600)
                                .line_height(38.0)
                                .mt(4.0)
                                .role(Role::Label, "On disk"),
                        ),
                )
                .child(
                    div()
                        .col()
                        .items_end()
                        .child(
                            text(
                                "BUILD OUTPUT \u{2014} REBUILT BY A REINSTALL",
                                11.0,
                                c(look::GRAY_500),
                            )
                            .tracking(0.05)
                            .line_height(16.5)
                            .silent(),
                        )
                        .child(
                            text(format_bytes(artifacts), 20.0, c(0x8ec5ff))
                                .weight(600)
                                .line_height(28.0)
                                .mt(4.0)
                                .silent(),
                        ),
                ),
        )
        .child(
            div()
                .row()
                .mt(16.0)
                .h(8.0)
                .rounded(4.0)
                .clip()
                .bg(white(0.06))
                .child(
                    div()
                        .h_full()
                        .w_pct(art_pct as f32)
                        .bg(Rgba::hexa(0x51a2ff, 0.7)),
                )
                .child(
                    div()
                        .h_full()
                        .w_pct(rem_pct as f32)
                        .bg(Rgba::hexa(0x51a2ff, 0.25)),
                ),
        )
        .child(
            div()
                .row()
                .gap(16.0)
                .mt(8.0)
                .child(small("Build output", c(look::GRAY_500)))
                .child(small("Removable worktrees", c(look::GRAY_500)))
                .child(div().grow())
                .child(small(
                    format!("{} in merged worktrees", format_bytes(removable)),
                    c(look::GRAY_500),
                )),
        );
    if artifacts > 0.0 {
        let confirming = page.confirming == Some(SetAct::AskReclaim);
        let n = page.reclaim_targets().len();
        let label = if confirming {
            format!(
                "Delete build output in {n} worktree{} \u{2014} frees {}",
                if n == 1 { "" } else { "s" },
                format_bytes(artifacts)
            )
        } else {
            format!("Reclaim {} of build output", format_bytes(artifacts))
        };
        let act = if confirming {
            SetAct::Reclaim
        } else {
            SetAct::AskReclaim
        };
        let id = s.acts.on("wt-reclaim", set(act));
        let hot = s.ui.hover(id);
        let mut b = div()
            .id(id)
            .row()
            .items_center()
            .gap(8.0)
            .px(12.0)
            .py(8.0)
            .rounded(6.0)
            .role(Role::Button, label.as_str());
        let fg = if confirming {
            b = b
                .bg(Rgba::hexa(0x2b7fff, 0.25 + 0.1 * hot))
                .border(1.0, Rgba::hexa(0x2b7fff, 0.3));
            c(look::WHITE)
        } else {
            b = b.bg(Rgba::hexa(0x2b7fff, 0.12 + 0.08 * hot));
            c(0x8ec5ff)
        };
        b = b
            .child(ico("trash-2", 13.0, 2.0, fg))
            .child(small(label.clone(), fg));
        b = if page.loading || page.busy {
            b.opacity(0.5)
        } else {
            b.on(Action::Click)
        };
        let mut actions = div().row().items_center().gap(8.0).mt(16.0).child(b);
        if confirming {
            let cancel = s.acts.on("wt-reclaim-cancel", set(SetAct::AskReclaim));
            let hot = s.ui.hover(cancel);
            actions = actions.child(
                div()
                    .id(cancel)
                    .on(Action::Click)
                    .px(12.0)
                    .py(8.0)
                    .role(Role::Button, "Cancel")
                    .child(small("Cancel", c(look::GRAY_400).mix(c(look::WHITE), hot))),
            );
        }
        card = card.child(actions);
    }
    card
}

fn checkbox(s: &Screen<'_>, e: &WtEntry, checked: bool) -> El {
    let id = s
        .acts
        .on("wt-check", set(SetAct::ToggleWorktree(e.path.clone())));
    let mut b = div()
        .id(id)
        .size(13.0, 13.0)
        .shrink0()
        .rounded(3.0)
        .role(Role::CheckBox, format!("Select {}", e.name))
        .toggled(checked);
    b = if checked {
        b.bg(c(0x2b7fff))
            .child(ico("check", 11.0, 3.0, c(look::WHITE)).ml(1.0).mt(1.0))
    } else {
        b.bg(c(0xffffff)).border(1.0, c(0x767676))
    };
    if e.selectable() {
        b.on(Action::Click)
    } else {
        b
    }
}

fn worktree_row(s: &Screen<'_>, e: &WtEntry, last: bool) -> El {
    let page = &s.st.settings.worktrees;
    let (label, fg, fill, alpha) = verdict_style(&e.level);
    let mut meta = div()
        .row()
        .items_center()
        .gap(8.0)
        .mt(4.0)
        .min_w0()
        .child(
            text(
                e.branch.clone().unwrap_or_else(|| "no branch".into()),
                11.0,
                c(look::GRAY_600),
            )
            .mono()
            .line_height(16.5)
            .truncate()
            .silent(),
        )
        .child(small("\u{b7}", c(look::GRAY_600)))
        .child(small(e.idle(), c(look::GRAY_600)));
    if !e.reasons.is_empty() {
        meta = meta.child(small("\u{b7}", c(look::GRAY_600))).child(
            text(e.reasons.join(", "), 11.0, c(look::GRAY_600))
                .line_height(16.5)
                .truncate()
                .silent(),
        );
    }
    let mut size = div().col().w(86.0).shrink0().items_end().child(small(
        if e.measured {
            format_bytes(e.size)
        } else {
            "\u{2014}".to_owned()
        },
        c(look::GRAY_300),
    ));
    if e.artifacts > 0.0 {
        let pct = if e.size > 0.0 {
            e.artifacts / e.size
        } else {
            0.0
        };
        size = size.child(
            div()
                .mt(4.0)
                .h(4.0)
                .w_full()
                .rounded(2.0)
                .clip()
                .bg(white(0.06))
                .child(
                    div()
                        .h_full()
                        .w_pct(pct as f32)
                        .bg(Rgba::hexa(0x51a2ff, 0.6)),
                ),
        );
    }
    let mut r = div()
        .row()
        .items_center()
        .gap(12.0)
        .py(10.0)
        .child(checkbox(s, e, page.selected.contains(&e.path)))
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
                        .min_w0()
                        .child(ico("folder-git-2", 13.0, 2.0, c(look::GRAY_600)))
                        .child(
                            text(&e.name, 13.0, c(look::GRAY_200))
                                .line_height(19.5)
                                .truncate()
                                .silent(),
                        )
                        .child(
                            div()
                                .px(6.0)
                                .py(2.0)
                                .rounded(4.0)
                                .shrink0()
                                .bg(Rgba::hexa(fill, alpha))
                                .child(text(label, 10.0, c(fg)).line_height(15.0).silent()),
                        ),
                )
                .child(meta),
        )
        .child(size);
    if !last {
        r = r.border_bottom(1.0, white(0.04));
    }
    if !e.selectable() {
        r = r.opacity(0.5);
    }
    r
}

fn stale_strip(s: &Screen<'_>, p: &WtProject) -> El {
    let page = &s.st.settings.worktrees;
    let expanded = page.expanded.contains(&p.path);
    let merged: Vec<_> = p.stale.iter().filter(|b| b.merged).collect();
    let chosen = p
        .stale
        .iter()
        .filter(|b| page.branches.contains(&(p.path.clone(), b.name.clone())))
        .count();
    let n = p.stale.len();
    let mut line = format!(
        "{n} branch{} left behind by removed worktrees",
        if n == 1 { "" } else { "es" }
    );
    if !merged.is_empty() {
        line.push_str(&format!(" \u{2014} {} already merged", merged.len()));
    }
    let mut strip = div()
        .col()
        .mt(12.0)
        .px(12.0)
        .py(10.0)
        .rounded(8.0)
        .border(1.0, white(0.06))
        .bg(white(0.02))
        .child(
            div()
                .row()
                .items_center()
                .gap(8.0)
                .child(ico("git-branch", 13.0, 2.0, c(look::GRAY_600)))
                .child(small(line, c(look::GRAY_400)))
                .child(div().grow())
                .child(link(
                    s,
                    "wt-branches",
                    if expanded { "Hide" } else { "Show" },
                    SetAct::ExpandBranches(p.path.clone()),
                    true,
                )),
        );
    if !expanded {
        return strip;
    }
    let mut chips = div().row().wrap_children().gap(6.0).mt(10.0);
    for b in &p.stale {
        let on = page.branches.contains(&(p.path.clone(), b.name.clone()));
        let id = s.acts.on(
            "wt-branch",
            set(SetAct::ToggleBranch(p.path.clone(), b.name.clone())),
        );
        let hot = s.ui.hover(id);
        let (border, bg, fg) = if on {
            (
                Rgba::hexa(0xff6467, 0.4),
                Rgba::hexa(0xfb2c36, 0.15),
                c(0xffc9c9),
            )
        } else if b.merged {
            (
                white(0.08),
                white(0.03),
                c(look::GRAY_400).mix(c(look::WHITE), hot),
            )
        } else {
            (
                Rgba::hexa(0xfe9a00, 0.25),
                Rgba::hexa(0xfe9a00, 0.06),
                Rgba::hexa(0xffd230, 0.8),
            )
        };
        let tip = if b.merged {
            "Merged \u{2014} deleting loses nothing"
        } else {
            "Not merged \u{2014} git will refuse to delete it"
        };
        let chip = div()
            .id(id)
            .on(Action::Click)
            .px(8.0)
            .py(4.0)
            .rounded(4.0)
            .border(1.0, border)
            .bg(bg)
            .role(Role::Button, b.name.as_str())
            .child(text(&b.name, 11.0, fg).mono().line_height(16.5).silent());
        chips = chips.child(crate::ui::tooltip(s.ui, chip, id, tip, None));
    }
    let del = s.acts.on(
        "wt-branch-delete",
        set(SetAct::DeleteBranches(p.path.clone())),
    );
    let hot = s.ui.hover(del);
    let mut del_btn = div()
        .id(del)
        .row()
        .items_center()
        .gap(6.0)
        .px(10.0)
        .py(4.0)
        .rounded(6.0)
        .bg(Rgba::hexa(0xfb2c36, 0.1 + 0.1 * hot))
        .role(Role::Button, "Delete branches")
        .child(ico("trash-2", 11.0, 2.0, c(0xffa2a2)))
        .child(small(
            format!(
                "Delete {}branch{}",
                if chosen > 0 {
                    format!("{chosen} ")
                } else {
                    String::new()
                },
                if chosen == 1 { "" } else { "es" }
            ),
            c(0xffa2a2),
        ));
    del_btn = if chosen == 0 || page.busy {
        del_btn.opacity(0.4)
    } else {
        del_btn.on(Action::Click)
    };
    strip = strip.child(chips).child(
        div()
            .row()
            .items_center()
            .gap(8.0)
            .mt(12.0)
            .child(link(
                s,
                "wt-merged",
                &format!("Select {} merged", merged.len()),
                SetAct::SelectMerged(p.path.clone()),
                !merged.is_empty(),
            ))
            .child(div().grow())
            .child(del_btn),
    );
    strip.child(
        div()
            .row()
            .mt(8.0)
            .child(
                text("Deletion uses ", 10.0, c(look::GRAY_600))
                    .line_height(15.0)
                    .silent(),
            )
            .child(
                text("git branch -d", 10.0, c(look::GRAY_600))
                    .mono()
                    .line_height(15.0)
                    .silent(),
            )
            .child(
                text(
                    ", so git refuses anything that isn't merged.",
                    10.0,
                    c(look::GRAY_600),
                )
                .line_height(15.0)
                .silent(),
            ),
    )
}

fn project(s: &Screen<'_>, p: &WtProject) -> Option<El> {
    let mut entries: Vec<&WtEntry> = p.entries.iter().filter(|e| !e.main).collect();
    entries.sort_by(|a, b| b.size.total_cmp(&a.size));
    if entries.is_empty() && p.stale.is_empty() && p.error.is_none() {
        return None;
    }
    let total: f64 = p.entries.iter().map(|e| e.size).sum();
    let mut title = div().row().items_end().gap(8.0).min_w0().child(
        text(&p.name, 13.0, c(look::GRAY_200))
            .weight(500)
            .line_height(19.5)
            .truncate()
            .role(Role::Heading, p.name.as_str()),
    );
    if let Some(b) = &p.default_branch {
        title = title.child(
            text(format!("vs {b}"), 11.0, c(look::GRAY_600))
                .mono()
                .line_height(16.5)
                .silent(),
        );
    }
    if p.remote {
        title = title.child(small("remote", c(look::GRAY_600)));
    }
    let mut col = div().col().mb(24.0).child(
        div()
            .row()
            .items_end()
            .justify_between()
            .gap(12.0)
            .pb(8.0)
            .border_bottom(1.0, white(0.06))
            .child(title)
            .child(small(format_bytes(total), c(look::GRAY_500))),
    );
    if let Some(e) = &p.error {
        col = col.child(div().py(12.0).child(small(
            format!("Could not scan \u{2014} {e}"),
            c(look::GRAY_500),
        )));
    }
    let n = entries.len();
    for (i, e) in entries.into_iter().enumerate() {
        col = col.child(worktree_row(s, e, i + 1 == n));
    }
    if !p.stale.is_empty() {
        col = col.child(stale_strip(s, p));
    }
    Some(col)
}

pub(super) fn page(s: &Screen<'_>) -> El {
    let page = &s.st.settings.worktrees;
    let projects = page.projects.clone().unwrap_or_default();
    let mut col = div()
        .col()
        .pb(96.0)
        .child(header(
            "Worktrees",
            "What every worktree costs on disk, and what can safely go",
            Some(rescan(s)),
        ))
        .child(summary(s, &projects));
    if page.loading && page.projects.is_none() {
        col = col.child(
            div()
                .row()
                .items_center()
                .justify_center()
                .gap(8.0)
                .py(40.0)
                .child(ico("loader-circle", 15.0, 2.0, c(look::GRAY_500)))
                .child(
                    text("Measuring worktrees\u{2026}", 13.0, c(look::GRAY_500))
                        .line_height(19.5)
                        .silent(),
                ),
        );
    }
    let anything = projects
        .iter()
        .any(|p| !p.entries.is_empty() || !p.stale.is_empty());
    if !page.loading && !anything {
        col = col.child(
            div().row().justify_center().py(40.0).child(
                text(
                    "No worktrees yet. They appear here as soon as you create one.",
                    13.0,
                    c(look::GRAY_500),
                )
                .line_height(19.5)
                .silent(),
            ),
        );
    }
    for p in &projects {
        col = col.child_if(project(s, p));
    }
    if !page.suggested().is_empty() {
        col = col.child(div().mt(16.0).child(link(
            s,
            "wt-suggested",
            "Select everything merged and idle",
            SetAct::SelectSuggested,
            true,
        )));
    }
    col
}

/// The floating bar while worktrees are selected.
pub(super) fn selection_bar(s: &Screen<'_>) -> Option<El> {
    let page = &s.st.settings.worktrees;
    let chosen = page.chosen();
    if chosen.is_empty() {
        return None;
    }
    let bytes: f64 = chosen.iter().map(|e| e.size).sum();
    let warnings = selection_warnings(&chosen);
    let confirming = page.confirming == Some(SetAct::AskRemove);
    let count = chosen.len();
    let mut left = div().col().min_w0().child(
        div()
            .row()
            .child(
                text(format!("{count} selected \u{b7} "), 13.0, c(look::WHITE))
                    .line_height(19.5)
                    .silent(),
            )
            .child(
                text(
                    format!("frees {}", format_bytes(bytes)),
                    13.0,
                    c(look::GRAY_300),
                )
                .line_height(19.5)
                .silent(),
            ),
    );
    if !warnings.is_empty() {
        left = left.child(
            div()
                .row()
                .gap(6.0)
                .mt(4.0)
                .child(
                    div()
                        .mt(2.0)
                        .child(ico("triangle-alert", 11.0, 2.0, c(look::AMBER_400))),
                )
                .child(
                    text(warnings.join(" \u{b7} "), 11.0, Rgba::hexa(0xffd230, 0.85))
                        .line_height(16.5)
                        .wrapping()
                        .silent(),
                ),
        );
    }
    let clear = s.acts.on("wt-clear", set(SetAct::ClearSelection));
    let act = if confirming {
        SetAct::Remove
    } else {
        SetAct::AskRemove
    };
    let remove = s.acts.on("wt-remove", set(act));
    let hot = s.ui.hover(remove);
    let label = if confirming {
        format!(
            "Yes \u{2014} remove {count} and free {}",
            format_bytes(bytes)
        )
    } else {
        "Remove".to_owned()
    };
    let (bg, border, fg) = if confirming {
        (
            Rgba::hexa(0xfb2c36, 0.3 + 0.15 * hot),
            Some(Rgba::hexa(0xff6467, 0.4)),
            c(look::WHITE),
        )
    } else {
        (Rgba::hexa(0xfb2c36, 0.12 + 0.08 * hot), None, c(0xffa2a2))
    };
    let mut rb = div()
        .id(remove)
        .row()
        .items_center()
        .gap(6.0)
        .px(12.0)
        .py(6.0)
        .rounded(6.0)
        .bg(bg)
        .role(Role::Button, label.as_str())
        .child(ico("trash-2", 12.0, 2.0, fg))
        .child(small(label.clone(), fg));
    if let Some(b) = border {
        rb = rb.border(1.0, b);
    }
    rb = if page.busy {
        rb.opacity(0.5)
    } else {
        rb.on(Action::Click)
    };
    let mut bar = div()
        .col()
        .w((s.size.0 - 64.0).min(560.0))
        .px(16.0)
        .py(12.0)
        .rounded(12.0)
        .border(1.0, white(0.1))
        .bg(c(look::SURFACE_OVERLAY))
        .role(Role::Region, "Selected worktrees")
        .child(
            div()
                .row()
                .items_center()
                .gap(12.0)
                .child(left)
                .child(div().grow())
                .child(
                    div()
                        .row()
                        .items_center()
                        .gap(8.0)
                        .shrink0()
                        .child(
                            div()
                                .id(clear)
                                .on(Action::Click)
                                .px(10.0)
                                .py(6.0)
                                .role(Role::Button, "Clear")
                                .child(small(
                                    "Clear",
                                    c(look::GRAY_400).mix(c(look::WHITE), s.ui.hover(clear)),
                                )),
                        )
                        .child(rb),
                ),
        );
    if confirming {
        bar = bar.child(small("This cannot be undone.", c(look::GRAY_500)).mt(8.0));
    }
    Some(
        div()
            .absolute_bottom()
            .row()
            .justify_center()
            .pb(24.0)
            .child(bar),
    )
}

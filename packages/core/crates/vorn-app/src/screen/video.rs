//! A tree's `video` part: a simulator's screen in its bezel, the title that
//! opens the picker, the extension's tools and controls, and the zoom. The
//! extension decides what is shown and hears what is pressed; the frames
//! come from this app, which takes them itself (see `crate::video`).

use serde_json::{json, Value};
use vorn_components::tree::{tone, Ev, TreeBind};
use vorn_tree::{Event, EventKind, Picker, Tool, Video};

use super::points::ScopedBinder;
use super::{c, ico, white, Screen};
use crate::look;
use crate::panes::stage_id;
use crate::ui::{custom, div, text, tooltip, Action, El, Id, Place, Role, Sense};
use crate::video::{frame_metrics, stepped_zoom, ZOOM_MAX, ZOOM_MIN};

/// What the part's controls tell its extension, all as events on `node`.
struct Tell<'a, 'b, 'c> {
    b: &'a ScopedBinder<'b, 'c>,
    node: &'a str,
}

impl Tell<'_, '_, '_> {
    fn send(&self, kind: EventKind, value: impl Into<String>) -> Id {
        let event = Event::new(self.node, kind, value);
        self.b.on(Ev::Send { event, ask: false })
    }

    /// A `page` event, `{type, ...}`.
    fn page(&self, what: Value) -> Id {
        self.send(EventKind::Page, what.to_string())
    }
}

/// `p-1 rounded hover:bg-white/10`: a tool's or control's button, read
/// out as `label`, its tooltip `tip`.
fn icon_button(
    s: &Screen<'_>,
    id: Id,
    icon: &str,
    label: &str,
    tip: &str,
    on: bool,
    dim: bool,
) -> El {
    let hot = s.ui.hover(id);
    let bg = if on {
        white(0.06)
    } else {
        white(0.0).mix(white(0.10), hot)
    };
    let b = div()
        .id(id)
        .on(Action::Click)
        .p(4.0)
        .rounded(4.0)
        .shrink0()
        .bg(bg)
        .role(Role::Button, label)
        .child(ico(icon, 14.0, 2.0, c(look::INK)))
        .toggled(on);
    let b = if dim { b.opacity(0.4) } else { b };
    tooltip(s.ui, b, id, tip, None)
}

/// A tool, after its divider if it has one; busy, it spins and is dimmed.
fn tool_button(s: &Screen<'_>, tell: &Tell<'_, '_, '_>, t: &Tool) -> [Option<El>; 2] {
    let id = tell.send(EventKind::Click, t.id.as_str());
    let icon = if t.busy {
        "loader-circle"
    } else {
        t.icon.as_str()
    };
    let tip = t.tip.as_deref().unwrap_or(&t.label);
    let b = icon_button(s, id, icon, &t.label, tip, t.on, t.busy);
    [t.divider.then(divider), Some(b)]
}

/// The line between groups of controls.
fn divider() -> El {
    div().w(1.0).h(14.0).mx(4.0).bg(white(0.08))
}

/// Video part `v` of the tree, drawn for the session it names.
pub(crate) fn part(b: &ScopedBinder<'_, '_>, node: &str, v: &Video) -> El {
    let s = b.s;
    let sid = v.view.as_str();
    let tell = Tell { b, node };
    let name = if v.title.is_empty() {
        "Simulator"
    } else {
        v.title.as_str()
    };

    let tid = tell.page(json!({ "type": "switch" }));
    let mut title = div()
        .id(tid)
        .on(Action::Click)
        .row()
        .items_center()
        .gap(4.0)
        .min_w0()
        .px(4.0)
        .py(2.0)
        .rounded(4.0)
        .bg(white(0.0).mix(white(0.06), s.ui.hover(tid)))
        .role(Role::Button, format!("Switch simulator, currently {name}"))
        .has_popup()
        .expanded(v.picker.is_some())
        .child(
            text(name, 11.0, c(look::GRAY_300))
                .weight(500)
                .truncate()
                .silent(),
        )
        .child(if v.busy {
            ico("loader-circle", 10.0, 2.5, c(look::GRAY_300))
        } else {
            ico("chevron-down", 10.0, 2.5, c(look::GRAY_500))
        });
    if let Some(p) = &v.picker {
        title = title.overlay(picker(s, &tell, p), Place::BelowStart, 4.0);
    }
    let title = tooltip(s.ui, title, tid, "Switch simulator", None);
    let tools = v
        .tools
        .iter()
        .flat_map(|t| tool_button(s, &tell, t))
        .flatten()
        .collect();
    let header = div()
        .row()
        .items_center()
        .gap(6.0)
        .px(8.0)
        .py(4.0)
        .shrink0()
        .child(ico("smartphone", 12.0, 2.0, c(look::GRAY_500)))
        .child(title)
        .child(div().grow())
        .child(b.chrome(node, tools));

    let notice = v.notice.as_deref().map(|e| {
        let id = tell.page(json!({ "type": "dismiss" }));
        div()
            .row()
            .gap(4.0)
            .px(8.0)
            .py(4.0)
            .shrink0()
            .child(
                text(e, 10.0, look::AMBER_400.alpha(0.9))
                    .wrapping()
                    .min_w0()
                    .grow()
                    .role(Role::Alert, e),
            )
            .child(
                div()
                    .id(id)
                    .on(Action::Click)
                    .p(2.0)
                    .rounded(4.0)
                    .shrink0()
                    .role(Role::Button, "Dismiss error")
                    .child(ico(
                        "x",
                        10.0,
                        2.5,
                        c(look::GRAY_500).mix(c(look::GRAY_200), s.ui.hover(id)),
                    )),
            )
    });

    let bench = &s.st.bench;
    let area = s.ui.rect_of(stage_id(sid)).map(|r| (r.w, r.h));
    let (scale, points) = bench.video_scale(sid, v.zoom, area);
    let mut stage = div()
        .id(stage_id(sid))
        // Sensed so the box has a measured rect to fit the zoom against.
        .sense(Sense::HOVER)
        .grow()
        .min_h(0.0)
        .min_w0()
        .clip()
        .center()
        .p(8.0)
        .bg(c(look::SURFACE_PANEL));
    let shown = bench
        .video_id(sid)
        .filter(|_| !v.boot && !v.busy && v.device.is_some());
    stage = match shown {
        Some(cid) if points.0 > 0.0 => {
            let (w, h) = (
                (points.0 * scale).round() as f32,
                (points.1 * scale).round() as f32,
            );
            let area = area.map(|(aw, ah)| (f64::from(aw), f64::from(ah)));
            let (inset, outer, inner) = frame_metrics((f64::from(w), f64::from(h)), area, points);
            let (inset, outer, inner) = (inset as f32, outer as f32, inner as f32);
            stage.child(
                div()
                    .p(inset)
                    .shrink0()
                    .rounded(outer)
                    .bg(c(look::SURFACE_OVERLAY))
                    .border(1.0, white(0.07))
                    .role(Role::Image, format!("Screen of {name}"))
                    .child(
                        div()
                            .size(w, h)
                            .rounded(inner)
                            .clip()
                            .opacity(if v.notice.is_some() { 0.4 } else { 1.0 })
                            .child(custom(cid).w_full().h_full()),
                    ),
            )
        }
        _ => {
            // 0.8's words: a pane that went wrong says so over its screen.
            let words = match (&v.device, &v.notice) {
                (None, _) => "Choose a simulator",
                (Some(_), Some(_)) => "No frame",
                (Some(_), None) => "Waiting for the device\u{2026}",
            };
            stage.child(text(words, 11.0, c(look::GRAY_500)).wrapping().min_w0())
        }
    };
    div()
        .col()
        .grow()
        .min_h(0.0)
        .min_w0()
        .clip()
        .bg(c(look::SURFACE_PANEL))
        .role(Role::Region, name)
        .child(header)
        .child_if(notice)
        .child(stage)
        .child(control_bar(s, &tell, v, scale))
}

/// The extension's controls, then the zoom.
fn control_bar(s: &Screen<'_>, tell: &Tell<'_, '_, '_>, v: &Video, scale: f64) -> El {
    let zoom = |z: f64| tell.page(json!({ "type": "zoom", "zoom": z }));
    let out = zoom(stepped_zoom(scale, false));
    let into = zoom(stepped_zoom(scale, true));
    let fit = tell.page(json!({ "type": "zoom" }));
    let actual = zoom(1.0);
    let one = div()
        .id(actual)
        .on(Action::Click)
        .p(4.0)
        .rounded(4.0)
        .bg(white(0.0).mix(white(0.10), s.ui.hover(actual)))
        .role(Role::Button, "Show the device at actual size")
        .child(
            text("1:1", 10.0, c(look::INK))
                .weight(500)
                .line_height(14.0)
                .silent(),
        );
    div()
        .row()
        .wrap_children()
        .items_center()
        .justify_center()
        .gap(2.0)
        .px(8.0)
        .py(4.0)
        .shrink0()
        .border_top(1.0, white(0.06))
        .role(Role::Toolbar, "Device controls")
        .children(
            v.controls
                .iter()
                .flat_map(|t| tool_button(s, tell, t))
                .flatten(),
        )
        .child_if((!v.controls.is_empty()).then(divider))
        .child(icon_button(
            s,
            out,
            "minus",
            "Zoom out",
            "Zoom out",
            false,
            scale <= ZOOM_MIN,
        ))
        .child(
            div().w(36.0).row().justify_center().child(
                text(
                    format!("{}%", (scale * 100.0).round()),
                    10.0,
                    c(look::GRAY_500),
                )
                .silent(),
            ),
        )
        .child(icon_button(
            s,
            into,
            "plus",
            "Zoom in",
            "Zoom in",
            false,
            scale >= ZOOM_MAX,
        ))
        .child(icon_button(
            s,
            fit,
            "scan",
            "Fit the device to the pane",
            "Fit the device to the pane",
            v.zoom.is_none(),
            false,
        ))
        .child(tooltip(s.ui, one, actual, "One pixel per point", None))
}

/// The picker of video part `node`, drawn over the card's button that
/// opened it, as 0.8 did: the pane opens only once a simulator is up.
pub(crate) fn card_picker(b: &ScopedBinder<'_, '_>, node: &str, p: &Picker) -> El {
    picker(b.s, &Tell { b, node }, p)
}

/// The devices the title offers.
fn picker(s: &Screen<'_>, tell: &Tell<'_, '_, '_>, p: &Picker) -> El {
    let list = div()
        .col()
        .min_w(240.0)
        .max_h(280.0)
        .rounded(8.0)
        .border(1.0, white(0.08))
        .bg(c(look::SURFACE_OVERLAY))
        .clip()
        .role(Role::ListBox, p.heading.as_str())
        .child(
            div()
                .px(12.0)
                .py(8.0)
                .border_bottom(1.0, white(0.06))
                .child(
                    text(p.heading.to_uppercase(), 10.0, c(look::GRAY_500))
                        .tracking(0.05)
                        .silent(),
                ),
        );
    let state = |el: El| div().px(12.0).py(12.0).row().center().child(el);
    if p.loading {
        return list.child(state(ico("loader-circle", 14.0, 2.0, c(look::GRAY_500))));
    }
    if let Some(e) = &p.error {
        return list.child(state(
            text(e.as_str(), 11.0, look::RED_400.alpha(0.8))
                .wrapping()
                .min_w0()
                .role(Role::Alert, e.as_str()),
        ));
    }
    if p.choices.is_empty() {
        let empty = p.empty.as_deref().unwrap_or_default();
        return list.child(state(text(empty, 12.0, c(look::GRAY_500))));
    }
    let mut rows = div()
        .id(Id::new(tell.b.key).child("video-picker"))
        .col()
        .scroll_y()
        .min_h(0.0);
    for ch in &p.choices {
        let id = tell.send(EventKind::Select, ch.id.as_str());
        let fg = if ch.disabled {
            c(look::GRAY_600)
        } else {
            c(look::GRAY_300)
        };
        let dot = match ch.status {
            Some(t) => tone(t).alpha(0.7),
            None => c(look::GRAY_600),
        };
        let tag = ch
            .badge
            .as_ref()
            .map(|b| text(b.text.as_str(), 9.0, tone(b.tone).alpha(0.6)).silent());
        let mut row = div()
            .id(id)
            .on(Action::Click)
            .row()
            .items_center()
            .gap(8.0)
            .px(12.0)
            .py(6.0)
            .bg(white(0.0).mix(white(0.06), s.ui.hover(id)))
            .role(Role::ListBoxOption, ch.title.as_str())
            .child(ico("smartphone", 11.0, 2.0, dot))
            .child(
                div()
                    .col()
                    .grow()
                    .min_w0()
                    .child(text(ch.title.as_str(), 12.0, fg).truncate().silent())
                    .child_if(
                        ch.detail
                            .as_deref()
                            .map(|t| text(t, 9.0, c(look::GRAY_500)).truncate().silent()),
                    ),
            )
            .child_if(tag);
        if ch.disabled {
            row = row.disabled(true);
        }
        rows = rows.child(row);
    }
    list.child(rows)
}

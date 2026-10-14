//! Extension sections at the foot of the sidebar: each one's template and
//! rows laid out with the kit, on Pixels and on Cells alike. A template or
//! rows that do not check draw a red box naming each error's place, never
//! an empty one.

use super::{c, white, Screen};
use crate::act::Act;
use crate::ext_tiers::Section;
use crate::look;
use crate::ui::{div, text, Action, El, Role};
use vorn_ext::template::{Action as Do, Node, Role as TextRole, Row, Tone};

/// At most this many rows of one list; a long list belongs in a pane.
const MAX_ROWS: usize = 12;

/// Every section, or nothing when no extension is linked.
pub(super) fn sections(s: &Screen<'_>) -> El {
    let mut col = div().col().shrink0();
    for (key, sec) in &s.st.tiers.all {
        col = col.child(section(s, key, sec));
    }
    col
}

fn section(s: &Screen<'_>, key: &str, sec: &Section) -> El {
    let trust = if sec.local {
        "Local, runs as you"
    } else {
        "Sandboxed"
    };
    let head = div()
        .row()
        .items_center()
        .gap(6.0)
        .child(
            div().grow().min_w0().child(
                text(&sec.title, 11.5, c(look::GRAY_200))
                    .weight(600)
                    .line_height(16.0)
                    .truncate()
                    .silent(),
            ),
        )
        .child(
            text(trust, 10.5, c(look::GRAY_500))
                .line_height(14.0)
                .silent(),
        );
    let body = match &sec.body {
        Err(errors) => invalid(errors),
        Ok((t, rows)) => node(s, key, &t.view, None, rows),
    };
    let mut out = div()
        .col()
        .gap(6.0)
        .mx(8.0)
        .mb(6.0)
        .px(10.0)
        .py(8.0)
        .rounded(4.0)
        .border(1.0, white(0.08))
        .bg(white(0.03))
        .role(Role::Group, sec.title.as_str())
        .child(head)
        .child(body);
    if let Some(e) = &sec.error {
        out = out.child(text(e, 10.5, c(look::RED_300)).line_height(14.0).wrapping());
    }
    out
}

/// The red box: every error with its place, so the author can go straight to it.
fn invalid(errors: &[String]) -> El {
    let mut b = div()
        .col()
        .gap(4.0)
        .px(8.0)
        .py(6.0)
        .rounded(4.0)
        .border(1.0, c(look::DANGER))
        .bg(c(look::DANGER).alpha(0.08))
        .role(Role::Alert, "This section's template has errors")
        .child(
            text("Template error", 11.0, c(look::RED_300))
                .weight(600)
                .line_height(14.0),
        );
    for e in errors {
        b = b.child(text(e, 10.5, c(look::RED_300)).line_height(14.0).wrapping());
    }
    b
}

fn tone(t: Tone) -> u32 {
    match t {
        Tone::Good => look::GREEN_400,
        Tone::Bad => look::RED_400,
        Tone::Warn => look::AMBER_400,
        Tone::Busy => look::BRONZO,
        Tone::Muted => look::GRAY_600,
    }
}

fn node(s: &Screen<'_>, key: &str, n: &Node, row: Option<&Row>, rows: &[Row]) -> El {
    match n {
        Node::Column(kids) => div().col().gap(4.0).children(
            kids.iter()
                .map(|k| node(s, key, k, row, rows))
                .collect::<Vec<_>>(),
        ),
        Node::Row(kids) => div().row().items_center().gap(6.0).children(
            kids.iter()
                .map(|k| node(s, key, k, row, rows))
                .collect::<Vec<_>>(),
        ),
        Node::Text { value, role, grow } => {
            let v = value.fill(row, rows);
            let (size, color) = match role {
                TextRole::Title => (11.5, c(look::GRAY_200)),
                TextRole::Body => (11.0, c(look::GRAY_300)),
                TextRole::Secondary => (10.5, c(look::GRAY_500)),
                TextRole::Mono => (10.5, c(look::GRAY_400)),
            };
            let mut t = text(&v, size, color).line_height(15.0).truncate().silent();
            if *role == TextRole::Mono {
                t = t.mono();
            }
            if *grow {
                div().grow().min_w0().child(t)
            } else {
                t
            }
        }
        Node::Status { value, tones } => {
            let v = value.fill(row, rows);
            div()
                .size(6.0, 6.0)
                .rounded(3.0)
                .shrink0()
                .bg(c(tone(Tone::of(tones, &v))))
                .role(Role::Status, v.as_str())
        }
        Node::Badge { value, tones } => {
            let v = value.fill(row, rows);
            text(&v, 10.5, c(tone(Tone::of(tones, &v))))
                .line_height(14.0)
                .silent()
        }
        Node::Button { label, action } => {
            let label = label.fill(row, rows);
            let act = match action {
                Do::Open(url) => Act::OpenUrl(url.fill(row, rows)),
                // Calls into the extension wait for the next stage.
                Do::Call(_) => Act::Nothing,
            };
            let place = format!("ext-section:{key}");
            let id = s.acts.on(&place, act);
            div()
                .id(id)
                .on(Action::Click)
                .shrink0()
                .px(6.0)
                .py(1.0)
                .rounded(4.0)
                .bg(white(0.0).mix(white(0.08), s.ui.hover(id)))
                .border(1.0, white(0.08))
                .role(Role::Button, label.as_str())
                .child(
                    text(&label, 10.5, c(look::GRAY_300))
                        .line_height(14.0)
                        .silent(),
                )
        }
        Node::List { item, empty } => {
            if rows.is_empty() {
                let e = empty
                    .as_ref()
                    .map(|e| e.fill(None, rows))
                    .unwrap_or_default();
                return text(&e, 10.5, c(look::GRAY_500))
                    .line_height(14.0)
                    .wrapping();
            }
            div().col().gap(2.0).children(
                rows.iter()
                    .take(MAX_ROWS)
                    .map(|r| node(s, key, item, Some(r), rows))
                    .collect::<Vec<_>>(),
            )
        }
    }
}

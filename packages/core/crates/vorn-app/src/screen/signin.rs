//! The sheet a connection signs in on (the desktop's `Sign in · <name>`
//! window): its page in the connection's own profile, over the app.

use super::{c, dialogs::backdrop, ico, white, Screen};
use crate::act::Act;
use crate::look;
use crate::ui::{custom, div, text, Action, El, Role};

/// The desktop opened the window at 520 by 720.
const SIZE: (f32, f32) = (520.0, 720.0);

pub(super) fn sheet(s: &Screen<'_>) -> Option<El> {
    let w = s.st.signin.as_ref()?;
    let title = w.title();
    let close = s.acts.on("signin-close", Act::SignInClose);
    let hot = s.ui.hover(close);
    let header = div()
        .row()
        .items_center()
        .justify_between()
        .h(40.0)
        .pl(16.0)
        .pr(8.0)
        .shrink0()
        .border_bottom(1.0, white(0.06))
        .child(
            text(title.as_str(), 13.0, c(look::GRAY_200))
                .weight(500)
                .line_height(18.0)
                .truncate()
                .role(Role::Heading, title.as_str()),
        )
        .child(
            div()
                .id(close)
                .on(Action::Click)
                .ring(4.0)
                .p(6.0)
                .rounded(4.0)
                .bg(white(0.06 * hot))
                .role(Role::Button, "Close")
                .child(ico(
                    "x",
                    14.0,
                    2.0,
                    c(look::GRAY_500).mix(c(look::WHITE), hot),
                )),
        );
    let key = crate::signin::window_key(&w.connection);
    let body = match s.st.bench.slot_id(&key) {
        Some(id) => custom(id).grow().min_h(0.0).w_full(),
        None => {
            let why =
                s.st.bench
                    .unavailable
                    .clone()
                    .unwrap_or_else(|| format!("Opening {}", w.url));
            div().grow().center().p(24.0).child(
                text(why, 12.0, c(look::GRAY_500))
                    .line_height(18.0)
                    .wrapping(),
            )
        }
    };
    let panel = div()
        .col()
        .w(SIZE.0.min(s.size.0 - 64.0))
        .h(SIZE.1.min(s.size.1 - 64.0))
        .bg(c(look::SURFACE_OVERLAY))
        .border(1.0, white(0.08))
        .rounded(8.0)
        .clip()
        .role(Role::Dialog, title.as_str())
        .child(header)
        .child(body);
    let panel = s.acts.click(panel, "signin", Act::Nothing);
    Some(backdrop(s, 0.5).center().child(panel))
}

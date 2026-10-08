//! The look test's screen: the app's sidebar, a session card holding the
//! live pane, and a small diff card, built from GPUI's styled divs with the
//! app's tokens (`src/renderer`), so it can be put side by side with the
//! other prototypes' screenshots.

use std::borrow::Cow;

use gpui::{
    div, prelude::*, px, rgb, rgba, svg, AnyElement, AnyView, App, AssetSource, Context, FontWeight,
    Hsla, Rgba, SharedString, Window,
};

use crate::Root;

// The app's tokens.
const BASE: u32 = 0x0d0d0f;
const PANEL: u32 = 0x101012;
const SUNKEN: u32 = 0x141416;
const OVERLAY: u32 = 0x1c1c20;
const INK: u32 = 0xfaf9f7;
const GRAY300: u32 = 0xd1d5db;
const GRAY400: u32 = 0x9ca3af;
const GRAY500: u32 = 0x6b7280;
const GRAY600: u32 = 0x4b5563;
const LINE: u32 = 0xffffff0f; // white/.06
const HAIRLINE: u32 = 0xffffff0a; // white/.04
const UI: &str = ".SystemUIFont";
const MONO: &str = "Menlo";

/// The shared stroke icons, embedded.
pub struct Icons;

macro_rules! icons {
    ($($name:literal),*) => {
        fn icon_bytes(path: &str) -> Option<&'static [u8]> {
            match path {
                $(concat!("icons/", $name, ".svg") => Some(include_bytes!(concat!("../../look/icons/", $name, ".svg"))),)*
                _ => None,
            }
        }
    };
}
icons!("terminal", "folder", "folder-open", "square-terminal", "globe", "git-branch", "panel-left", "file-diff");

impl AssetSource for Icons {
    fn load(&self, path: &str) -> gpui::Result<Option<Cow<'static, [u8]>>> {
        Ok(icon_bytes(path).map(Cow::Borrowed))
    }
    fn list(&self, _: &str) -> gpui::Result<Vec<SharedString>> {
        Ok(Vec::new())
    }
}

fn icon(name: &str, size: f32, color: u32) -> impl IntoElement {
    svg()
        .path(SharedString::from(format!("icons/{name}.svg")))
        .size(px(size))
        .flex_none()
        .text_color(rgb(color))
}

fn white(a: f32) -> Hsla {
    gpui::hsla(0.0, 0.0, 1.0, a)
}

fn text(s: &'static str, size: f32, color: impl Into<Hsla>) -> gpui::Div {
    div().font_family(UI).text_size(px(size)).text_color(color).child(s)
}

fn session_row(name: &'static str, branch: &'static str, status: Hsla, selected: bool) -> impl IntoElement {
    div()
        .relative()
        .flex()
        .items_center()
        .gap(px(8.0))
        .px(px(8.0))
        .py(px(4.0))
        .child(icon("terminal", 14.0, GRAY400))
        .child(
            div()
                .flex()
                .flex_col()
                .flex_1()
                .child(text(name, 12.0, rgb(if selected { 0xffffff } else { GRAY400 })).line_height(px(16.0)))
                .child(text(branch, 10.0, rgb(GRAY600)).line_height(px(14.0))),
        )
        .child(div().size(px(6.0)).rounded_full().bg(status))
        .when(selected, |d| {
            d.child(div().absolute().left_0().top(px(4.0)).bottom(px(4.0)).w(px(1.0)).bg(rgb(0xffffff)))
        })
}

fn sidebar(polish: bool) -> impl IntoElement {
    let bg: Hsla = if polish { Rgba { r: 0.063, g: 0.063, b: 0.071, a: 0.78 }.into() } else { rgb(PANEL).into() };
    div()
        .w(px(256.0))
        .h_full()
        .flex_none()
        .flex()
        .flex_col()
        .bg(bg)
        .border_r_1()
        .border_color(rgba(LINE))
        .child(
            // The titlebar row; the traffic lights sit on its left.
            div()
                .h(px(40.0))
                .flex()
                .items_center()
                .justify_end()
                .pl(px(80.0))
                .pr(px(12.0))
                .border_b_1()
                .border_color(rgba(LINE))
                .child(div().p(px(4.0)).child(icon("panel-left", 14.0, GRAY500))),
        )
        .child(
            div()
                .px(px(12.0))
                .flex()
                .flex_col()
                .gap(px(2.0))
                .child(
                    text("SESSIONS", 11.0, rgb(GRAY500))
                        .font_weight(FontWeight::MEDIUM)
                        .pt(px(12.0))
                        .pb(px(6.0)),
                )
                .child(
                    div()
                        .flex()
                        .items_center()
                        .gap(px(8.0))
                        .px(px(8.0))
                        .py(px(6.0))
                        .rounded(px(4.0))
                        .bg(white(0.08))
                        .child(icon("folder", 14.0, GRAY500))
                        .child(text("vorn", 13.0, rgb(0xffffff)).flex_1())
                        .child(text("3", 12.0, rgb(GRAY600))),
                )
                .child(
                    div()
                        .pl(px(16.0))
                        .flex()
                        .flex_col()
                        .gap(px(2.0))
                        .child(session_row("fix flaky resume test", "fix/resume-flake", rgb(INK).into(), true))
                        .child(session_row("grid protocol docs", "docs/grid-mode", rgb(0xc9972a).into(), false))
                        .child(session_row("native ui spike", "native-ui-spike", white(0.18), false)),
                ),
        )
}

struct Tip(&'static str);

impl Render for Tip {
    fn render(&mut self, _: &mut Window, _: &mut Context<Self>) -> impl IntoElement {
        div()
            .px(px(8.0))
            .py(px(4.0))
            .rounded(px(4.0))
            .bg(rgb(OVERLAY))
            .border_1()
            .border_color(rgba(LINE))
            .child(text(self.0, 12.0, rgb(INK)))
    }
}

fn icon_button(name: &'static str, tip: &'static str) -> impl IntoElement {
    div()
        .id(tip)
        .p(px(4.0))
        .rounded(px(4.0))
        .hover(|s| s.bg(white(0.10)))
        .tooltip(move |_, cx: &mut App| -> AnyView { cx.new(|_| Tip(tip)).into() })
        .child(icon(name, 14.0, INK))
}

fn card_header(icon_name: &str, title: &'static str, branch: Option<&'static str>, trailing: AnyElement) -> impl IntoElement {
    div()
        .flex()
        .items_center()
        .gap(px(8.0))
        .px(px(12.0))
        .py(px(10.0))
        .border_b_1()
        .border_color(rgba(HAIRLINE))
        .child(icon(icon_name, 18.0, GRAY400))
        .child(text(title, 13.0, rgb(GRAY300)).font_weight(FontWeight::MEDIUM))
        .when_some(branch, |d, b| {
            d.child(
                div()
                    .flex()
                    .items_center()
                    .gap(px(4.0))
                    .child(icon("git-branch", 11.0, GRAY500))
                    .child(div().font_family(MONO).text_size(px(11.0)).text_color(rgb(GRAY400)).child(b)),
            )
        })
        .child(div().flex_1())
        .child(trailing)
}

fn card(header: impl IntoElement, body: impl IntoElement) -> gpui::Div {
    div()
        .flex()
        .flex_col()
        .bg(rgb(SUNKEN))
        .border_1()
        .border_color(rgba(LINE))
        .child(header)
        .child(body)
}

/// (old number, new number, kind, text); kind: ' ', '-', '+'.
const DIFF: &[(Option<u32>, Option<u32>, char, &str)] = &[
    (Some(14), Some(14), ' ', "  broken: 'bg-danger',"),
    (Some(15), Some(15), ' ', "  blocked: 'bg-bronzo',"),
    (Some(16), None, '-', "  settled: 'bg-ink-faint',"),
    (None, Some(16), '+', "  settled: 'bg-ink-faint/80',"),
    (None, Some(17), '+', "  parked: 'bg-ink-ghost',"),
    (Some(17), Some(18), ' ', "  live: 'bg-ink',"),
    (Some(18), Some(19), ' ', "  idle: 'bg-ink-ghost'"),
    (Some(19), Some(20), ' ', "}"),
];

fn mono(s: impl Into<SharedString>, size: f32, color: impl Into<Hsla>) -> gpui::Div {
    div().font_family(MONO).text_size(px(size)).text_color(color).child(s.into())
}

fn diff_view() -> impl IntoElement {
    let gutter = |n: Option<u32>, color: u32| {
        div()
            .w(px(35.0))
            .flex_none()
            .flex()
            .justify_end()
            .pr(px(8.0))
            .child(mono(n.map(|n| n.to_string()).unwrap_or_default(), 11.0, rgb(color)))
    };
    let mut lines = div().flex().flex_col();
    for &(old, new, kind, s) in DIFF {
        let (bg, fg): (Hsla, u32) = match kind {
            '+' => (Rgba { r: 34.0 / 255.0, g: 197.0 / 255.0, b: 94.0 / 255.0, a: 0.10 }.into(), 0x86efac),
            '-' => (Rgba { r: 239.0 / 255.0, g: 68.0 / 255.0, b: 68.0 / 255.0, a: 0.10 }.into(), 0xfca5a5),
            _ => (gpui::transparent_black(), GRAY400),
        };
        lines = lines.child(
            div()
                .flex()
                .items_center()
                .h(px(19.2))
                .bg(bg)
                .child(gutter(old, if kind == '-' { 0xdc2626 } else { GRAY600 }))
                .child(gutter(new, if kind == '+' { 0x16a34a } else { GRAY600 }))
                .child(mono(s, 12.0, rgb(fg))),
        );
    }
    div()
        .flex()
        .flex_col()
        .child(
            div()
                .flex()
                .items_center()
                .gap(px(8.0))
                .px(px(12.0))
                .py(px(6.0))
                .bg(rgb(OVERLAY))
                .border_b_1()
                .border_color(rgba(LINE))
                .child(mono("src/renderer/lib/status-tone.ts", 12.0, rgb(GRAY300)).flex_1())
                .child(mono("+2", 11.0, rgb(0x7ea96a)))
                .child(mono("-1", 11.0, rgb(0xc96f62))),
        )
        .child(
            div()
                .px(px(12.0))
                .py(px(2.0))
                .bg(white(0.05))
                .child(mono("@@ -14,7 +14,8 @@ export const TONE_DOT = {", 12.0, white(0.55))),
        )
        .child(lines)
}

pub fn screen(root: &Root, polish: bool, cx: &mut Context<Root>) -> gpui::Div {
    let buttons = div()
        .flex()
        .gap(px(2.0))
        .child(icon_button("folder-open", "Browse files"))
        .child(icon_button("square-terminal", "Add a terminal"))
        .child(icon_button("globe", "Open browser"));
    let session = card(
        card_header("terminal", "fix flaky resume test", Some("fix/resume-flake"), buttons.into_any_element()),
        div().flex_1().flex().pt(px(2.0)).child(root.pane(0, cx)),
    )
    .w(px(744.0))
    .h_full();
    let changes = card(
        card_header("file-diff", "Changes", None, text("1 file", 11.0, rgb(GRAY500)).into_any_element()),
        diff_view(),
    )
    .w(px(440.0))
    .h(px(450.0));
    div()
        .size_full()
        .flex()
        .flex_row()
        .when(!polish, |d| d.bg(rgb(BASE)))
        .child(sidebar(polish))
        .child(div().flex_1().h_full().flex().bg(rgb(BASE)).child(session).child(changes))
}

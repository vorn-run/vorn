//! Model C: Worktree cleanup's UI as code that returns an element tree; the client lays it out and draws it.

use std::cell::RefCell;
use std::collections::BTreeSet;

use ext_ui_sdk::theme::{self, white};
use ext_ui_sdk::widgets::{self, button};
use ext_ui_sdk::{
    div, shape, text, El, Radius, Role, TextSize, Token, Tone,
};
use serde_json::Value;

ext_ui_sdk::wit_bindgen::generate!({
    world: "ui-c",
    path: "../wit",
    runtime_path: "ext_ui_sdk::wit_bindgen::rt",
    with: { "vorn:ext-ui-poc/tree@0.1.0": ext_ui_sdk::tree },
});

struct Row {
    id: String,
    name: String,
    branch: String,
    status: String,
    size: String,
    bytes: u64,
    removable: bool,
}

impl Row {
    fn of(r: &Value) -> Row {
        let s = |k: &str| r[k].as_str().unwrap_or("").to_owned();
        Row {
            id: s("id"),
            name: s("name"),
            branch: s("branch"),
            status: s("status"),
            size: s("size"),
            bytes: r["bytes"].as_u64().unwrap_or(0),
            removable: r["removable"].as_bool().unwrap_or(false),
        }
    }
}

#[derive(Default)]
struct Screen {
    rows: Vec<Row>,
    selected: BTreeSet<String>,
    anchor: Option<usize>,
    confirming: bool,
    filter: String,
    status: String,
}

const ROW: &str = "worktrees/";

impl Screen {
    fn visible(&self) -> impl Iterator<Item = (usize, &Row)> {
        let q = self.filter.to_lowercase();
        self.rows.iter().enumerate().filter(move |(_, r)| {
            q.is_empty() || r.name.to_lowercase().contains(&q) || r.branch.to_lowercase().contains(&q)
        })
    }

    fn view(&self, s: &Surface) -> El {
        let removable = self.rows.iter().filter(|r| r.removable).count();
        let n = self.selected.len();
        let mut remove = button("remove", format!("Remove {n}"));
        if n == 0 {
            remove = remove.disabled(true).opacity(0.4);
        }
        if self.confirming {
            let msg = format!("Remove {n} worktrees? Their branches stay.");
            remove = widgets::confirm(remove, msg, "Remove", true);
        }
        div()
            .col()
            .gap(12.0)
            .p(16.0)
            .w_full()
            .h_full()
            .bg(theme::BASE)
            .child(
                div()
                    .row()
                    .justify_between()
                    .items_center()
                    .child(
                        div()
                            .col()
                            .gap(2.0)
                            .child(text("Worktree cleanup", TextSize::Sm, theme::INK).weight(600))
                            .child(text(
                                format!("{} worktrees · {removable} can go", self.rows.len()),
                                TextSize::Xs,
                                theme::INK_SECONDARY,
                            )),
                    )
                    .child(
                        div()
                            .row()
                            .items_center()
                            .gap(8.0)
                            .child(self.sparkline(s))
                            .child(widgets::text_input("filter", "Filter", "Filter").w(200.0))
                            .child(remove),
                    ),
            )
            .child(
                widgets::list("worktrees", "Worktrees", self.visible().map(|(i, r)| self.row(i, r)))
                    .grow()
                    .min_h(0.0),
            )
            .child(text(self.status.clone(), TextSize::Xs, theme::INK_SECONDARY))
    }

    fn row(&self, i: usize, r: &Row) -> El {
        let on = self.selected.contains(&r.id);
        let tone = match r.status.as_str() {
            "merged" => Tone::Good,
            "stale" => Tone::Warn,
            "dirty" => Tone::Bad,
            "active" => Tone::Info,
            _ => Tone::Neutral,
        };
        let check = div()
            .w(14.0)
            .h(14.0)
            .rounded(Radius::Sm)
            .border(1.0, white(if on { 0.6 } else { 0.25 }))
            .bg(if on { theme::ACCENT } else { theme::CLEAR })
            .glyph(if on { "☑" } else { "☐" });
        let row = div()
            .id(format!("{ROW}{i}"))
            .row()
            .items_center()
            .gap(12.0)
            .px(8.0)
            .py(6.0)
            .rounded(Radius::Md)
            .on_click()
            .on_drag()
            .opacity(if r.removable { 1.0 } else { 0.55 });
        let row = if on {
            row.bg(white(0.06))
        } else {
            row.bg(theme::CLEAR).hover_bg(white(0.04))
        };
        row.child(check)
            .child(text(r.name.clone(), TextSize::Xs, theme::INK).w(160.0).truncate())
            .child(text(r.branch.clone(), TextSize::Xs, theme::MUTED).mono().grow().truncate())
            .child(widgets::badge(r.status.clone(), tone))
            .child(text(r.size.clone(), TextSize::Xs, theme::INK_SECONDARY).w(64.0))
    }

    /// What a template cannot say: the visible worktrees' sizes as a line, computed here.
    fn sparkline(&self, s: &Surface) -> El {
        let (w, h) = (96.0f32, 20.0f32);
        let sizes: Vec<(u64, &Row)> = self.visible().map(|(_, r)| (r.bytes, r)).collect();
        let bins = if s.cells { 12 } else { 48 }.min(sizes.len().max(1));
        let peak: Vec<u64> = (0..bins)
            .map(|b| {
                let (lo, hi) = (b * sizes.len() / bins, ((b + 1) * sizes.len() / bins).max(b * sizes.len() / bins + 1));
                sizes[lo.min(sizes.len())..hi.min(sizes.len())].iter().map(|x| x.0).max().unwrap_or(0)
            })
            .collect();
        let top = peak.iter().copied().max().unwrap_or(0).max(1) as f32;
        let largest = sizes.iter().max_by_key(|x| x.0).map(|x| format!(", largest {} at {}", x.1.name, x.1.size));
        let name = format!("Sizes of {} worktrees{}", sizes.len(), largest.unwrap_or_default());
        let step = if bins > 1 { w / (bins - 1) as f32 } else { 0.0 };
        let pts: Vec<String> = peak
            .iter()
            .enumerate()
            .map(|(i, v)| format!("{:.1},{:.1}", i as f32 * step, h - 1.0 - *v as f32 / top * (h - 2.0)))
            .collect();
        let svg = format!(
            "<svg xmlns='http://www.w3.org/2000/svg' viewBox='0 0 {w} {h}'><polygon fill-opacity='0.25' points='0,{h} {} {w},{h}'/><polyline fill='none' stroke='#fff' stroke-width='1.5' stroke-linejoin='round' points='{}'/></svg>",
            pts.join(" "),
            pts.join(" ")
        );
        let blocks = ['▁', '▂', '▃', '▄', '▅', '▆', '▇', '█'];
        let glyph: String = peak.iter().map(|v| blocks[((*v as f32 / top) * 7.0).round() as usize]).collect();
        shape(svg, w, h, ext_ui_sdk::theme::tint(Token::Accent, 0.9)).glyph(glyph).access(Role::Image, name)
    }

    fn row_at(id: &str) -> Option<usize> {
        id.strip_prefix(ROW)?.parse().ok()
    }

    fn event(&mut self, id: &str, kind: EventKind, data: &str) -> bool {
        match (kind, id) {
            (EventKind::Change, "filter") => self.filter = data.to_owned(),
            (EventKind::Click, "remove") if !self.selected.is_empty() => self.confirming = true,
            (EventKind::Click, "remove.cancel") => self.confirming = false,
            (EventKind::Click, "remove.ok") => {
                self.confirming = false;
                let ids: Vec<&String> = self.selected.iter().collect();
                let args = serde_json::to_string(&ids).unwrap_or_default();
                self.status = call("remove", &args).unwrap_or_else(|e| e);
                self.selected.clear();
            }
            (EventKind::Click, _) => {
                let Some(r) = Screen::row_at(id).and_then(|i| self.rows.get(i).map(|r| (i, r))) else {
                    return false;
                };
                if !r.1.removable {
                    return false;
                }
                if !self.selected.remove(&r.1.id) {
                    self.selected.insert(r.1.id.clone());
                }
                self.anchor = Some(r.0);
            }
            (EventKind::DragOver, _) => {
                let (Some(a), Some(b)) = (self.anchor, Screen::row_at(data)) else {
                    return false;
                };
                let before = self.selected.len();
                let hi = a.max(b).min(self.rows.len().saturating_sub(1));
                for r in &self.rows[a.min(b)..=hi] {
                    if r.removable {
                        self.selected.insert(r.id.clone());
                    }
                }
                return self.selected.len() != before;
            }
            _ => return false,
        }
        true
    }
}

thread_local! {
    static SCREEN: RefCell<Screen> = RefCell::default();
}

struct Component;

impl Guest for Component {
    fn data(rows: String) {
        let v: Value = serde_json::from_str(&rows).unwrap_or(Value::Null);
        let rows: Vec<Row> = v.as_array().into_iter().flatten().map(Row::of).collect();
        SCREEN.with_borrow_mut(|s| {
            let live: BTreeSet<&str> = rows.iter().map(|r| r.id.as_str()).collect();
            s.selected.retain(|id| live.contains(id.as_str()));
            s.rows = rows;
        });
    }

    fn patch(op: String) {
        let v: Value = serde_json::from_str(&op).unwrap_or(Value::Null);
        let row = Row::of(&v["row"]);
        SCREEN.with_borrow_mut(|s| match s.rows.iter_mut().find(|r| r.id == row.id) {
            Some(r) => *r = row,
            None => s.rows.push(row),
        });
    }

    fn render(surface: Surface) -> Vec<Node> {
        SCREEN.with_borrow(|s| s.view(&surface).flatten())
    }

    fn event(id: String, kind: EventKind, data: String) -> bool {
        SCREEN.with_borrow_mut(|s| s.event(&id, kind, &data))
    }
}

export!(Component);

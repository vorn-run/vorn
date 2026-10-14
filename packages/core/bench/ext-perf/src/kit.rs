//! The kit's board component as a client would ship it: an extension's template and keyed rows, drawn with vornui, drag and scroll kept in the client.

use std::collections::HashMap;

use serde_json::Value;
use vorn_app::look;
use vorn_app::tasks::{short_date, short_id, status as status_of};
use vornui::{div, icon, text, Action, El, Event, Id, Rgba, Sense, Ui};

/// Cards drawn past each end of a column's visible part when it draws only those.
const OVERSCAN: usize = 4;
const GAP: f32 = 8.0;

struct Column {
    key: String,
    label: String,
    icon: String,
    tone: String,
}

/// One board: the template, its rows, and what the client keeps between frames.
pub struct Board {
    columns: Vec<Column>,
    card: Value,
    card_h: f32,
    /// Only the cards a column shows are built, at a fixed height.
    windowed: bool,
    rows: Vec<Value>,
    index: HashMap<String, usize>,
    /// The card being dragged and the column under it.
    dragging: Option<(String, Option<String>)>,
    /// The rows behind this frame's cards, by element.
    cards: HashMap<Id, String>,
    /// Calls the board made, for the host to send.
    pub calls: Vec<(String, Value)>,
}

fn white(a: f32) -> Rgba {
    Rgba::hexa(look::WHITE, a)
}

fn ink(a: f32) -> Rgba {
    Rgba::hexa(look::INK, a)
}

fn tone(name: &str) -> Rgba {
    match name {
        "slate" | "todo" => Rgba::hex(look::STATUS_SLATE),
        "blue" | "in_progress" => Rgba::hex(look::STATUS_BLUE),
        "bronze" | "in_review" => Rgba::hex(look::BRONZO),
        "sage" | "done" => Rgba::hex(look::STATUS_SAGE),
        _ => ink(look::INK_FAINT),
    }
}

fn ico(name: &str, size: f32, color: Rgba) -> El {
    icon(look::icon(name, 2.0).unwrap_or_default(), size, color)
}

fn column_id(key: &str) -> Id {
    Id::new("kit-column").child(key)
}

fn scroll_id(key: &str) -> Id {
    Id::new("kit-column-scroll").child(key)
}

pub fn card_id(row: &str) -> Id {
    Id::new("kit-card").child(row)
}

fn scalar(v: &Value) -> String {
    match v {
        Value::String(s) => s.clone(),
        Value::Null => String::new(),
        other => other.to_string(),
    }
}

impl Board {
    pub fn new(template: &str, windowed: bool) -> Result<Board, String> {
        let t: Value = serde_json::from_str(template).map_err(|e| e.to_string())?;
        let view = &t["view"];
        if view["c"] != "board" {
            return Err(format!("view: expected a board, got {}", view["c"]));
        }
        let text = |c: &Value, k: &str| c[k].as_str().unwrap_or_default().to_owned();
        let columns = view["columns"]
            .as_array()
            .ok_or("view.columns: missing")?
            .iter()
            .map(|c| Column {
                key: text(c, "key"),
                label: text(c, "label"),
                icon: text(c, "icon"),
                tone: text(c, "tone"),
            })
            .collect();
        Ok(Board {
            columns,
            card: view["card"].clone(),
            card_h: view["cardHeight"].as_f64().unwrap_or(92.0) as f32,
            windowed,
            rows: Vec::new(),
            index: HashMap::new(),
            dragging: None,
            cards: HashMap::new(),
            calls: Vec::new(),
        })
    }

    /// Replaces every row, as a first load does.
    pub fn set_rows(&mut self, rows: Vec<Value>) {
        self.rows = rows;
        self.reindex();
    }

    fn reindex(&mut self) {
        self.index = self
            .rows
            .iter()
            .enumerate()
            .map(|(i, r)| (scalar(&r["id"]), i))
            .collect();
    }

    /// One row op: an upsert by id.
    pub fn upsert(&mut self, row: Value) {
        let id = scalar(&row["id"]);
        match self.index.get(&id) {
            Some(&i) => self.rows[i] = row,
            None => {
                self.rows.push(row);
                self.index.insert(id, self.rows.len() - 1);
            }
        }
    }

    /// The text a template string names for `row`: `{row.key}` with an optional `|short`, `|date` or `|icon`.
    fn fill(&self, s: &str, row: &Value, column: &str) -> String {
        let mut out = String::with_capacity(s.len() + 16);
        let mut rest = s;
        while let Some(a) = rest.find('{') {
            out.push_str(&rest[..a]);
            let Some(b) = rest[a..].find('}') else { break };
            let path = &rest[a + 1..a + b];
            let (path, filter) = path.split_once('|').unwrap_or((path, ""));
            let v = match path.strip_prefix("row.") {
                Some(k) => scalar(&row[k]),
                None if path == "column" => column.to_owned(),
                None => String::new(),
            };
            match filter {
                "short" => out.push_str(&short_id(&scalar(&row["projectName"]), &v)),
                "date" => out.push_str(&short_date(&v)),
                "icon" => out.push_str(status_of(&v).icon),
                _ => out.push_str(&v),
            }
            rest = &rest[a + b + 1..];
        }
        out.push_str(rest);
        out
    }

    fn node(&self, n: &Value, row: &Value, hot: f32) -> El {
        let kids = |me: &Board| -> Vec<El> {
            n["children"]
                .as_array()
                .into_iter()
                .flatten()
                .map(|c| me.node(c, row, hot))
                .collect()
        };
        if let Some(when) = n["when"].as_str().and_then(|w| w.strip_prefix("row.")) {
            if !vorn_ext_perf_guest::is_truthy(row.get(when)) {
                return div();
            }
        }
        let mut el = match n["c"].as_str().unwrap_or("") {
            "row" | "column" => {
                let mut d = if n["c"] == "row" {
                    div().row().items_center()
                } else {
                    div().col()
                };
                if let Some(g) = n["gap"].as_f64() {
                    d = d.gap(g as f32);
                }
                if n["justify"] == "between" {
                    d = d.justify_between();
                }
                if let Some(p) = n["pad_bottom"].as_f64() {
                    d = d.pb(p as f32);
                }
                if let Some(p) = n["pad_top"].as_f64() {
                    d = d.pt(p as f32);
                }
                d.children(kids(self))
            }
            "text" => {
                let v = self.fill(n["value"].as_str().unwrap_or(""), row, "");
                match n["role"].as_str().unwrap_or("body") {
                    "meta" => text(v, 10.0, ink(look::INK_FAINT))
                        .weight(500)
                        .line_height(14.0)
                        .silent(),
                    "title" => {
                        let t = text(v, 13.0, ink(1.0))
                            .weight(500)
                            .line_height(18.0)
                            .silent();
                        let t = if self.windowed {
                            t.truncate()
                        } else {
                            t.wrapping()
                        };
                        div().grow().min_w0().child(t)
                    }
                    "secondary" => text(v, 11.0, ink(look::INK_FAINT))
                        .line_height(16.0)
                        .silent(),
                    _ => text(v, 12.0, ink(1.0)).silent(),
                }
            }
            "icon" => {
                let name = self.fill(n["name"].as_str().unwrap_or(""), row, "");
                let size = n["size"].as_f64().unwrap_or(14.0) as f32;
                let color = n["tone"]
                    .as_str()
                    .map_or(ink(look::INK_SECONDARY), |t| tone(&self.fill(t, row, "")));
                div().pt(2.0).child(ico(&name, size, color))
            }
            "menu" => div()
                .p(4.0)
                .rounded(4.0)
                .opacity(if hot > 0.0 { 1.0 } else { 0.0 })
                .child(ico(
                    n["icon"].as_str().unwrap_or("ellipsis"),
                    14.0,
                    ink(look::INK_FAINT),
                )),
            _ => div(),
        };
        if n["grow"] == true {
            el = el.grow().min_w0();
        }
        el
    }

    fn card(&self, ui: &Ui, row: &Value, ghost: bool) -> El {
        let rid = scalar(&row["id"]);
        let id = card_id(&rid);
        let hot = if ghost { 0.0 } else { ui.hover(id) };
        let mut card = div()
            .col()
            .px(14.0)
            .py(12.0)
            .rounded(6.0)
            .bg(Rgba::hex(look::SURFACE_RAISED))
            .border(1.0, white(0.08).mix(white(0.16), hot))
            .children(
                self.card["children"]
                    .as_array()
                    .into_iter()
                    .flatten()
                    .map(|c| self.node(c, row, hot))
                    .collect::<Vec<_>>(),
            );
        if self.windowed {
            card = card.h(self.card_h);
        }
        if ghost {
            return card;
        }
        let dragging = self.dragging.as_ref().is_some_and(|(d, _)| d == &rid);
        card.id(id)
            .sense(Sense::CLICK | Sense::MOVE | Sense::HOVER)
            .on(Action::Click)
            .opacity(if dragging { 0.4 } else { 1.0 })
    }

    fn header(&self, c: &Column, count: usize) -> El {
        div()
            .row()
            .items_center()
            .gap(8.0)
            .px(12.0)
            .py(12.0)
            .child(ico(&c.icon, 14.0, tone(&c.tone)))
            .child(
                text(c.label.clone(), 13.0, ink(look::INK_SECONDARY))
                    .weight(500)
                    .line_height(18.0)
                    .silent(),
            )
            .child(
                div().pl(2.0).child(
                    text(count.to_string(), 11.0, ink(look::INK_FAINT))
                        .line_height(16.0)
                        .silent(),
                ),
            )
            .child(div().grow())
    }

    /// The frame's tree: five columns of cards, and the dragged card at the pointer.
    pub fn view(&mut self, ui: &Ui, size: (f32, f32)) -> El {
        let mut groups: Vec<Vec<usize>> = vec![Vec::new(); self.columns.len()];
        for (i, r) in self.rows.iter().enumerate() {
            if r.get("archivedAt").is_some_and(|a| !a.is_null()) {
                continue;
            }
            let s = r["status"].as_str().unwrap_or("");
            if let Some(c) = self.columns.iter().position(|c| c.key == s) {
                groups[c].push(i);
            }
        }
        self.cards.clear();
        let stride = self.card_h + GAP;
        let mut board = div().row().gap(8.0).grow().min_h(0.0).w_full();
        for (c, group) in self.columns.iter().zip(&groups) {
            let sid = scroll_id(&c.key);
            let mut cards = div()
                .id(sid)
                .col()
                .grow()
                .min_h(0.0)
                .scroll_y()
                .px(8.0)
                .pb(8.0);
            let (first, last) = if self.windowed {
                let off = ui.scroll_offset(sid);
                let view_h = ui.rect_of(sid).map_or(size.1, |r| r.h);
                let first = ((off / stride).floor() as usize)
                    .saturating_sub(OVERSCAN)
                    .min(group.len());
                let last =
                    (first + (view_h / stride).ceil() as usize + 2 * OVERSCAN).min(group.len());
                (first, last)
            } else {
                (0, group.len())
            };
            if self.windowed && first > 0 {
                cards = cards.child(div().h(first as f32 * stride).w_full());
            }
            for &i in &group[first..last] {
                let row = &self.rows[i];
                let rid = scalar(&row["id"]);
                self.cards.insert(card_id(&rid), rid);
                cards = cards.child(div().pb(GAP).child(self.card(ui, row, false)));
            }
            if self.windowed && last < group.len() {
                cards = cards.child(div().h((group.len() - last) as f32 * stride).w_full());
            }
            let over = self
                .dragging
                .as_ref()
                .is_some_and(|(_, col)| col.as_deref() == Some(c.key.as_str()));
            let mut column = div()
                .id(column_id(&c.key))
                .sense(Sense::HOVER)
                .col()
                .flex(1.0)
                .min_w0()
                .min_h(0.0)
                .rounded(8.0)
                .child(self.header(c, group.len()))
                .child(cards);
            if over {
                column = column.bg(white(0.04)).border(1.0, white(0.10));
            }
            board = board.child(column);
        }
        let mut root = div().col().w(size.0).h(size.1).p(16.0).child(board);
        if let Some(ghost) = self.ghost(ui) {
            root = root.child(ghost);
        }
        root
    }

    fn ghost(&self, ui: &Ui) -> Option<El> {
        let (id, _) = self.dragging.as_ref()?;
        let (mid, dx, dy) = ui.moving()?;
        if mid != card_id(id) {
            return None;
        }
        let r = ui.rect_of(mid)?;
        let row = &self.rows[*self.index.get(id)?];
        Some(
            div()
                .absolute(r.x + dx, r.y + dy)
                .w(r.w)
                .opacity(0.9)
                .silent()
                .child(self.card(ui, row, true)),
        )
    }

    /// What the UI reported: a drag moves over columns, a drop moves the row and calls the extension once.
    pub fn event(&mut self, ui: &Ui, e: &Event) {
        match *e {
            Event::Move { id, x, y, .. } => {
                let Some(rid) = self.cards.get(&id).cloned() else {
                    return;
                };
                let over = self
                    .columns
                    .iter()
                    .find(|c| {
                        ui.rect_of(column_id(&c.key))
                            .is_some_and(|r| x >= r.x && x < r.x + r.w && y >= r.y && y < r.y + r.h)
                    })
                    .map(|c| c.key.clone());
                self.dragging = Some((rid, over));
            }
            Event::Drop { id, .. } => {
                let Some((rid, over)) = self.dragging.take() else {
                    return;
                };
                if card_id(&rid) != id {
                    return;
                }
                let Some(to) = over else { return };
                let Some(&i) = self.index.get(&rid) else {
                    return;
                };
                if self.rows[i]["status"] == to.as_str() {
                    return;
                }
                self.rows[i]["status"] = Value::String(to.clone());
                let call = &self.card["on"]["drop"];
                let row = &self.rows[i];
                let args: serde_json::Map<String, Value> = call["args"]
                    .as_object()
                    .into_iter()
                    .flatten()
                    .map(|(k, v)| {
                        (
                            k.clone(),
                            Value::String(self.fill(v.as_str().unwrap_or(""), row, &to)),
                        )
                    })
                    .collect();
                let method = call["call"].as_str().unwrap_or("task:setStatus").to_owned();
                self.calls.push((method, Value::Object(args)));
            }
            _ => {}
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn fills_rows_and_filters_into_template_text() {
        let b = Board::new(include_str!("../board.json"), true).unwrap();
        let row = json!({ "id": "abcd1234", "projectName": "bench", "status": "done", "createdAt": "2026-10-01T10:00:00.000Z", "title": "T" });
        assert_eq!(b.fill("{row.title}!", &row, ""), "T!");
        assert_eq!(
            b.fill("{row.id|short}", &row, ""),
            short_id("bench", "abcd1234")
        );
        assert_eq!(b.fill("{row.status|icon}", &row, ""), "circle-check");
        assert_eq!(b.fill("to {column}", &row, "todo"), "to todo");
        assert_eq!(b.fill("{row.missing}", &row, ""), "");
    }

    #[test]
    fn refuses_a_template_that_is_not_a_board() {
        assert!(Board::new(r#"{"view":{"c":"list"}}"#, false).is_err());
    }

    #[test]
    fn an_upsert_replaces_by_id_or_adds() {
        let mut b = Board::new(include_str!("../board.json"), false).unwrap();
        b.set_rows(vec![json!({ "id": "a", "status": "todo" })]);
        b.upsert(json!({ "id": "a", "status": "done" }));
        b.upsert(json!({ "id": "b", "status": "todo" }));
        assert_eq!(b.rows.len(), 2);
        assert_eq!(b.rows[0]["status"], "done");
    }
}

//! What a card view asks of vornd while it shows: how each card stands, the last lines its one-line cards print, which terminals stay attached, its drags, and the agents' permission requests drawn over its cards.

use serde_json::{json, Value};
use vorn_tree::view::CardSize;
use web_time::Instant;

use super::{App, Call};
use crate::card_view::{self, ColumnsBy, ASK_EVERY, LINE_ROWS};
use crate::client::RpcError;
use crate::screen::views::{card_id, column_id};
use crate::screen::MainView;
use crate::state::LayoutMode;
use crate::ui::{Event as UiEvent, Ui};

/// A card view call whose answer changes the screen.
pub(super) enum CardCall {
    /// `sessions:cards`.
    Fields,
    /// `terminal:readOutput` for a one-line card's session.
    Line(String),
    /// `permission:list`.
    Asks,
}

/// How many last lines are asked for at once.
const LINES_AT_ONCE: usize = 8;

impl App {
    /// The card view showing, if the sessions show as one.
    fn showing_view(&self) -> Option<String> {
        (self.st.view == MainView::Sessions)
            .then(|| self.st.layout.view().map(str::to_owned))
            .flatten()
    }

    /// Each frame: a view gone falls back to Grid; a showing one asks how its cards stand and reads its stale last lines.
    pub(super) fn views_tick(&mut self, ui: &Ui) {
        let Some(key) = self.st.layout.view().map(str::to_owned) else {
            return;
        };
        let found = self.st.points.card_view(&key).map(|(p, _)| p.title.clone());
        match found {
            Some(title) => self.st.cards.seen = Some((key.clone(), title)),
            None if self.st.points.loaded => {
                // Off since it showed: say so; never there: Grid without a word.
                if let Some((_, title)) = self.st.cards.seen.take().filter(|(k, _)| *k == key) {
                    self.st.toast(
                        format!("{title} is off, so the cards show in a grid"),
                        false,
                    );
                }
                self.st.layout = LayoutMode::Grid;
                return;
            }
            None => return,
        }
        if self.showing_view().is_none() || ui.is_cells() {
            return;
        }
        let now = Instant::now();
        if !self.st.cards.asking && self.st.cards.asked.is_none_or(|t| now >= t + ASK_EVERY) {
            self.st.cards.asking = true;
            self.st.cards.asked = Some(now);
            let ids: Vec<&str> = self
                .store
                .sessions
                .iter()
                .map(|s| s.info.id.as_str())
                .collect();
            self.call(
                Call::Card(CardCall::Fields),
                "sessions:cards",
                json!({ "ids": ids }),
            );
        }
        ui.wake_at(now + ASK_EVERY);
        // Typing reaches a selected one-line card through a pane that never sizes its session.
        let selected = self
            .st
            .selected
            .as_deref()
            .and_then(|id| self.store.session(id));
        if let (Some(g), Some(s), Some((_, spec))) =
            (&self.grid, selected, self.st.points.card_view(&key))
        {
            let now_ms = crate::state::now_ms().max(0.0) as u64;
            let attention = self.st.points.attention(self.store.workspace());
            let asking = card_view::asked(&self.st.asks, &attention, &s.info.id);
            let status =
                card_view::status_of(s, self.st.cards.printed_at(&s.info.id), now_ms, asking);
            if !s.ended() && spec.size(status) == CardSize::Line {
                g.attach_unsized(&s.info.id, super::cell_px(ui));
            }
        }
    }

    /// Reads again the last lines of the one-line cards that printed since theirs was read, a few at a time.
    fn read_lines(&mut self) {
        let Some(key) = self.showing_view() else {
            return;
        };
        let lines = self.line_cards(&key);
        let stale = self.st.cards.stale_lines(lines.iter().map(String::as_str));
        for id in stale.into_iter().take(LINES_AT_ONCE) {
            self.st.cards.lines.entry(id.clone()).or_default().reading = true;
            let params = json!({ "id": id, "lines": LINE_ROWS });
            self.call(
                Call::Card(CardCall::Line(id)),
                "terminal:readOutput",
                params,
            );
        }
    }

    /// The sessions view `key` draws as one-line cards now.
    fn line_cards(&self, key: &str) -> Vec<String> {
        let Some((_, spec)) = self.st.points.card_view(key) else {
            return Vec::new();
        };
        let now = crate::state::now_ms().max(0.0) as u64;
        let attention = self.st.points.attention(self.store.workspace());
        self.st
            .visible(&self.store)
            .into_iter()
            .map(|i| &self.store.sessions[i])
            .filter(|s| {
                let asking = card_view::asked(&self.st.asks, &attention, &s.info.id);
                let status =
                    card_view::status_of(s, self.st.cards.printed_at(&s.info.id), now, asking);
                spec.size(status) == CardSize::Line
            })
            .map(|s| s.info.id.clone())
            .collect()
    }

    /// After a view's frame is painted: the sessions it drew no terminal for let go of theirs, the selected one aside.
    pub(super) fn views_painted(&mut self, painted: &[String]) {
        if self.showing_view().is_none() {
            return;
        }
        let Some(g) = &self.grid else { return };
        for id in g.attached() {
            let card = self.store.session(&id).is_some();
            if card && !painted.contains(&id) && self.st.selected.as_deref() != Some(id.as_str()) {
                g.detach(&id);
                self.views.remove(&id);
                self.scrolls.remove(&id);
            }
        }
    }

    pub(super) fn card_answered(&mut self, call: CardCall, answer: Result<Value, RpcError>) {
        match call {
            CardCall::Fields => match answer {
                Ok(v) => {
                    self.st.cards.answered(&v);
                    self.st.cards.retain(&self.store);
                    self.read_lines();
                }
                Err(_) => self.st.cards.asking = false,
            },
            CardCall::Line(id) => self.st.cards.line_read(&id, answer.ok().as_ref()),
            CardCall::Asks => {
                if let Ok(v) = answer {
                    card_view::fold_asks(&mut self.st.asks, "permission:list", &v);
                }
            }
        }
    }

    /// The agents' requests open now, read again on every link to vornd.
    pub(super) fn ask_asks(&mut self) {
        self.call(Call::Card(CardCall::Asks), "permission:list", Value::Null);
    }

    /// Answers an agent's request from the card it is drawn over.
    pub(super) fn answer_permission(&mut self, request: String, allow: bool) {
        self.st.asks.retain(|a| a.request != request);
        self.call(
            Call::Quiet,
            "permission:resolve",
            json!({ "requestId": request, "allow": allow }),
        );
    }

    /// A card dragged in a view: the column under it lights; let go over another of your groups, its session moves there.
    pub(super) fn view_event(&mut self, ui: &Ui, e: &UiEvent) -> bool {
        let (id, x, y, end) = match *e {
            UiEvent::Move { id, x, y, .. } => (id, x, y, false),
            UiEvent::Drop { id, x, y, .. } => (id, x, y, true),
            _ => return false,
        };
        let Some(key) = self.showing_view() else {
            return false;
        };
        let Some(sid) = (self.store.sessions.iter())
            .map(|s| s.info.id.clone())
            .find(|sid| card_id(sid) == id)
        else {
            return false;
        };
        let Some((_, spec)) = self.st.points.card_view(&key) else {
            return false;
        };
        let keys: Vec<String> = match &spec.columns {
            ColumnsBy::Groups => (self.store.groups.iter().map(|g| g.id.clone()))
                .chain([String::new()])
                .collect(),
            _ => Vec::new(),
        };
        let over = keys.into_iter().find(|k| {
            ui.rect_of(column_id(k))
                .is_some_and(|r| x >= r.x && x < r.x + r.w && y >= r.y && y < r.y + r.h)
        });
        if !end {
            self.st.view_drag = Some((sid, over));
            return true;
        }
        self.st.view_drag = None;
        let Some(to) = over else { return true };
        let from = self
            .store
            .session(&sid)
            .and_then(|s| s.info.group_id.clone())
            .unwrap_or_default();
        if to != from {
            let group = (!to.is_empty()).then_some(to);
            self.move_to_group(sid, group, Call::Quiet);
        }
        true
    }
}

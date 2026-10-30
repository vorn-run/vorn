//! Card views: what an extension's view declares, the columns it puts the sessions in, the size each card takes, what a card shows beside its terminal, and the agents' permission requests drawn over cards.

use std::collections::{BTreeMap, HashMap};

use serde_json::Value;
use vorn_tree::view::{CardSize, CardStatus};
use web_time::Instant;

use crate::client::{Session, Store};
use crate::points::Row;

/// What a declared view's columns are.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ColumnsBy {
    Status,
    Project,
    Worktree,
    /// The person's own session groups.
    Groups,
    /// A grouping an extension contributes, by its point's key (`ext/groupings/id`).
    Grouping(String),
}

impl ColumnsBy {
    /// As the manifest writes it: `status`, `project`, `worktree`, `groups` or `<extension>/<grouping>`.
    pub fn parse(key: &str) -> Option<ColumnsBy> {
        Some(match key {
            "status" => ColumnsBy::Status,
            "project" => ColumnsBy::Project,
            "worktree" => ColumnsBy::Worktree,
            "groups" => ColumnsBy::Groups,
            _ => {
                let (ext, id) = key.split_once('/')?;
                ColumnsBy::Grouping(format!("{ext}/{}/{id}", crate::points::kind::GROUPINGS))
            }
        })
    }

    /// What the view's toolbar says its columns follow.
    pub fn label(&self) -> &str {
        match self {
            ColumnsBy::Status => "Status",
            ColumnsBy::Project => "Project",
            ColumnsBy::Worktree => "Worktree",
            ColumnsBy::Groups => "My groups",
            ColumnsBy::Grouping(_) => "Grouping",
        }
    }

    /// Whether dropping a card on another column moves its session there: only your own groups can.
    pub fn moves(&self) -> bool {
        matches!(self, ColumnsBy::Groups)
    }
}

/// A declared card view, as vornd lists it under `views`.
#[derive(Debug, Clone, PartialEq)]
pub struct Spec {
    pub columns: ColumnsBy,
    pub card: CardSize,
    pub by_status: BTreeMap<CardStatus, CardSize>,
}

impl Spec {
    /// A declared view's spec; `None` for a view drawn from its extension's tree.
    pub fn from_value(v: &Value) -> Option<Spec> {
        if v.get("tree").and_then(Value::as_bool) == Some(true) {
            return None;
        }
        let columns = ColumnsBy::parse(v.get("columns")?.as_str()?)?;
        let card = v
            .get("card")
            .and_then(Value::as_str)
            .and_then(CardSize::parse)
            .unwrap_or(CardSize::Full);
        let by_status = v
            .get("byStatus")
            .and_then(Value::as_object)
            .into_iter()
            .flatten()
            .filter_map(|(k, s)| Some((CardStatus::parse(k)?, CardSize::parse(s.as_str()?)?)))
            .collect();
        Some(Spec {
            columns,
            card,
            by_status,
        })
    }

    /// The size a card of `status` takes; a tile, which only a view's tree draws, reads as one line.
    pub fn size(&self, status: CardStatus) -> CardSize {
        match self.by_status.get(&status).copied().unwrap_or(self.card) {
            CardSize::Tile => CardSize::Line,
            s => s,
        }
    }
}

/// One column: its key, heading and sessions, as indexes into `store.sessions`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Column {
    pub key: String,
    pub title: String,
    pub sessions: Vec<usize>,
}

/// The heading a session goes under in a grouping's rows; Other when they name none.
pub const OTHER: &str = "Other";
/// The column of sessions in none of your groups.
pub const NO_GROUP: &str = "No group";

/// The status columns, in order; a started session counts as running.
const STATUS_COLUMNS: [CardStatus; 4] = [
    CardStatus::Printing,
    CardStatus::Waiting,
    CardStatus::Finished,
    CardStatus::Idle,
];

/// The columns `shown` (indexes into `store.sessions`, in order) fall into under `by`; `status` reads each card's status, `rows` are a grouping's.
pub fn columns(
    by: &ColumnsBy,
    shown: &[usize],
    store: &Store,
    status: impl Fn(&Session) -> CardStatus,
    rows: &[&Row],
) -> Vec<Column> {
    let mut out: Vec<Column> = Vec::new();
    let put = |key: String, title: String, i: usize, out: &mut Vec<Column>| match out
        .iter_mut()
        .find(|c| c.key == key)
    {
        Some(c) => c.sessions.push(i),
        None => out.push(Column {
            key,
            title,
            sessions: vec![i],
        }),
    };
    match by {
        ColumnsBy::Status => {
            out = STATUS_COLUMNS
                .iter()
                .map(|s| Column {
                    key: s.key().to_owned(),
                    title: s.label().to_owned(),
                    sessions: Vec::new(),
                })
                .collect();
            for &i in shown {
                let s = match status(&store.sessions[i]) {
                    CardStatus::Started => CardStatus::Printing,
                    s => s,
                };
                if let Some(c) = out.iter_mut().find(|c| c.key == s.key()) {
                    c.sessions.push(i);
                }
            }
        }
        ColumnsBy::Project => {
            for &i in shown {
                let name = store.sessions[i].info.project_name.clone();
                put(name.clone(), name, i, &mut out);
            }
        }
        ColumnsBy::Worktree => {
            for &i in shown {
                let info = &store.sessions[i].info;
                let path = info
                    .worktree_path
                    .clone()
                    .filter(|w| !w.is_empty())
                    .unwrap_or_else(|| info.project_path.clone());
                let place = info
                    .worktree_name
                    .clone()
                    .filter(|_| info.is_worktree == Some(true))
                    .unwrap_or_else(|| "main".to_owned());
                put(
                    path,
                    format!("{} · {place}", info.project_name),
                    i,
                    &mut out,
                );
            }
        }
        ColumnsBy::Groups => {
            out = store
                .groups
                .iter()
                .map(|g| Column {
                    key: g.id.clone(),
                    title: g.name.clone(),
                    sessions: Vec::new(),
                })
                .collect();
            out.push(Column {
                key: String::new(),
                title: NO_GROUP.to_owned(),
                sessions: Vec::new(),
            });
            for &i in shown {
                let g = store.sessions[i].info.group_id.as_deref().unwrap_or("");
                let at = out.iter().position(|c| c.key == g).unwrap_or(out.len() - 1);
                out[at].sessions.push(i);
            }
        }
        ColumnsBy::Grouping(_) => {
            for &i in shown {
                let id = &store.sessions[i].info.id;
                let head = rows
                    .iter()
                    .find(|r| r.id == *id)
                    .map(|r| r.text("group"))
                    .filter(|g| !g.is_empty())
                    .unwrap_or_else(|| OTHER.to_owned());
                put(head.clone(), head, i, &mut out);
            }
            // Other last, the rest as the rows first name them.
            let rank = |h: &str| {
                rows.iter()
                    .position(|r| r.text("group") == h)
                    .unwrap_or(usize::MAX)
            };
            out.sort_by_key(|c| rank(&c.key));
        }
    }
    out
}

/// What vornd says of one session's card (`sessions:cards`).
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct Fields {
    /// When it last printed, in epoch ms.
    pub printed_at: Option<u64>,
}

/// A card's last line, and the print it was read after.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Line {
    pub text: Option<String>,
    pub read_for: Option<u64>,
    pub reading: bool,
}

/// What card views show of each session beside its terminal, read from vornd while one shows.
#[derive(Debug, Default)]
pub struct Cards {
    pub fields: HashMap<String, Fields>,
    pub lines: HashMap<String, Line>,
    /// When `sessions:cards` was last asked, and whether its answer is still out.
    pub asked: Option<Instant>,
    pub asking: bool,
    /// The view last drawn, by key and title, so its going off is told.
    pub seen: Option<(String, String)>,
}

/// How often a showing view asks vornd how its cards stand.
pub const ASK_EVERY: std::time::Duration = std::time::Duration::from_secs(1);
/// The screen rows read for a card's last line.
pub const LINE_ROWS: u64 = 12;

impl Cards {
    /// Takes `sessions:cards`' answer.
    pub fn answered(&mut self, v: &Value) {
        self.asking = false;
        for c in v.as_array().into_iter().flatten() {
            let Some(id) = c.get("id").and_then(Value::as_str) else {
                continue;
            };
            let printed_at = c.get("printedAt").and_then(Value::as_u64);
            self.fields.insert(id.to_owned(), Fields { printed_at });
        }
    }

    /// When session `id` last printed, in epoch ms.
    pub fn printed_at(&self, id: &str) -> Option<u64> {
        self.fields.get(id).and_then(|f| f.printed_at)
    }

    /// The sessions of `ids` whose last line is older than their last print: each to be read again.
    pub fn stale_lines<'a>(&self, ids: impl IntoIterator<Item = &'a str>) -> Vec<String> {
        ids.into_iter()
            .filter(|id| {
                let line = self.lines.get(*id);
                !line.is_some_and(|l| l.reading)
                    && line.is_none_or(|l| l.read_for != self.printed_at(id) || l.text.is_none())
            })
            .map(str::to_owned)
            .collect()
    }

    /// `terminal:readOutput`'s answer for session `id`: its last line.
    pub fn line_read(&mut self, id: &str, v: Option<&Value>) {
        let read_for = self.printed_at(id);
        let lines: Vec<&str> = v
            .and_then(Value::as_array)
            .into_iter()
            .flatten()
            .filter_map(Value::as_str)
            .collect();
        let text = vorn_tree::view::last_line(lines).or_else(|| Some(String::new()));
        self.lines.insert(
            id.to_owned(),
            Line {
                text,
                read_for,
                reading: false,
            },
        );
    }

    /// Forgets what is kept of sessions no longer in `store`.
    pub fn retain(&mut self, store: &Store) {
        self.fields.retain(|id, _| store.session(id).is_some());
        self.lines.retain(|id, _| store.session(id).is_some());
    }
}

/// What a column shows of `cards` (each its place, height and whether a request waits on it) in `room` with `gap` between: requests first and always, then the rest while they fit; and how many it leaves out.
pub fn fit(cards: &[(usize, f32, bool)], room: f32, gap: f32) -> (Vec<usize>, usize) {
    let asking = cards.iter().filter(|c| c.2);
    let mut used: f32 = asking.clone().map(|c| c.1 + gap).sum();
    let mut shown: Vec<usize> = asking.map(|c| c.0).collect();
    let mut hidden = 0;
    for &(i, h, _) in cards.iter().filter(|c| !c.2) {
        if hidden > 0 || used + h > room {
            hidden += 1;
        } else {
            used += h + gap;
            shown.push(i);
        }
    }
    (shown, hidden)
}

/// A card's status, as vornd's rules read it from what the app knows of its session; one a request waits on is waiting.
pub fn status_of(s: &Session, printed_at: Option<u64>, now: u64, asking: bool) -> CardStatus {
    if asking {
        return CardStatus::Waiting;
    }
    CardStatus::of(
        s.info.agent_type.as_str(),
        s.status(),
        s.ended(),
        s.info.created_at.max(0.0) as u64,
        printed_at,
        now,
    )
}

/// An agent's request to use a tool, open until answered (`permission:requested`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Ask {
    pub request: String,
    /// The terminal it is about.
    pub session: String,
    pub tool: String,
    pub description: Option<String>,
}

impl Ask {
    pub fn from_value(v: &Value) -> Option<Ask> {
        let text = |k: &str| v.get(k).and_then(Value::as_str).filter(|s| !s.is_empty());
        Some(Ask {
            request: text("requestId")?.to_owned(),
            session: text("terminalId")?.to_owned(),
            tool: text("toolName").unwrap_or("a tool").to_owned(),
            description: text("description").map(str::to_owned),
        })
    }
}

/// Folds `permission:requested`, `permission:closed` or a `permission:list` answer into `asks`.
pub fn fold_asks(asks: &mut Vec<Ask>, method: &str, v: &Value) {
    match method {
        "permission:requested" => {
            if let Some(a) = Ask::from_value(v) {
                if !asks.iter().any(|x| x.request == a.request) {
                    asks.push(a);
                }
            }
        }
        "permission:closed" => {
            let id = v.get("requestId").and_then(Value::as_str);
            asks.retain(|a| Some(a.request.as_str()) != id);
        }
        "permission:list" => {
            *asks = v
                .as_array()
                .into_iter()
                .flatten()
                .filter_map(Ask::from_value)
                .collect();
        }
        _ => {}
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn store(sessions: Value, groups: Value) -> Store {
        let mut s = Store::default();
        s.apply("config:changed", json!({ "sessionGroups": groups }));
        for v in sessions.as_array().unwrap() {
            s.upsert(serde_json::from_value(v.clone()).unwrap());
        }
        s
    }

    fn session(id: &str, project: &str, status: &str, extra: Value) -> Value {
        let mut v = json!({ "id": id, "agentType": "acme", "projectName": project,
            "projectPath": format!("/{project}"), "status": status, "createdAt": 0.0, "pid": 1.0 });
        v.as_object_mut()
            .unwrap()
            .extend(extra.as_object().unwrap().clone());
        v
    }

    #[test]
    fn a_spec_reads_its_columns_and_sizes_by_status() {
        let spec = Spec::from_value(&json!({ "columns": "acme.chat/channels", "card": "mini",
            "byStatus": { "finished": "line", "idle": "tile", "bad": "line" } }))
        .unwrap();
        assert_eq!(
            spec.columns,
            ColumnsBy::Grouping("acme.chat/groupings/channels".into())
        );
        assert_eq!(spec.size(CardStatus::Printing), CardSize::Mini);
        assert_eq!(spec.size(CardStatus::Finished), CardSize::Line);
        assert_eq!(
            spec.size(CardStatus::Idle),
            CardSize::Line,
            "a tile reads as a line"
        );
        assert!(Spec::from_value(&json!({ "tree": true })).is_none());
        assert!(Spec::from_value(&json!({ "columns": "colour" })).is_none());
        assert_eq!(
            Spec::from_value(&json!({ "columns": "status" }))
                .unwrap()
                .card,
            CardSize::Full
        );
    }

    #[test]
    fn columns_follow_each_grouping() {
        let st = store(
            json!([
                session("a", "vorn", "running", json!({ "groupId": "g1" })),
                session("b", "novum", "waiting", json!({})),
                session(
                    "c",
                    "vorn",
                    "idle",
                    json!({ "worktreePath": "/w/x", "worktreeName": "x", "isWorktree": true })
                ),
            ]),
            json!([{ "id": "g1", "name": "Review", "order": 0.0, "workspaceId": "personal" }]),
        );
        let shown = [0, 1, 2];
        let by_status = |s: &Session| status_of(s, None, 1_000_000, false);
        let titles = |cols: Vec<Column>| -> Vec<(String, Vec<usize>)> {
            cols.into_iter().map(|c| (c.title, c.sessions)).collect()
        };
        assert_eq!(
            titles(columns(&ColumnsBy::Status, &shown, &st, by_status, &[])),
            [
                ("Running".to_owned(), vec![0]),
                ("Waiting".to_owned(), vec![1]),
                ("Done".to_owned(), vec![]),
                ("Idle".to_owned(), vec![2]),
            ]
        );
        assert_eq!(
            titles(columns(&ColumnsBy::Project, &shown, &st, by_status, &[])),
            [
                ("vorn".to_owned(), vec![0, 2]),
                ("novum".to_owned(), vec![1])
            ]
        );
        assert_eq!(
            titles(columns(&ColumnsBy::Worktree, &shown, &st, by_status, &[])),
            [
                ("vorn · main".to_owned(), vec![0]),
                ("novum · main".to_owned(), vec![1]),
                ("vorn · x".to_owned(), vec![2]),
            ]
        );
        assert_eq!(
            titles(columns(&ColumnsBy::Groups, &shown, &st, by_status, &[])),
            [
                ("Review".to_owned(), vec![0]),
                (NO_GROUP.to_owned(), vec![1, 2])
            ]
        );
        let rows: Vec<Row> = [("b", "#chat"), ("c", "#ops"), ("x", "#ops")]
            .iter()
            .map(|(id, g)| Row::from_value(&json!({ "id": id, "group": g })).unwrap())
            .collect();
        let rows: Vec<&Row> = rows.iter().collect();
        let by = ColumnsBy::Grouping("acme/groupings/channels".into());
        assert_eq!(
            titles(columns(&by, &shown, &st, by_status, &rows)),
            [
                ("#chat".to_owned(), vec![1]),
                ("#ops".to_owned(), vec![2]),
                (OTHER.to_owned(), vec![0]),
            ]
        );
        assert!(ColumnsBy::Groups.moves());
        assert!(!ColumnsBy::Status.moves() && !by.moves());
    }

    #[test]
    fn a_full_column_never_hides_a_card_a_request_waits_on() {
        let cards = [
            (0, 100.0, false),
            (1, 100.0, true),
            (2, 100.0, false),
            (3, 100.0, true),
            (4, 100.0, true),
        ];
        assert_eq!(
            fit(&cards, 250.0, 10.0),
            (vec![1, 3, 4], 2),
            "requests show even past the room"
        );
        assert_eq!(fit(&cards, 450.0, 10.0), (vec![1, 3, 4, 0], 1));
        assert_eq!(fit(&cards[..1], 50.0, 10.0), (vec![], 1));
    }

    #[test]
    fn a_session_a_request_waits_on_is_waiting() {
        let st = store(
            json!([session("a", "vorn", "running", json!({}))]),
            json!([]),
        );
        assert_eq!(
            status_of(&st.sessions[0], None, 1_000_000, true),
            CardStatus::Waiting
        );
        assert_eq!(
            status_of(&st.sessions[0], None, 1_000_000, false),
            CardStatus::Printing
        );
    }

    #[test]
    fn a_line_is_read_again_only_after_the_session_prints() {
        let mut cards = Cards::default();
        cards.answered(&json!([{ "id": "a", "printedAt": 5 }, { "id": "b" }]));
        assert_eq!(cards.stale_lines(["a", "b"]), ["a", "b"]);
        cards.line_read("a", Some(&json!(["$ make", "\u{1b}[32mok\u{1b}[0m", ""])));
        cards.line_read("b", None);
        assert_eq!(cards.lines["a"].text.as_deref(), Some("ok"));
        assert!(cards.stale_lines(["a", "b"]).is_empty());
        cards.answered(&json!([{ "id": "a", "printedAt": 9 }]));
        assert_eq!(cards.stale_lines(["a", "b"]), ["a"]);
    }

    #[test]
    fn asks_open_and_close_by_request() {
        let mut asks = Vec::new();
        let ask = json!({ "requestId": "r1", "terminalId": "t", "toolName": "Bash", "description": "git push" });
        fold_asks(&mut asks, "permission:requested", &ask);
        fold_asks(&mut asks, "permission:requested", &ask);
        assert_eq!(asks.len(), 1);
        assert_eq!(asks[0].description.as_deref(), Some("git push"));
        fold_asks(
            &mut asks,
            "permission:closed",
            &json!({ "requestId": "r1" }),
        );
        assert!(asks.is_empty());
        fold_asks(
            &mut asks,
            "permission:list",
            &json!([ask, { "requestId": "r2" }]),
        );
        assert_eq!(asks.len(), 1, "a request naming no terminal is not drawn");
    }
}

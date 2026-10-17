//! Tasks: the board's writes (`task:*` but the reads, which are the core's
//! paged list), each task's images, the workflow triggers a write fires, the
//! tasks connectors import, the MCP task tools, the badge of the task a
//! session works on, and the board's tree.
//!
//! Vorn ships it as a Wasm component (`vorn-extension.toml` beside this
//! crate). The rows stay in `vorn.db` under the names they have always had
//! (`tasks`, `task_source_links`): Vorn lets this one extension write them
//! with its storage transactions, and reads them back through the core's
//! `task.get`, so a task reads the same wherever it is read. Each write tells
//! clients the rows it changed (`task.changed`), and a task created or moved
//! is a public event and a `workflow.fire`.

use std::collections::BTreeMap;

use vorn_extension_sdk::tree::{self, Surface};
use vorn_extension_sdk::{arg, json, Ctx, Error, Event, Extension, Host, Init, Op, Value};

pub mod calls;
pub mod db;
pub mod images;
pub mod js;
pub mod nodes;
pub mod sources;
pub mod tools;
pub mod triggers;
mod view;

use db::{Db, HostDb};

/// The id the extension goes by.
pub const ID: &str = "vorn.tasks";

/// Its manifest: what it declares it uses, what it draws and the MCP tools it adds.
pub const MANIFEST: &str = include_str!("../vorn-extension.toml");

/// What `describe_workflow_nodes` tells of the node types it declares, by id.
pub const NODE_DOCS: &str = include_str!("../generated/nodes.json");

/// The MCP tools it adds, as `tools/list` lists them, exactly as the TypeScript defines them.
pub const TOOLS: &str = include_str!("../generated/tools.json");

/// What a call to a task method is told while Tasks is off.
pub const OFF: &str = "Tasks is off. Turn it on in Settings → Extensions.";

/// The collection the card badge draws: one row per session working on a task.
pub const BADGES: &str = "working";

/// The core method that tells clients which tasks changed and emits the events a write fires.
pub const CHANGED: &str = "task.changed";

/// Every `task:*` call it answers.
pub const METHODS: &[&str] = &[
    "task:setStatus",
    "task:create",
    "task:update",
    "task:patch",
    "task:delete",
    "task:reorder",
    "task:archive",
    "task:imageSave",
    "task:imageDelete",
    "task:imageGetPath",
    "task:imageCleanup",
    "task:imageUpload",
];

/// Now, as an ISO 8601 UTC time with milliseconds.
pub fn now_iso() -> String {
    let ms = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |d| i64::try_from(d.as_millis()).unwrap_or(i64::MAX));
    js::iso(ms)
}

/// A fresh task id.
pub fn new_id() -> String {
    uuid::Uuid::new_v4().to_string()
}

/// What a write did to the board, as clients are told it.
#[derive(Debug, Clone, PartialEq)]
pub enum Change {
    /// These rows, whole, as `task.get` reads them: new or changed.
    Rows(Vec<Value>),
    /// These tasks are gone.
    Removed(Vec<String>),
}

impl Change {
    fn args(&self) -> Value {
        match self {
            Change::Rows(rows) => json!({ "rows": rows }),
            Change::Removed(ids) => json!({ "removed": ids }),
        }
    }
}

/// The extension: the badge rows it last published, read back from its rows
/// the first time it needs them, so a restart of its instance loses nothing.
#[derive(Debug, Default)]
pub struct Tasks {
    badges: Option<BTreeMap<String, Value>>,
}

fn failed(message: impl Into<String>) -> Error {
    Error::new("failed", message)
}

impl Tasks {
    /// Answers a `task:*` call.
    pub fn answer(&mut self, host: &mut dyn Host, method: &str, params: &Value) -> Result<Value, Error> {
        if method.starts_with("task:image") {
            return images::answer(host, method, params).map_err(failed);
        }
        if !METHODS.contains(&method) {
            return Err(Error::unknown(method));
        }
        let mut db = HostDb::new(host);
        let done = calls::write(&mut db, method, params, &now_iso(), new_id).map_err(failed)?;
        db.flush().map_err(failed)?;
        drop(db);
        if let Some(trigger) = done.trigger {
            fire(host, &trigger);
        }
        if let Some(change) = done.change {
            self.told(host, &change);
        }
        Ok(done.value.unwrap_or(Value::Null))
    }

    /// Tells clients `change`, and moves the badges when it can have moved one.
    fn told(&mut self, host: &mut dyn Host, change: &Change) {
        if let Err(err) = host.call(CHANGED, change.args()) {
            host.log(vorn_extension_sdk::Level::Warn, &format!("could not tell clients the tasks changed: {err}"));
        }
        if self.touches_badges(host, change) {
            self.badges(host);
        }
    }

    /// `task.importItems`: a connection's items on the board, each answered `{taskId, created}`.
    fn import(&mut self, host: &mut dyn Host, args: Value) -> Result<Value, Error> {
        let import: sources::Import = serde_json::from_value(args).map_err(|e| {
            failed(format!(
                "task.importItems takes a connection and its items ({e})"
            ))
        })?;
        let now = now_iso();
        let mut db = HostDb::new(host);
        let answers = sources::import(&mut db, &import, &now, new_id).map_err(failed)?;
        let mut rows = Vec::with_capacity(answers.len());
        for a in &answers {
            let found = db
                .call("dbGetTask", json!([a.get("taskId")]))
                .map_err(failed)?;
            if !found.is_null() {
                rows.push(found);
            }
        }
        drop(db);
        if !rows.is_empty() {
            self.told(host, &Change::Rows(rows));
        }
        Ok(Value::Array(answers))
    }

    /// A connector item on the board, as `createTaskFromItem` puts it: read
    /// as a task by Connectors, imported here, and its connection stamped
    /// synced. Answers `{taskId, created}`.
    fn upsert_item(&mut self, host: &mut dyn Host, item: Value) -> Result<Value, String> {
        let args = host
            .call("connection.itemImport", item)
            .map_err(|e| e.message)?;
        let answered = self.import(host, args.clone()).map_err(|e| e.message)?;
        host.call("connection.synced", args)
            .map_err(|e| e.message)?;
        Ok(answered
            .as_array()
            .and_then(|a| a.last())
            .cloned()
            .unwrap_or(Value::Null))
    }

    /// `task.take`: task `id` in progress for a workflow step's session and
    /// agent; `task.handBack`: back in the queue, unassigned. Each fires the
    /// workflows watching for the move, as any write does.
    fn take_or_hand_back(
        &mut self,
        host: &mut dyn Host,
        method: &str,
        args: &Value,
    ) -> Result<Value, Error> {
        let id = arg(args, "id", method)
            .map_err(|_| failed(format!("{method} takes the id of the task")))?
            .to_owned();
        let mut db = HostDb::new(host);
        let moved =
            calls::take_or_hand_back(&mut db, method, &id, args, &now_iso()).map_err(failed)?;
        drop(db);
        let Some((before, after)) = moved else {
            return Ok(Value::Null);
        };
        if let Some(trigger) = triggers::for_task(Some(&before), &after) {
            fire(host, &trigger);
        }
        self.told(host, &Change::Rows(vec![after.clone()]));
        Ok(after)
    }

    /// Keeps the conversation a terminal started on the in-progress task it
    /// was assigned, when that task has none yet.
    fn keep_conversation(&mut self, host: &mut dyn Host, terminal: &str, conversation: &str) {
        let mut db = HostDb::new(host);
        let kept = calls::keep_conversation(&mut db, terminal, conversation, &now_iso());
        drop(db);
        match kept {
            Ok(Some(after)) => self.told(host, &Change::Rows(vec![after])),
            Ok(None) => {}
            Err(err) => host.log(
                vorn_extension_sdk::Level::Warn,
                &format!("could not keep the conversation on the task: {err}"),
            ),
        }
    }

    fn shown(&mut self, host: &mut dyn Host) -> &mut BTreeMap<String, Value> {
        self.badges.get_or_insert_with(|| {
            host.rows(BADGES)
                .unwrap_or_default()
                .into_iter()
                .filter_map(|r| Some((r.get("id")?.as_str()?.to_owned(), r)))
                .collect()
        })
    }

    /// Whether `change` can move a badge: a task in progress on a session
    /// now, or one a badge shows. Others leave the badges as they are, so a
    /// write never reads the tasks in progress for nothing.
    fn touches_badges(&mut self, host: &mut dyn Host, change: &Change) -> bool {
        let shown = self.shown(host);
        let on_a_badge = |id: &str| {
            shown
                .values()
                .any(|b| b.get("task").and_then(Value::as_str) == Some(id))
        };
        match change {
            Change::Removed(ids) => ids.iter().any(|id| on_a_badge(id)),
            Change::Rows(rows) => rows.iter().any(|r| {
                let working = r.get("status").and_then(Value::as_str) == Some("in_progress")
                    && r.get("assignedSessionId").is_some_and(|s| !s.is_null());
                working || r.get("id").and_then(Value::as_str).is_some_and(on_a_badge)
            }),
        }
    }

    /// Publishes the card badge's rows again, as the board is now: one per
    /// session with a task in progress, publishing only what moved.
    pub fn badges(&mut self, host: &mut dyn Host) {
        let working = HostDb::new(host).call("dbListTasks", json!([null, "in_progress"]));
        let working = match working {
            Ok(w) => w,
            Err(err) => {
                host.log(
                    vorn_extension_sdk::Level::Warn,
                    &format!("could not read the tasks in progress for their badges: {err}"),
                );
                return;
            }
        };
        let now = calls::badge_rows(&working);
        let was = self.shown(host);
        let mut ops: Vec<Op> = was
            .keys()
            .filter(|k| !now.contains_key(*k))
            .map(|k| Op::Delete(k.clone()))
            .collect();
        ops.extend(
            now.iter()
                .filter(|(k, v)| was.get(*k) != Some(*v))
                .map(|(_, v)| Op::Upsert(v.clone())),
        );
        *was = now;
        if !ops.is_empty() {
            host.publish(BADGES, ops);
        }
    }

    /// What a client that draws the board from its tree alone asks of it: a
    /// card dropped on a column, or a card's menu entry.
    fn board_action(&mut self, host: &mut dyn Host, action: &str, args: &Value) -> Result<Value, Error> {
        let id = arg(args, "id", action)?.to_owned();
        let status = |s: &str| json!({ "id": id, "status": s });
        let (method, params) = match action {
            "move" => ("task:setStatus", status(arg(args, "to", action)?)),
            "complete" => ("task:setStatus", status("done")),
            "reopen" => ("task:setStatus", status("todo")),
            "cancel" => ("task:setStatus", status("cancelled")),
            "archive" => ("task:archive", json!({ "id": id, "archived": true })),
            "unarchive" => ("task:archive", json!({ "id": id, "archived": false })),
            "delete" => ("task:delete", json!({ "id": id })),
            // Opening a card is the client's; there is nothing to write.
            "open" => return Ok(Value::Null),
            _ => return Err(Error::new("unknown-action", format!("Tasks has no action {action}"))),
        };
        self.answer(host, method, &params)
    }
}

/// A trigger a write fired: the public event every subscriber hears, and
/// the workflows watching for it started (`workflow.fire`).
fn fire(host: &mut dyn Host, trigger: &Value) {
    if let Err(err) = host.call(CHANGED, json!({ "events": [trigger] })) {
        host.log(
            vorn_extension_sdk::Level::Warn,
            &format!("could not announce a task's move: {err}"),
        );
    }
    if let Some(args) = triggers::fire_args(trigger) {
        if let Err(err) = host.call("workflow.fire", args) {
            host.log(
                vorn_extension_sdk::Level::Info,
                &format!("no workflow was started for a task's move: {err}"),
            );
        }
    }
}

impl Extension for Tasks {
    fn init(_: &Init, _: &mut dyn Host) -> Result<Self, Error> {
        Ok(Tasks::default())
    }

    fn handle(
        &mut self,
        host: &mut dyn Host,
        method: &str,
        args: Value,
        _: &Ctx,
    ) -> Result<Value, Error> {
        if let Some(tool) = method.strip_prefix("mcp.") {
            return tools::call(self, host, tool, args)
                .ok_or_else(|| Error::unknown(method));
        }
        match method {
            m if m.starts_with("task:") => self.answer(host, m, &args),
            "task.importItems" => self.import(host, args),
            "task.take" | "task.handBack" => self.take_or_hand_back(host, method, &args),
            "task.sourceLink" => {
                let task = arg(&args, "taskId", "task.sourceLink")?.to_owned();
                sources::link(&mut HostDb::new(host), &task).map_err(failed)
            }
            "workflow.node.createTaskFromItem" => Ok(nodes::create_task_from_item(&args, |item| {
                self.upsert_item(host, item)
            })),
            "badges" => {
                self.badges(host);
                Ok(Value::Null)
            }
            other => self.board_action(host, other, &args),
        }
    }

    fn on_events(&mut self, host: &mut dyn Host, batch: Vec<Event>) {
        for e in batch.iter().filter(|e| e.name == "agent.conversation") {
            let text = |k: &str| e.data.get(k).and_then(Value::as_str).map(str::to_owned);
            if let (Some(terminal), Some(conversation)) =
                (text("sessionId"), text("conversationId"))
            {
                self.keep_conversation(host, &terminal, &conversation);
            }
        }
    }

    fn render(
        &mut self,
        host: &mut dyn Host,
        point: &str,
        _: Surface,
    ) -> Result<tree::Node, Error> {
        match point {
            "board" => Ok(view::board()),
            "task" => {
                let shown = self.shown(host);
                Ok(view::badges(shown.values()))
            }
            other => Err(Error::new("unknown-action", format!("nothing draws {other}"))),
        }
    }
}

vorn_extension_sdk::export!(Tasks);

#[cfg(test)]
mod tests;

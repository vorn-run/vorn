//! What the gate controls do: `answerGate` with the comment, note or
//! rewrite the reviewer wrote, and the editor's table and source.

use serde_json::{json, Value};

use super::{workflows::WfCall, App, Call};
use crate::gates::{self, cell_id, edit_id, text_id, Editor, GateOp, GateUi, Mode};
use crate::runs::{self, s};
use crate::ui::{Event as UiEvent, Ui};

impl App {
    /// The waiting state of a gate.
    fn gate_state(&self, run: &str, node: &str) -> Option<Value> {
        let r = self.find_run(run)?;
        let found = runs::states(&r)
            .find(|n| s(n, "nodeId") == Some(node))
            .cloned();
        found
    }

    /// What Approve sends: the editor's draft while it is open, else the
    /// kept rewrite.
    pub fn gate_edited(&self, key: &str) -> Option<String> {
        let g = self.st.runs.gates.get(key)?;
        match (&g.mode, &g.editor) {
            (Some(Mode::Edit), Some(e)) => e.edited(),
            _ => g.edited.clone(),
        }
    }

    /// Why the open editor's draft cannot be approved yet.
    pub fn gate_blocked(&self, key: &str) -> Option<String> {
        let g = self.st.runs.gates.get(key)?;
        match (&g.mode, &g.editor) {
            (Some(Mode::Edit), Some(e)) => e.invalid(),
            _ => None,
        }
    }

    fn answer(
        &mut self,
        key: &str,
        decision: &str,
        comment: Option<String>,
        edited: Option<String>,
    ) {
        let Some((run, node)) = gates::parts(key) else {
            return;
        };
        let mut params = json!({ "runId": run, "nodeId": node, "decision": decision });
        if let Some(c) = comment.filter(|c| !c.is_empty()) {
            params["comment"] = json!(c);
        }
        if let Some(e) = edited {
            params["edited"] = json!(e);
        }
        if let Some(g) = self.st.runs.gates.get_mut(key) {
            g.sending = true;
        }
        self.call(
            Call::Workflow(WfCall::Gate(Some(key.to_owned()), decision == "changes")),
            "workflow:resolveGate",
            params,
        );
    }

    /// The gate settled, or refused: the draft goes, or waits again.
    pub(super) fn gate_answered(&mut self, key: &str, taken: bool) {
        if taken {
            // Every place that gate was answered from is done with it.
            let rest = key.split_once('|').map_or("", |(_, r)| r).to_owned();
            self.st.runs.gates.retain(|k, _| !k.ends_with(&rest));
        } else if let Some(g) = self.st.runs.gates.get_mut(key) {
            g.sending = false;
        }
    }

    pub(super) fn run_gate(&mut self, ui: &mut Ui, op: GateOp) {
        match op {
            GateOp::Open(key, mode) => {
                let Some((run, node)) =
                    gates::parts(&key).map(|(r, n)| (r.to_owned(), n.to_owned()))
                else {
                    return;
                };
                let state = self.gate_state(&run, &node).unwrap_or(Value::Null);
                let g = self.st.runs.gates.entry(key.clone()).or_default();
                g.mode = Some(mode);
                match mode {
                    Mode::Edit => {
                        let original = gates::editable(&state).unwrap_or("").to_owned();
                        let initial = g.edited.clone().unwrap_or_else(|| original.clone());
                        let e = Editor::open(&original, &initial);
                        ui.set_edit_text(edit_id(&key), &e.text);
                        seed_cells(ui, &key, &e);
                        if e.table.is_none() {
                            ui.set_focus(Some(edit_id(&key)));
                        }
                        g.editor = Some(e);
                    }
                    _ => {
                        ui.set_edit_text(text_id(&key), "");
                        ui.set_focus(Some(text_id(&key)));
                    }
                }
            }
            GateOp::Cancel(key) => {
                if let Some(g) = self.st.runs.gates.get_mut(&key) {
                    g.mode = None;
                    g.editor = None;
                }
                ui.set_focus(None);
            }
            GateOp::Send(key) => {
                let Some(mode) = self.st.runs.gates.get(&key).and_then(|g| g.mode) else {
                    return;
                };
                let note = ui.edit_text(text_id(&key)).trim().to_owned();
                match mode {
                    Mode::Changes if note.is_empty() => {}
                    Mode::Changes => {
                        let edited = self.st.runs.gates.get(&key).and_then(|g| g.edited.clone());
                        self.answer(&key, "changes", Some(note), edited);
                    }
                    Mode::Reject => self.answer(&key, "reject", Some(note), None),
                    Mode::Edit => {}
                }
            }
            GateOp::Approve(key) => {
                if self.gate_blocked(&key).is_some() {
                    return;
                }
                let edited = self.gate_edited(&key);
                self.answer(&key, "approve", None, edited);
            }
            GateOp::Reject(key) => self.answer(&key, "reject", None, None),
            GateOp::Save(key) => {
                let Some(g) = self.st.runs.gates.get_mut(&key) else {
                    return;
                };
                let Some(e) = &g.editor else {
                    return;
                };
                if e.invalid().is_some() {
                    return;
                }
                g.edited = e.edited();
                g.mode = None;
                g.editor = None;
                ui.set_focus(None);
            }
            GateOp::Revert(key) => {
                if let Some(e) = self
                    .st
                    .runs
                    .gates
                    .get_mut(&key)
                    .and_then(|g| g.editor.as_mut())
                {
                    e.revert();
                    ui.set_edit_text(edit_id(&key), &e.text);
                    seed_cells(ui, &key, e);
                }
            }
            GateOp::Source(key, source) => {
                if let Some(e) = self
                    .st
                    .runs
                    .gates
                    .get_mut(&key)
                    .and_then(|g| g.editor.as_mut())
                {
                    if source {
                        e.to_source();
                        ui.set_edit_text(edit_id(&key), &e.text);
                    } else if e.to_table() {
                        seed_cells(ui, &key, e);
                    }
                }
            }
            GateOp::Remove(key, row, removed) => {
                if let Some(t) = self
                    .st
                    .runs
                    .gates
                    .get_mut(&key)
                    .and_then(|g| g.editor.as_mut())
                    .and_then(|e| e.table.as_mut())
                {
                    t.set_removed(row, removed);
                }
            }
            GateOp::Flip(key, row, col) => {
                if let Some(t) = self
                    .st
                    .runs
                    .gates
                    .get_mut(&key)
                    .and_then(|g| g.editor.as_mut())
                    .and_then(|e| e.table.as_mut())
                {
                    t.toggle_bool(row, &col);
                }
            }
            GateOp::Review(at) => {
                self.st.runs.review = at;
                if self.st.runs.review.is_some() {
                    self.st.dialog = Some(crate::state::Dialog::GateReview);
                } else if self.st.dialog == Some(crate::state::Dialog::GateReview) {
                    self.st.dialog = None;
                }
            }
        }
    }

    /// Typing in a gate's composer, editor or table; Cmd+Enter sends or
    /// saves. True when the event was a gate's.
    pub(super) fn gate_event(&mut self, ui: &mut Ui, event: &UiEvent) -> bool {
        let (UiEvent::Changed(id) | UiEvent::Submit(id)) = *event else {
            return false;
        };
        let submit = matches!(event, UiEvent::Submit(_));
        let keys: Vec<String> = self.st.runs.gates.keys().cloned().collect();
        for key in keys {
            if id == text_id(&key) {
                if submit {
                    self.run_gate(ui, GateOp::Send(key));
                }
                return true;
            }
            if id == edit_id(&key) {
                if submit {
                    self.run_gate(ui, GateOp::Save(key));
                } else if let Some(e) = self
                    .st
                    .runs
                    .gates
                    .get_mut(&key)
                    .and_then(|g| g.editor.as_mut())
                {
                    e.text = ui.edit_text(id).to_owned();
                }
                return true;
            }
            let Some(g) = self.st.runs.gates.get_mut(&key) else {
                continue;
            };
            let Some(t) = g.editor.as_mut().and_then(|e| e.table.as_mut()) else {
                continue;
            };
            let cols = t.columns();
            for row in 0..t.rows.len() {
                for col in &cols {
                    if id != cell_id(&key, row, col) {
                        continue;
                    }
                    if submit {
                        self.run_gate(ui, GateOp::Save(key));
                        return true;
                    }
                    let text = ui.edit_text(id).to_owned();
                    if t.rows[row].values.get(col).is_some_and(Value::is_number)
                        || t.rows[row].typed.contains_key(col)
                    {
                        t.set_number(row, col, &text);
                    } else {
                        t.set_text(row, col, &text);
                    }
                    return true;
                }
            }
        }
        false
    }
}

/// Puts each table cell's text in its field.
fn seed_cells(ui: &mut Ui, key: &str, e: &Editor) {
    let Some(t) = &e.table else {
        return;
    };
    for col in t.columns() {
        for (i, r) in t.rows.iter().enumerate() {
            let text = match (r.typed.get(&col), r.values.get(&col)) {
                (Some(typed), _) => typed.clone(),
                (None, Some(Value::String(v))) => v.clone(),
                (None, Some(Value::Number(n))) => n.to_string(),
                _ => String::new(),
            };
            ui.set_edit_text(cell_id(key, i, &col), &text);
        }
    }
}

impl GateUi {
    /// Whether anything is open that keys should not reach past.
    pub fn busy(&self) -> bool {
        self.mode.is_some()
    }
}

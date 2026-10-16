//! An approval gate as a reviewer answers it: the answer being
//! written, the rewrite kept until it is sent, and a list of records as a
//! table that cannot produce broken JSON.

use std::collections::{BTreeMap, HashSet};

use serde_json::{Map, Value};

use crate::runs::s;
use crate::ui::Id;

/// `MAX_GATE_ROUNDS`.
pub const MAX_ROUNDS: u64 = 10;
/// `DEFAULT_GATE_ROUNDS`.
pub const DEFAULT_ROUNDS: u64 = 3;

/// Where a gate is answered; each keeps its own draft.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Place {
    /// The run pane's large actions.
    Pane,
    /// The small row under the waiting step in a trace.
    Trace,
    /// The review modal's footer.
    Review,
}

impl Place {
    fn tag(self) -> &'static str {
        match self {
            Place::Pane => "pane",
            Place::Trace => "trace",
            Place::Review => "review",
        }
    }
}

/// One gate at one place: `place|run|node`.
pub fn key(place: Place, run: &str, node: &str) -> String {
    format!("{}|{run}|{node}", place.tag())
}

/// The composer's text area at a gate.
pub fn text_id(key: &str) -> Id {
    Id::new("gate-text").child(key)
}

/// The editor's text area (prose, or the JSON source) at a gate.
pub fn edit_id(key: &str) -> Id {
    Id::new("gate-edit").child(key)
}

/// A table cell of the editor at a gate.
pub fn cell_id(key: &str, row: usize, col: &str) -> Id {
    Id::new("gate-cell").child((key, row, col))
}

/// What the gate controls do; each names its gate by [`key`].
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub enum GateOp {
    /// Opens the comment, the note or the editor.
    Open(String, Mode),
    Cancel(String),
    /// Sends the work back, or rejects with the note.
    Send(String),
    /// Approves with the kept rewrite, or the editor's draft.
    Approve(String),
    /// Rejects at once: the pane's Reject run and the R key.
    Reject(String),
    /// Keeps the editor's draft as the rewrite.
    Save(String),
    Revert(String),
    /// The JSON editor's Table and Source.
    Source(String, bool),
    /// Removes a table row, or puts it back.
    Remove(String, usize, bool),
    /// Flips a true or false cell.
    Flip(String, usize, String),
    /// Opens the review page of `(run, node)`, or closes it.
    Review(Option<(String, String)>),
}

/// The run and node a gate key names.
pub fn parts(key: &str) -> Option<(&str, &str)> {
    let mut it = key.splitn(3, '|');
    let _place = it.next()?;
    Some((it.next()?, it.next()?))
}

/// What the reviewer has open at a gate.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Mode {
    /// A comment for sending the work back.
    Changes,
    /// An optional note for ending the run.
    Reject,
    /// The gate's text, being rewritten.
    Edit,
}

/// A gate's draft at one place.
#[derive(Debug, Default)]
pub struct GateUi {
    pub mode: Option<Mode>,
    /// The saved rewrite: approving sends it, sending back carries it.
    pub edited: Option<String>,
    pub editor: Option<Editor>,
    /// A request or approval on its way; its button waits.
    pub sending: bool,
}

/// `gateMaxRounds`.
pub fn max_rounds(config: Option<&Value>) -> u64 {
    match config
        .and_then(|c| c.pointer("/feedback/maxRounds"))
        .and_then(Value::as_f64)
    {
        Some(n) if n.is_finite() => (n.floor() as i64).clamp(1, MAX_ROUNDS as i64) as u64,
        _ => DEFAULT_ROUNDS,
    }
}

fn round(state: &Value) -> u64 {
    state.get("round").and_then(Value::as_u64).unwrap_or(1)
}

fn sends_back(config: Option<&Value>) -> bool {
    config
        .and_then(|c| c.pointer("/feedback/from"))
        .and_then(Value::as_str)
        .is_some_and(|f| !f.is_empty())
}

/// `canRequestChanges`.
pub fn can_request_changes(config: Option<&Value>, state: &Value) -> bool {
    sends_back(config) && round(state) < max_rounds(config)
}

/// "round 2 of 3" on a gate that takes changes.
pub fn round_label(config: Option<&Value>, state: &Value) -> Option<String> {
    sends_back(config).then(|| format!("round {} of {}", round(state), max_rounds(config)))
}

/// `GateAsk`'s second line: the last change the reviewer asked for.
pub fn last_asked(state: &Value) -> Option<&str> {
    state
        .get("feedback")?
        .as_array()?
        .iter()
        .rev()
        .find(|f| s(f, "decision") == Some("changes"))
        .and_then(|f| s(f, "comment"))
}

/// The text the gate offers for rewriting, when it offers one.
pub fn editable(state: &Value) -> Option<&str> {
    s(state, "editableText")
}

/// `ComposerProps`' hint under the text.
pub fn composer_hint(mode: Mode, from: &str, state: &Value, config: Option<&Value>) -> String {
    match mode {
        Mode::Changes => format!(
            "Runs again from {from} with your comment, then asks you. Round {} of {}.",
            round(state) + 1,
            max_rounds(config)
        ),
        _ => "Ends the run. A note is kept as its reason.".to_owned(),
    }
}

/// `toItemList`: the list in `value`, and the key it sat under when it was
/// wrapped in an object.
pub fn item_list(value: &Value) -> Result<(Vec<Value>, Option<String>), String> {
    match value {
        Value::Array(items) => Ok((items.clone(), None)),
        Value::Object(map) => {
            let lists: Vec<(&String, &Value)> = map.iter().filter(|(_, v)| v.is_array()).collect();
            match lists.as_slice() {
                [(k, Value::Array(items))] => Ok((items.clone(), Some((*k).clone()))),
                [] => Err("Not a list: expected a JSON array, or an object holding one.".into()),
                many => Err(format!(
                    "Not a list: the object holds several ({}).",
                    many.iter()
                        .map(|(k, _)| k.as_str())
                        .collect::<Vec<_>>()
                        .join(", ")
                )),
            }
        }
        _ => Err("Not a list: expected a JSON array, or an object holding one.".into()),
    }
}

/// `isRecordList`.
pub fn is_record_list(items: &[Value]) -> bool {
    !items.is_empty() && items.iter().all(Value::is_object)
}

/// Whether the gate's text opens as a table.
pub fn is_table(text: &str) -> bool {
    let t = text.trim();
    if t.is_empty() {
        return false;
    }
    serde_json::from_str::<Value>(t)
        .ok()
        .and_then(|v| item_list(&v).ok())
        .is_some_and(|(items, _)| is_record_list(&items))
}

/// A row of the table.
#[derive(Debug, Clone)]
pub struct Row {
    pub values: Map<String, Value>,
    pub removed: bool,
    /// What was typed into number cells, by key, as typed.
    pub typed: BTreeMap<String, String>,
    pub had: HashSet<String>,
    pub nullable: HashSet<String>,
}

/// The rows a table draws and the object the list sat in.
#[derive(Debug, Clone)]
pub struct Table {
    pub rows: Vec<Row>,
    wrapper: Option<(Map<String, Value>, String)>,
}

/// Why text cannot be a table, and the line to point at.
#[derive(Debug, Clone, PartialEq)]
pub struct ParseError {
    pub error: String,
    pub line: Option<usize>,
}

/// `parseRows`.
pub fn parse_rows(text: &str) -> Result<Table, ParseError> {
    let data: Value = serde_json::from_str(text).map_err(|e| ParseError {
        error: format!("Line {}, column {}: {e}", e.line(), e.column()),
        line: Some(e.line()),
    })?;
    let (items, wrapper_key) =
        item_list(&data).map_err(|error| ParseError { error, line: None })?;
    if !items.is_empty() && !is_record_list(&items) {
        return Err(ParseError {
            error: "Not a table: every item has to be an object.".into(),
            line: None,
        });
    }
    let rows = items
        .into_iter()
        .filter_map(|v| match v {
            Value::Object(values) => Some(values),
            _ => None,
        })
        .map(|values| Row {
            had: values.keys().cloned().collect(),
            nullable: values
                .iter()
                .filter(|(_, v)| v.is_null())
                .map(|(k, _)| k.clone())
                .collect(),
            values,
            removed: false,
            typed: BTreeMap::new(),
        })
        .collect();
    let wrapper = match (wrapper_key, data) {
        (Some(k), Value::Object(o)) => Some((o, k)),
        _ => None,
    };
    Ok(Table { rows, wrapper })
}

/// `parseNumber`.
pub fn parse_number(text: &str) -> Option<Value> {
    let t = text.trim();
    if t.is_empty() {
        return None;
    }
    let n: f64 = t.parse().ok().filter(|n: &f64| n.is_finite())?;
    if n.fract() == 0.0 && n.abs() < 9.0e15 {
        Some(Value::from(n as i64))
    } else {
        serde_json::Number::from_f64(n).map(Value::Number)
    }
}

impl Table {
    /// Every key any row has, first seen first.
    pub fn columns(&self) -> Vec<String> {
        let mut out: Vec<String> = Vec::new();
        for r in &self.rows {
            for k in r.values.keys() {
                if !out.contains(k) {
                    out.push(k.clone());
                }
            }
        }
        out
    }

    /// The kept rows, wrapped back where they came from.
    pub fn value(&self) -> Value {
        let kept: Vec<Value> = self
            .rows
            .iter()
            .filter(|r| !r.removed)
            .map(|r| Value::Object(r.values.clone()))
            .collect();
        match &self.wrapper {
            Some((o, k)) => {
                let mut o = o.clone();
                o.insert(k.clone(), Value::Array(kept));
                Value::Object(o)
            }
            None => Value::Array(kept),
        }
    }

    /// The first kept cell holding something other than a number.
    pub fn bad_cell(&self) -> Option<String> {
        self.rows
            .iter()
            .enumerate()
            .filter(|(_, r)| !r.removed)
            .find_map(|(i, r)| {
                r.typed
                    .iter()
                    .find(|(_, t)| parse_number(t).is_none())
                    .map(|(k, _)| format!("Row {}, {k}: not a number.", i + 1))
            })
    }

    /// `Cell`'s onChange for a text cell.
    pub fn set_text(&mut self, row: usize, col: &str, text: &str) {
        let Some(r) = self.rows.get_mut(row) else {
            return;
        };
        if text.is_empty() && r.nullable.contains(col) {
            r.values.insert(col.to_owned(), Value::Null);
        } else if text.is_empty() && !r.had.contains(col) {
            r.values.remove(col);
        } else {
            r.values
                .insert(col.to_owned(), Value::String(text.to_owned()));
        }
    }

    /// A number cell: the text kept as typed, the number when it parses.
    pub fn set_number(&mut self, row: usize, col: &str, text: &str) {
        let Some(r) = self.rows.get_mut(row) else {
            return;
        };
        r.typed.insert(col.to_owned(), text.to_owned());
        if let Some(n) = parse_number(text) {
            r.values.insert(col.to_owned(), n);
        }
    }

    pub fn toggle_bool(&mut self, row: usize, col: &str) {
        if let Some(Value::Bool(b)) = self.rows.get_mut(row).and_then(|r| r.values.get_mut(col)) {
            *b = !*b;
        }
    }

    pub fn set_removed(&mut self, row: usize, removed: bool) {
        if let Some(r) = self.rows.get_mut(row) {
            r.removed = removed;
        }
    }

    pub fn kept(&self) -> usize {
        self.rows.iter().filter(|r| !r.removed).count()
    }
}

/// Same value whatever the spacing; text that does not parse only matches
/// itself.
pub fn canonical(text: &str) -> String {
    serde_json::from_str::<Value>(text)
        .map(|v| v.to_string())
        .unwrap_or_else(|_| text.to_owned())
}

/// `JSON.stringify(value, null, 2)`.
pub fn pretty(value: &Value) -> String {
    serde_json::to_string_pretty(value).unwrap_or_default()
}

/// The open editor: prose in a text area, or a list as a table with its
/// source one toggle away.
#[derive(Debug, Clone)]
pub struct Editor {
    pub original: String,
    /// `Some` for a list of records.
    pub table: Option<Table>,
    /// The source view (or prose) is showing.
    pub source: bool,
    /// The text area's text: prose, or the JSON source.
    pub text: String,
}

impl Editor {
    /// Opens on `initial` (a rewrite not yet sent, or the original).
    pub fn open(original: &str, initial: &str) -> Editor {
        if is_table(original) {
            let table = parse_rows(initial).ok();
            Editor {
                original: original.to_owned(),
                source: table.is_none(),
                table,
                text: initial.to_owned(),
            }
        } else {
            Editor {
                original: original.to_owned(),
                table: None,
                source: true,
                text: initial.to_owned(),
            }
        }
    }

    /// A JSON editor rather than prose.
    pub fn is_json(&self) -> bool {
        is_table(&self.original)
    }

    /// The source view's parse, when it shows.
    pub fn source_parse(&self) -> Option<Result<Table, ParseError>> {
        (self.is_json() && self.source).then(|| parse_rows(&self.text))
    }

    /// Why the draft cannot be saved or approved.
    pub fn invalid(&self) -> Option<String> {
        if !self.is_json() {
            return None;
        }
        if self.source {
            self.source_parse().and_then(|p| p.err()).map(|e| e.error)
        } else {
            self.table.as_ref().and_then(Table::bad_cell)
        }
    }

    /// The rewrite to send, or none when it says what the gate had.
    pub fn edited(&self) -> Option<String> {
        if !self.is_json() {
            return (self.text.trim() != self.original.trim()).then(|| self.text.clone());
        }
        let text = if self.source {
            match self.source_parse() {
                Some(Ok(_)) => serde_json::from_str::<Value>(&self.text)
                    .ok()
                    .map(|v| pretty(&v)),
                _ => None,
            }
        } else {
            self.table.as_ref().map(|t| pretty(&t.value()))
        }?;
        (canonical(&text) != canonical(&self.original)).then_some(text)
    }

    /// Switches the editor to the source view.
    pub fn to_source(&mut self) {
        if self.source {
            return;
        }
        if let Some(t) = &self.table {
            self.text = pretty(&t.value());
        }
        self.source = true;
    }

    /// Switches to the table view, only from source that parses.
    pub fn to_table(&mut self) -> bool {
        if !self.source {
            return false;
        }
        match parse_rows(&self.text) {
            Ok(t) => {
                self.table = Some(t);
                self.source = false;
                true
            }
            Err(_) => false,
        }
    }

    /// `revert`.
    pub fn revert(&mut self) {
        self.text = self.original.clone();
        if self.is_json() {
            self.table = parse_rows(&self.original).ok();
            self.source = self.table.is_none();
        }
    }

    /// "3 of 4 kept", or a dash for source that does not parse.
    pub fn count(&self) -> String {
        if self.source {
            match self.source_parse() {
                Some(Ok(t)) => format!("{0} of {0} kept", t.rows.len()),
                _ => "\u{2014}".into(),
            }
        } else {
            let t = self.table.as_ref();
            format!(
                "{} of {} kept",
                t.map_or(0, Table::kept),
                t.map_or(0, |t| t.rows.len())
            )
        }
    }

    /// The prose editor's word count.
    pub fn words(&self) -> usize {
        self.text.split_whitespace().count()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn rounds_follow_the_feedback_config() {
        let cfg = json!({ "feedback": { "from": "a", "maxRounds": 2 } });
        assert!(can_request_changes(Some(&cfg), &json!({ "round": 1 })));
        assert!(!can_request_changes(Some(&cfg), &json!({ "round": 2 })));
        assert!(!can_request_changes(Some(&json!({})), &json!({})));
        assert_eq!(
            round_label(Some(&cfg), &json!({})).as_deref(),
            Some("round 1 of 2")
        );
        assert_eq!(
            max_rounds(Some(&json!({ "feedback": { "maxRounds": 99 } }))),
            10
        );
        assert_eq!(max_rounds(None), 3);
    }

    #[test]
    fn lists_are_found_bare_or_wrapped() {
        assert!(is_table(r#"[{"a":1}]"#));
        assert!(is_table(r#"{"findings":[{"a":1}],"n":1}"#));
        assert!(!is_table(r#"{"a":[{"x":1}],"b":[]}"#));
        assert!(!is_table("[1,2]"));
        assert!(!is_table("plain words"));
    }

    #[test]
    fn a_table_trims_and_corrects_rows_without_breaking_the_json() {
        let original =
            r#"{"findings":[{"id":1,"ok":true,"note":null},{"id":2,"ok":false,"note":"x"}]}"#;
        let mut e = Editor::open(original, original);
        assert!(!e.source);
        assert_eq!(e.edited(), None, "opening edits nothing");
        let t = e.table.as_mut().expect("table");
        assert_eq!(t.columns(), ["id", "ok", "note"]);
        t.set_removed(1, true);
        t.toggle_bool(0, "ok");
        t.set_text(0, "note", "");
        assert_eq!(e.count(), "1 of 2 kept");
        let sent: Value = serde_json::from_str(&e.edited().expect("edited")).expect("json");
        assert_eq!(
            sent,
            json!({ "findings": [{ "id": 1, "ok": false, "note": null }] })
        );

        let t = e.table.as_mut().expect("table");
        t.set_number(0, "id", "1.");
        assert_eq!(e.invalid(), None, "1. parses");
        e.table.as_mut().expect("table").set_number(0, "id", "-");
        assert_eq!(e.invalid().as_deref(), Some("Row 1, id: not a number."));
        e.revert();
        assert_eq!(e.edited(), None);
    }

    #[test]
    fn source_that_does_not_parse_keeps_the_table_shut() {
        let original = r#"[{"a":"b"}]"#;
        let mut e = Editor::open(original, original);
        e.to_source();
        e.text = "[{\"a\":".into();
        assert!(e.invalid().expect("invalid").starts_with("Line 1"));
        assert!(!e.to_table());
        assert_eq!(e.count(), "\u{2014}");
        e.text = r#"[{"a":"c"},{"a":"d"}]"#.into();
        assert!(e.to_table());
        assert_eq!(e.count(), "2 of 2 kept");
    }

    #[test]
    fn prose_is_edited_when_its_words_change() {
        let mut e = Editor::open("Ship it", "Ship it");
        assert!(!e.is_json());
        e.text = "Ship it ".into();
        assert_eq!(e.edited(), None, "trailing space is no edit");
        e.text = "Ship it now".into();
        assert_eq!(e.words(), 3);
        assert_eq!(e.edited().as_deref(), Some("Ship it now"));
    }
}

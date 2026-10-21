//! What Tasks draws: the board, whose cards the client reads from the
//! core's task list and draws itself, and the badge on the card of each
//! session working on a task.

use vorn_extension_sdk::tree::sdk::{board_of, col, icon, lane, row, text};
use vorn_extension_sdk::tree::{Cards, Node, TextStyle, Tone};
use vorn_extension_sdk::Value;

/// The board's columns: a status, its title, icon and tone.
pub(crate) const LANES: [(&str, &str, &str, Tone); 5] = [
    ("todo", "Todo", "circle", Tone::Idle),
    ("in_progress", "In Progress", "clock", Tone::Accent),
    ("in_review", "In Review", "eye", Tone::Busy),
    ("done", "Done", "circle-check", Tone::Good),
    ("cancelled", "Cancelled", "circle-x", Tone::Muted),
];

/// The board: a column per status, a card per task.
pub fn board() -> Node {
    board_of(
        Cards::Tasks,
        LANES
            .iter()
            .map(|(value, title, glyph, tone)| lane(*value, *title).icon(*glyph).tone(*tone)),
    )
    .empty("Drop tasks here")
}

/// One badge per row of `working`, each the child of the session it shows on.
pub fn badges<'a>(working: impl Iterator<Item = &'a Value>) -> Node {
    col(working.filter_map(|r| {
        let session = r.get("id")?.as_str()?;
        let title = r.get("title").and_then(Value::as_str).unwrap_or_default();
        Some(
            row([icon("list-todo"), text(title).style(TextStyle::Secondary)])
                .gap(8.0)
                .id(session),
        )
    }))
}

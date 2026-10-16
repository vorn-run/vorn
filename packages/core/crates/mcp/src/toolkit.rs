//! What a tool another crate contributes is written with: the server it
//! calls through, its parsed arguments, and the shapes of an answer, so a
//! built-in extension's tools answer as Vorn's own do.

pub use crate::tools::{
    agents_off, error_of, failed, items, object, pretty, text, Args, Cx, Outcome,
};

/// `answer.key` as JavaScript destructures it: a TypeError on `null`.
pub fn destructure<'a>(
    answer: &'a serde_json::Value,
    key: &str,
) -> Result<Option<&'a serde_json::Value>, String> {
    crate::tools::destructure(answer, key)
}

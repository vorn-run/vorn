//! Text a program in a terminal puts on the clipboard with an escape code
//! (OSC 52): whether it lands, is asked about, or is dropped.
//!
//! Each place a program can run has its own choice, kept in `defaults`:
//! this machine under [`LOCAL_KEY`], a remote host under [`host_key`]. A
//! program on this machine may copy unless told not to; one on a remote host
//! is asked about first. A copy that is allowed from a card the person is not
//! typing into still lands, and says so.

use serde_json::Value;

/// `defaults` key of this machine's choice.
pub const LOCAL_KEY: &str = "clipboardFromPrograms";

/// `defaults` key of the choice for the remote host `id`.
pub fn host_key(id: &str) -> String {
    format!("clipboardFromHost:{id}")
}

/// What a place's programs may do with the clipboard.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Policy {
    Allow,
    Ask,
    Never,
}

impl Policy {
    /// The stored word, else `fallback`.
    fn read(v: Option<&Value>, fallback: Policy) -> Policy {
        match v.and_then(Value::as_str) {
            Some("allow") => Policy::Allow,
            Some("ask") => Policy::Ask,
            Some("off") => Policy::Never,
            _ => fallback,
        }
    }
}

/// The options a settings row offers, as `(label, stored word)`.
pub const OPTIONS: [(&str, &str); 3] = [("Allow", "allow"), ("Ask", "ask"), ("Never", "off")];

/// The choice for a session on `host` (a remote host's id), or on this machine with none.
pub fn policy(defaults: Option<&Value>, host: Option<&str>) -> Policy {
    let at = |key: &str| defaults.and_then(|d| d.get(key));
    match host.filter(|h| !h.is_empty()) {
        Some(id) => Policy::read(at(&host_key(id)), Policy::Ask),
        None => Policy::read(at(LOCAL_KEY), Policy::Allow),
    }
}

/// What to do with one copy.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Decision {
    /// Onto the clipboard, quietly: the person is typing into that card.
    Copy,
    /// Onto the clipboard, with a toast naming who copied.
    CopyAndTell,
    /// Asked about first.
    Ask,
    Drop,
}

/// The decision under `policy` for a card that is or is not the one the
/// person types into (`foreground`).
pub fn decide(policy: Policy, foreground: bool) -> Decision {
    match (policy, foreground) {
        (Policy::Allow, true) => Decision::Copy,
        (Policy::Allow, false) => Decision::CopyAndTell,
        (Policy::Ask, _) => Decision::Ask,
        (Policy::Never, _) => Decision::Drop,
    }
}

/// A copy's size for a sentence: `12 characters`, `1 line`, `3 lines`.
pub fn size(text: &str) -> String {
    let lines = text.lines().count();
    if lines > 1 {
        format!("{lines} lines")
    } else {
        let n = text.chars().count();
        format!("{n} character{}", if n == 1 { "" } else { "s" })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn this_machine_may_copy_and_a_remote_host_is_asked_unless_told() {
        let none = json!({});
        assert_eq!(policy(Some(&none), None), Policy::Allow);
        assert_eq!(policy(Some(&none), Some("h1")), Policy::Ask);
        assert_eq!(policy(None, Some("")), Policy::Allow);
        let set = json!({ LOCAL_KEY: "off", "clipboardFromHost:h1": "allow", "clipboardFromHost:h2": "x" });
        assert_eq!(policy(Some(&set), None), Policy::Never);
        assert_eq!(policy(Some(&set), Some("h1")), Policy::Allow);
        assert_eq!(policy(Some(&set), Some("h2")), Policy::Ask);
    }

    #[test]
    fn a_copy_from_a_card_in_the_background_lands_and_says_so() {
        assert_eq!(decide(Policy::Allow, true), Decision::Copy);
        assert_eq!(decide(Policy::Allow, false), Decision::CopyAndTell);
        assert_eq!(decide(Policy::Ask, true), Decision::Ask);
        assert_eq!(decide(Policy::Never, true), Decision::Drop);
    }

    #[test]
    fn says_how_much_was_copied() {
        assert_eq!(size("a"), "1 character");
        assert_eq!(size("héllo"), "5 characters");
        assert_eq!(size("a\nb\nc"), "3 lines");
    }
}

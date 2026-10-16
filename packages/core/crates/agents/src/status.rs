//! An agent's screen rules: what its screen shows when it is working,
//! waiting for input or idle, as its adapter's `[[agent.status]]` entries.
//!
//! Where no rule holds, the generic output patterns decide (errors come only
//! from them), hooks decide over both, and an agent reporting its own status
//! (OSC 7501) over everything.

use std::collections::HashMap;
use std::sync::{Mutex, OnceLock};

use vorn_analysis::rules::{RuleError, RuleSpec, Rules};

/// The rules `specs` compile to for `agent`, kept for the life of the
/// process: a session holds them by reference while it runs, and an
/// adapter read again with the same rules gets the same ones back, so a
/// reload costs nothing new.
pub fn intern(agent: &str, specs: &[RuleSpec]) -> Result<&'static Rules, RuleError> {
    static KEPT: OnceLock<Mutex<HashMap<String, &'static Rules>>> = OnceLock::new();
    let key = format!("{agent}\0{specs:?}");
    let kept = KEPT.get_or_init(Mutex::default);
    if let Some(rules) = kept.lock().unwrap_or_else(|e| e.into_inner()).get(&key) {
        return Ok(rules);
    }
    let rules: &'static Rules = Box::leak(Box::new(Rules::new(agent, specs)?));
    Ok(*kept
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .entry(key)
        .or_insert(rules))
}

#[cfg(test)]
mod tests {
    use vorn_analysis::rules::ScreenStatus;

    use super::*;

    fn spec(pattern: &str) -> RuleSpec {
        RuleSpec {
            status: ScreenStatus::Idle,
            rows: None,
            cursor_rows: None,
            any: vec![pattern.to_owned()],
            all: Vec::new(),
            none: Vec::new(),
            cursor: None,
            title: None,
        }
    }

    #[test]
    fn the_same_rules_are_kept_once() {
        let a = intern("x", &[spec(">")]).unwrap();
        let b = intern("x", &[spec(">")]).unwrap();
        assert!(std::ptr::eq(a, b));
        let c = intern("x", &[spec("<")]).unwrap();
        assert!(!std::ptr::eq(a, c));
        assert_eq!(c.agent(), "x");
        assert!(intern("x", &[spec("(")]).is_err());
        assert!(intern("x", &[]).is_err());
    }
}

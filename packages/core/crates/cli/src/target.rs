//! The one rule every command uses to turn what a person typed into a session.
//!
//! Tried in order, the first step that matches anything decides:
//!
//! 1. the whole id;
//! 2. the label (display name), exactly, then ignoring case;
//! 3. a position, counting from 1 in `vorn session list` order;
//! 4. the start or the end of an id.
//!
//! A step that matches more than one session is [`Miss::Ambiguous`]; the
//! command exits 3 and lists the candidates rather than guessing.

use serde_json::{json, Value};

use crate::js::field;
use crate::output::short_id;

/// A session a target may name, in list order.
#[derive(Debug, Clone, PartialEq)]
pub struct Candidate {
    pub id: String,
    pub label: String,
    pub agent: String,
    pub project: String,
    /// A headless run has no terminal.
    pub headless: bool,
    pub created: f64,
}

impl Candidate {
    /// From a `terminal:listActive` or `headless:list` record.
    pub fn from_record(v: &Value, headless: bool) -> Candidate {
        let s = |k| field(v, k).and_then(Value::as_str).unwrap_or("").to_owned();
        Candidate {
            id: s("id"),
            label: s("displayName"),
            agent: s("agentType"),
            project: s("projectName"),
            headless,
            created: field(v, "createdAt").and_then(Value::as_f64).unwrap_or(0.0),
        }
    }

    /// The candidate as `--json` data.
    pub fn to_json(&self, position: usize) -> Value {
        json!({
            "position": position, "id": self.id, "label": self.label, "agent": self.agent,
            "project": self.project, "headless": self.headless,
        })
    }
}

/// Why a target named no single session.
#[derive(Debug, Clone, PartialEq)]
pub enum Miss {
    None,
    /// The candidates, with their positions in the list.
    Ambiguous(Vec<(usize, Candidate)>),
}

fn pick(list: &[Candidate], hit: impl Fn(&Candidate) -> bool) -> Option<Result<&Candidate, Miss>> {
    let found: Vec<(usize, &Candidate)> = list.iter().enumerate().filter(|(_, c)| hit(c)).collect();
    match found.as_slice() {
        [] => None,
        [(_, one)] => Some(Ok(one)),
        many => Some(Err(Miss::Ambiguous(
            many.iter().map(|(i, c)| (i + 1, (*c).clone())).collect(),
        ))),
    }
}

/// The session `given` names in `list`.
pub fn resolve<'a>(given: &str, list: &'a [Candidate]) -> Result<&'a Candidate, Miss> {
    if given.is_empty() {
        return Err(Miss::None);
    }
    if let Some(c) = list.iter().find(|c| c.id == given) {
        return Ok(c);
    }
    if let Some(done) = pick(list, |c| c.label == given)
        .or_else(|| pick(list, |c| c.label.eq_ignore_ascii_case(given)))
    {
        return done;
    }
    if given.bytes().all(|b| b.is_ascii_digit()) {
        if let Some(c) = given
            .parse::<usize>()
            .ok()
            .and_then(|n| n.checked_sub(1))
            .and_then(|i| list.get(i))
        {
            return Ok(c);
        }
    }
    pick(list, |c| c.id.starts_with(given) || c.id.ends_with(given)).unwrap_or(Err(Miss::None))
}

/// The ambiguity as lines for a person.
pub fn describe(given: &str, found: &[(usize, Candidate)]) -> String {
    let mut out = format!(
        "\"{given}\" matches {} sessions; name one by id:\n",
        found.len()
    );
    for (position, c) in found {
        out.push_str(&format!(
            "  {position:>2}  {}  {}  {}  {}\n",
            short_id(&c.id),
            c.agent,
            c.project,
            c.label
        ));
    }
    out
}

/// The ambiguity as `--json` data.
pub fn describe_json(given: &str, found: &[(usize, Candidate)]) -> Value {
    json!({
        "error": "ambiguous", "target": given,
        "candidates": found.iter().map(|(i, c)| c.to_json(*i)).collect::<Vec<_>>(),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn c(id: &str, label: &str) -> Candidate {
        Candidate {
            id: id.into(),
            label: label.into(),
            agent: "claude".into(),
            project: "p".into(),
            headless: false,
            created: 0.0,
        }
    }

    fn ids(m: Miss) -> Vec<usize> {
        match m {
            Miss::Ambiguous(f) => f.iter().map(|(i, _)| *i).collect(),
            Miss::None => vec![],
        }
    }

    #[test]
    fn an_exact_id_beats_a_label_and_a_position() {
        let list = [c("aaa111", "2"), c("2", "x")];
        assert_eq!(resolve("2", &list).unwrap().label, "x");
    }

    #[test]
    fn a_label_beats_a_position_and_two_of_them_are_ambiguous() {
        let list = [c("aaa111", "build"), c("bbb222", "Build"), c("ccc333", "2")];
        assert_eq!(resolve("build", &list).unwrap().id, "aaa111");
        assert_eq!(resolve("BUILD", &list).map_err(ids), Err(vec![1, 2]));
        assert_eq!(resolve("2", &list).unwrap().id, "ccc333");
    }

    #[test]
    fn a_position_counts_from_one_and_falls_through_when_out_of_range() {
        let list = [c("aaa111", ""), c("9bbb22", "")];
        assert_eq!(resolve("2", &list).unwrap().id, "9bbb22");
        assert_eq!(resolve("9", &list).unwrap().id, "9bbb22");
        assert_eq!(resolve("0", &list), Err(Miss::None));
    }

    #[test]
    fn an_id_start_or_end_must_be_unique() {
        let list = [c("abc111", ""), c("abd222", ""), c("zzz1ab", "")];
        assert_eq!(resolve("abc", &list).unwrap().id, "abc111");
        assert_eq!(resolve("zzz", &list).unwrap().id, "zzz1ab");
        assert_eq!(resolve("ab", &list).map_err(ids), Err(vec![1, 2, 3]));
        assert_eq!(resolve("nope", &list), Err(Miss::None));
        assert_eq!(resolve("", &list), Err(Miss::None));
    }
}

//! A terminal's program status records (OSC 7501) as clients are told them,
//! and the status they decide while the program reports its own.

use serde::{Deserialize, Serialize};
use vorn_engine::{BlockedKind, ProgramRecord, ProgramState, Programs};

use super::AgentStatus;

/// `ProgramStatus` in packages/shared.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ProgramStatus {
    /// The program reports its own status: hooks and the screen do not decide it.
    pub native: bool,
    /// The root record (id "") first, each child after its parent.
    pub records: Vec<ProgramEntry>,
    /// It finished and nobody has looked at it since.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub unseen: bool,
}

/// `ProgramRecord` in packages/shared.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ProgramEntry {
    pub id: String,
    pub state: EntryState,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub kind: Option<WaitKind>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub progress: Option<u8>,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub app: String,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub title: String,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub msg: String,
}

/// `ProgramRecord['state']` in packages/shared.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum EntryState {
    Idle,
    Working,
    Done,
    Blocked,
    Error,
}

/// `ProgramRecord['kind']` in packages/shared: what a blocked program waits for.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum WaitKind {
    Permission,
    Question,
    Auth,
}

impl ProgramStatus {
    /// What the engine's records say, given what was told before: `None`
    /// when there is nothing to show.
    pub fn read(p: &Programs, was: Option<&ProgramStatus>) -> Option<ProgramStatus> {
        if !p.native && p.records.is_empty() {
            return None;
        }
        let mut next = ProgramStatus {
            native: p.native,
            records: p.records.iter().filter_map(entry).collect(),
            unseen: false,
        };
        let done = next.state() == Some(EntryState::Done);
        let was_done = was.and_then(ProgramStatus::state) == Some(EntryState::Done);
        next.unseen = done && (!was_done || was.is_some_and(|w| w.unseen));
        Some(next)
    }

    /// The root record's state, or the most pressing one of the rest.
    pub fn state(&self) -> Option<EntryState> {
        if let Some(root) = self.records.iter().find(|r| r.id.is_empty()) {
            return Some(root.state);
        }
        self.records.iter().map(|r| r.state).max_by_key(|s| match s {
            EntryState::Idle => 0,
            EntryState::Done => 1,
            EntryState::Working => 2,
            EntryState::Error => 3,
            EntryState::Blocked => 4,
        })
    }

    /// The status the records decide, while the program reports its own.
    pub fn status(&self) -> Option<AgentStatus> {
        if !self.native {
            return None;
        }
        Some(match self.state()? {
            EntryState::Working => AgentStatus::Running,
            EntryState::Blocked => AgentStatus::Waiting,
            EntryState::Error => AgentStatus::Error,
            EntryState::Done | EntryState::Idle => AgentStatus::Idle,
        })
    }

    /// What a blocked program waits for, as its most pressing record says.
    pub fn waiting_for(&self) -> Option<WaitKind> {
        let blocked = |r: &&ProgramEntry| r.state == EntryState::Blocked;
        let root = self.records.iter().find(|r| r.id.is_empty());
        root.filter(blocked)
            .or_else(|| self.records.iter().find(blocked))
            .and_then(|r| r.kind)
    }

    /// The root record's message, or the first one a record has.
    pub fn message(&self) -> Option<&str> {
        self.records
            .iter()
            .map(|r| r.msg.as_str())
            .find(|m| !m.is_empty())
    }
}

fn entry(r: &ProgramRecord) -> Option<ProgramEntry> {
    let state = match r.state {
        ProgramState::Idle => EntryState::Idle,
        ProgramState::Working => EntryState::Working,
        ProgramState::Done => EntryState::Done,
        ProgramState::Blocked => EntryState::Blocked,
        ProgramState::Error => EntryState::Error,
        ProgramState::Clear => return None,
    };
    Some(ProgramEntry {
        id: shown(&r.id),
        state,
        kind: r.kind.map(|k| match k {
            BlockedKind::Permission => WaitKind::Permission,
            BlockedKind::Question => WaitKind::Question,
            BlockedKind::Auth => WaitKind::Auth,
        }),
        progress: r.progress.map(|p| p.min(100)),
        app: shown(&r.app),
        title: shown(&r.title),
        msg: shown(&r.msg),
    })
}

/// `s` without the invisible and direction-changing characters a program
/// could use to make what it reports read as something else.
pub fn shown(s: &str) -> String {
    s.chars()
        .filter(|&c| {
            !c.is_control()
                && !matches!(c,
                    '\u{00AD}' | '\u{061C}' | '\u{180E}' | '\u{200B}'..='\u{200F}'
                    | '\u{202A}'..='\u{202E}' | '\u{2060}'..='\u{2069}' | '\u{FEFF}')
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn rec(id: &str, state: ProgramState) -> ProgramRecord {
        ProgramRecord {
            id: id.into(),
            state,
            kind: None,
            progress: None,
            app: String::new(),
            title: String::new(),
            msg: String::new(),
            from_progress: false,
        }
    }

    fn programs(native: bool, records: Vec<ProgramRecord>) -> Programs {
        Programs { native, records }
    }

    #[test]
    fn the_root_decides_and_children_only_without_one() {
        let p = programs(true, vec![rec("", ProgramState::Working), rec("a", ProgramState::Blocked)]);
        let s = ProgramStatus::read(&p, None).unwrap();
        assert_eq!(s.status(), Some(AgentStatus::Running));
        let p = programs(true, vec![rec("a", ProgramState::Working), rec("b", ProgramState::Blocked)]);
        let s = ProgramStatus::read(&p, None).unwrap();
        assert_eq!(s.status(), Some(AgentStatus::Waiting));
        let p = programs(false, vec![rec("", ProgramState::Working)]);
        assert_eq!(ProgramStatus::read(&p, None).unwrap().status(), None);
        assert_eq!(ProgramStatus::read(&programs(false, vec![]), None), None);
    }

    #[test]
    fn done_stays_unseen_until_seen() {
        let p = programs(true, vec![rec("", ProgramState::Done)]);
        let mut s = ProgramStatus::read(&p, None).unwrap();
        assert!(s.unseen);
        assert!(ProgramStatus::read(&p, Some(&s)).unwrap().unseen);
        s.unseen = false;
        assert!(!ProgramStatus::read(&p, Some(&s)).unwrap().unseen);
        let working = programs(true, vec![rec("", ProgramState::Working)]);
        let w = ProgramStatus::read(&working, Some(&s)).unwrap();
        assert!(!w.unseen);
        assert!(ProgramStatus::read(&p, Some(&w)).unwrap().unseen);
    }

    #[test]
    fn hidden_characters_are_not_shown() {
        assert_eq!(shown("ok\u{202E}gnp.exe\u{200B}"), "okgnp.exe");
    }
}

//! A session's program status records (OSC 7501), kept as the protocol says:
//! one record per id, each report replacing its record, a clear removing a
//! record and every record beneath it, working, blocked and idle records
//! dropped at the next prompt or when the program exits, and at most
//! [`MAX_RECORDS`], the one updated longest ago going first. Until the program
//! sends its first report, its OSC 9;4 progress stands in as the root record.

use vorn_screen::{BlockedKind, ProgramReport, ProgramState, ProgressState};

/// The most records a session keeps.
pub const MAX_RECORDS: usize = 256;

/// One record as clients see it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ProgramRecord {
    /// "" for the program itself; "a/b" is a child of "a".
    pub id: String,
    /// Never [`ProgramState::Clear`].
    pub state: ProgramState,
    pub kind: Option<BlockedKind>,
    pub progress: Option<u8>,
    /// The record's app, or its nearest ancestor's.
    pub app: String,
    pub title: String,
    pub msg: String,
    /// Read from OSC 9;4 progress, not reported.
    pub from_progress: bool,
}

/// What a session's records stand at: an [`crate::Effect::Program`] state.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Programs {
    /// Whether the program has reported since its last prompt or exit: while
    /// it has, its records decide its status rather than hooks or the screen.
    pub native: bool,
    /// Sorted by id, so the root comes first and children follow parents.
    pub records: Vec<ProgramRecord>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct Stored {
    rec: ProgramRecord,
    /// The record's own app, before inheritance.
    app: String,
    updated: u64,
}

/// The records and what keeps them in order.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub(crate) struct Store {
    native: bool,
    records: Vec<Stored>,
    clock: u64,
    changed: bool,
}

impl Store {
    pub(crate) fn report(&mut self, r: ProgramReport) {
        self.native = true;
        self.changed = true;
        if r.state == ProgramState::Clear {
            if r.id.is_empty() {
                self.records.clear();
            } else {
                self.records.retain(|s| !within(&s.rec.id, &r.id));
            }
            return;
        }
        let rec = ProgramRecord {
            id: r.id,
            state: r.state,
            kind: if r.state == ProgramState::Blocked {
                r.kind
            } else {
                None
            },
            progress: r.progress,
            app: String::new(),
            title: r.title,
            msg: r.msg,
            from_progress: false,
        };
        self.put(rec, r.app);
    }

    /// An OSC 9;4 report: the root record's progress until the program reports itself.
    pub(crate) fn progress(&mut self, state: ProgressState, percent: Option<u8>) {
        if self.native {
            return;
        }
        let root = self.records.iter().position(|s| s.rec.id.is_empty());
        if root.is_some_and(|i| !self.records[i].rec.from_progress) {
            return;
        }
        let state = match state {
            ProgressState::Remove => {
                if let Some(i) = root {
                    self.records.remove(i);
                    self.changed = true;
                }
                return;
            }
            ProgressState::Error => ProgramState::Error,
            ProgressState::Set | ProgressState::Indeterminate | ProgressState::Pause => {
                ProgramState::Working
            }
        };
        let rec = ProgramRecord {
            id: String::new(),
            state,
            kind: None,
            progress: if state == ProgramState::Working {
                percent
            } else {
                None
            },
            app: String::new(),
            title: String::new(),
            msg: String::new(),
            from_progress: true,
        };
        if root.is_some_and(|i| self.records[i].rec == rec) {
            return;
        }
        self.changed = true;
        self.put(rec, String::new());
    }

    /// A new prompt or the program's exit: what was running is not any more.
    pub(crate) fn ended(&mut self) {
        let before = self.records.len();
        self.records.retain(|s| {
            matches!(s.rec.state, ProgramState::Done | ProgramState::Error) && !s.rec.from_progress
        });
        if self.native || self.records.len() != before {
            self.changed = true;
        }
        self.native = false;
    }

    /// The records when they changed since the last call.
    pub(crate) fn take_changed(&mut self) -> Option<Programs> {
        std::mem::take(&mut self.changed).then(|| self.snapshot())
    }

    pub(crate) fn snapshot(&self) -> Programs {
        let mut records: Vec<ProgramRecord> = self
            .records
            .iter()
            .map(|s| {
                let mut rec = s.rec.clone();
                rec.app = self.app_of(&s.rec.id, &s.app).to_owned();
                rec
            })
            .collect();
        records.sort_by(|a, b| a.id.cmp(&b.id));
        Programs {
            native: self.native,
            records,
        }
    }

    fn put(&mut self, rec: ProgramRecord, app: String) {
        self.clock += 1;
        let updated = self.clock;
        if let Some(s) = self.records.iter_mut().find(|s| s.rec.id == rec.id) {
            *s = Stored { rec, app, updated };
            return;
        }
        if self.records.len() >= MAX_RECORDS {
            if let Some(oldest) = (0..self.records.len()).min_by_key(|&i| self.records[i].updated)
            {
                self.records.swap_remove(oldest);
            }
        }
        self.records.push(Stored { rec, app, updated });
    }

    fn app_of<'a>(&'a self, id: &str, own: &'a str) -> &'a str {
        if !own.is_empty() || id.is_empty() {
            return own;
        }
        let parent = id.rsplit_once('/').map_or("", |(p, _)| p);
        match self.records.iter().find(|s| s.rec.id == parent) {
            Some(s) => self.app_of(parent, &s.app),
            None => self.app_of(parent, ""),
        }
    }

    pub(crate) fn encode(&self, out: &mut Vec<u8>) {
        out.push(u8::from(self.native));
        out.extend_from_slice(&self.clock.to_le_bytes());
        out.extend_from_slice(&(self.records.len() as u16).to_le_bytes());
        for s in &self.records {
            let r = &s.rec;
            out.push(state_code(r.state));
            out.push(r.kind.map_or(0, kind_code));
            out.push(r.progress.unwrap_or(u8::MAX));
            out.push(u8::from(r.from_progress));
            out.extend_from_slice(&s.updated.to_le_bytes());
            for text in [&r.id, &s.app, &r.title, &r.msg] {
                out.extend_from_slice(&(text.len() as u16).to_le_bytes());
                out.extend_from_slice(text.as_bytes());
            }
        }
    }

    pub(crate) fn decode(mut b: &[u8]) -> Option<Store> {
        let native = match take(&mut b, 1)?[0] {
            0 => false,
            1 => true,
            _ => return None,
        };
        let clock = u64::from_le_bytes(take(&mut b, 8)?.try_into().ok()?);
        let n = u16::from_le_bytes(take(&mut b, 2)?.try_into().ok()?);
        if usize::from(n) > MAX_RECORDS {
            return None;
        }
        let mut records = Vec::with_capacity(usize::from(n));
        for _ in 0..n {
            let head = take(&mut b, 4)?;
            let state = match head[0] {
                0 => ProgramState::Idle,
                1 => ProgramState::Working,
                2 => ProgramState::Done,
                3 => ProgramState::Blocked,
                4 => ProgramState::Error,
                _ => return None,
            };
            let kind = match head[1] {
                0 => None,
                1 => Some(BlockedKind::Permission),
                2 => Some(BlockedKind::Question),
                3 => Some(BlockedKind::Auth),
                _ => return None,
            };
            let progress = match head[2] {
                u8::MAX => None,
                p if p <= 100 => Some(p),
                _ => return None,
            };
            let from_progress = head[3] == 1;
            let updated = u64::from_le_bytes(take(&mut b, 8)?.try_into().ok()?);
            let mut text = || -> Option<String> {
                let len = u16::from_le_bytes(take(&mut b, 2)?.try_into().ok()?);
                String::from_utf8(take(&mut b, usize::from(len))?.to_vec()).ok()
            };
            let (id, app, title, msg) = (text()?, text()?, text()?, text()?);
            records.push(Stored {
                rec: ProgramRecord {
                    id,
                    state,
                    kind,
                    progress,
                    app: String::new(),
                    title,
                    msg,
                    from_progress,
                },
                app,
                updated,
            });
        }
        b.is_empty().then_some(Store {
            native,
            records,
            clock,
            changed: false,
        })
    }
}

/// Whether `id` is `root` or beneath it.
fn within(id: &str, root: &str) -> bool {
    id.strip_prefix(root)
        .is_some_and(|rest| rest.is_empty() || rest.starts_with('/'))
}

fn take<'a>(b: &mut &'a [u8], n: usize) -> Option<&'a [u8]> {
    if b.len() < n {
        return None;
    }
    let (head, rest) = b.split_at(n);
    *b = rest;
    Some(head)
}

fn state_code(s: ProgramState) -> u8 {
    match s {
        ProgramState::Idle | ProgramState::Clear => 0,
        ProgramState::Working => 1,
        ProgramState::Done => 2,
        ProgramState::Blocked => 3,
        ProgramState::Error => 4,
    }
}

fn kind_code(k: BlockedKind) -> u8 {
    match k {
        BlockedKind::Permission => 1,
        BlockedKind::Question => 2,
        BlockedKind::Auth => 3,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn report(state: ProgramState, id: &str) -> ProgramReport {
        ProgramReport {
            state,
            kind: None,
            progress: None,
            id: id.into(),
            app: String::new(),
            title: String::new(),
            msg: String::new(),
        }
    }

    fn ids(s: &Store) -> Vec<String> {
        s.snapshot().records.into_iter().map(|r| r.id).collect()
    }

    #[test]
    fn reports_replace_and_clears_remove_subtrees() {
        let mut s = Store::default();
        s.report(ProgramReport {
            app: "deploy".into(),
            msg: "rolling out".into(),
            ..report(ProgramState::Working, "")
        });
        s.report(report(ProgramState::Working, "us-east"));
        s.report(report(ProgramState::Working, "us-east/db"));
        s.report(report(ProgramState::Working, "us-eastern"));
        let snap = s.take_changed().unwrap();
        assert!(snap.native);
        assert_eq!(snap.records[0].msg, "rolling out");
        assert_eq!(snap.records[2].app, "deploy", "inherited from the root");
        // A report replaces its record: the message is gone.
        s.report(report(ProgramState::Done, ""));
        assert_eq!(s.snapshot().records[0].msg, "");
        s.report(report(ProgramState::Clear, "us-east"));
        assert_eq!(ids(&s), ["", "us-eastern"]);
        s.report(report(ProgramState::Clear, ""));
        assert!(ids(&s).is_empty());
        assert!(s.take_changed().is_some());
        assert!(s.take_changed().is_none());
    }

    #[test]
    fn a_prompt_or_exit_drops_what_was_running() {
        let mut s = Store::default();
        for (state, id) in [
            (ProgramState::Working, "a"),
            (ProgramState::Blocked, "b"),
            (ProgramState::Idle, "c"),
            (ProgramState::Done, "d"),
            (ProgramState::Error, "e"),
        ] {
            s.report(report(state, id));
        }
        s.ended();
        assert_eq!(ids(&s), ["d", "e"]);
        assert!(!s.snapshot().native);
    }

    #[test]
    fn the_oldest_record_goes_past_the_limit() {
        let mut s = Store::default();
        for i in 0..MAX_RECORDS {
            s.report(report(ProgramState::Working, &format!("r{i}")));
        }
        s.report(report(ProgramState::Working, "r0"));
        s.report(report(ProgramState::Working, "new"));
        let ids = ids(&s);
        assert_eq!(ids.len(), MAX_RECORDS);
        assert!(ids.contains(&"r0".to_owned()) && !ids.contains(&"r1".to_owned()));
    }

    #[test]
    fn osc_progress_stands_in_until_the_first_report() {
        let mut s = Store::default();
        s.progress(ProgressState::Set, Some(30));
        let snap = s.take_changed().unwrap();
        assert!(!snap.native);
        assert_eq!(snap.records[0].progress, Some(30));
        assert!(snap.records[0].from_progress);
        s.progress(ProgressState::Set, Some(30));
        assert!(s.take_changed().is_none());
        s.report(report(ProgramState::Working, ""));
        s.progress(ProgressState::Set, Some(90));
        assert_eq!(s.snapshot().records[0].progress, None);
        assert!(!s.snapshot().records[0].from_progress);
    }

    #[test]
    fn records_round_trip_and_damage_is_refused() {
        let mut s = Store::default();
        s.report(ProgramReport {
            kind: Some(BlockedKind::Auth),
            progress: Some(5),
            app: "x".into(),
            title: "t".into(),
            msg: "é".into(),
            ..report(ProgramState::Blocked, "a/b")
        });
        s.take_changed();
        let mut b = Vec::new();
        s.encode(&mut b);
        assert_eq!(Store::decode(&b), Some(s));
        for n in 0..b.len() {
            assert!(Store::decode(&b[..n]).is_none(), "{n}");
        }
    }
}

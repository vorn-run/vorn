//! The shipped adapters' screen rules read against fixtures: a screen each
//! agent shows in each state, and a recorded session replayed through the
//! emulator.
//!
//! A fixture is `agents/<name>/fixtures/status/<state>-<name>.toml`:
//!
//! ```toml
//! status = "working"            # what the rules must read
//! cursor = { row = 3, col = 4 } # where a shown cursor is; absent, it is hidden
//! title = "..."                 # the window title, if any
//! screen = '''
//! ...one line per screen row...
//! '''
//! ```

mod support;

use std::collections::BTreeSet;
use std::path::PathBuf;

use serde::Deserialize;
use vorn_agents::Agent;
use vorn_analysis::rules::{Cursor, Rules, Screen, ScreenStatus};
use vorn_screen::Emulator;
use vorn_term_proto::Record;

const STATES: [ScreenStatus; 3] = [
    ScreenStatus::Working,
    ScreenStatus::Waiting,
    ScreenStatus::Idle,
];

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Fixture {
    status: ScreenStatus,
    cursor: Option<At>,
    #[serde(default)]
    title: String,
    screen: String,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct At {
    row: u16,
    col: u16,
}

fn fixtures(agent: &Agent) -> Vec<PathBuf> {
    let dir = support::package(agent.id()).join("fixtures/status");
    let mut files: Vec<PathBuf> = std::fs::read_dir(&dir)
        .unwrap_or_else(|e| panic!("{}: {e}", dir.display()))
        .map(|e| e.unwrap().path())
        .filter(|p| p.extension().is_some_and(|x| x == "toml"))
        .collect();
    files.sort();
    files
}

fn read(rules: &Rules, fixture: &Fixture) -> Option<ScreenStatus> {
    let rows: Vec<String> = fixture
        .screen
        .lines()
        .map(|l| l.trim_end().to_owned())
        .collect();
    let cursor = fixture
        .cursor
        .as_ref()
        .map_or(Cursor::default(), |c| Cursor {
            row: c.row,
            col: c.col,
            visible: true,
        });
    rules.status(&Screen {
        rows: &rows,
        top: 0,
        height: rows.len().try_into().unwrap(),
        cursor,
        title: &fixture.title,
    })
}

#[test]
fn every_fixture_reads_as_the_state_it_shows() {
    let mut wrong = Vec::new();
    for agent in support::all().iter().filter(|a| a.screen_rules().is_some()) {
        let rules = agent.screen_rules().unwrap();
        let mut seen = BTreeSet::new();
        for path in fixtures(agent) {
            let name = path.file_stem().unwrap().to_string_lossy().into_owned();
            let text = std::fs::read_to_string(&path).unwrap();
            let fixture: Fixture =
                toml::from_str(&text).unwrap_or_else(|e| panic!("{}: {e}", path.display()));
            assert!(
                name.starts_with(fixture.status.as_str()),
                "{}: named for another state than {}",
                path.display(),
                fixture.status.as_str()
            );
            seen.insert(fixture.status.as_str());
            let got = read(rules, &fixture);
            if got != Some(fixture.status) {
                wrong.push(format!(
                    "{}/{name}: read {:?}",
                    agent.id(),
                    got.map(ScreenStatus::as_str)
                ));
            }
        }
        for state in STATES {
            assert!(
                seen.contains(state.as_str()),
                "{} has no {} fixture",
                agent.id(),
                state.as_str()
            );
        }
    }
    assert!(wrong.is_empty(), "misread fixtures:\n{}", wrong.join("\n"));
}

/// Replays a recorded session through the emulator, reading the rules after
/// each read of output as the engine does, and returns each status they
/// changed to.
fn replay(rules: &Rules, name: &str) -> Vec<ScreenStatus> {
    let log = vorn_recovery::transcript::builtin(name).unwrap();
    let mut em = Emulator::new(log.size.cols.into(), log.size.rows.into()).unwrap();
    let mut fx = Vec::new();
    let mut rows = Vec::new();
    let mut seen = Vec::new();
    for e in &log.entries {
        match &e.rec {
            Record::Data { bytes, .. } => em.feed(bytes, &mut fx),
            Record::Resize { cols, rows, .. } => {
                em.resize((*cols).into(), (*rows).into(), &mut fx).unwrap();
                continue;
            }
            _ => continue,
        }
        fx.clear();
        if em.render_hold().is_some() {
            continue;
        }
        let (row, col, visible) = em.cursor();
        let height = em.rows();
        em.rows_text(0, height - 1, &mut rows);
        let status = rules.status(&Screen {
            rows: &rows,
            top: 0,
            height,
            cursor: Cursor { row, col, visible },
            title: em.title(),
        });
        if let Some(s) = status {
            if seen.last() != Some(&s) {
                seen.push(s);
            }
        }
    }
    seen
}

#[test]
fn a_recorded_session_reads_as_waiting_at_its_picker() {
    let seen = replay(support::agent("claude").screen_rules().unwrap(), "agent");
    assert_eq!(seen, [ScreenStatus::Waiting]);
}

#[test]
fn other_programs_screens_read_as_nothing() {
    for agent in support::all().iter().filter(|a| a.screen_rules().is_some()) {
        for program in ["vim", "htop"] {
            let seen = replay(agent.screen_rules().unwrap(), program);
            assert!(seen.is_empty(), "{} read {program} as {seen:?}", agent.id());
        }
    }
}

//! A session reads its agent's status with the screen rules the host names
//! for it, once the host knows them, and with the analyzer's patterns until
//! then.

use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, OnceLock};
use std::time::{Duration, Instant};

use vorn_analysis::rules::Rules;
use vorn_analysis::{STATUS_IDLE, STATUS_RUNNING, STATUS_WAITING};
use vorn_engine::{Agents, Config, Effect, Out, Session};
use vorn_recovery::{LogBuilder, Size};
use vorn_term_proto::{Cursor, Entry};

fn rules() -> &'static Rules {
    static RULES: OnceLock<Rules> = OnceLock::new();
    RULES.get_or_init(|| {
        Rules::parse(
            r#"
agent = "test"
[[rule]]
status = "working"
rows = [-2, -1]
any = ['esc to interrupt']
[[rule]]
status = "idle"
rows = [-1, -1]
any = ['^>']
"#,
        )
        .unwrap()
    })
}

fn statuses(out: &[Out]) -> Vec<u32> {
    out.iter()
        .filter_map(|o| match o {
            Out::Effect(_, Effect::Status(s)) => Some(*s),
            _ => None,
        })
        .collect()
}

#[test]
fn the_hosts_rules_take_over_once_it_knows_the_agent_and_it_is_asked_sparingly() {
    let asked = Arc::new(AtomicUsize::new(0));
    let count = Arc::clone(&asked);
    let cfg = Arc::new(Config {
        agents: Some(Agents(Arc::new(move |id: &str| {
            assert_eq!(id, "s");
            // Unknown the first time it is asked, then known.
            (count.fetch_add(1, Ordering::SeqCst) > 0).then_some(Some(rules()))
        }))),
        ..Config::default()
    });
    let mut b = LogBuilder::new(Size::new(30, 4));
    b.data("Continue? (y/n) ");
    b.data("\x1b[2J\x1b[4;1H> ");
    b.data("\x1b[3;1H* esc to interrupt\x1b[4;1H\x1b[2K");
    b.data("\x1b[3;1H\x1b[2K\x1b[4;1H> ");
    b.data("x");
    let entries: Vec<Entry> = b.build().entries;
    let mut s = Session::fresh("s", cfg, (30, 4), Cursor::start(0)).unwrap();
    let t0 = Instant::now();
    let mut out = Vec::new();

    // Not known yet: the analyzer's patterns read the prompt.
    s.apply_all(&entries[..1], t0, &mut out);
    assert_eq!(statuses(&out), [STATUS_WAITING]);
    // Asked again only after a while.
    out.clear();
    s.apply_all(&entries[1..2], t0 + Duration::from_millis(200), &mut out);
    assert_eq!(asked.load(Ordering::SeqCst), 1);
    out.clear();
    s.apply_all(&entries[2..4], t0 + Duration::from_secs(2), &mut out);
    assert_eq!(asked.load(Ordering::SeqCst), 2);
    assert_eq!(statuses(&out), [STATUS_RUNNING, STATUS_IDLE]);
    // Known now: never asked again.
    out.clear();
    s.apply_all(&entries[4..], t0 + Duration::from_secs(10), &mut out);
    assert!(statuses(&out).is_empty());
    assert_eq!(asked.load(Ordering::SeqCst), 2);
}

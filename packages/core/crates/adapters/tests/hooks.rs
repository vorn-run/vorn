//! The shipped adapters' hook installers write what 0.8 wrote: the recorded
//! settings merge and hooks file under `agents/<name>/fixtures/`.

mod support;

use std::path::Path;

use serde_json::Value;
use vorn_hooks::{file, settings};

fn fixture(name: &str, file: &str) -> String {
    std::fs::read_to_string(support::package(name).join("fixtures").join(file)).unwrap()
}

#[test]
fn merges_the_settings_hooks_as_recorded() {
    let claude = support::agent("claude");
    let spec = claude
        .spec()
        .hooks
        .as_ref()
        .unwrap()
        .settings
        .as_ref()
        .unwrap();
    let all: Vec<&str> = spec.events.iter().map(String::as_str).collect();
    let own = spec.own_status.as_ref().unwrap();
    let answered: Vec<&str> = own.events.iter().map(String::as_str).collect();
    let recorded: Value = serde_json::from_str(&fixture("claude", "hooks-settings.json")).unwrap();
    let full = settings::install(recorded["start"].clone(), 56432, "tok", &all, &all);
    assert_eq!(full, recorded["full"]);
    let trimmed = settings::install(full, 56432, "tok", &all, &answered);
    assert_eq!(trimmed, recorded["trimmed"]);
    assert_eq!(settings::uninstall(trimmed, &all), recorded["removed"]);
    assert!(settings::reports_status(
        "2.1.295 (Claude Code)",
        &own.since
    ));
    assert!(!settings::reports_status(
        "2.1.294 (Claude Code)",
        &own.since
    ));
    assert_eq!(spec.file, "~/.claude/settings.json");
}

#[test]
fn writes_the_hooks_file_as_recorded() {
    let copilot = support::agent("copilot");
    let spec = copilot
        .spec()
        .hooks
        .as_ref()
        .unwrap()
        .file
        .as_ref()
        .unwrap();
    let file_spec = file::Spec {
        session: &spec.session,
        events: &spec.events,
    };
    assert_eq!(
        file::hooks_json(file_spec, Path::new("/home/me/.vorn")),
        fixture("copilot", "hooks-file.json")
    );
    assert_eq!(file_spec.session_id("t1"), "copilot-t1");
    let places = vorn_agents::spec::Places {
        home: "/h".into(),
        data: "/d".into(),
    };
    assert_eq!(
        places.path_with(&spec.path, &|_| None),
        Path::new("/h")
            .join(".copilot")
            .join("hooks")
            .join("vorn.json")
    );
    assert_eq!(
        places.path_with(&spec.path, &|n| (n == "COPILOT_HOME")
            .then(|| "/c".to_owned())),
        Path::new("/c").join("hooks").join("vorn.json")
    );
}

#[test]
fn captures_codex_and_opencode_conversations_from_their_databases() {
    let dir = tempfile::tempdir().unwrap();
    let places = vorn_agents::spec::Places {
        home: dir.path().to_path_buf(),
        data: dir.path().join(".local/share"),
    };
    let codex = support::agent("codex");
    let capture = codex
        .spec()
        .resume
        .as_ref()
        .unwrap()
        .capture
        .as_ref()
        .unwrap();
    let db = places.path(&capture.db);
    assert_eq!(db, dir.path().join(".codex").join("state_5.sqlite"));
    std::fs::create_dir_all(db.parent().unwrap()).unwrap();
    rusqlite::Connection::open(&db)
        .unwrap()
        .execute_batch(
            "CREATE TABLE threads (id TEXT, cwd TEXT, archived INT, updated_at INT);
             INSERT INTO threads VALUES ('old', 'C:\\Work\\App\\', 0, 1);
             INSERT INTO threads VALUES ('new', 'c:/work/app', 0, 2);
             INSERT INTO threads VALUES ('gone', 'c:/work/app', 1, 3);",
        )
        .unwrap();
    assert_eq!(
        vorn_hooks::capture::capture(&capture.query, "C:\\work\\app", &db).as_deref(),
        Some("new")
    );
    let opencode = support::agent("opencode");
    let capture = opencode
        .spec()
        .resume
        .as_ref()
        .unwrap()
        .capture
        .as_ref()
        .unwrap();
    assert_eq!(
        places.path(&capture.db),
        dir.path()
            .join(".local/share")
            .join("opencode")
            .join("opencode.db")
    );
}

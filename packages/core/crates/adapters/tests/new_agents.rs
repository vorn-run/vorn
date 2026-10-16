//! An adapter added with no change to Vorn's core: it is only a package
//! in the public format, installed the way any person installs one, and
//! launches, resumes and lists past sessions from its data.

mod support;

use std::fs;

use rusqlite::Connection;
use vorn_agents::history::{recent_sessions_for, Homes};
use vorn_agents::launch::{self, LaunchRequest, Machine, Platform, Quoting};

const POSIX: Machine = Machine {
    platform: Platform::Posix,
    quoting: Quoting::Posix,
};

fn line(req: &LaunchRequest) -> String {
    launch::launch_line(req, None, &[], &POSIX).unwrap()
}

#[test]
fn every_shipped_adapter_is_a_package_in_the_public_format() {
    for s in vorn_adapters::shipped() {
        let manifest = vorn_extensions::manifest::file::parse(s.manifest)
            .unwrap_or_else(|e| panic!("{}: {e}", s.name));
        assert!(
            manifest.agent.is_some(),
            "{} has an [agent] section",
            s.name
        );
        assert_eq!(manifest.service, None, "{} needs no program", s.name);
        assert_eq!(support::agent(s.name).id(), s.name);
    }
}

#[test]
fn an_adapter_installs_from_its_folder_as_any_extension_does() {
    let root = tempfile::tempdir().unwrap();
    let store = vorn_extensions::pack::PackStore::new(root.path().join("packages"));
    let installs = vorn_extensions::install::Installs::new(store.clone(), "git", Vec::new());
    for name in ["hermes"] {
        let pack = installs.link(&support::package(name)).unwrap();
        let section = pack.manifest.agent.clone().expect("an [agent] section");
        let spec = vorn_agents::AgentSpec::from_toml(section).unwrap();
        assert_eq!(spec.id, name);
        assert!(pack.path.join(spec.icon.unwrap()).is_file());
    }
}

#[test]
fn hermes_launches_into_a_chat_and_resumes_by_id() {
    let hermes = support::agent("hermes");
    let mut req = LaunchRequest::new(hermes.clone());
    req.initial_prompt = Some("plan it".into());
    assert_eq!(line(&req), "hermes chat -q 'plan it'");
    let mut resumed = LaunchRequest::new(hermes.clone());
    resumed.resume_session_id = Some("20261015_101500_a1b2c3".into());
    assert_eq!(line(&resumed), "hermes --resume 20261015_101500_a1b2c3");
    assert!(!hermes.pins_session_ids());
    assert!(hermes.spec().headless.is_none());
    assert_eq!(
        launch::headless_spawn(&LaunchRequest::new(hermes), None, &[], &POSIX)
            .unwrap_err()
            .to_string(),
        "hermes cannot run headless."
    );
}

#[test]
fn hermes_lists_past_sessions_from_hermes_home() {
    let dir = tempfile::tempdir().unwrap();
    let home = dir.path().join("h");
    let db = home.join("state.db");
    fs::create_dir_all(&home).unwrap();
    Connection::open(&db)
        .unwrap()
        .execute_batch(
            "CREATE TABLE sessions (id TEXT, cwd TEXT, title TEXT, started_at REAL, message_count INTEGER, archived INTEGER, hidden INTEGER);
             INSERT INTO sessions VALUES ('s1', '/proj', 'Fix it', 1735787046, 3, 0, 0);",
        )
        .unwrap();
    let places = Homes {
        home: dir.path().to_path_buf(),
        data: dir.path().join("data"),
    };
    let spec = support::agent("hermes");
    let sqlite = spec
        .spec()
        .history
        .as_ref()
        .unwrap()
        .sqlite
        .as_ref()
        .unwrap();
    let set = |n: &str| (n == "HERMES_HOME").then(|| home.to_string_lossy().into_owned());
    assert_eq!(places.path_with(&sqlite.db, &set), db);
    assert_eq!(
        places.path_with(&sqlite.db, &|_| None),
        dir.path().join(".hermes").join("state.db")
    );
    // Without HERMES_HOME set, the store is under the home directory.
    let default = dir.path().join(".hermes");
    fs::create_dir_all(&default).unwrap();
    fs::copy(&db, default.join("state.db")).unwrap();
    let list = recent_sessions_for(&spec, &places, None, 20);
    assert_eq!(list.len(), 1);
    assert_eq!(list[0].to_json()["display"], "Fix it");
    assert_eq!(list[0].to_json()["timestamp"], 1_735_787_046_000_i64);
}

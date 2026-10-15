//! The shipped adapters' past sessions, read as 0.8 read each agent's files.

mod support;

use std::fs;
use std::path::Path;

use rusqlite::Connection;
use serde_json::json;
use vorn_agents::history::{
    recent_sessions, recent_sessions_for, Homes, ProjectScope, RecentSession,
};

fn homes(dir: &Path) -> Homes {
    Homes {
        home: dir.to_path_buf(),
        data: dir.join(".local").join("share"),
    }
}

fn write(path: &Path, text: &str) {
    fs::create_dir_all(path.parent().unwrap()).unwrap();
    fs::write(path, text).unwrap();
}

fn sqlite(path: &Path, sql: &str) {
    fs::create_dir_all(path.parent().unwrap()).unwrap();
    Connection::open(path).unwrap().execute_batch(sql).unwrap();
}

fn ids(list: &[RecentSession]) -> Vec<&str> {
    list.iter().map(|s| s.session_id.as_str()).collect()
}

#[test]
fn folds_claude_prompts_into_sessions_by_their_latest_time() {
    let dir = tempfile::tempdir().unwrap();
    write(
        &dir.path().join(".claude/history.jsonl"),
        concat!(
            "\u{FEFF}{\"sessionId\":\"s1\",\"display\":\"Fix bug\",\"project\":\"/app\",\"timestamp\":1000}\n",
            "not json\n",
            "{\"sessionId\":\"s2\",\"display\":\"Add\",\"project\":\"/other\",\"timestamp\":1500}\r\n",
            "{\"sessionId\":\"s1\",\"display\":\"later\",\"project\":\"/app\",\"timestamp\":2000}\n",
            "{\"sessionId\":\"\",\"project\":\"/app\",\"timestamp\":9000}\n",
        ),
    );
    let claude = support::agent("claude");
    let all = recent_sessions_for(&claude, &homes(dir.path()), None, 20);
    assert_eq!(ids(&all), ["s1", "s2"]);
    assert_eq!(all[0].timestamp, 2000.0);
    assert_eq!(all[0].activity_count, 2.0);
    assert_eq!(all[0].display, "Fix bug");
    let scoped = recent_sessions_for(
        &claude,
        &homes(dir.path()),
        Some(&ProjectScope::new("/app/", ["/no/such/worktree"])),
        20,
    );
    assert_eq!(ids(&scoped), ["s1"]);
}

#[test]
fn reads_gemini_chats_by_project_newest_file_first() {
    let dir = tempfile::tempdir().unwrap();
    let chats = dir.path().join(".gemini/tmp/abc/chats");
    write(&dir.path().join(".gemini/tmp/abc/.project_root"), "/proj\n");
    write(
        &chats.join("session-1.json"),
        r#"{"sessionId":"g1","lastUpdated":"2025-01-02T03:04:05.678Z","messages":[
            {"type":"user","content":[{"text":"hello"},{"parts":[{"text":"world"}]}]},
            {"type":"gemini","content":"hi"},{"type":"user","content":"again"}]}"#,
    );
    write(&chats.join("session-2.json"), "{broken");
    write(&chats.join("other.json"), r#"{"sessionId":"x"}"#);
    let gemini = support::agent("gemini");
    let list = recent_sessions_for(&gemini, &homes(dir.path()), None, 20);
    assert_eq!(ids(&list), ["g1"]);
    assert_eq!(list[0].display, "hello world");
    assert_eq!(list[0].activity_count, 2.0);
    assert_eq!(list[0].timestamp, 1_735_787_045_678.0);
    assert_eq!(list[0].project_path, "/proj");
    assert!(!list[0].agent.resumes_exactly());
    let elsewhere = ProjectScope::new("/elsewhere", []);
    assert!(recent_sessions_for(&gemini, &homes(dir.path()), Some(&elsewhere), 20).is_empty());
}

#[test]
fn reads_codex_threads_with_their_prompt_counts() {
    let dir = tempfile::tempdir().unwrap();
    sqlite(
        &dir.path().join(".codex/state_5.sqlite"),
        "CREATE TABLE threads (id TEXT, cwd TEXT, title TEXT, updated_at INTEGER, first_user_message TEXT, archived INTEGER);
         INSERT INTO threads VALUES ('c1', '/proj/', '', 100, 'first message', 0);
         INSERT INTO threads VALUES ('c2', '/PROJ', 'Title', 200, NULL, 0);
         INSERT INTO threads VALUES ('c3', '/proj', 'gone', 300, NULL, 1);",
    );
    write(
        &dir.path().join(".codex/history.jsonl"),
        "{\"session_id\":\"c1\"}\n{\"session_id\":\"c1\"}\n{}\n",
    );
    let codex = support::agent("codex");
    let list = recent_sessions_for(&codex, &homes(dir.path()), None, 20);
    assert_eq!(ids(&list), ["c2", "c1"]);
    assert_eq!(list[1].display, "first message");
    assert_eq!(list[1].activity_count, 2.0);
    assert_eq!(list[0].activity_count, 1.0);
    assert_eq!(list[0].timestamp, 200_000.0);
    let scoped = recent_sessions_for(
        &codex,
        &homes(dir.path()),
        Some(&ProjectScope::new("/proj", [])),
        20,
    );
    if cfg!(windows) {
        assert_eq!(ids(&scoped), ["c2", "c1"]);
    } else {
        assert_eq!(ids(&scoped), ["c1"]);
    }
}

#[test]
fn reads_copilot_and_opencode_sessions_with_their_counts() {
    let dir = tempfile::tempdir().unwrap();
    sqlite(
        &dir.path().join(".copilot/session-store.db"),
        "CREATE TABLE sessions (id TEXT, cwd TEXT, summary TEXT, updated_at TEXT);
         CREATE TABLE turns (id INTEGER, session_id TEXT);
         INSERT INTO sessions VALUES ('p1', '/proj', 'Summary', '2025-01-02T03:04:05Z');
         INSERT INTO turns VALUES (1, 'p1'), (2, 'p1');",
    );
    sqlite(
        &dir.path().join(".local/share/opencode/opencode.db"),
        "CREATE TABLE session (id TEXT, directory TEXT, title TEXT, time_updated INTEGER, time_archived INTEGER);
         CREATE TABLE message (id TEXT, session_id TEXT);
         INSERT INTO session VALUES ('o1', '/proj', 'Open', 1735787046000, NULL);
         INSERT INTO message VALUES ('m1', 'o1');",
    );
    let list = recent_sessions(&support::on_by_default(), &homes(dir.path()), None, 20);
    assert_eq!(ids(&list), ["o1", "p1"]);
    assert_eq!(list[1].activity_count, 2.0);
    assert_eq!(list[1].timestamp, 1_735_787_045_000.0);
    assert_eq!(list[0].activity_count, 1.0);
    assert_eq!(
        list[0].to_json(),
        json!({
            "sessionId": "o1", "agentType": "opencode", "display": "Open",
            "projectPath": "/proj", "timestamp": 1_735_787_046_000_i64,
            "activityCount": 1, "activityLabel": "message", "canResumeExact": true
        })
    );
    assert_eq!(list[1].to_json()["activityLabel"], "turn");
}

#[test]
fn keeps_the_newest_up_to_the_limit_across_agents() {
    let dir = tempfile::tempdir().unwrap();
    let lines: String = (0..30)
        .map(|i| format!("{{\"sessionId\":\"s{i}\",\"project\":\"/p\",\"timestamp\":{i}}}\n"))
        .collect();
    write(&dir.path().join(".claude/history.jsonl"), &lines);
    let list = recent_sessions(&support::on_by_default(), &homes(dir.path()), None, 5);
    assert_eq!(ids(&list), ["s29", "s28", "s27", "s26", "s25"]);
}

#[test]
fn reads_hermes_sessions_from_its_store() {
    let dir = tempfile::tempdir().unwrap();
    sqlite(
        &dir.path().join(".hermes/state.db"),
        "CREATE TABLE sessions (id TEXT, cwd TEXT, title TEXT, started_at REAL, message_count INTEGER, archived INTEGER, hidden INTEGER);
         INSERT INTO sessions VALUES ('h1', '/proj', 'Plan', 1735787046.5, 4, 0, 0);
         INSERT INTO sessions VALUES ('h2', '/proj', 'Gone', 1735787047, 1, 1, 0);",
    );
    let hermes = support::agent("hermes");
    let list = recent_sessions_for(&hermes, &homes(dir.path()), None, 20);
    assert_eq!(ids(&list), ["h1"]);
    assert_eq!(list[0].timestamp, 1_735_787_046_500.0);
    assert_eq!(list[0].activity_count, 4.0);
    assert_eq!(list[0].display, "Plan");
}

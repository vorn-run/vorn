//! The conversations each agent has had, read from the agent's own files
//! (`agent-history`), for offering one to resume.
//!
//! Every agent keeps its sessions differently, and its adapter's
//! `[agent.history]` says how, as one of three shapes:
//!
//! - **`jsonl`**: one JSON object per line, one line per prompt, folded into
//!   conversations by their id.
//! - **`sqlite`**: a query over the agent's own SQLite file, with the
//!   project's directories matched in SQL.
//! - **`json_files`**: one JSON file per conversation, in a directory per
//!   project, the projects named by marker files or a map.
//!
//! A file that is missing or unreadable lists nothing for its agent, as a
//! malformed line or session file lists nothing for itself. One departure from
//! the server, on input no agent writes: a line or file whose fields are not
//! the types the agent writes (a numeric session id, a missing timestamp) is
//! skipped here, where the server may list it half-filled.

use std::collections::{HashMap, HashSet};
use std::fs;
use std::path::{Path, PathBuf};

use rusqlite::types::ValueRef;
use rusqlite::{Connection, OpenFlags};
use serde_json::{json, Map, Value};

use crate::spec::{History, JsonFilesHistory, JsonlHistory, Places, SqliteHistory, TimeUnit};
use crate::{js, paths, Agent, Agents};

/// How many sessions an agent lists, and how many the merged list keeps.
pub const DEFAULT_LIMIT: usize = 20;

/// Where the agents keep their files.
pub type Homes = Places;

/// A project and its worktrees: the directories an agent may have recorded
/// one of the project's sessions under.
#[derive(Clone, Debug, Default)]
pub struct ProjectScope {
    /// As given, first spelling of each kept, for the SQL filters.
    raw: Vec<String>,
    normalized: HashSet<String>,
}

impl ProjectScope {
    /// The project at `project` and the worktrees at `worktrees`.
    pub fn new<'a>(project: &'a str, worktrees: impl IntoIterator<Item = &'a str>) -> ProjectScope {
        let mut scope = ProjectScope::default();
        for p in std::iter::once(project).chain(worktrees) {
            if scope.normalized.insert(paths::normalize(p)) {
                scope.raw.push(p.to_owned());
            }
        }
        scope
    }

    fn contains(&self, normalized: &str) -> bool {
        self.normalized.contains(normalized)
    }

    /// A `WHERE` term matching `column` against any of the paths, as
    /// `buildPathWhereClause` writes it.
    fn where_clause(&self, column: &str) -> String {
        let terms: Vec<String> = self
            .raw
            .iter()
            .map(|p| {
                let wanted = paths::comparable(p).replace('\'', "''");
                format!("lower(rtrim(replace({column}, char(92), '/'), '/')) = '{wanted}'")
            })
            .collect();
        match terms.as_slice() {
            [one] => one.clone(),
            _ => format!("({})", terms.join(" OR ")),
        }
    }
}

/// One past session, as `sessions:getRecent` lists it (`RecentSession`).
#[derive(Clone, Debug, PartialEq)]
pub struct RecentSession {
    pub session_id: String,
    pub agent: Agent,
    pub display: String,
    pub project_path: String,
    /// Milliseconds since the epoch; NaN when the agent's record of it does
    /// not read as a time.
    pub timestamp: f64,
    pub activity_count: f64,
}

impl RecentSession {
    /// The object the server sends, in its key order.
    pub fn to_json(&self) -> Value {
        json!({
            "sessionId": self.session_id,
            "agentType": self.agent.id(),
            "display": self.display,
            "projectPath": self.project_path,
            "timestamp": js::number(self.timestamp),
            "activityCount": js::number(self.activity_count),
            "activityLabel": self.agent.activity_label(),
            "canResumeExact": self.agent.resumes_exactly(),
        })
    }
}

/// Every agent's sessions, newest first, at most `limit` (`getRecentSessions`).
/// With a scope, only the sessions recorded in one of its directories.
/// Sessions at the same moment keep `agents`' order.
pub fn recent_sessions(
    agents: &Agents,
    homes: &Homes,
    scope: Option<&ProjectScope>,
    limit: usize,
) -> Vec<RecentSession> {
    let mut all: Vec<RecentSession> = agents
        .iter()
        .flat_map(|agent| provider(agent, homes, scope, limit))
        .collect();
    all.sort_by(|a, b| js::newest_first(a.timestamp, b.timestamp));
    all.truncate(limit);
    all
}

/// One agent's sessions, newest first, so a busy agent cannot crowd out a
/// quiet one (`getRecentSessionsFor`).
pub fn recent_sessions_for(
    agent: &Agent,
    homes: &Homes,
    scope: Option<&ProjectScope>,
    limit: usize,
) -> Vec<RecentSession> {
    let mut list = provider(agent, homes, scope, limit);
    list.sort_by(|a, b| js::newest_first(a.timestamp, b.timestamp));
    list
}

fn provider(
    agent: &Agent,
    homes: &Homes,
    scope: Option<&ProjectScope>,
    limit: usize,
) -> Vec<RecentSession> {
    let Some(history) = &agent.spec().history else {
        return Vec::new();
    };
    let found = if let Some(j) = &history.jsonl {
        jsonl(agent, j, homes, scope, limit)
    } else if let Some(q) = &history.sqlite {
        sqlite(agent, history, q, homes, scope, limit)
    } else if let Some(f) = &history.json_files {
        json_files(agent, history, f, homes, scope, limit)
    } else {
        None
    };
    found.unwrap_or_default()
}

/// A title cut to the adapter's limit.
fn titled(history: &History, text: &str) -> String {
    match history.title_limit {
        Some(n) => js::slice_utf16(text, n),
        None => text.to_owned(),
    }
}

/// `normalizePath` once per distinct path: history repeats them.
#[derive(Default)]
struct Normalized(HashMap<String, String>);

impl Normalized {
    fn of(&mut self, p: &str) -> &str {
        self.0
            .entry(p.to_owned())
            .or_insert_with(|| paths::normalize(p))
    }
}

/// A file as `readFileSync(p, 'utf-8')` reads it.
fn read_text(path: &Path) -> Option<String> {
    let bytes = fs::read(path).ok()?;
    Some(String::from_utf8_lossy(&bytes).into_owned())
}

/// Lines as `raw.trim().split('\n').filter(Boolean)` gives them.
fn lines(raw: &str) -> impl Iterator<Item = &str> {
    js::trim(raw).split('\n').filter(|l| !l.is_empty())
}

fn jsonl(
    agent: &Agent,
    spec: &JsonlHistory,
    homes: &Homes,
    scope: Option<&ProjectScope>,
    limit: usize,
) -> Option<Vec<RecentSession>> {
    struct Seen {
        id: String,
        display: String,
        project: String,
        last: f64,
        count: f64,
    }
    let raw = read_text(&homes.path(&spec.file))?;
    let mut normalized = Normalized::default();
    let mut sessions: Vec<Seen> = Vec::new();
    let mut index: HashMap<String, usize> = HashMap::new();
    for line in lines(&raw) {
        let Ok(Value::Object(entry)) = serde_json::from_str::<Value>(line) else {
            continue;
        };
        let (Some(id), Some(project)) = (text(&entry, &spec.session), text(&entry, &spec.project))
        else {
            continue;
        };
        if id.is_empty() || project.is_empty() {
            continue;
        }
        let Some(timestamp) = entry.get(&spec.time).and_then(Value::as_f64) else {
            continue;
        };
        if scope.is_some_and(|s| !s.contains(normalized.of(project))) {
            continue;
        }
        match index.get(id) {
            Some(&at) => {
                let seen = &mut sessions[at];
                seen.last = seen.last.max(timestamp);
                seen.count += 1.0;
            }
            None => {
                let display = match entry.get(&spec.title) {
                    Some(Value::String(s)) => s.clone(),
                    None | Some(Value::Null) | Some(Value::Bool(false)) => String::new(),
                    Some(_) => continue,
                };
                index.insert(id.to_owned(), sessions.len());
                sessions.push(Seen {
                    id: id.to_owned(),
                    display,
                    project: project.to_owned(),
                    last: timestamp,
                    count: 1.0,
                });
            }
        }
    }
    let mut list: Vec<RecentSession> = sessions
        .into_iter()
        .map(|s| RecentSession {
            session_id: s.id,
            agent: agent.clone(),
            display: s.display,
            project_path: s.project,
            timestamp: s.last,
            activity_count: s.count,
        })
        .collect();
    list.sort_by(|a, b| js::newest_first(a.timestamp, b.timestamp));
    list.truncate(limit);
    Some(list)
}

fn text<'a>(entry: &'a Map<String, Value>, key: &str) -> Option<&'a str> {
    entry.get(key).and_then(Value::as_str)
}

/// Names in a directory, sorted by their bytes as Node's `readdirSync` sorts
/// them; an error when the directory cannot be read.
fn read_dir_sorted(dir: &Path) -> std::io::Result<Vec<String>> {
    let mut names: Vec<String> = fs::read_dir(dir)?
        .filter_map(Result::ok)
        .map(|e| e.file_name().to_string_lossy().into_owned())
        .collect();
    if !cfg!(windows) {
        names.sort();
    }
    Ok(names)
}

/// `stat.mtimeMs`: milliseconds with the fraction kept, so two files are
/// told apart as finely as the server tells them apart.
fn mtime_ms(t: std::time::SystemTime) -> f64 {
    match t.duration_since(std::time::UNIX_EPOCH) {
        Ok(d) => d.as_secs_f64() * 1000.0,
        Err(e) => -(e.duration().as_secs_f64() * 1000.0),
    }
}

/// A project's path and the directory its conversations are in.
struct Source {
    project: String,
    chats: PathBuf,
}

fn json_sources(spec: &JsonFilesHistory, homes: &Homes) -> std::io::Result<Vec<Source>> {
    let mut sources = Vec::new();
    let mut seen = HashSet::new();
    let mut add = |project: String, chats: PathBuf| {
        if project.is_empty() || !chats.exists() {
            return;
        }
        if seen.insert(format!(
            "{}::{}",
            paths::normalize(&project),
            chats.display()
        )) {
            sources.push(Source { project, chats });
        }
    };
    if let Some(markers) = &spec.markers {
        let root = homes.path(&markers.dir);
        if root.exists() {
            for name in read_dir_sorted(&root)? {
                let dir = root.join(&name);
                let marker = dir.join(&markers.file);
                if !marker.exists() {
                    continue;
                }
                if let Some(project) = read_text(&marker) {
                    add(js::trim(&project).to_owned(), dir.join(&markers.sessions));
                }
            }
        }
    }
    if let Some(map) = &spec.map {
        let file = homes.path(&map.file);
        if file.exists() {
            let listed = read_text(&file)
                .and_then(|raw| serde_json::from_str::<Value>(&raw).ok())
                .and_then(|v| v.get(&map.key).and_then(Value::as_object).cloned());
            for (project, name) in listed.into_iter().flatten() {
                if let Value::String(name) = name {
                    add(project, homes.path(&map.sessions.replace("{name}", &name)));
                }
            }
        }
    }
    Ok(sources)
}

/// The text of a message's content, however it is nested: a string, a list
/// of parts, or an object with `text`, `content` or `parts`.
fn nested_text(value: &Value) -> String {
    match value {
        Value::String(s) => s.clone(),
        Value::Array(parts) => {
            let joined: Vec<String> = parts
                .iter()
                .map(nested_text)
                .filter(|s| !s.is_empty())
                .collect();
            js::trim(&joined.join(" ")).to_owned()
        }
        Value::Object(record) => {
            if let Some(Value::String(text)) = record.get("text") {
                text.clone()
            } else if let Some(content) = record.get("content") {
                nested_text(content)
            } else if let Some(parts) = record.get("parts") {
                nested_text(parts)
            } else {
                String::new()
            }
        }
        _ => String::new(),
    }
}

fn json_files(
    agent: &Agent,
    history: &History,
    spec: &JsonFilesHistory,
    homes: &Homes,
    scope: Option<&ProjectScope>,
    limit: usize,
) -> Option<Vec<RecentSession>> {
    let (prefix, suffix) = spec.files.split_once('*')?;
    let sources: Vec<Source> = json_sources(spec, homes)
        .ok()?
        .into_iter()
        .filter(|s| scope.is_none_or(|scope| scope.contains(&paths::normalize(&s.project))))
        .collect();
    let mut sessions = Vec::new();
    for source in &sources {
        let mut files = Vec::new();
        for name in read_dir_sorted(&source.chats).ok()? {
            if !(name.starts_with(prefix) && name.ends_with(suffix)) {
                continue;
            }
            let modified = fs::metadata(source.chats.join(&name))
                .ok()?
                .modified()
                .ok()?;
            files.push((name, mtime_ms(modified)));
        }
        files.sort_by(|a, b| js::newest_first(a.1, b.1));
        files.truncate(limit);
        for (name, _) in files {
            let file = source.chats.join(name);
            if let Some(session) = json_session(agent, history, spec, &file, &source.project) {
                sessions.push(session);
            }
        }
    }
    sessions.sort_by(|a, b| js::newest_first(a.timestamp, b.timestamp));
    sessions.truncate(limit);
    Some(sessions)
}

fn json_session(
    agent: &Agent,
    history: &History,
    spec: &JsonFilesHistory,
    file: &Path,
    project: &str,
) -> Option<RecentSession> {
    let Value::Object(session) = serde_json::from_str(&read_text(file)?).ok()? else {
        return None;
    };
    let id = text(&session, &spec.id)?;
    let updated = text(&session, &spec.time)?;
    let messages = session.get(&spec.messages)?.as_array()?;
    let [field, value] = &spec.user;
    let users: Vec<&Value> = messages
        .iter()
        .filter(|m| m.get(field).and_then(Value::as_str) == Some(value.as_str()))
        .collect();
    let first = users
        .first()
        .and_then(|m| m.get(&spec.text))
        .map_or_else(String::new, nested_text);
    Some(RecentSession {
        session_id: id.to_owned(),
        agent: agent.clone(),
        display: titled(history, &first),
        project_path: project.to_owned(),
        timestamp: js::date_ms(updated),
        activity_count: users.len() as f64,
    })
}

/// A row as libsql hands it to JavaScript, one value per column.
type Row = Map<String, Value>;

/// The rows of a read-only query, or none when the file cannot be opened
/// or the query fails (`querySqlite`).
fn query(db: &Path, sql: &str) -> Vec<Row> {
    let run = || -> rusqlite::Result<Vec<Row>> {
        let conn = Connection::open_with_flags(
            db,
            OpenFlags::SQLITE_OPEN_READ_ONLY | OpenFlags::SQLITE_OPEN_NO_MUTEX,
        )?;
        let mut stmt = conn.prepare(sql)?;
        let names: Vec<String> = stmt
            .column_names()
            .iter()
            .map(|s| (*s).to_owned())
            .collect();
        let mut rows = stmt.query([])?;
        let mut out = Vec::new();
        while let Some(row) = rows.next()? {
            let mut map = Map::new();
            for (i, name) in names.iter().enumerate() {
                map.insert(name.clone(), column(row.get_ref(i)?));
            }
            out.push(map);
        }
        Ok(out)
    };
    run().unwrap_or_default()
}

fn column(value: ValueRef<'_>) -> Value {
    match value {
        ValueRef::Null => Value::Null,
        ValueRef::Integer(i) => Value::from(i),
        ValueRef::Real(f) => js::number(f),
        ValueRef::Text(t) => Value::String(String::from_utf8_lossy(t).into_owned()),
        ValueRef::Blob(b) => Value::String(String::from_utf8_lossy(b).into_owned()),
    }
}

/// `String(v)`.
fn js_string(v: Option<&Value>) -> String {
    match v {
        None => "undefined".to_owned(),
        Some(Value::Null) => "null".to_owned(),
        Some(Value::String(s)) => s.clone(),
        Some(Value::Number(n)) => match n.as_i64() {
            Some(i) => i.to_string(),
            None => n.to_string(),
        },
        Some(other) => other.to_string(),
    }
}

/// `Number(v)`.
fn js_number(v: Option<&Value>) -> f64 {
    match v {
        None => f64::NAN,
        Some(Value::Null) => 0.0,
        Some(Value::Number(n)) => n.as_f64().unwrap_or(f64::NAN),
        Some(Value::String(s)) => js::to_number(s),
        Some(Value::Bool(b)) => f64::from(u8::from(*b)),
        Some(_) => f64::NAN,
    }
}

/// `v || ''` read as a string: the first of `keys` whose value is truthy.
fn first_truthy(row: &Row, keys: &[&str]) -> String {
    keys.iter()
        .filter_map(|k| row.get(*k))
        .find(|v| truthy(v))
        .map_or_else(String::new, |v| js_string(Some(v)))
}

fn truthy(v: &Value) -> bool {
    match v {
        Value::Null => false,
        Value::Bool(b) => *b,
        Value::Number(n) => n.as_f64().is_some_and(|f| f != 0.0 && !f.is_nan()),
        Value::String(s) => !s.is_empty(),
        Value::Array(_) | Value::Object(_) => true,
    }
}

/// `Number(v) || 0`.
fn count(v: Option<&Value>) -> f64 {
    let n = js_number(v);
    if n.is_nan() {
        0.0
    } else {
        n
    }
}

/// A scoped query, then the whole table when the scoped one finds nothing,
/// each kept to the sessions in scope: the SQL filter compares spellings,
/// and an agent may have recorded a path through a symlink.
fn scoped_rows(
    db: &Path,
    scope: Option<&ProjectScope>,
    scoped_sql: impl Fn(&ProjectScope) -> String,
    all_sql: &str,
    map: impl Fn(&Row) -> RecentSession,
    limit: usize,
) -> Vec<RecentSession> {
    let keep = |rows: Vec<Row>| -> Vec<RecentSession> {
        let mut normalized = Normalized::default();
        rows.iter()
            .map(&map)
            .filter(|s| scope.is_none_or(|scope| scope.contains(normalized.of(&s.project_path))))
            .collect()
    };
    let mut sessions = match scope {
        Some(scope) => keep(query(db, &scoped_sql(scope))),
        None => keep(query(db, &format!("{all_sql} LIMIT {limit}"))),
    };
    if scope.is_some() && sessions.is_empty() {
        sessions = keep(query(db, all_sql));
    }
    sessions.truncate(limit);
    sessions
}

fn sqlite(
    agent: &Agent,
    history: &History,
    spec: &SqliteHistory,
    homes: &Homes,
    scope: Option<&ProjectScope>,
    limit: usize,
) -> Option<Vec<RecentSession>> {
    let db = homes.path(&spec.db);
    if !db.exists() {
        return None;
    }
    // Counted by the id as written: a numeric one never matches a row's.
    let mut lines_named: HashMap<String, f64> = HashMap::new();
    if let Some(counted) = &spec.count_lines {
        let file = homes.path(&counted.file);
        if file.exists() {
            for line in lines(&read_text(&file)?) {
                let entry = serde_json::from_str::<Value>(line).ok();
                match entry.as_ref().and_then(|v| v.get(&counted.key)) {
                    Some(Value::String(id)) if !id.is_empty() => {
                        *lines_named.entry(id.clone()).or_insert(0.0) += 1.0;
                    }
                    _ => {}
                }
            }
        }
    }
    let title: Vec<&str> = spec.title.iter().map(String::as_str).collect();
    Some(scoped_rows(
        &db,
        scope,
        |s| {
            format!(
                "{} LIMIT {limit}",
                spec.query.replace("{scope}", &s.where_clause(&spec.scope))
            )
        },
        &spec.query.replace("{scope}", "1"),
        |row| {
            let id = js_string(row.get(&spec.id));
            let activity_count = match (&spec.count_lines, &spec.count) {
                (Some(counted), _) => lines_named.get(&id).copied().unwrap_or(counted.missing),
                (None, Some(column)) => count(row.get(column)),
                (None, None) => 1.0,
            };
            let at = row.get(&spec.time);
            let timestamp = match spec.unit {
                TimeUnit::Seconds => js_number(at) * 1000.0,
                TimeUnit::Milliseconds => count(at),
                TimeUnit::Date => js::date_ms(&js_string(at)),
            };
            RecentSession {
                activity_count,
                session_id: id,
                agent: agent.clone(),
                display: titled(history, &first_truthy(row, &title)),
                project_path: first_truthy(row, &[spec.project.as_str()]),
                timestamp,
            }
        },
        limit,
    ))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::testing::agent;

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

    fn ids(list: &[RecentSession]) -> Vec<&str> {
        list.iter().map(|s| s.session_id.as_str()).collect()
    }

    fn adapter(id: &str, history: &str) -> Agent {
        agent(&format!(
            "[agent]\nid = \"{id}\"\nname = \"{id}\"\n[agent.launch]\ncommand = \"{id}\"\n\
             [agent.resume]\nargs = [\"--resume\", \"{{id}}\"]\n{history}"
        ))
    }

    /// One line per prompt.
    fn folded() -> Agent {
        adapter(
            "folded",
            "[agent.history]\nactivity = \"entry\"\n[agent.history.jsonl]\n\
             file = \"~/.folded/history.jsonl\"\nsession = \"sessionId\"\nproject = \"project\"\n\
             time = \"timestamp\"\ntitle = \"display\"\n",
        )
    }

    /// One file per chat, in a directory per project.
    fn chats() -> Agent {
        agent(
            "[agent]\nid = \"chats\"\nname = \"Chats\"\n[agent.launch]\ncommand = \"chats\"\n\
             [agent.history]\nactivity = \"prompt\"\ntitle_limit = 80\n[agent.history.json_files]\n\
             markers = { dir = \"~/.chats/tmp\", file = \".project_root\", sessions = \"chats\" }\n\
             map = { file = \"~/.chats/projects.json\", key = \"projects\", sessions = \"~/.chats/tmp/{name}/chats\" }\n\
             files = \"session-*.json\"\nid = \"sessionId\"\ntime = \"lastUpdated\"\nmessages = \"messages\"\n\
             user = [\"type\", \"user\"]\ntext = \"content\"\n",
        )
    }

    /// A table of threads, counted from a file of prompts.
    fn threads() -> Agent {
        adapter(
            "threads",
            "[agent.history]\nactivity = \"entry\"\ntitle_limit = 80\n[agent.history.sqlite]\n\
             db = \"~/.threads/state.sqlite\"\n\
             query = \"SELECT id, cwd, title, updated_at, first_user_message FROM threads WHERE archived = 0 AND {scope} ORDER BY updated_at DESC\"\n\
             scope = \"cwd\"\nid = \"id\"\nproject = \"cwd\"\ntitle = [\"title\", \"first_user_message\"]\n\
             time = \"updated_at\"\nunit = \"seconds\"\n\
             count_lines = { file = \"~/.threads/history.jsonl\", key = \"session_id\", missing = 1 }\n",
        )
    }

    /// Sessions and turns, the time as text.
    fn turns() -> Agent {
        adapter(
            "turns",
            "[agent.history]\nactivity = \"turn\"\ntitle_limit = 80\n[agent.history.sqlite]\n\
             db = \"~/.turns/store.db\"\n\
             query = \"SELECT s.id, s.cwd, s.summary, s.updated_at, COUNT(t.id) as turn_count FROM sessions s LEFT JOIN turns t ON s.id = t.session_id WHERE {scope} GROUP BY s.id ORDER BY s.updated_at DESC\"\n\
             scope = \"s.cwd\"\nid = \"id\"\nproject = \"cwd\"\ntitle = [\"summary\"]\n\
             time = \"updated_at\"\nunit = \"date\"\ncount = \"turn_count\"\n",
        )
    }

    /// Sessions and messages under the data directory, the time in milliseconds.
    fn messages() -> Agent {
        adapter(
            "messages",
            "[agent.history]\nactivity = \"message\"\ntitle_limit = 80\n[agent.history.sqlite]\n\
             db = \"{data}/messages/messages.db\"\n\
             query = \"SELECT s.id, s.directory, s.title, s.time_updated, COUNT(m.id) as message_count FROM session s LEFT JOIN message m ON s.id = m.session_id WHERE s.time_archived IS NULL AND {scope} GROUP BY s.id ORDER BY s.time_updated DESC\"\n\
             scope = \"s.directory\"\nid = \"id\"\nproject = \"directory\"\ntitle = [\"title\"]\n\
             time = \"time_updated\"\nunit = \"milliseconds\"\ncount = \"message_count\"\n",
        )
    }

    fn all() -> Agents {
        Agents::new([folded(), chats(), threads(), turns(), messages()])
    }

    #[test]
    fn lists_nothing_without_any_history() {
        let dir = tempfile::tempdir().unwrap();
        assert!(recent_sessions(&all(), &homes(dir.path()), None, DEFAULT_LIMIT).is_empty());
        let none = crate::testing::bare("none");
        assert!(recent_sessions_for(&none, &homes(dir.path()), None, 20).is_empty());
    }

    #[test]
    fn folds_prompts_into_sessions_by_their_latest_time() {
        let dir = tempfile::tempdir().unwrap();
        write(
            &dir.path().join(".folded/history.jsonl"),
            concat!(
                "\u{FEFF}{\"sessionId\":\"s1\",\"display\":\"Fix bug\",\"project\":\"/app\",\"timestamp\":1000}\n",
                "not json\n",
                "{\"sessionId\":\"s2\",\"display\":\"Add\",\"project\":\"/other\",\"timestamp\":1500}\r\n",
                "{\"sessionId\":\"s1\",\"display\":\"later\",\"project\":\"/app\",\"timestamp\":2000}\n",
                "{\"sessionId\":\"\",\"project\":\"/app\",\"timestamp\":9000}\n",
            ),
        );
        let all = recent_sessions_for(&folded(), &homes(dir.path()), None, 20);
        assert_eq!(ids(&all), ["s1", "s2"]);
        assert_eq!(all[0].timestamp, 2000.0);
        assert_eq!(all[0].activity_count, 2.0);
        assert_eq!(all[0].display, "Fix bug");
        let scoped = recent_sessions_for(
            &folded(),
            &homes(dir.path()),
            Some(&ProjectScope::new("/app/", ["/no/such/worktree"])),
            20,
        );
        assert_eq!(ids(&scoped), ["s1"]);
    }

    #[test]
    fn reads_chat_files_by_project_newest_file_first() {
        let dir = tempfile::tempdir().unwrap();
        let chats_dir = dir.path().join(".chats/tmp/abc/chats");
        write(&dir.path().join(".chats/tmp/abc/.project_root"), "/proj\n");
        write(
            &chats_dir.join("session-1.json"),
            r#"{"sessionId":"g1","lastUpdated":"2025-01-02T03:04:05.678Z","messages":[
                {"type":"user","content":[{"text":"hello"},{"parts":[{"text":"world"}]}]},
                {"type":"model","content":"hi"},{"type":"user","content":"again"}]}"#,
        );
        write(&chats_dir.join("session-2.json"), "{broken");
        write(&chats_dir.join("other.json"), r#"{"sessionId":"x"}"#);
        let list = recent_sessions_for(&chats(), &homes(dir.path()), None, 20);
        assert_eq!(ids(&list), ["g1"]);
        assert_eq!(list[0].display, "hello world");
        assert_eq!(list[0].activity_count, 2.0);
        assert_eq!(list[0].timestamp, 1_735_787_045_678.0);
        assert_eq!(list[0].project_path, "/proj");
        assert!(!list[0].agent.resumes_exactly());
        let elsewhere = ProjectScope::new("/elsewhere", []);
        assert!(recent_sessions_for(&chats(), &homes(dir.path()), Some(&elsewhere), 20).is_empty());
    }

    #[test]
    fn finds_chat_directories_through_the_project_map() {
        let dir = tempfile::tempdir().unwrap();
        write(
            &dir.path().join(".chats/projects.json"),
            r#"{"projects":{"/mapped":"named"}}"#,
        );
        write(
            &dir.path().join(".chats/tmp/named/chats/session-a.json"),
            r#"{"sessionId":"m1","lastUpdated":"2025-01-02","messages":[]}"#,
        );
        let list = recent_sessions_for(&chats(), &homes(dir.path()), None, 20);
        assert_eq!(ids(&list), ["m1"]);
        assert_eq!(list[0].project_path, "/mapped");
        assert_eq!(list[0].display, "");
    }

    fn sqlite(path: &Path, sql: &str) {
        fs::create_dir_all(path.parent().unwrap()).unwrap();
        Connection::open(path).unwrap().execute_batch(sql).unwrap();
    }

    #[test]
    fn reads_threads_with_their_prompt_counts() {
        let dir = tempfile::tempdir().unwrap();
        sqlite(
            &dir.path().join(".threads/state.sqlite"),
            "CREATE TABLE threads (id TEXT, cwd TEXT, title TEXT, updated_at INTEGER, first_user_message TEXT, archived INTEGER);
             INSERT INTO threads VALUES ('c1', '/proj/', '', 100, 'first message', 0);
             INSERT INTO threads VALUES ('c2', '/PROJ', 'Title', 200, NULL, 0);
             INSERT INTO threads VALUES ('c3', '/proj', 'gone', 300, NULL, 1);",
        );
        write(
            &dir.path().join(".threads/history.jsonl"),
            "{\"session_id\":\"c1\"}\n{\"session_id\":\"c1\"}\n{}\n",
        );
        let list = recent_sessions_for(&threads(), &homes(dir.path()), None, 20);
        assert_eq!(ids(&list), ["c2", "c1"]);
        assert_eq!(list[1].display, "first message");
        assert_eq!(list[1].activity_count, 2.0);
        assert_eq!(list[0].activity_count, 1.0);
        assert_eq!(list[0].timestamp, 200_000.0);
        // The SQL filter matches either spelling; the kept rows are the ones
        // whose path normalizes to the project's, which ignores case on
        // Windows.
        let scoped = recent_sessions_for(
            &threads(),
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
    fn reads_turns_and_messages_with_their_counts() {
        let dir = tempfile::tempdir().unwrap();
        sqlite(
            &dir.path().join(".turns/store.db"),
            "CREATE TABLE sessions (id TEXT, cwd TEXT, summary TEXT, updated_at TEXT);
             CREATE TABLE turns (id INTEGER, session_id TEXT);
             INSERT INTO sessions VALUES ('p1', '/proj', 'Summary', '2025-01-02T03:04:05Z');
             INSERT INTO turns VALUES (1, 'p1'), (2, 'p1');",
        );
        sqlite(
            &dir.path().join(".local/share/messages/messages.db"),
            "CREATE TABLE session (id TEXT, directory TEXT, title TEXT, time_updated INTEGER, time_archived INTEGER);
             CREATE TABLE message (id TEXT, session_id TEXT);
             INSERT INTO session VALUES ('o1', '/proj', 'Open', 1735787046000, NULL);
             INSERT INTO message VALUES ('m1', 'o1');",
        );
        let list = recent_sessions(&all(), &homes(dir.path()), None, 20);
        assert_eq!(ids(&list), ["o1", "p1"]);
        assert_eq!(list[1].activity_count, 2.0);
        assert_eq!(list[1].timestamp, 1_735_787_045_000.0);
        assert_eq!(list[0].activity_count, 1.0);
        assert_eq!(
            list[0].to_json(),
            json!({
                "sessionId": "o1", "agentType": "messages", "display": "Open",
                "projectPath": "/proj", "timestamp": 1_735_787_046_000_i64,
                "activityCount": 1, "activityLabel": "message", "canResumeExact": true
            })
        );
        let scoped = recent_sessions_for(
            &turns(),
            &homes(dir.path()),
            Some(&ProjectScope::new("/proj", [])),
            20,
        );
        assert_eq!(ids(&scoped), ["p1"]);
    }

    #[test]
    fn keeps_the_newest_up_to_the_limit_across_agents() {
        let dir = tempfile::tempdir().unwrap();
        let lines: String = (0..30)
            .map(|i| format!("{{\"sessionId\":\"s{i}\",\"project\":\"/p\",\"timestamp\":{i}}}\n"))
            .collect();
        write(&dir.path().join(".folded/history.jsonl"), &lines);
        let list = recent_sessions(&all(), &homes(dir.path()), None, 5);
        assert_eq!(ids(&list), ["s29", "s28", "s27", "s26", "s25"]);
    }

    #[test]
    fn writes_quotes_in_a_scope_as_sql_strings() {
        let scope = ProjectScope::new("/it's", ["/b", "/it's/"]);
        assert_eq!(
            scope.where_clause("cwd"),
            "(lower(rtrim(replace(cwd, char(92), '/'), '/')) = '/it''s' OR lower(rtrim(replace(cwd, char(92), '/'), '/')) = '/b')"
        );
    }
}

//! The intent bar's help (IntentBar, command-history, completions): the
//! commands typed before, and completions for the token being typed.

use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::sync::OnceLock;
use std::time::{Duration, Instant};

use serde::{Deserialize, Serialize};

/// Inputs kept, most recent first.
const MAX_ENTRIES: usize = 1000;
/// History rows a search offers.
pub const MAX_HISTORY: usize = 8;
/// History rows shown above completions.
pub const MAX_HISTORY_WITH_COMPLETIONS: usize = 3;
const MAX_RESULTS: usize = 12;
/// The pause in typing before completions are worked out.
pub const DEBOUNCE: Duration = Duration::from_millis(80);

const EXECUTABLES_TTL: Duration = Duration::from_secs(5 * 60);
const DIR_TTL: Duration = Duration::from_secs(5);
const BRANCH_TTL: Duration = Duration::from_secs(10);
const SCRIPT_TTL: Duration = Duration::from_secs(30);

/// One submitted input.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Entry {
    pub text: String,
    pub kind: String,
    pub timestamp: u64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub project_path: Option<String>,
}

/// The commands typed into shells, kept on disk beside the app's view.
#[derive(Debug, Default)]
pub struct History {
    file: Option<PathBuf>,
    entries: Vec<Entry>,
}

pub fn history_file(data_dir: &Path) -> PathBuf {
    data_dir.join("desktop").join("command-history.json")
}

/// `isSubsequence`.
fn subsequence(query: &str, text: &str) -> bool {
    let mut q = query.chars().peekable();
    for c in text.chars() {
        if q.peek() == Some(&c) {
            q.next();
        }
    }
    q.peek().is_none()
}

impl History {
    pub fn load(file: Option<PathBuf>) -> History {
        let entries = file
            .as_deref()
            .and_then(|f| std::fs::read_to_string(f).ok())
            .and_then(|s| serde_json::from_str(&s).ok())
            .unwrap_or_default();
        History { file, entries }
    }

    fn shell(&self) -> impl Iterator<Item = &Entry> {
        self.entries.iter().filter(|e| e.kind == "shell")
    }

    /// `recordCommand`: to the front, once.
    pub fn record(&mut self, text: &str, project: Option<&str>, now_ms: u64) {
        let text = text.trim();
        if text.is_empty() {
            return;
        }
        self.entries
            .retain(|e| !(e.text == text && e.kind == "shell"));
        self.entries.insert(
            0,
            Entry {
                text: text.to_owned(),
                kind: "shell".to_owned(),
                timestamp: now_ms,
                project_path: project.map(str::to_owned),
            },
        );
        self.entries.truncate(MAX_ENTRIES);
        let Some(f) = &self.file else { return };
        if let Some(dir) = f.parent() {
            let _ = std::fs::create_dir_all(dir);
        }
        // A full disk costs the history, not the command.
        if let Ok(s) = serde_json::to_string(&self.entries) {
            let _ = std::fs::write(f, s);
        }
    }

    /// `searchHistory`: prefix over substring over subsequence, this
    /// project's and recent ones first; the most recent for no query.
    pub fn search(&self, query: &str, project: Option<&str>) -> Vec<Entry> {
        let q = query.trim().to_lowercase();
        let here = |e: &Entry| e.project_path.as_deref() == project;
        if q.is_empty() {
            let mut all: Vec<&Entry> = self.shell().collect();
            all.sort_by(|a, b| here(b).cmp(&here(a)).then(b.timestamp.cmp(&a.timestamp)));
            return all.into_iter().take(MAX_HISTORY).cloned().collect();
        }
        let mut scored: Vec<(i64, &Entry)> = Vec::new();
        for (i, e) in self.shell().enumerate() {
            let text = e.text.to_lowercase();
            if text == q {
                continue;
            }
            let mut score = if text.starts_with(&q) {
                3000
            } else if text.contains(&q) {
                2000
            } else if subsequence(&q, &text) {
                1000
            } else {
                continue;
            };
            if here(e) {
                score += 500;
            }
            scored.push((score - i as i64, e));
        }
        scored.sort_by_key(|s| std::cmp::Reverse(s.0));
        scored
            .into_iter()
            .take(MAX_HISTORY)
            .map(|(_, e)| e.clone())
            .collect()
    }

    /// `ghostSuggestion`: the latest input that carries on from `input`.
    pub fn ghost(&self, input: &str, project: Option<&str>) -> Option<&str> {
        if input.is_empty() || input.contains('\n') {
            return None;
        }
        let mut fallback = None;
        for e in self.shell() {
            if e.text.len() <= input.len() || !e.text.starts_with(input) {
                continue;
            }
            if e.project_path.as_deref() == project {
                return Some(&e.text);
            }
            fallback.get_or_insert(e.text.as_str());
        }
        fallback
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Kind {
    Command,
    Subcommand,
    Flag,
    Path,
    Branch,
    Script,
}

impl Kind {
    /// The menu row's icon.
    pub fn icon(self, dir: bool) -> &'static str {
        match self {
            Kind::Command => "terminal",
            Kind::Subcommand => "corner-down-right",
            Kind::Flag => "sliders-horizontal",
            Kind::Branch => "git-branch",
            Kind::Script => "play",
            Kind::Path if dir => "folder",
            Kind::Path => "file",
        }
    }
}

/// A replacement for the token being typed.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Completion {
    pub insert: String,
    pub label: String,
    pub detail: Option<String>,
    pub kind: Kind,
    /// A directory: the menu stays open to go further in.
    pub continues: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Arg {
    Path,
    Dir,
    Branch,
    Script,
    None,
}

#[derive(Debug, Default)]
struct Outline {
    detail: Option<&'static str>,
    arg: Option<Arg>,
    flags: Vec<(&'static str, Option<&'static str>)>,
    sub: Vec<(&'static str, Outline)>,
}

fn o(detail: Option<&'static str>, arg: Option<Arg>) -> Outline {
    Outline {
        detail,
        arg,
        ..Outline::default()
    }
}

impl Outline {
    fn flags(mut self, flags: &[(&'static str, Option<&'static str>)]) -> Outline {
        self.flags = flags.to_vec();
        self
    }

    fn sub(mut self, sub: Vec<(&'static str, Outline)>) -> Outline {
        self.sub = sub;
        self
    }

    fn child(&self, name: &str) -> Option<&Outline> {
        self.sub.iter().find(|(n, _)| *n == name).map(|(_, o)| o)
    }
}

fn git() -> Outline {
    use Arg::{Branch, None as No, Path};
    o(Some("version control"), None).sub(vec![
        ("status", o(Some("working tree status"), Some(No))),
        (
            "add",
            o(Some("stage changes"), Some(Path)).flags(&[("-p", Some("stage hunks"))]),
        ),
        (
            "commit",
            o(Some("record staged changes"), Some(No)).flags(&[
                ("-m", Some("message")),
                ("-a", Some("stage tracked files")),
                ("--amend", Some("rewrite last commit")),
                ("--no-verify", Some("skip hooks")),
            ]),
        ),
        (
            "push",
            o(Some("update remote"), Some(No)).flags(&[
                ("-u", Some("set upstream")),
                ("--force-with-lease", Some("safe force push")),
                ("--tags", Some("push tags")),
            ]),
        ),
        (
            "pull",
            o(Some("fetch and merge"), Some(No)).flags(&[("--rebase", None)]),
        ),
        (
            "fetch",
            o(Some("download refs"), Some(No)).flags(&[("--all", None), ("--prune", None)]),
        ),
        (
            "checkout",
            o(Some("switch branch or restore"), Some(Branch))
                .flags(&[("-b", Some("create branch"))]),
        ),
        (
            "switch",
            o(Some("switch branch"), Some(Branch)).flags(&[("-c", Some("create branch"))]),
        ),
        (
            "branch",
            o(Some("list or manage branches"), Some(Branch)).flags(&[
                ("-d", Some("delete merged")),
                ("-D", Some("force delete")),
                ("-m", Some("rename")),
            ]),
        ),
        (
            "merge",
            o(Some("merge branch"), Some(Branch)).flags(&[("--abort", None), ("--no-ff", None)]),
        ),
        (
            "rebase",
            o(Some("reapply commits"), Some(Branch)).flags(&[
                ("--continue", None),
                ("--abort", None),
                ("-i", Some("interactive")),
            ]),
        ),
        (
            "log",
            o(Some("commit history"), Some(No)).flags(&[("--oneline", None), ("--graph", None)]),
        ),
        (
            "diff",
            o(Some("show changes"), Some(Path)).flags(&[("--staged", None), ("--stat", None)]),
        ),
        (
            "restore",
            o(Some("discard changes"), Some(Path)).flags(&[("--staged", Some("unstage"))]),
        ),
        (
            "reset",
            o(Some("move HEAD"), Some(No)).flags(&[("--hard", None), ("--soft", None)]),
        ),
        (
            "stash",
            o(Some("shelve changes"), None).sub(vec![
                ("push", o(Some("stash working tree"), Some(No))),
                ("pop", o(Some("apply and drop"), Some(No))),
                ("list", o(None, Some(No))),
                ("apply", o(None, Some(No))),
                ("drop", o(None, Some(No))),
            ]),
        ),
        (
            "cherry-pick",
            o(Some("apply a commit"), Some(No)).flags(&[("--continue", None), ("--abort", None)]),
        ),
        (
            "worktree",
            o(Some("manage worktrees"), None).sub(vec![
                ("add", o(Some("create worktree"), Some(Path))),
                ("list", o(None, Some(No))),
                ("remove", o(Some("delete worktree"), Some(Path))),
            ]),
        ),
        (
            "remote",
            o(Some("manage remotes"), Some(No)).flags(&[("-v", None)]),
        ),
        ("tag", o(Some("manage tags"), Some(No))),
        ("show", o(Some("inspect objects"), Some(No))),
        ("blame", o(Some("line authorship"), Some(Path))),
        ("revert", o(Some("undo a commit"), Some(No))),
        ("clone", o(Some("copy a repository"), Some(No))),
        ("init", o(Some("create a repository"), Some(No))),
    ])
}

fn yarn() -> Outline {
    use Arg::{None as No, Script};
    o(Some("package manager"), Some(Script)).sub(vec![
        ("install", o(Some("install dependencies"), Some(No))),
        (
            "add",
            o(Some("add dependency"), Some(No)).flags(&[("-D", Some("dev dependency"))]),
        ),
        ("remove", o(Some("remove dependency"), Some(No))),
        ("run", o(Some("run package script"), Some(Script))),
        ("why", o(Some("explain dependency"), Some(No))),
        ("upgrade", o(Some("upgrade dependencies"), Some(No))),
    ])
}

fn npm() -> Outline {
    use Arg::{None as No, Script};
    o(Some("package manager"), None).sub(vec![
        (
            "install",
            o(Some("install dependencies"), Some(No)).flags(&[("-D", Some("dev dependency"))]),
        ),
        ("run", o(Some("run package script"), Some(Script))),
        ("test", o(None, Some(No))),
        ("ci", o(Some("clean install"), Some(No))),
        ("init", o(None, Some(No))),
        ("publish", o(None, Some(No))),
    ])
}

/// Commands with an outline; anything else completes paths.
fn outlines() -> &'static [(&'static str, Outline)] {
    static ALL: OnceLock<Vec<(&'static str, Outline)>> = OnceLock::new();
    ALL.get_or_init(|| {
        use Arg::{Dir, None as No, Path};
        vec![
            ("git", git()),
            ("yarn", yarn()),
            ("npm", npm()),
            ("pnpm", yarn()),
            ("cd", o(Some("change directory"), Some(Dir))),
            ("ls", o(Some("list directory"), Some(Path))),
            ("cat", o(Some("print file"), Some(Path))),
            ("code", o(Some("open in editor"), Some(Path))),
            ("open", o(Some("open with default app"), Some(Path))),
            ("mkdir", o(Some("create directory"), Some(Dir))),
            (
                "rm",
                o(Some("remove"), Some(Path))
                    .flags(&[("-r", Some("recursive")), ("-f", Some("force"))]),
            ),
            ("cp", o(Some("copy"), Some(Path))),
            ("mv", o(Some("move or rename"), Some(Path))),
            ("touch", o(Some("create file"), Some(Path))),
            ("vim", o(Some("edit file"), Some(Path))),
            ("nano", o(Some("edit file"), Some(Path))),
            ("head", o(None, Some(Path))),
            ("tail", o(None, Some(Path)).flags(&[("-f", Some("follow"))])),
            ("node", o(Some("run JavaScript"), Some(Path))),
            ("python", o(Some("run Python"), Some(Path))),
            ("source", o(Some("run in current shell"), Some(Path))),
            ("which", o(Some("locate executable"), Some(No))),
            (
                "grep",
                o(Some("search text"), Some(Path))
                    .flags(&[("-r", Some("recursive")), ("-i", Some("ignore case"))]),
            ),
        ]
    })
}

fn outline(name: &str) -> Option<&'static Outline> {
    outlines().iter().find(|(n, _)| *n == name).map(|(_, o)| o)
}

/// What completions need fetched from vornd.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub enum Need {
    Executables,
    /// A directory's entries.
    Dir(String),
    /// A repository's local branches.
    Branches(String),
    /// A directory's package scripts.
    Scripts(String),
}

impl Need {
    /// The call that answers it: method and params.
    pub fn call(&self) -> (&'static str, serde_json::Value) {
        use serde_json::json;
        match self {
            Need::Executables => ("shell:listExecutables", json!({})),
            Need::Dir(d) => ("file:listDir", json!({ "dirPath": d })),
            Need::Branches(repo) => ("git:listBranches", json!(repo)),
            Need::Scripts(cwd) => (
                "file:readContent",
                json!({ "filePath": format!("{cwd}/package.json"), "maxBytes": 64 * 1024 }),
            ),
        }
    }

    fn ttl(&self) -> Duration {
        match self {
            Need::Executables => EXECUTABLES_TTL,
            Need::Dir(_) => DIR_TTL,
            Need::Branches(_) => BRANCH_TTL,
            Need::Scripts(_) => SCRIPT_TTL,
        }
    }
}

/// A fetched list: names, and for a directory which are directories.
type Fetched = (Instant, Vec<(String, bool)>);

/// What vornd answered, kept a while as the desktop's caches.
#[derive(Debug, Default)]
pub struct Sources {
    got: HashMap<Need, Fetched>,
    /// Asked and not yet answered.
    pub asked: HashSet<Need>,
}

impl Sources {
    /// Keeps vornd's answer to `need`.
    pub fn answer(&mut self, need: Need, answer: Option<&serde_json::Value>, now: Instant) {
        use serde_json::Value;
        self.asked.remove(&need);
        let names = |v: Option<&Value>| -> Vec<(String, bool)> {
            v.and_then(Value::as_array)
                .into_iter()
                .flatten()
                .filter_map(|x| x.as_str().map(|s| (s.to_owned(), false)))
                .collect()
        };
        let list = match &need {
            Need::Executables => names(answer),
            Need::Branches(_) => names(answer.and_then(|a| a.get("local"))),
            Need::Dir(_) => answer
                .and_then(Value::as_array)
                .into_iter()
                .flatten()
                .filter_map(|e| {
                    let name = e.get("name")?.as_str()?.to_owned();
                    let dir = e.get("isDirectory").and_then(Value::as_bool) == Some(true);
                    Some((name, dir))
                })
                .collect(),
            Need::Scripts(_) => answer
                .and_then(Value::as_str)
                .and_then(|s| serde_json::from_str::<Value>(s).ok())
                .and_then(|p| {
                    p.get("scripts")?
                        .as_object()
                        .map(|m| m.keys().map(|k| (k.clone(), false)).collect())
                })
                .unwrap_or_default(),
        };
        self.got.insert(need, (now, list));
    }

    /// The list for `need`, noting in `wants` when it must be fetched.
    fn get(&self, need: Need, now: Instant, wants: &mut Vec<Need>) -> &[(String, bool)] {
        let hit = self.got.get(&need);
        let stale = hit.is_none_or(|(at, _)| now.duration_since(*at) >= need.ttl());
        if stale && !self.asked.contains(&need) && !wants.contains(&need) {
            wants.push(need);
        }
        hit.map_or(&[], |(_, l)| l.as_slice())
    }
}

/// `escapeToken`: one shell word.
fn escape(name: &str) -> String {
    let mut out = String::with_capacity(name.len());
    for c in name.chars() {
        if " '\"\\()&;|<>*?$`#".contains(c) {
            out.push('\\');
        }
        out.push(c);
    }
    out
}

/// `prefixFilter`: names starting with `prefix`, any case, but not it.
fn starts<'a>(names: impl Iterator<Item = &'a str>, prefix: &str) -> Vec<&'a str> {
    let lower = prefix.to_lowercase();
    names
        .filter(|n| n.to_lowercase().starts_with(&lower) && *n != prefix)
        .collect()
}

/// What the engine works from: the shell's directory and its repository.
pub struct Place<'a> {
    pub cwd: Option<&'a str>,
    pub repo: Option<&'a str>,
}

fn paths(
    current: &str,
    place: &Place<'_>,
    dirs_only: bool,
    src: &Sources,
    now: Instant,
    wants: &mut Vec<Need>,
) -> Vec<Completion> {
    if current.starts_with('~') {
        return Vec::new();
    }
    let (dir_part, base) = match current.rfind('/') {
        Some(i) => current.split_at(i + 1),
        None => ("", current),
    };
    let dir = if dir_part.starts_with('/') {
        dir_part.to_owned()
    } else {
        let Some(cwd) = place.cwd else {
            return Vec::new();
        };
        if dir_part.is_empty() {
            cwd.to_owned()
        } else {
            format!("{cwd}/{dir_part}")
        }
    };
    let lower = base.to_lowercase();
    let mut entries: Vec<&(String, bool)> = src
        .get(Need::Dir(dir), now, wants)
        .iter()
        .filter(|(n, d)| n.to_lowercase().starts_with(&lower) && (!dirs_only || *d))
        .collect();
    entries.sort_by(|a, b| b.1.cmp(&a.1).then_with(|| a.0.cmp(&b.0)));
    entries
        .into_iter()
        .take(MAX_RESULTS)
        .map(|(name, d)| {
            let slash = if *d { "/" } else { "" };
            Completion {
                insert: format!("{dir_part}{}{slash}", escape(name)),
                label: format!("{name}{slash}"),
                detail: None,
                kind: Kind::Path,
                continues: *d,
            }
        })
        .collect()
}

/// `getCompletions` for the token at the end of `input`, from what
/// `src` holds; and what it still has to fetch for a better answer.
pub fn complete(
    input: &str,
    place: &Place<'_>,
    src: &Sources,
    now: Instant,
) -> (Vec<Completion>, Vec<Need>) {
    let mut wants = Vec::new();
    let line = input.rsplit('\n').next().unwrap_or("");
    let ends_with_space = line.ends_with(char::is_whitespace);
    let mut words: Vec<&str> = line.split_whitespace().collect();
    let current = if ends_with_space {
        ""
    } else {
        words.pop().unwrap_or("")
    };
    if words.is_empty() {
        if current.is_empty() {
            return (Vec::new(), wants);
        }
        let exes = src.get(Need::Executables, now, &mut wants);
        let mut names: Vec<&str> = outlines().iter().map(|(n, _)| *n).collect();
        let curated = names.len();
        let mut seen: HashSet<&str> = names.iter().copied().collect();
        names.extend(
            exes.iter()
                .map(|(n, _)| n.as_str())
                .filter(|n| seen.insert(n)),
        );
        let mut matches: Vec<(bool, &str)> = starts(names.iter().copied(), current)
            .into_iter()
            .map(|n| (names[..curated].contains(&n), n))
            .collect();
        matches.sort_by(|a, b| b.0.cmp(&a.0).then_with(|| a.1.cmp(b.1)));
        let out = matches
            .into_iter()
            .take(MAX_RESULTS)
            .map(|(_, n)| Completion {
                insert: n.to_owned(),
                label: n.to_owned(),
                detail: outline(n).and_then(|o| o.detail).map(str::to_owned),
                kind: Kind::Command,
                continues: false,
            })
            .collect();
        return (out, wants);
    }
    let mut node = outline(words[0]);
    let mut unmatched = 0;
    for token in &words[1..] {
        if token.starts_with('-') {
            continue;
        }
        match node.and_then(|n| n.child(token)) {
            Some(child) => node = Some(child),
            None => unmatched += 1,
        }
    }
    let mut out = Vec::new();
    if current.starts_with('-') {
        for (flag, detail) in node.map_or(&[][..], |n| n.flags.as_slice()) {
            if flag.starts_with(current) && *flag != current {
                out.push(Completion {
                    insert: (*flag).to_owned(),
                    label: (*flag).to_owned(),
                    detail: detail.map(str::to_owned),
                    kind: Kind::Flag,
                    continues: false,
                });
            }
        }
        out.truncate(MAX_RESULTS);
        return (out, wants);
    }
    if let Some(n) = node.filter(|n| !n.sub.is_empty() && unmatched == 0) {
        for name in starts(n.sub.iter().map(|(s, _)| *s), current) {
            out.push(Completion {
                insert: name.to_owned(),
                label: name.to_owned(),
                detail: n.child(name).and_then(|c| c.detail).map(str::to_owned),
                kind: Kind::Subcommand,
                continues: false,
            });
        }
    }
    let arg = match node {
        Some(n) => n.arg,
        None => Some(Arg::Path),
    };
    let listed = |need: Option<Need>, kind: Kind, detail: Option<&str>, wants: &mut Vec<Need>| {
        let Some(need) = need else {
            return Vec::new();
        };
        starts(
            src.get(need, now, wants).iter().map(|(n, _)| n.as_str()),
            current,
        )
        .into_iter()
        .map(|n| Completion {
            insert: n.to_owned(),
            label: n.to_owned(),
            detail: detail.map(str::to_owned),
            kind,
            continues: false,
        })
        .collect::<Vec<_>>()
    };
    match arg {
        Some(Arg::Branch) => out.extend(listed(
            place.repo.map(|r| Need::Branches(r.to_owned())),
            Kind::Branch,
            None,
            &mut wants,
        )),
        Some(Arg::Script) => out.extend(listed(
            place.cwd.map(|c| Need::Scripts(c.to_owned())),
            Kind::Script,
            Some("package script"),
            &mut wants,
        )),
        Some(a @ (Arg::Path | Arg::Dir)) => {
            out.extend(paths(current, place, a == Arg::Dir, src, now, &mut wants));
        }
        None if current.contains('/') || current.starts_with('.') => {
            out.extend(paths(current, place, false, src, now, &mut wants));
        }
        _ => {}
    }
    out.truncate(MAX_RESULTS);
    (out, wants)
}

/// `applyCompletion`: `input` with its last token replaced.
pub fn apply(input: &str, c: &Completion) -> String {
    let cut = input.rfind(char::is_whitespace).map_or(0, |i| {
        i + input[i..].chars().next().map_or(1, char::len_utf8)
    });
    let space = if c.continues { "" } else { " " };
    format!("{}{}{space}", &input[..cut], c.insert)
}

/// Tab with no row picked: the common start of every completion when it
/// is longer than the token, else the first.
pub fn tab(input: &str, all: &[Completion]) -> Option<String> {
    let first = all.first()?;
    if all.len() > 1 {
        let mut lcp = first.insert.as_str();
        for c in &all[1..] {
            let n = lcp
                .char_indices()
                .zip(c.insert.chars())
                .take_while(|((_, a), b)| a == b)
                .last()
                .map_or(0, |((i, a), _)| i + a.len_utf8());
            lcp = &lcp[..n];
        }
        let token = input.rsplit(char::is_whitespace).next().unwrap_or("");
        if lcp.len() > token.len() {
            let cut = input.len() - token.len();
            return Some(format!("{}{lcp}", &input[..cut]));
        }
    }
    Some(apply(input, first))
}

/// A shell composer's menu: history and completions over what is typed.
#[derive(Debug, Default)]
pub struct Menu {
    pub open: bool,
    pub history: Vec<Entry>,
    pub completions: Vec<Completion>,
    /// The picked row, if any.
    pub highlight: Option<usize>,
    /// The text the menu was worked out for.
    pub text: String,
    /// When the text last changed, for the debounce.
    pub changed: Option<Instant>,
    /// The completions are for `text`.
    pub completed: bool,
    /// The next text change was put in, so it skips the debounce.
    pub filled: bool,
}

/// A menu row.
pub enum Row<'a> {
    History(&'a Entry),
    Completion(&'a Completion),
}

impl Menu {
    pub fn rows(&self) -> Vec<Row<'_>> {
        let cap = if self.completions.is_empty() {
            MAX_HISTORY
        } else {
            MAX_HISTORY_WITH_COMPLETIONS
        };
        self.history
            .iter()
            .take(cap)
            .map(Row::History)
            .chain(self.completions.iter().map(Row::Completion))
            .collect()
    }

    pub fn len(&self) -> usize {
        self.rows().len()
    }

    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }

    pub fn shown(&self) -> bool {
        self.open && !self.is_empty()
    }

    pub fn close(&mut self) {
        self.open = false;
        self.highlight = None;
    }

    pub fn step(&mut self, by: isize) {
        let n = self.len();
        if n == 0 {
            return;
        }
        let at = self
            .highlight
            .map_or(if by > 0 { -1 } else { 0 }, |h| h as isize);
        self.highlight = Some((at + by).clamp(0, n as isize - 1) as usize);
    }
}

/// What a block's or the composer menu's controls do.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub enum IntentAct {
    /// Copies a block's command and output.
    Copy {
        session: String,
        key: (u32, u64, u64),
    },
    /// Runs or fills in menu row `index`.
    Pick { session: String, index: usize },
}

/// History, caches and every shell's menu.
#[derive(Debug, Default)]
pub struct Intent {
    pub history: History,
    pub sources: Sources,
    pub menus: HashMap<String, Menu>,
    /// The block whose Copy was last pressed, and when.
    pub copied: Option<((u32, u64, u64), Instant)>,
    /// Shells whose composer stood aside for a running command while focused.
    pub stood_aside: HashSet<String>,
}

/// Relative time for a history row: now, m, h, d.
pub fn ago(then_ms: u64, now_ms: u64) -> String {
    let s = now_ms.saturating_sub(then_ms) / 1000;
    match s {
        0..60 => "now".to_owned(),
        60..3600 => format!("{}m", s / 60),
        3600..86_400 => format!("{}h", s / 3600),
        _ => format!("{}d", s / 86_400),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn place() -> Place<'static> {
        Place {
            cwd: Some("/p"),
            repo: Some("/p"),
        }
    }

    fn labels(c: &[Completion]) -> Vec<&str> {
        c.iter().map(|c| c.label.as_str()).collect()
    }

    #[test]
    fn history_ranks_prefix_then_substring_then_subsequence() {
        let mut h = History::default();
        h.record("git status", Some("/a"), 1);
        h.record("echo git", Some("/b"), 2);
        h.record("grep -i t", Some("/a"), 3);
        h.record("git status", Some("/a"), 4);
        let found: Vec<_> = h
            .search("git", Some("/a"))
            .into_iter()
            .map(|e| e.text)
            .collect();
        assert_eq!(found, vec!["git status", "echo git", "grep -i t"]);
        assert!(h.search("git status", Some("/a")).is_empty());
        assert_eq!(h.search("", Some("/b"))[0].text, "echo git");
        assert_eq!(h.ghost("git s", Some("/z")), Some("git status"));
        assert_eq!(h.ghost("git status", None), None);
    }

    #[test]
    fn history_survives_a_restart() {
        let dir = std::env::temp_dir().join(format!("vorn-intent-{}", std::process::id()));
        let file = history_file(&dir);
        History::load(Some(file.clone())).record("ls -la", None, 1);
        assert_eq!(History::load(Some(file)).search("", None)[0].text, "ls -la");
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn commands_complete_from_outlines_then_the_path() {
        let now = Instant::now();
        let mut src = Sources::default();
        let (c, wants) = complete("gi", &place(), &src, now);
        assert_eq!(labels(&c), vec!["git"]);
        assert_eq!(wants, vec![Need::Executables]);
        src.answer(Need::Executables, Some(&json!(["gitk", "git", "go"])), now);
        let (c, wants) = complete("gi", &place(), &src, now);
        assert_eq!(labels(&c), vec!["git", "gitk"]);
        assert!(wants.is_empty());
    }

    #[test]
    fn subcommands_flags_branches_and_scripts() {
        let now = Instant::now();
        let mut src = Sources::default();
        let (c, _) = complete("git sta", &place(), &src, now);
        assert_eq!(labels(&c), vec!["status", "stash"]);
        assert_eq!(tab("git st", &c).as_deref(), Some("git sta"));
        let (c, _) = complete("git commit --a", &place(), &src, now);
        assert_eq!(labels(&c), vec!["--amend"]);
        src.answer(
            Need::Branches("/p".into()),
            Some(&json!({"local": ["main", "feature"]})),
            now,
        );
        let (c, _) = complete("git checkout ma", &place(), &src, now);
        assert_eq!(labels(&c), vec!["main"]);
        src.answer(
            Need::Scripts("/p".into()),
            Some(&json!(r#"{"scripts":{"dev":"x","test":"y"}}"#)),
            now,
        );
        let (c, _) = complete("yarn ", &place(), &src, now);
        assert_eq!(
            labels(&c),
            vec!["install", "add", "remove", "run", "why", "upgrade", "dev", "test"]
        );
    }

    #[test]
    fn paths_put_directories_first_and_escape_names() {
        let now = Instant::now();
        let mut src = Sources::default();
        let (_, wants) = complete("cd s", &place(), &src, now);
        assert_eq!(wants, vec![Need::Dir("/p".into())]);
        src.answer(
            Need::Dir("/p".into()),
            Some(&json!([
                {"name": "src", "isDirectory": true},
                {"name": "setup file", "isDirectory": false},
                {"name": "scripts", "isDirectory": true}
            ])),
            now,
        );
        let (c, _) = complete("cat s", &place(), &src, now);
        assert_eq!(labels(&c), vec!["scripts/", "src/", "setup file"]);
        assert_eq!(c[2].insert, "setup\\ file");
        assert!(c[0].continues);
        let (c, _) = complete("cd s", &place(), &src, now);
        assert_eq!(labels(&c), vec!["scripts/", "src/"]);
        assert_eq!(apply("cd s", &c[1]), "cd src/");
        assert_eq!(tab("cd s", &c).as_deref(), Some("cd scripts/"));
        let (c, _) = complete("cd sc", &place(), &src, now);
        assert_eq!(tab("cd sc", &c).as_deref(), Some("cd scripts/"));
        let (c, _) = complete("cat se", &place(), &src, now);
        assert_eq!(tab("cat se", &c).as_deref(), Some("cat setup\\ file "));
    }

    #[test]
    fn the_menu_moves_within_its_rows() {
        let mut m = Menu {
            history: vec![
                Entry {
                    text: "a".into(),
                    kind: "shell".into(),
                    timestamp: 0,
                    project_path: None,
                };
                2
            ],
            ..Menu::default()
        };
        m.step(1);
        assert_eq!(m.highlight, Some(0));
        m.step(5);
        assert_eq!(m.highlight, Some(1));
        m.step(-1);
        assert_eq!(m.highlight, Some(0));
        assert_eq!(ago(0, 90_000), "1m");
    }
}

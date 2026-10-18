//! Completions for the token being typed in a shell's composer: commands,
//! their subcommands and flags from a small outline of each, and paths,
//! branches and package scripts read through Vorn and kept a while.

use std::collections::{HashMap, HashSet};
use std::sync::OnceLock;
use std::time::{Duration, Instant};

use vorn_extension_sdk::{json, serde_json, Host, Value};

const MAX_RESULTS: usize = 12;

const EXECUTABLES_TTL: Duration = Duration::from_secs(5 * 60);
const DIR_TTL: Duration = Duration::from_secs(5);
const BRANCH_TTL: Duration = Duration::from_secs(10);
const SCRIPT_TTL: Duration = Duration::from_secs(30);

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
    /// The core method that answers it, and its args.
    pub fn call(&self) -> (&'static str, Value) {
        match self {
            Need::Executables => ("shell.executables", json!({})),
            Need::Dir(d) => ("files.list", json!({ "path": d })),
            Need::Branches(repo) => ("git.branches", json!({ "worktree": repo })),
            Need::Scripts(cwd) => (
                "files.read",
                json!({ "path": format!("{cwd}/package.json"), "maxBytes": 64 * 1024 }),
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
    pub fn answer(&mut self, need: Need, answer: Option<&Value>, now: Instant) {
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

/// The completions for the token at the end of `input`, reading through
/// `host` what the cache does not hold fresh.
pub fn complete_with(
    host: &mut dyn Host,
    src: &mut Sources,
    input: &str,
    place: &Place<'_>,
) -> Vec<Completion> {
    let now = Instant::now();
    let (found, wants) = complete(input, place, src, now);
    if wants.is_empty() {
        return found;
    }
    for need in wants {
        let (method, args) = need.call();
        let answer = host.call(method, args).ok();
        src.answer(need, answer.as_ref(), now);
    }
    complete(input, place, src, now).0
}

/// One shell word.
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

/// Names starting with `prefix`, any case, but not it.
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

/// The completions for the token at the end of `input`, from what
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

/// `input` with its last token replaced.
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

#[cfg(test)]
mod tests {
    use super::*;
    use vorn_extension_sdk::testing::FakeHost;

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
    fn what_the_cache_lacks_is_read_through_vorn_once() {
        let mut host = FakeHost::default()
            .answer("shell.executables", |_| Ok(json!(["gitk", "git", "go"])))
            .answer("files.list", |a| {
                assert_eq!(a["path"], "/p");
                Ok(json!([{"name": "src", "isDirectory": true}]))
            });
        let mut src = Sources::default();
        let c = complete_with(&mut host, &mut src, "gi", &place());
        assert_eq!(labels(&c), vec!["git", "gitk"]);
        let c = complete_with(&mut host, &mut src, "git", &place());
        assert_eq!(labels(&c), vec!["gitk"]);
        assert_eq!(host.asked("shell.executables").len(), 1);
        let c = complete_with(&mut host, &mut src, "cd s", &place());
        assert_eq!(labels(&c), vec!["src/"]);
    }
}

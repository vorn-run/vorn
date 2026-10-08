//! The file explorer's calls on this machine: list a directory, read, stamp
//! and write a file, as the server's `file-utils` answers them.
//!
//! A call naming a remote host is the server's, which reaches it over SSH;
//! the dispatcher forwards those before they get here.

use std::cmp::Ordering;
use std::collections::{HashMap, HashSet};
use std::io::Read;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant, UNIX_EPOCH};

use serde_json::{json, Value};

use super::env::SafeEnv;

/// Names never listed.
const ALWAYS_EXCLUDE: [&str; 3] = [".git", ".DS_Store", "Thumbs.db"];

/// What a read returns at most unless the caller says otherwise: 512 KB.
pub const MAX_READ_BYTES: u64 = 512 * 1024;

/// How long one repository's ignored paths are reused.
const IGNORE_TTL: Duration = Duration::from_secs(30);

/// Bytes checked for a NUL before a file is taken for text.
const BINARY_CHECK: usize = 8192;

/// One repository's ignored paths, and when they were read.
type Ignored = (Instant, Arc<HashSet<String>>);

/// The ignored paths of each repository listed recently, by its root.
#[derive(Debug, Default)]
pub struct IgnoreCache {
    roots: Mutex<HashMap<String, Ignored>>,
}

/// One entry of a directory listing.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FileEntry {
    pub name: String,
    pub path: String,
    pub is_directory: bool,
}

impl FileEntry {
    pub fn to_json(&self) -> Value {
        json!({ "name": self.name, "path": self.path, "isDirectory": self.is_directory })
    }
}

/// `dir`'s entries, directories first and then by name as the server sorts
/// them, without dotfiles (but `.github`) and without what git ignores.
/// Empty when the directory cannot be read.
pub fn list_dir(dir: &str, env: &Arc<SafeEnv>, cache: &IgnoreCache) -> Vec<FileEntry> {
    let ignored = git_ignored(dir, env, cache);
    let Ok(read) = std::fs::read_dir(dir) else {
        return Vec::new();
    };
    let mut out = Vec::new();
    for entry in read.flatten() {
        let name = entry.file_name().to_string_lossy().into_owned();
        if ALWAYS_EXCLUDE.contains(&name.as_str()) {
            continue;
        }
        if name.starts_with('.') && name != ".github" {
            continue;
        }
        let path = node_join(dir, &name);
        if ignored.as_ref().is_some_and(|set| set.contains(&path)) {
            continue;
        }
        // Not followed: a link to a directory is not one, as for a Dirent.
        let is_directory = entry.file_type().is_ok_and(|t| t.is_dir());
        out.push(FileEntry {
            name,
            path,
            is_directory,
        });
    }
    // Stable, as the server's sort is: equal names keep the order read.
    out.sort_by(|a, b| {
        b.is_directory
            .cmp(&a.is_directory)
            .then_with(|| locale_compare(&a.name, &b.name))
    });
    out
}

/// The paths git ignores in the repository `dir` is in, as absolute paths;
/// `None` outside a repository or when git fails.
fn git_ignored(dir: &str, env: &Arc<SafeEnv>, cache: &IgnoreCache) -> Option<Arc<HashSet<String>>> {
    let git = vorn_git::repo::Git {
        bin: env.git_bin(),
        env: env.get(),
        ssh: None,
    };
    let request = |args: &[&str], cwd: &str, timeout_ms: u64| vorn_git::Request {
        bin: git.bin.clone(),
        args: args.iter().map(|a| (*a).to_owned()).collect(),
        cwd: cwd.into(),
        env: git.env.clone(),
        timeout: Duration::from_millis(timeout_ms),
        max_buffer: vorn_git::repo::DEFAULT_MAX_BUFFER,
    };
    let root = vorn_git::run(&request(&["rev-parse", "--show-toplevel"], dir, 3000)).ok()?;
    let root = root.stdout.trim().to_owned();
    if root.is_empty() {
        return None;
    }
    {
        let roots = cache.roots.lock().unwrap_or_else(|e| e.into_inner());
        if let Some((at, set)) = roots.get(&root) {
            if at.elapsed() < IGNORE_TTL {
                return Some(Arc::clone(set));
            }
        }
    }
    let args = [
        "ls-files",
        "--others",
        "--ignored",
        "--exclude-standard",
        "--directory",
    ];
    let out = vorn_git::run(&request(&args, &root, 5000)).ok()?;
    let lines = out.stdout.trim();
    let set: HashSet<String> = if lines.is_empty() {
        HashSet::new()
    } else {
        lines
            .split('\n')
            .map(|p| node_join(&root, p.strip_suffix('/').unwrap_or(p)))
            .collect()
    };
    let set = Arc::new(set);
    cache
        .roots
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .insert(root, (Instant::now(), Arc::clone(&set)));
    Some(set)
}

/// The start of a file as text, at most `max_bytes` of it, marked when cut;
/// `None` for anything that is not a readable file of text.
pub fn read_content(path: &str, max_bytes: u64) -> Option<String> {
    let meta = std::fs::metadata(path).ok()?;
    if !meta.is_file() {
        return None;
    }
    let size = meta.len();
    let want = usize::try_from(size.min(max_bytes)).ok()?;
    let mut buf = Vec::with_capacity(want);
    std::fs::File::open(path)
        .ok()?
        .take(want as u64)
        .read_to_end(&mut buf)
        .ok()?;
    if buf[..buf.len().min(BINARY_CHECK)].contains(&0) {
        return None;
    }
    let mut text = String::from_utf8_lossy(&buf).into_owned();
    if (buf.len() as u64) < size {
        text.push_str(&format!("\n\n--- truncated ({size} bytes total) ---"));
    }
    Some(text)
}

/// A file's size and modification time in whole milliseconds; `None` for
/// anything that is not a file this can stat.
pub fn stamp(path: &str) -> Option<Value> {
    let meta = std::fs::metadata(path).ok()?;
    if !meta.is_file() {
        return None;
    }
    let modified = meta.modified().ok()?;
    let ms = match modified.duration_since(UNIX_EPOCH) {
        Ok(d) => i64::try_from(d.as_millis()).ok()?,
        // Before 1970, floored as `Math.floor` floors a negative.
        Err(e) => {
            let d = e.duration();
            let whole = i64::try_from(d.as_millis()).ok()?;
            if d.subsec_nanos() % 1_000_000 == 0 {
                -whole
            } else {
                -whole - 1
            }
        }
    };
    Some(json!({ "size": meta.len(), "mtimeMs": ms }))
}

/// Writes `content` over the file, creating it; the error is worded as Node
/// words it.
pub fn write_content(path: &str, content: &str) -> Value {
    match std::fs::write(path, content) {
        Ok(()) => json!({ "success": true }),
        Err(error) => json!({
            "success": false,
            "error": vorn_git::fs_message("open", path, &error),
        }),
    }
}

/// The same calls for a remote host's files, each one ssh command as the
/// server's `file-utils` runs them there; a failure reads as nothing there.
pub mod remote {
    use std::time::Duration;

    use serde_json::{json, Value};
    use vorn_remote::{quote, Login};

    use super::{locale_compare, FileEntry, ALWAYS_EXCLUDE, BINARY_CHECK};

    const TIMEOUT: Duration = Duration::from_secs(10);
    const SEP: &str = "__VORN_SEP__";

    /// `ls` of `dir` beside git's ignored paths there.
    pub fn list_dir(login: &Login, dir: &str) -> Vec<FileEntry> {
        let q = quote(dir);
        let cmd = format!(
            "ls -1aF {q} && echo '{SEP}' && (cd {q} && git ls-files --others --ignored --exclude-standard --directory 2>/dev/null || true)"
        );
        let Ok(out) = login.exec(&cmd, None, TIMEOUT) else {
            return Vec::new();
        };
        let (listed, ignored) = out
            .split_once(&format!("{SEP}\n"))
            .unwrap_or((out.as_str(), ""));
        if listed.trim().is_empty() {
            return Vec::new();
        }
        let ignored: Vec<&str> = ignored
            .trim()
            .split('\n')
            .map(|p| p.strip_suffix('/').unwrap_or(p))
            .filter(|p| !p.is_empty())
            .collect();
        let mut entries: Vec<FileEntry> = listed
            .trim()
            .split('\n')
            .filter_map(|line| {
                let is_directory = line.ends_with('/');
                let name = line.strip_suffix(['/', '@', '*', '|', '=']).unwrap_or(line);
                let skip = name.is_empty()
                    || name == "."
                    || name == ".."
                    || ALWAYS_EXCLUDE.contains(&name)
                    || (name.starts_with('.') && name != ".github")
                    || ignored.contains(&name);
                (!skip).then(|| FileEntry {
                    name: name.to_owned(),
                    path: format!("{dir}/{name}"),
                    is_directory,
                })
            })
            .collect();
        entries.sort_by(|a, b| {
            b.is_directory
                .cmp(&a.is_directory)
                .then_with(|| locale_compare(&a.name, &b.name))
        });
        entries
    }

    /// The first `max_bytes` of a file as text, marked when that many came back.
    pub fn read_content(login: &Login, path: &str, max_bytes: u64) -> Option<String> {
        let text = login
            .exec(
                &format!("head -c {max_bytes} {}", quote(path)),
                None,
                TIMEOUT,
            )
            .ok()?;
        if text.chars().take(BINARY_CHECK).any(|c| c == '\0') {
            return None;
        }
        // `head -c` cuts silently, and a cut file saved back would lose its tail.
        if text.len() as u64 >= max_bytes {
            return Some(format!("{text}\n\n--- truncated ---"));
        }
        Some(text)
    }

    /// Writes the file through ssh's stdin, as `cat > file`.
    pub fn write_content(login: &Login, path: &str, content: &str) -> Value {
        let cmd = format!("cat > {}", quote(path));
        match login.exec(&cmd, Some(content.as_bytes()), Duration::from_secs(30)) {
            Ok(_) => json!({ "success": true }),
            Err(error) => json!({ "success": false, "error": error }),
        }
    }

    /// Size and whole-second modification time, from GNU or BSD `stat`.
    pub fn stamp(login: &Login, path: &str) -> Option<Value> {
        let q = quote(path);
        let script = format!("stat -c '%s %Y' {q} 2>/dev/null || stat -f '%z %m' {q} 2>/dev/null");
        let out = login.exec(&script, None, TIMEOUT).ok()?;
        let mut parts = out.split_whitespace();
        let size: f64 = parts.next()?.parse().ok()?;
        let seconds: f64 = parts.next()?.parse().ok()?;
        // Whole numbers as JSON integers, as `Number` prints them.
        let num = |n: f64| {
            if n.fract() == 0.0 && n.abs() < 9e15 {
                json!(n as i64)
            } else {
                json!(n)
            }
        };
        (size.is_finite() && seconds.is_finite())
            .then(|| json!({ "size": num(size), "mtimeMs": num(seconds * 1000.0) }))
    }
}

/// Node's `path.join` for two parts: joined with the separator, then
/// normalised (`.` and `..` resolved, repeated separators collapsed).
pub fn node_join(base: &str, child: &str) -> String {
    let joined = if base.is_empty() {
        child.to_owned()
    } else if child.is_empty() {
        base.to_owned()
    } else {
        format!("{base}/{child}")
    };
    normalize(&joined)
}

/// Node's `path.posix.normalize`.
fn normalize(path: &str) -> String {
    if path.is_empty() {
        return ".".to_owned();
    }
    let absolute = path.starts_with('/');
    let trailing = path.ends_with('/');
    let mut parts: Vec<&str> = Vec::new();
    for part in path.split('/') {
        match part {
            "" | "." => {}
            ".." => {
                if parts.last().is_some_and(|p| *p != "..") {
                    parts.pop();
                } else if !absolute {
                    parts.push("..");
                }
            }
            other => parts.push(other),
        }
    }
    let mut out = parts.join("/");
    if out.is_empty() && !absolute {
        out.push('.');
    }
    if trailing && !out.is_empty() {
        out.push('/');
    }
    if absolute {
        out.insert(0, '/');
    }
    out
}

/// `a.localeCompare(b)` as Node's default collation orders file names: by
/// base letters first, ignoring case and accents; then by accents; then
/// lower case before upper. Punctuation and symbols sort before digits and
/// digits before letters, each in the root collation's order. Exact for
/// ASCII and the accented Latin letters; other symbols sort before digits
/// and other letters after the Latin ones, each by code point.
pub fn locale_compare(a: &str, b: &str) -> Ordering {
    let ka: Vec<Weights> = a.chars().filter_map(weights).collect();
    let kb: Vec<Weights> = b.chars().filter_map(weights).collect();
    let level = |f: fn(&Weights) -> u32| {
        ka.iter()
            .map(f)
            .filter(|w| *w != 0)
            .cmp(kb.iter().map(f).filter(|w| *w != 0))
    };
    level(|w| w.primary)
        .then_with(|| level(|w| w.secondary))
        .then_with(|| level(|w| w.tertiary))
}

/// One character's collation weights; 0 is "nothing at this level".
#[derive(Debug, Clone, Copy)]
struct Weights {
    primary: u32,
    secondary: u32,
    tertiary: u32,
}

/// The root collation's order of ASCII punctuation and symbols.
const PUNCTUATION: &str = " _-,;:!?.'\"()[]{}@*/\\&#%`^+<=>|~$";

/// Weights for one character; `None` for a control character, which the
/// collation ignores.
fn weights(c: char) -> Option<Weights> {
    // Primary ranges, in the root collation's order of the classes.
    const SYMBOLS: u32 = 100;
    const DIGITS: u32 = 2_000_000;
    const LETTERS: u32 = 3_000_000;
    const OTHER_LETTERS: u32 = 4_000_000;
    if c.is_control() {
        return None;
    }
    let plain = |primary| Weights {
        primary,
        secondary: 1,
        tertiary: 1,
    };
    if let Some(at) = PUNCTUATION.find(c) {
        return Some(plain(1 + at as u32));
    }
    if c.is_ascii_digit() {
        return Some(plain(DIGITS + (c as u32 - '0' as u32)));
    }
    let (base, accent) = fold_accent(c);
    if base.is_ascii_alphabetic() {
        let lower = base.to_ascii_lowercase();
        return Some(Weights {
            primary: LETTERS + (lower as u32 - 'a' as u32),
            secondary: 1 + accent,
            tertiary: if base.is_ascii_uppercase() { 2 } else { 1 },
        });
    }
    if !c.is_alphanumeric() {
        return Some(plain(SYMBOLS + c as u32));
    }
    let lower = c.to_lowercase().next().unwrap_or(c);
    Some(Weights {
        primary: OTHER_LETTERS + lower as u32,
        secondary: 1,
        tertiary: if lower == c { 1 } else { 2 },
    })
}

/// An accented Latin letter as its base letter and which accent it carries
/// (0 for none), in the order the root collation ranks the accents.
fn fold_accent(c: char) -> (char, u32) {
    const TABLE: &[(&str, char)] = &[
        ("áàâäãåā", 'a'),
        ("ÁÀÂÄÃÅĀ", 'A'),
        ("çć", 'c'),
        ("ÇĆ", 'C'),
        ("éèêëē", 'e'),
        ("ÉÈÊËĒ", 'E'),
        ("íìîïī", 'i'),
        ("ÍÌÎÏĪ", 'I'),
        ("ñ", 'n'),
        ("Ñ", 'N'),
        ("óòôöõō", 'o'),
        ("ÓÒÔÖÕŌ", 'O'),
        ("úùûüū", 'u'),
        ("ÚÙÛÜŪ", 'U'),
        ("ýÿ", 'y'),
        ("ÝŸ", 'Y'),
    ];
    // Acute, grave, circumflex, ring, diaeresis, tilde, macron, cedilla: the
    // order of their secondary weights.
    const ACCENTS: &[char] = &[
        '\u{301}', '\u{300}', '\u{302}', '\u{30a}', '\u{308}', '\u{303}', '\u{304}', '\u{327}',
    ];
    for (letters, base) in TABLE {
        if let Some(i) = letters.chars().position(|l| l == c) {
            let accent = accent_of(base, i);
            let rank = ACCENTS.iter().position(|a| *a == accent).unwrap_or(0) as u32;
            return (*base, 1 + rank);
        }
    }
    (c, 0)
}

/// Which accent the `i`th letter of `base`'s row carries.
fn accent_of(base: &char, i: usize) -> char {
    let row: &[char] = match base.to_ascii_lowercase() {
        'a' => &[
            '\u{301}', '\u{300}', '\u{302}', '\u{308}', '\u{303}', '\u{30a}', '\u{304}',
        ],
        'c' => &['\u{327}', '\u{301}'],
        'e' | 'i' | 'u' => &['\u{301}', '\u{300}', '\u{302}', '\u{308}', '\u{304}'],
        'n' => &['\u{303}'],
        'o' => &[
            '\u{301}', '\u{300}', '\u{302}', '\u{308}', '\u{303}', '\u{304}',
        ],
        'y' => &['\u{301}', '\u{308}'],
        _ => &[],
    };
    row.get(i).copied().unwrap_or('\u{301}')
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sorted(names: &[&str]) -> Vec<String> {
        let mut v: Vec<String> = names.iter().map(|s| (*s).to_owned()).collect();
        v.sort_by(|a, b| locale_compare(a, b));
        v
    }

    #[test]
    fn orders_ascii_as_node_does() {
        // `[...printable ASCII].sort((a, b) => a.localeCompare(b))` in Node 22.
        let node = " _-,;:!?.'\"()[]{}@*/\\&#%`^+<=>|~$0123456789aAbBcCdDeEfFgGhHiIjJkKlLmMnNoOpPqQrRsStTuUvVwWxXyYzZ";
        let mut chars: Vec<String> = (32u8..127).map(|b| (b as char).to_string()).collect();
        chars.sort_by(|a, b| locale_compare(a, b));
        assert_eq!(chars.concat(), node);
    }

    #[test]
    fn compares_letters_before_accents_before_case() {
        // Node 22's order for the same names.
        assert_eq!(
            sorted(&[
                "zz", "Zz", "b", "B", "ab", "Aa", "a-b", "a_b", "a.b", "a b", "ab1", "ab10", "ab2",
                "é", "e", "f", "E", "éa", "eb", "résumé", "resume", "Resume", "a", "A", "ä"
            ]),
            [
                "a", "A", "ä", "a b", "a_b", "a-b", "a.b", "Aa", "ab", "ab1", "ab10", "ab2", "b",
                "B", "e", "E", "é", "éa", "eb", "f", "resume", "Resume", "résumé", "zz", "Zz"
            ]
        );
    }

    #[test]
    fn joins_and_normalises_as_node_does() {
        assert_eq!(node_join("/a/b", "c"), "/a/b/c");
        assert_eq!(node_join("/a/b/", "c"), "/a/b/c");
        assert_eq!(node_join("/a/b", "../c"), "/a/c");
        assert_eq!(node_join("/a", "./x/"), "/a/x/");
        assert_eq!(node_join("/", ".."), "/");
        assert_eq!(node_join("a", "../.."), "..");
    }

    #[test]
    fn ranks_accents_as_node_does() {
        // Node 22's order for the same letters.
        assert_eq!(
            sorted(&[
                "e", "é", "è", "ê", "ë", "ē", "E", "É", "È", "a", "á", "à", "â", "ä", "ã", "å",
                "ā", "o", "ö", "õ", "ô", "c", "ç", "ć", "n", "ñ", "y", "ý", "ÿ", "u", "ü", "ū",
                "i", "ï"
            ]),
            [
                "a", "á", "à", "â", "å", "ä", "ã", "ā", "c", "ć", "ç", "e", "E", "é", "É", "è",
                "È", "ê", "ë", "ē", "i", "ï", "n", "ñ", "o", "ô", "ö", "õ", "u", "ü", "ū", "y",
                "ý", "ÿ"
            ]
        );
        assert_eq!(
            sorted(&["ab", "äa", "áb", "àa", "aä"]),
            ["aä", "àa", "äa", "ab", "áb"]
        );
    }

    #[test]
    fn reads_text_and_refuses_what_is_not() {
        let dir = tempfile::tempdir().unwrap();
        let text = dir.path().join("a.txt");
        std::fs::write(&text, "hello\nworld\n").unwrap();
        let p = text.to_str().unwrap();
        assert_eq!(
            read_content(p, MAX_READ_BYTES).as_deref(),
            Some("hello\nworld\n")
        );
        assert_eq!(
            read_content(p, 5).as_deref(),
            Some("hello\n\n--- truncated (12 bytes total) ---")
        );
        let bin = dir.path().join("b.bin");
        std::fs::write(&bin, [b'a', 0, b'b']).unwrap();
        assert_eq!(read_content(bin.to_str().unwrap(), MAX_READ_BYTES), None);
        assert_eq!(
            read_content(dir.path().to_str().unwrap(), MAX_READ_BYTES),
            None
        );
        assert_eq!(read_content("/no/such/file", MAX_READ_BYTES), None);
        // Invalid UTF-8 reads as replacement characters, as Node decodes it.
        let odd = dir.path().join("odd.txt");
        std::fs::write(&odd, [b'a', 0xff, b'b']).unwrap();
        assert_eq!(
            read_content(odd.to_str().unwrap(), 100).as_deref(),
            Some("a\u{fffd}b")
        );
    }

    #[test]
    fn stamps_files_only_and_writes_with_node_errors() {
        let dir = tempfile::tempdir().unwrap();
        let file = dir.path().join("f.txt");
        let p = file.to_str().unwrap();
        assert_eq!(write_content(p, "abc"), json!({ "success": true }));
        let s = stamp(p).unwrap();
        assert_eq!(s["size"], 3);
        assert!(s["mtimeMs"].as_i64().unwrap() > 0);
        assert_eq!(stamp(dir.path().to_str().unwrap()), None);
        assert_eq!(stamp("/no/such/file"), None);
        let missing = dir.path().join("no/such.txt");
        let m = missing.to_str().unwrap();
        assert_eq!(
            write_content(m, "x"),
            json!({ "success": false, "error": format!("ENOENT: no such file or directory, open '{m}'") })
        );
    }
}

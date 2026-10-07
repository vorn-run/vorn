//! How much a worktree holds, and how much of that is build output.
//!
//! `du` and `find` do the walking where they exist: `du` counts allocated
//! blocks, which is what removing a tree gives back, and `find -prune` counts
//! a nested `node_modules` once, by its outermost parent. Elsewhere, or when
//! `find` fails, the tree is walked here. Sizes change slowly, so each is kept
//! for [`SIZE_TTL`] and only measured again when asked to refresh.

use std::collections::{HashMap, HashSet};
use std::io::Read;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::Mutex;
use std::time::{Duration, Instant};

/// How long a measured size is trusted.
pub const SIZE_TTL: Duration = Duration::from_secs(5 * 60);
const DU_TIMEOUT: Duration = Duration::from_secs(45);
const FIND_TIMEOUT: Duration = Duration::from_secs(30);
/// Paths per `du`, so its argument list stays short.
const DU_CHUNK: usize = 50;

/// A worktree's size.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Size {
    pub size_bytes: u64,
    /// The part in build-output directories, which a reinstall rebuilds.
    pub artifact_bytes: u64,
    /// False when sizing failed and nothing was known before: both read 0.
    pub measured: bool,
}

#[derive(Clone, Copy, Debug)]
struct Sample {
    size: Size,
    at: Instant,
}

/// Measured sizes, by worktree path.
#[derive(Debug, Default)]
pub struct Sizes {
    samples: Mutex<HashMap<String, Sample>>,
}

impl Sizes {
    /// `root`'s size: the one kept, unless it is older than [`SIZE_TTL`] or
    /// `refresh` asks again. A failure keeps the last known size, if any.
    pub fn measure(
        &self,
        root: &str,
        artifact_dirs: &[String],
        refresh: bool,
        env: &[(String, String)],
    ) -> Size {
        let kept = self.lock().get(root).copied();
        if let Some(s) = kept.filter(|s| !refresh && s.at.elapsed() < SIZE_TTL) {
            return s.size;
        }
        match measure_now(root, artifact_dirs, env) {
            Some(size) => {
                self.lock().insert(
                    root.to_owned(),
                    Sample {
                        size,
                        at: Instant::now(),
                    },
                );
                size
            }
            None => kept.map(|s| s.size).unwrap_or_default(),
        }
    }

    /// Forgets `prefix` and everything under it, after it changed on disk.
    pub fn invalidate(&self, prefix: &str) {
        let under = |key: &str| {
            key.strip_prefix(prefix).is_some_and(|rest| {
                rest.is_empty() || rest.starts_with(['/', std::path::MAIN_SEPARATOR])
            })
        };
        self.lock().retain(|key, _| !under(key));
    }

    fn lock(&self) -> std::sync::MutexGuard<'_, HashMap<String, Sample>> {
        self.samples.lock().unwrap_or_else(|e| e.into_inner())
    }
}

fn measure_now(root: &str, artifact_dirs: &[String], env: &[(String, String)]) -> Option<Size> {
    let artifacts = find_artifact_dirs(root, artifact_dirs, env);
    let (size_bytes, artifact_bytes) = if cfg!(unix) {
        let total = du_bytes(&[root.to_owned()], env)?;
        let artifacts = if artifacts.is_empty() {
            0
        } else {
            du_bytes(&artifacts, env)?
        };
        (total, artifacts)
    } else {
        let prune: HashSet<PathBuf> = artifacts.iter().map(PathBuf::from).collect();
        let artifacts: u64 = artifacts
            .iter()
            .map(|d| walk_bytes(Path::new(d), &HashSet::new()))
            .sum();
        (walk_bytes(Path::new(root), &prune) + artifacts, artifacts)
    };
    // `du` counts blocks, so rounding could put the part above the whole.
    Some(Size {
        size_bytes,
        artifact_bytes: artifact_bytes.min(size_bytes),
        measured: true,
    })
}

/// The build-output directories inside `root`, each counted once by its
/// outermost match. `.git` and symlinks are never entered.
pub fn find_artifact_dirs(root: &str, names: &[String], env: &[(String, String)]) -> Vec<String> {
    if names.is_empty() {
        return Vec::new();
    }
    if cfg!(unix) {
        let mut args = vec![root.to_owned(), "-type".into(), "d".into(), "(".into()];
        for (i, name) in names.iter().enumerate() {
            if i > 0 {
                args.push("-o".into());
            }
            args.extend(["-name".into(), name.clone()]);
        }
        args.extend([")".into(), "-prune".into(), "-print".into()]);
        if let Ok(out) = run("find", &args, env, FIND_TIMEOUT) {
            return out
                .lines()
                .map(str::trim)
                .filter(|l| !l.is_empty())
                .map(str::to_owned)
                .collect();
        }
    }
    let names: HashSet<&str> = names.iter().map(String::as_str).collect();
    let mut found = Vec::new();
    walk_dirs(Path::new(root), &names, &mut found);
    found
}

fn walk_dirs(dir: &Path, names: &HashSet<&str>, found: &mut Vec<String>) {
    let Ok(entries) = std::fs::read_dir(dir) else {
        return;
    };
    for entry in entries.flatten() {
        let Ok(kind) = entry.file_type() else {
            continue;
        };
        if !kind.is_dir() {
            continue;
        }
        let name = entry.file_name();
        let name = name.to_string_lossy();
        let full = entry.path();
        if names.contains(name.as_ref()) {
            found.push(full.to_string_lossy().into_owned());
        } else if name != ".git" {
            walk_dirs(&full, names, found);
        }
    }
}

/// Bytes `du -sk` reports for `paths`, in chunks. An error when any `du`
/// fails, so nothing partial is taken for the whole.
pub(crate) fn du_bytes(paths: &[String], env: &[(String, String)]) -> Option<u64> {
    let mut kb = 0u64;
    for chunk in paths.chunks(DU_CHUNK) {
        let mut args = vec!["-sk".to_owned()];
        args.extend(chunk.iter().cloned());
        let out = run("du", &args, env, DU_TIMEOUT).ok()?;
        kb += out
            .lines()
            .filter_map(|l| l.split_whitespace().next()?.parse::<u64>().ok())
            .sum::<u64>();
    }
    Some(kb * 1024)
}

/// Bytes of the files under `root`, not entering `prune` or any symlink.
pub(crate) fn walk_bytes(root: &Path, prune: &HashSet<PathBuf>) -> u64 {
    let Ok(entries) = std::fs::read_dir(root) else {
        return 0;
    };
    let mut total = 0;
    for entry in entries.flatten() {
        let Ok(kind) = entry.file_type() else {
            continue;
        };
        let full = entry.path();
        if kind.is_dir() && !prune.contains(&full) {
            total += walk_bytes(&full, prune);
        } else if kind.is_file() {
            total += entry.metadata().map_or(0, |m| m.len());
        }
    }
    total
}

/// Runs `program` with `args` and only `env`, and answers its stdout; an
/// error when it cannot start, fails or outlives `timeout`.
fn run(
    program: &str,
    args: &[String],
    env: &[(String, String)],
    timeout: Duration,
) -> Result<String, String> {
    let mut cmd = Command::new(program);
    cmd.args(args)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::null());
    if !env.is_empty() {
        cmd.env_clear().envs(env.iter().map(|(k, v)| (k, v)));
    }
    let mut child = cmd.spawn().map_err(|e| e.to_string())?;
    // Read on a thread, so a full pipe never stalls the wait below.
    let mut stdout = child.stdout.take().ok_or("no stdout")?;
    let reader = std::thread::spawn(move || {
        let mut out = Vec::new();
        let _ = stdout.read_to_end(&mut out);
        out
    });
    let started = Instant::now();
    let status = loop {
        if let Some(status) = child.try_wait().map_err(|e| e.to_string())? {
            break status;
        }
        if started.elapsed() > timeout {
            let _ = child.kill();
            let _ = child.wait();
            return Err(format!("{program} timed out"));
        }
        std::thread::sleep(Duration::from_millis(10));
    };
    let out = reader
        .join()
        .map_err(|_| format!("{program}'s output was lost"))?;
    if !status.success() {
        return Err(format!("{program} failed"));
    }
    Ok(String::from_utf8_lossy(&out).into_owned())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tree() -> tempfile::TempDir {
        let tmp = tempfile::tempdir().unwrap();
        let root = tmp.path();
        for dir in [
            "src",
            "node_modules/a/node_modules",
            ".git/node_modules",
            "pkg/dist",
        ] {
            std::fs::create_dir_all(root.join(dir)).unwrap();
        }
        std::fs::write(root.join("src/a.txt"), vec![b'a'; 10_000]).unwrap();
        std::fs::write(root.join("node_modules/a/b.js"), vec![b'b'; 20_000]).unwrap();
        std::fs::write(root.join("pkg/dist/c.js"), vec![b'c'; 5_000]).unwrap();
        tmp
    }

    fn names() -> Vec<String> {
        vec!["node_modules".into(), "dist".into()]
    }

    #[test]
    fn finds_build_output_once_by_its_outermost_directory() {
        let tmp = tree();
        let root = tmp.path().to_string_lossy().into_owned();
        let mut found = find_artifact_dirs(&root, &names(), &[]);
        found.sort();
        let mut walked = Vec::new();
        let set: HashSet<&str> = ["node_modules", "dist"].into();
        walk_dirs(tmp.path(), &set, &mut walked);
        walked.sort();
        let at = |p: &str| tmp.path().join(p).to_string_lossy().into_owned();
        // `find` enters `.git`; the walk does not, as the server's does not.
        assert!(found.contains(&at("node_modules")) && found.contains(&at("pkg/dist")));
        assert!(!found.contains(&at("node_modules/a/node_modules")));
        assert_eq!(walked, vec![at("node_modules"), at("pkg/dist")]);
        assert!(find_artifact_dirs(&root, &[], &[]).is_empty());
    }

    #[test]
    fn measures_once_and_again_only_when_asked_or_forgotten() {
        let tmp = tree();
        let root = tmp.path().to_string_lossy().into_owned();
        let sizes = Sizes::default();
        let first = sizes.measure(&root, &names(), false, &[]);
        assert!(first.measured);
        assert!(first.size_bytes >= 35_000, "{first:?}");
        assert!(first.artifact_bytes >= 25_000 && first.artifact_bytes <= first.size_bytes);

        std::fs::write(tmp.path().join("src/big"), vec![b'x'; 200_000]).unwrap();
        assert_eq!(sizes.measure(&root, &names(), false, &[]), first);
        let fresh = sizes.measure(&root, &names(), true, &[]);
        assert!(fresh.size_bytes > first.size_bytes);

        std::fs::remove_file(tmp.path().join("src/big")).unwrap();
        sizes.invalidate(&format!("{root}x"));
        assert_eq!(sizes.measure(&root, &names(), false, &[]), fresh);
        sizes.invalidate(&root);
        assert!(sizes.measure(&root, &names(), false, &[]).size_bytes < fresh.size_bytes);
    }

    #[test]
    fn a_tree_that_cannot_be_measured_reads_unmeasured() {
        let sizes = Sizes::default();
        let gone = std::env::temp_dir().join("vorn-worktrees-no-such-dir");
        let size = sizes.measure(&gone.to_string_lossy(), &names(), false, &[]);
        assert_eq!(size, Size::default());
    }

    #[test]
    fn walks_files_without_the_pruned_directories() {
        let tmp = tree();
        let prune: HashSet<PathBuf> = [tmp.path().join("node_modules")].into();
        assert_eq!(walk_bytes(tmp.path(), &prune), 15_000);
        assert_eq!(walk_bytes(tmp.path(), &HashSet::new()), 35_000);
    }

    #[cfg(unix)]
    #[test]
    fn a_program_that_fails_or_overstays_is_an_error() {
        assert!(run("false", &[], &[], FIND_TIMEOUT).is_err());
        assert!(run("vorn-no-such-program", &[], &[], FIND_TIMEOUT).is_err());
        assert!(run("sleep", &["5".into()], &[], Duration::from_millis(50)).is_err());
        assert_eq!(
            run("echo", &["hi".into()], &[], FIND_TIMEOUT).as_deref(),
            Ok("hi\n")
        );
    }
}

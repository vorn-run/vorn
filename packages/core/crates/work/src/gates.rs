//! A gate's review page (`gate-views.ts`): the HTML a gate shows, kept
//! beside its run in the data directory for this round, opened only with
//! the round's token, and served under a policy that lets it reach nothing.

use std::fs;
use std::path::{Path, PathBuf};

/// The largest review page a gate keeps.
pub const MAX_BYTES: u64 = 5 * 1024 * 1024;

/// A review page runs its own scripts and styles, loads nothing from the
/// network, and cannot leave its frame. Artifacts are served under it too.
pub const CSP: &str = "default-src 'none'; script-src 'unsafe-inline'; style-src 'unsafe-inline'; img-src data: blob:; font-src data:; media-src data:; form-action 'none'; sandbox allow-scripts";

/// An id as one safe path segment.
pub fn segment(id: &str) -> String {
    id.chars()
        .map(|c| {
            if c.is_ascii_alphanumeric() || matches!(c, '.' | '_' | '-') {
                c
            } else {
                '_'
            }
        })
        .collect()
}

fn run_dir(data_dir: &Path, run_id: &str) -> PathBuf {
    data_dir.join("gate-views").join(segment(run_id))
}

/// Where a gate's page for `round` is kept.
pub fn file(data_dir: &Path, run_id: &str, node_id: &str, round: u32) -> PathBuf {
    run_dir(data_dir, run_id).join(format!("{}-{round}.html", segment(node_id)))
}

fn too_big(bytes: u64) -> String {
    format!(
        "The review page is {:.1} MB; the limit is 5 MB.",
        bytes as f64 / 1024.0 / 1024.0
    )
}

/// A token for a page or an artifact: 16 random bytes as hex.
pub fn new_token() -> String {
    uuid::Uuid::new_v4().simple().to_string()
}

/// Keeps this round's page: the HTML itself, or a `.html` file named by
/// path. The token that opens it, or why there is none.
pub fn publish(
    data_dir: &Path,
    run_id: &str,
    node_id: &str,
    round: u32,
    view: &str,
) -> Result<String, String> {
    let text = view.trim();
    if text.is_empty() {
        return Err("The review page came out empty.".into());
    }
    let html = if text.starts_with('<') {
        text.to_owned()
    } else {
        let lower = text.to_ascii_lowercase();
        if !(lower.ends_with(".html") || lower.ends_with(".htm")) {
            return Err(format!(
                "The review page is neither HTML nor a .html file: {}",
                crate::js::head(text, 120)
            ));
        }
        let path = Path::new(text);
        let meta = match fs::metadata(path) {
            Ok(m) => m,
            Err(_) => return Err(format!("The review page file does not exist: {text}")),
        };
        if meta.len() > MAX_BYTES {
            return Err(too_big(meta.len()));
        }
        fs::read_to_string(path).map_err(|e| format!("The review page could not be kept: {e}"))?
    };
    if html.len() as u64 > MAX_BYTES {
        return Err(too_big(html.len() as u64));
    }
    let target = file(data_dir, run_id, node_id, round);
    let kept = target
        .parent()
        .map_or(Ok(()), fs::create_dir_all)
        .and_then(|()| fs::write(&target, html));
    kept.map_err(|e| format!("The review page could not be kept: {e}"))?;
    Ok(new_token())
}

/// Compares tokens without stopping at the first difference.
pub fn same_token(expected: &str, given: &str) -> bool {
    let (a, b) = (expected.as_bytes(), given.as_bytes());
    a.len() == b.len() && a.iter().zip(b).fold(0u8, |acc, (x, y)| acc | (x ^ y)) == 0
}

/// Removes a run's pages.
pub fn remove(data_dir: &Path, run_id: &str) {
    let _ = fs::remove_dir_all(run_dir(data_dir, run_id));
}

/// Removes the pages of runs no longer kept.
pub fn sweep<'a>(data_dir: &Path, kept: impl IntoIterator<Item = &'a str>) {
    let root = data_dir.join("gate-views");
    let Ok(dirs) = fs::read_dir(&root) else {
        return;
    };
    let kept: std::collections::HashSet<String> = kept.into_iter().map(segment).collect();
    for dir in dirs.flatten() {
        let name = dir.file_name().to_string_lossy().into_owned();
        if !kept.contains(&name) {
            let _ = fs::remove_dir_all(dir.path());
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn keeps_html_and_html_files_and_says_why_not_otherwise() {
        let dir = tempfile::tempdir().unwrap();
        let token = publish(dir.path(), "run/1", "gate", 2, "  <h1>Hi</h1> ").unwrap();
        assert_eq!(token.len(), 32);
        let kept = file(dir.path(), "run/1", "gate", 2);
        assert!(kept.ends_with("gate-views/run_1/gate-2.html"));
        assert_eq!(fs::read_to_string(&kept).unwrap(), "<h1>Hi</h1>");

        let page = dir.path().join("page.HTML");
        fs::write(&page, "<p>file</p>").unwrap();
        publish(dir.path(), "r", "g", 1, page.to_str().unwrap()).unwrap();
        assert_eq!(
            fs::read_to_string(file(dir.path(), "r", "g", 1)).unwrap(),
            "<p>file</p>"
        );

        assert_eq!(
            publish(dir.path(), "r", "g", 1, " ").unwrap_err(),
            "The review page came out empty."
        );
        assert_eq!(
            publish(dir.path(), "r", "g", 1, "just words").unwrap_err(),
            "The review page is neither HTML nor a .html file: just words"
        );
        assert!(publish(dir.path(), "r", "g", 1, "/nope/x.html")
            .unwrap_err()
            .starts_with("The review page file does not exist"));
        let big = format!("<{}", "x".repeat(MAX_BYTES as usize));
        assert_eq!(
            publish(dir.path(), "r", "g", 1, &big).unwrap_err(),
            "The review page is 5.0 MB; the limit is 5 MB."
        );
    }

    #[test]
    fn sweeps_pages_of_runs_no_longer_kept() {
        let dir = tempfile::tempdir().unwrap();
        publish(dir.path(), "keep", "g", 1, "<p>").unwrap();
        publish(dir.path(), "drop", "g", 1, "<p>").unwrap();
        sweep(dir.path(), ["keep"]);
        assert!(file(dir.path(), "keep", "g", 1).exists());
        assert!(!file(dir.path(), "drop", "g", 1).exists());
        remove(dir.path(), "keep");
        assert!(!file(dir.path(), "keep", "g", 1).exists());
    }
}

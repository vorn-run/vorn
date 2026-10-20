//! What this app needs of a page's address and text while it shows a
//! session's page for the Browser extension. The address bar's rules and
//! the tabs live in the Browser; these few stay here because the app also
//! builds for the web, where the extension's crate does not, and because
//! only this side can look at the disk.

use std::path::{Path, PathBuf};

use url::Url;

/// What a tab is called before its page has a title.
pub fn display_host(url: &str) -> String {
    if url == "about:blank" {
        return "New tab".to_owned();
    }
    let Ok(parsed) = Url::parse(url) else {
        return url.to_owned();
    };
    if parsed.scheme() == "file" {
        let Some(name) = parsed.path().split('/').rfind(|s| !s.is_empty()) else {
            return url.to_owned();
        };
        return percent_decode(name).unwrap_or_else(|| name.to_owned());
    }
    match (parsed.host_str().filter(|h| !h.is_empty()), parsed.port()) {
        (None, _) => url.to_owned(),
        (Some(h), Some(p)) => format!("{h}:{p}"),
        (Some(h), None) => h.to_owned(),
    }
}

/// `decodeURIComponent`, or `None` where it would throw.
fn percent_decode(s: &str) -> Option<String> {
    let mut out = Vec::with_capacity(s.len());
    let bytes = s.as_bytes();
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] == b'%' {
            let hex = s.get(i + 1..i + 3)?;
            out.push(u8::from_str_radix(hex, 16).ok()?);
            i += 3;
        } else {
            out.push(bytes[i]);
            i += 1;
        }
    }
    String::from_utf8(out).ok()
}

/// Page text on one line, cut at `max` characters.
pub fn flatten_page_text(raw: &str, max: usize) -> String {
    let spaced: String = raw
        .chars()
        .map(|c| if c.is_control() { ' ' } else { c })
        .collect();
    let one_line = spaced.split_whitespace().collect::<Vec<_>>().join(" ");
    // JavaScript counts UTF-16 units; the cut lands on the same character
    // for text in the Basic Multilingual Plane.
    if one_line.encode_utf16().count() > max {
        let cut: String = one_line.chars().take(max).collect();
        format!("{cut}\u{2026}")
    } else {
        one_line
    }
}

/// Where a paged read resumes; a cursor from another page
/// generation starts over.
pub fn parse_cursor(cursor: Option<&str>, generation: u32) -> usize {
    let Some((gen, at)) = cursor.and_then(|c| c.split_once(':')) else {
        return 0;
    };
    if gen.parse::<u32>().ok() != Some(generation) {
        return 0;
    }
    at.parse().unwrap_or(0)
}

/// The design file a `file:` address shows, which its tweaks are kept by.
pub fn design_path(url: &str) -> Option<PathBuf> {
    file_path(&Url::parse(url).ok().filter(|u| u.scheme() == "file")?)
}

/// `url` when the file it names resolves, links followed, inside `root`.
/// The Browser vets an address by its words; only here can a link that
/// leads out of the session's folder be seen.
pub fn file_inside(url: &str, root: &Path) -> Option<String> {
    let parsed = Url::parse(url).ok().filter(|u| u.scheme() == "file")?;
    if parsed.host_str().is_some_and(|h| !h.is_empty()) {
        return None;
    }
    let real_root = root.canonicalize().ok()?;
    let real = file_path(&parsed)?.canonicalize().ok()?;
    real.starts_with(real_root).then(|| url.to_owned())
}

#[cfg(not(target_arch = "wasm32"))]
fn file_path(url: &Url) -> Option<PathBuf> {
    url.to_file_path().ok()
}

/// A page has no files of its own to show.
#[cfg(target_arch = "wasm32")]
fn file_path(_url: &Url) -> Option<PathBuf> {
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn cursors_resume_only_on_the_same_page() {
        assert_eq!(parse_cursor(Some("3:40"), 3), 40);
        assert_eq!(parse_cursor(Some("2:40"), 3), 0);
        assert_eq!(parse_cursor(None, 3), 0);
    }

    #[test]
    fn a_tab_is_named_by_its_host_or_file() {
        assert_eq!(display_host("about:blank"), "New tab");
        assert_eq!(display_host("http://localhost:3000/a"), "localhost:3000");
        assert_eq!(display_host("file:///p/My%20Page.html"), "My Page.html");
    }

    #[test]
    fn page_text_is_one_line() {
        assert_eq!(flatten_page_text("a\n\tb  c", 10), "a b c");
        assert_eq!(flatten_page_text("abcdef", 3), "abc\u{2026}");
    }

    #[test]
    fn a_file_must_resolve_inside_the_root() {
        let dir = std::env::temp_dir().join(format!("vorn-address-{}", std::process::id()));
        let root = dir.join("root");
        std::fs::create_dir_all(&root).unwrap();
        std::fs::write(root.join("in.html"), "x").unwrap();
        std::fs::write(dir.join("out.html"), "x").unwrap();
        let url = |p: &Path| Url::from_file_path(p).unwrap().to_string();
        let inside = url(&root.join("in.html"));
        assert_eq!(file_inside(&inside, &root), Some(inside.clone()));
        assert_eq!(file_inside(&url(&dir.join("out.html")), &root), None);
        assert_eq!(file_inside(&url(&root.join("missing.html")), &root), None);
        assert_eq!(file_inside("https://example.com/", &root), None);
        #[cfg(unix)]
        {
            std::os::unix::fs::symlink(dir.join("out.html"), root.join("link.html")).unwrap();
            assert_eq!(file_inside(&url(&root.join("link.html")), &root), None);
        }
        std::fs::remove_dir_all(&dir).unwrap();
    }
}

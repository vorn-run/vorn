//! Pane pages: which file an address names inside a pack, and the headers
//! it is served with.
//!
//! A page is served from an origin of its own, never the app's: a page and
//! the app on one origin would share storage and sockets, and the
//! permissions an extension asked for would mean nothing.

use std::path::{Component, Path, PathBuf};
use std::sync::OnceLock;

use crate::js;
use crate::manifest::PaneDraws;
use crate::pack::InstalledPack;

/// What a page may be made of, and the type each is served as. A `.cjs`,
/// `.node` or `.wasm` is code the pack's entry can reach, never a page.
const PAGE_TYPES: [(&str, &str); 15] = [
    ("html", "text/html; charset=utf-8"),
    ("css", "text/css; charset=utf-8"),
    ("js", "text/javascript; charset=utf-8"),
    ("mjs", "text/javascript; charset=utf-8"),
    ("json", "application/json; charset=utf-8"),
    ("svg", "image/svg+xml"),
    ("png", "image/png"),
    ("jpg", "image/jpeg"),
    ("jpeg", "image/jpeg"),
    ("webp", "image/webp"),
    ("gif", "image/gif"),
    ("woff", "font/woff"),
    ("woff2", "font/woff2"),
    ("txt", "text/plain; charset=utf-8"),
    ("md", "text/plain; charset=utf-8"),
];

/// The type `file` is served as, or `None` when it is not a page's.
pub fn media_type(file: &Path) -> Option<&'static str> {
    let ext = file.extension()?.to_str()?.to_lowercase();
    PAGE_TYPES
        .iter()
        .find(|(name, _)| *name == ext)
        .map(|(_, media)| *media)
}

/// `root` joined with `rest`, `.` and `..` resolved without touching the disk.
fn resolve(root: &Path, rest: &str) -> PathBuf {
    let mut out = root.to_path_buf();
    for part in Path::new(rest).components() {
        match part {
            Component::ParentDir => {
                out.pop();
            }
            Component::Normal(p) => out.push(p),
            // An absolute `rest` restarts from its root, as `path.resolve` does.
            Component::RootDir | Component::Prefix(_) => out = PathBuf::from(part.as_os_str()),
            Component::CurDir => {}
        }
    }
    out
}

/// Whether `path` is strictly inside `root`.
fn inside(path: &Path, root: &Path) -> bool {
    path != root && path.starts_with(root)
}

/// The file `rest` names under pane `pane_id`'s page directory, or `None`
/// for anything that is not a page file inside it, links included.
pub fn page_file(pack: &InstalledPack, pane_id: &str, rest: &str) -> Option<PathBuf> {
    let pane = pack
        .contributions()?
        .panes()
        .iter()
        .find(|p| p.base.id == pane_id)?;
    let PaneDraws::Web(web) = &pane.draws else {
        return None;
    };
    let root = resolve(&pack.path, web).parent()?.to_path_buf();
    let wanted = if rest.is_empty() || rest.ends_with('/') {
        format!("{rest}index.html")
    } else {
        rest.to_owned()
    };
    let candidate = resolve(&root, &wanted);
    if !inside(&candidate, &root) || !candidate.is_file() || media_type(&candidate).is_none() {
        return None;
    }
    let (real, real_root) = (candidate.canonicalize().ok()?, root.canonicalize().ok()?);
    inside(&real, &real_root).then_some(candidate)
}

/// `%XX` escapes decoded, as the server's router decodes a path; `None`
/// when the result is not UTF-8 or an escape is broken.
pub fn decode_path(raw: &str) -> Option<String> {
    let bytes = raw.as_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] == b'%' {
            let hex = raw.get(i + 1..i + 3)?;
            out.push(u8::from_str_radix(hex, 16).ok()?);
            i += 3;
        } else {
            out.push(bytes[i]);
            i += 1;
        }
    }
    String::from_utf8(out).ok()
}

/// The origins the app's windows are drawn from, which may frame a page:
/// each local port on both loopback names, then what the launcher declared
/// (`VORN_APP_ORIGINS`, comma-separated), keeping only plain origins since
/// they are written straight into a header.
pub fn frame_ancestors(ports: &[u16], declared: Option<&str>) -> Vec<String> {
    static APP_ORIGIN: OnceLock<regress::Regex> = OnceLock::new();
    let pattern =
        APP_ORIGIN.get_or_init(|| js::regex(r"^[a-zA-Z][a-zA-Z0-9+.-]*:(\/\/[^\s;,']+)?$"));
    let mut origins: Vec<String> = Vec::new();
    for port in ports {
        for host in ["127.0.0.1", "localhost"] {
            let origin = format!("http://{host}:{port}");
            if !origins.contains(&origin) {
                origins.push(origin);
            }
        }
    }
    origins.extend(
        declared
            .unwrap_or_default()
            .split(',')
            .map(js::trim)
            .filter(|o| js::test(pattern, o))
            .map(str::to_owned),
    );
    origins
}

/// The headers a page is served with: never cached, never sniffed, and
/// framed only by the app.
pub fn page_headers(media: &'static str, ancestors: &[String]) -> [(&'static str, String); 6] {
    let framed_by = if ancestors.is_empty() {
        "'none'".to_owned()
    } else {
        ancestors.join(" ")
    };
    [
        ("content-type", media.to_owned()),
        ("x-content-type-options", "nosniff".to_owned()),
        ("cache-control", "private, no-store".to_owned()),
        ("referrer-policy", "no-referrer".to_owned()),
        ("cross-origin-opener-policy", "same-origin".to_owned()),
        (
            "content-security-policy",
            format!("default-src 'self'; connect-src 'self'; frame-ancestors {framed_by}"),
        ),
    ]
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::pack::{tests::install, PackStore};
    use serde_json::json;
    use std::fs;

    fn pack() -> (tempfile::TempDir, InstalledPack) {
        let root = tempfile::tempdir().unwrap();
        let dir = install(
            root.path(),
            "x",
            "1.0.0",
            &json!({ "id": "x", "name": "X", "kind": "extension", "contributes": { "panes": [
                { "id": "page", "web": "web/pane/index.html" }, { "id": "prog", "command": ["x"] }
            ] } }),
        );
        let web = dir.join("web/pane");
        fs::create_dir_all(web.join("lib")).unwrap();
        fs::write(web.join("index.html"), "<p>").unwrap();
        fs::write(web.join("lib/app.JS"), "1").unwrap();
        fs::write(web.join("tool.wasm"), "").unwrap();
        fs::write(dir.join("web/secret.html"), "").unwrap();
        let pack = PackStore::new(root.path()).describe("x").unwrap();
        (root, pack)
    }

    #[test]
    fn serves_the_page_and_what_it_is_made_of() {
        let (_root, pack) = pack();
        let index = page_file(&pack, "page", "").unwrap();
        assert!(index.ends_with("web/pane/index.html"));
        assert_eq!(page_file(&pack, "page", "lib/"), None);
        let script = page_file(&pack, "page", "lib/app.JS").unwrap();
        assert_eq!(media_type(&script), Some("text/javascript; charset=utf-8"));
    }

    #[test]
    fn serves_nothing_else() {
        let (_root, pack) = pack();
        for rest in [
            "tool.wasm",
            "../secret.html",
            "../../index.js",
            "/etc/hosts",
            "missing.html",
            "lib",
        ] {
            assert_eq!(page_file(&pack, "page", rest), None, "{rest}");
        }
        assert_eq!(page_file(&pack, "prog", ""), None);
        assert_eq!(page_file(&pack, "none", ""), None);
    }

    #[cfg(unix)]
    #[test]
    fn serves_nothing_through_a_link_out() {
        let (_root, pack) = pack();
        let web = pack.path.join("web/pane");
        std::os::unix::fs::symlink(pack.path.join("web/secret.html"), web.join("out.html"))
            .unwrap();
        assert_eq!(page_file(&pack, "page", "out.html"), None);
    }

    #[test]
    fn decodes_escapes() {
        assert_eq!(decode_path("a%20b/%2e%2E").as_deref(), Some("a b/.."));
        assert_eq!(decode_path("%zz"), None);
        assert_eq!(decode_path("%ff"), None);
    }

    #[test]
    fn names_the_app_origins_that_may_frame_a_page() {
        assert_eq!(
            frame_ancestors(
                &[1, 1],
                Some(" app://vorn , bad origin;x,file:,https://a.b ")
            ),
            [
                "http://127.0.0.1:1",
                "http://localhost:1",
                "app://vorn",
                "file:",
                "https://a.b"
            ]
        );
        let headers = page_headers("text/html; charset=utf-8", &[]);
        assert_eq!(
            headers[5].1,
            "default-src 'self'; connect-src 'self'; frame-ancestors 'none'"
        );
    }
}

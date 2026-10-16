//! The plain HTTP routes of vornd as the server: `/health`, the web client
//! under `/app`, and a task's images under `/api/task-images`.
//!
//! The web client is the authenticated UI and shares an origin with the
//! token it stores, so no page may frame it. A task image is user content: it
//! is never cached by anything shared, never sniffed, and served under a
//! policy that lets it run nothing.

use std::path::{Component, Path, PathBuf};

use hyper::header::{self, HeaderMap, HeaderValue};
use hyper::{Response, StatusCode};
use serde_json::{json, Value};

use crate::endpoint::{full, Body};

/// The prefix the web client is served under.
pub const APP_PREFIX: &str = "/app";
const TASK_IMAGES: &str = "/api/task-images/";

/// Whether `path` is one of these routes.
pub fn answers(path: &str) -> bool {
    path == "/health" || is_app(path) || path.starts_with(TASK_IMAGES)
}

fn is_app(path: &str) -> bool {
    path == APP_PREFIX || path.starts_with("/app/")
}

/// Whether a request's `Accept-Encoding` takes brotli.
pub fn takes_brotli(headers: &HeaderMap) -> bool {
    headers
        .get_all(header::ACCEPT_ENCODING)
        .iter()
        .filter_map(|v| v.to_str().ok())
        .flat_map(|v| v.split(','))
        .any(|e| {
            let mut parts = e.split(';').map(str::trim);
            parts.next() == Some("br") && !parts.any(|q| q.replace(' ', "") == "q=0")
        })
}

/// Answers a GET for `path`; `web` is the web client's build, `data_dir` the
/// server's. `brotli` serves a file's `.br` copy where the build has one.
pub fn answer(path: &str, brotli: bool, web: Option<&Path>, data_dir: &Path) -> Response<Body> {
    if path == "/health" {
        return json_reply(StatusCode::OK, &json!({ "status": "ok" }));
    }
    if let Some(rest) = path.strip_prefix(TASK_IMAGES) {
        return task_image(rest, data_dir);
    }
    match web {
        Some(web) => app(path, brotli, web),
        None => not_found(),
    }
}

/// A file of the web client, or its `index.html` for any route it routes itself.
fn app(path: &str, brotli: bool, web: &Path) -> Response<Body> {
    let rest = path.strip_prefix(APP_PREFIX).unwrap_or("");
    let rest = percent_decode(rest.trim_start_matches('/'));
    let file = rest
        .as_deref()
        .and_then(|r| inside(web, r))
        .filter(|f| f.is_file())
        .unwrap_or_else(|| web.join("index.html"));
    let packed = brotli
        .then(|| {
            let mut name = file.clone().into_os_string();
            name.push(".br");
            PathBuf::from(name)
        })
        .filter(|p| p.is_file());
    let Ok(bytes) = std::fs::read(packed.as_ref().unwrap_or(&file)) else {
        return not_found();
    };
    let mut res = Response::new(full(bytes));
    let headers = res.headers_mut();
    headers.insert(header::CONTENT_TYPE, HeaderValue::from_static(mime(&file)));
    headers.insert(header::VARY, HeaderValue::from_static("accept-encoding"));
    if packed.is_some() {
        headers.insert(header::CONTENT_ENCODING, HeaderValue::from_static("br"));
    }
    headers.insert("x-frame-options", HeaderValue::from_static("DENY"));
    headers.insert(
        header::CONTENT_SECURITY_POLICY,
        HeaderValue::from_static("frame-ancestors 'none'"),
    );
    res
}

/// `rest` under `root`, refused if it climbs out or names a root of its own.
fn inside(root: &Path, rest: &str) -> Option<PathBuf> {
    if rest.is_empty() {
        return None;
    }
    let rel = Path::new(rest);
    rel.components()
        .all(|c| matches!(c, Component::Normal(_)))
        .then(|| root.join(rel))
}

fn percent_decode(s: &str) -> Option<String> {
    url::form_urlencoded::parse(format!("p={}", s.replace('+', "%2B")).as_bytes())
        .next()
        .map(|(_, v)| v.into_owned())
}

/// A task's image, by `taskId/filename`.
fn task_image(rest: &str, data_dir: &Path) -> Response<Body> {
    let mut parts = rest.splitn(2, '/');
    let (Some(task), Some(name)) = (parts.next(), parts.next()) else {
        return json_reply(StatusCode::NOT_FOUND, &json!({ "error": "Not found" }));
    };
    let (Some(task), Some(name)) = (percent_decode(task), percent_decode(name)) else {
        return invalid();
    };
    let images = vorn_tasks::images::TaskImages::new(data_dir);
    let Ok(file) = images.path(&task, &name) else {
        return invalid();
    };
    let file = PathBuf::from(file);
    let Ok(bytes) = std::fs::read(&file) else {
        return json_reply(
            StatusCode::NOT_FOUND,
            &json!({ "error": "Image not found" }),
        );
    };
    let mut res = Response::new(full(bytes));
    let headers = res.headers_mut();
    headers.insert(
        header::CONTENT_TYPE,
        HeaderValue::from_static(image_mime(&file)),
    );
    headers.insert(
        header::CACHE_CONTROL,
        HeaderValue::from_static("private, max-age=86400"),
    );
    headers.insert(
        header::X_CONTENT_TYPE_OPTIONS,
        HeaderValue::from_static("nosniff"),
    );
    headers.insert(
        header::CONTENT_SECURITY_POLICY,
        HeaderValue::from_static("default-src 'none'; sandbox"),
    );
    headers.insert(
        "cross-origin-resource-policy",
        HeaderValue::from_static("same-origin"),
    );
    res
}

fn extension(file: &Path) -> String {
    file.extension()
        .and_then(|e| e.to_str())
        .unwrap_or("")
        .to_ascii_lowercase()
}

/// An image served as what it is; anything else as an opaque download.
fn image_mime(file: &Path) -> &'static str {
    match extension(file).as_str() {
        "png" => "image/png",
        "jpg" | "jpeg" => "image/jpeg",
        "gif" => "image/gif",
        "webp" => "image/webp",
        "bmp" => "image/bmp",
        _ => "application/octet-stream",
    }
}

fn mime(file: &Path) -> &'static str {
    match extension(file).as_str() {
        "html" => "text/html; charset=utf-8",
        "js" | "mjs" => "text/javascript; charset=utf-8",
        "css" => "text/css; charset=utf-8",
        "json" | "map" => "application/json; charset=utf-8",
        "webmanifest" => "application/manifest+json",
        "svg" => "image/svg+xml",
        "png" => "image/png",
        "jpg" | "jpeg" => "image/jpeg",
        "gif" => "image/gif",
        "webp" => "image/webp",
        "ico" => "image/x-icon",
        "woff" => "font/woff",
        "woff2" => "font/woff2",
        "ttf" => "font/ttf",
        "wasm" => "application/wasm",
        "txt" => "text/plain; charset=utf-8",
        _ => "application/octet-stream",
    }
}

fn invalid() -> Response<Body> {
    json_reply(
        StatusCode::BAD_REQUEST,
        &json!({ "error": "Invalid request" }),
    )
}

/// What a route nobody serves answers.
pub fn not_found() -> Response<Body> {
    json_reply(StatusCode::NOT_FOUND, &json!({ "error": "Not found" }))
}

pub fn json_reply(status: StatusCode, body: &Value) -> Response<Body> {
    let mut res = Response::new(full(body.to_string()));
    *res.status_mut() = status;
    res.headers_mut().insert(
        header::CONTENT_TYPE,
        HeaderValue::from_static("application/json; charset=utf-8"),
    );
    res
}

#[cfg(test)]
mod tests {
    use http_body_util::BodyExt;

    use super::*;

    async fn body(res: Response<Body>) -> String {
        let bytes = res.into_body().collect().await.unwrap().to_bytes();
        String::from_utf8(bytes.to_vec()).unwrap()
    }

    #[tokio::test]
    async fn says_it_is_up() {
        let res = answer("/health", false, None, Path::new("/nowhere"));
        assert_eq!(res.status(), StatusCode::OK);
        assert_eq!(body(res).await, r#"{"status":"ok"}"#);
        assert!(answers("/health") && answers("/app") && answers("/app/x") && !answers("/apps"));
    }

    #[tokio::test]
    async fn serves_the_web_client_and_its_routes_unframed() {
        let web = tempfile::tempdir().unwrap();
        std::fs::write(web.path().join("index.html"), "<html>").unwrap();
        std::fs::create_dir(web.path().join("assets")).unwrap();
        std::fs::write(web.path().join("assets/app.js"), "x()").unwrap();
        let data = tempfile::tempdir().unwrap();
        let res = answer("/app/assets/app.js", false, Some(web.path()), data.path());
        assert_eq!(
            res.headers()[header::CONTENT_TYPE],
            "text/javascript; charset=utf-8"
        );
        assert_eq!(res.headers()["x-frame-options"], "DENY");
        assert_eq!(body(res).await, "x()");
        for route in [
            "/app",
            "/app/",
            "/app/tasks/1",
            "/app/../secret",
            "/app/%2e%2e/x",
        ] {
            let res = answer(route, false, Some(web.path()), data.path());
            assert_eq!(body(res).await, "<html>", "{route}");
        }
        assert_eq!(
            answer("/app", false, None, data.path()).status(),
            StatusCode::NOT_FOUND
        );
    }

    #[tokio::test]
    async fn serves_the_brotli_copy_to_a_browser_that_takes_it() {
        let web = tempfile::tempdir().unwrap();
        std::fs::write(web.path().join("index.html"), "<html>").unwrap();
        std::fs::write(web.path().join("app.wasm"), "raw").unwrap();
        std::fs::write(web.path().join("app.wasm.br"), "packed").unwrap();
        let data = tempfile::tempdir().unwrap();
        let res = answer("/app/app.wasm", true, Some(web.path()), data.path());
        assert_eq!(res.headers()[header::CONTENT_TYPE], "application/wasm");
        assert_eq!(res.headers()[header::CONTENT_ENCODING], "br");
        assert_eq!(body(res).await, "packed");
        let res = answer("/app/app.wasm", false, Some(web.path()), data.path());
        assert!(res.headers().get(header::CONTENT_ENCODING).is_none());
        assert_eq!(body(res).await, "raw");
        let res = answer("/app/", true, Some(web.path()), data.path());
        assert_eq!(body(res).await, "<html>");

        let takes = |v: &str| {
            let mut h = HeaderMap::new();
            h.insert(header::ACCEPT_ENCODING, HeaderValue::from_str(v).unwrap());
            takes_brotli(&h)
        };
        assert!(takes("gzip, deflate, br, zstd") && takes("br;q=0.5"));
        assert!(!takes("gzip") && !takes("br;q=0") && !takes("brotli"));
        assert!(!takes_brotli(&HeaderMap::new()));
    }

    #[tokio::test]
    async fn serves_a_tasks_images_as_inert_content() {
        let data = tempfile::tempdir().unwrap();
        let images = vorn_tasks::images::TaskImages::new(data.path());
        let src = data.path().join("shot.png");
        std::fs::write(&src, b"png").unwrap();
        let name = images.save("t1", src.to_str().unwrap()).unwrap();
        let name = Path::new(&name)
            .file_name()
            .unwrap()
            .to_str()
            .unwrap()
            .to_owned();
        let res = answer(
            &format!("/api/task-images/t1/{name}"),
            false,
            None,
            data.path(),
        );
        assert_eq!(res.status(), StatusCode::OK);
        assert_eq!(res.headers()[header::CONTENT_TYPE], "image/png");
        assert_eq!(
            res.headers()[header::CONTENT_SECURITY_POLICY],
            "default-src 'none'; sandbox"
        );
        assert_eq!(body(res).await, "png");
        let missing = answer("/api/task-images/t1/none.png", false, None, data.path());
        assert_eq!(missing.status(), StatusCode::NOT_FOUND);
        let climbing = answer("/api/task-images/..%2F/x.png", false, None, data.path());
        assert_eq!(climbing.status(), StatusCode::BAD_REQUEST);
    }
}

//! The page's own services, which stand in for the OS's in a browser: its
//! address, storage, tabs, notifications, file pickers and downloads.

use std::cell::RefCell;
use std::path::PathBuf;

use wasm_bindgen::prelude::*;
use wasm_bindgen_futures::JsFuture;
use web_time::Duration;

fn window() -> web_sys::Window {
    web_sys::window().expect("a page has a window")
}

/// Where the page was served from.
pub struct Location {
    /// `host[:port]`.
    pub host: String,
    pub port: u16,
    /// Served over https, so sockets go over wss.
    pub secure: bool,
}

pub fn location() -> Location {
    let loc = window().location();
    let secure = loc.protocol().is_ok_and(|p| p == "https:");
    let port = loc
        .port()
        .ok()
        .and_then(|p| p.parse().ok())
        .unwrap_or(if secure { 443 } else { 80 });
    Location {
        host: loc.host().unwrap_or_default(),
        port,
        secure,
    }
}

/// The page's address, for links back to it (the phone's pairing code).
pub fn href() -> String {
    window().location().href().unwrap_or_default()
}

fn storage() -> Option<web_sys::Storage> {
    window().local_storage().ok().flatten()
}

/// What `key` holds in the page's storage.
pub fn load(key: &str) -> Option<String> {
    storage()?.get_item(key).ok().flatten()
}

/// Keeps `value` under `key`; a full or blocked storage just forgets.
pub fn save(key: &str, value: &str) {
    if let Some(s) = storage() {
        let _ = s.set_item(key, value);
    }
}

pub fn forget(key: &str) {
    if let Some(s) = storage() {
        let _ = s.remove_item(key);
    }
}

/// The page's stand-in for a file under the data directory: its name.
pub fn key_for(path: &std::path::Path) -> String {
    let name = path.file_name().map_or_else(
        || path.to_string_lossy().into_owned(),
        |n| n.to_string_lossy().into_owned(),
    );
    format!("vorn.file.{name}")
}

/// Opens `url` in a new tab.
pub fn open_tab(url: &str) {
    let _ = window().open_with_url_and_target_and_features(url, "_blank", "noopener");
}

/// Shows a notification when the person allowed them, asking once.
pub fn notify(title: &str, body: &str) {
    use web_sys::{Notification, NotificationOptions, NotificationPermission};
    let show = {
        let (title, body) = (title.to_owned(), body.to_owned());
        move || {
            let opts = NotificationOptions::new();
            opts.set_body(&body);
            let _ = Notification::new_with_options(&title, &opts);
        }
    };
    match Notification::permission() {
        NotificationPermission::Granted => show(),
        NotificationPermission::Default => {
            let Ok(asked) = Notification::request_permission() else {
                return;
            };
            wasm_bindgen_futures::spawn_local(async move {
                if JsFuture::from(asked)
                    .await
                    .ok()
                    .and_then(|v| v.as_string())
                    .is_some_and(|p| p == "granted")
                {
                    show();
                }
            });
        }
        _ => {}
    }
}

thread_local! {
    /// The files the person picked, read, by the name [`pick_file`] answered with.
    static PICKED: RefCell<Vec<(PathBuf, Vec<u8>)>> = const { RefCell::new(Vec::new()) };
}

/// Asks the page for one file, as the OS picker does: the answer is the
/// file's name once it is read, and [`picked`] hands over its contents.
pub fn pick_file(
    accept: &str,
    wake: crate::client::rpc::Wake,
) -> std::sync::mpsc::Receiver<Option<PathBuf>> {
    let (tx, rx) = std::sync::mpsc::channel();
    let Some(input) = window()
        .document()
        .and_then(|d| d.create_element("input").ok())
        .and_then(|e| e.dyn_into::<web_sys::HtmlInputElement>().ok())
    else {
        let _ = tx.send(None);
        return rx;
    };
    input.set_type("file");
    input.set_accept(accept);
    let el = input.clone();
    let on_change = Closure::once(move || {
        let file = el.files().and_then(|f| f.get(0));
        wasm_bindgen_futures::spawn_local(async move {
            let mut path = None;
            if let Some(f) = file {
                if let Ok(buf) = JsFuture::from(f.array_buffer()).await {
                    let at = PathBuf::from(f.name());
                    let bytes = js_sys::Uint8Array::new(&buf).to_vec();
                    PICKED.with(|p| p.borrow_mut().push((at.clone(), bytes)));
                    path = Some(at);
                }
            }
            let _ = tx.send(path);
            wake();
        });
    });
    input.set_onchange(Some(on_change.as_ref().unchecked_ref()));
    on_change.forget();
    input.click();
    rx
}

/// The contents of a file [`pick_file`] answered with, handed over once.
pub fn picked(path: &std::path::Path) -> Option<Vec<u8>> {
    PICKED.with(|p| {
        let mut p = p.borrow_mut();
        let i = p.iter().position(|(at, _)| at == path)?;
        Some(p.remove(i).1)
    })
}

/// Saves `bytes` as a download named `name`.
pub fn download(name: &str, kind: &str, bytes: &[u8]) {
    let parts = js_sys::Array::of1(&js_sys::Uint8Array::from(bytes));
    let opts = web_sys::BlobPropertyBag::new();
    opts.set_type(kind);
    let Ok(blob) = web_sys::Blob::new_with_u8_array_sequence_and_options(&parts, &opts) else {
        return;
    };
    let Ok(url) = web_sys::Url::create_object_url_with_blob(&blob) else {
        return;
    };
    if let Some(a) = window()
        .document()
        .and_then(|d| d.create_element("a").ok())
        .and_then(|e| e.dyn_into::<web_sys::HtmlAnchorElement>().ok())
    {
        a.set_href(&url);
        a.set_download(name);
        a.click();
    }
    let _ = web_sys::Url::revoke_object_url(&url);
}

/// Resolves after `d`, without holding the page.
pub async fn sleep(d: Duration) {
    let ms = d.as_millis().min(i32::MAX as u128) as i32;
    let promise = js_sys::Promise::new(&mut |resolve, _| {
        let _ = window().set_timeout_with_callback_and_timeout_and_arguments_0(&resolve, ms);
    });
    let _ = JsFuture::from(promise).await;
}

/// A GET of `path` from where the page was served, as bytes.
pub async fn get_bytes(path: &str) -> Option<Vec<u8>> {
    let res: web_sys::Response = JsFuture::from(window().fetch_with_str(path))
        .await
        .ok()?
        .dyn_into()
        .ok()?;
    if !res.ok() {
        return None;
    }
    let buf = JsFuture::from(res.array_buffer().ok()?).await.ok()?;
    Some(js_sys::Uint8Array::new(&buf).to_vec())
}

/// A GET of `path` from the page's vornd, as JSON.
pub async fn get_json(path: &str) -> Option<serde_json::Value> {
    let res: web_sys::Response = JsFuture::from(window().fetch_with_str(path))
        .await
        .ok()?
        .dyn_into()
        .ok()?;
    if !res.ok() {
        return None;
    }
    let text = JsFuture::from(res.text().ok()?).await.ok()?.as_string()?;
    serde_json::from_str(&text).ok()
}

/// POSTs `body` as JSON to `path` on the page's vornd; the JSON answer, or
/// vornd's error message.
pub async fn post_json(path: &str, body: &serde_json::Value) -> Result<serde_json::Value, String> {
    let init = web_sys::RequestInit::new();
    init.set_method("POST");
    init.set_body(&JsValue::from_str(&body.to_string()));
    let headers = web_sys::Headers::new().map_err(js_error)?;
    headers
        .set("content-type", "application/json")
        .map_err(js_error)?;
    init.set_headers(&headers);
    let req = web_sys::Request::new_with_str_and_init(path, &init).map_err(js_error)?;
    let res: web_sys::Response = JsFuture::from(window().fetch_with_request(&req))
        .await
        .map_err(|_| "Could not reach vornd.".to_owned())?
        .dyn_into()
        .map_err(js_error)?;
    let text = JsFuture::from(res.text().map_err(js_error)?)
        .await
        .map_err(js_error)?
        .as_string()
        .unwrap_or_default();
    let value: serde_json::Value = serde_json::from_str(&text).unwrap_or_default();
    if res.ok() {
        return Ok(value);
    }
    Err(value
        .get("error")
        .and_then(|e| e.as_str().or_else(|| e.get("message")?.as_str()))
        .map_or_else(|| format!("vornd answered {}", res.status()), str::to_owned))
}

fn js_error(e: JsValue) -> String {
    e.as_string().unwrap_or_else(|| format!("{e:?}"))
}

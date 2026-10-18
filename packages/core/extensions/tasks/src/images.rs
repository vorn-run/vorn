//! A task's images, kept in the extension's own files as `<task id>/<name>`,
//! each named afresh so nothing a client sends becomes a path. Only bitmap
//! types are taken: an SVG is a document whose script would run on the
//! app's origin.
//!
//! Vorn keeps Tasks' files where the board always kept its images
//! (`task-images/` beside `vorn.db`), so a path an agent was handed still
//! reads. Where one is, is Vorn's to say: `task:imageGetPath` answers the
//! file's name, `{file}`, and vornd answers the client with its path.

use serde_json::{json, Value};
use vorn_extension_sdk::Host;

/// The decoded size an uploaded image may have.
const MAX_IMAGE_BYTES: usize = 10 * 1024 * 1024;
const ALLOWED: [&str; 6] = [".png", ".jpg", ".jpeg", ".gif", ".webp", ".bmp"];

fn safe_id(value: &str) -> bool {
    !value.is_empty()
        && value
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || b == b'-' || b == b'_')
}

fn safe_filename(value: &str) -> bool {
    !value.is_empty()
        && !value.starts_with('.')
        && value
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || matches!(b, b'-' | b'_' | b'.'))
}

/// `path.extname`, lower-cased: from the last dot of the last segment.
fn extension(name: &str) -> String {
    let base = name.rsplit(['/', '\\']).next().unwrap_or(name);
    match base.rfind('.') {
        Some(0) | None => String::new(),
        Some(at) => base[at..].to_lowercase(),
    }
}

fn task_dir(task: &str) -> Result<&str, String> {
    if safe_id(task) {
        Ok(task)
    } else {
        Err(format!("Invalid taskId: {task}"))
    }
}

/// The file name of image `name` of `task`.
pub fn file(task: &str, name: &str) -> Result<String, String> {
    let task = task_dir(task)?;
    if !safe_filename(name) {
        return Err(format!("Invalid filename: {name}"));
    }
    Ok(format!("{task}/{name}"))
}

fn fresh_name(ext: &str) -> String {
    format!("{}{ext}", crate::new_id())
}

fn text(params: &Value, key: &str) -> String {
    match params.get(key) {
        Some(Value::String(s)) => s.clone(),
        None | Some(Value::Null) => String::new(),
        Some(other) => crate::js::to_string(other),
    }
}

/// Answers a `task:image*` call: a name, `{file}`, or null.
pub fn answer(host: &mut dyn Host, method: &str, params: &Value) -> Result<Value, String> {
    let task = text(params, "taskId");
    match method {
        "task:imageSave" => save(host, &task, &text(params, "sourcePath")).map(Value::String),
        "task:imageUpload" => upload(
            host,
            &task,
            &text(params, "base64"),
            &text(params, "filename"),
        )
        .map(Value::String),
        "task:imageDelete" => {
            let file = file(&task, &text(params, "filename"))?;
            host.delete_file(&file).map_err(|e| e.message)?;
            Ok(Value::Null)
        }
        "task:imageGetPath" => Ok(json!({ "file": file(&task, &text(params, "filename"))? })),
        "task:imageCleanup" => {
            let task = params
                .as_str()
                .map_or_else(|| crate::js::to_string(params), str::to_owned);
            cleanup(host, &task).map(|()| Value::Null)
        }
        other => Err(format!("Tasks has no {other}")),
    }
}

/// `task:imageSave`: copies a file on this machine in; the name it got.
fn save(host: &mut dyn Host, task: &str, source: &str) -> Result<String, String> {
    let task = task_dir(task)?;
    let ext = extension(source);
    if !ALLOWED.contains(&ext.as_str()) {
        return Err(format!("Unsupported image type: {ext}"));
    }
    let read = host
        .call(
            "files.read",
            json!({ "path": source, "encoding": "base64" }),
        )
        .map_err(|e| e.message)?;
    let bytes = decode_base64(
        read.get("base64")
            .and_then(Value::as_str)
            .unwrap_or_default(),
    );
    let name = fresh_name(&ext);
    host.put_file(&format!("{task}/{name}"), &bytes)
        .map_err(|e| e.message)?;
    Ok(name)
}

/// `task:imageUpload`: an image sent as base64; the name it got.
fn upload(host: &mut dyn Host, task: &str, base64: &str, original: &str) -> Result<String, String> {
    let task = task_dir(task)?;
    let ext = match extension(original) {
        e if e.is_empty() => ".png".to_owned(),
        e => e,
    };
    if !ALLOWED.contains(&ext.as_str()) {
        return Err(format!("Unsupported image type: {ext}"));
    }
    let estimated = (base64.len() as f64 * 0.75).ceil() as usize;
    if estimated > MAX_IMAGE_BYTES {
        return Err(format!(
            "Image too large ({:.1}MB). Max: 10MB",
            estimated as f64 / 1024.0 / 1024.0
        ));
    }
    let name = fresh_name(&ext);
    host.put_file(&format!("{task}/{name}"), &decode_base64(base64))
        .map_err(|e| e.message)?;
    Ok(name)
}

/// `task:imageCleanup`: every image of a task.
fn cleanup(host: &mut dyn Host, task: &str) -> Result<(), String> {
    let task = task_dir(task)?;
    for blob in host.files(&format!("{task}/")).map_err(|e| e.message)? {
        host.delete_file(&blob.name).map_err(|e| e.message)?;
    }
    Ok(())
}

/// Base64, lenient about padding, whitespace and the URL-safe alphabet; it
/// stops at the first character it cannot read.
fn decode_base64(text: &str) -> Vec<u8> {
    let value = |c: u8| match c {
        b'A'..=b'Z' => Some(c - b'A'),
        b'a'..=b'z' => Some(c - b'a' + 26),
        b'0'..=b'9' => Some(c - b'0' + 52),
        b'+' | b'-' => Some(62),
        b'/' | b'_' => Some(63),
        _ => None,
    };
    let mut out = Vec::with_capacity(text.len() * 3 / 4);
    let (mut acc, mut bits) = (0u32, 0u32);
    for c in text.bytes() {
        if c.is_ascii_whitespace() {
            continue;
        }
        let Some(v) = value(c) else { break };
        acc = (acc << 6) | u32::from(v);
        bits += 6;
        if bits >= 8 {
            bits -= 8;
            out.push((acc >> bits) as u8);
            acc &= (1 << bits) - 1;
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use vorn_extension_sdk::testing::FakeHost;

    use super::*;

    fn ask(host: &mut FakeHost, method: &str, params: Value) -> Result<Value, String> {
        answer(host, method, &params)
    }

    #[test]
    fn keeps_a_tasks_images_under_names_it_chose() {
        let mut host = FakeHost::default().answer("files.read", |a| {
            assert_eq!(a["encoding"], "base64");
            Ok(json!({ "base64": "anBlZw==" }))
        });
        let name = ask(
            &mut host,
            "task:imageUpload",
            json!({ "taskId": "t-1", "base64": "aGVsbG8=", "filename": "shot.PNG" }),
        )
        .unwrap();
        let name = name.as_str().unwrap().to_owned();
        assert!(name.ends_with(".png") && name.len() == 40, "{name}");
        assert_eq!(host.files[&format!("t-1/{name}")], b"hello");
        assert_eq!(
            ask(
                &mut host,
                "task:imageGetPath",
                json!({ "taskId": "t-1", "filename": name })
            )
            .unwrap(),
            json!({ "file": format!("t-1/{name}") })
        );
        let copied = ask(
            &mut host,
            "task:imageSave",
            json!({ "taskId": "t-1", "sourcePath": "/x/pic.jpg" }),
        )
        .unwrap();
        let copied = copied.as_str().unwrap().to_owned();
        assert!(copied.ends_with(".jpg"));
        assert_eq!(host.files[&format!("t-1/{copied}")], b"jpeg");
        let delete = json!({ "taskId": "t-1", "filename": copied });
        assert_eq!(
            ask(&mut host, "task:imageDelete", delete.clone()),
            Ok(Value::Null)
        );
        assert_eq!(ask(&mut host, "task:imageDelete", delete), Ok(Value::Null));
        ask(&mut host, "task:imageCleanup", json!("t-1")).unwrap();
        assert!(host.files.is_empty());
        ask(&mut host, "task:imageCleanup", json!("t-1")).unwrap();
    }

    #[test]
    fn refuses_names_and_types_that_could_escape_or_run() {
        let mut host = FakeHost::default();
        let upload = |task: &str, b64: &str, name: &str| json!({ "taskId": task, "base64": b64, "filename": name });
        assert_eq!(
            ask(&mut host, "task:imageUpload", upload("../x", "", "a.png")).unwrap_err(),
            "Invalid taskId: ../x"
        );
        assert_eq!(
            ask(&mut host, "task:imageUpload", upload("t", "", "a.svg")).unwrap_err(),
            "Unsupported image type: .svg"
        );
        assert_eq!(file("t", "../etc").unwrap_err(), "Invalid filename: ../etc");
        assert_eq!(
            file("t", ".hidden").unwrap_err(),
            "Invalid filename: .hidden"
        );
        let big = "A".repeat(14 * 1024 * 1024);
        assert!(
            ask(&mut host, "task:imageUpload", upload("t", &big, "a.png"))
                .unwrap_err()
                .starts_with("Image too large (10.5MB)")
        );
        let named = ask(&mut host, "task:imageUpload", upload("t", "aGk", "noext")).unwrap();
        assert!(named.as_str().unwrap().ends_with(".png"));
        // A file that is not an image is never read.
        assert_eq!(
            ask(
                &mut host,
                "task:imageSave",
                json!({ "taskId": "t", "sourcePath": "/etc/passwd" })
            )
            .unwrap_err(),
            "Unsupported image type: "
        );
        assert!(host.asked("files.read").is_empty());
    }

    #[test]
    fn decodes_base64_leniently() {
        assert_eq!(decode_base64("aGVsbG8="), b"hello");
        assert_eq!(decode_base64("aGVs bG8"), b"hello");
        assert_eq!(decode_base64("_-8"), [0xff, 0xef]);
        assert_eq!(decode_base64("aGk*garbage"), b"hi");
    }
}

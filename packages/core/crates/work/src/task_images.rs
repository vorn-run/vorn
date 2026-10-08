//! A task's images, kept under `<data dir>/task-images/<task id>/`, each
//! named afresh so nothing a client sends becomes a path. Only bitmap types
//! are taken: an SVG is a document whose script would run on the app's origin.

use std::fs;
use std::path::{Path, PathBuf};

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

/// The images of every task, under the data directory.
#[derive(Debug, Clone)]
pub struct TaskImages {
    dir: PathBuf,
}

impl TaskImages {
    pub fn new(data_dir: &Path) -> TaskImages {
        TaskImages {
            dir: data_dir.join("task-images"),
        }
    }

    fn task_dir(&self, task: &str) -> Result<PathBuf, String> {
        if !safe_id(task) {
            return Err(format!("Invalid taskId: {task}"));
        }
        Ok(self.dir.join(task))
    }

    fn file(&self, task: &str, name: &str) -> Result<PathBuf, String> {
        let dir = self.task_dir(task)?;
        if !safe_filename(name) {
            return Err(format!("Invalid filename: {name}"));
        }
        Ok(dir.join(name))
    }

    fn fresh_name(ext: &str) -> String {
        format!("{}{ext}", uuid_v4())
    }

    /// `task:imageSave`: copies a file on this machine in; the name it got.
    pub fn save(&self, task: &str, source: &str) -> Result<String, String> {
        let dir = self.task_dir(task)?;
        let ext = extension(source);
        if !ALLOWED.contains(&ext.as_str()) {
            return Err(format!("Unsupported image type: {ext}"));
        }
        fs::create_dir_all(&dir).map_err(|e| e.to_string())?;
        let name = Self::fresh_name(&ext);
        fs::copy(source, dir.join(&name)).map_err(|e| e.to_string())?;
        Ok(name)
    }

    /// `task:imageUpload`: an image sent as base64; the name it got.
    pub fn upload(&self, task: &str, base64: &str, original: &str) -> Result<String, String> {
        let dir = self.task_dir(task)?;
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
        let bytes = decode_base64(base64);
        fs::create_dir_all(&dir).map_err(|e| e.to_string())?;
        let name = Self::fresh_name(&ext);
        fs::write(dir.join(&name), bytes).map_err(|e| e.to_string())?;
        Ok(name)
    }

    /// `task:imageDelete`; an image already gone is not an error.
    pub fn delete(&self, task: &str, name: &str) -> Result<(), String> {
        let path = self.file(task, name)?;
        match fs::remove_file(path) {
            Err(e) if e.kind() != std::io::ErrorKind::NotFound => Err(e.to_string()),
            _ => Ok(()),
        }
    }

    /// `task:imageGetPath`.
    pub fn path(&self, task: &str, name: &str) -> Result<String, String> {
        Ok(self.file(task, name)?.to_string_lossy().into_owned())
    }

    /// `task:imageCleanup`: every image of a task.
    pub fn cleanup(&self, task: &str) -> Result<(), String> {
        let dir = self.task_dir(task)?;
        match fs::remove_dir_all(dir) {
            Err(e) if e.kind() != std::io::ErrorKind::NotFound => Err(e.to_string()),
            _ => Ok(()),
        }
    }
}

/// `Buffer.from(text, 'base64')`: lenient, as Node's is, about padding,
/// whitespace and the URL-safe alphabet; it stops at the first character it
/// cannot read.
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

/// A random version-4 UUID, as `randomUUID` makes.
fn uuid_v4() -> String {
    uuid::Uuid::new_v4().to_string()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn keeps_a_task_s_images_under_names_it_chose() {
        let dir = tempfile::tempdir().unwrap();
        let images = TaskImages::new(dir.path());
        let name = images.upload("t-1", "aGVsbG8=", "shot.PNG").unwrap();
        assert!(name.ends_with(".png") && name.len() == 40, "{name}");
        let path = images.path("t-1", &name).unwrap();
        assert_eq!(std::fs::read(&path).unwrap(), b"hello");

        let src = dir.path().join("pic.jpg");
        std::fs::write(&src, b"jpeg").unwrap();
        let copied = images.save("t-1", src.to_str().unwrap()).unwrap();
        assert!(copied.ends_with(".jpg"));
        images.delete("t-1", &copied).unwrap();
        images.delete("t-1", &copied).unwrap();
        images.cleanup("t-1").unwrap();
        assert!(!dir.path().join("task-images/t-1").exists());
        images.cleanup("t-1").unwrap();
    }

    #[test]
    fn refuses_names_and_types_that_could_escape_or_run() {
        let dir = tempfile::tempdir().unwrap();
        let images = TaskImages::new(dir.path());
        assert_eq!(
            images.upload("../x", "", "a.png").unwrap_err(),
            "Invalid taskId: ../x"
        );
        assert_eq!(
            images.upload("t", "", "a.svg").unwrap_err(),
            "Unsupported image type: .svg"
        );
        assert_eq!(
            images.path("t", "../etc").unwrap_err(),
            "Invalid filename: ../etc"
        );
        assert_eq!(
            images.path("t", ".hidden").unwrap_err(),
            "Invalid filename: .hidden"
        );
        let big = "A".repeat(14 * 1024 * 1024);
        assert!(images
            .upload("t", &big, "a.png")
            .unwrap_err()
            .starts_with("Image too large (10.5MB)"));
        assert_eq!(
            images
                .upload("t", "aGk", "noext")
                .map(|n| n.ends_with(".png")),
            Ok(true)
        );
    }

    #[test]
    fn decodes_base64_as_node_does() {
        assert_eq!(decode_base64("aGVsbG8="), b"hello");
        assert_eq!(decode_base64("aGVs bG8"), b"hello");
        assert_eq!(decode_base64("_-8"), [0xff, 0xef]);
        assert_eq!(decode_base64("aGk*garbage"), b"hi");
    }
}

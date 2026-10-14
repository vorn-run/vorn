//! The terminal's selection and clipboard: a drag or double-click selects,
//! the copy key copies it, the paste keys paste, and a paste vornd flags
//! asks first. A clipboard holding only an image sends Ctrl-V, which agent
//! CLIs answer by reading the image themselves; dropped files paste their
//! quoted paths, and a dropped image is saved first.

use std::path::Path;

use vorn_term_proto::msg::GridPoint;

use super::{App, Src};
use crate::act::Act;
use crate::grid::Reply;
use crate::screen::term_id;
use crate::select::Sel;
use crate::state::Dialog;
use crate::ui::{mods, Dropped, Event as UiEvent, Id, Ui};

/// What a key does with a focused terminal's clipboard.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Clip {
    Copy,
    Paste,
    SelectAll,
}

/// The renderer's clipboard keys: Cmd-C, Cmd-V and Cmd-A on a Mac;
/// elsewhere Ctrl-Shift-C and Ctrl-Shift-V, and Ctrl-C only over a
/// selection, else it interrupts. Ctrl-V is the program's everywhere (agent
/// CLIs paste an image on it). Shift-Insert pastes everywhere.
pub fn clipboard_key(code: &str, m: u16, selection: bool, mac: bool) -> Option<Clip> {
    let held = m & (mods::SHIFT | mods::CTRL | mods::ALT | mods::SUPER);
    if code == "Insert" && held == mods::SHIFT {
        return Some(Clip::Paste);
    }
    if mac {
        return match code {
            "KeyC" if held == mods::SUPER => Some(Clip::Copy),
            "KeyV" if held == mods::SUPER => Some(Clip::Paste),
            "KeyA" if held == mods::SUPER => Some(Clip::SelectAll),
            _ => None,
        };
    }
    let ctrl = held == mods::CTRL;
    let ctrl_shift = held == mods::CTRL | mods::SHIFT;
    match code {
        "KeyC" if ctrl_shift || ctrl && selection => Some(Clip::Copy),
        "KeyV" if ctrl_shift => Some(Clip::Paste),
        _ => None,
    }
}

/// The task vornd keeps a browser's pasted and dropped images under.
const PASTE_TASK: &str = "terminal-paste";

/// A browser's image on its way through vornd: saved under a name, then
/// asked where that is.
#[derive(Debug)]
#[cfg_attr(not(target_arch = "wasm32"), allow(dead_code))]
pub(super) enum TermImage {
    Saved(String),
    Path(String),
}

/// Characters a shell reads as themselves in a word.
fn plain(c: char, windows: bool) -> bool {
    c.is_ascii_alphanumeric() || "_@%+=:,./-".contains(c) || windows && c == '\\'
}

/// `path` as one shell word: as it is when nothing in it is special, else
/// single-quoted (double-quoted for a Windows path).
pub fn shell_quoted(path: &str) -> String {
    let b = path.as_bytes();
    let windows = path.starts_with("\\\\")
        || b.len() > 2 && b[0].is_ascii_alphabetic() && b[1] == b':' && b[2] == b'\\';
    if !path.is_empty() && path.chars().all(|c| plain(c, windows)) {
        path.to_owned()
    } else if windows {
        format!("\"{path}\"")
    } else {
        format!("'{}'", path.replace('\'', "'\\''"))
    }
}

/// Paths as a shell reads them back, each quoted, space separated and
/// followed by a space, as a terminal types a drop.
pub fn quoted_paths<P: AsRef<Path>>(paths: &[P]) -> String {
    paths
        .iter()
        .map(|p| shell_quoted(&p.as_ref().to_string_lossy()) + " ")
        .collect()
}

/// Writes `bytes` to a new file in `dir`, named for the time and with
/// `name`'s image extension (PNG when it has none).
#[cfg(not(target_arch = "wasm32"))]
pub fn save_image(dir: &Path, name: &str, bytes: &[u8]) -> std::io::Result<std::path::PathBuf> {
    use std::io::Write;
    own_dir(dir)?;
    let ext = Path::new(name)
        .extension()
        .map(|e| e.to_string_lossy().to_ascii_lowercase())
        .filter(|e| ["png", "jpg", "jpeg", "gif", "webp", "bmp", "tiff"].contains(&e.as_str()))
        .unwrap_or_else(|| "png".into());
    let stamp = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |d| d.as_millis());
    for n in 0u32.. {
        let file = match n {
            0 => format!("image-{stamp}.{ext}"),
            n => format!("image-{stamp}-{n}.{ext}"),
        };
        let path = dir.join(file);
        match std::fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&path)
        {
            Ok(mut f) => return f.write_all(bytes).map(|()| path),
            Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists => {}
            Err(e) => return Err(e),
        }
    }
    unreachable!("a free name exists below u32::MAX")
}

/// Makes `dir` private to this user, refusing a link or someone else's folder.
#[cfg(not(target_arch = "wasm32"))]
fn own_dir(dir: &Path) -> std::io::Result<()> {
    #[cfg(unix)]
    {
        use std::os::unix::fs::{DirBuilderExt, MetadataExt, PermissionsExt};
        match std::fs::DirBuilder::new()
            .recursive(true)
            .mode(0o700)
            .create(dir)
        {
            Ok(()) => {}
            Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists => {}
            Err(e) => return Err(e),
        }
        let meta = std::fs::symlink_metadata(dir)?;
        // SAFETY: geteuid has no preconditions and cannot fail.
        if !meta.is_dir() || meta.uid() != unsafe { libc::geteuid() } {
            return Err(std::io::Error::new(
                std::io::ErrorKind::PermissionDenied,
                format!("{} is not this user's folder", dir.display()),
            ));
        }
        if meta.mode() & 0o077 != 0 {
            std::fs::set_permissions(dir, std::fs::Permissions::from_mode(0o700))?;
        }
        Ok(())
    }
    #[cfg(not(unix))]
    std::fs::create_dir_all(dir)
}

impl App {
    /// A clipboard key while a terminal has the keyboard. True when used.
    pub(super) fn clip_key(&mut self, ui: &mut Ui, code: &str, m: u16) -> bool {
        let Some(sid) = self.typing_target() else {
            return false;
        };
        let selection = self.st.sel.as_ref().is_some_and(|(s, _)| *s == sid);
        let Some(clip) = clipboard_key(code, m, selection, mods::mac()) else {
            return false;
        };
        let act = match clip {
            Clip::Copy => Act::CopySelection(sid),
            Clip::Paste => Act::Paste(sid),
            Clip::SelectAll => {
                if let Some(v) = self.views.get(&sid) {
                    self.st.sel = Some((sid, Sel::all(v)));
                }
                return true;
            }
        };
        self.run(ui, act, Src::default());
        true
    }

    /// Copies `session`'s selection, once vornd has its text; the selection goes.
    pub(super) fn copy_selection(&mut self, session: &str) {
        let Some((sid, sel)) = self.st.sel.take() else {
            return;
        };
        if sid != session {
            self.st.sel = Some((sid, sel));
            return;
        }
        if let Some(g) = &self.grid {
            g.copy(session, sel.from, sel.to);
        }
    }

    /// The clipboard into `session`: bracketed, and asked about first when
    /// vornd says the program would run it line by line. An image with no
    /// text is Ctrl-V, for the program to read from the clipboard itself.
    pub(super) fn paste_clipboard(&mut self, ui: &mut Ui, session: &str) {
        self.st.sel = None;
        let Some(g) = self.grid.clone() else {
            return;
        };
        let clip = ui.clipboard();
        match clip.get().filter(|t| !t.is_empty()) {
            Some(text) => {
                self.return_to_live(session);
                g.paste_clipboard(session, &text);
            }
            None if clip.has_image() => {
                self.return_to_live(session);
                g.key(session, "KeyV", mods::CTRL, None);
            }
            None => {}
        }
    }

    /// Files or an image dropped at `at` (or, with no place, where the
    /// keyboard is): the terminal under it takes the keyboard and the quoted
    /// paths, as typed text in a bracketed paste.
    pub fn dropped(&mut self, ui: &mut Ui, what: Dropped, at: Option<(f32, f32)>) {
        let busy = self.st.dialog.is_some() || self.st.menu.is_some() || self.st.settings.open;
        let Some(sid) = (!busy).then(|| self.drop_target(ui, at)).flatten() else {
            return;
        };
        if let Some(act) = self.acts.clicked(term_id(&sid)) {
            self.run(ui, act, Src::default());
        }
        match what {
            Dropped::Files(paths) => self.paste_paths(&sid, &paths),
            Dropped::Image { name, bytes } => self.paste_image(&sid, &name, &bytes),
        }
        self.events(ui);
    }

    /// The terminal under `at`, in logical pixels; with no place, the one
    /// typed into.
    fn drop_target(&self, ui: &Ui, at: Option<(f32, f32)>) -> Option<String> {
        let Some((x, y)) = at else {
            return self.typing_target();
        };
        let k = ui.scale();
        self.term_boxes
            .iter()
            .find(|(_, r)| r.contains(x * k, y * k))
            .map(|(s, _)| s.clone())
    }

    pub(super) fn paste_paths<P: AsRef<Path>>(&mut self, session: &str, paths: &[P]) {
        let text = quoted_paths(paths);
        if let (Some(g), false) = (self.grid.clone(), text.is_empty()) {
            self.return_to_live(session);
            g.paste(session, &text);
        }
    }

    /// Saves a dropped image to a temporary file and pastes its path.
    #[cfg(not(target_arch = "wasm32"))]
    fn paste_image(&mut self, session: &str, name: &str, bytes: &[u8]) {
        match save_image(&std::env::temp_dir().join("vorn-images"), name, bytes) {
            Ok(path) => self.paste_paths(session, &[path]),
            Err(e) => self
                .st
                .toast(format!("Could not save the image: {e}"), true),
        }
    }

    /// A page cannot write files where the program can read them: vornd
    /// keeps the image, and its path is pasted once vornd says where.
    #[cfg(target_arch = "wasm32")]
    fn paste_image(&mut self, session: &str, name: &str, bytes: &[u8]) {
        self.call(
            super::Call::TermImage(TermImage::Saved(session.to_owned())),
            "task:imageUpload",
            serde_json::json!({
                "taskId": PASTE_TASK,
                "base64": crate::device::base64(bytes),
                "filename": name,
            }),
        );
    }

    /// vornd's answers about a pasted or dropped image in a browser.
    pub(super) fn term_image_answered(
        &mut self,
        call: TermImage,
        answer: Result<serde_json::Value, crate::client::RpcError>,
    ) {
        let name = match answer {
            Ok(serde_json::Value::String(s)) => s,
            Ok(_) => return,
            Err(e) => {
                return self
                    .st
                    .toast(format!("Could not paste the image: {e}"), true)
            }
        };
        match call {
            TermImage::Saved(session) => self.call(
                super::Call::TermImage(TermImage::Path(session)),
                "task:imageGetPath",
                serde_json::json!({ "taskId": PASTE_TASK, "filename": name }),
            ),
            TermImage::Path(session) => self.paste_paths(&session, &[name]),
        }
    }

    /// The paste confirm dialog's answer.
    pub(super) fn confirm_paste(&mut self, ui: &mut Ui, run: bool) {
        let pending = self.st.paste.take();
        if self.st.dialog == Some(Dialog::Paste) {
            self.st.dialog = None;
            ui.set_focus(None);
        }
        if let (true, Some(g), Some((session, text))) = (run, self.grid.clone(), pending) {
            self.return_to_live(&session);
            g.paste(&session, &text);
        }
    }

    /// What vornd answered: a word to select, text to copy, a paste to confirm.
    pub(super) fn grid_replies(&mut self, ui: &mut Ui) {
        let Some(g) = &self.grid else {
            return;
        };
        for reply in g.take_replies() {
            match reply {
                Reply::Selection { session, range } => {
                    let primary = self.views.get(&session).is_some_and(|v| v.primary);
                    self.st.sel = range.map(|(a, b)| (session, Sel::new(a, b, primary)));
                }
                Reply::Copied(text) => {
                    if !text.is_empty() {
                        ui.clipboard().set(&text);
                    }
                }
                Reply::ConfirmPaste { session, text } => {
                    // A second paste while one is asked about waits for the next.
                    if self.st.dialog.is_none() {
                        self.st.paste = Some((session, text));
                        self.st.dialog = Some(Dialog::Paste);
                    }
                }
            }
        }
    }

    /// The session whose terminal box is `id`.
    fn term_at(&self, id: Id) -> Option<String> {
        self.term_boxes.keys().find(|s| term_id(s) == id).cloned()
    }

    /// The cell under `(x, y)` logical pixels in `session`'s terminal,
    /// held inside the screen.
    fn cell_at(&self, ui: &Ui, session: &str, (x, y): (f32, f32)) -> Option<GridPoint> {
        let (r, v) = (self.term_boxes.get(session)?, self.views.get(session)?);
        let (cell, scale) = (ui.text.cell(), ui.scale());
        let at = |p: f32, o: f32, size: f32, n: u16| {
            ((p * scale - o.round()) / size)
                .floor()
                .clamp(0.0, f32::from(n.saturating_sub(1))) as u16
        };
        Some(v.point(at(x, r.x, cell.w, v.cols), at(y, r.y, cell.h, v.rows)))
    }

    /// A drag or double-click in a terminal selects; a click clears the
    /// selection. True when the event was the terminal's alone.
    pub(super) fn term_event(&mut self, ui: &mut Ui, e: &UiEvent) -> bool {
        match *e {
            UiEvent::Move { id, dx, dy, x, y } | UiEvent::Drop { id, dx, dy, x, y } => {
                let Some(sid) = self.term_at(id) else {
                    return false;
                };
                let from = self.cell_at(ui, &sid, (x - dx, y - dy));
                let to = self.cell_at(ui, &sid, (x, y));
                if let (Some(a), Some(b)) = (from, to) {
                    let primary = self.views.get(&sid).is_some_and(|v| v.primary);
                    self.st.sel = Some((sid, Sel::new(a, b, primary)));
                }
                // A drag that selected still focuses the terminal, as a click would.
                if matches!(e, UiEvent::Drop { .. }) {
                    self.word_pending = false;
                    if let Some(act) = self.acts.clicked(id) {
                        self.run(ui, act, Src::default());
                    }
                }
                true
            }
            UiEvent::DoubleClick(id) => {
                let Some(sid) = self.term_at(id) else {
                    return false;
                };
                let at = ui.pointer().and_then(|p| self.cell_at(ui, &sid, p));
                if let (Some(g), Some(at)) = (&self.grid, at) {
                    g.select_word(&sid, at);
                    self.word_pending = true;
                }
                true
            }
            UiEvent::Click(id) if self.term_at(id).is_some() => {
                if !std::mem::take(&mut self.word_pending) {
                    self.st.sel = None;
                }
                false
            }
            _ => false,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const SHIFT: u16 = mods::SHIFT;
    const CTRL: u16 = mods::CTRL;
    const CMD: u16 = mods::SUPER;

    #[test]
    fn mac_clipboard_keys_are_cmd_c_v_and_a() {
        let key = |code, m| clipboard_key(code, m, false, true);
        assert_eq!(key("KeyC", CMD), Some(Clip::Copy));
        assert_eq!(key("KeyV", CMD), Some(Clip::Paste));
        assert_eq!(key("KeyA", CMD), Some(Clip::SelectAll));
        assert_eq!(key("Insert", SHIFT), Some(Clip::Paste));
        // Ctrl stays the terminal's: Ctrl-C interrupts, Ctrl-V is quoted insert.
        assert_eq!(clipboard_key("KeyC", CTRL, true, true), None);
        assert_eq!(key("KeyV", CTRL), None);
        assert_eq!(key("KeyA", CTRL), None);
        assert_eq!(key("KeyC", CMD | SHIFT), None);
    }

    #[test]
    fn ctrl_c_copies_only_over_a_selection_off_the_mac() {
        let key = |code, m, sel| clipboard_key(code, m, sel, false);
        assert_eq!(key("KeyC", CTRL, true), Some(Clip::Copy));
        assert_eq!(key("KeyC", CTRL, false), None);
        assert_eq!(key("KeyC", CTRL | SHIFT, false), Some(Clip::Copy));
        // Ctrl-V is the program's: agent CLIs paste the clipboard's image on it.
        assert_eq!(key("KeyV", CTRL, false), None);
        assert_eq!(key("KeyV", CTRL, true), None);
        assert_eq!(key("KeyV", CTRL | SHIFT, false), Some(Clip::Paste));
        assert_eq!(key("Insert", SHIFT, false), Some(Clip::Paste));
        assert_eq!(key("KeyA", CTRL, false), None);
        assert_eq!(key("KeyC", CMD, true), None);
        assert_eq!(key("Insert", 0, false), None);
    }

    #[test]
    fn paths_are_quoted_as_a_shell_reads_them_back() {
        assert_eq!(shell_quoted("/tmp/shot.png"), "/tmp/shot.png");
        assert_eq!(shell_quoted("/a/My Shot.png"), "'/a/My Shot.png'");
        assert_eq!(shell_quoted("/a/it's.png"), "'/a/it'\\''s.png'");
        assert_eq!(shell_quoted("/a/$HOME;rm"), "'/a/$HOME;rm'");
        assert_eq!(shell_quoted("/a/caf\u{e9}.png"), "'/a/caf\u{e9}.png'");
        assert_eq!(shell_quoted(""), "''");
        assert_eq!(shell_quoted(r"C:\Users\me\a.png"), r"C:\Users\me\a.png");
        assert_eq!(shell_quoted(r"C:\My Pics\a.png"), r#""C:\My Pics\a.png""#);
        assert_eq!(shell_quoted(r"/a\b"), r"'/a\b'");
    }

    #[test]
    fn dropped_paths_are_space_separated_with_a_trailing_space() {
        assert_eq!(
            quoted_paths(&["/a/one.png", "/b/two words.txt"]),
            "/a/one.png '/b/two words.txt' "
        );
        assert_eq!(quoted_paths::<&str>(&[]), "");
    }

    #[test]
    fn a_dropped_image_is_saved_under_a_new_name_each_time() {
        let dir = std::env::temp_dir().join(format!("vorn-app-drop-{}", std::process::id()));
        let a = save_image(&dir, "image.png", b"one").expect("saved");
        let b = save_image(&dir, "image.png", b"two").expect("saved");
        let c = save_image(&dir, "photo.JPG", b"three").expect("saved");
        let d = save_image(&dir, "noext", b"four").expect("saved");
        assert_ne!(a, b);
        assert_eq!(std::fs::read(&a).unwrap(), b"one");
        assert_eq!(std::fs::read(&b).unwrap(), b"two");
        assert_eq!(c.extension().unwrap(), "jpg");
        assert_eq!(d.extension().unwrap(), "png");
        assert!(a
            .file_name()
            .unwrap()
            .to_string_lossy()
            .starts_with("image-"));
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[cfg(unix)]
    #[test]
    fn images_are_only_saved_in_a_private_folder_of_this_user() {
        use std::os::unix::fs::PermissionsExt;
        let base = std::env::temp_dir().join(format!("vorn-app-private-{}", std::process::id()));
        let open = base.join("open");
        std::fs::create_dir_all(&open).unwrap();
        std::fs::set_permissions(&open, std::fs::Permissions::from_mode(0o777)).unwrap();
        save_image(&open, "image.png", b"x").expect("saved");
        let mode = std::fs::metadata(&open).unwrap().permissions().mode();
        assert_eq!(mode & 0o777, 0o700);
        let link = base.join("link");
        std::os::unix::fs::symlink(&open, &link).unwrap();
        let err = save_image(&link, "image.png", b"x").expect_err("a link is refused");
        assert_eq!(err.kind(), std::io::ErrorKind::PermissionDenied);
        std::fs::remove_dir_all(&base).unwrap();
    }
}

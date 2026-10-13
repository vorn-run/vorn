//! The `vorn` command on PATH, set up by installing Vorn.
//!
//! macOS and Linux: a link named `vorn` to the command inside this app (for
//! an AppImage, to the AppImage, whose `AppRun` runs the command line when
//! started through the link), so an update that replaces the app in place
//! keeps it working. A new link goes in `/usr/local/bin` on macOS when this
//! user may write there, else in `~/.local/bin`; a .deb ships `/usr/bin/vorn`.
//! Windows: the folder holding `vorn.exe` on the user's PATH, which the
//! installer adds and its uninstaller removes.
//!
//! The app makes the link on launch, repairs one that points at another copy
//! or nowhere, and says so once; Settings installs, reinstalls and removes it
//! the same way. After a removal the app leaves it alone until Install.
//! `VORN_CLI_DIR` names the folder to use instead (macOS and Linux), for a test.

use std::path::{Path, PathBuf};
use std::sync::mpsc::Receiver;

use serde::{Deserialize, Serialize};

use crate::client::rpc::Wake;
use crate::settings::CliState;
use crate::update::Install;

/// The folder to link into instead of choosing one.
pub const DIR_ENV: &str = "VORN_CLI_DIR";
/// What the app remembers about the link, in the data directory.
pub const MEMORY_FILE: &str = "cli-link.json";
const NAME: &str = "vorn";

/// What is at a place the link may be.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Found {
    Missing,
    /// A link to this app's command.
    Ours,
    /// A link or script an earlier Vorn made, to another copy or to nothing.
    Stale,
    /// Somebody else's `vorn`, replaced only when the person asks.
    Foreign,
}

/// What is at `link`, for a link that should point at `to`.
pub fn inspect(link: &Path, to: &Path) -> Found {
    let Ok(meta) = std::fs::symlink_metadata(link) else {
        return Found::Missing;
    };
    if meta.file_type().is_symlink() {
        let Ok(dest) = std::fs::read_link(link) else {
            return Found::Stale;
        };
        let dest = match link.parent() {
            Some(dir) if dest.is_relative() => dir.join(dest),
            _ => dest,
        };
        if dest == to || same_file(&dest, to) {
            return Found::Ours;
        }
        return if !dest.exists() || vorn_made(&dest) {
            Found::Stale
        } else {
            Found::Foreign
        };
    }
    // Earlier Vorns and install.sh wrote a script that runs the command inside the app.
    let mut head = [0u8; 1024];
    let n = std::fs::File::open(link)
        .and_then(|mut f| std::io::Read::read(&mut f, &mut head))
        .unwrap_or(0);
    let head = String::from_utf8_lossy(&head[..n]);
    if head.starts_with("#!") && (head.contains("vornd/vorn") || head.contains("VORN_CLI")) {
        Found::Stale
    } else {
        Found::Foreign
    }
}

fn same_file(a: &Path, b: &Path) -> bool {
    matches!((a.canonicalize(), b.canonicalize()), (Ok(a), Ok(b)) if a == b)
}

/// A path inside some copy of Vorn: its bundled command, or a Vorn AppImage.
fn vorn_made(dest: &Path) -> bool {
    dest.ends_with(Path::new("vornd").join(NAME))
        || dest
            .file_name()
            .and_then(|n| n.to_str())
            .is_some_and(|n| n.starts_with("Vorn") && n.ends_with(".AppImage"))
}

/// Where this machine keeps commands, as far as the link is concerned.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Env {
    pub home: PathBuf,
    /// The folders a new terminal searches, as far as the app can tell.
    pub path: Vec<PathBuf>,
    /// `VORN_CLI_DIR`.
    pub only: Option<PathBuf>,
    pub mac: bool,
}

impl Env {
    pub fn current() -> Env {
        let mut path: Vec<PathBuf> = std::env::var_os("PATH")
            .map(|p| std::env::split_paths(&p).collect())
            .unwrap_or_default();
        let mac = cfg!(target_os = "macos");
        // An app opened from Finder has launchd's PATH; a login shell adds these.
        if mac {
            path.extend(etc_paths(Path::new("/etc")));
        }
        Env {
            home: std::env::var_os("HOME").map_or_else(PathBuf::new, PathBuf::from),
            path,
            only: std::env::var_os(DIR_ENV)
                .filter(|v| !v.is_empty())
                .map(PathBuf::from),
            mac,
        }
    }

    fn user_bin(&self) -> PathBuf {
        self.home.join(".local").join("bin")
    }

    /// Where a link may already be, the one a shell finds first first.
    pub fn dirs(&self) -> Vec<PathBuf> {
        if let Some(only) = &self.only {
            return vec![only.clone()];
        }
        let mut dirs = vec![PathBuf::from("/usr/local/bin"), self.user_bin()];
        if !self.mac {
            dirs.insert(0, PathBuf::from("/usr/bin"));
        }
        dirs
    }

    /// The folder a new link goes in.
    pub fn new_dir(&self, writable: &dyn Fn(&Path) -> bool) -> PathBuf {
        if let Some(only) = &self.only {
            return only.clone();
        }
        let system = Path::new("/usr/local/bin");
        if self.mac && writable(system) {
            system.to_owned()
        } else {
            self.user_bin()
        }
    }

    /// Whether a new terminal finds commands in `dir`.
    pub fn on_path(&self, dir: &Path) -> bool {
        let fold = |p: &Path| p.to_string_lossy().trim_end_matches('/').to_owned();
        self.path.iter().any(|e| fold(e) == fold(dir))
    }
}

/// The folders `path_helper` puts on a macOS login shell's PATH: those in
/// `/etc/paths` and in each file of `/etc/paths.d`.
pub fn etc_paths(etc: &Path) -> Vec<PathBuf> {
    let mut files = vec![etc.join("paths")];
    if let Ok(dir) = std::fs::read_dir(etc.join("paths.d")) {
        let mut more: Vec<PathBuf> = dir.flatten().map(|e| e.path()).collect();
        more.sort();
        files.extend(more);
    }
    files
        .iter()
        .filter_map(|f| std::fs::read_to_string(f).ok())
        .flat_map(|body| {
            body.lines()
                .map(str::trim)
                .filter(|l| !l.is_empty() && !l.starts_with('#'))
                .map(PathBuf::from)
                .collect::<Vec<_>>()
        })
        .collect()
}

/// Whether this user may create files in `dir`.
#[cfg(unix)]
pub fn writable(dir: &Path) -> bool {
    use std::os::unix::ffi::OsStrExt;
    let Ok(c) = std::ffi::CString::new(dir.as_os_str().as_bytes()) else {
        return false;
    };
    // SAFETY: `c` is a valid NUL-terminated path that outlives the call.
    unsafe { libc::access(c.as_ptr(), libc::W_OK) == 0 }
}

#[cfg(not(unix))]
pub fn writable(dir: &Path) -> bool {
    std::fs::metadata(dir).is_ok_and(|m| !m.permissions().readonly())
}

/// Where the link is, or would go, and what is there now.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Status {
    pub link: PathBuf,
    pub found: Found,
}

/// The link to `to` on this machine: one already right, else one to repair,
/// else the place a new one goes.
pub fn status(env: &Env, to: &Path, writable: &dyn Fn(&Path) -> bool) -> Status {
    let seen: Vec<Status> = env
        .dirs()
        .into_iter()
        .map(|d| {
            let link = d.join(NAME);
            let found = inspect(&link, to);
            Status { link, found }
        })
        .collect();
    pick(&seen, writable).unwrap_or_else(|| {
        let link = env.new_dir(writable).join(NAME);
        let found = inspect(&link, to);
        Status { link, found }
    })
}

/// Of the links `seen`, the right one, else one this user can repair.
pub fn pick(seen: &[Status], writable: &dyn Fn(&Path) -> bool) -> Option<Status> {
    seen.iter()
        .find(|s| s.found == Found::Ours)
        .or_else(|| {
            seen.iter()
                .find(|s| s.found == Found::Stale && s.link.parent().is_some_and(writable))
        })
        .cloned()
}

/// Makes the link `status` names point at `to`; `force` replaces someone
/// else's `vorn` and remakes one already right.
pub fn link(status: &Status, to: &Path, force: bool) -> Result<PathBuf, String> {
    let at = &status.link;
    match status.found {
        Found::Ours if !force => return Ok(at.clone()),
        Found::Foreign if !force => {
            return Err(format!(
                "Another vorn is at {}, so Vorn left it alone.",
                tidy(at)
            ))
        }
        _ => {}
    }
    let dir = at.parent().ok_or("The link has no folder")?;
    let cannot = |e: std::io::Error| format!("Vorn cannot write to {}: {e}.", tidy(dir));
    std::fs::create_dir_all(dir).map_err(cannot)?;
    // Made beside it and renamed over it, so `vorn` is never missing.
    let fresh = dir.join(".vorn-link");
    let _ = std::fs::remove_file(&fresh);
    symlink(to, &fresh).map_err(cannot)?;
    std::fs::rename(&fresh, at).map_err(|e| {
        let _ = std::fs::remove_file(&fresh);
        cannot(e)
    })?;
    Ok(at.clone())
}

#[cfg(unix)]
fn symlink(to: &Path, at: &Path) -> std::io::Result<()> {
    std::os::unix::fs::symlink(to, at)
}

#[cfg(not(unix))]
fn symlink(_to: &Path, _at: &Path) -> std::io::Result<()> {
    Err(std::io::ErrorKind::Unsupported.into())
}

/// Removes every link to `to`, or to another copy, that `env` knows of and
/// this user may change; someone else's `vorn`, and a package's, stay. The
/// links removed.
pub fn unlink(
    env: &Env,
    to: &Path,
    writable: &dyn Fn(&Path) -> bool,
) -> Result<Vec<PathBuf>, String> {
    let mut removed = Vec::new();
    for dir in env.dirs() {
        let link = dir.join(NAME);
        if writable(&dir) && matches!(inspect(&link, to), Found::Ours | Found::Stale) {
            std::fs::remove_file(&link)
                .map_err(|e| format!("Vorn cannot remove {}: {e}.", tidy(&link)))?;
            removed.push(link);
        }
    }
    Ok(removed)
}

/// A path as a person reads it: the home folder as `~`.
pub fn tidy(path: &Path) -> String {
    let home = std::env::var_os(if cfg!(windows) { "USERPROFILE" } else { "HOME" })
        .filter(|h| !h.is_empty())
        .map(PathBuf::from);
    match home.and_then(|h| path.strip_prefix(&h).ok().map(Path::to_owned)) {
        Some(rest) if !cfg!(windows) => format!("~/{}", rest.display()),
        _ => path.display().to_string(),
    }
}

// Windows: the user's PATH, as the registry keeps it (`%VAR%` unexpanded).

/// `%NAME%` replaced with `var(NAME)`; an unknown name stays as written.
pub fn expand_vars(s: &str, var: &dyn Fn(&str) -> Option<String>) -> String {
    let mut out = String::new();
    let mut rest = s;
    while let Some(start) = rest.find('%') {
        out.push_str(&rest[..start]);
        let after = &rest[start + 1..];
        match after.find('%') {
            Some(end) => match var(&after[..end]) {
                Some(v) => {
                    out.push_str(&v);
                    rest = &after[end + 1..];
                }
                None => {
                    out.push('%');
                    rest = after;
                }
            },
            None => {
                out.push_str(&rest[start..]);
                rest = "";
            }
        }
    }
    out.push_str(rest);
    out
}

fn fold_win(s: &str) -> String {
    s.trim()
        .replace('/', "\\")
        .trim_end_matches('\\')
        .to_lowercase()
}

fn same_win_dir(entry: &str, dir: &str, var: &dyn Fn(&str) -> Option<String>) -> bool {
    !entry.trim().is_empty() && fold_win(&expand_vars(entry, var)) == fold_win(dir)
}

/// Whether the PATH `list` holds `dir`.
pub fn path_has(list: &str, dir: &str, var: &dyn Fn(&str) -> Option<String>) -> bool {
    list.split(';').any(|e| same_win_dir(e, dir, var))
}

/// `list` with `dir` at its end, or none when it is there already.
pub fn path_with(list: &str, dir: &str, var: &dyn Fn(&str) -> Option<String>) -> Option<String> {
    if path_has(list, dir, var) {
        return None;
    }
    let list = list.trim_end_matches(';');
    Some(if list.is_empty() {
        dir.to_owned()
    } else {
        format!("{list};{dir}")
    })
}

/// `list` without `dir`, or none when it is not there.
pub fn path_without(list: &str, dir: &str, var: &dyn Fn(&str) -> Option<String>) -> Option<String> {
    if !path_has(list, dir, var) {
        return None;
    }
    Some(
        list.split(';')
            .filter(|e| !same_win_dir(e, dir, var))
            .collect::<Vec<_>>()
            .join(";"),
    )
}

/// A path without Windows' `\\?\` prefix, as PATH and the installer write it.
fn plain(p: PathBuf) -> PathBuf {
    match p.to_str().and_then(|s| s.strip_prefix(r"\\?\")) {
        Some(rest) if !rest.starts_with("UNC\\") => PathBuf::from(rest),
        _ => p,
    }
}

/// What the link points at: the command inside this installed app, or the
/// AppImage itself. None for a build tree.
pub fn target() -> Option<PathBuf> {
    let exe = std::env::current_exe().ok()?;
    let exe = exe.canonicalize().unwrap_or(exe);
    let cmd = format!("{NAME}{}", std::env::consts::EXE_SUFFIX);
    let to = match Install::current() {
        Install::Mac(app) => app
            .join("Contents")
            .join("Resources")
            .join("vornd")
            .join(cmd),
        Install::AppImage(image) => image,
        Install::Windows | Install::Deb => exe.parent()?.join("resources").join("vornd").join(cmd),
        Install::Dev => return None,
    };
    to.is_file().then(|| plain(to))
}

/// Everything the app does to the link, for this machine.
mod os {
    use super::*;

    #[cfg(not(windows))]
    pub fn status(to: &Path) -> (Status, bool) {
        let env = Env::current();
        let s = super::status(&env, to, &writable);
        let on = s.link.parent().is_some_and(|d| env.on_path(d));
        (s, on)
    }

    #[cfg(not(windows))]
    pub fn install(to: &Path, force: bool) -> Result<PathBuf, String> {
        let env = Env::current();
        link(&super::status(&env, to, &writable), to, force)
    }

    #[cfg(not(windows))]
    pub fn remove(to: &Path) -> Result<(), String> {
        unlink(&Env::current(), to, &writable).map(|_| ())
    }

    #[cfg(windows)]
    fn var(name: &str) -> Option<String> {
        std::env::var(name).ok()
    }

    #[cfg(windows)]
    fn dir_of(to: &Path) -> String {
        to.parent()
            .map(|d| d.display().to_string())
            .unwrap_or_default()
    }

    #[cfg(windows)]
    pub fn status(to: &Path) -> (Status, bool) {
        let on = win::user_path()
            .ok()
            .flatten()
            .is_some_and(|list| path_has(&list, &dir_of(to), &var));
        let found = if on { Found::Ours } else { Found::Missing };
        let link = to.to_owned();
        (Status { link, found }, on)
    }

    #[cfg(windows)]
    pub fn install(to: &Path, _force: bool) -> Result<PathBuf, String> {
        let list = win::user_path()?.unwrap_or_default();
        if let Some(list) = path_with(&list, &dir_of(to), &var) {
            win::set_user_path(&list)?;
        }
        Ok(to.to_owned())
    }

    #[cfg(windows)]
    pub fn remove(to: &Path) -> Result<(), String> {
        let list = win::user_path()?.unwrap_or_default();
        match path_without(&list, &dir_of(to), &var) {
            Some(list) => win::set_user_path(&list),
            None => Ok(()),
        }
    }
}

#[cfg(windows)]
mod win {
    use windows_sys::Win32::Foundation::{ERROR_FILE_NOT_FOUND, ERROR_SUCCESS};
    use windows_sys::Win32::System::Registry::{
        RegCloseKey, RegGetValueW, RegOpenKeyExW, RegSetValueExW, HKEY, HKEY_CURRENT_USER,
        KEY_SET_VALUE, REG_EXPAND_SZ, RRF_NOEXPAND, RRF_RT_REG_EXPAND_SZ, RRF_RT_REG_SZ,
    };
    use windows_sys::Win32::UI::WindowsAndMessaging::{
        SendMessageTimeoutW, HWND_BROADCAST, SMTO_ABORTIFHUNG, WM_SETTINGCHANGE,
    };

    fn wide(s: &str) -> Vec<u16> {
        s.encode_utf16().chain(std::iter::once(0)).collect()
    }

    /// `HKCU\Environment\Path`, unexpanded; none when it is not set.
    pub fn user_path() -> Result<Option<String>, String> {
        let key = wide("Environment");
        let name = wide("Path");
        let flags = RRF_RT_REG_SZ | RRF_RT_REG_EXPAND_SZ | RRF_NOEXPAND;
        let mut bytes: u32 = 0;
        // SAFETY: the strings are NUL-terminated; a null buffer asks for the size.
        let rc = unsafe {
            RegGetValueW(
                HKEY_CURRENT_USER,
                key.as_ptr(),
                name.as_ptr(),
                flags,
                std::ptr::null_mut(),
                std::ptr::null_mut(),
                &mut bytes,
            )
        };
        if rc == ERROR_FILE_NOT_FOUND {
            return Ok(None);
        }
        if rc != ERROR_SUCCESS {
            return Err(format!("Vorn cannot read your PATH ({rc})."));
        }
        let mut buf = vec![0u16; (bytes as usize).div_ceil(2) + 1];
        let mut bytes = u32::try_from(buf.len() * 2).unwrap_or(u32::MAX);
        // SAFETY: `buf` holds `bytes` bytes and outlives the call.
        let rc = unsafe {
            RegGetValueW(
                HKEY_CURRENT_USER,
                key.as_ptr(),
                name.as_ptr(),
                flags,
                std::ptr::null_mut(),
                buf.as_mut_ptr().cast(),
                &mut bytes,
            )
        };
        if rc != ERROR_SUCCESS {
            return Err(format!("Vorn cannot read your PATH ({rc})."));
        }
        let len = buf.iter().position(|&c| c == 0).unwrap_or(buf.len());
        Ok(Some(String::from_utf16_lossy(&buf[..len])))
    }

    /// Writes the user's PATH and tells running programs, so new terminals see it.
    pub fn set_user_path(list: &str) -> Result<(), String> {
        let path = wide("Environment");
        let name = wide("Path");
        let mut key: HKEY = std::ptr::null_mut();
        // SAFETY: `path` is NUL-terminated; `key` is written on success.
        let rc =
            unsafe { RegOpenKeyExW(HKEY_CURRENT_USER, path.as_ptr(), 0, KEY_SET_VALUE, &mut key) };
        if rc != ERROR_SUCCESS {
            return Err(format!("Vorn cannot change your PATH ({rc})."));
        }
        let data = wide(list);
        let bytes = u32::try_from(data.len() * 2).unwrap_or(u32::MAX);
        // SAFETY: `key` is open; `data` is a NUL-terminated UTF-16 buffer of `bytes` bytes.
        let rc = unsafe {
            RegSetValueExW(
                key,
                name.as_ptr(),
                0,
                REG_EXPAND_SZ,
                data.as_ptr().cast(),
                bytes,
            )
        };
        // SAFETY: `key` was opened above and is closed once.
        unsafe { RegCloseKey(key) };
        if rc != ERROR_SUCCESS {
            return Err(format!("Vorn cannot change your PATH ({rc})."));
        }
        let what = wide("Environment");
        // SAFETY: `what` is NUL-terminated and outlives the call; a hung window is skipped.
        unsafe {
            SendMessageTimeoutW(
                HWND_BROADCAST,
                WM_SETTINGCHANGE,
                0,
                what.as_ptr() as isize,
                SMTO_ABORTIFHUNG,
                1000,
                std::ptr::null_mut(),
            )
        };
        Ok(())
    }
}

/// The Settings row's state.
pub fn state() -> CliState {
    let Some(to) = target() else {
        return CliState {
            available: false,
            installed: false,
            path: String::new(),
            on_path: false,
            note: Some("Only an installed Vorn can put its command on your PATH.".into()),
            managed: false,
        };
    };
    let (s, on_path) = os::status(&to);
    let managed = s.found == Found::Ours && !s.link.parent().is_some_and(writable);
    let note = match s.found {
        _ if managed => Some(format!(
            "{} comes with the package, which installs and removes it.",
            tidy(&s.link)
        )),
        Found::Foreign => Some(format!("Another vorn is at {}.", tidy(&s.link))),
        Found::Stale => Some(format!("{} points at another copy of Vorn.", tidy(&s.link))),
        _ => None,
    };
    CliState {
        available: true,
        installed: s.found == Found::Ours,
        path: tidy(&s.link),
        on_path,
        note,
        managed,
    }
}

/// What the app remembers about the link.
#[derive(Debug, Default, Clone, PartialEq, Eq, Serialize, Deserialize)]
struct Memory {
    /// The person removed it: the app leaves it alone until Install.
    #[serde(default)]
    removed: bool,
    /// The last notice shown, so an unchanged one is not shown again.
    #[serde(default)]
    told: Option<String>,
}

impl Memory {
    fn load(data_dir: &Path) -> Memory {
        std::fs::read(data_dir.join(MEMORY_FILE))
            .ok()
            .and_then(|b| serde_json::from_slice(&b).ok())
            .unwrap_or_default()
    }

    fn save(&self, data_dir: &Path) {
        if let Ok(body) = serde_json::to_vec_pretty(self) {
            let _ = std::fs::create_dir_all(data_dir);
            let _ = std::fs::write(data_dir.join(MEMORY_FILE), body);
        }
    }
}

/// What a notice offers to do about what it says.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Fix {
    /// Install again, replacing what is in the way.
    Install(&'static str),
    /// macOS: list the folder in `/etc/paths.d`, behind an administrator prompt.
    AddToPath(PathBuf),
    /// Copy the line that puts the folder on a shell's PATH.
    CopyPathLine(PathBuf),
}

impl Fix {
    pub fn label(&self) -> &'static str {
        match self {
            Fix::Install(label) => label,
            Fix::AddToPath(_) => "Add to PATH",
            Fix::CopyPathLine(_) => "Copy PATH line",
        }
    }
}

/// The line a shell's startup file takes to search `dir`.
pub fn path_line(dir: &Path) -> String {
    format!(
        "export PATH=\"{}:$PATH\"",
        tidy(dir).replacen('~', "$HOME", 1)
    )
}

/// The notice the launch step shows.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Notice {
    pub text: String,
    pub error: bool,
    pub fix: Option<Fix>,
}

/// What a link at `at` means to a person: where it is, and whether a new
/// terminal finds it.
pub fn installed_notice(at: &Path, on_path: bool, mac: bool) -> Notice {
    let text = format!("The vorn command is installed at {}.", tidy(at));
    let dir = at.parent().unwrap_or(at);
    if on_path || cfg!(windows) {
        return Notice {
            text,
            error: false,
            fix: None,
        };
    }
    Notice {
        text: format!("{text} {} is not on your PATH yet.", tidy(dir)),
        error: false,
        fix: Some(if mac {
            Fix::AddToPath(dir.to_owned())
        } else {
            Fix::CopyPathLine(dir.to_owned())
        }),
    }
}

/// The launch step: makes or repairs the link unless the person removed
/// it, and says what happened once.
pub fn on_launch(data_dir: &Path) -> Option<Notice> {
    let to = target()?;
    let mut memory = Memory::load(data_dir);
    if memory.removed {
        return None;
    }
    let (status, on_path) = os::status(&to);
    let (notice, changed) = match status.found {
        Found::Ours => (
            installed_notice(&status.link, on_path, cfg!(target_os = "macos")),
            false,
        ),
        Found::Foreign => (
            Notice {
                text: format!(
                    "Another vorn is at {}, so Vorn left it alone.",
                    tidy(&status.link)
                ),
                error: true,
                fix: Some(Fix::Install("Replace")),
            },
            false,
        ),
        Found::Missing | Found::Stale => match os::install(&to, false) {
            Ok(at) => {
                let (_, on_path) = os::status(&to);
                (
                    installed_notice(&at, on_path, cfg!(target_os = "macos")),
                    true,
                )
            }
            Err(e) => (
                Notice {
                    text: format!("Vorn could not install the vorn command. {e}"),
                    error: true,
                    fix: Some(Fix::Install("Try again")),
                },
                false,
            ),
        },
    };
    if !changed && memory.told.as_deref() == Some(notice.text.as_str()) {
        return None;
    }
    memory.told = Some(notice.text.clone());
    memory.save(data_dir);
    Some(notice)
}

/// Runs the launch step on a thread, waking the app with its notice.
pub fn start(data_dir: PathBuf, wake: Wake) -> Receiver<Option<Notice>> {
    let (tx, rx) = std::sync::mpsc::channel();
    std::thread::spawn(move || {
        let _ = tx.send(on_launch(&data_dir));
        wake();
    });
    rx
}

/// Settings' Install and Reinstall, and a notice's fix: makes the link,
/// replacing what is in the way, and forgets a removal.
pub fn install(data_dir: Option<&Path>) -> Result<PathBuf, String> {
    let to = target().ok_or("Only an installed Vorn can put its command on your PATH.")?;
    let at = os::install(&to, true)?;
    if let Some(dir) = data_dir {
        let (_, on_path) = os::status(&to);
        let mut memory = Memory::load(dir);
        memory.removed = false;
        memory.told = Some(installed_notice(&at, on_path, cfg!(target_os = "macos")).text);
        memory.save(dir);
    }
    Ok(at)
}

/// Settings' Remove: takes the link away and keeps the app from making it again.
pub fn remove(data_dir: Option<&Path>) -> Result<(), String> {
    let to = target().ok_or("Only an installed Vorn has a command to remove.")?;
    os::remove(&to)?;
    if let Some(dir) = data_dir {
        let mut memory = Memory::load(dir);
        memory.removed = true;
        memory.save(dir);
    }
    Ok(())
}

/// Lists `dir` in `/etc/paths.d/vorn` behind the system's administrator
/// prompt, on a thread; the answer wakes the app. `Ok(false)` is a cancel.
pub fn add_to_path(dir: PathBuf, wake: Wake) -> Receiver<Result<bool, String>> {
    let (tx, rx) = std::sync::mpsc::channel();
    std::thread::spawn(move || {
        let _ = tx.send(run_add_to_path(&dir));
        wake();
    });
    rx
}

fn run_add_to_path(dir: &Path) -> Result<bool, String> {
    // The folder arrives as an argument, quoted by AppleScript, never as script.
    let out = vorn_spawn::command("osascript")
        .args([
            "-e",
            "on run argv",
            "-e",
            "do shell script \"mkdir -p /etc/paths.d && printf '%s\\\\n' \" & quoted form of (item 1 of argv) & \" > /etc/paths.d/vorn\" with administrator privileges",
            "-e",
            "end run",
        ])
        .arg(dir)
        .stdin(std::process::Stdio::null())
        .output()
        .map_err(|e| e.to_string())?;
    if out.status.success() {
        return Ok(true);
    }
    let err = String::from_utf8_lossy(&out.stderr);
    // -128: the person cancelled the prompt.
    if err.contains("-128") {
        return Ok(false);
    }
    Err(format!(
        "Vorn could not add {} to your PATH. {}",
        tidy(dir),
        err.trim()
    ))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn temp(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("vorn-cli-link-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).expect("temp dir");
        dir
    }

    fn env(home: &Path, only: Option<PathBuf>, mac: bool) -> Env {
        Env {
            home: home.to_owned(),
            path: vec![PathBuf::from("/usr/bin")],
            only,
            mac,
        }
    }

    /// A fake app whose command the link points at.
    fn app(root: &Path, name: &str) -> PathBuf {
        let to = root.join(name).join("Contents/Resources/vornd/vorn");
        std::fs::create_dir_all(to.parent().unwrap()).unwrap();
        std::fs::write(&to, "#!/bin/sh\n").unwrap();
        to
    }

    #[test]
    fn a_new_link_prefers_a_writable_usr_local_bin_on_macos() {
        let home = Path::new("/Users/u");
        let yes = |_: &Path| true;
        let no = |_: &Path| false;
        let mac = env(home, None, true);
        assert_eq!(mac.new_dir(&yes), Path::new("/usr/local/bin"));
        assert_eq!(mac.new_dir(&no), home.join(".local/bin"));
        let linux = env(home, None, false);
        assert_eq!(linux.new_dir(&yes), home.join(".local/bin"));
        assert_eq!(linux.dirs()[0], Path::new("/usr/bin"));
        let only = env(home, Some(PathBuf::from("/t/bin")), true);
        assert_eq!(only.new_dir(&yes), Path::new("/t/bin"));
        assert_eq!(only.dirs(), vec![PathBuf::from("/t/bin")]);
    }

    #[cfg(unix)]
    #[test]
    fn links_repairs_and_removes_only_its_own() {
        let root = temp("flow");
        let bin = root.join("bin");
        let e = env(&root, Some(bin.clone()), true);
        let to = app(&root, "Vorn.app");
        let all = |_: &Path| true;

        // First launch: no folder yet; the link and its folder are made.
        let s = status(&e, &to, &all);
        assert_eq!(
            s,
            Status {
                link: bin.join("vorn"),
                found: Found::Missing
            }
        );
        assert_eq!(link(&s, &to, false).unwrap(), bin.join("vorn"));
        assert_eq!(std::fs::read_link(bin.join("vorn")).unwrap(), to);
        assert_eq!(status(&e, &to, &all).found, Found::Ours);

        // The app moved: the old link is broken, then repaired.
        let moved = app(&root, "Apps/Vorn.app");
        std::fs::remove_dir_all(root.join("Vorn.app")).unwrap();
        let s = status(&e, &moved, &all);
        assert_eq!(s.found, Found::Stale);
        link(&s, &moved, false).unwrap();
        assert_eq!(std::fs::read_link(bin.join("vorn")).unwrap(), moved);

        // Another copy of Vorn's command is replaced too.
        let other = app(&root, "Other/Vorn.app");
        assert_eq!(inspect(&bin.join("vorn"), &other), Found::Stale);

        // An earlier shim is Vorn's; somebody else's file is not.
        std::fs::remove_file(bin.join("vorn")).unwrap();
        std::fs::write(
            bin.join("vorn"),
            "#!/bin/sh\nexec \"/Applications/Vorn.app/Contents/Resources/vornd/vorn\" \"$@\"\n",
        )
        .unwrap();
        assert_eq!(inspect(&bin.join("vorn"), &moved), Found::Stale);
        std::fs::write(bin.join("vorn"), "#!/bin/sh\necho mine\n").unwrap();
        let s = status(&e, &moved, &all);
        assert_eq!(s.found, Found::Foreign);
        assert!(link(&s, &moved, false).is_err());
        assert_eq!(
            unlink(&e, &moved, &|_| true).unwrap(),
            Vec::<PathBuf>::new()
        );
        assert!(bin.join("vorn").is_file());
        link(&s, &moved, true).unwrap();
        assert_eq!(inspect(&bin.join("vorn"), &moved), Found::Ours);

        assert_eq!(
            unlink(&e, &moved, &|_| false).unwrap(),
            Vec::<PathBuf>::new()
        );
        assert_eq!(
            unlink(&e, &moved, &|_| true).unwrap(),
            vec![bin.join("vorn")]
        );
        assert_eq!(inspect(&bin.join("vorn"), &moved), Found::Missing);
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn an_existing_link_wins_then_one_this_user_can_repair() {
        let at = |dir: &str, found| Status {
            link: Path::new(dir).join("vorn"),
            found,
        };
        let seen = [
            at("/usr/bin", Found::Stale),
            at("/usr/local/bin", Found::Foreign),
            at("/h/.local/bin", Found::Ours),
        ];
        assert_eq!(pick(&seen, &|_| true), Some(seen[2].clone()));
        let seen = [
            at("/usr/bin", Found::Stale),
            at("/h/.local/bin", Found::Stale),
        ];
        let home_only = |d: &Path| d.starts_with("/h");
        assert_eq!(pick(&seen, &home_only), Some(seen[1].clone()));
        assert_eq!(pick(&seen[..1], &home_only), None);
        assert_eq!(pick(&[at("/x", Found::Foreign)], &|_| true), None);
    }

    #[test]
    fn macos_login_paths_come_from_etc_paths_and_paths_d() {
        let etc = temp("etc");
        std::fs::write(etc.join("paths"), "/usr/local/bin\n/usr/bin\n").unwrap();
        std::fs::create_dir_all(etc.join("paths.d")).unwrap();
        std::fs::write(etc.join("paths.d/vorn"), "/Users/u/.local/bin\n").unwrap();
        let paths = etc_paths(&etc);
        assert_eq!(
            paths,
            ["/usr/local/bin", "/usr/bin", "/Users/u/.local/bin"].map(PathBuf::from)
        );
        let e = Env {
            home: PathBuf::from("/Users/u"),
            path: paths,
            only: None,
            mac: true,
        };
        assert!(e.on_path(Path::new("/Users/u/.local/bin/")));
        assert!(!e.on_path(Path::new("/opt/bin")));
        let _ = std::fs::remove_dir_all(&etc);
    }

    #[test]
    fn the_notice_offers_a_path_fix_only_when_needed() {
        let at = Path::new("/x/bin/vorn");
        let n = installed_notice(at, true, true);
        assert_eq!(n.text, "The vorn command is installed at /x/bin/vorn.");
        assert_eq!(n.fix, None);
        if !cfg!(windows) {
            let n = installed_notice(at, false, true);
            assert!(n.text.ends_with("/x/bin is not on your PATH yet."));
            assert_eq!(n.fix, Some(Fix::AddToPath(PathBuf::from("/x/bin"))));
            let n = installed_notice(at, false, false);
            assert_eq!(n.fix, Some(Fix::CopyPathLine(PathBuf::from("/x/bin"))));
        }
        assert_eq!(
            path_line(Path::new("/x/bin")),
            "export PATH=\"/x/bin:$PATH\""
        );
    }

    #[test]
    fn the_memory_survives_a_restart_and_a_bad_file() {
        let dir = temp("memory");
        assert_eq!(Memory::load(&dir), Memory::default());
        let m = Memory {
            removed: true,
            told: Some("x".into()),
        };
        m.save(&dir);
        assert_eq!(Memory::load(&dir), m);
        std::fs::write(dir.join(MEMORY_FILE), "{").unwrap();
        assert_eq!(Memory::load(&dir), Memory::default());
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn the_windows_path_gains_and_loses_the_folder_once() {
        let var = |n: &str| (n == "LOCALAPPDATA").then(|| r"C:\Users\u\AppData\Local".to_owned());
        let dir = r"C:\Users\u\AppData\Local\Programs\vorn\resources\vornd";
        assert_eq!(path_with("", dir, &var).as_deref(), Some(dir));
        let list = r"C:\bin;%USERPROFILE%\x;";
        let added = path_with(list, dir, &var).unwrap();
        assert_eq!(added, format!(r"C:\bin;%USERPROFILE%\x;{dir}"));
        assert_eq!(path_with(&added, dir, &var), None);
        // Written another way, it is the same folder.
        let other = r"C:\bin;%LOCALAPPDATA%\Programs\Vorn\resources\vornd\";
        assert!(path_has(other, dir, &var));
        assert_eq!(path_with(other, dir, &var), None);
        assert_eq!(path_without(other, dir, &var).as_deref(), Some(r"C:\bin"));
        assert_eq!(path_without(r"C:\bin", dir, &var), None);
        assert_eq!(
            path_without(&added, dir, &var).as_deref(),
            Some(r"C:\bin;%USERPROFILE%\x")
        );
    }

    #[test]
    fn expands_known_variables_only() {
        let var = |n: &str| (n == "A").then(|| "x".to_owned());
        assert_eq!(expand_vars("%A%\\b", &var), "x\\b");
        assert_eq!(expand_vars("%B%\\%A%", &var), "%B%\\x");
        assert_eq!(expand_vars("50% off", &var), "50% off");
    }

    #[test]
    fn strips_the_verbatim_prefix() {
        assert_eq!(
            plain(PathBuf::from(r"\\?\C:\V\vorn.exe")),
            PathBuf::from(r"C:\V\vorn.exe")
        );
        assert_eq!(plain(PathBuf::from("/a/vorn")), PathBuf::from("/a/vorn"));
    }
}

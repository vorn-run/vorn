//! What the desktop's main process does for settings, done here: starting
//! at sign-in (`login-item.ts`), the way this platform does it. The `vorn`
//! command on PATH is `cli_link`'s.

use std::path::{Path, PathBuf};

use serde_json::Value;

/// Whether this copy runs from an install rather than a build tree: a
/// dev build would register a path into `target/`.
pub fn packaged() -> bool {
    std::env::current_exe().is_ok_and(|exe| !exe.components().any(|c| c.as_os_str() == "target"))
}

fn home() -> PathBuf {
    std::env::var_os(if cfg!(windows) { "USERPROFILE" } else { "HOME" })
        .map_or_else(PathBuf::new, PathBuf::from)
}

/// `loginItemFor`: what the OS record should say, or none to leave it
/// alone (a dev build, or no config yet).
pub fn login_item_for(config: &Value, packaged: bool) -> Option<bool> {
    if !packaged || !config.is_object() {
        return None;
    }
    Some(
        config
            .pointer("/defaults/startAtLogin")
            .and_then(Value::as_bool)
            == Some(true),
    )
}

/// The autostart entry on Linux, under `config` (`~/.config`).
pub fn desktop_entry(config: &Path) -> PathBuf {
    config.join("autostart").join("vorn.desktop")
}

/// The launch agent on macOS, under `home`.
pub fn launch_agent(home: &Path) -> PathBuf {
    home.join("Library/LaunchAgents/run.vorn.app.plist")
}

/// What the autostart file holds for `exe`.
pub fn autostart_file(exe: &Path, mac: bool) -> String {
    if mac {
        return format!(
            "<?xml version=\"1.0\" encoding=\"UTF-8\"?>\n<!DOCTYPE plist PUBLIC \"-//Apple//DTD PLIST 1.0//EN\" \"http://www.apple.com/DTDs/PropertyList-1.0.dtd\">\n<plist version=\"1.0\">\n<dict>\n  <key>Label</key><string>run.vorn.app</string>\n  <key>ProgramArguments</key><array><string>{}</string></array>\n  <key>RunAtLoad</key><true/>\n</dict>\n</plist>\n",
            exe.display()
        );
    }
    format!(
        "[Desktop Entry]\nType=Application\nName=Vorn\nExec=\"{}\"\nX-GNOME-Autostart-enabled=true\n",
        exe.display()
    )
}

/// Writes or removes the autostart file at `file`; true when it changed.
pub fn set_autostart_file(file: &Path, exe: &Path, on: bool, mac: bool) -> std::io::Result<bool> {
    if on {
        let body = autostart_file(exe, mac);
        if std::fs::read_to_string(file).ok().as_deref() == Some(body.as_str()) {
            return Ok(false);
        }
        if let Some(dir) = file.parent() {
            std::fs::create_dir_all(dir)?;
        }
        std::fs::write(file, body)?;
        return Ok(true);
    }
    match std::fs::remove_file(file) {
        Ok(()) => Ok(true),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(false),
        Err(e) => Err(e),
    }
}

/// `applyLoginItem`: makes the OS record match the config.
pub fn apply_login_item(config: &Value) -> Result<(), String> {
    let Some(on) = login_item_for(config, packaged()) else {
        return Ok(());
    };
    // An AppImage runs from a mount that is gone by the next login.
    let exe = match std::env::var_os("APPIMAGE").filter(|_| cfg!(target_os = "linux")) {
        Some(image) => PathBuf::from(image),
        None => std::env::current_exe().map_err(|e| e.to_string())?,
    };
    clear_legacy_login();
    set_login(&exe, on)
}

/// Removes the login item releases up to 0.8.0-beta.8 registered, which this
/// app's own record replaces and could not otherwise turn off.
#[cfg(target_os = "macos")]
fn clear_legacy_login() {
    use objc2::msg_send;
    use objc2::runtime::{AnyClass, AnyObject, Bool};
    #[link(name = "ServiceManagement", kind = "framework")]
    extern "C" {}
    // SMAppService arrived in macOS 13; before it there is nothing to clear.
    let Some(class) = AnyClass::get("SMAppService") else {
        return;
    };
    // SAFETY: documented SMAppService selectors with their declared types;
    // the service is a shared object that is not retained past this scope.
    unsafe {
        let service: *mut AnyObject = msg_send![class, mainAppService];
        if service.is_null() {
            return;
        }
        // 1 enabled, 2 waiting for approval.
        let status: isize = msg_send![service, status];
        if status == 1 || status == 2 {
            let mut error: *mut AnyObject = std::ptr::null_mut();
            let _: Bool = msg_send![service, unregisterAndReturnError: &mut error];
        }
    }
}

#[cfg(windows)]
fn clear_legacy_login() {
    for name in ["com.vorn.app", "electron.app.Vorn"] {
        let _ = win::set_run_value(name, None);
    }
}

#[cfg(all(unix, not(target_os = "macos")))]
fn clear_legacy_login() {}

/// `~/x` with the home directory spelled out.
pub fn expand_home(path: &str) -> PathBuf {
    match path.strip_prefix("~/").or_else(|| path.strip_prefix("~\\")) {
        Some(rest) => home().join(rest),
        None => PathBuf::from(path),
    }
}

/// A short tone, for the notification sound's Test button.
pub fn bell() {
    use std::io::Write;
    let mut out = std::io::stdout();
    let _ = out.write_all(b"\x07");
    let _ = out.flush();
}

/// A GET of `path` from vornd's HTTP port, as JSON.
pub fn http_get_json(port: u16, path: &str) -> Option<Value> {
    use std::io::{Read, Write};
    let mut s = std::net::TcpStream::connect(("127.0.0.1", port)).ok()?;
    s.set_read_timeout(Some(std::time::Duration::from_secs(3)))
        .ok()?;
    let req = format!("GET {path} HTTP/1.1\r\nHost: 127.0.0.1\r\nConnection: close\r\n\r\n");
    s.write_all(req.as_bytes()).ok()?;
    let mut res = String::new();
    s.read_to_string(&mut res).ok()?;
    serde_json::from_str(res.split_once("\r\n\r\n")?.1).ok()
}

/// The pid an older holder announced under `home`, if it still names
/// `instance` (`readAnnouncement`).
pub fn announced_pid(home: &Path, instance: &str) -> Option<i64> {
    if instance.is_empty()
        || instance.len() > 32
        || !instance
            .bytes()
            .all(|b| b.is_ascii_hexdigit() && !b.is_ascii_uppercase())
    {
        return None;
    }
    let text =
        std::fs::read_to_string(home.join("run").join(format!("sessiond-{instance}.info"))).ok()?;
    let field = |k: &str| {
        text.lines()
            .filter_map(|l| l.split_once('='))
            .find(|(a, _)| *a == k)
            .map(|(_, v)| v.to_owned())
    };
    let pid: i64 = field("pid")?.parse().ok().filter(|p| *p > 0)?;
    (field("instance")?.as_str() == instance).then_some(pid)
}

/// `endOlderHolder`: stops an older session holder vornd reports.
pub fn end_older_holder(
    home: &Path,
    reported: &crate::settings::Holders,
    instance: &str,
) -> Result<(), String> {
    let older = reported
        .older
        .iter()
        .find(|h| h.instance == instance)
        .ok_or("vornd does not report that session holder as older")?;
    match announced_pid(home, instance) {
        Some(pid) if pid == older.pid => {}
        _ => return Err("that session holder is no longer running".to_owned()),
    }
    kill(older.pid)?;
    let _ = std::fs::remove_file(home.join("run").join(format!("sessiond-{instance}.info")));
    Ok(())
}

#[cfg(unix)]
fn kill(pid: i64) -> Result<(), String> {
    let pid = i32::try_from(pid).map_err(|e| e.to_string())?;
    // SAFETY: kill(2) takes any pid; a stale one fails with ESRCH.
    if unsafe { libc::kill(pid, libc::SIGKILL) } == 0 {
        Ok(())
    } else {
        Err(std::io::Error::last_os_error().to_string())
    }
}

#[cfg(windows)]
fn kill(pid: i64) -> Result<(), String> {
    use windows_sys::Win32::Foundation::CloseHandle;
    use windows_sys::Win32::System::Threading::{OpenProcess, TerminateProcess, PROCESS_TERMINATE};
    let pid = u32::try_from(pid).map_err(|e| e.to_string())?;
    // SAFETY: OpenProcess returns null on failure, checked before use.
    let h = unsafe { OpenProcess(PROCESS_TERMINATE, 0, pid) };
    if h.is_null() {
        return Err(std::io::Error::last_os_error().to_string());
    }
    // SAFETY: `h` is a process handle opened above and closed once.
    let ok = unsafe { TerminateProcess(h, 1) } != 0;
    // SAFETY: as above.
    unsafe { CloseHandle(h) };
    if ok {
        Ok(())
    } else {
        Err(std::io::Error::last_os_error().to_string())
    }
}

#[cfg(windows)]
fn set_login(exe: &Path, on: bool) -> Result<(), String> {
    win::set_run_value(
        "Vorn",
        on.then(|| format!("\"{}\"", exe.display())).as_deref(),
    )
}

#[cfg(target_os = "macos")]
fn set_login(exe: &Path, on: bool) -> Result<(), String> {
    set_autostart_file(&launch_agent(&home()), exe, on, true)
        .map(|_| ())
        .map_err(|e| e.to_string())
}

#[cfg(all(unix, not(target_os = "macos")))]
fn set_login(exe: &Path, on: bool) -> Result<(), String> {
    let config =
        std::env::var_os("XDG_CONFIG_HOME").map_or_else(|| home().join(".config"), PathBuf::from);
    set_autostart_file(&desktop_entry(&config), exe, on, false)
        .map(|_| ())
        .map_err(|e| e.to_string())
}

#[cfg(windows)]
mod win {
    use windows_sys::Win32::Foundation::ERROR_SUCCESS;
    use windows_sys::Win32::System::Registry::{
        RegCloseKey, RegCreateKeyExW, RegDeleteValueW, RegSetValueExW, HKEY, HKEY_CURRENT_USER,
        KEY_SET_VALUE, REG_OPTION_NON_VOLATILE, REG_SZ,
    };

    fn wide(s: &str) -> Vec<u16> {
        s.encode_utf16().chain(std::iter::once(0)).collect()
    }

    /// Sets (or with none, deletes) `name` under the user's Run key.
    pub fn set_run_value(name: &str, value: Option<&str>) -> Result<(), String> {
        let path = wide("Software\\Microsoft\\Windows\\CurrentVersion\\Run");
        let name = wide(name);
        let mut key: HKEY = std::ptr::null_mut();
        // SAFETY: the strings are NUL-terminated and outlive the call; `key` is written on success.
        let rc = unsafe {
            RegCreateKeyExW(
                HKEY_CURRENT_USER,
                path.as_ptr(),
                0,
                std::ptr::null(),
                REG_OPTION_NON_VOLATILE,
                KEY_SET_VALUE,
                std::ptr::null(),
                &mut key,
                std::ptr::null_mut(),
            )
        };
        if rc != ERROR_SUCCESS {
            return Err(format!("could not open the Run key ({rc})"));
        }
        let rc = match value {
            Some(v) => {
                let data = wide(v);
                let bytes = u32::try_from(data.len() * 2).unwrap_or(u32::MAX);
                // SAFETY: `key` is open; `data` is a NUL-terminated UTF-16 buffer of `bytes` bytes.
                unsafe {
                    RegSetValueExW(key, name.as_ptr(), 0, REG_SZ, data.as_ptr().cast(), bytes)
                }
            }
            // SAFETY: `key` is open and `name` NUL-terminated.
            None => match unsafe { RegDeleteValueW(key, name.as_ptr()) } {
                2 => ERROR_SUCCESS,
                rc => rc,
            },
        };
        // SAFETY: `key` was opened above and is closed once.
        unsafe { RegCloseKey(key) };
        if rc == ERROR_SUCCESS {
            Ok(())
        } else {
            Err(format!("could not write the Run key ({rc})"))
        }
    }
}

/// What the OS is asked to choose.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Pick {
    /// An existing connector pack.
    Pack(String),
    /// An existing folder.
    Folder(String),
    /// An existing workflow file.
    Workflow(String),
    /// Where to write a file, offering `at`.
    Save { title: String, at: PathBuf },
}

/// Answers a [`Pick`]; the OS dialogs unless a test swaps them.
pub type Picker = std::sync::Arc<dyn Fn(Pick) -> Option<PathBuf> + Send + Sync>;

/// The system's own dialogs.
pub fn os_picker() -> Picker {
    std::sync::Arc::new(run_picker)
}

/// Runs `picker` on a thread, waking the app with the answer; `None` when
/// the picker was cancelled or this system has none.
pub fn pick(
    picker: &Picker,
    what: Pick,
    wake: crate::client::rpc::Wake,
) -> std::sync::mpsc::Receiver<Option<PathBuf>> {
    let (tx, rx) = std::sync::mpsc::channel();
    let picker = picker.clone();
    std::thread::spawn(move || {
        let _ = tx.send(picker(what));
        wake();
    });
    rx
}

fn run_picker(what: Pick) -> Option<PathBuf> {
    let attempts: Vec<std::process::Command> = if cfg!(target_os = "macos") {
        let script = match &what {
            Pick::Folder(_) => "POSIX path of (choose folder with prompt (item 1 of argv))",
            Pick::Save { .. } => {
                "POSIX path of (choose file name with prompt (item 1 of argv) default name (item 2 of argv) default location (POSIX file (item 3 of argv)))"
            }
            Pick::Pack(_) | Pick::Workflow(_) => {
                "POSIX path of (choose file with prompt (item 1 of argv))"
            }
        };
        let mut c = vorn_spawn::command("osascript");
        c.args(["-e", "on run argv", "-e", script, "-e", "end run"]);
        c.args(pick_args(&what));
        vec![c]
    } else if cfg!(windows) {
        let (script, filter) = match &what {
            Pick::Folder(_) => (WINDOWS_FOLDER, ""),
            Pick::Save { .. } => (WINDOWS_SAVE, WORKFLOW_FILTER),
            Pick::Workflow(_) => (WINDOWS_OPEN, WORKFLOW_FILTER),
            Pick::Pack(_) => (
                WINDOWS_OPEN,
                "Vorn packs (*.vorn.tgz)|*.vorn.tgz|All files (*.*)|*.*",
            ),
        };
        let args = pick_args(&what);
        let mut c = vorn_spawn::command("powershell");
        c.args(["-NoProfile", "-NonInteractive", "-STA", "-Command", script])
            .env("VORN_PICK_TITLE", &args[0])
            .env("VORN_PICK_NAME", args.get(1).map_or("", String::as_str))
            .env("VORN_PICK_DIR", args.get(2).map_or("", String::as_str))
            .env("VORN_PICK_FILTER", filter);
        vec![c]
    } else {
        let args = pick_args(&what);
        let title = args[0].as_str();
        let mut z = vorn_spawn::command("zenity");
        z.args(["--file-selection", "--title", title]);
        let mut k = vorn_spawn::command("kdialog");
        match &what {
            Pick::Folder(_) => {
                z.arg("--directory");
                k.args(["--getexistingdirectory", "."]);
            }
            Pick::Save { at, .. } => {
                z.args(["--save", "--confirm-overwrite", "--filename"])
                    .arg(at);
                k.arg("--getsavefilename").arg(at);
            }
            Pick::Pack(_) | Pick::Workflow(_) => {
                k.args(["--getopenfilename", "."]);
            }
        }
        k.args(["--title", title]);
        vec![z, k]
    };
    for mut c in attempts {
        let Ok(out) = c.stdin(std::process::Stdio::null()).output() else {
            continue;
        };
        let path = String::from_utf8_lossy(&out.stdout).trim().to_owned();
        return (out.status.success() && !path.is_empty()).then(|| PathBuf::from(path));
    }
    None
}

/// The dialog's title, then, when saving, the name and folder offered.
fn pick_args(what: &Pick) -> Vec<String> {
    match what {
        Pick::Pack(t) | Pick::Folder(t) | Pick::Workflow(t) => vec![t.clone()],
        Pick::Save { title, at } => {
            let name = at.file_name().unwrap_or_default().to_string_lossy();
            let dir = at.parent().unwrap_or(at).to_string_lossy();
            vec![title.clone(), name.into_owned(), dir.into_owned()]
        }
    }
}

const WORKFLOW_FILTER: &str = "Vorn workflow (*.json)|*.json|All files (*.*)|*.*";

/// The system's open dialog; its title and filter come from the environment so they are never read as script.
const WINDOWS_OPEN: &str = r"
Add-Type -AssemblyName System.Windows.Forms
$d = New-Object System.Windows.Forms.OpenFileDialog
$d.Title = $env:VORN_PICK_TITLE
$d.Filter = $env:VORN_PICK_FILTER
if ($d.ShowDialog() -eq [System.Windows.Forms.DialogResult]::OK) { $d.FileName }
";

/// The system's folder dialog.
const WINDOWS_FOLDER: &str = r"
Add-Type -AssemblyName System.Windows.Forms
$d = New-Object System.Windows.Forms.FolderBrowserDialog
$d.Description = $env:VORN_PICK_TITLE
if ($d.ShowDialog() -eq [System.Windows.Forms.DialogResult]::OK) { $d.SelectedPath }
";

/// The system's save dialog.
const WINDOWS_SAVE: &str = r"
Add-Type -AssemblyName System.Windows.Forms
$d = New-Object System.Windows.Forms.SaveFileDialog
$d.Title = $env:VORN_PICK_TITLE
$d.FileName = $env:VORN_PICK_NAME
$d.Filter = $env:VORN_PICK_FILTER
$d.InitialDirectory = $env:VORN_PICK_DIR
if ($d.ShowDialog() -eq [System.Windows.Forms.DialogResult]::OK) { $d.FileName }
";

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn a_dev_build_leaves_the_login_record_alone() {
        let c = json!({ "defaults": { "startAtLogin": true } });
        assert_eq!(login_item_for(&c, false), None);
        assert_eq!(login_item_for(&c, true), Some(true));
        assert_eq!(
            login_item_for(&json!({ "defaults": {} }), true),
            Some(false)
        );
        assert_eq!(login_item_for(&Value::Null, true), None);
    }

    #[test]
    fn the_autostart_file_is_written_once_and_removed() {
        let dir = std::env::temp_dir().join(format!("vorn-autostart-{}", std::process::id()));
        let file = desktop_entry(&dir);
        let exe = Path::new("/opt/vorn/vorn-app");
        assert!(set_autostart_file(&file, exe, true, false).expect("write"));
        assert!(!set_autostart_file(&file, exe, true, false).expect("same"));
        let body = std::fs::read_to_string(&file).expect("read");
        assert!(body.contains("Exec=\"/opt/vorn/vorn-app\""));
        assert!(set_autostart_file(&file, exe, false, false).expect("remove"));
        assert!(!set_autostart_file(&file, exe, false, false).expect("gone"));
        let _ = std::fs::remove_dir_all(&dir);
    }
}

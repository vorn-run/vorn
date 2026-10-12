//! What the desktop's main process does for settings, done here: the
//! `vorn` command on PATH (`cli-shim.ts`) and starting at sign-in
//! (`login-item.ts`), each the way this platform does it.

use std::path::{Path, PathBuf};

use serde_json::Value;

use crate::settings::CliState;

/// Whether this copy runs from an install rather than a build tree: a
/// dev build would register a path into `target/`.
pub fn packaged() -> bool {
    std::env::current_exe().is_ok_and(|exe| !exe.components().any(|c| c.as_os_str() == "target"))
}

/// Whether a shell would find a command in `dir`; case-folded on Windows.
pub fn on_path(dir: &Path, path_var: &str) -> bool {
    let fold = |p: &Path| {
        let s = p.to_string_lossy().trim_end_matches(['/', '\\']).to_owned();
        if cfg!(windows) {
            s.to_lowercase()
        } else {
            s
        }
    };
    let wanted = fold(dir);
    std::env::split_paths(path_var).any(|e| !e.as_os_str().is_empty() && fold(&e) == wanted)
}

fn writable(dir: &Path) -> bool {
    std::fs::metadata(dir).is_ok_and(|m| !m.permissions().readonly())
}

/// `shimDirectory`: beside the app on Windows; elsewhere the first
/// writable directory the shell searches, else `~/.local/bin`.
pub fn shim_dir(exe: &Path, home: &Path, path_var: &str) -> PathBuf {
    if cfg!(windows) {
        return exe.parent().map_or_else(PathBuf::new, Path::to_path_buf);
    }
    let user = home.join(".local").join("bin");
    let candidates: Vec<PathBuf> = [PathBuf::from("/usr/local/bin"), user.clone()]
        .into_iter()
        .filter(|d| writable(d))
        .collect();
    candidates
        .iter()
        .find(|d| on_path(d, path_var))
        .or(candidates.first())
        .cloned()
        .unwrap_or(user)
}

/// `shimScript`: no arguments opens the app, anything else runs `vorn`.
pub fn shim_script(app: &Path, cli: &Path, windows: bool) -> String {
    if windows {
        return [
            "@echo off".to_owned(),
            "setlocal".to_owned(),
            "if \"%~1\"==\"\" (".to_owned(),
            format!("  start \"\" \"{}\"", app.display()),
            "  exit /b".to_owned(),
            ")".to_owned(),
            format!("\"{}\" %*", cli.display()),
            String::new(),
        ]
        .join("\r\n");
    }
    format!(
        "#!/bin/sh\n# No command: open the app, which is what typing `vorn` should mean.\nif [ \"$#\" -eq 0 ]; then\n  exec \"{}\"\nfi\n\nexec \"{}\" \"$@\"\n",
        app.display(),
        cli.display()
    )
}

fn home() -> PathBuf {
    std::env::var_os(if cfg!(windows) { "USERPROFILE" } else { "HOME" })
        .map_or_else(PathBuf::new, PathBuf::from)
}

fn shim_name() -> &'static str {
    if cfg!(windows) {
        "vorn.cmd"
    } else {
        "vorn"
    }
}

/// The `vorn` command beside this app, which the shim runs.
fn cli_beside(exe: &Path) -> Option<PathBuf> {
    let cli = exe
        .parent()?
        .join(format!("vorn{}", std::env::consts::EXE_SUFFIX));
    cli.is_file().then_some(cli)
}

/// `cliShimStatus`.
pub fn cli_status() -> CliState {
    let exe = std::env::current_exe().unwrap_or_default();
    let path_var = std::env::var("PATH").unwrap_or_default();
    let dir = shim_dir(&exe, &home(), &path_var);
    let target = dir.join(shim_name());
    CliState {
        available: packaged() && cli_beside(&exe).is_some(),
        installed: target.is_file(),
        path: target.to_string_lossy().into_owned(),
        on_path: on_path(&dir, &path_var),
    }
}

/// `installCliShim`: writes the shim, executable, and says where.
pub fn install_cli() -> Result<String, String> {
    let exe = std::env::current_exe().map_err(|e| e.to_string())?;
    let cli = cli_beside(&exe).ok_or("This build has no vorn command beside it")?;
    let path_var = std::env::var("PATH").unwrap_or_default();
    let dir = shim_dir(&exe, &home(), &path_var);
    std::fs::create_dir_all(&dir).map_err(|e| e.to_string())?;
    let target = dir.join(shim_name());
    std::fs::write(&target, shim_script(&exe, &cli, cfg!(windows))).map_err(|e| e.to_string())?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(&target, std::fs::Permissions::from_mode(0o755))
            .map_err(|e| e.to_string())?;
    }
    Ok(target.to_string_lossy().into_owned())
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
    let exe = std::env::current_exe().map_err(|e| e.to_string())?;
    set_login(&exe, on)
}

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

    #[test]
    fn shims_open_the_app_or_run_the_command() {
        let sh = shim_script(Path::new("/a/vorn-app"), Path::new("/a/vorn"), false);
        assert!(sh.starts_with("#!/bin/sh"));
        assert!(sh.contains("exec \"/a/vorn\" \"$@\""));
        let cmd = shim_script(
            Path::new("C:\\V\\vorn-app.exe"),
            Path::new("C:\\V\\vorn.exe"),
            true,
        );
        assert!(cmd.contains("start \"\" \"C:\\V\\vorn-app.exe\""));
        assert!(cmd.contains("\r\n\"C:\\V\\vorn.exe\" %*"));
    }

    #[test]
    fn path_entries_match_with_or_without_a_trailing_separator() {
        let sep = if cfg!(windows) { ";" } else { ":" };
        let var = format!("/usr/bin{sep}/home/u/.local/bin/");
        assert!(on_path(Path::new("/home/u/.local/bin"), &var));
        assert!(!on_path(Path::new("/opt/bin"), &var));
    }
}

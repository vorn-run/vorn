//! The optional macOS sandbox profile for a program extension: a person
//! turns it on per extension, and its programs then start under Seatbelt
//! (`sandbox-exec`) with no network of their own, every request going
//! through the bridge on stdio or through the network proxy
//! ([`crate::netproxy`]) to the hosts it declares, and writes only inside the
//! extension's own folder. Reading stays open, as the program and its
//! runtime need it.
//!
//! The switch is a file beside the package's pointer, so removing the
//! extension removes it too.

use std::fs;
use std::path::{Path, PathBuf};

use crate::child::Launch;

/// Where the switch is kept, beside `current.json`.
pub const SETTINGS_FILE: &str = "sandbox.json";

/// The program that applies a profile.
pub const SANDBOX_EXEC: &str = "/usr/bin/sandbox-exec";

/// Whether this machine can start a program under the profile.
pub fn available() -> bool {
    cfg!(target_os = "macos") && Path::new(SANDBOX_EXEC).is_file()
}

/// Whether the person turned the profile on for the extension kept at `home` (`<root>/<id>`).
pub fn is_on(home: &Path) -> bool {
    fs::read_to_string(home.join(SETTINGS_FILE))
        .ok()
        .and_then(|t| serde_json::from_str::<serde_json::Value>(&t).ok())
        .and_then(|v| v.get("on").and_then(serde_json::Value::as_bool))
        .unwrap_or(false)
}

/// Turns the profile on or off for the extension kept at `home`.
pub fn set(home: &Path, on: bool) -> Result<(), String> {
    if on && !available() {
        return Err("The sandbox profile needs macOS".to_owned());
    }
    if !home.is_dir() {
        return Err(format!("{} is not an installed extension", home.display()));
    }
    let path = home.join(SETTINGS_FILE);
    if !on {
        return match fs::remove_file(&path) {
            Err(e) if e.kind() != std::io::ErrorKind::NotFound => {
                Err(format!("cannot turn the sandbox off: {e}"))
            }
            _ => Ok(()),
        };
    }
    fs::write(&path, "{\"on\":true}\n").map_err(|e| format!("cannot turn the sandbox on: {e}"))
}

/// SBPL string literal for `s`.
fn quoted(s: &str) -> String {
    let mut out = String::with_capacity(s.len() + 2);
    out.push('"');
    for c in s.chars() {
        if c == '"' || c == '\\' {
            out.push('\\');
        }
        out.push(c);
    }
    out.push('"');
    out
}

/// The profile for a program whose own folder is `dir`, already resolved
/// through symlinks, as Seatbelt matches real paths; `proxy` is the loopback
/// port of the network proxy, the one place it may connect to.
pub fn profile(dir: &Path, proxy: Option<u16>) -> String {
    let through = proxy.map_or_else(String::new, |port| {
        format!("(allow network-outbound (remote tcp \"localhost:{port}\"))\n")
    });
    format!(
        "(version 1)
(allow default)
(deny network*)
{through}(deny file-write*)
(allow file-write*
  (subpath {dir})
  (literal \"/dev/null\")
  (literal \"/dev/zero\")
  (literal \"/dev/dtracehelper\")
  (regex #\"^/dev/fd/\")
  (regex #\"^/dev/tty\"))
",
        dir = quoted(&dir.to_string_lossy())
    )
}

/// `launch` started under the profile, writes held to `dir`, no network.
pub fn wrap(launch: Launch, dir: &Path) -> Launch {
    wrap_through(launch, dir, None)
}

/// `launch` started under the profile, writes held to `dir`, reaching the
/// network only through the proxy at `proxy`.
pub fn wrap_through(launch: Launch, dir: &Path, proxy: Option<u16>) -> Launch {
    let real: PathBuf = fs::canonicalize(dir).unwrap_or_else(|_| dir.to_path_buf());
    let mut args = Vec::with_capacity(launch.args.len() + 3);
    args.push("-p".to_owned());
    args.push(profile(&real, proxy));
    args.push(launch.program.to_string_lossy().into_owned());
    args.extend(launch.args);
    Launch {
        program: PathBuf::from(SANDBOX_EXEC),
        args,
        ..launch
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn quotes_the_folder_and_holds_writes_to_it() {
        let p = profile(Path::new("/a \"b\"/c"), None);
        assert!(p.contains("(deny network*)"));
        assert!(!p.contains("network-outbound"));
        let through = profile(Path::new("/x"), Some(4242));
        assert!(through.contains("(allow network-outbound (remote tcp \"localhost:4242\"))"));
        assert!(p.contains("(subpath \"/a \\\"b\\\"/c\")"), "{p}");
    }

    #[test]
    fn wraps_the_program_and_keeps_its_place_and_environment() {
        let launch = Launch {
            program: "/bin/node".into(),
            args: vec!["main.js".into()],
            cwd: "/x".into(),
            env: vec![("A".into(), "1".into())],
        };
        let wrapped = wrap(launch, Path::new("/nowhere/x"));
        assert_eq!(wrapped.program, PathBuf::from(SANDBOX_EXEC));
        assert_eq!(wrapped.args[0], "-p");
        assert_eq!(&wrapped.args[2..], ["/bin/node", "main.js"]);
        assert_eq!(wrapped.cwd, PathBuf::from("/x"));
        assert_eq!(wrapped.env, [("A".to_owned(), "1".to_owned())]);
    }

    #[test]
    fn the_switch_is_a_file_beside_the_pointer() {
        let home = tempfile::tempdir().unwrap();
        assert!(!is_on(home.path()));
        if available() {
            set(home.path(), true).unwrap();
            assert!(is_on(home.path()));
        } else {
            assert_eq!(
                set(home.path(), true).unwrap_err(),
                "The sandbox profile needs macOS"
            );
        }
        set(home.path(), false).unwrap();
        assert!(!is_on(home.path()));
        set(home.path(), false).unwrap();
    }
}

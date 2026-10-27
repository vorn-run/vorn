//! Files only this user may read: mode 0600 on Unix, and on Windows an
//! access list that names this user alone, with nothing inherited from the
//! folder.

use std::io;
use std::path::Path;

/// Restricts `path` to its owner. On Windows this runs `icacls`, as the
/// one place Vorn changes an access list; a failure is the caller's to report.
pub fn restrict(path: &Path) -> io::Result<()> {
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o600))
    }
    #[cfg(windows)]
    {
        let user = windows_user().ok_or_else(|| io::Error::other("no USERNAME to grant"))?;
        let out = vorn_spawn::command("icacls")
            .arg(path)
            .args(["/inheritance:r", "/grant:r"])
            .arg(format!("{user}:F"))
            .stdin(std::process::Stdio::null())
            .output()?;
        if out.status.success() {
            Ok(())
        } else {
            Err(io::Error::other(format!(
                "icacls could not restrict {}: {}",
                path.display(),
                String::from_utf8_lossy(&out.stdout).trim()
            )))
        }
    }
    #[cfg(not(any(unix, windows)))]
    {
        let _ = path;
        Ok(())
    }
}

/// `DOMAIN\user`, or the bare user name, as `icacls` takes it.
#[cfg(windows)]
fn windows_user() -> Option<String> {
    let user = std::env::var("USERNAME").ok().filter(|u| !u.is_empty())?;
    Some(
        match std::env::var("USERDOMAIN").ok().filter(|d| !d.is_empty()) {
            Some(domain) => format!("{domain}\\{user}"),
            None => user,
        },
    )
}

#[cfg(test)]
mod tests {
    #[cfg(unix)]
    #[test]
    fn a_file_left_readable_by_others_is_made_private() {
        use std::os::unix::fs::PermissionsExt;
        let dir = tempfile::tempdir().unwrap();
        let file = dir.path().join("f");
        std::fs::write(&file, "x").unwrap();
        std::fs::set_permissions(&file, std::fs::Permissions::from_mode(0o644)).unwrap();
        super::restrict(&file).unwrap();
        let mode = std::fs::metadata(&file).unwrap().permissions().mode();
        assert_eq!(mode & 0o777, 0o600);
    }
}

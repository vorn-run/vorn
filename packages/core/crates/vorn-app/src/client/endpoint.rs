//! Finding a running vornd: its data directory holds `ws-port` (`{port, pid}`)
//! and `local-token`, and the pid names its grid endpoint.

use std::fmt;
use std::path::{Path, PathBuf};

use serde::Deserialize;

/// Overrides the data directory, as the other native clients honour it.
pub const DATA_DIR_ENV: &str = "VORN_DATA_DIR";

/// Why a vornd could not be found.
#[derive(Debug)]
pub enum FindError {
    /// No home directory to look under.
    NoHome,
    /// A file vornd writes when it starts is missing or unreadable.
    Read(PathBuf, std::io::Error),
    /// `ws-port` is not the `{port, pid}` vornd writes.
    BadPortFile(String),
    /// `local-token` is empty.
    NoToken,
}

impl fmt::Display for FindError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            FindError::NoHome => f.write_str("no home directory to find vornd under"),
            FindError::Read(path, e) => write!(f, "{}: {e}", path.display()),
            FindError::BadPortFile(e) => write!(f, "ws-port: {e}"),
            FindError::NoToken => f.write_str("local-token is empty"),
        }
    }
}

impl std::error::Error for FindError {}

/// Where one vornd listens and the token it admits.
#[derive(Clone, PartialEq, Eq)]
pub struct Endpoint {
    pub port: u16,
    pub pid: u32,
    pub token: String,
    /// The grid endpoint: a Unix socket path, or a named pipe on Windows.
    pub grid: String,
}

// The token is a credential: keep it out of logs and panics.
impl fmt::Debug for Endpoint {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Endpoint")
            .field("port", &self.port)
            .field("pid", &self.pid)
            .field("grid", &self.grid)
            .finish_non_exhaustive()
    }
}

#[derive(Deserialize)]
struct PortFile {
    port: u16,
    pid: u32,
}

impl Endpoint {
    /// The vornd serving `data_dir`.
    pub fn find(data_dir: &Path) -> Result<Endpoint, FindError> {
        let read = |name: &str| {
            let path = data_dir.join(name);
            std::fs::read_to_string(&path).map_err(|e| FindError::Read(path, e))
        };
        let ports: PortFile = serde_json::from_str(&read("ws-port")?)
            .map_err(|e| FindError::BadPortFile(e.to_string()))?;
        let token = read("local-token")?.trim().to_owned();
        if token.is_empty() {
            return Err(FindError::NoToken);
        }
        Ok(Endpoint {
            port: ports.port,
            pid: ports.pid,
            token,
            grid: grid_endpoint(data_dir, ports.pid),
        })
    }

    /// The WebSocket URL.
    pub fn ws_url(&self) -> String {
        format!("ws://127.0.0.1:{}/ws", self.port)
    }
}

/// The data directory: `VORN_DATA_DIR`, else `~/.vorn`.
pub fn data_dir() -> Result<PathBuf, FindError> {
    if let Some(dir) = std::env::var_os(DATA_DIR_ENV).filter(|d| !d.is_empty()) {
        return Ok(PathBuf::from(dir));
    }
    let home = std::env::var_os(if cfg!(windows) { "USERPROFILE" } else { "HOME" })
        .filter(|h| !h.is_empty())
        .ok_or(FindError::NoHome)?;
    Ok(PathBuf::from(home).join(".vorn"))
}

/// vornd's grid endpoint for process `pid` serving `data_dir`; one per
/// process, so a vornd handing over never takes another's.
#[cfg(unix)]
pub fn grid_endpoint(data_dir: &Path, pid: u32) -> String {
    data_dir
        .join("run")
        .join(format!("vornd-grid-{pid}.sock"))
        .to_string_lossy()
        .into_owned()
}

/// vornd's grid endpoint for process `pid`: a pipe named for the user, so
/// another account's vornd is never reached.
#[cfg(windows)]
pub fn grid_endpoint(_data_dir: &Path, pid: u32) -> String {
    let sid = sid::current_user().unwrap_or_else(|_| "user".into());
    format!(r"\\.\pipe\vorn-grid-{sid}-{pid}")
}

#[cfg(windows)]
mod sid {
    use std::io;
    use std::ptr;

    use windows_sys::Win32::Foundation::{CloseHandle, LocalFree, HANDLE};
    use windows_sys::Win32::Security::Authorization::ConvertSidToStringSidW;
    use windows_sys::Win32::Security::{GetTokenInformation, TokenUser, TOKEN_QUERY, TOKEN_USER};
    use windows_sys::Win32::System::Threading::{GetCurrentProcess, OpenProcessToken};

    /// The current user's SID as a string, `S-1-5-21-…`, as vornd names its pipe.
    pub fn current_user() -> io::Result<String> {
        // SAFETY: the second GetTokenInformation gets a buffer of the size the
        // first asked for; the token handle and the SID string are released.
        unsafe {
            let mut token: HANDLE = ptr::null_mut();
            if OpenProcessToken(GetCurrentProcess(), TOKEN_QUERY, &mut token) == 0 {
                return Err(io::Error::last_os_error());
            }
            let mut len = 0u32;
            GetTokenInformation(token, TokenUser, ptr::null_mut(), 0, &mut len);
            // u64s keep the buffer aligned for TOKEN_USER.
            let mut buf = vec![0u64; (len as usize).div_ceil(8)];
            let ok = GetTokenInformation(token, TokenUser, buf.as_mut_ptr().cast(), len, &mut len);
            CloseHandle(token);
            if ok == 0 {
                return Err(io::Error::last_os_error());
            }
            let user = &*buf.as_ptr().cast::<TOKEN_USER>();
            let mut s: *mut u16 = ptr::null_mut();
            if ConvertSidToStringSidW(user.User.Sid, &mut s) == 0 {
                return Err(io::Error::last_os_error());
            }
            let n = (0..).take_while(|&i| *s.add(i) != 0).count();
            let out = String::from_utf16_lossy(std::slice::from_raw_parts(s, n));
            LocalFree(s.cast());
            Ok(out)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn dir(name: &str) -> PathBuf {
        let d = std::env::temp_dir().join(format!("vorn-app-ep-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&d);
        std::fs::create_dir_all(&d).unwrap();
        d
    }

    #[test]
    fn reads_what_vornd_writes() {
        let d = dir("ok");
        std::fs::write(d.join("ws-port"), r#"{"port":4123,"pid":77}"#).unwrap();
        std::fs::write(d.join("local-token"), "tok\n").unwrap();
        let ep = Endpoint::find(&d).unwrap();
        let _ = std::fs::remove_dir_all(&d);
        assert_eq!((ep.port, ep.pid, ep.token.as_str()), (4123, 77, "tok"));
        assert_eq!(ep.ws_url(), "ws://127.0.0.1:4123/ws");
        assert!(
            ep.grid.contains("vorn") && ep.grid.contains("-77"),
            "{}",
            ep.grid
        );
        assert!(!format!("{ep:?}").contains("tok"));
    }

    #[test]
    fn says_which_file_is_wrong() {
        let d = dir("bad");
        assert!(matches!(Endpoint::find(&d), Err(FindError::Read(..))));
        std::fs::write(d.join("ws-port"), "4123").unwrap();
        assert!(matches!(Endpoint::find(&d), Err(FindError::BadPortFile(_))));
        std::fs::write(d.join("ws-port"), r#"{"port":1,"pid":2}"#).unwrap();
        std::fs::write(d.join("local-token"), " \n").unwrap();
        let got = Endpoint::find(&d);
        let _ = std::fs::remove_dir_all(&d);
        assert!(matches!(got, Err(FindError::NoToken)));
    }

    #[cfg(unix)]
    #[test]
    fn grid_socket_is_under_run() {
        assert_eq!(
            grid_endpoint(Path::new("/d"), 9),
            "/d/run/vornd-grid-9.sock"
        );
    }
}

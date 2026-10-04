//! The user-only endpoint on each OS (RC §5): a Unix socket in a 0700
//! directory, mode 0600, with the peer's UID checked; or a named pipe whose
//! DACL grants only the current user, refusing remote clients.

#[cfg(unix)]
pub use unix::*;
#[cfg(windows)]
pub use windows::*;

#[cfg(unix)]
mod unix {
    use std::fs;
    use std::io;
    use std::os::unix::fs::{DirBuilderExt, PermissionsExt};
    use std::path::{Path, PathBuf};

    pub type Stream = tokio::net::UnixStream;

    pub struct Listener {
        inner: tokio::net::UnixListener,
        path: PathBuf,
    }

    impl Listener {
        pub fn bind(home: &Path, endpoint: &str) -> io::Result<Listener> {
            let run = home.join("run");
            fs::DirBuilder::new()
                .recursive(true)
                .mode(0o700)
                .create(&run)?;
            // The directory may predate this build with looser bits.
            fs::set_permissions(&run, fs::Permissions::from_mode(0o700))?;
            let path = PathBuf::from(endpoint);
            match fs::remove_file(&path) {
                Err(e) if e.kind() != io::ErrorKind::NotFound => return Err(e),
                _ => {}
            }
            let inner = tokio::net::UnixListener::bind(&path)?;
            fs::set_permissions(&path, fs::Permissions::from_mode(0o600))?;
            Ok(Listener { inner, path })
        }

        /// The next connection from this user; anyone else is dropped.
        pub async fn accept(&mut self) -> io::Result<Stream> {
            loop {
                let (s, _) = self.inner.accept().await?;
                // SAFETY: geteuid has no preconditions.
                let me = unsafe { libc::geteuid() };
                if s.peer_cred().is_ok_and(|c| c.uid() == me) {
                    return Ok(s);
                }
            }
        }

        pub fn close(self) {
            let _ = fs::remove_file(&self.path);
        }
    }

    pub async fn connect(endpoint: &str) -> io::Result<Stream> {
        tokio::net::UnixStream::connect(endpoint).await
    }
}

#[cfg(windows)]
mod windows {
    use std::ffi::c_void;
    use std::io;
    use std::path::Path;
    use std::ptr;

    use tokio::net::windows::named_pipe::{ClientOptions, NamedPipeServer, ServerOptions};
    use windows_sys::Win32::Foundation::{CloseHandle, LocalFree, HANDLE};
    use windows_sys::Win32::Security::Authorization::{
        ConvertSidToStringSidW, ConvertStringSecurityDescriptorToSecurityDescriptorW,
        SDDL_REVISION_1,
    };
    use windows_sys::Win32::Security::{
        GetTokenInformation, TokenUser, PSECURITY_DESCRIPTOR, SECURITY_ATTRIBUTES, TOKEN_QUERY,
        TOKEN_USER,
    };
    use windows_sys::Win32::System::Threading::{GetCurrentProcess, OpenProcessToken};

    pub type Stream = NamedPipeServer;

    /// The current user's SID as a string, `S-1-5-21-…`.
    pub fn user_sid() -> io::Result<String> {
        // SAFETY: each call gets buffers of the size the previous one asked
        // for, and every handle and allocation is released below.
        unsafe {
            let mut token: HANDLE = ptr::null_mut();
            if OpenProcessToken(GetCurrentProcess(), TOKEN_QUERY, &mut token) == 0 {
                return Err(io::Error::last_os_error());
            }
            let mut len = 0u32;
            GetTokenInformation(token, TokenUser, ptr::null_mut(), 0, &mut len);
            let mut buf = vec![0u8; len as usize];
            let ok = GetTokenInformation(token, TokenUser, buf.as_mut_ptr().cast(), len, &mut len);
            CloseHandle(token);
            if ok == 0 {
                return Err(io::Error::last_os_error());
            }
            let user = &*(buf.as_ptr() as *const TOKEN_USER);
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

    /// A security descriptor granting the current user everything and
    /// nobody else anything.
    struct UserOnly {
        sd: PSECURITY_DESCRIPTOR,
    }

    impl UserOnly {
        fn new() -> io::Result<UserOnly> {
            let sddl: Vec<u16> = format!("D:P(A;;GA;;;{})", user_sid()?)
                .encode_utf16()
                .chain([0])
                .collect();
            let mut sd: PSECURITY_DESCRIPTOR = ptr::null_mut();
            // SAFETY: sddl is NUL-terminated; sd is freed in Drop.
            let ok = unsafe {
                ConvertStringSecurityDescriptorToSecurityDescriptorW(
                    sddl.as_ptr(),
                    SDDL_REVISION_1,
                    &mut sd,
                    ptr::null_mut(),
                )
            };
            if ok == 0 {
                return Err(io::Error::last_os_error());
            }
            Ok(UserOnly { sd })
        }

        fn create(&self, name: &str, first: bool) -> io::Result<NamedPipeServer> {
            let mut sa = SECURITY_ATTRIBUTES {
                nLength: std::mem::size_of::<SECURITY_ATTRIBUTES>() as u32,
                lpSecurityDescriptor: self.sd,
                bInheritHandle: 0,
            };
            // SAFETY: sa and the descriptor it points to outlive the call.
            unsafe {
                ServerOptions::new()
                    .first_pipe_instance(first)
                    .reject_remote_clients(true)
                    .create_with_security_attributes_raw(name, &mut sa as *mut _ as *mut c_void)
            }
        }
    }

    impl Drop for UserOnly {
        fn drop(&mut self) {
            // SAFETY: allocated by ConvertStringSecurityDescriptorToSecurityDescriptorW.
            unsafe { LocalFree(self.sd.cast()) };
        }
    }

    // The descriptor is only read after creation.
    unsafe impl Send for UserOnly {}
    unsafe impl Sync for UserOnly {}

    pub struct Listener {
        name: String,
        sd: UserOnly,
        current: NamedPipeServer,
    }

    impl Listener {
        pub fn bind(_home: &Path, endpoint: &str) -> io::Result<Listener> {
            let sd = UserOnly::new()?;
            let current = sd.create(endpoint, true)?;
            Ok(Listener {
                name: endpoint.to_owned(),
                sd,
                current,
            })
        }

        /// The next connection. Cancelling it leaves the waiting instance in place.
        pub async fn accept(&mut self) -> io::Result<Stream> {
            self.current.connect().await?;
            let next = self.sd.create(&self.name, false)?;
            Ok(std::mem::replace(&mut self.current, next))
        }

        pub fn close(self) {}
    }

    pub async fn connect(
        endpoint: &str,
    ) -> io::Result<tokio::net::windows::named_pipe::NamedPipeClient> {
        ClientOptions::new().open(endpoint)
    }
}

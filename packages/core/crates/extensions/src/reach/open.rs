//! Opening what [`Places::admit`](super::Places::admit) checked, never by
//! its name again: the resolved path is walked from the root one folder at
//! a time without following a link (on Windows, opened as a reparse point
//! and its final path checked), so a link swapped in after the check is
//! refused rather than followed, and the open file is what gets read,
//! written or listed.

use std::ffi::OsString;
use std::fs::{File, Metadata};
use std::io;
use std::path::Path;

use super::Access;

/// Why an open failed: the path changed into a link after it was checked,
/// or the disk said no.
#[derive(Debug)]
pub(super) enum Failed {
    Link,
    Io(io::Error),
}

impl From<io::Error> for Failed {
    fn from(e: io::Error) -> Self {
        Failed::Io(e)
    }
}

fn not_file_or_folder() -> Failed {
    Failed::Io(io::Error::new(
        io::ErrorKind::InvalidInput,
        "not a file or folder",
    ))
}

/// How many names the open `file` has: more than one means hard links.
pub(super) fn names(meta: &Metadata, file: &File) -> io::Result<u64> {
    #[cfg(unix)]
    {
        use std::os::unix::fs::MetadataExt;
        let _ = file;
        Ok(meta.nlink())
    }
    #[cfg(windows)]
    {
        let _ = meta;
        windows::info(file).map(|i| u64::from(i.nNumberOfLinks))
    }
}

#[cfg(unix)]
pub(super) use unix::{entries, open};
#[cfg(windows)]
pub(super) use windows::{entries, open};

#[cfg(unix)]
mod unix {
    use super::*;
    use std::ffi::OsStr;
    use std::os::fd::OwnedFd;
    use std::os::unix::ffi::OsStrExt;
    use std::path::Component;

    use rustix::fs::{openat, statat, AtFlags, Dir, FileType, Mode, OFlags, CWD};
    use rustix::io::Errno;

    const FOLDER: OFlags = OFlags::RDONLY
        .union(OFlags::DIRECTORY)
        .union(OFlags::NOFOLLOW)
        .union(OFlags::CLOEXEC);

    fn is_link(dir: &OwnedFd, name: &OsStr) -> bool {
        statat(dir, name, AtFlags::SYMLINK_NOFOLLOW)
            .is_ok_and(|s| FileType::from_raw_mode(s.st_mode) == FileType::Symlink)
    }

    /// A failed open of `name` in `dir`: a link where the check saw none is [`Failed::Link`].
    fn failed(dir: &OwnedFd, name: &OsStr, e: Errno) -> Failed {
        if e == Errno::LOOP || (e == Errno::NOTDIR && is_link(dir, name)) {
            Failed::Link
        } else {
            Failed::Io(e.into())
        }
    }

    /// Opens the resolved `real` for `access`, walking from `/` with each
    /// folder opened in the one before and none through a link. A write
    /// creates the file; it is not emptied here.
    pub(in crate::reach) fn open(real: &Path, access: Access) -> Result<File, Failed> {
        let names: Vec<&OsStr> = real
            .components()
            .filter_map(|c| match c {
                Component::Normal(n) => Some(n),
                _ => None,
            })
            .collect();
        let mut dir = openat(CWD, "/", FOLDER, Mode::empty()).map_err(io::Error::from)?;
        let Some((last, folders)) = names.split_last() else {
            return match access {
                Access::Look => Ok(File::from(dir)),
                _ => Err(not_file_or_folder()),
            };
        };
        for name in folders {
            dir = openat(&dir, *name, FOLDER, Mode::empty()).map_err(|e| failed(&dir, name, e))?;
        }
        match statat(&dir, *last, AtFlags::SYMLINK_NOFOLLOW) {
            Ok(st) => match FileType::from_raw_mode(st.st_mode) {
                FileType::Symlink => return Err(Failed::Link),
                FileType::RegularFile | FileType::Directory => {}
                _ => return Err(not_file_or_folder()),
            },
            Err(Errno::NOENT) if access == Access::Write => {}
            Err(e) => return Err(Failed::Io(e.into())),
        }
        // Non-blocking, so a pipe swapped in after the look above cannot hold the call.
        let base = OFlags::NOFOLLOW | OFlags::CLOEXEC | OFlags::NOCTTY | OFlags::NONBLOCK;
        let flags = match access {
            Access::Look | Access::Read => base | OFlags::RDONLY,
            Access::Write => base | OFlags::WRONLY | OFlags::CREATE,
        };
        let fd = openat(&dir, *last, flags, Mode::from_raw_mode(0o666))
            .map_err(|e| failed(&dir, last, e))?;
        let file = File::from(fd);
        let kind = file.metadata()?.file_type();
        if !(kind.is_file() || kind.is_dir()) {
            return Err(not_file_or_folder());
        }
        Ok(file)
    }

    /// The entries of the open folder `file`, each name with whether it is
    /// a folder (a link to one is not), read through the open folder.
    pub(in crate::reach) fn entries(
        file: &File,
        _real: &Path,
    ) -> io::Result<Vec<(OsString, bool)>> {
        let mut out = Vec::new();
        for entry in Dir::read_from(file)? {
            let entry = entry?;
            let name = entry.file_name().to_bytes();
            if name == b"." || name == b".." {
                continue;
            }
            let name = OsStr::from_bytes(name);
            let is_dir = match entry.file_type() {
                FileType::Unknown => statat(file, name, AtFlags::SYMLINK_NOFOLLOW)
                    .is_ok_and(|s| FileType::from_raw_mode(s.st_mode) == FileType::Directory),
                kind => kind == FileType::Directory,
            };
            out.push((name.to_os_string(), is_dir));
        }
        Ok(out)
    }
}

#[cfg(windows)]
mod windows {
    use super::*;
    use std::os::windows::ffi::OsStringExt;
    use std::os::windows::fs::{MetadataExt, OpenOptionsExt};
    use std::os::windows::io::AsRawHandle;

    use windows_sys::Win32::Storage::FileSystem::{
        GetFileInformationByHandle, GetFinalPathNameByHandleW, BY_HANDLE_FILE_INFORMATION,
        FILE_ATTRIBUTE_REPARSE_POINT, FILE_FLAG_BACKUP_SEMANTICS, FILE_FLAG_OPEN_REPARSE_POINT,
        FILE_SHARE_READ, FILE_SHARE_WRITE, VOLUME_NAME_DOS,
    };

    pub(super) fn info(file: &File) -> io::Result<BY_HANDLE_FILE_INFORMATION> {
        // SAFETY: BY_HANDLE_FILE_INFORMATION is plain data, valid all zero.
        let mut info: BY_HANDLE_FILE_INFORMATION = unsafe { std::mem::zeroed() };
        // SAFETY: the handle stays open while `file` is borrowed; `info` is writable.
        if unsafe { GetFileInformationByHandle(file.as_raw_handle(), &mut info) } == 0 {
            return Err(io::Error::last_os_error());
        }
        Ok(info)
    }

    /// The path the open `file` has, as the disk names it (`\\?\C:\…`).
    fn final_path(file: &File) -> io::Result<OsString> {
        let mut buf = vec![0u16; 512];
        loop {
            let len = u32::try_from(buf.len()).unwrap_or(u32::MAX);
            // SAFETY: the handle stays open while `file` is borrowed; `buf` holds `len` u16s.
            let n = unsafe {
                GetFinalPathNameByHandleW(
                    file.as_raw_handle(),
                    buf.as_mut_ptr(),
                    len,
                    VOLUME_NAME_DOS,
                )
            } as usize;
            if n == 0 {
                return Err(io::Error::last_os_error());
            }
            if n < buf.len() {
                return Ok(OsString::from_wide(&buf[..n]));
            }
            buf.resize(n + 1, 0);
        }
    }

    /// Opens the resolved `real` for `access` as a reparse point, never
    /// through one, and refuses it unless the handle's own path is `real`:
    /// a junction or link swapped in on the way changes that path. Held
    /// without delete sharing, so it is not renamed while open. A write
    /// creates the file; it is not emptied here.
    pub(in crate::reach) fn open(real: &Path, access: Access) -> Result<File, Failed> {
        let mut options = std::fs::OpenOptions::new();
        match access {
            Access::Look | Access::Read => options.read(true),
            Access::Write => options.write(true).create(true),
        };
        options
            .custom_flags(FILE_FLAG_OPEN_REPARSE_POINT | FILE_FLAG_BACKUP_SEMANTICS)
            .share_mode(FILE_SHARE_READ | FILE_SHARE_WRITE);
        let file = options.open(real)?;
        let meta = file.metadata()?;
        if meta.file_attributes() & FILE_ATTRIBUTE_REPARSE_POINT != 0 {
            return Err(Failed::Link);
        }
        let lower = |s: &std::ffi::OsStr| s.to_string_lossy().to_lowercase();
        if lower(&final_path(&file)?) != lower(real.as_os_str()) {
            return Err(Failed::Link);
        }
        if !(meta.is_file() || meta.is_dir()) {
            return Err(not_file_or_folder());
        }
        Ok(file)
    }

    /// The entries of the folder at `real`, held open as `_file` so it is
    /// not renamed or replaced while they are read.
    pub(in crate::reach) fn entries(
        _file: &File,
        real: &Path,
    ) -> io::Result<Vec<(OsString, bool)>> {
        let mut out = Vec::new();
        for entry in std::fs::read_dir(real)? {
            let entry = entry?;
            let is_dir = entry.file_type().is_ok_and(|t| t.is_dir());
            out.push((entry.file_name(), is_dir));
        }
        Ok(out)
    }
}

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
use std::time::SystemTime;

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

/// What a look at a path, without opening it, tells.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Looked {
    /// A file, rather than a folder.
    pub is_file: bool,
    pub len: u64,
    pub modified: Option<SystemTime>,
}

/// Which file a name is: its volume and its number there. Two names with
/// one identity are hard links to one file.
pub(super) type Identity = (u64, u64);

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

/// The identity of the open `file`.
pub(super) fn identity(meta: &Metadata, file: &File) -> io::Result<Identity> {
    #[cfg(unix)]
    {
        use std::os::unix::fs::MetadataExt;
        let _ = file;
        Ok((meta.dev(), meta.ino()))
    }
    #[cfg(windows)]
    {
        let _ = meta;
        windows::info(file).map(|i| windows::identity_of(&i))
    }
}

/// The identity of what `path` names, not following a link; `None` when it
/// cannot be told.
pub(super) fn identity_at(path: &Path) -> Option<Identity> {
    #[cfg(unix)]
    {
        use std::os::unix::fs::MetadataExt;
        let m = std::fs::symlink_metadata(path).ok()?;
        Some((m.dev(), m.ino()))
    }
    #[cfg(windows)]
    {
        windows::identity_at(path)
    }
}

#[cfg(unix)]
pub(super) use unix::{entries, open, stat};
#[cfg(windows)]
pub(super) use windows::{entries, open, stat};

#[cfg(unix)]
mod unix {
    use super::*;
    use std::ffi::OsStr;
    use std::os::fd::{AsFd, OwnedFd};
    use std::os::unix::ffi::OsStrExt;
    use std::path::Component;
    use std::time::{Duration, UNIX_EPOCH};

    use rustix::fs::{fstat, openat, statat, AtFlags, Dir, FileType, Mode, OFlags, Stat, CWD};
    use rustix::io::Errno;

    const WALK: OFlags = OFlags::DIRECTORY
        .union(OFlags::NOFOLLOW)
        .union(OFlags::CLOEXEC);

    /// Opens folder `name` in `dir` to go through it, needing only search
    /// permission where the system can open a folder so.
    fn through(dir: impl AsFd, name: &OsStr) -> Result<OwnedFd, Errno> {
        #[cfg(any(target_os = "linux", target_os = "android"))]
        {
            openat(dir, name, OFlags::PATH | WALK, Mode::empty())
        }
        #[cfg(target_vendor = "apple")]
        {
            // O_EXEC, which on a folder is O_SEARCH; a system without it says EINVAL.
            const EXEC: OFlags = OFlags::from_bits_retain(0x4000_0000);
            match openat(&dir, name, EXEC | WALK, Mode::empty()) {
                Err(Errno::INVAL) => openat(dir, name, OFlags::RDONLY | WALK, Mode::empty()),
                opened => opened,
            }
        }
        #[cfg(not(any(target_os = "linux", target_os = "android", target_vendor = "apple")))]
        {
            openat(dir, name, OFlags::RDONLY | WALK, Mode::empty())
        }
    }

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

    /// The folder holding `real`'s last part, walked to from `/` with each
    /// folder opened in the one before and none through a link, and that
    /// part; `None` for `/` itself.
    fn parent(real: &Path) -> Result<(OwnedFd, Option<&OsStr>), Failed> {
        let names: Vec<&OsStr> = real
            .components()
            .filter_map(|c| match c {
                Component::Normal(n) => Some(n),
                _ => None,
            })
            .collect();
        let mut dir = through(CWD, OsStr::new("/")).map_err(io::Error::from)?;
        let Some((last, folders)) = names.split_last() else {
            return Ok((dir, None));
        };
        for name in folders {
            dir = through(&dir, name).map_err(|e| failed(&dir, name, e))?;
        }
        Ok((dir, Some(*last)))
    }

    /// Opens the resolved `real` for `access` in the folder [`parent`]
    /// opens. A write creates the file; it is not emptied here.
    pub(in crate::reach) fn open(real: &Path, access: Access) -> Result<File, Failed> {
        let (dir, last) = parent(real)?;
        let Some(last) = last else {
            return match access {
                Access::Look => Ok(File::from(
                    openat(CWD, "/", OFlags::RDONLY | WALK, Mode::empty())
                        .map_err(io::Error::from)?,
                )),
                _ => Err(not_file_or_folder()),
            };
        };
        match statat(&dir, last, AtFlags::SYMLINK_NOFOLLOW) {
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
        let fd = openat(&dir, last, flags, Mode::from_raw_mode(0o666))
            .map_err(|e| failed(&dir, last, e))?;
        let file = File::from(fd);
        let kind = file.metadata()?.file_type();
        if !(kind.is_file() || kind.is_dir()) {
            return Err(not_file_or_folder());
        }
        Ok(file)
    }

    /// Looks at the resolved `real` in the folder [`parent`] opens, never
    /// opening it, so a file only its owner may write still tells its size.
    pub(in crate::reach) fn stat(real: &Path) -> Result<Looked, Failed> {
        let (dir, last) = parent(real)?;
        let st = match last {
            Some(last) => statat(&dir, last, AtFlags::SYMLINK_NOFOLLOW),
            None => fstat(&dir),
        }
        .map_err(io::Error::from)?;
        let is_file = match FileType::from_raw_mode(st.st_mode) {
            FileType::Symlink => return Err(Failed::Link),
            FileType::RegularFile => true,
            FileType::Directory => false,
            _ => return Err(not_file_or_folder()),
        };
        Ok(Looked {
            is_file,
            len: u64::try_from(st.st_size).unwrap_or(0),
            modified: modified(&st),
        })
    }

    #[allow(clippy::unnecessary_cast)] // The fields' types differ by system.
    fn modified(st: &Stat) -> Option<SystemTime> {
        let (secs, nanos) = (st.st_mtime as i64, st.st_mtime_nsec as i64);
        let nanos = Duration::from_nanos(u64::try_from(nanos).ok()?);
        match u64::try_from(secs) {
            Ok(secs) => UNIX_EPOCH.checked_add(Duration::from_secs(secs) + nanos),
            Err(_) => UNIX_EPOCH
                .checked_sub(Duration::from_secs(secs.unsigned_abs()))?
                .checked_add(nanos),
        }
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
    use std::os::windows::ffi::{OsStrExt, OsStringExt};
    use std::os::windows::fs::OpenOptionsExt;
    use std::os::windows::io::{AsRawHandle, FromRawHandle};

    use windows_sys::Wdk::Foundation::OBJECT_ATTRIBUTES;
    use windows_sys::Wdk::Storage::FileSystem::{
        NtCreateFile, FILE_NON_DIRECTORY_FILE, FILE_OPEN_IF, FILE_OPEN_REPARSE_POINT as OPEN_LINK,
        FILE_SYNCHRONOUS_IO_NONALERT,
    };
    use windows_sys::Win32::Foundation::{
        RtlNtStatusToDosError, OBJ_CASE_INSENSITIVE, UNICODE_STRING,
    };
    use windows_sys::Win32::Storage::FileSystem::{
        FileAttributeTagInfo, GetFileInformationByHandle, GetFileInformationByHandleEx,
        GetFinalPathNameByHandleW, BY_HANDLE_FILE_INFORMATION, FILE_ATTRIBUTE_NORMAL,
        FILE_ATTRIBUTE_REPARSE_POINT, FILE_ATTRIBUTE_TAG_INFO, FILE_FLAG_BACKUP_SEMANTICS,
        FILE_FLAG_OPEN_REPARSE_POINT, FILE_GENERIC_READ, FILE_GENERIC_WRITE, FILE_READ_ATTRIBUTES,
        FILE_SHARE_DELETE, FILE_SHARE_READ, FILE_SHARE_WRITE, FILE_TRAVERSE, SYNCHRONIZE,
        VOLUME_NAME_DOS,
    };
    use windows_sys::Win32::System::SystemServices::{
        IO_REPARSE_TAG_MOUNT_POINT, IO_REPARSE_TAG_SYMLINK,
    };
    use windows_sys::Win32::System::IO::IO_STATUS_BLOCK;

    pub(super) fn info(file: &File) -> io::Result<BY_HANDLE_FILE_INFORMATION> {
        // SAFETY: BY_HANDLE_FILE_INFORMATION is plain data, valid all zero.
        let mut info: BY_HANDLE_FILE_INFORMATION = unsafe { std::mem::zeroed() };
        // SAFETY: the handle stays open while `file` is borrowed; `info` is writable.
        if unsafe { GetFileInformationByHandle(file.as_raw_handle(), &mut info) } == 0 {
            return Err(io::Error::last_os_error());
        }
        Ok(info)
    }

    pub(super) fn identity_of(i: &BY_HANDLE_FILE_INFORMATION) -> Identity {
        (
            u64::from(i.dwVolumeSerialNumber),
            (u64::from(i.nFileIndexHigh) << 32) | u64::from(i.nFileIndexLow),
        )
    }

    pub(super) fn identity_at(path: &Path) -> Option<Identity> {
        let file = std::fs::OpenOptions::new()
            .access_mode(FILE_READ_ATTRIBUTES)
            .share_mode(FILE_SHARE_READ | FILE_SHARE_WRITE | FILE_SHARE_DELETE)
            .custom_flags(FILE_FLAG_OPEN_REPARSE_POINT | FILE_FLAG_BACKUP_SEMANTICS)
            .open(path)
            .ok()?;
        info(&file).ok().map(|i| identity_of(&i))
    }

    /// What a reparse point is to an open.
    #[derive(Debug, PartialEq, Eq)]
    enum Reparse {
        /// Not one.
        No,
        /// A symbolic link or junction, which leads elsewhere.
        Link,
        /// Data a filter serves in place: a cloud file, a deduplicated one.
        InPlace,
    }

    /// What a file with `attributes` and reparse `tag` is, told as the
    /// standard library tells a link: only a symbolic link or a junction
    /// leads elsewhere.
    fn reparse(attributes: u32, tag: u32) -> Reparse {
        if attributes & FILE_ATTRIBUTE_REPARSE_POINT == 0 {
            Reparse::No
        } else if tag == IO_REPARSE_TAG_SYMLINK || tag == IO_REPARSE_TAG_MOUNT_POINT {
            Reparse::Link
        } else {
            Reparse::InPlace
        }
    }

    fn tag_info(file: &File) -> io::Result<FILE_ATTRIBUTE_TAG_INFO> {
        let mut info = FILE_ATTRIBUTE_TAG_INFO::default();
        // SAFETY: the handle stays open while `file` is borrowed; `info` is writable and sized.
        let ok = unsafe {
            GetFileInformationByHandleEx(
                file.as_raw_handle(),
                FileAttributeTagInfo,
                (&mut info as *mut FILE_ATTRIBUTE_TAG_INFO).cast(),
                std::mem::size_of::<FILE_ATTRIBUTE_TAG_INFO>() as u32,
            )
        };
        if ok == 0 {
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

    /// Opens with `access`, held without delete sharing so it is not renamed
    /// while open; as the reparse point itself unless `follow`.
    fn opener(access: u32, follow: bool) -> std::fs::OpenOptions {
        let mut options = std::fs::OpenOptions::new();
        let flags = if follow {
            FILE_FLAG_BACKUP_SEMANTICS
        } else {
            FILE_FLAG_BACKUP_SEMANTICS | FILE_FLAG_OPEN_REPARSE_POINT
        };
        options
            .access_mode(access)
            .share_mode(FILE_SHARE_READ | FILE_SHARE_WRITE)
            .custom_flags(flags);
        options
    }

    /// `first`, opened as a reparse point, kept unless it is a link; a cloud
    /// or deduplicated file is opened again through its filter. Refused
    /// unless the handle's own path is `real`: a junction or link swapped in
    /// on the way changes that path.
    fn settle(first: File, real: &Path, access: u32) -> Result<File, Failed> {
        let tag = tag_info(&first)?;
        let file = match reparse(tag.FileAttributes, tag.ReparseTag) {
            Reparse::Link => return Err(Failed::Link),
            Reparse::No => first,
            // `first` is held until this open, so it is not replaced between them.
            Reparse::InPlace => opener(access, true).open(real)?,
        };
        let lower = |s: &std::ffi::OsStr| s.to_string_lossy().to_lowercase();
        if lower(&final_path(&file)?) != lower(real.as_os_str()) {
            return Err(Failed::Link);
        }
        let meta = file.metadata()?;
        if !(meta.is_file() || meta.is_dir()) {
            return Err(not_file_or_folder());
        }
        Ok(file)
    }

    fn mask(access: Access) -> u32 {
        match access {
            Access::Look => FILE_READ_ATTRIBUTES | SYNCHRONIZE,
            Access::Read => FILE_GENERIC_READ,
            Access::Write => FILE_GENERIC_WRITE | FILE_READ_ATTRIBUTES,
        }
    }

    /// Opens the resolved `real` for `access` ([`settle`]). A write creates
    /// the file; it is not emptied here.
    pub(in crate::reach) fn open(real: &Path, access: Access) -> Result<File, Failed> {
        let first = match access {
            Access::Write => create(real)?,
            _ => opener(mask(access), false).open(real)?,
        };
        settle(first, real, mask(access))
    }

    /// Looks at the resolved `real` through a handle that may only read its
    /// attributes, so a file only its owner may write still tells its size.
    pub(in crate::reach) fn stat(real: &Path) -> Result<Looked, Failed> {
        let meta = open(real, Access::Look)?.metadata()?;
        Ok(Looked {
            is_file: meta.is_file(),
            len: meta.len(),
            modified: meta.modified().ok(),
        })
    }

    /// `real` opened for writing as a reparse point, created when missing,
    /// relative to its folder opened and checked first: a junction swapped
    /// in for a folder on the way is never created through.
    fn create(real: &Path) -> Result<File, Failed> {
        let (Some(folder), Some(name)) = (real.parent(), real.file_name()) else {
            return Err(not_file_or_folder());
        };
        let through = FILE_TRAVERSE | FILE_READ_ATTRIBUTES | SYNCHRONIZE;
        let parent = settle(opener(through, false).open(folder)?, folder, through)?;
        if !parent.metadata()?.is_dir() {
            return Err(not_file_or_folder());
        }
        let wide: Vec<u16> = name.encode_wide().collect();
        let bytes = u16::try_from(wide.len() * 2)
            .map_err(|_| io::Error::new(io::ErrorKind::InvalidInput, "name too long"))?;
        let name = UNICODE_STRING {
            Length: bytes,
            MaximumLength: bytes,
            Buffer: wide.as_ptr().cast_mut(),
        };
        let attributes = OBJECT_ATTRIBUTES {
            Length: std::mem::size_of::<OBJECT_ATTRIBUTES>() as u32,
            RootDirectory: parent.as_raw_handle(),
            ObjectName: &name,
            Attributes: OBJ_CASE_INSENSITIVE,
            ..Default::default()
        };
        let mut handle = std::ptr::null_mut();
        let mut status = IO_STATUS_BLOCK::default();
        // SAFETY: every pointer is to a live local (`wide` outlives the call),
        // and `parent` stays open across it.
        let rc = unsafe {
            NtCreateFile(
                &mut handle,
                mask(Access::Write),
                &attributes,
                &mut status,
                std::ptr::null(),
                FILE_ATTRIBUTE_NORMAL,
                FILE_SHARE_READ | FILE_SHARE_WRITE,
                FILE_OPEN_IF,
                FILE_NON_DIRECTORY_FILE | FILE_SYNCHRONOUS_IO_NONALERT | OPEN_LINK,
                std::ptr::null(),
                0,
            )
        };
        if rc < 0 {
            // SAFETY: a plain lookup of a status code.
            let code = unsafe { RtlNtStatusToDosError(rc) };
            return Err(io::Error::from_raw_os_error(code as i32).into());
        }
        // SAFETY: NtCreateFile succeeded, so `handle` is an open handle owned here.
        Ok(unsafe { File::from_raw_handle(handle) })
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

    #[cfg(test)]
    mod tests {
        use super::*;

        #[test]
        fn only_a_symbolic_link_or_junction_leads_elsewhere() {
            const CLOUD: u32 = 0x9000_601A;
            const DEDUP: u32 = 0x8000_0013;
            let r = FILE_ATTRIBUTE_REPARSE_POINT;
            assert_eq!(reparse(0, IO_REPARSE_TAG_SYMLINK), Reparse::No);
            assert_eq!(reparse(r, IO_REPARSE_TAG_SYMLINK), Reparse::Link);
            assert_eq!(reparse(r, IO_REPARSE_TAG_MOUNT_POINT), Reparse::Link);
            assert_eq!(reparse(r, CLOUD), Reparse::InPlace);
            assert_eq!(reparse(r, DEDUP), Reparse::InPlace);
        }

        #[test]
        fn creates_a_file_in_its_checked_folder_and_tells_hard_links_alike() {
            let dir = tempfile::tempdir().unwrap();
            let real = std::fs::canonicalize(dir.path()).unwrap().join("new.txt");
            drop(open(&real, Access::Write).unwrap());
            assert!(real.is_file());
            let other = real.with_file_name("other.txt");
            std::fs::hard_link(&real, &other).unwrap();
            assert!(identity_at(&real).is_some());
            assert_eq!(identity_at(&real), identity_at(&other));
            let read = open(&other, Access::Read).unwrap();
            assert_eq!(names(&read.metadata().unwrap(), &read).unwrap(), 2);
            assert!(open(&real.join("no"), Access::Write).is_err());
        }
    }
}

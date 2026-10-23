//! A project's path resolved one folder at a time from `/`, each opened
//! without following a link. A link is read and followed where it is found,
//! so the folder it sat in is known from the same look that followed it:
//! nothing can change between resolving a path and checking how it got there.

use std::collections::VecDeque;
use std::ffi::{CString, OsStr, OsString};
use std::io;
use std::os::fd::{AsFd, AsRawFd, BorrowedFd, FromRawFd, OwnedFd};
use std::os::unix::ffi::{OsStrExt, OsStringExt};
use std::path::{Component, Path, PathBuf};

use super::Walked;

/// Links followed before giving up, as the kernel does.
const MAX_LINKS: usize = 40;

#[cfg(target_os = "linux")]
const DIR: libc::c_int = libc::O_PATH | libc::O_DIRECTORY | libc::O_NOFOLLOW | libc::O_CLOEXEC;
#[cfg(target_os = "macos")]
const DIR: libc::c_int = libc::O_RDONLY | libc::O_DIRECTORY | libc::O_NOFOLLOW | libc::O_CLOEXEC;

/// One step of a path: into a folder, or up out of one.
enum Step {
    Into(OsString),
    Up,
}

fn steps(path: &Path) -> impl DoubleEndedIterator<Item = Step> + '_ {
    path.components().filter_map(|c| match c {
        Component::Normal(name) => Some(Step::Into(name.to_owned())),
        Component::ParentDir => Some(Step::Up),
        Component::RootDir | Component::CurDir | Component::Prefix(_) => None,
    })
}

/// The folder at the absolute `path`: its real path, as the folder the walk
/// opened names itself, and the real folder each link on the way was in.
pub(super) fn walk(path: &Path) -> io::Result<Walked> {
    if !path.is_absolute() {
        return Err(io::Error::other("not an absolute path"));
    }
    let root = open_in(None, OsStr::new("/"))?;
    // The folders walked into below the root; `..` steps back out of one.
    let mut dirs: Vec<OwnedFd> = Vec::new();
    let mut rest: VecDeque<Step> = steps(path).collect();
    let mut links_in = Vec::new();
    while let Some(step) = rest.pop_front() {
        let name = match step {
            Step::Up => {
                dirs.pop();
                continue;
            }
            Step::Into(name) => name,
        };
        let here = dirs.last().unwrap_or(&root).as_fd();
        let err = match open_in(Some(here), &name) {
            Ok(fd) => {
                dirs.push(fd);
                continue;
            }
            Err(err) => err,
        };
        if !matches!(err.raw_os_error(), Some(libc::ELOOP | libc::ENOTDIR)) {
            return Err(err);
        }
        // Not a folder: a link to follow, or the walk ends here.
        let Ok(target) = read_link_in(here, &name) else {
            return Err(err);
        };
        links_in.push(fd_path(here)?);
        if links_in.len() > MAX_LINKS {
            return Err(io::Error::from_raw_os_error(libc::ELOOP));
        }
        let target = PathBuf::from(target);
        if target.is_absolute() {
            dirs.clear();
        }
        for step in steps(&target).rev() {
            rest.push_front(step);
        }
    }
    let real = fd_path(dirs.last().unwrap_or(&root).as_fd())?;
    Ok(Walked { real, links_in })
}

fn c_name(name: &OsStr) -> io::Result<CString> {
    CString::new(name.as_bytes()).map_err(|_| io::Error::other("a name with a NUL in it"))
}

/// The folder `name` in `dir` (or the absolute `name`), never through a link.
fn open_in(dir: Option<BorrowedFd<'_>>, name: &OsStr) -> io::Result<OwnedFd> {
    let c = c_name(name)?;
    let at = dir.map_or(libc::AT_FDCWD, |d| d.as_raw_fd());
    // SAFETY: `c` is NUL-terminated and outlives the call; `at` is open or AT_FDCWD.
    let fd = unsafe { libc::openat(at, c.as_ptr(), DIR) };
    if fd < 0 {
        return Err(io::Error::last_os_error());
    }
    // SAFETY: openat just returned this descriptor, and nothing else owns it.
    Ok(unsafe { OwnedFd::from_raw_fd(fd) })
}

/// What the link `name` in `dir` points to.
fn read_link_in(dir: BorrowedFd<'_>, name: &OsStr) -> io::Result<OsString> {
    let c = c_name(name)?;
    let mut buf = vec![0u8; libc::PATH_MAX as usize];
    // SAFETY: `c` is NUL-terminated, `buf` is writable for its length, `dir` is open.
    let n = unsafe {
        libc::readlinkat(
            dir.as_raw_fd(),
            c.as_ptr(),
            buf.as_mut_ptr().cast(),
            buf.len(),
        )
    };
    let n = usize::try_from(n).map_err(|_| io::Error::last_os_error())?;
    if n >= buf.len() {
        return Err(io::Error::from_raw_os_error(libc::ENAMETOOLONG));
    }
    buf.truncate(n);
    Ok(OsString::from_vec(buf))
}

/// The path the folder open at `fd` has now.
#[cfg(target_os = "linux")]
fn fd_path(fd: BorrowedFd<'_>) -> io::Result<PathBuf> {
    std::fs::read_link(format!("/proc/self/fd/{}", fd.as_raw_fd()))
}

/// The path the folder open at `fd` has now.
#[cfg(target_os = "macos")]
fn fd_path(fd: BorrowedFd<'_>) -> io::Result<PathBuf> {
    let mut buf = vec![0u8; libc::PATH_MAX as usize];
    // SAFETY: F_GETPATH writes at most PATH_MAX bytes, NUL-terminated, into `buf`.
    if unsafe { libc::fcntl(fd.as_raw_fd(), libc::F_GETPATH, buf.as_mut_ptr()) } < 0 {
        return Err(io::Error::last_os_error());
    }
    let end = buf.iter().position(|&b| b == 0).unwrap_or(buf.len());
    buf.truncate(end);
    Ok(PathBuf::from(OsString::from_vec(buf)))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::os::unix::fs::symlink;

    #[test]
    fn follows_each_link_and_says_where_it_was() {
        let root = tempfile::tempdir().unwrap();
        let root = std::fs::canonicalize(root.path()).unwrap();
        std::fs::create_dir_all(root.join("a/b")).unwrap();
        std::fs::create_dir(root.join("c")).unwrap();
        symlink(root.join("a"), root.join("c/to-a")).unwrap();
        symlink("../../c", root.join("a/b/up")).unwrap();

        let plain = walk(&root.join("a/b")).unwrap();
        assert_eq!(plain.real, root.join("a/b"));
        // Only links above `root` (the temporary folder's own, if any).
        assert!(plain.links_in.iter().all(|d| !d.starts_with(&root)));

        let linked = walk(&root.join("c/to-a/b/up/../a")).unwrap();
        assert_eq!(linked.real, root.join("a"));
        let ours: Vec<_> = linked
            .links_in
            .iter()
            .filter(|d| d.starts_with(&root))
            .collect();
        assert_eq!(ours, [&root.join("c"), &root.join("a/b")]);
    }

    #[test]
    fn refuses_what_is_not_a_folder() {
        let root = tempfile::tempdir().unwrap();
        std::fs::write(root.path().join("file"), "").unwrap();
        symlink(root.path().join("loop"), root.path().join("loop")).unwrap();
        assert!(walk(&root.path().join("file")).is_err());
        assert!(walk(&root.path().join("gone")).is_err());
        assert!(walk(&root.path().join("loop")).is_err());
        assert!(walk(Path::new("relative")).is_err());
    }
}

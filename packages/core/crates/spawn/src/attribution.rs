//! Which app macOS counts a process as.
//!
//! macOS holds one process responsible for each other process: the app it
//! started from. Privacy permissions (camera, Files and Folders,
//! Accessibility, and so on) are asked for and granted in that app's name,
//! so a program in a Vorn terminal shares Vorn's.
//!
//! Responsibility is inherited at spawn and never changes after. Once the
//! responsible app exits, macOS counts the process as responsible for itself,
//! and so is everything it starts from then on. vornd and vorn-sessiond
//! outlive the app that started them, so a later app finds them counted as
//! no app at all, and [`counts_as_another_app`] is how it notices.
//!
//! Elsewhere there is no such notion: every answer is "unknown".

/// The pid of the process macOS holds responsible for `pid`: the app it
/// counts as, or `pid` itself when it counts as no other. `None` off macOS,
/// for a process that is gone, or when the system cannot say.
pub fn responsible_for(pid: u32) -> Option<u32> {
    #[cfg(target_os = "macos")]
    {
        macos::responsible_for(pid)
    }
    #[cfg(not(target_os = "macos"))]
    {
        let _ = pid;
        None
    }
}

/// Whether macOS counts `other` as a different app from the one this process
/// counts as, so a program `other` starts asks for privacy permissions in
/// another name than one this process starts would. False whenever either
/// answer is unknown.
pub fn counts_as_another_app(other: u32) -> bool {
    let mine = responsible_for(std::process::id());
    let theirs = responsible_for(other);
    mine.is_some() && theirs.is_some() && mine != theirs
}

/// Whether this process counts as an app other than itself: one started by
/// an app, while that app runs.
pub fn counts_as_an_app() -> bool {
    let me = std::process::id();
    responsible_for(me).is_some_and(|r| r != me)
}

#[cfg(target_os = "macos")]
mod macos {
    use std::sync::OnceLock;

    type Get = unsafe extern "C" fn(libc::pid_t) -> libc::pid_t;

    /// `responsibility_get_pid_responsible_for_pid`, private to libSystem,
    /// so looked up at run time and done without when a macOS lacks it.
    fn get() -> Option<Get> {
        static F: OnceLock<Option<Get>> = OnceLock::new();
        *F.get_or_init(|| {
            // SAFETY: dlsym with a NUL-terminated name.
            let p = unsafe {
                libc::dlsym(
                    libc::RTLD_DEFAULT,
                    c"responsibility_get_pid_responsible_for_pid".as_ptr(),
                )
            };
            // SAFETY: the symbol is that function, which takes and returns a
            // pid in every macOS that has it.
            (!p.is_null()).then(|| unsafe { std::mem::transmute::<*mut libc::c_void, Get>(p) })
        })
    }

    pub fn responsible_for(pid: u32) -> Option<u32> {
        let pid = libc::pid_t::try_from(pid).ok()?;
        // SAFETY: the function takes any pid and answers -1 for one it does not know.
        let r = unsafe { get()?(pid) };
        u32::try_from(r).ok().filter(|&r| r > 0)
    }
}

#[cfg(all(test, unix))]
mod tests {
    use super::*;

    #[test]
    fn a_child_counts_as_the_same_app_as_its_parent() {
        let mut child = crate::command("sleep")
            .arg("30")
            .spawn()
            .expect("sleep starts");
        let me = std::process::id();
        if cfg!(target_os = "macos") {
            assert!(responsible_for(me).is_some());
            assert_eq!(responsible_for(child.id()), responsible_for(me));
        } else {
            assert_eq!(responsible_for(me), None);
        }
        assert!(!counts_as_another_app(child.id()));
        let _ = child.kill();
        let _ = child.wait();
    }

    #[test]
    fn nothing_is_known_of_a_process_that_is_gone() {
        let mut child = crate::command("true").spawn().expect("true starts");
        let pid = child.id();
        child.wait().expect("reaped");
        assert_eq!(responsible_for(pid), None);
        assert!(!counts_as_another_app(pid));
    }
}

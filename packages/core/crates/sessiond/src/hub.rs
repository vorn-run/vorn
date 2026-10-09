//! One thread that waits on every session's descriptors at once, on macOS
//! and Linux: epoll on Linux, kqueue on macOS.
//!
//! A thread per output stream, per input and per program cost three stacks
//! and three wakeups a session, which at thousands of mostly quiet sessions
//! is most of what sessiond does. Here each descriptor is registered once
//! and armed one-shot: it fires once, its handler reads or writes one
//! chunk, and the handler arms it again when it wants more. A session that
//! is frozen, or whose log is full, simply does not arm, so nothing spins;
//! and one read per wakeup takes turns fairly among busy sessions.
//!
//! Handlers run on the hub thread and must not block. They are called
//! outside the hub's own locks, so they may arm, register and drop freely.
//! Tokens are never reused, so an event for a descriptor dropped meanwhile
//! finds no handler and is ignored.
//!
//! Program exits are watched here too: a pidfd on Linux (from 5.3), an
//! `EVFILT_PROC` note on macOS. Timers stand in for the sleeps the threads
//! did (the drain after exit, retries for a full log, polling for an exit
//! another sessiond writes down).

use std::cmp::Reverse;
use std::collections::{BinaryHeap, HashMap};
use std::io;
use std::os::fd::{AsRawFd, FromRawFd, OwnedFd, RawFd};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex, MutexGuard, OnceLock};
use std::time::{Duration, Instant};

/// What a registered descriptor waits for.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Dir {
    Read,
    Write,
}

/// Called on the hub thread when its descriptor is ready.
pub(crate) type Handler = Arc<dyn Fn() + Send + Sync>;
type Once = Box<dyn FnOnce() + Send>;

enum Entry {
    Io(Handler),
    /// A program's exit; the pidfd, on Linux, closes with the entry.
    Exit(libc::pid_t, Option<OwnedFd>, Once),
}

pub(crate) struct Hub {
    poller: Poller,
    next: AtomicU64,
    entries: Mutex<HashMap<u64, Entry>>,
    /// Who waits for each pid: the system is asked once per pid (kqueue
    /// keeps one watch per pid), and the first token stands for them all.
    exits: Mutex<HashMap<libc::pid_t, Vec<u64>>>,
    timers: Mutex<Timers>,
    /// Written to wake the thread for a timer earlier than it waits for.
    wake: (OwnedFd, OwnedFd),
}

#[derive(Default)]
struct Timers {
    due: BinaryHeap<Reverse<(Instant, u64)>>,
    run: HashMap<u64, Once>,
}

/// The token the wake pipe is registered under.
const WAKE: u64 = 0;
/// Events taken per wait.
const EVENTS: usize = 256;

/// The process's hub, started on first use.
pub(crate) fn hub() -> &'static Hub {
    static HUB: OnceLock<&'static Hub> = OnceLock::new();
    HUB.get_or_init(|| {
        let hub: &'static Hub = Box::leak(Box::new(Hub::new().expect("start the I/O hub")));
        std::thread::Builder::new()
            .name("sessiond-io".into())
            .spawn(move || hub.run())
            .expect("spawn the I/O hub thread");
        hub
    })
}

impl Hub {
    fn new() -> io::Result<Hub> {
        let poller = Poller::new()?;
        let wake = pipe()?;
        set_nonblocking(wake.0.as_raw_fd())?;
        set_nonblocking(wake.1.as_raw_fd())?;
        poller.add_level(wake.0.as_raw_fd(), WAKE)?;
        Ok(Hub {
            poller,
            next: AtomicU64::new(WAKE + 1),
            entries: Mutex::new(HashMap::new()),
            exits: Mutex::new(HashMap::new()),
            timers: Mutex::new(Timers::default()),
            wake,
        })
    }

    fn entries(&self) -> MutexGuard<'_, HashMap<u64, Entry>> {
        self.entries.lock().unwrap_or_else(|e| e.into_inner())
    }

    fn timers(&self) -> MutexGuard<'_, Timers> {
        self.timers.lock().unwrap_or_else(|e| e.into_inner())
    }

    fn token(&self) -> u64 {
        self.next.fetch_add(1, Ordering::Relaxed)
    }

    /// Run `f` on the hub thread after `d`.
    pub(crate) fn after(&self, d: Duration, f: impl FnOnce() + Send + 'static) {
        let at = Instant::now() + d;
        let t = self.token();
        let earliest = {
            let mut timers = self.timers();
            let earliest = timers.due.peek().is_none_or(|Reverse((e, _))| at < *e);
            timers.due.push(Reverse((at, t)));
            timers.run.insert(t, Box::new(f));
            earliest
        };
        if earliest {
            // A full pipe already wakes the thread.
            // SAFETY: a one-byte write from a live buffer to our pipe.
            unsafe { libc::write(self.wake.1.as_raw_fd(), [1u8].as_ptr().cast(), 1) };
        }
    }

    /// Run `f` on the hub thread once process `pid` has exited (it may
    /// already have). An error when this system cannot watch for it, and
    /// the caller must.
    pub(crate) fn when_exited(
        &self,
        pid: libc::pid_t,
        f: impl FnOnce() + Send + 'static,
    ) -> io::Result<()> {
        let t = self.token();
        let mut exits = self.exits.lock().unwrap_or_else(|e| e.into_inner());
        if let Some(waiting) = exits.get_mut(&pid) {
            waiting.push(t);
            self.entries()
                .insert(t, Entry::Exit(pid, None, Box::new(f)));
            return Ok(());
        }
        let pidfd = self.poller.pidfd(pid)?;
        let raw = pidfd.as_ref().map(AsRawFd::as_raw_fd);
        self.entries()
            .insert(t, Entry::Exit(pid, pidfd, Box::new(f)));
        exits.insert(pid, vec![t]);
        let added = self.poller.add_exit(pid, raw, t);
        drop(exits);
        match added {
            Ok(true) => Ok(()),
            // Gone already: no event will come, so run it now.
            Ok(false) => {
                self.fire_exit(t);
                Ok(())
            }
            Err(e) => {
                self.exits
                    .lock()
                    .unwrap_or_else(|e| e.into_inner())
                    .remove(&pid);
                let _ = self.entries().remove(&t);
                Err(e)
            }
        }
    }

    /// Run everything waiting for the exit `t` was registered for.
    fn fire_exit(&self, t: u64) {
        let pid = match self.entries().get(&t) {
            Some(Entry::Exit(pid, ..)) => *pid,
            _ => return,
        };
        let waiting = self
            .exits
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .remove(&pid)
            .unwrap_or_default();
        for w in waiting {
            if let Some(Entry::Exit(_, fd, f)) = self.entries().remove(&w) {
                drop(fd);
                self.after(Duration::ZERO, f);
            }
        }
    }

    fn run(&self) {
        let mut events = Vec::with_capacity(EVENTS);
        loop {
            let timeout = self.run_timers();
            events.clear();
            if let Err(e) = self.poller.wait(&mut events, timeout) {
                if e.kind() != io::ErrorKind::Interrupted {
                    // Nothing else can wait for sessions; keep trying.
                    std::thread::sleep(Duration::from_millis(10));
                }
                continue;
            }
            for &t in &events {
                if t == WAKE {
                    let mut buf = [0u8; 64];
                    // SAFETY: reads into a live buffer from our own pipe.
                    while unsafe {
                        libc::read(self.wake.0.as_raw_fd(), buf.as_mut_ptr().cast(), buf.len())
                    } > 0
                    {}
                    continue;
                }
                let handler = match self.entries().get(&t) {
                    Some(Entry::Io(h)) => Some(Arc::clone(h)),
                    Some(Entry::Exit(..)) => None,
                    None => continue,
                };
                match handler {
                    Some(h) => h(),
                    None => self.fire_exit(t),
                }
            }
        }
    }

    /// Run the timers that are due; how long until the next.
    fn run_timers(&self) -> Option<Duration> {
        loop {
            let f = {
                let mut timers = self.timers();
                let now = Instant::now();
                match timers.due.peek() {
                    None => return None,
                    Some(Reverse((at, _))) if *at > now => return Some(*at - now),
                    Some(_) => {}
                }
                let Reverse((_, t)) = timers.due.pop()?;
                timers.run.remove(&t)
            };
            if let Some(f) = f {
                f();
            }
        }
    }
}

/// A descriptor the hub waits on for one direction. Registered on first
/// [`Watched::arm`]; dropping it unregisters it and then closes it, so the
/// hub never hears of a descriptor number that has been reused.
pub(crate) struct Watched {
    fd: OwnedFd,
    dir: Dir,
    token: u64,
    registered: bool,
}

impl Watched {
    /// `fd`, non-blocking from now on, calling `handler` when ready.
    pub(crate) fn new(
        fd: OwnedFd,
        dir: Dir,
        handler: impl Fn(u64) + Send + Sync + 'static,
    ) -> io::Result<Watched> {
        set_nonblocking(fd.as_raw_fd())?;
        let hub = hub();
        let token = hub.token();
        hub.entries()
            .insert(token, Entry::Io(Arc::new(move || handler(token))));
        Ok(Watched {
            fd,
            dir,
            token,
            registered: false,
        })
    }

    pub(crate) fn token(&self) -> u64 {
        self.token
    }

    pub(crate) fn fd(&self) -> RawFd {
        self.fd.as_raw_fd()
    }

    /// Fire once when ready. Arming one already armed changes nothing.
    pub(crate) fn arm(&mut self) -> io::Result<()> {
        hub()
            .poller
            .arm(self.fd(), self.dir, self.token, !self.registered)?;
        self.registered = true;
        Ok(())
    }
}

impl Drop for Watched {
    fn drop(&mut self) {
        let hub = hub();
        if self.registered {
            hub.poller.remove(self.fd(), self.dir);
        }
        hub.entries().remove(&self.token);
    }
}

fn pipe() -> io::Result<(OwnedFd, OwnedFd)> {
    let mut fds = [0; 2];
    // SAFETY: pipe writes two descriptors into the array on success.
    if unsafe { libc::pipe(fds.as_mut_ptr()) } == -1 {
        return Err(io::Error::last_os_error());
    }
    // SAFETY: both were just opened and are owned by nothing else.
    let (r, w) = unsafe { (OwnedFd::from_raw_fd(fds[0]), OwnedFd::from_raw_fd(fds[1])) };
    set_cloexec(r.as_raw_fd())?;
    set_cloexec(w.as_raw_fd())?;
    Ok((r, w))
}

fn set_cloexec(fd: RawFd) -> io::Result<()> {
    // SAFETY: fcntl flag calls on a descriptor the caller owns.
    unsafe {
        let flags = libc::fcntl(fd, libc::F_GETFD);
        if flags == -1 || libc::fcntl(fd, libc::F_SETFD, flags | libc::FD_CLOEXEC) == -1 {
            return Err(io::Error::last_os_error());
        }
    }
    Ok(())
}

/// Make `fd` non-blocking: shared by every handle on the same open file,
/// which for a terminal's master is all of sessiond's and nobody else's.
pub(crate) fn set_nonblocking(fd: RawFd) -> io::Result<()> {
    // SAFETY: fcntl flag calls on a descriptor the caller owns.
    unsafe {
        let flags = libc::fcntl(fd, libc::F_GETFL);
        if flags == -1 || libc::fcntl(fd, libc::F_SETFL, flags | libc::O_NONBLOCK) == -1 {
            return Err(io::Error::last_os_error());
        }
    }
    Ok(())
}

#[cfg(target_os = "linux")]
struct Poller(OwnedFd);

#[cfg(target_os = "linux")]
impl Poller {
    fn new() -> io::Result<Poller> {
        // SAFETY: plain syscall; the result is checked.
        let fd = unsafe { libc::epoll_create1(libc::EPOLL_CLOEXEC) };
        if fd == -1 {
            return Err(io::Error::last_os_error());
        }
        // SAFETY: just created, owned by nothing else.
        Ok(Poller(unsafe { OwnedFd::from_raw_fd(fd) }))
    }

    fn ctl(&self, op: libc::c_int, fd: RawFd, events: u32, token: u64) -> io::Result<()> {
        let mut ev = libc::epoll_event { events, u64: token };
        // SAFETY: an event this frame owns, on our epoll descriptor.
        if unsafe { libc::epoll_ctl(self.0.as_raw_fd(), op, fd, &mut ev) } == -1 {
            return Err(io::Error::last_os_error());
        }
        Ok(())
    }

    fn add_level(&self, fd: RawFd, token: u64) -> io::Result<()> {
        self.ctl(libc::EPOLL_CTL_ADD, fd, libc::EPOLLIN as u32, token)
    }

    fn arm(&self, fd: RawFd, dir: Dir, token: u64, first: bool) -> io::Result<()> {
        let want = match dir {
            Dir::Read => libc::EPOLLIN | libc::EPOLLRDHUP,
            Dir::Write => libc::EPOLLOUT,
        };
        let events = (want | libc::EPOLLONESHOT) as u32;
        let op = if first {
            libc::EPOLL_CTL_ADD
        } else {
            libc::EPOLL_CTL_MOD
        };
        self.ctl(op, fd, events, token)
    }

    fn remove(&self, fd: RawFd, _dir: Dir) {
        let _ = self.ctl(libc::EPOLL_CTL_DEL, fd, 0, 0);
    }

    /// A pidfd for `pid`; an error on a kernel without them.
    fn pidfd(&self, pid: libc::pid_t) -> io::Result<Option<OwnedFd>> {
        // SAFETY: pidfd_open takes a pid and flags; the result is checked.
        let fd = unsafe { libc::syscall(libc::SYS_pidfd_open, pid, 0) };
        if fd == -1 {
            return Err(io::Error::last_os_error());
        }
        // SAFETY: a new descriptor, owned by nothing else.
        let fd = unsafe { OwnedFd::from_raw_fd(fd as RawFd) };
        set_cloexec(fd.as_raw_fd())?;
        Ok(Some(fd))
    }

    /// Watch the pidfd; it is readable once the process has exited.
    fn add_exit(&self, _pid: libc::pid_t, pidfd: Option<RawFd>, token: u64) -> io::Result<bool> {
        let fd = pidfd.ok_or_else(|| io::Error::other("no pidfd"))?;
        self.ctl(
            libc::EPOLL_CTL_ADD,
            fd,
            (libc::EPOLLIN | libc::EPOLLONESHOT) as u32,
            token,
        )?;
        Ok(true)
    }

    fn wait(&self, out: &mut Vec<u64>, timeout: Option<Duration>) -> io::Result<()> {
        let mut evs = [libc::epoll_event { events: 0, u64: 0 }; EVENTS];
        // Rounded up, so a timer is never woken for just before it is due.
        let ms = timeout.map_or(-1, |d| {
            d.as_nanos().div_ceil(1_000_000).min(60_000) as libc::c_int
        });
        // SAFETY: a buffer of EVENTS events this frame owns.
        let n = unsafe {
            libc::epoll_wait(
                self.0.as_raw_fd(),
                evs.as_mut_ptr(),
                EVENTS as libc::c_int,
                ms,
            )
        };
        if n == -1 {
            return Err(io::Error::last_os_error());
        }
        out.extend(evs[..n as usize].iter().map(|e| e.u64));
        Ok(())
    }
}

#[cfg(target_os = "macos")]
struct Poller(OwnedFd);

#[cfg(target_os = "macos")]
impl Poller {
    fn new() -> io::Result<Poller> {
        // SAFETY: plain syscall; the result is checked.
        let fd = unsafe { libc::kqueue() };
        if fd == -1 {
            return Err(io::Error::last_os_error());
        }
        // SAFETY: just created, owned by nothing else.
        let fd = unsafe { OwnedFd::from_raw_fd(fd) };
        set_cloexec(fd.as_raw_fd())?;
        Ok(Poller(fd))
    }

    fn change(
        &self,
        ident: usize,
        filter: i16,
        flags: u16,
        fflags: u32,
        token: u64,
    ) -> io::Result<()> {
        let ev = libc::kevent {
            ident,
            filter,
            flags,
            fflags,
            data: 0,
            udata: token as usize as *mut libc::c_void,
        };
        // SAFETY: one change from this frame, no events asked for.
        let n = unsafe {
            libc::kevent(
                self.0.as_raw_fd(),
                &ev,
                1,
                std::ptr::null_mut(),
                0,
                std::ptr::null(),
            )
        };
        if n == -1 {
            return Err(io::Error::last_os_error());
        }
        Ok(())
    }

    fn filter(dir: Dir) -> i16 {
        match dir {
            Dir::Read => libc::EVFILT_READ,
            Dir::Write => libc::EVFILT_WRITE,
        }
    }

    fn add_level(&self, fd: RawFd, token: u64) -> io::Result<()> {
        self.change(fd as usize, libc::EVFILT_READ, libc::EV_ADD, 0, token)
    }

    /// EV_ADD on a knote that exists only updates it; EV_ENABLE re-arms one
    /// that fired and was disabled by EV_DISPATCH.
    fn arm(&self, fd: RawFd, dir: Dir, token: u64, _first: bool) -> io::Result<()> {
        let flags = libc::EV_ADD | libc::EV_ENABLE | libc::EV_DISPATCH;
        self.change(fd as usize, Self::filter(dir), flags, 0, token)
    }

    fn remove(&self, fd: RawFd, dir: Dir) {
        let _ = self.change(fd as usize, Self::filter(dir), libc::EV_DELETE, 0, 0);
    }

    fn pidfd(&self, _pid: libc::pid_t) -> io::Result<Option<OwnedFd>> {
        Ok(None)
    }

    /// False when the process is gone already.
    fn add_exit(&self, pid: libc::pid_t, _pidfd: Option<RawFd>, token: u64) -> io::Result<bool> {
        match self.change(
            pid as usize,
            libc::EVFILT_PROC,
            libc::EV_ADD | libc::EV_ONESHOT,
            libc::NOTE_EXIT,
            token,
        ) {
            Ok(()) => Ok(true),
            Err(e) if e.raw_os_error() == Some(libc::ESRCH) => Ok(false),
            Err(e) => Err(e),
        }
    }

    fn wait(&self, out: &mut Vec<u64>, timeout: Option<Duration>) -> io::Result<()> {
        // SAFETY: kevent is plain data; an all-zero one is valid.
        let mut evs: [libc::kevent; EVENTS] = unsafe { std::mem::zeroed() };
        let ts = timeout.map(|d| libc::timespec {
            tv_sec: d.as_secs().min(60) as libc::time_t,
            tv_nsec: libc::c_long::from(d.subsec_nanos()),
        });
        let tsp = ts
            .as_ref()
            .map_or(std::ptr::null(), |t| t as *const libc::timespec);
        // SAFETY: a buffer of EVENTS events this frame owns.
        let n = unsafe {
            libc::kevent(
                self.0.as_raw_fd(),
                std::ptr::null(),
                0,
                evs.as_mut_ptr(),
                EVENTS as libc::c_int,
                tsp,
            )
        };
        if n == -1 {
            return Err(io::Error::last_os_error());
        }
        out.extend(evs[..n as usize].iter().map(|e| e.udata as usize as u64));
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::mpsc;

    #[test]
    fn a_descriptor_fires_once_per_arming() {
        let (r, w) = pipe().unwrap();
        let (tx, rx) = mpsc::channel();
        let tx = Mutex::new(tx);
        let mut watched = Watched::new(r, Dir::Read, move |_| {
            let _ = tx.lock().unwrap().send(());
        })
        .unwrap();
        // SAFETY: a write from a live buffer to our pipe.
        unsafe { libc::write(w.as_raw_fd(), b"x".as_ptr().cast(), 1) };
        watched.arm().unwrap();
        rx.recv_timeout(Duration::from_secs(5)).expect("fires");
        // Still readable, but not armed again: no second call.
        assert!(rx.recv_timeout(Duration::from_millis(100)).is_err());
        watched.arm().unwrap();
        rx.recv_timeout(Duration::from_secs(5))
            .expect("fires again");
        drop(watched);
    }

    #[test]
    fn timers_run_in_order_and_exits_are_heard() {
        let (tx, rx) = mpsc::channel();
        let t2 = tx.clone();
        hub().after(Duration::from_millis(60), move || t2.send("late").unwrap());
        let t1 = tx.clone();
        hub().after(Duration::from_millis(10), move || t1.send("early").unwrap());
        assert_eq!(rx.recv_timeout(Duration::from_secs(5)).unwrap(), "early");
        assert_eq!(rx.recv_timeout(Duration::from_secs(5)).unwrap(), "late");

        let child = std::process::Command::new("sh")
            .args(["-c", "sleep 0.1"])
            .spawn()
            .unwrap();
        let pid = child.id() as libc::pid_t;
        // Two watchers of one pid both hear it.
        let t3 = tx.clone();
        hub()
            .when_exited(pid, move || t3.send("exited").unwrap())
            .unwrap();
        hub()
            .when_exited(pid, move || tx.send("exited").unwrap())
            .unwrap();
        assert_eq!(rx.recv_timeout(Duration::from_secs(5)).unwrap(), "exited");
        assert_eq!(rx.recv_timeout(Duration::from_secs(5)).unwrap(), "exited");
        let mut child = child;
        child.wait().unwrap();
    }
}

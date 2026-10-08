//! Frames with descriptors beside them over a Unix socket (`SCM_RIGHTS`),
//! for handing live sessions from one sessiond to another.
//!
//! A frame's descriptors ride on its first byte. The kernel delivers them no
//! later than that byte, so by the time a whole frame has been read every
//! descriptor sent with it, and with each frame before it, is queued here in
//! the order sent.

use std::collections::VecDeque;
use std::io::{self, Write};
use std::net::Shutdown;
use std::os::fd::{AsRawFd, FromRawFd, OwnedFd, RawFd};
use std::os::unix::net::UnixStream;

use crate::wire::{FrameReader, Message};

/// The most descriptors one frame carries: a session passes at most three.
const MAX_FDS: usize = 8;
/// Room for the descriptors one read may bring: a read can span frames.
const RECV_FDS: usize = 64;

/// Send `msg` with `fds` beside it. The descriptors stay open here: the
/// receiver gets its own.
pub fn send<M: Message>(sock: &mut UnixStream, msg: &M, fds: &[RawFd]) -> io::Result<()> {
    let frame = msg.encode();
    if fds.is_empty() {
        return sock.write_all(&frame);
    }
    if fds.len() > MAX_FDS {
        return Err(io::Error::other("too many descriptors for one frame"));
    }
    let n = send_with_fds(sock.as_raw_fd(), &frame, fds)?;
    sock.write_all(&frame[n..])
}

fn send_with_fds(sock: RawFd, bytes: &[u8], fds: &[RawFd]) -> io::Result<usize> {
    let fd_bytes = std::mem::size_of_val(fds);
    // u64s keep the control buffer aligned for `cmsghdr`.
    // SAFETY: CMSG_SPACE only computes a size.
    let space = unsafe { libc::CMSG_SPACE(fd_bytes as u32) } as usize;
    let mut control = vec![0u64; space.div_ceil(8)];
    let mut iov = libc::iovec {
        iov_base: bytes.as_ptr() as *mut libc::c_void,
        iov_len: bytes.len(),
    };
    // SAFETY: an all-zero msghdr is valid; the fields set below point at
    // buffers that outlive the call.
    let mut msg: libc::msghdr = unsafe { std::mem::zeroed() };
    msg.msg_iov = &mut iov;
    msg.msg_iovlen = 1;
    msg.msg_control = control.as_mut_ptr().cast();
    msg.msg_controllen = space as _;
    // SAFETY: `control` holds CMSG_SPACE(fd_bytes) bytes, so the first header
    // and its data fit, and CMSG_DATA points inside it.
    unsafe {
        let cmsg = libc::CMSG_FIRSTHDR(&msg);
        (*cmsg).cmsg_level = libc::SOL_SOCKET;
        (*cmsg).cmsg_type = libc::SCM_RIGHTS;
        (*cmsg).cmsg_len = libc::CMSG_LEN(fd_bytes as u32) as _;
        std::ptr::copy_nonoverlapping(fds.as_ptr().cast::<u8>(), libc::CMSG_DATA(cmsg), fd_bytes);
    }
    loop {
        // SAFETY: `msg` and everything it points at are valid for the call.
        let n = unsafe { libc::sendmsg(sock, &msg, NOSIGPIPE) };
        if n >= 0 {
            return Ok(n as usize);
        }
        let e = io::Error::last_os_error();
        if e.kind() != io::ErrorKind::Interrupted {
            return Err(e);
        }
    }
}

#[cfg(target_os = "linux")]
const NOSIGPIPE: libc::c_int = libc::MSG_NOSIGNAL;
// macOS has no MSG_NOSIGNAL; Rust ignores SIGPIPE in every binary it starts.
#[cfg(not(target_os = "linux"))]
const NOSIGPIPE: libc::c_int = 0;

/// Reads frames, and the descriptors that came with them, off one socket.
pub struct Receiver {
    sock: UnixStream,
    frames: FrameReader,
    fds: VecDeque<OwnedFd>,
    buf: Vec<u8>,
}

impl Receiver {
    pub fn new(sock: UnixStream) -> Receiver {
        Receiver {
            sock,
            frames: FrameReader::default(),
            fds: VecDeque::new(),
            buf: vec![0u8; 64 << 10],
        }
    }

    pub fn socket(&mut self) -> &mut UnixStream {
        &mut self.sock
    }

    /// The next frame. The end of the stream is `UnexpectedEof`, and a read
    /// timeout set on the socket is `WouldBlock` or `TimedOut`.
    pub fn recv<M: Message>(&mut self) -> io::Result<M> {
        loop {
            match self.frames.read::<M>() {
                Ok(Some(m)) => return Ok(m),
                Ok(None) => {}
                Err(e) => return Err(io::Error::new(io::ErrorKind::InvalidData, format!("{e:?}"))),
            }
            let n = self.recv_some()?;
            if n == 0 {
                return Err(io::ErrorKind::UnexpectedEof.into());
            }
        }
    }

    /// The next `n` descriptors received, in the order they were sent.
    pub fn take_fds(&mut self, n: usize) -> io::Result<Vec<OwnedFd>> {
        if self.fds.len() < n {
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                "fewer descriptors than the frame names",
            ));
        }
        Ok(self.fds.drain(..n).collect())
    }

    /// Descriptors received that no frame claimed.
    pub fn unclaimed(&self) -> usize {
        self.fds.len()
    }

    /// Hang up and read until the other end closes too, or the read timeout
    /// passes, closing every descriptor that comes: a plain read would leave
    /// them for the kernel to dispose of, which macOS may not do soon.
    pub fn drain(&mut self) {
        let _ = self.sock.shutdown(Shutdown::Write);
        while matches!(self.recv_some(), Ok(n) if n > 0) {
            self.fds.clear();
            self.frames = FrameReader::default();
        }
        self.fds.clear();
    }

    fn recv_some(&mut self) -> io::Result<usize> {
        // SAFETY: CMSG_SPACE only computes a size.
        let space =
            unsafe { libc::CMSG_SPACE((RECV_FDS * std::mem::size_of::<RawFd>()) as u32) } as usize;
        let mut control = vec![0u64; space.div_ceil(8)];
        let mut iov = libc::iovec {
            iov_base: self.buf.as_mut_ptr().cast(),
            iov_len: self.buf.len(),
        };
        // SAFETY: as in `send_with_fds`.
        let mut msg: libc::msghdr = unsafe { std::mem::zeroed() };
        msg.msg_iov = &mut iov;
        msg.msg_iovlen = 1;
        msg.msg_control = control.as_mut_ptr().cast();
        msg.msg_controllen = space as _;
        let n = loop {
            // SAFETY: `msg` points at buffers that outlive the call.
            let n = unsafe { libc::recvmsg(self.sock.as_raw_fd(), &mut msg, CLOEXEC) };
            if n >= 0 {
                break n as usize;
            }
            let e = io::Error::last_os_error();
            if e.kind() != io::ErrorKind::Interrupted {
                return Err(e);
            }
        };
        // SAFETY: the kernel filled `msg_controllen` bytes of `control` with
        // whole headers; the macros walk only those.
        unsafe {
            let mut cmsg = libc::CMSG_FIRSTHDR(&msg);
            while !cmsg.is_null() {
                if (*cmsg).cmsg_level == libc::SOL_SOCKET && (*cmsg).cmsg_type == libc::SCM_RIGHTS {
                    let data = libc::CMSG_DATA(cmsg);
                    let len = (*cmsg).cmsg_len as usize - (data as usize - cmsg as usize);
                    for i in 0..len / std::mem::size_of::<RawFd>() {
                        let fd = std::ptr::read_unaligned(data.cast::<RawFd>().add(i));
                        let fd = OwnedFd::from_raw_fd(fd);
                        set_cloexec(&fd);
                        self.fds.push_back(fd);
                    }
                }
                cmsg = libc::CMSG_NXTHDR(&msg, cmsg);
            }
        }
        if msg.msg_flags & libc::MSG_CTRUNC != 0 {
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                "descriptors were cut off",
            ));
        }
        self.frames.push(&self.buf[..n]);
        Ok(n)
    }
}

#[cfg(target_os = "linux")]
const CLOEXEC: libc::c_int = libc::MSG_CMSG_CLOEXEC;
#[cfg(not(target_os = "linux"))]
const CLOEXEC: libc::c_int = 0;

/// macOS has no MSG_CMSG_CLOEXEC: a descriptor received there is made
/// close-on-exec at once, before any program this process starts can
/// inherit it but for one forked in between.
fn set_cloexec(fd: &OwnedFd) {
    // SAFETY: fcntl on a descriptor this process owns.
    unsafe {
        let flags = libc::fcntl(fd.as_raw_fd(), libc::F_GETFD);
        if flags != -1 && flags & libc::FD_CLOEXEC == 0 {
            libc::fcntl(fd.as_raw_fd(), libc::F_SETFD, flags | libc::FD_CLOEXEC);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::wire::{Nonce, ToVornd};
    use std::io::{Read, Write};

    #[test]
    fn descriptors_arrive_with_their_frame_and_in_order() {
        let (mut a, b) = UnixStream::pair().unwrap();
        let (mut p1, q1) = UnixStream::pair().unwrap();
        let (mut p2, q2) = UnixStream::pair().unwrap();
        send(&mut a, &ToVornd::Pong(Nonce { nonce: 1 }), &[]).unwrap();
        send(
            &mut a,
            &ToVornd::Pong(Nonce { nonce: 2 }),
            &[q1.as_raw_fd(), q2.as_raw_fd()],
        )
        .unwrap();
        // A big frame after it: the receiver reads it in several pieces.
        let big = ToVornd::Failed(crate::wire::Failed {
            req: 3,
            error: "x".repeat(300 << 10),
        });
        let sender = std::thread::spawn(move || {
            send(&mut a, &big, &[]).unwrap();
            a
        });
        let mut r = Receiver::new(b);
        assert_eq!(
            r.recv::<ToVornd>().unwrap(),
            ToVornd::Pong(Nonce { nonce: 1 })
        );
        assert_eq!(
            r.recv::<ToVornd>().unwrap(),
            ToVornd::Pong(Nonce { nonce: 2 })
        );
        let got = r.take_fds(2).unwrap();
        assert!(
            matches!(r.recv::<ToVornd>().unwrap(), ToVornd::Failed(f) if f.error.len() == 300 << 10)
        );
        assert_eq!(r.unclaimed(), 0);
        drop(sender.join().unwrap());
        assert_eq!(
            r.recv::<ToVornd>().unwrap_err().kind(),
            io::ErrorKind::UnexpectedEof
        );

        // Each received descriptor is the socket end sent, close-on-exec.
        drop((q1, q2));
        let mut ends: Vec<UnixStream> = got.into_iter().map(UnixStream::from).collect();
        ends[0].write_all(b"one").unwrap();
        ends[1].write_all(b"two").unwrap();
        let mut buf = [0u8; 3];
        p1.read_exact(&mut buf).unwrap();
        assert_eq!(&buf, b"one");
        p2.read_exact(&mut buf).unwrap();
        assert_eq!(&buf, b"two");
        // SAFETY: fcntl on a descriptor this test owns.
        let flags = unsafe { libc::fcntl(ends[0].as_raw_fd(), libc::F_GETFD) };
        assert_ne!(flags & libc::FD_CLOEXEC, 0);
    }
}

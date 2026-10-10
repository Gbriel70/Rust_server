//! Linux readiness API. All project-owned unsafe is confined to this module.
use std::{
    io,
    os::fd::{AsRawFd, BorrowedFd, FromRawFd, OwnedFd},
};

#[derive(Clone, Copy, Debug)]
pub struct Event {
    pub token: u64,
    pub flags: u32,
}
pub struct Epoll {
    fd: OwnedFd,
}
impl Epoll {
    pub fn new() -> io::Result<Self> {
        // SAFETY: epoll_create1 has no pointers; CLOEXEC is a valid flag.
        let fd = unsafe { libc::epoll_create1(libc::EPOLL_CLOEXEC) };
        if fd < 0 {
            return Err(io::Error::last_os_error());
        }
        // SAFETY: successful creation transfers one fresh, exclusively owned fd.
        Ok(Self {
            fd: unsafe { OwnedFd::from_raw_fd(fd) },
        })
    }
    pub fn add(&self, fd: BorrowedFd<'_>, token: u64, flags: u32) -> io::Result<()> {
        self.control(libc::EPOLL_CTL_ADD, fd, token, flags)
    }
    pub fn modify(&self, fd: BorrowedFd<'_>, token: u64, flags: u32) -> io::Result<()> {
        self.control(libc::EPOLL_CTL_MOD, fd, token, flags)
    }
    pub fn delete(&self, fd: BorrowedFd<'_>) -> io::Result<()> {
        self.control(libc::EPOLL_CTL_DEL, fd, 0, 0)
    }
    fn control(&self, op: i32, fd: BorrowedFd<'_>, token: u64, flags: u32) -> io::Result<()> {
        let mut event = libc::epoll_event {
            events: flags,
            u64: token,
        };
        // SAFETY: both borrowed descriptors remain live throughout the call;
        // event points to initialized storage and epoll_ctl does not retain it.
        if unsafe { libc::epoll_ctl(self.fd.as_raw_fd(), op, fd.as_raw_fd(), &mut event) } < 0 {
            return Err(io::Error::last_os_error());
        }
        Ok(())
    }
    /// A fixed initialized array avoids exposing uninitialized C output to Rust.
    pub fn wait(&self, timeout_ms: i32) -> io::Result<Vec<Event>> {
        let mut events = [libc::epoll_event { events: 0, u64: 0 }; 256];
        // SAFETY: the live epoll fd and writable array of 256 properly laid-out
        // libc events are valid; the kernel writes at most the supplied count.
        let n =
            unsafe { libc::epoll_wait(self.fd.as_raw_fd(), events.as_mut_ptr(), 256, timeout_ms) };
        if n < 0 {
            return Err(io::Error::last_os_error());
        }
        Ok(events[..n as usize]
            .iter()
            .map(|e| Event {
                token: e.u64,
                flags: e.events,
            })
            .collect())
    }
}

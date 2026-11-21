//! POSIX PTY ownership and readiness. AsyncFd uses the existing Tokio reactor;
//! interactive sessions need no blocking worker thread or polling timer.
use std::{
    io,
    os::fd::{AsRawFd, FromRawFd, OwnedFd},
    ptr,
    sync::Arc,
};
use tokio::io::{Interest, unix::AsyncFd};

pub struct Pending {
    pub master: Arc<Master>,
    pub slave: OwnedFd,
    pub term: String,
}

pub struct Master(AsyncFd<OwnedFd>);

fn window(columns: u32, rows: u32) -> libc::winsize {
    libc::winsize {
        ws_row: if rows == 0 { 24 } else { rows.min(1024) as u16 },
        ws_col: if columns == 0 { 80 } else { columns.min(1024) as u16 },
        ws_xpixel: 0,
        ws_ypixel: 0,
    }
}

impl Pending {
    pub fn open(term: &str, columns: u32, rows: u32) -> io::Result<Self> {
        if term.len() > 64 || !term.bytes().all(|b| b.is_ascii_alphanumeric() || b"-_.".contains(&b)) {
            return Err(io::Error::new(io::ErrorKind::InvalidInput, "invalid terminal name"));
        }
        let mut master = -1;
        let mut slave = -1;
        let mut dimensions = window(columns, rows);
        // openpty initializes both fds on success; OwnedFd takes them exactly
        // once and closes them along every subsequent error path.
        if unsafe { libc::openpty(&mut master, &mut slave, ptr::null_mut(), ptr::null_mut(), &raw mut dimensions) } != 0
        {
            return Err(io::Error::last_os_error());
        }
        let master = unsafe { OwnedFd::from_raw_fd(master) };
        let slave = unsafe { OwnedFd::from_raw_fd(slave) };
        for fd in [master.as_raw_fd(), slave.as_raw_fd()] {
            if unsafe { libc::fcntl(fd, libc::F_SETFD, libc::FD_CLOEXEC) } < 0 {
                return Err(io::Error::last_os_error());
            }
        }
        let flags = unsafe { libc::fcntl(master.as_raw_fd(), libc::F_GETFL) };
        if flags < 0 || unsafe { libc::fcntl(master.as_raw_fd(), libc::F_SETFL, flags | libc::O_NONBLOCK) } < 0 {
            return Err(io::Error::last_os_error());
        }
        Ok(Self {
            // OwnedFd remains inside AsyncFd until its registration is dropped;
            // no operation here replaces, closes or reuses its descriptor.
            master: Arc::new(Master(unsafe { AsyncFd::register(master)? })),
            slave,
            term: if term.is_empty() { "xterm".into() } else { term.into() },
        })
    }
}

impl Master {
    pub fn resize(&self, columns: u32, rows: u32) -> io::Result<()> {
        let dimensions = window(columns, rows);
        if unsafe { libc::ioctl(self.0.get_ref().as_raw_fd(), libc::TIOCSWINSZ, &dimensions) } != 0 {
            Err(io::Error::last_os_error())
        } else {
            Ok(())
        }
    }

    pub async fn read(&self, bytes: &mut [u8]) -> io::Result<usize> {
        loop {
            let result = self
                .0
                .async_io(Interest::READABLE, |fd| {
                    let count = unsafe { libc::read(fd.as_raw_fd(), bytes.as_mut_ptr().cast(), bytes.len()) };
                    if count < 0 { Err(io::Error::last_os_error()) } else { Ok(count as usize) }
                })
                .await;
            match result {
                Err(e) if e.kind() == io::ErrorKind::Interrupted => continue,
                // Linux PTYs report EIO when their last slave closes.
                Err(e) if e.raw_os_error() == Some(libc::EIO) => return Ok(0),
                result => return result,
            }
        }
    }

    pub async fn write_all(&self, mut bytes: &[u8]) -> io::Result<()> {
        while !bytes.is_empty() {
            let count = self
                .0
                .async_io(Interest::WRITABLE, |fd| {
                    let count = unsafe { libc::write(fd.as_raw_fd(), bytes.as_ptr().cast(), bytes.len()) };
                    if count < 0 { Err(io::Error::last_os_error()) } else { Ok(count as usize) }
                })
                .await;
            match count {
                Err(e) if e.kind() == io::ErrorKind::Interrupted => continue,
                Err(e) => return Err(e),
                Ok(0) => return Err(io::Error::from(io::ErrorKind::WriteZero)),
                Ok(n) => bytes = &bytes[n..],
            }
        }
        Ok(())
    }
}

/// A connection's commands have their own session/process group. A dropped
/// SSH connection also terminates descendants that still hold its pipes/PTY.
pub struct ProcessGroup(pub i32);
impl Drop for ProcessGroup {
    fn drop(&mut self) {
        if self.0 > 0 {
            unsafe {
                libc::kill(-self.0, libc::SIGHUP);
            }
        }
    }
}

/// Called only in a child's pre_exec hook: no allocation or locks after fork.
pub unsafe fn session(controlling_terminal: bool) -> io::Result<()> {
    if unsafe { libc::setsid() } < 0 {
        return Err(io::Error::last_os_error());
    }
    if controlling_terminal && unsafe { libc::ioctl(0, libc::TIOCSCTTY as _, 0) } < 0 {
        return Err(io::Error::last_os_error());
    }
    Ok(())
}

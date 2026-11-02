//! AF_UNIX sockets (`UnixListener`, `UnixStream`), in the shape of std's
//! `std::os::unix::net`, plus what std leaves to `libc`: handles passed
//! with the bytes (`SCM_RIGHTS`), the peer's identity, abstract names.
//! They are the kernel's sockets, so Linux programs connect to a native
//! listener's path, and the other way round.

use crate::abi::{self, Handle, PeerInfo};
use crate::io::{Read, Write};
use crate::{Result, sys};
use alloc::vec::Vec;

/// A socket that takes connections.
#[derive(Debug)]
pub struct UnixListener {
    handle: Handle,
}

/// A connected stream socket, closed when dropped.
#[derive(Debug)]
pub struct UnixStream {
    handle: Handle,
}

fn listen_on(name: &[u8], abstract_name: bool) -> Result<UnixListener> {
    let handle = sys::socket(abi::socket::STREAM, false)?;
    let l = UnixListener { handle };
    sys::bind(handle, name, abstract_name)?;
    sys::listen(handle, 128)?;
    Ok(l)
}

impl UnixListener {
    /// Listen at `path` in the file system.
    pub fn bind(path: &str) -> Result<UnixListener> {
        listen_on(path.as_bytes(), false)
    }

    /// Listen at `name` in the abstract name space (unix(7)): no file.
    pub fn bind_abstract(name: &[u8]) -> Result<UnixListener> {
        listen_on(name, true)
    }

    /// The next connection (waiting for one unless non-blocking).
    pub fn accept(&self) -> Result<UnixStream> {
        sys::accept(self.handle, false).map(|handle| UnixStream { handle })
    }

    pub fn set_nonblocking(&self, on: bool) -> Result<()> {
        sys::set_nonblocking(self.handle, on)
    }

    pub fn handle(&self) -> Handle {
        self.handle
    }
}

impl Drop for UnixListener {
    fn drop(&mut self) {
        let _ = sys::close(self.handle);
    }
}

impl UnixStream {
    /// Connect to the listener at `path`.
    pub fn connect(path: &str) -> Result<UnixStream> {
        let handle = sys::socket(abi::socket::STREAM, false)?;
        let s = UnixStream { handle };
        sys::connect(handle, path.as_bytes(), false)?;
        Ok(s)
    }

    /// Two connected streams.
    pub fn pair() -> Result<(UnixStream, UnixStream)> {
        let [a, b] = sys::socket_pair(abi::socket::STREAM, false)?;
        Ok((UnixStream { handle: a }, UnixStream { handle: b }))
    }

    /// Take ownership of `handle`.
    pub fn from_handle(handle: Handle) -> UnixStream {
        UnixStream { handle }
    }

    pub fn handle(&self) -> Handle {
        self.handle
    }

    /// Give up ownership without closing.
    pub fn into_handle(self) -> Handle {
        let h = self.handle;
        core::mem::forget(self);
        h
    }

    pub fn set_nonblocking(&self, on: bool) -> Result<()> {
        sys::set_nonblocking(self.handle, on)
    }

    /// Send `bytes` and `handles` (with the first byte); returns how many
    /// bytes went. The handles stay the caller's too.
    pub fn send_with_handles(&self, bytes: &[u8], handles: &[Handle]) -> Result<usize> {
        sys::send(self.handle, bytes, handles)
    }

    /// Receive into `buf`; handles that came along are appended to
    /// `handles` (up to [`MAX_HANDLES`] a call). 0 bytes: the peer closed.
    pub fn recv_with_handles(&self, buf: &mut [u8], handles: &mut Vec<Handle>) -> Result<usize> {
        let mut got = [0u32; MAX_HANDLES];
        let (n, count) = sys::recv(self.handle, buf, &mut got)?;
        handles.extend_from_slice(&got[..count]);
        Ok(n)
    }

    /// Who is at the other end (its pid and ids when it connected).
    pub fn peer(&self) -> Result<PeerInfo> {
        sys::socket_peer(self.handle)
    }
}

/// The most handles [`UnixStream::recv_with_handles`] takes in one call.
pub const MAX_HANDLES: usize = 32;

impl Read for UnixStream {
    fn read(&mut self, buf: &mut [u8]) -> Result<usize> {
        sys::read(self.handle, buf)
    }
}

impl Write for UnixStream {
    fn write(&mut self, buf: &[u8]) -> Result<usize> {
        sys::send(self.handle, buf, &[])
    }
}

impl Drop for UnixStream {
    fn drop(&mut self) {
        let _ = sys::close(self.handle);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::Error;

    #[test]
    fn sockets_need_the_kernel() {
        assert_eq!(UnixListener::bind("/run/x").err(), Some(Error::Unsupported));
        assert_eq!(UnixStream::pair().err(), Some(Error::Unsupported));
    }
}

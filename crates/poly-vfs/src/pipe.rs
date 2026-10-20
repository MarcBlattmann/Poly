//! Pipes, as pipe(7) describes them.
//!
//! A [`Pipe`] is a bounded byte buffer ([`PIPE_CAPACITY`]) with a count of
//! read ends and write ends; each end is a [`PipeEnd`], the
//! [`FileObject`] behind one open file description. The rules:
//!
//! - Reading an empty pipe gives end of file (0) once no write end is
//!   left, else would block (`EAGAIN` here; the open file waits).
//! - Writing with no read end left fails with `EPIPE` (the system call
//!   layer raises `SIGPIPE`).
//! - A write of at most [`PIPE_BUF`] bytes is atomic: all of it goes in
//!   at once or, without room, none of it (`EAGAIN`). A longer write takes
//!   whatever fits (at least one byte) and the open file's blocking write
//!   comes back for the rest.
//! - Packet mode (`O_DIRECT`, pipe2(2)): every write is a packet (writes
//!   longer than `PIPE_BUF` are split into packets of `PIPE_BUF`), a read
//!   returns at most one packet and drops what of it does not fit.
//! - Readiness: a read end is readable while there is data and hung up
//!   (`POLLHUP`) without write ends; a write end is writable while
//!   `PIPE_BUF` bytes fit (so a write of that size would not block; Poly's
//!   threshold, to be confirmed by the diff runner) and reports `POLLERR`
//!   once no read end is left (poll(2)).

use crate::object::FileObject;
use crate::poll::{PollEvents, WaitQueue};
use crate::sync::SpinMutex;
use crate::{Errno, Result};
use alloc::collections::VecDeque;
use alloc::sync::Arc;

/// Writes of at most this many bytes are atomic (`PIPE_BUF`, POSIX
/// `limits.h`; 4096 on Linux, pipe(7)).
pub const PIPE_BUF: usize = 4096;

/// What a pipe holds: 65536 bytes, as pipe(7) gives for Linux since 2.6.11.
pub const PIPE_CAPACITY: usize = 65536;

/// The buffer and the ends of one pipe.
pub struct Pipe {
    state: SpinMutex<State>,
    queue: Arc<WaitQueue>,
}

struct State {
    data: VecDeque<u8>,
    /// Packet lengths, in order, in packet mode.
    packets: Option<VecDeque<usize>>,
    readers: usize,
    writers: usize,
}

impl Pipe {
    /// A new pipe and its two ends: `(read end, write end)`. `packet`:
    /// packet mode (`O_DIRECT`).
    pub fn pair(packet: bool) -> (PipeEnd, PipeEnd) {
        let pipe = Arc::new(Pipe {
            state: SpinMutex::new(State {
                data: VecDeque::new(),
                packets: packet.then(VecDeque::new),
                readers: 0,
                writers: 0,
            }),
            queue: Arc::new(WaitQueue::new()),
        });
        let read = PipeEnd::new(&pipe, true, false);
        let write = PipeEnd::new(&pipe, false, true);
        (read, write)
    }

    /// Bytes in the pipe.
    pub fn len(&self) -> usize {
        self.state.lock().data.len()
    }

    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }

    /// The queue readers and writers wait on.
    pub fn queue(&self) -> &Arc<WaitQueue> {
        &self.queue
    }
}

impl core::fmt::Debug for Pipe {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        let s = self.state.lock();
        f.debug_struct("Pipe")
            .field("len", &s.data.len())
            .field("readers", &s.readers)
            .field("writers", &s.writers)
            .field("packet", &s.packets.is_some())
            .finish()
    }
}

/// One end of a pipe (both for a FIFO opened for reading and writing).
/// Dropping it closes that end.
pub struct PipeEnd {
    pipe: Arc<Pipe>,
    read: bool,
    write: bool,
}

impl PipeEnd {
    /// Another end of `pipe` (the FIFO case: an end per `open`).
    pub fn new(pipe: &Arc<Pipe>, read: bool, write: bool) -> PipeEnd {
        let mut s = pipe.state.lock();
        s.readers += usize::from(read);
        s.writers += usize::from(write);
        drop(s);
        PipeEnd { pipe: Arc::clone(pipe), read, write }
    }

    pub fn pipe(&self) -> &Arc<Pipe> {
        &self.pipe
    }
}

impl Drop for PipeEnd {
    fn drop(&mut self) {
        let mut s = self.pipe.state.lock();
        s.readers -= usize::from(self.read);
        s.writers -= usize::from(self.write);
        let mut events = PollEvents::NONE;
        if self.write && s.writers == 0 {
            // Readers see end of file.
            events = events | PollEvents::HUP | PollEvents::READABLE;
        }
        if self.read && s.readers == 0 {
            // Writers get EPIPE.
            events = events | PollEvents::ERR;
        }
        drop(s);
        if !events.is_empty() {
            self.pipe.queue.notify(events);
        }
    }
}

impl core::fmt::Debug for PipeEnd {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.debug_struct("PipeEnd").field("read", &self.read).field("write", &self.write).finish_non_exhaustive()
    }
}

/// Copy the first `buf.len()` bytes of `data` out and drop them.
fn take(data: &mut VecDeque<u8>, buf: &mut [u8]) {
    let (a, b) = data.as_slices();
    let n = buf.len();
    let from_a = n.min(a.len());
    buf[..from_a].copy_from_slice(&a[..from_a]);
    buf[from_a..].copy_from_slice(&b[..n - from_a]);
    data.drain(..n);
}

impl FileObject for PipeEnd {
    fn read(&self, buf: &mut [u8]) -> Result<usize> {
        if !self.read {
            return Err(Errno::EBADF);
        }
        if buf.is_empty() {
            return Ok(0);
        }
        let mut s = self.pipe.state.lock();
        if s.data.is_empty() {
            return if s.writers == 0 { Ok(0) } else { Err(Errno::EAGAIN) };
        }
        let n = match s.packets.as_mut().and_then(VecDeque::pop_front) {
            Some(packet) => {
                let n = packet.min(buf.len());
                take(&mut s.data, &mut buf[..n]);
                // pipe(7): "the excess bytes in the packet are discarded".
                s.data.drain(..packet - n);
                n
            }
            None => {
                let n = s.data.len().min(buf.len());
                take(&mut s.data, &mut buf[..n]);
                n
            }
        };
        drop(s);
        self.pipe.queue.notify(PollEvents::WRITABLE);
        Ok(n)
    }

    fn write(&self, buf: &[u8]) -> Result<usize> {
        if !self.write {
            return Err(Errno::EBADF);
        }
        if buf.is_empty() {
            // write(2): with a count of 0 on anything but a regular file
            // "the results are not specified"; Poly writes nothing and
            // succeeds (to be confirmed by the diff runner).
            return Ok(0);
        }
        let mut s = self.pipe.state.lock();
        if s.readers == 0 {
            return Err(Errno::EPIPE);
        }
        let room = PIPE_CAPACITY - s.data.len();
        let n = if buf.len() <= PIPE_BUF {
            if room < buf.len() {
                return Err(Errno::EAGAIN);
            }
            buf.len()
        } else {
            if room == 0 {
                return Err(Errno::EAGAIN);
            }
            room.min(buf.len())
        };
        s.data.extend(&buf[..n]);
        if let Some(packets) = s.packets.as_mut() {
            let mut left = n;
            while left > 0 {
                let p = left.min(PIPE_BUF);
                packets.push_back(p);
                left -= p;
            }
        }
        drop(s);
        self.pipe.queue.notify(PollEvents::READABLE);
        Ok(n)
    }

    fn poll(&self) -> PollEvents {
        let s = self.pipe.state.lock();
        let mut events = PollEvents::NONE;
        if self.read {
            if !s.data.is_empty() {
                events = events | PollEvents::READABLE;
            }
            if s.writers == 0 {
                events = events | PollEvents::HUP;
            }
        }
        if self.write {
            if s.readers == 0 {
                events = events | PollEvents::ERR;
            } else if PIPE_CAPACITY - s.data.len() >= PIPE_BUF {
                events = events | PollEvents::WRITABLE;
            }
        }
        events
    }

    fn wait_queue(&self) -> Option<&Arc<WaitQueue>> {
        Some(&self.pipe.queue)
    }

    fn bytes_readable(&self) -> Option<usize> {
        Some(self.pipe.len())
    }
}

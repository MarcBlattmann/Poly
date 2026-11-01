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
//! - Room is counted in pages of [`PIPE_PAGE`] bytes (pipe2's spec sheet):
//!   the pipe has [`PIPE_CAPACITY`] / [`PIPE_PAGE`] of them by default
//!   (`F_SETPIPE_SZ` changes that, [`Pipe::set_capacity`]). A write puts
//!   the part of it that is not whole pages (its length modulo
//!   [`PIPE_PAGE`]) into the last page if all of that part fits there, and
//!   the rest into new pages of at most a page each; a page is free again
//!   only once every byte of it was read, so a partly read page takes no
//!   new bytes. (A 5000-byte write into 15 full pages and one with 1000
//!   bytes free takes 904 bytes: pipe2's visible test.)
//! - Packet mode (`O_DIRECT`, pipe2(2)): every write is a packet (writes
//!   longer than `PIPE_BUF` are split into packets of `PIPE_BUF`), a read
//!   returns at most one packet and drops what of it does not fit.
//! - Readiness: a read end is readable while there is data and hung up
//!   (`POLLHUP`) without write ends; a write end is writable while a page
//!   is free (so a write of `PIPE_BUF` bytes would not block) and reports
//!   `POLLERR` once no read end is left (poll(2)).

use crate::object::FileObject;
use crate::poll::{Blocker, PollEvents, WaitQueue, Woke};
use crate::sync::SpinMutex;
use crate::{Errno, Result};
use alloc::collections::VecDeque;
use alloc::sync::Arc;

/// Writes of at most this many bytes are atomic (`PIPE_BUF`, POSIX
/// `limits.h`; 4096 on Linux, pipe(7)).
pub const PIPE_BUF: usize = 4096;

/// What a pipe holds: 65536 bytes, as pipe(7) gives for Linux since 2.6.11.
pub const PIPE_CAPACITY: usize = 65536;

/// The unit a pipe's room is counted in (a page).
pub const PIPE_PAGE: usize = 4096;

/// The largest capacity [`Pipe::set_capacity`] takes (1 GiB; pipe(7)'s
/// `/proc/sys/fs/pipe-max-size` limit applies to unprivileged callers,
/// which the system call layer checks).
pub const PIPE_MAX_CAPACITY: usize = 1 << 30;

/// The buffer and the ends of one pipe.
pub struct Pipe {
    state: SpinMutex<State>,
    queue: Arc<WaitQueue>,
}

/// One page of a pipe's buffer: how many bytes were written into it and
/// how many of those are still unread, and whether a later write may add
/// to it (a page a write started; not one splice(2), tee(2) or
/// vmsplice(2) put in, which stays a buffer of its own).
#[derive(Clone, Copy, Debug)]
struct Page {
    filled: usize,
    left: usize,
    merge: bool,
}

struct State {
    data: VecDeque<u8>,
    /// The pages `data` occupies, oldest first. In packet mode each
    /// packet has a page of its own.
    pages: VecDeque<Page>,
    /// How many pages the pipe may hold.
    slots: usize,
    packet: bool,
    readers: usize,
    writers: usize,
    /// Read and write ends opened so far, for a FIFO open waiting for the
    /// other side ([`PipeEnd::wait_for_peer`]).
    read_opens: u64,
    write_opens: u64,
}

impl State {
    /// Pages a write could start now.
    fn free_pages(&self) -> usize {
        self.slots.saturating_sub(self.pages.len())
    }

    /// How many of a `len`-byte write's bytes go into the last page: the
    /// part that is not whole pages (`len` modulo a page), if all of it
    /// fits there; else none (and never in packet mode).
    fn merge(&self, len: usize) -> usize {
        let tail = len % PIPE_PAGE;
        match self.pages.back() {
            Some(p) if !self.packet && p.merge && tail > 0 && p.filled + tail <= PIPE_PAGE => tail,
            _ => 0,
        }
    }

    /// How many bytes of a `len`-byte write go in now: [`merge`](Self::merge)
    /// into the last page, then whole free pages.
    fn fits(&self, len: usize) -> usize {
        let merge = self.merge(len);
        merge + (len - merge).min(self.free_pages() * PIPE_PAGE)
    }

    /// Account for `n` bytes of a `len`-byte write appended to `data`
    /// (`n` from [`fits`](Self::fits)).
    fn fill(&mut self, len: usize, n: usize) {
        let merge = self.merge(len).min(n);
        if let Some(p) = self.pages.back_mut().filter(|_| merge > 0) {
            p.filled += merge;
            p.left += merge;
        }
        let mut n = n - merge;
        while n > 0 {
            let k = n.min(PIPE_PAGE);
            self.pages.push_back(Page { filled: k, left: k, merge: true });
            n -= k;
        }
    }

    /// Account for `n` bytes taken from the front of `data`.
    fn drain(&mut self, mut n: usize) {
        while n > 0 {
            let Some(p) = self.pages.front_mut() else { break };
            let k = n.min(p.left);
            p.left -= k;
            n -= k;
            if p.left == 0 {
                self.pages.pop_front();
            }
        }
    }
}

impl Pipe {
    /// A new pipe and its two ends: `(read end, write end)`. `packet`:
    /// packet mode (`O_DIRECT`).
    pub fn pair(packet: bool) -> (PipeEnd, PipeEnd) {
        let pipe = Pipe::fifo_or_packet(packet);
        let read = PipeEnd::new(&pipe, true, false);
        let write = PipeEnd::new(&pipe, false, true);
        (read, write)
    }

    /// A pipe without ends yet, for a FIFO inode (`fifo.rs`).
    pub fn fifo() -> Arc<Pipe> {
        Pipe::fifo_or_packet(false)
    }

    fn fifo_or_packet(packet: bool) -> Arc<Pipe> {
        Arc::new(Pipe {
            state: SpinMutex::new(State {
                data: VecDeque::new(),
                pages: VecDeque::new(),
                slots: PIPE_CAPACITY / PIPE_PAGE,
                packet,
                readers: 0,
                writers: 0,
                read_opens: 0,
                write_opens: 0,
            }),
            queue: Arc::new(WaitQueue::new()),
        })
    }

    /// Read ends open now.
    pub fn readers(&self) -> usize {
        self.state.lock().readers
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

    /// What the pipe can hold, in bytes (`F_GETPIPE_SZ`).
    pub fn capacity(&self) -> usize {
        self.state.lock().slots * PIPE_PAGE
    }

    /// The bytes of the pages no write has started yet: what a transfer
    /// that fills whole pages, like sendfile(2) into a pipe, may put in now
    /// (a partly filled or partly read page counts as taken).
    pub fn free_room(&self) -> usize {
        self.state.lock().free_pages() * PIPE_PAGE
    }

    /// `F_SETPIPE_SZ`: at least `bytes`, rounded up to a power-of-two
    /// number of pages (at least one); returns the new capacity. `EINVAL`
    /// above [`PIPE_MAX_CAPACITY`], `EBUSY` if the pages in use would not
    /// fit (pipe2's spec sheet).
    pub fn set_capacity(&self, bytes: usize) -> Result<usize> {
        if bytes > PIPE_MAX_CAPACITY {
            return Err(Errno::EINVAL);
        }
        let slots = bytes.div_ceil(PIPE_PAGE).max(1).next_power_of_two();
        let mut s = self.state.lock();
        if s.pages.len() > slots {
            return Err(Errno::EBUSY);
        }
        let grew = slots > s.slots;
        s.slots = slots;
        drop(s);
        if grew {
            self.queue.notify(PollEvents::WRITABLE);
        }
        Ok(slots * PIPE_PAGE)
    }

    /// What the pipe holds and who has it open.
    pub fn status(&self) -> PipeStatus {
        let s = self.state.lock();
        PipeStatus { len: s.data.len(), free_pages: s.free_pages(), readers: s.readers, writers: s.writers }
    }

    /// Copy up to `buf.len()` of the bytes at the front out without taking
    /// them; returns how many.
    pub fn peek(&self, buf: &mut [u8]) -> usize {
        let s = self.state.lock();
        let n = s.data.len().min(buf.len());
        for (d, b) in buf[..n].iter_mut().zip(s.data.iter()) {
            *d = *b;
        }
        n
    }

    /// vmsplice(2) into the pipe: each of `pieces` (at most a page each,
    /// non-empty) becomes a buffer of its own, not merged with what is
    /// there; whole pieces go in, in order, while pages are free. Returns
    /// the bytes taken. `EPIPE` without read ends; `EAGAIN` when not even
    /// the first piece finds a free page.
    pub fn push_pieces(&self, pieces: &[&[u8]]) -> Result<usize> {
        let mut s = self.state.lock();
        if s.readers == 0 {
            return Err(Errno::EPIPE);
        }
        let mut n = 0;
        for piece in pieces.iter().filter(|p| !p.is_empty()) {
            if s.free_pages() == 0 {
                break;
            }
            debug_assert!(piece.len() <= PIPE_PAGE);
            s.data.extend(piece.iter());
            s.pages.push_back(Page { filled: piece.len(), left: piece.len(), merge: false });
            n += piece.len();
        }
        drop(s);
        if n == 0 {
            return Err(Errno::EAGAIN);
        }
        self.queue.notify(PollEvents::READABLE);
        Ok(n)
    }

    /// splice(2) (`keep` false) or tee(2) (`keep` true) between two pipes:
    /// up to `len` bytes from the front of this pipe go to the end of
    /// `out`, each of this pipe's buffers (or the part of it within `len`)
    /// becoming a buffer of `out`'s while `out` has free pages. tee(2)
    /// leaves this pipe as it was. In this order: an empty pipe with
    /// writers waits ([`Transfer::WaitInput`]), an `out` without a free
    /// page waits ([`Transfer::WaitOutput`]), an `out` without readers is
    /// `EPIPE`, an empty pipe without writers is the end
    /// ([`Transfer::Moved`] of 0). `EINVAL` if `out` is this pipe.
    pub fn transfer(&self, out: &Pipe, len: usize, keep: bool) -> Result<Transfer> {
        if core::ptr::eq(self, out) {
            return Err(Errno::EINVAL);
        }
        // Two locks, always taken in address order.
        let (mut src, mut dst) = if (self as *const Pipe) < (out as *const Pipe) {
            let a = self.state.lock();
            (a, out.state.lock())
        } else {
            let b = out.state.lock();
            (self.state.lock(), b)
        };
        if src.data.is_empty() && src.writers > 0 {
            return Ok(Transfer::WaitInput);
        }
        if dst.free_pages() == 0 {
            return Ok(Transfer::WaitOutput);
        }
        if dst.readers == 0 {
            return Err(Errno::EPIPE);
        }
        let mut moved = 0;
        let mut at = 0;
        let mut page = 0;
        while moved < len && page < src.pages.len() && dst.free_pages() > 0 {
            let k = src.pages[page].left.min(len - moved);
            dst.data.extend(src.data.range(at..at + k));
            dst.pages.push_back(Page { filled: k, left: k, merge: false });
            moved += k;
            at += k;
            page += 1;
        }
        if !keep {
            src.data.drain(..moved);
            src.drain(moved);
        }
        drop((src, dst));
        if moved > 0 {
            out.queue.notify(PollEvents::READABLE);
            if !keep {
                self.queue.notify(PollEvents::WRITABLE);
            }
        }
        Ok(Transfer::Moved(moved))
    }
}

/// What [`Pipe::status`] tells.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct PipeStatus {
    /// Bytes in the pipe.
    pub len: usize,
    /// Pages no write has started yet.
    pub free_pages: usize,
    pub readers: usize,
    pub writers: usize,
}

/// What [`Pipe::transfer`] did, or what it waits for.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Transfer {
    /// This many bytes went (0: the input is empty and has no writers).
    Moved(usize),
    /// The input is empty but has writers.
    WaitInput,
    /// The output has no free page.
    WaitOutput,
}

impl core::fmt::Debug for Pipe {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        let s = self.state.lock();
        f.debug_struct("Pipe")
            .field("len", &s.data.len())
            .field("readers", &s.readers)
            .field("writers", &s.writers)
            .field("packet", &s.packet)
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
        s.read_opens += u64::from(read);
        s.write_opens += u64::from(write);
        drop(s);
        // A FIFO open waiting for this side wakes up.
        pipe.queue.notify(PollEvents::READABLE | PollEvents::WRITABLE);
        PipeEnd { pipe: Arc::clone(pipe), read, write }
    }

    /// fifo(7): a blocking open of a FIFO for reading waits for a writer,
    /// one for writing for a reader (an end that does both waits for
    /// nothing): until such an end is open, or one was opened since this
    /// one. `EINTR` if `blocker` is interrupted.
    pub fn wait_for_peer(&self, blocker: &mut dyn Blocker) -> Result<()> {
        if self.read && self.write {
            return Ok(());
        }
        let opens = |s: &State| if self.read { s.write_opens } else { s.read_opens };
        let start = opens(&self.pipe.state.lock());
        let ready = || {
            let s = self.pipe.state.lock();
            (if self.read { s.writers } else { s.readers }) > 0 || opens(&s) != start
        };
        if ready() {
            return Ok(());
        }
        let _registered = self.pipe.queue.register(PollEvents::READABLE | PollEvents::WRITABLE, blocker.waker());
        while !ready() {
            if blocker.block(None) == Woke::Interrupted {
                return Err(Errno::EINTR);
            }
        }
        Ok(())
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
        let n = if s.packet {
            let packet = s.pages.pop_front().map_or(0, |p| p.left);
            let n = packet.min(buf.len());
            take(&mut s.data, &mut buf[..n]);
            // pipe(7): "the excess bytes in the packet are discarded".
            s.data.drain(..packet - n);
            n
        } else {
            let n = s.data.len().min(buf.len());
            take(&mut s.data, &mut buf[..n]);
            s.drain(n);
            n
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
        let n = s.fits(buf.len());
        // At most PIPE_BUF bytes: all or nothing; more: whatever fits.
        if n == 0 || (buf.len() <= PIPE_BUF && n < buf.len()) {
            return Err(Errno::EAGAIN);
        }
        s.data.extend(&buf[..n]);
        // In packet mode each packet of at most PIPE_BUF bytes takes a page.
        s.fill(buf.len(), n);
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
            // Without readers a write would not block (it fails with
            // EPIPE): POLLERR and POLLOUT (poll's spec sheet).
            if s.readers == 0 {
                events = events | PollEvents::ERR | PollEvents::WRITABLE;
            } else if s.free_pages() > 0 {
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

#[cfg(test)]
mod tests {
    use super::*;
    use alloc::vec::Vec;

    fn write(end: &PipeEnd, data: &[u8]) {
        assert_eq!(end.write(data), Ok(data.len()));
    }

    fn read_all(end: &PipeEnd) -> Vec<u8> {
        let mut buf = alloc::vec![0u8; 1 << 21];
        let n = end.read(&mut buf).unwrap_or(0);
        buf.truncate(n);
        buf
    }

    #[test]
    fn splice_moves_buffers_and_stops_inside_one() {
        let (pr, pw) = Pipe::pair(false);
        let (qr, qw) = Pipe::pair(false);
        write(&pw, b"0123456789");
        assert_eq!(pr.pipe().transfer(qw.pipe(), 4, false), Ok(Transfer::Moved(4)));
        assert_eq!((pr.pipe().len(), qr.pipe().len()), (6, 4));
        assert_eq!(pr.pipe().transfer(qw.pipe(), 100, false), Ok(Transfer::Moved(6)));
        assert_eq!(read_all(&qr), b"0123456789");
        // Empty with a writer waits; without writers it is the end.
        assert_eq!(pr.pipe().transfer(qw.pipe(), 1, false), Ok(Transfer::WaitInput));
        drop(pw);
        assert_eq!(pr.pipe().transfer(qw.pipe(), 1, false), Ok(Transfer::Moved(0)));
        assert_eq!(pr.pipe().transfer(pr.pipe(), 1, false), Err(Errno::EINVAL));
    }

    #[test]
    fn tee_keeps_the_input_and_spans_buffers() {
        let (pr, pw) = Pipe::pair(false);
        let (qr, qw) = Pipe::pair(false);
        write(&pw, b"aaa");
        write(&pw, b"bbbb");
        assert_eq!(pr.pipe().transfer(qw.pipe(), 5, true), Ok(Transfer::Moved(5)));
        assert_eq!(pr.pipe().len(), 7);
        assert_eq!(read_all(&qr), b"aaabb");
        let mut peek = [0u8; 4];
        assert_eq!(pr.pipe().peek(&mut peek), 4);
        assert_eq!(&peek, b"aaab");
        assert_eq!(read_all(&pr), b"aaabbbb");
    }

    #[test]
    fn output_room_is_counted_in_buffers_and_waited_for_before_epipe() {
        let (pr, pw) = Pipe::pair(false);
        let (_qr, qw) = Pipe::pair(false);
        pr.pipe().set_capacity(1 << 20).unwrap();
        let big = alloc::vec![7u8; 1 << 20];
        let mut n = 0;
        while n < big.len() {
            n += pw.write(&big[n..]).unwrap();
        }
        assert_eq!(pr.pipe().transfer(qw.pipe(), 1 << 20, true), Ok(Transfer::Moved(PIPE_CAPACITY)));
        assert_eq!(pr.pipe().transfer(qw.pipe(), 1, true), Ok(Transfer::WaitOutput));
        // No reader on the output: EPIPE, and the input keeps its bytes.
        let (zr, zw) = Pipe::pair(false);
        let (yr, yw) = Pipe::pair(false);
        drop(yr);
        write(&zw, b"abc");
        assert_eq!(zr.pipe().transfer(yw.pipe(), 3, false), Err(Errno::EPIPE));
        assert_eq!(zr.pipe().len(), 3);
    }

    #[test]
    fn vmsplice_pieces_take_a_buffer_each() {
        let (pr, pw) = Pipe::pair(false);
        let pieces: Vec<&[u8]> = (0..20).map(|_| &b"x"[..]).collect();
        assert_eq!(pw.pipe().push_pieces(&pieces), Ok(16));
        assert_eq!(pw.pipe().push_pieces(&pieces), Err(Errno::EAGAIN));
        assert_eq!(read_all(&pr).len(), 16);
        write(&pw, b"y");
        assert_eq!(pw.pipe().push_pieces(&pieces), Ok(15));
        read_all(&pr);
        // A piece is not merged into by a later write.
        assert_eq!(pw.pipe().push_pieces(&[b"ab"]), Ok(2));
        write(&pw, b"c");
        assert_eq!(pw.pipe().status().free_pages, 14);
        drop(pr);
        assert_eq!(pw.pipe().push_pieces(&[b"z"]), Err(Errno::EPIPE));
    }
}

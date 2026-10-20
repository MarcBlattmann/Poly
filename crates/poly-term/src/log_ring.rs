//! The kernel log's most recent bytes.

use core::fmt;

/// The last `N` bytes written, oldest overwritten first.
///
/// Readers do not take bytes out: each keeps its own position (bytes ever
/// written, [`written`](Self::written) is the end) and reads from there
/// with [`read_from`](Self::read_from), which says how much was lost if the
/// writer lapped it. So a reader that starts late (the screen terminal,
/// which comes up after the boot log) still gets the last `N` bytes.
///
/// It is plain data; the kernel keeps it behind a lock that writers and
/// readers take.
#[derive(Clone)]
pub struct LogRing<const N: usize> {
    buf: [u8; N],
    /// Bytes ever written; the next goes to `written % N`.
    written: u64,
}

/// What [`LogRing::read_from`] returned.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct LogRead {
    /// Bytes copied out.
    pub len: usize,
    /// Bytes between the position asked for and the oldest one still kept:
    /// overwritten before they were read.
    pub lost: u64,
    /// Where to read from next time.
    pub next: u64,
}

impl<const N: usize> fmt::Debug for LogRing<N> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("LogRing").field("capacity", &N).field("written", &self.written).finish_non_exhaustive()
    }
}

impl<const N: usize> Default for LogRing<N> {
    fn default() -> Self {
        Self::new()
    }
}

impl<const N: usize> LogRing<N> {
    pub const fn new() -> Self {
        assert!(N > 0, "a ring needs room for a byte");
        LogRing { buf: [0; N], written: 0 }
    }

    /// Bytes ever written: the position after the newest byte.
    pub const fn written(&self) -> u64 {
        self.written
    }

    /// Position of the oldest byte still kept.
    pub const fn oldest(&self) -> u64 {
        self.written.saturating_sub(N as u64)
    }

    pub fn push(&mut self, mut bytes: &[u8]) {
        // Only the last N can survive.
        if bytes.len() > N {
            self.written += (bytes.len() - N) as u64;
            bytes = &bytes[bytes.len() - N..];
        }
        while !bytes.is_empty() {
            let at = (self.written % N as u64) as usize;
            let n = bytes.len().min(N - at);
            self.buf[at..at + n].copy_from_slice(&bytes[..n]);
            self.written += n as u64;
            bytes = &bytes[n..];
        }
    }

    /// Copy bytes from position `from` on into `out`, as many as fit. If
    /// `from` is older than what the ring keeps, reading starts at the
    /// oldest byte and `lost` counts the gap; a position past the end
    /// reads nothing (and `next` is the end).
    pub fn read_from(&self, from: u64, out: &mut [u8]) -> LogRead {
        let start = from.max(self.oldest()).min(self.written);
        let lost = start - from.min(start);
        let len = ((self.written - start) as usize).min(out.len());
        let mut done = 0;
        while done < len {
            let at = ((start + done as u64) % N as u64) as usize;
            let n = (len - done).min(N - at);
            out[done..done + n].copy_from_slice(&self.buf[at..at + n]);
            done += n;
        }
        LogRead { len, lost, next: start + len as u64 }
    }
}

impl<const N: usize> fmt::Write for LogRing<N> {
    fn write_str(&mut self, s: &str) -> fmt::Result {
        self.push(s.as_bytes());
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use core::fmt::Write;
    use std::vec::Vec;

    fn read_all<const N: usize>(r: &LogRing<N>, from: u64) -> (Vec<u8>, LogRead) {
        let mut out = [0u8; 64];
        let got = r.read_from(from, &mut out);
        (out[..got.len].to_vec(), got)
    }

    #[test]
    fn reads_what_was_written_in_order() {
        let mut r = LogRing::<16>::new();
        let _ = write!(r, "hello {}", 42);
        assert_eq!(r.written(), 8);
        let (bytes, got) = read_all(&r, 0);
        assert_eq!((bytes.as_slice(), got.lost, got.next), (&b"hello 42"[..], 0, 8));
        let (bytes, got) = read_all(&r, 6);
        assert_eq!((bytes.as_slice(), got.next), (&b"42"[..], 8));
        assert_eq!(read_all(&r, 8).1, LogRead { len: 0, lost: 0, next: 8 });
        assert_eq!(read_all(&r, 100).1, LogRead { len: 0, lost: 0, next: 8 }, "a position past the end");
    }

    #[test]
    fn overwrites_the_oldest_and_reports_the_loss() {
        let mut r = LogRing::<8>::new();
        r.push(b"abcdef");
        r.push(b"ghij");
        assert_eq!((r.oldest(), r.written()), (2, 10));
        let (bytes, got) = read_all(&r, 0);
        assert_eq!((bytes.as_slice(), got.lost, got.next), (&b"cdefghij"[..], 2, 10));
        let (bytes, got) = read_all(&r, 5);
        assert_eq!((bytes.as_slice(), got.lost), (&b"fghij"[..], 0));
        // One push longer than the ring keeps its end.
        r.push(b"0123456789xyz");
        let (bytes, got) = read_all(&r, 10);
        assert_eq!((bytes.as_slice(), got.lost, got.next), (&b"56789xyz"[..], 5, 23));
    }

    #[test]
    fn short_buffers_read_in_pieces() {
        let mut r = LogRing::<8>::new();
        r.push(b"abcdefghijk");
        let mut out = [0u8; 3];
        let mut pos = 0;
        let mut text = Vec::new();
        let mut lost = 0;
        loop {
            let got = r.read_from(pos, &mut out);
            if got.len == 0 {
                break;
            }
            text.extend_from_slice(&out[..got.len]);
            lost += got.lost;
            pos = got.next;
        }
        assert_eq!((text.as_slice(), lost), (&b"defghijk"[..], 3));
    }
}

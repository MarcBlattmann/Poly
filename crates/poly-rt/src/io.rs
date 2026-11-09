//! Output, handles and the [`Read`] / [`Write`] traits: [`Stdout`],
//! [`Stderr`], [`write_all`], [`control`], and the [`print!`](crate::print)
//! family's [`_print`].

use crate::abi::{Error, Handle, STDERR, STDOUT};
use crate::{Result, sys};
use alloc::string::String;
use alloc::vec::Vec;
use core::fmt;

/// Write all of `buf` to `handle`, however many writes it takes.
pub fn write_all(handle: Handle, mut buf: &[u8]) -> Result<()> {
    while !buf.is_empty() {
        match sys::write(handle, buf) {
            Ok(0) => return Err(Error::BrokenPipe),
            Ok(n) => buf = &buf[n..],
            Err(Error::Interrupted) => {}
            Err(e) => return Err(e),
        }
    }
    Ok(())
}

/// A device request on `handle` (its ioctl: DRM, evdev, frame buffer and
/// sound devices take their own request numbers and structures).
///
/// # Safety
///
/// `arg` is what the request takes: a pointer must point at memory of the
/// size and layout the device reads and writes.
pub unsafe fn control(handle: Handle, request: u32, arg: u64) -> Result<u64> {
    // SAFETY: passed on from the caller.
    unsafe { sys::control(handle, request, arg) }
}

/// Something bytes are read from (std's `Read`, for handles).
pub trait Read {
    /// Read into `buf`; 0 at the end.
    fn read(&mut self, buf: &mut [u8]) -> Result<usize>;

    /// Fill `buf` exactly; `BrokenPipe` if the end comes first.
    fn read_exact(&mut self, mut buf: &mut [u8]) -> Result<()> {
        while !buf.is_empty() {
            match self.read(buf) {
                Ok(0) => return Err(Error::BrokenPipe),
                Ok(n) => buf = &mut buf[n..],
                Err(Error::Interrupted) => {}
                Err(e) => return Err(e),
            }
        }
        Ok(())
    }

    /// Read until the end, appending to `out`; returns the bytes read.
    fn read_to_end(&mut self, out: &mut Vec<u8>) -> Result<usize> {
        let start = out.len();
        let mut chunk = [0u8; 4096];
        loop {
            match self.read(&mut chunk) {
                Ok(0) => return Ok(out.len() - start),
                Ok(n) => out.extend_from_slice(&chunk[..n]),
                Err(Error::Interrupted) => {}
                Err(e) => return Err(e),
            }
        }
    }
}

/// Something bytes are written to (std's `Write`, for handles).
pub trait Write {
    /// Write some of `buf`; returns how much.
    fn write(&mut self, buf: &[u8]) -> Result<usize>;

    fn write_all(&mut self, mut buf: &[u8]) -> Result<()> {
        while !buf.is_empty() {
            match self.write(buf) {
                Ok(0) => return Err(Error::BrokenPipe),
                Ok(n) => buf = &buf[n..],
                Err(Error::Interrupted) => {}
                Err(e) => return Err(e),
            }
        }
        Ok(())
    }

    /// Nothing is buffered: a no-op, for code written against std.
    fn flush(&mut self) -> Result<()> {
        Ok(())
    }
}

/// The standard output, unbuffered: each `write_str` is a `write`.
#[derive(Clone, Copy, Debug, Default)]
pub struct Stdout;

/// The standard error, unbuffered (what the panic handler writes to).
#[derive(Clone, Copy, Debug, Default)]
pub struct Stderr;

impl fmt::Write for Stdout {
    fn write_str(&mut self, s: &str) -> fmt::Result {
        write_all(STDOUT, s.as_bytes()).map_err(|_| fmt::Error)
    }
}

impl fmt::Write for Stderr {
    fn write_str(&mut self, s: &str) -> fmt::Result {
        write_all(STDERR, s.as_bytes()).map_err(|_| fmt::Error)
    }
}

/// Format `args` into one string, so a line goes out in one `write` and
/// does not mix with another process's output on the same console.
pub fn format(args: fmt::Arguments<'_>) -> String {
    let mut s = String::new();
    let _ = fmt::write(&mut s, args);
    s
}

/// What [`print!`](crate::print) and friends call.
#[doc(hidden)]
pub fn _print(handle: Handle, args: fmt::Arguments<'_>) {
    let _ = write_all(handle, format(args).as_bytes());
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_line_is_formatted_in_one_piece() {
        assert_eq!(format(format_args!("{}-{:03}\n", "pid", 7)), "pid-007\n");
    }

    #[test]
    fn writes_fail_on_the_host() {
        assert_eq!(write_all(STDOUT, b"x"), Err(Error::Unsupported));
        assert_eq!(write_all(STDOUT, b""), Ok(()));
    }

    /// A reader of a fixed byte string, in pieces.
    struct Pieces(&'static [u8], usize);

    impl Read for Pieces {
        fn read(&mut self, buf: &mut [u8]) -> Result<usize> {
            let n = buf.len().min(self.1).min(self.0.len());
            buf[..n].copy_from_slice(&self.0[..n]);
            self.0 = &self.0[n..];
            Ok(n)
        }
    }

    #[test]
    fn read_exact_and_to_end_gather_pieces() {
        let mut b = [0u8; 5];
        Pieces(b"hello world", 2).read_exact(&mut b).unwrap();
        assert_eq!(&b, b"hello");
        assert_eq!(Pieces(b"hi", 1).read_exact(&mut b), Err(Error::BrokenPipe));
        let mut all = Vec::new();
        assert_eq!(Pieces(b"hello world", 3).read_to_end(&mut all), Ok(11));
        assert_eq!(all, b"hello world");
    }
}

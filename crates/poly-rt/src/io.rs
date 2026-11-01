//! Output and handles: [`Stdout`], [`Stderr`], [`write_all`], and the
//! [`print!`](crate::print) family's [`_print`].

use crate::abi::{Error, Handle, STDERR, STDOUT};
use crate::{Result, sys};
use alloc::string::String;
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
}

//! The compositor's error type: a native error ([`ErrorKind`], what Poly's
//! calls report) with an optional message, in the shape of `std::io`'s so
//! the code reads as it did on std (ADR 0125).

use alloc::string::String;
use core::fmt;

pub use poly_rt::Error as ErrorKind;
pub use poly_rt::io::{Read, Write};

pub type Result<T> = core::result::Result<T, Error>;

/// What went wrong: the kind, and words for it if the kind is not enough.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Error {
    kind: ErrorKind,
    message: Option<String>,
}

impl Error {
    pub fn new(kind: ErrorKind, message: impl Into<String>) -> Error {
        Error { kind, message: Some(message.into()) }
    }

    /// An error with only a message.
    pub fn other(message: impl Into<String>) -> Error {
        Error::new(ErrorKind::Io, message)
    }

    /// The error for a Linux error number (a device model's, in tests).
    pub fn from_raw_os_error(errno: i32) -> Error {
        ErrorKind::from_errno(errno as u16).into()
    }

    pub fn kind(&self) -> ErrorKind {
        self.kind
    }
}

impl From<ErrorKind> for Error {
    fn from(kind: ErrorKind) -> Error {
        Error { kind, message: None }
    }
}

impl fmt::Display for Error {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match &self.message {
            Some(m) => f.write_str(m),
            None => write!(f, "{}", self.kind),
        }
    }
}

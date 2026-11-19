//! Why a program could not be linked.

use alloc::string::String;
use core::fmt;

/// Why [`link`](crate::link) failed. Every variant names the file or
/// symbol, the way dyld's own messages do, so a log line says what to fix.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Error {
    /// The file at `path` is not a Mach-O file Poly can load.
    Macho { path: String, error: poly_macho::Error },
    /// The file at `path` has no slice for the CPU being loaded.
    WrongCpu { path: String },
    /// The main file is not an `MH_EXECUTE` program, or a dependency is
    /// not a dylib.
    WrongFileType { path: String },
    /// The program has neither `LC_MAIN` nor `LC_UNIXTHREAD`.
    NoEntry { path: String },
    /// A required library was not found: `name` as `from` asks for it.
    LibraryNotFound { name: String, from: String },
    /// A symbol was not found where the image that imports it says.
    SymbolNotFound { symbol: String, from: String, expected_in: String },
    /// A segment cannot be mapped: not page aligned, or it overlaps
    /// another image.
    BadLayout { path: String, why: &'static str },
    /// A fixup does not fit Poly yet (a 32-bit text relocation, a
    /// re-export chain that loops).
    Unsupported { path: String, why: &'static str },
    /// More images than [`MAX_IMAGES`](crate::MAX_IMAGES).
    TooManyImages,
}

impl fmt::Display for Error {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Error::Macho { path, error } => write!(f, "{path}: not a loadable Mach-O file ({error:?})"),
            Error::WrongCpu { path } => write!(f, "{path}: no slice for this CPU"),
            Error::WrongFileType { path } => write!(f, "{path}: wrong Mach-O file type"),
            Error::NoEntry { path } => write!(f, "{path}: no entry point (LC_MAIN or LC_UNIXTHREAD)"),
            Error::LibraryNotFound { name, from } => write!(f, "Library not loaded: {name}, referenced from {from}"),
            Error::SymbolNotFound { symbol, from, expected_in } => {
                write!(f, "Symbol not found: {symbol}, referenced from {from}, expected in {expected_in}")
            }
            Error::BadLayout { path, why } => write!(f, "{path}: {why}"),
            Error::Unsupported { path, why } => write!(f, "{path}: {why}"),
            Error::TooManyImages => f.write_str("too many images"),
        }
    }
}

/// `Result` with [`Error`].
pub type Result<T> = core::result::Result<T, Error>;

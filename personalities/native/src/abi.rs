//! The native ABI: what a native program and the kernel agree on (ADR 0125).
//!
//! Both sides use this module: the kernel's dispatcher ([`crate::Native`])
//! and the runtime native programs link (`poly-rt`). Nothing here comes
//! from another operating system: the numbers, error codes, flags and
//! structures are Poly's own.
//!
//! # Calling convention
//!
//! | | |
//! |---|---|
//! | instruction | `syscall` |
//! | call number | `rax` ([`nr`]) |
//! | arguments | `rdi`, `rsi`, `rdx`, `r10`, `r8`, `r9` |
//! | result | `rax`: a value, or `-code` of an [`Error`] (`-4095..=-1`) |
//! | clobbered | `rcx`, `r11` (by the instruction); every other register is kept |
//!
//! Strings and buffers are passed as a pointer and a length, never NUL
//! terminated; arrays of strings as a pointer to [`Str`] records and a
//! count.

pub use poly_binfmt::{NOTE_OWNER, NT_POLY_ABI};

/// The native ABI version this crate describes, the one `poly-rt` writes
/// into its programs' ABI note. The kernel runs programs of versions
/// `1..=ABI_VERSION`.
pub const ABI_VERSION: u32 = 1;

/// The call numbers (`rax`).
pub mod nr {
    /// `exit(code)`: end the process with status `code & 0xff`. Never returns.
    pub const EXIT: u64 = 0;
    /// `write(handle, buf, len) -> written`.
    pub const WRITE: u64 = 1;
    /// `read(handle, buf, len) -> read` (0 at the end of a file).
    pub const READ: u64 = 2;
    /// `open(path, path_len, flags, mode) -> handle` ([`super::open`] flags).
    pub const OPEN: u64 = 3;
    /// `close(handle)`.
    pub const CLOSE: u64 = 4;
    /// `dup(handle) -> handle`: a second handle on the same open file.
    pub const DUP: u64 = 5;
    /// `seek(handle, offset, whence) -> position` ([`super::Whence`]).
    pub const SEEK: u64 = 6;
    /// `map(hint, len, prot, flags) -> address`: private zeroed memory
    /// ([`super::prot`]; `flags` must be 0).
    pub const MAP: u64 = 7;
    /// `unmap(address, len)`.
    pub const UNMAP: u64 = 8;
    /// `protect(address, len, prot)`.
    pub const PROTECT: u64 = 9;
    /// `clock(id) -> nanoseconds` ([`super::Clock`]).
    pub const CLOCK: u64 = 10;
    /// `sleep(nanoseconds)`.
    pub const SLEEP: u64 = 11;
    /// `yield()`: let other threads run.
    pub const YIELD: u64 = 12;
    /// `spawn(request, request_size) -> pid` ([`super::SpawnRequest`]).
    pub const SPAWN: u64 = 13;
    /// `wait(pid) -> status`: wait for a child to end ([`super::ExitStatus`]).
    pub const WAIT: u64 = 14;
    /// `process_id() -> pid`.
    pub const PROCESS_ID: u64 = 15;
}

/// The largest error code: results in `-MAX_ERROR..=-1` are errors.
pub const MAX_ERROR: u64 = 4095;

/// Why a call failed: `rax` is `-(code)`.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
#[repr(u16)]
pub enum Error {
    /// No such call, or not supported on this object.
    Unsupported = 1,
    InvalidArgument = 2,
    /// Not an open handle.
    BadHandle = 3,
    NotFound = 4,
    PermissionDenied = 5,
    AlreadyExists = 6,
    NoMemory = 7,
    /// A pointer that is not mapped user memory with the access needed.
    BadAddress = 8,
    /// A non-blocking handle has nothing to give or take now.
    WouldBlock = 9,
    /// The wait was cut short (the process is ending).
    Interrupted = 10,
    TimedOut = 11,
    /// The other end is gone.
    BrokenPipe = 12,
    NotADirectory = 13,
    IsADirectory = 14,
    NoSpace = 15,
    /// A name, buffer or argument list over its limit.
    TooLarge = 16,
    Busy = 17,
    /// No such child to wait for.
    NoChild = 18,
    /// The file is not a program Poly can run.
    NotExecutable = 19,
    /// Every other failure of the device or file system.
    Io = 20,
    TooManyHandles = 21,
    ConnectionRefused = 22,
}

impl Error {
    /// Every error, in code order.
    pub const ALL: [Error; 22] = [
        Error::Unsupported,
        Error::InvalidArgument,
        Error::BadHandle,
        Error::NotFound,
        Error::PermissionDenied,
        Error::AlreadyExists,
        Error::NoMemory,
        Error::BadAddress,
        Error::WouldBlock,
        Error::Interrupted,
        Error::TimedOut,
        Error::BrokenPipe,
        Error::NotADirectory,
        Error::IsADirectory,
        Error::NoSpace,
        Error::TooLarge,
        Error::Busy,
        Error::NoChild,
        Error::NotExecutable,
        Error::Io,
        Error::TooManyHandles,
        Error::ConnectionRefused,
    ];

    pub const fn code(self) -> u16 {
        self as u16
    }

    /// The error with `code`; unknown codes (a newer kernel's) are [`Error::Io`].
    pub fn from_code(code: u16) -> Error {
        Error::ALL.iter().copied().find(|e| e.code() == code).unwrap_or(Error::Io)
    }

    pub const fn name(self) -> &'static str {
        match self {
            Error::Unsupported => "unsupported",
            Error::InvalidArgument => "invalid argument",
            Error::BadHandle => "bad handle",
            Error::NotFound => "not found",
            Error::PermissionDenied => "permission denied",
            Error::AlreadyExists => "already exists",
            Error::NoMemory => "out of memory",
            Error::BadAddress => "bad address",
            Error::WouldBlock => "would block",
            Error::Interrupted => "interrupted",
            Error::TimedOut => "timed out",
            Error::BrokenPipe => "broken pipe",
            Error::NotADirectory => "not a directory",
            Error::IsADirectory => "is a directory",
            Error::NoSpace => "no space left",
            Error::TooLarge => "too large",
            Error::Busy => "busy",
            Error::NoChild => "no such child",
            Error::NotExecutable => "not executable",
            Error::Io => "input/output error",
            Error::TooManyHandles => "too many handles",
            Error::ConnectionRefused => "connection refused",
        }
    }

    /// The native error for an error number of the kernel's shared objects
    /// (the VFS, sockets, pipes), which use Linux's numbering because the
    /// Linux personality hands them out unchanged (`asm-generic/errno-base.h`,
    /// `asm-generic/errno.h` of the Linux UAPI headers). Numbers without a
    /// native counterpart are [`Error::Io`].
    pub const fn from_errno(errno: u16) -> Error {
        match errno {
            1 | 13 | 30 => Error::PermissionDenied, // EPERM, EACCES, EROFS
            2 | 3 | 6 | 19 => Error::NotFound,      // ENOENT, ESRCH, ENXIO, ENODEV
            4 => Error::Interrupted,                // EINTR
            7 | 27 | 36 | 75 => Error::TooLarge,    // E2BIG, EFBIG, ENAMETOOLONG, EOVERFLOW
            8 => Error::NotExecutable,              // ENOEXEC
            9 => Error::BadHandle,                  // EBADF
            10 => Error::NoChild,                   // ECHILD
            11 => Error::WouldBlock,                // EAGAIN
            12 => Error::NoMemory,                  // ENOMEM
            14 => Error::BadAddress,                // EFAULT
            16 | 26 => Error::Busy,                 // EBUSY, ETXTBSY
            17 | 98 => Error::AlreadyExists,        // EEXIST, EADDRINUSE
            20 => Error::NotADirectory,             // ENOTDIR
            21 => Error::IsADirectory,              // EISDIR
            22 | 34 | 40 => Error::InvalidArgument, // EINVAL, ERANGE, ELOOP
            23 | 24 => Error::TooManyHandles,       // ENFILE, EMFILE
            28 | 122 => Error::NoSpace,             // ENOSPC, EDQUOT
            29 | 38 | 95 => Error::Unsupported,     // ESPIPE, ENOSYS, EOPNOTSUPP
            32 | 104 => Error::BrokenPipe,          // EPIPE, ECONNRESET
            110 => Error::TimedOut,                 // ETIMEDOUT
            111 => Error::ConnectionRefused,        // ECONNREFUSED
            _ => Error::Io,
        }
    }
}

impl core::fmt::Display for Error {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.write_str(self.name())
    }
}

/// A call's result as it goes into `rax`.
pub const fn encode(result: Result<u64, Error>) -> u64 {
    match result {
        Ok(value) => value,
        Err(e) => (e.code() as u64).wrapping_neg(),
    }
}

/// `rax` after a call, as a result.
pub fn decode(rax: u64) -> Result<u64, Error> {
    if rax >= MAX_ERROR.wrapping_neg() { Err(Error::from_code(rax.wrapping_neg() as u16)) } else { Ok(rax) }
}

/// A handle: a small number naming one of the process's open objects. The
/// objects are the kernel's shared open files, the ones Linux processes
/// see as file descriptors, so the same pipe, socket or device can be open
/// in native and Linux processes at once. 0, 1 and 2 are the standard
/// input, output and error.
pub type Handle = u32;

/// The standard input, output and error.
pub const STDIN: Handle = 0;
pub const STDOUT: Handle = 1;
pub const STDERR: Handle = 2;

/// `open` flags.
pub mod open {
    pub const READ: u32 = 1 << 0;
    pub const WRITE: u32 = 1 << 1;
    /// Create the file if it is not there (with the `mode` given).
    pub const CREATE: u32 = 1 << 2;
    /// With [`CREATE`]: fail with `AlreadyExists` if it is there.
    pub const EXCLUSIVE: u32 = 1 << 3;
    /// Cut the file to length 0 (needs [`WRITE`]).
    pub const TRUNCATE: u32 = 1 << 4;
    /// Every write goes to the end.
    pub const APPEND: u32 = 1 << 5;
    /// Reads and writes that would wait fail with `WouldBlock` instead.
    pub const NONBLOCK: u32 = 1 << 6;
    /// Fail with `NotADirectory` unless it is a directory.
    pub const DIRECTORY: u32 = 1 << 7;
    /// Every flag this ABI version knows.
    pub const ALL: u32 = (1 << 8) - 1;
}

/// `map` / `protect` protection bits.
pub mod prot {
    pub const READ: u32 = 1 << 0;
    pub const WRITE: u32 = 1 << 1;
    pub const EXEC: u32 = 1 << 2;
    pub const ALL: u32 = READ | WRITE | EXEC;
}

/// The page size: `map`, `unmap` and `protect` work in whole pages.
pub const PAGE_SIZE: u64 = 4096;

/// Where `seek` counts from.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[repr(u32)]
pub enum Whence {
    Start = 0,
    Current = 1,
    End = 2,
}

impl Whence {
    pub const fn from_u64(v: u64) -> Option<Whence> {
        match v {
            0 => Some(Whence::Start),
            1 => Some(Whence::Current),
            2 => Some(Whence::End),
            _ => None,
        }
    }
}

/// The clocks of `clock`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[repr(u32)]
pub enum Clock {
    /// Nanoseconds since boot; never goes back.
    Monotonic = 0,
    /// Nanoseconds since 1970-01-01 00:00 UTC.
    Realtime = 1,
}

impl Clock {
    pub const fn from_u64(v: u64) -> Option<Clock> {
        match v {
            0 => Some(Clock::Monotonic),
            1 => Some(Clock::Realtime),
            _ => None,
        }
    }
}

/// A string or buffer in the caller's memory.
#[repr(C)]
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Str {
    pub ptr: u64,
    pub len: u64,
}

impl Str {
    pub const SIZE: usize = 16;

    pub fn from_bytes(b: [u8; Self::SIZE]) -> Str {
        let (ptr, len) = b.split_at(8);
        Str { ptr: u64::from_le_bytes(ptr.try_into().unwrap()), len: u64::from_le_bytes(len.try_into().unwrap()) }
    }
}

/// In [`SpawnRequest::stdio`]: the child gets no handle there.
pub const STDIO_NONE: u32 = u32::MAX;

/// What `spawn` starts: a program at `path` with its arguments
/// (`args[0]` is its name by convention) and environment (`NAME=value`).
/// The child gets exactly three handles, 0, 1 and 2: the caller's handles
/// `stdio[0..3]` (shared, as `dup` shares them), or none where an entry is
/// [`STDIO_NONE`]. Any program Poly runs can be spawned: a native one, or
/// a Linux, Windows or macOS program in its own personality.
#[repr(C)]
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct SpawnRequest {
    pub path: Str,
    /// `*const Str`, `arg_count` of them.
    pub args: u64,
    pub arg_count: u64,
    /// `*const Str`, `env_count` of them.
    pub env: u64,
    pub env_count: u64,
    pub stdio: [u32; 3],
    /// Must be 0.
    pub flags: u32,
}

impl SpawnRequest {
    /// The size `spawn`'s second argument must give (this ABI version's).
    pub const SIZE: usize = 64;

    pub fn from_bytes(b: &[u8; Self::SIZE]) -> SpawnRequest {
        let u64_at = |at: usize| u64::from_le_bytes(b[at..at + 8].try_into().unwrap());
        let u32_at = |at: usize| u32::from_le_bytes(b[at..at + 4].try_into().unwrap());
        SpawnRequest {
            path: Str { ptr: u64_at(0), len: u64_at(8) },
            args: u64_at(16),
            arg_count: u64_at(24),
            env: u64_at(32),
            env_count: u64_at(40),
            stdio: [u32_at(48), u32_at(52), u32_at(56)],
            flags: u32_at(60),
        }
    }
}

/// How a child ended, as `wait` returns it: bits 0..8 the value, bits
/// 8..16 the kind (0 exited with the value as its status, 1 killed by
/// signal value (a Linux child), 2 killed by CPU exception value).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ExitStatus {
    Exited(u8),
    Signaled(u8),
    Faulted(u8),
}

impl ExitStatus {
    pub const fn encode(self) -> u64 {
        match self {
            ExitStatus::Exited(v) => v as u64,
            ExitStatus::Signaled(v) => 1 << 8 | v as u64,
            ExitStatus::Faulted(v) => 2 << 8 | v as u64,
        }
    }

    pub const fn decode(v: u64) -> Option<ExitStatus> {
        let value = v as u8;
        match v >> 8 {
            0 => Some(ExitStatus::Exited(value)),
            1 => Some(ExitStatus::Signaled(value)),
            2 => Some(ExitStatus::Faulted(value)),
            _ => None,
        }
    }

    /// Whether it exited with status 0.
    pub const fn success(self) -> bool {
        matches!(self, ExitStatus::Exited(0))
    }
}

/// Limits the kernel checks.
pub mod limits {
    /// Bytes in a path.
    pub const PATH: u64 = 4096;
    /// Bytes one `read` or `write` moves at most (a larger request is
    /// a short transfer, as a pipe's would be).
    pub const TRANSFER: u64 = 64 * 1024;
    /// Arguments plus environment strings of `spawn`.
    pub const SPAWN_STRINGS: u64 = 4096;
    /// Bytes of all of them together.
    pub const SPAWN_BYTES: u64 = 256 * 1024;
}

/// The ABI note `poly-rt` puts into every native program (see `poly-binfmt`):
/// the bytes of an `SHT_NOTE` section.
#[repr(C, align(4))]
#[derive(Debug)]
pub struct AbiNote {
    pub namesz: u32,
    pub descsz: u32,
    pub kind: u32,
    /// `"Poly\0"`, padded to 4 bytes.
    pub name: [u8; 8],
    pub abi: u32,
}

impl AbiNote {
    /// The note for [`ABI_VERSION`].
    pub const CURRENT: AbiNote = AbiNote {
        namesz: NOTE_OWNER.len() as u32,
        descsz: 4,
        kind: NT_POLY_ABI,
        name: *b"Poly\0\0\0\0",
        abi: ABI_VERSION,
    };
}

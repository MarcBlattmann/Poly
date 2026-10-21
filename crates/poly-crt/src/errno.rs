//! `errno` values (`errno.h`), their messages (`strerror`, `perror`,
//! `_sys_errlist`) and the Win32 error → `errno` mapping the runtime applies
//! after a failed system call (`_doserrno` keeps the Win32 code).
//!
//! The messages are the descriptions of Learn's "errno constants" table,
//! one per value up to `EILSEQ` (42), "Unknown error" otherwise; whether
//! the real runtime's texts differ in a word is for the spec agent to
//! confirm. The Win32 mapping is Poly's reading of each error's meaning
//! (Learn: "System Error Codes"), to be confirmed as well.

pub const EPERM: i32 = 1;
pub const ENOENT: i32 = 2;
pub const ESRCH: i32 = 3;
pub const EINTR: i32 = 4;
pub const EIO: i32 = 5;
pub const ENXIO: i32 = 6;
pub const E2BIG: i32 = 7;
pub const ENOEXEC: i32 = 8;
pub const EBADF: i32 = 9;
pub const ECHILD: i32 = 10;
pub const EAGAIN: i32 = 11;
pub const ENOMEM: i32 = 12;
pub const EACCES: i32 = 13;
pub const EFAULT: i32 = 14;
pub const EBUSY: i32 = 16;
pub const EEXIST: i32 = 17;
pub const EXDEV: i32 = 18;
pub const ENODEV: i32 = 19;
pub const ENOTDIR: i32 = 20;
pub const EISDIR: i32 = 21;
pub const EINVAL: i32 = 22;
pub const ENFILE: i32 = 23;
pub const EMFILE: i32 = 24;
pub const ENOTTY: i32 = 25;
pub const EFBIG: i32 = 27;
pub const ENOSPC: i32 = 28;
pub const ESPIPE: i32 = 29;
pub const EROFS: i32 = 30;
pub const EMLINK: i32 = 31;
pub const EPIPE: i32 = 32;
pub const EDOM: i32 = 33;
pub const ERANGE: i32 = 34;
pub const EDEADLK: i32 = 36;
pub const ENAMETOOLONG: i32 = 38;
pub const ENOLCK: i32 = 39;
pub const ENOSYS: i32 = 40;
pub const ENOTEMPTY: i32 = 41;
pub const EILSEQ: i32 = 42;
pub const STRUNCATE: i32 = 80;

/// `_sys_errlist`: index = `errno`; `_sys_nerr` is its length.
pub const MESSAGES: [&str; 43] = [
    "No error",
    "Operation not permitted",
    "No such file or directory",
    "No such process",
    "Interrupted function",
    "I/O error",
    "No such device or address",
    "Argument list too long",
    "Exec format error",
    "Bad file number",
    "No spawned processes",
    "No more processes or not enough memory or maximum nesting level reached",
    "Not enough memory",
    "Permission denied",
    "Bad address",
    "Unknown error",
    "Device or resource busy",
    "File exists",
    "Cross-device link",
    "No such device",
    "Not a directory",
    "Is a directory",
    "Invalid argument",
    "Too many files open in system",
    "Too many open files",
    "Inappropriate I/O control operation",
    "Unknown error",
    "File too large",
    "No space left on device",
    "Invalid seek",
    "Read-only file system",
    "Too many links",
    "Broken pipe",
    "Math argument",
    "Result too large",
    "Unknown error",
    "Resource deadlock would occur",
    "Unknown error",
    "Filename too long",
    "No locks available",
    "Function not supported",
    "Directory not empty",
    "Illegal byte sequence",
];

/// `strerror(e)`.
pub fn message(e: i32) -> &'static str {
    match e {
        0..=42 => MESSAGES[e as usize],
        STRUNCATE => "String was truncated",
        _ => "Unknown error",
    }
}

/// The `errno` of a failed Win32 call's error code (`winerror.h`).
pub fn from_win32(error: u32) -> i32 {
    match error {
        1 => EINVAL,                            // ERROR_INVALID_FUNCTION
        2 | 3 | 15 | 18 | 53 | 67 => ENOENT, // FILE/PATH_NOT_FOUND, INVALID_DRIVE, NO_MORE_FILES, BAD_NETPATH, BAD_NET_NAME
        4 => EMFILE,                         // ERROR_TOO_MANY_OPEN_FILES
        5 | 19 | 32 | 33 | 82 | 1307 => EACCES, // ACCESS_DENIED, WRITE_PROTECT, SHARING/LOCK_VIOLATION, CANNOT_MAKE
        6 => EBADF,                          // ERROR_INVALID_HANDLE
        8 | 14 => ENOMEM,                    // NOT_ENOUGH_MEMORY, OUTOFMEMORY
        11 => ENOEXEC,                       // ERROR_BAD_FORMAT
        13 | 87 => EINVAL,                   // INVALID_DATA, INVALID_PARAMETER
        17 => EXDEV,                         // ERROR_NOT_SAME_DEVICE
        80 | 183 => EEXIST,                  // FILE_EXISTS, ALREADY_EXISTS
        109 => EPIPE,                        // ERROR_BROKEN_PIPE
        112 => ENOSPC,                       // ERROR_DISK_FULL
        123 | 161 => ENOENT,                 // INVALID_NAME, BAD_PATHNAME
        145 => ENOTEMPTY,                    // ERROR_DIR_NOT_EMPTY
        206 => ENAMETOOLONG,                 // ERROR_FILENAME_EXCED_RANGE
        267 => ENOTDIR,                      // ERROR_DIRECTORY
        _ => EINVAL,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn messages_and_mapping() {
        assert_eq!(message(ENOENT), "No such file or directory");
        assert_eq!(message(EILSEQ), "Illegal byte sequence");
        assert_eq!(message(STRUNCATE), "String was truncated");
        assert_eq!(message(-1), "Unknown error");
        assert_eq!(message(1000), "Unknown error");
        assert_eq!(MESSAGES[EINVAL as usize], "Invalid argument");
        assert_eq!(from_win32(2), ENOENT);
        assert_eq!(from_win32(5), EACCES);
        assert_eq!(from_win32(80), EEXIST);
        assert_eq!(from_win32(6), EBADF);
        assert_eq!(from_win32(145), ENOTEMPTY);
        assert_eq!(from_win32(9999), EINVAL);
    }
}

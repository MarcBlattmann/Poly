//! Darwin errno values (macOS SDK `<sys/errno.h>`) and the mapping from
//! the Linux-numbered errors Poly's VFS and memory code return
//! (`<asm-generic/errno-base.h>`, `<asm-generic/errno.h>`). 1 to 34 are the
//! same numbers on both systems; the rest differ.

pub const EPERM: u32 = 1;
pub const ENOENT: u32 = 2;
pub const ESRCH: u32 = 3;
pub const EINTR: u32 = 4;
pub const EIO: u32 = 5;
pub const ENOEXEC: u32 = 8;
pub const EBADF: u32 = 9;
pub const ECHILD: u32 = 10;
pub const ENOMEM: u32 = 12;
pub const EACCES: u32 = 13;
pub const EFAULT: u32 = 14;
pub const EEXIST: u32 = 17;
pub const ENODEV: u32 = 19;
pub const ENOTDIR: u32 = 20;
pub const EISDIR: u32 = 21;
pub const EINVAL: u32 = 22;
pub const EMFILE: u32 = 24;
pub const ESPIPE: u32 = 29;
pub const EPIPE: u32 = 32;
pub const ERANGE: u32 = 34;
pub const EAGAIN: u32 = 35;
pub const ELOOP: u32 = 62;
pub const ENAMETOOLONG: u32 = 63;
pub const ENOTEMPTY: u32 = 66;
pub const ENOSYS: u32 = 78;
pub const ENOTSUP: u32 = 45;
pub const EOVERFLOW: u32 = 84;

/// The Darwin errno for a Linux errno.
pub const fn from_linux(e: u32) -> u32 {
    match e {
        // Linux EAGAIN; Darwin's 11 is EDEADLK.
        11 => EAGAIN,
        1..=34 => e,
        35 => 11, // EDEADLK
        36 => ENAMETOOLONG,
        37 => 77, // ENOLCK
        38 => ENOSYS,
        39 => ENOTEMPTY,
        40 => ELOOP,
        61 => 96, // ENODATA
        75 => EOVERFLOW,
        84 => 92, // EILSEQ
        88 => 38, // ENOTSOCK
        95 => ENOTSUP,
        98 => 48,  // EADDRINUSE
        104 => 54, // ECONNRESET
        110 => 60, // ETIMEDOUT
        111 => 61, // ECONNREFUSED
        _ => EINVAL,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn maps_the_numbers_that_differ() {
        assert_eq!(from_linux(2), ENOENT);
        assert_eq!(from_linux(34), ERANGE);
        assert_eq!(from_linux(11), EAGAIN);
        assert_eq!(from_linux(36), ENAMETOOLONG);
        assert_eq!(from_linux(40), ELOOP);
        assert_eq!(from_linux(39), ENOTEMPTY);
    }
}

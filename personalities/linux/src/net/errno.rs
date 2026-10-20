//! The socket error numbers (UAPI `asm-generic/errno.h`, which x86-64 uses
//! unchanged), kept next to the socket calls.

use crate::errno::Errno;

const fn errno(value: u16) -> Errno {
    match Errno::new(value) {
        Some(e) => e,
        None => panic!("not an error number"),
    }
}

impl Errno {
    pub const ENOTSOCK: Errno = errno(88);
    pub const EDESTADDRREQ: Errno = errno(89);
    pub const EMSGSIZE: Errno = errno(90);
    pub const EPROTOTYPE: Errno = errno(91);
    pub const ENOPROTOOPT: Errno = errno(92);
    pub const EPROTONOSUPPORT: Errno = errno(93);
    pub const ESOCKTNOSUPPORT: Errno = errno(94);
    pub const EAFNOSUPPORT: Errno = errno(97);
    pub const EADDRINUSE: Errno = errno(98);
    pub const EADDRNOTAVAIL: Errno = errno(99);
    pub const ENETDOWN: Errno = errno(100);
    pub const ENETUNREACH: Errno = errno(101);
    pub const ECONNABORTED: Errno = errno(103);
    pub const ECONNRESET: Errno = errno(104);
    pub const ENOBUFS: Errno = errno(105);
    pub const EISCONN: Errno = errno(106);
    pub const ENOTCONN: Errno = errno(107);
    pub const ECONNREFUSED: Errno = errno(111);
    pub const EHOSTUNREACH: Errno = errno(113);
    pub const EALREADY: Errno = errno(114);
    pub const EINPROGRESS: Errno = errno(115);
    pub const ETOOMANYREFS: Errno = errno(109);
}

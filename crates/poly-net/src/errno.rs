//! Socket errors as Linux `errno` values, so the Linux personality can
//! return them unchanged (as `-errno`) and the others can translate them.
//!
//! Values are from the Linux UAPI headers `include/uapi/asm-generic/errno-base.h`
//! (1-34) and `include/uapi/asm-generic/errno.h` (35 and up), which x86-64
//! uses unchanged.

use core::fmt;

/// A Linux `errno` value.
#[derive(Clone, Copy, PartialEq, Eq, Hash)]
pub struct Errno(pub u16);

impl Errno {
    /// `EBADF` (errno-base.h): not a valid socket handle.
    pub const EBADF: Errno = Errno(9);
    /// `EAGAIN` (errno-base.h): the operation would block.
    pub const EAGAIN: Errno = Errno(11);
    /// `ENOMEM` (errno-base.h).
    pub const ENOMEM: Errno = Errno(12);
    /// `EACCES` (errno-base.h): e.g. sending to a broadcast address without `SO_BROADCAST`.
    pub const EACCES: Errno = Errno(13);
    /// `EINVAL` (errno-base.h).
    pub const EINVAL: Errno = Errno(22);
    /// `EPIPE` (errno-base.h): writing after the connection was shut down.
    pub const EPIPE: Errno = Errno(32);
    /// `EDESTADDRREQ` (errno.h): no destination for an unconnected datagram socket.
    pub const EDESTADDRREQ: Errno = Errno(89);
    /// `EMSGSIZE` (errno.h): a datagram larger than the path allows.
    pub const EMSGSIZE: Errno = Errno(90);
    /// `ENOPROTOOPT` (errno.h).
    pub const ENOPROTOOPT: Errno = Errno(92);
    /// `EPROTONOSUPPORT` (errno.h).
    pub const EPROTONOSUPPORT: Errno = Errno(93);
    /// `ESOCKTNOSUPPORT` (errno.h).
    pub const ESOCKTNOSUPPORT: Errno = Errno(94);
    /// `EOPNOTSUPP` (errno.h): e.g. `listen` on a datagram socket.
    pub const EOPNOTSUPP: Errno = Errno(95);
    /// `EAFNOSUPPORT` (errno.h): an address of the wrong family.
    pub const EAFNOSUPPORT: Errno = Errno(97);
    /// `EADDRINUSE` (errno.h).
    pub const EADDRINUSE: Errno = Errno(98);
    /// `EADDRNOTAVAIL` (errno.h): not one of our addresses, or no free port.
    pub const EADDRNOTAVAIL: Errno = Errno(99);
    /// `ENETDOWN` (errno.h).
    pub const ENETDOWN: Errno = Errno(100);
    /// `ENETUNREACH` (errno.h): no route.
    pub const ENETUNREACH: Errno = Errno(101);
    /// `ECONNABORTED` (errno.h).
    pub const ECONNABORTED: Errno = Errno(103);
    /// `ECONNRESET` (errno.h): the peer reset the connection.
    pub const ECONNRESET: Errno = Errno(104);
    /// `ENOBUFS` (errno.h): out of socket slots or buffer space.
    pub const ENOBUFS: Errno = Errno(105);
    /// `EISCONN` (errno.h).
    pub const EISCONN: Errno = Errno(106);
    /// `ENOTCONN` (errno.h).
    pub const ENOTCONN: Errno = Errno(107);
    /// `ETIMEDOUT` (errno.h): the peer stopped answering.
    pub const ETIMEDOUT: Errno = Errno(110);
    /// `ECONNREFUSED` (errno.h): reset during connect, or ICMP port unreachable.
    pub const ECONNREFUSED: Errno = Errno(111);
    /// `EHOSTUNREACH` (errno.h): neighbour resolution failed or ICMP host unreachable.
    pub const EHOSTUNREACH: Errno = Errno(113);
    /// `EALREADY` (errno.h): a connect is already in progress.
    pub const EALREADY: Errno = Errno(114);
    /// `EINPROGRESS` (errno.h): a nonblocking connect has started.
    pub const EINPROGRESS: Errno = Errno(115);

    /// The symbolic name, for logs.
    pub fn name(self) -> &'static str {
        match self.0 {
            9 => "EBADF",
            11 => "EAGAIN",
            12 => "ENOMEM",
            13 => "EACCES",
            22 => "EINVAL",
            32 => "EPIPE",
            89 => "EDESTADDRREQ",
            90 => "EMSGSIZE",
            92 => "ENOPROTOOPT",
            93 => "EPROTONOSUPPORT",
            94 => "ESOCKTNOSUPPORT",
            95 => "EOPNOTSUPP",
            97 => "EAFNOSUPPORT",
            98 => "EADDRINUSE",
            99 => "EADDRNOTAVAIL",
            100 => "ENETDOWN",
            101 => "ENETUNREACH",
            103 => "ECONNABORTED",
            104 => "ECONNRESET",
            105 => "ENOBUFS",
            106 => "EISCONN",
            107 => "ENOTCONN",
            110 => "ETIMEDOUT",
            111 => "ECONNREFUSED",
            113 => "EHOSTUNREACH",
            114 => "EALREADY",
            115 => "EINPROGRESS",
            _ => "E?",
        }
    }
}

impl fmt::Debug for Errno {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}({})", self.name(), self.0)
    }
}

impl fmt::Display for Errno {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        fmt::Debug::fmt(self, f)
    }
}

//! Linux error numbers.
//!
//! Names and values are the Linux UAPI headers' `asm-generic/errno-base.h`
//! (1-34) and `asm-generic/errno.h` (35-133), which x86-64 uses unchanged.
//! The aliases those headers define (`EWOULDBLOCK` = `EAGAIN`, `EDEADLOCK` =
//! `EDEADLK`) are associated constants, and so is `ENOTSUP`, which C
//! libraries on Linux define as `EOPNOTSUPP`.
//!
//! The VFS reports every failure as an [`Errno`], so the Linux personality
//! returns [`Errno::to_syscall_return`] without translating; the other
//! personalities map these to their own codes.

/// A Linux error number. The discriminant is the value from the UAPI headers.
#[allow(non_camel_case_types)] // the names are the ABI's, kept exactly
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
#[repr(u16)]
pub enum Errno {
    EPERM = 1,
    ENOENT = 2,
    ESRCH = 3,
    EINTR = 4,
    EIO = 5,
    ENXIO = 6,
    E2BIG = 7,
    ENOEXEC = 8,
    EBADF = 9,
    ECHILD = 10,
    EAGAIN = 11,
    ENOMEM = 12,
    EACCES = 13,
    EFAULT = 14,
    ENOTBLK = 15,
    EBUSY = 16,
    EEXIST = 17,
    EXDEV = 18,
    ENODEV = 19,
    ENOTDIR = 20,
    EISDIR = 21,
    EINVAL = 22,
    ENFILE = 23,
    EMFILE = 24,
    ENOTTY = 25,
    ETXTBSY = 26,
    EFBIG = 27,
    ENOSPC = 28,
    ESPIPE = 29,
    EROFS = 30,
    EMLINK = 31,
    EPIPE = 32,
    EDOM = 33,
    ERANGE = 34,
    EDEADLK = 35,
    ENAMETOOLONG = 36,
    ENOLCK = 37,
    ENOSYS = 38,
    ENOTEMPTY = 39,
    ELOOP = 40,
    ENOMSG = 42,
    EIDRM = 43,
    ECHRNG = 44,
    EL2NSYNC = 45,
    EL3HLT = 46,
    EL3RST = 47,
    ELNRNG = 48,
    EUNATCH = 49,
    ENOCSI = 50,
    EL2HLT = 51,
    EBADE = 52,
    EBADR = 53,
    EXFULL = 54,
    ENOANO = 55,
    EBADRQC = 56,
    EBADSLT = 57,
    EBFONT = 59,
    ENOSTR = 60,
    ENODATA = 61,
    ETIME = 62,
    ENOSR = 63,
    ENONET = 64,
    ENOPKG = 65,
    EREMOTE = 66,
    ENOLINK = 67,
    EADV = 68,
    ESRMNT = 69,
    ECOMM = 70,
    EPROTO = 71,
    EMULTIHOP = 72,
    EDOTDOT = 73,
    EBADMSG = 74,
    EOVERFLOW = 75,
    ENOTUNIQ = 76,
    EBADFD = 77,
    EREMCHG = 78,
    ELIBACC = 79,
    ELIBBAD = 80,
    ELIBSCN = 81,
    ELIBMAX = 82,
    ELIBEXEC = 83,
    EILSEQ = 84,
    ERESTART = 85,
    ESTRPIPE = 86,
    EUSERS = 87,
    ENOTSOCK = 88,
    EDESTADDRREQ = 89,
    EMSGSIZE = 90,
    EPROTOTYPE = 91,
    ENOPROTOOPT = 92,
    EPROTONOSUPPORT = 93,
    ESOCKTNOSUPPORT = 94,
    EOPNOTSUPP = 95,
    EPFNOSUPPORT = 96,
    EAFNOSUPPORT = 97,
    EADDRINUSE = 98,
    EADDRNOTAVAIL = 99,
    ENETDOWN = 100,
    ENETUNREACH = 101,
    ENETRESET = 102,
    ECONNABORTED = 103,
    ECONNRESET = 104,
    ENOBUFS = 105,
    EISCONN = 106,
    ENOTCONN = 107,
    ESHUTDOWN = 108,
    ETOOMANYREFS = 109,
    ETIMEDOUT = 110,
    ECONNREFUSED = 111,
    EHOSTDOWN = 112,
    EHOSTUNREACH = 113,
    EALREADY = 114,
    EINPROGRESS = 115,
    ESTALE = 116,
    EUCLEAN = 117,
    ENOTNAM = 118,
    ENAVAIL = 119,
    EISNAM = 120,
    EREMOTEIO = 121,
    EDQUOT = 122,
    ENOMEDIUM = 123,
    EMEDIUMTYPE = 124,
    ECANCELED = 125,
    ENOKEY = 126,
    EKEYEXPIRED = 127,
    EKEYREVOKED = 128,
    EKEYREJECTED = 129,
    EOWNERDEAD = 130,
    ENOTRECOVERABLE = 131,
    ERFKILL = 132,
    EHWPOISON = 133,
}

impl Errno {
    /// `EAGAIN` under its other name.
    pub const EWOULDBLOCK: Errno = Errno::EAGAIN;
    /// `EDEADLK` under its other name.
    pub const EDEADLOCK: Errno = Errno::EDEADLK;
    /// What C libraries on Linux define `ENOTSUP` as.
    pub const ENOTSUP: Errno = Errno::EOPNOTSUPP;

    /// Every error number, in increasing order.
    pub const ALL: [Errno; 131] = [
        Errno::EPERM,
        Errno::ENOENT,
        Errno::ESRCH,
        Errno::EINTR,
        Errno::EIO,
        Errno::ENXIO,
        Errno::E2BIG,
        Errno::ENOEXEC,
        Errno::EBADF,
        Errno::ECHILD,
        Errno::EAGAIN,
        Errno::ENOMEM,
        Errno::EACCES,
        Errno::EFAULT,
        Errno::ENOTBLK,
        Errno::EBUSY,
        Errno::EEXIST,
        Errno::EXDEV,
        Errno::ENODEV,
        Errno::ENOTDIR,
        Errno::EISDIR,
        Errno::EINVAL,
        Errno::ENFILE,
        Errno::EMFILE,
        Errno::ENOTTY,
        Errno::ETXTBSY,
        Errno::EFBIG,
        Errno::ENOSPC,
        Errno::ESPIPE,
        Errno::EROFS,
        Errno::EMLINK,
        Errno::EPIPE,
        Errno::EDOM,
        Errno::ERANGE,
        Errno::EDEADLK,
        Errno::ENAMETOOLONG,
        Errno::ENOLCK,
        Errno::ENOSYS,
        Errno::ENOTEMPTY,
        Errno::ELOOP,
        Errno::ENOMSG,
        Errno::EIDRM,
        Errno::ECHRNG,
        Errno::EL2NSYNC,
        Errno::EL3HLT,
        Errno::EL3RST,
        Errno::ELNRNG,
        Errno::EUNATCH,
        Errno::ENOCSI,
        Errno::EL2HLT,
        Errno::EBADE,
        Errno::EBADR,
        Errno::EXFULL,
        Errno::ENOANO,
        Errno::EBADRQC,
        Errno::EBADSLT,
        Errno::EBFONT,
        Errno::ENOSTR,
        Errno::ENODATA,
        Errno::ETIME,
        Errno::ENOSR,
        Errno::ENONET,
        Errno::ENOPKG,
        Errno::EREMOTE,
        Errno::ENOLINK,
        Errno::EADV,
        Errno::ESRMNT,
        Errno::ECOMM,
        Errno::EPROTO,
        Errno::EMULTIHOP,
        Errno::EDOTDOT,
        Errno::EBADMSG,
        Errno::EOVERFLOW,
        Errno::ENOTUNIQ,
        Errno::EBADFD,
        Errno::EREMCHG,
        Errno::ELIBACC,
        Errno::ELIBBAD,
        Errno::ELIBSCN,
        Errno::ELIBMAX,
        Errno::ELIBEXEC,
        Errno::EILSEQ,
        Errno::ERESTART,
        Errno::ESTRPIPE,
        Errno::EUSERS,
        Errno::ENOTSOCK,
        Errno::EDESTADDRREQ,
        Errno::EMSGSIZE,
        Errno::EPROTOTYPE,
        Errno::ENOPROTOOPT,
        Errno::EPROTONOSUPPORT,
        Errno::ESOCKTNOSUPPORT,
        Errno::EOPNOTSUPP,
        Errno::EPFNOSUPPORT,
        Errno::EAFNOSUPPORT,
        Errno::EADDRINUSE,
        Errno::EADDRNOTAVAIL,
        Errno::ENETDOWN,
        Errno::ENETUNREACH,
        Errno::ENETRESET,
        Errno::ECONNABORTED,
        Errno::ECONNRESET,
        Errno::ENOBUFS,
        Errno::EISCONN,
        Errno::ENOTCONN,
        Errno::ESHUTDOWN,
        Errno::ETOOMANYREFS,
        Errno::ETIMEDOUT,
        Errno::ECONNREFUSED,
        Errno::EHOSTDOWN,
        Errno::EHOSTUNREACH,
        Errno::EALREADY,
        Errno::EINPROGRESS,
        Errno::ESTALE,
        Errno::EUCLEAN,
        Errno::ENOTNAM,
        Errno::ENAVAIL,
        Errno::EISNAM,
        Errno::EREMOTEIO,
        Errno::EDQUOT,
        Errno::ENOMEDIUM,
        Errno::EMEDIUMTYPE,
        Errno::ECANCELED,
        Errno::ENOKEY,
        Errno::EKEYEXPIRED,
        Errno::EKEYREVOKED,
        Errno::EKEYREJECTED,
        Errno::EOWNERDEAD,
        Errno::ENOTRECOVERABLE,
        Errno::ERFKILL,
        Errno::EHWPOISON,
    ];

    /// The number (positive), as `errno` holds it.
    pub const fn code(self) -> i32 {
        self as u16 as i32
    }

    /// The value a Linux system call returns in `rax` for this error: `-errno`.
    pub const fn to_syscall_return(self) -> u64 {
        (-(self.code() as i64)) as u64
    }

    /// The error with this number, if Linux defines one.
    pub fn from_code(code: i32) -> Option<Errno> {
        Errno::ALL.iter().copied().find(|e| e.code() == code)
    }

    /// The symbolic name, such as `"ENOENT"`.
    pub fn name(self) -> &'static str {
        match self {
            Errno::EPERM => "EPERM",
            Errno::ENOENT => "ENOENT",
            Errno::ESRCH => "ESRCH",
            Errno::EINTR => "EINTR",
            Errno::EIO => "EIO",
            Errno::ENXIO => "ENXIO",
            Errno::E2BIG => "E2BIG",
            Errno::ENOEXEC => "ENOEXEC",
            Errno::EBADF => "EBADF",
            Errno::ECHILD => "ECHILD",
            Errno::EAGAIN => "EAGAIN",
            Errno::ENOMEM => "ENOMEM",
            Errno::EACCES => "EACCES",
            Errno::EFAULT => "EFAULT",
            Errno::ENOTBLK => "ENOTBLK",
            Errno::EBUSY => "EBUSY",
            Errno::EEXIST => "EEXIST",
            Errno::EXDEV => "EXDEV",
            Errno::ENODEV => "ENODEV",
            Errno::ENOTDIR => "ENOTDIR",
            Errno::EISDIR => "EISDIR",
            Errno::EINVAL => "EINVAL",
            Errno::ENFILE => "ENFILE",
            Errno::EMFILE => "EMFILE",
            Errno::ENOTTY => "ENOTTY",
            Errno::ETXTBSY => "ETXTBSY",
            Errno::EFBIG => "EFBIG",
            Errno::ENOSPC => "ENOSPC",
            Errno::ESPIPE => "ESPIPE",
            Errno::EROFS => "EROFS",
            Errno::EMLINK => "EMLINK",
            Errno::EPIPE => "EPIPE",
            Errno::EDOM => "EDOM",
            Errno::ERANGE => "ERANGE",
            Errno::EDEADLK => "EDEADLK",
            Errno::ENAMETOOLONG => "ENAMETOOLONG",
            Errno::ENOLCK => "ENOLCK",
            Errno::ENOSYS => "ENOSYS",
            Errno::ENOTEMPTY => "ENOTEMPTY",
            Errno::ELOOP => "ELOOP",
            Errno::ENOMSG => "ENOMSG",
            Errno::EIDRM => "EIDRM",
            Errno::ECHRNG => "ECHRNG",
            Errno::EL2NSYNC => "EL2NSYNC",
            Errno::EL3HLT => "EL3HLT",
            Errno::EL3RST => "EL3RST",
            Errno::ELNRNG => "ELNRNG",
            Errno::EUNATCH => "EUNATCH",
            Errno::ENOCSI => "ENOCSI",
            Errno::EL2HLT => "EL2HLT",
            Errno::EBADE => "EBADE",
            Errno::EBADR => "EBADR",
            Errno::EXFULL => "EXFULL",
            Errno::ENOANO => "ENOANO",
            Errno::EBADRQC => "EBADRQC",
            Errno::EBADSLT => "EBADSLT",
            Errno::EBFONT => "EBFONT",
            Errno::ENOSTR => "ENOSTR",
            Errno::ENODATA => "ENODATA",
            Errno::ETIME => "ETIME",
            Errno::ENOSR => "ENOSR",
            Errno::ENONET => "ENONET",
            Errno::ENOPKG => "ENOPKG",
            Errno::EREMOTE => "EREMOTE",
            Errno::ENOLINK => "ENOLINK",
            Errno::EADV => "EADV",
            Errno::ESRMNT => "ESRMNT",
            Errno::ECOMM => "ECOMM",
            Errno::EPROTO => "EPROTO",
            Errno::EMULTIHOP => "EMULTIHOP",
            Errno::EDOTDOT => "EDOTDOT",
            Errno::EBADMSG => "EBADMSG",
            Errno::EOVERFLOW => "EOVERFLOW",
            Errno::ENOTUNIQ => "ENOTUNIQ",
            Errno::EBADFD => "EBADFD",
            Errno::EREMCHG => "EREMCHG",
            Errno::ELIBACC => "ELIBACC",
            Errno::ELIBBAD => "ELIBBAD",
            Errno::ELIBSCN => "ELIBSCN",
            Errno::ELIBMAX => "ELIBMAX",
            Errno::ELIBEXEC => "ELIBEXEC",
            Errno::EILSEQ => "EILSEQ",
            Errno::ERESTART => "ERESTART",
            Errno::ESTRPIPE => "ESTRPIPE",
            Errno::EUSERS => "EUSERS",
            Errno::ENOTSOCK => "ENOTSOCK",
            Errno::EDESTADDRREQ => "EDESTADDRREQ",
            Errno::EMSGSIZE => "EMSGSIZE",
            Errno::EPROTOTYPE => "EPROTOTYPE",
            Errno::ENOPROTOOPT => "ENOPROTOOPT",
            Errno::EPROTONOSUPPORT => "EPROTONOSUPPORT",
            Errno::ESOCKTNOSUPPORT => "ESOCKTNOSUPPORT",
            Errno::EOPNOTSUPP => "EOPNOTSUPP",
            Errno::EPFNOSUPPORT => "EPFNOSUPPORT",
            Errno::EAFNOSUPPORT => "EAFNOSUPPORT",
            Errno::EADDRINUSE => "EADDRINUSE",
            Errno::EADDRNOTAVAIL => "EADDRNOTAVAIL",
            Errno::ENETDOWN => "ENETDOWN",
            Errno::ENETUNREACH => "ENETUNREACH",
            Errno::ENETRESET => "ENETRESET",
            Errno::ECONNABORTED => "ECONNABORTED",
            Errno::ECONNRESET => "ECONNRESET",
            Errno::ENOBUFS => "ENOBUFS",
            Errno::EISCONN => "EISCONN",
            Errno::ENOTCONN => "ENOTCONN",
            Errno::ESHUTDOWN => "ESHUTDOWN",
            Errno::ETOOMANYREFS => "ETOOMANYREFS",
            Errno::ETIMEDOUT => "ETIMEDOUT",
            Errno::ECONNREFUSED => "ECONNREFUSED",
            Errno::EHOSTDOWN => "EHOSTDOWN",
            Errno::EHOSTUNREACH => "EHOSTUNREACH",
            Errno::EALREADY => "EALREADY",
            Errno::EINPROGRESS => "EINPROGRESS",
            Errno::ESTALE => "ESTALE",
            Errno::EUCLEAN => "EUCLEAN",
            Errno::ENOTNAM => "ENOTNAM",
            Errno::ENAVAIL => "ENAVAIL",
            Errno::EISNAM => "EISNAM",
            Errno::EREMOTEIO => "EREMOTEIO",
            Errno::EDQUOT => "EDQUOT",
            Errno::ENOMEDIUM => "ENOMEDIUM",
            Errno::EMEDIUMTYPE => "EMEDIUMTYPE",
            Errno::ECANCELED => "ECANCELED",
            Errno::ENOKEY => "ENOKEY",
            Errno::EKEYEXPIRED => "EKEYEXPIRED",
            Errno::EKEYREVOKED => "EKEYREVOKED",
            Errno::EKEYREJECTED => "EKEYREJECTED",
            Errno::EOWNERDEAD => "EOWNERDEAD",
            Errno::ENOTRECOVERABLE => "ENOTRECOVERABLE",
            Errno::ERFKILL => "ERFKILL",
            Errno::EHWPOISON => "EHWPOISON",
        }
    }
}

impl core::fmt::Display for Errno {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        write!(f, "{} ({})", self.name(), self.code())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn values_match_the_uapi_headers() {
        // Spot checks against asm-generic/errno-base.h and errno.h.
        assert_eq!(Errno::EPERM.code(), 1);
        assert_eq!(Errno::ENOENT.code(), 2);
        assert_eq!(Errno::EBADF.code(), 9);
        assert_eq!(Errno::EACCES.code(), 13);
        assert_eq!(Errno::EEXIST.code(), 17);
        assert_eq!(Errno::ENOTDIR.code(), 20);
        assert_eq!(Errno::EISDIR.code(), 21);
        assert_eq!(Errno::EINVAL.code(), 22);
        assert_eq!(Errno::EMFILE.code(), 24);
        assert_eq!(Errno::ERANGE.code(), 34);
        assert_eq!(Errno::ENAMETOOLONG.code(), 36);
        assert_eq!(Errno::ENOSYS.code(), 38);
        assert_eq!(Errno::ENOTEMPTY.code(), 39);
        assert_eq!(Errno::ELOOP.code(), 40);
        assert_eq!(Errno::EOPNOTSUPP.code(), 95);
        assert_eq!(Errno::EHWPOISON.code(), 133);
        assert_eq!(Errno::EWOULDBLOCK, Errno::EAGAIN);
    }

    #[test]
    fn round_trips_and_names() {
        for (i, e) in Errno::ALL.iter().enumerate() {
            assert_eq!(Errno::from_code(e.code()), Some(*e));
            assert!(e.name().starts_with('E'));
            if i > 0 {
                assert!(Errno::ALL[i - 1] < *e);
            }
        }
        // 41 and 58 are unassigned in the headers.
        assert_eq!(Errno::from_code(41), None);
        assert_eq!(Errno::from_code(58), None);
        assert_eq!(Errno::from_code(0), None);
        assert_eq!(Errno::ENOENT.to_syscall_return() as i64, -2);
    }
}

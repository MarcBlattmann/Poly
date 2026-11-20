//! Darwin errno values (macOS SDK `sys/errno.h`).

pub const ENOENT: u32 = 2;
pub const EINTR: u32 = 4;
pub const ENOMEM: u32 = 12;
pub const EACCES: u32 = 13;
pub const EFAULT: u32 = 14;
pub const EINVAL: u32 = 22;
pub const ENOTSUP: u32 = 45;
pub const ETIMEDOUT: u32 = 60;
pub const ENOSYS: u32 = 78;
pub const EOWNERDEAD: u32 = 105;

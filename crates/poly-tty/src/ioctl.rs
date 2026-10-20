//! Terminal ioctl requests (`asm-generic/ioctls.h`, which x86-64 uses),
//! as ioctl_tty(2) describes them.
//!
//! The requests with a size in their number are `_IOR` / `_IOW`
//! encodings (`asm-generic/ioctl.h`: direction in bits 30-31, size in
//! bits 16-29, type `'T'` in bits 8-15, number in bits 0-7).

pub const TCGETS: u32 = 0x5401;
pub const TCSETS: u32 = 0x5402;
pub const TCSETSW: u32 = 0x5403;
pub const TCSETSF: u32 = 0x5404;
pub const TCGETA: u32 = 0x5405;
pub const TCSETA: u32 = 0x5406;
pub const TCSETAW: u32 = 0x5407;
pub const TCSETAF: u32 = 0x5408;
pub const TCSBRK: u32 = 0x5409;
pub const TCXONC: u32 = 0x540A;
pub const TCFLSH: u32 = 0x540B;
pub const TIOCEXCL: u32 = 0x540C;
pub const TIOCNXCL: u32 = 0x540D;
pub const TIOCSCTTY: u32 = 0x540E;
pub const TIOCGPGRP: u32 = 0x540F;
pub const TIOCSPGRP: u32 = 0x5410;
pub const TIOCOUTQ: u32 = 0x5411;
pub const TIOCSTI: u32 = 0x5412;
pub const TIOCGWINSZ: u32 = 0x5413;
pub const TIOCSWINSZ: u32 = 0x5414;
pub const FIONREAD: u32 = 0x541B;
/// `TIOCINQ` is `FIONREAD`.
pub const TIOCINQ: u32 = FIONREAD;
pub const TIOCNOTTY: u32 = 0x5422;
pub const TIOCSETD: u32 = 0x5423;
pub const TIOCGETD: u32 = 0x5424;
pub const TCSBRKP: u32 = 0x5425;
pub const TIOCGSID: u32 = 0x5429;
/// `_IOR('T', 0x2A, struct termios2)`.
pub const TCGETS2: u32 = 0x802C_542A;
/// `_IOW('T', 0x2B, struct termios2)`.
pub const TCSETS2: u32 = 0x402C_542B;
pub const TCSETSW2: u32 = 0x402C_542C;
pub const TCSETSF2: u32 = 0x402C_542D;
/// `_IOR('T', 0x30, unsigned int)`: the pty number of a master.
pub const TIOCGPTN: u32 = 0x8004_5430;
/// `_IOW('T', 0x31, int)`: lock or unlock the slave of a master.
pub const TIOCSPTLCK: u32 = 0x4004_5431;
/// `_IOW('T', 0x36, int)`: send a signal to the slave's foreground group.
pub const TIOCSIG: u32 = 0x4004_5436;
/// `_IOR('T', 0x39, int)`: whether the slave is locked.
pub const TIOCGPTLCK: u32 = 0x8004_5439;
/// `_IOR('T', 0x40, int)`: whether the terminal is in exclusive mode.
pub const TIOCGEXCL: u32 = 0x8004_5440;

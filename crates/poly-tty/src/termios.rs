//! The terminal attributes: `struct termios` and its flags, the special
//! characters, `struct winsize`, and their byte layouts.
//!
//! Values and layouts are those of the Linux UAPI headers, which x86-64
//! uses unchanged: `asm-generic/termbits-common.h` and
//! `asm-generic/termbits.h` (flags, `c_cc` indices, `struct termios`,
//! `struct termios2`, baud rates) and `asm-generic/termios.h`
//! (`struct winsize`, `struct termio`). The meaning of each flag is
//! termios(3)'s.

/// `NCCS`: the size of `c_cc` (`asm-generic/termbits.h`).
pub const NCCS: usize = 19;
/// `NCC`: the size of `c_cc` in the old `struct termio`
/// (`asm-generic/termios.h`).
pub const NCC: usize = 8;

/// Indices into `c_cc` (`asm-generic/termbits.h`).
pub mod cc {
    pub const VINTR: usize = 0;
    pub const VQUIT: usize = 1;
    pub const VERASE: usize = 2;
    pub const VKILL: usize = 3;
    pub const VEOF: usize = 4;
    pub const VTIME: usize = 5;
    pub const VMIN: usize = 6;
    pub const VSWTC: usize = 7;
    pub const VSTART: usize = 8;
    pub const VSTOP: usize = 9;
    pub const VSUSP: usize = 10;
    pub const VEOL: usize = 11;
    pub const VREPRINT: usize = 12;
    pub const VDISCARD: usize = 13;
    pub const VWERASE: usize = 14;
    pub const VLNEXT: usize = 15;
    pub const VEOL2: usize = 16;
}

/// `c_iflag` bits (`asm-generic/termbits-common.h`, `asm-generic/termbits.h`).
pub mod iflag {
    pub const IGNBRK: u32 = 0x001;
    pub const BRKINT: u32 = 0x002;
    pub const IGNPAR: u32 = 0x004;
    pub const PARMRK: u32 = 0x008;
    pub const INPCK: u32 = 0x010;
    pub const ISTRIP: u32 = 0x020;
    pub const INLCR: u32 = 0x040;
    pub const IGNCR: u32 = 0x080;
    pub const ICRNL: u32 = 0x100;
    pub const IUCLC: u32 = 0x0200;
    pub const IXON: u32 = 0x0400;
    pub const IXANY: u32 = 0x800;
    pub const IXOFF: u32 = 0x1000;
    pub const IMAXBEL: u32 = 0x2000;
    pub const IUTF8: u32 = 0x4000;
}

/// `c_oflag` bits (`asm-generic/termbits-common.h`, `asm-generic/termbits.h`).
pub mod oflag {
    pub const OPOST: u32 = 0x01;
    pub const OLCUC: u32 = 0x00002;
    pub const ONLCR: u32 = 0x00004;
    pub const OCRNL: u32 = 0x08;
    pub const ONOCR: u32 = 0x10;
    pub const ONLRET: u32 = 0x20;
    pub const OFILL: u32 = 0x40;
    pub const OFDEL: u32 = 0x80;
    pub const NLDLY: u32 = 0x00100;
    pub const CRDLY: u32 = 0x00600;
    pub const TABDLY: u32 = 0x01800;
    /// `TAB3` (= `XTABS`): tabs are expanded to spaces.
    pub const TAB3: u32 = 0x01800;
    pub const BSDLY: u32 = 0x02000;
    pub const VTDLY: u32 = 0x04000;
    pub const FFDLY: u32 = 0x08000;
}

/// `c_cflag` bits and baud rates (`asm-generic/termbits-common.h`,
/// `asm-generic/termbits.h`).
pub mod cflag {
    pub const CBAUD: u32 = 0x0000_100f;
    pub const CSIZE: u32 = 0x0000_0030;
    pub const CS5: u32 = 0x0000_0000;
    pub const CS6: u32 = 0x0000_0010;
    pub const CS7: u32 = 0x0000_0020;
    pub const CS8: u32 = 0x0000_0030;
    pub const CSTOPB: u32 = 0x0000_0040;
    pub const CREAD: u32 = 0x0000_0080;
    pub const PARENB: u32 = 0x0000_0100;
    pub const PARODD: u32 = 0x0000_0200;
    pub const HUPCL: u32 = 0x0000_0400;
    pub const CLOCAL: u32 = 0x0000_0800;
    pub const CBAUDEX: u32 = 0x0000_1000;
    /// The speed is in `c_ispeed` / `c_ospeed` (`struct termios2`).
    pub const BOTHER: u32 = 0x0000_1000;
    /// The input speed, `IBSHIFT` bits above the output speed.
    pub const CIBAUD: u32 = 0x100f_0000;
    pub const IBSHIFT: u32 = 16;
    pub const CMSPAR: u32 = 0x4000_0000;
    pub const CRTSCTS: u32 = 0x8000_0000;

    pub const B0: u32 = 0x0000_0000;
    pub const B9600: u32 = 0x0000_000d;
    pub const B38400: u32 = 0x0000_000f;
    pub const B115200: u32 = 0x0000_1002;

    /// The baud rate codes and their speeds in bits per second.
    pub const SPEEDS: [(u32, u32); 31] = [
        (0x0000, 0),
        (0x0001, 50),
        (0x0002, 75),
        (0x0003, 110),
        (0x0004, 134),
        (0x0005, 150),
        (0x0006, 200),
        (0x0007, 300),
        (0x0008, 600),
        (0x0009, 1200),
        (0x000a, 1800),
        (0x000b, 2400),
        (0x000c, 4800),
        (0x000d, 9600),
        (0x000e, 19200),
        (0x000f, 38400),
        (0x1001, 57600),
        (0x1002, 115_200),
        (0x1003, 230_400),
        (0x1004, 460_800),
        (0x1005, 500_000),
        (0x1006, 576_000),
        (0x1007, 921_600),
        (0x1008, 1_000_000),
        (0x1009, 1_152_000),
        (0x100a, 1_500_000),
        (0x100b, 2_000_000),
        (0x100c, 2_500_000),
        (0x100d, 3_000_000),
        (0x100e, 3_500_000),
        (0x100f, 4_000_000),
    ];

    /// The speed of baud code `code` (`CBAUD` bits); `None` for `BOTHER`
    /// and codes no rate has.
    pub fn speed(code: u32) -> Option<u32> {
        SPEEDS.iter().find(|&&(c, _)| c == code).map(|&(_, s)| s)
    }

    /// The baud code of `speed` bits per second, if it has one.
    pub fn code(speed: u32) -> Option<u32> {
        SPEEDS.iter().find(|&&(_, s)| s == speed).map(|&(c, _)| c)
    }
}

/// `c_lflag` bits (`asm-generic/termbits.h`).
pub mod lflag {
    pub const ISIG: u32 = 0x00001;
    pub const ICANON: u32 = 0x00002;
    pub const XCASE: u32 = 0x00004;
    pub const ECHO: u32 = 0x00008;
    pub const ECHOE: u32 = 0x00010;
    pub const ECHOK: u32 = 0x00020;
    pub const ECHONL: u32 = 0x00040;
    pub const NOFLSH: u32 = 0x00080;
    pub const TOSTOP: u32 = 0x00100;
    pub const ECHOCTL: u32 = 0x00200;
    pub const ECHOPRT: u32 = 0x00400;
    pub const ECHOKE: u32 = 0x00800;
    pub const FLUSHO: u32 = 0x01000;
    pub const PENDIN: u32 = 0x04000;
    pub const IEXTEN: u32 = 0x08000;
    pub const EXTPROC: u32 = 0x10000;
}

/// `tcflush` / `TCFLSH` queue selectors (`asm-generic/termbits.h`).
pub const TCIFLUSH: u32 = 0;
pub const TCOFLUSH: u32 = 1;
pub const TCIOFLUSH: u32 = 2;
/// `tcflow` / `TCXONC` actions (`asm-generic/termbits.h`).
pub const TCOOFF: u32 = 0;
pub const TCOON: u32 = 1;
pub const TCIOFF: u32 = 2;
pub const TCION: u32 = 3;

/// The line discipline number of the normal terminal discipline (`N_TTY`,
/// `linux/tty.h`), the only one Poly has.
pub const N_TTY: u32 = 0;

/// A special character with this value is disabled (`_POSIX_VDISABLE`,
/// 0 on Linux systems; Poly's choice, to be confirmed by the diff runner).
pub const VDISABLE: u8 = 0;

/// Control character `c` (`b'C'` for Ctrl-C).
pub const fn ctrl(c: u8) -> u8 {
    c & 0x1f
}

/// The attributes of a terminal: the kernel's `struct termios2` (also
/// `struct ktermios`), which holds everything `struct termios` does plus
/// the two speeds.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Termios {
    pub iflag: u32,
    pub oflag: u32,
    pub cflag: u32,
    pub lflag: u32,
    pub line: u8,
    pub cc: [u8; NCCS],
    pub ispeed: u32,
    pub ospeed: u32,
}

impl Termios {
    /// The size of `struct termios` (`TCGETS`): four flag words, the line
    /// discipline, `NCCS` characters.
    pub const SIZE: usize = 36;
    /// The size of `struct termios2` (`TCGETS2`): the speeds after it.
    pub const SIZE2: usize = 44;
    /// The size of `struct termio` (`TCGETA`): four 16-bit flag words, the
    /// line discipline, `NCC` characters, padded to 2 bytes.
    pub const SIZE_TERMIO: usize = 18;

    /// The attributes a terminal starts with: what the GNU coreutils
    /// manual lists for `stty sane` (`cread brkint icrnl imaxbel opost
    /// onlcr isig icanon iexten echo echoe echok echoctl echoke`, every
    /// other flag off) plus `ixon` and 8 data bits, and the traditional
    /// special characters (termios(3): `^C` INTR, `^\` QUIT, DEL ERASE,
    /// `^U` KILL, `^D` EOF, `^Q` START, `^S` STOP, `^Z` SUSP, `^R` REPRINT,
    /// `^O` DISCARD, `^W` WERASE, `^V` LNEXT; MIN 1, TIME 0). `speed` is
    /// the baud rate. (To be confirmed by the diff runner.)
    pub fn sane(speed: u32) -> Termios {
        let code = cflag::code(speed).unwrap_or(cflag::B38400);
        let mut cc = [VDISABLE; NCCS];
        cc[cc::VINTR] = ctrl(b'C');
        cc[cc::VQUIT] = ctrl(b'\\');
        cc[cc::VERASE] = 0x7f;
        cc[cc::VKILL] = ctrl(b'U');
        cc[cc::VEOF] = ctrl(b'D');
        cc[cc::VTIME] = 0;
        cc[cc::VMIN] = 1;
        cc[cc::VSTART] = ctrl(b'Q');
        cc[cc::VSTOP] = ctrl(b'S');
        cc[cc::VSUSP] = ctrl(b'Z');
        cc[cc::VREPRINT] = ctrl(b'R');
        cc[cc::VDISCARD] = ctrl(b'O');
        cc[cc::VWERASE] = ctrl(b'W');
        cc[cc::VLNEXT] = ctrl(b'V');
        let speed = cflag::speed(code).unwrap_or(38400);
        Termios {
            iflag: iflag::BRKINT | iflag::ICRNL | iflag::IXON | iflag::IMAXBEL,
            oflag: oflag::OPOST | oflag::ONLCR,
            cflag: code | (code << cflag::IBSHIFT) | cflag::CS8 | cflag::CREAD | cflag::HUPCL,
            lflag: lflag::ISIG
                | lflag::ICANON
                | lflag::IEXTEN
                | lflag::ECHO
                | lflag::ECHOE
                | lflag::ECHOK
                | lflag::ECHOCTL
                | lflag::ECHOKE,
            line: N_TTY as u8,
            cc,
            ispeed: speed,
            ospeed: speed,
        }
    }

    pub fn has_i(&self, bit: u32) -> bool {
        self.iflag & bit != 0
    }

    pub fn has_o(&self, bit: u32) -> bool {
        self.oflag & bit != 0
    }

    pub fn has_l(&self, bit: u32) -> bool {
        self.lflag & bit != 0
    }

    pub fn has_c(&self, bit: u32) -> bool {
        self.cflag & bit != 0
    }

    /// Canonical mode (`ICANON`).
    pub fn canonical(&self) -> bool {
        self.has_l(lflag::ICANON)
    }

    /// Special character `index` if it is enabled.
    pub fn special(&self, index: usize) -> Option<u8> {
        match self.cc[index] {
            VDISABLE => None,
            c => Some(c),
        }
    }

    /// Whether byte `c` is special character `index` (and that one is
    /// enabled).
    pub fn is(&self, index: usize, c: u8) -> bool {
        self.special(index) == Some(c)
    }

    /// `MIN` and `TIME` (tenths of a second) for non-canonical reads.
    pub fn min_time(&self) -> (u8, u8) {
        (self.cc[cc::VMIN], self.cc[cc::VTIME])
    }

    /// `struct termios`, as `TCGETS` returns it.
    pub fn to_bytes(&self) -> [u8; Self::SIZE] {
        let mut b = [0u8; Self::SIZE];
        b[0..4].copy_from_slice(&self.iflag.to_le_bytes());
        b[4..8].copy_from_slice(&self.oflag.to_le_bytes());
        b[8..12].copy_from_slice(&self.cflag.to_le_bytes());
        b[12..16].copy_from_slice(&self.lflag.to_le_bytes());
        b[16] = self.line;
        b[17..17 + NCCS].copy_from_slice(&self.cc);
        b
    }

    /// `struct termios2`, as `TCGETS2` returns it.
    pub fn to_bytes2(&self) -> [u8; Self::SIZE2] {
        let mut b = [0u8; Self::SIZE2];
        b[..Self::SIZE].copy_from_slice(&self.to_bytes());
        b[36..40].copy_from_slice(&self.ispeed.to_le_bytes());
        b[40..44].copy_from_slice(&self.ospeed.to_le_bytes());
        b
    }

    /// `struct termio`, as `TCGETA` returns it: the low 16 bits of each
    /// flag word and the first `NCC` characters.
    pub fn to_termio(&self) -> [u8; Self::SIZE_TERMIO] {
        let mut b = [0u8; Self::SIZE_TERMIO];
        b[0..2].copy_from_slice(&(self.iflag as u16).to_le_bytes());
        b[2..4].copy_from_slice(&(self.oflag as u16).to_le_bytes());
        b[4..6].copy_from_slice(&(self.cflag as u16).to_le_bytes());
        b[6..8].copy_from_slice(&(self.lflag as u16).to_le_bytes());
        b[8] = self.line;
        b[9..9 + NCC].copy_from_slice(&self.cc[..NCC]);
        b
    }

    /// The attributes `TCSETS` sets from `struct termios` `b`. The speeds
    /// follow the baud codes in `c_cflag` (an input code of 0 means "the
    /// same as the output", termios(3) `cfsetispeed`).
    pub fn from_bytes(&self, b: &[u8; Self::SIZE]) -> Termios {
        let word = |i: usize| u32::from_le_bytes(b[i..i + 4].try_into().expect("four bytes"));
        let mut cc = [0u8; NCCS];
        cc.copy_from_slice(&b[17..17 + NCCS]);
        let mut t = Termios {
            iflag: word(0),
            oflag: word(4),
            cflag: word(8),
            lflag: word(12),
            line: b[16],
            cc,
            ispeed: self.ispeed,
            ospeed: self.ospeed,
        };
        t.speeds_from_cflag();
        t
    }

    /// The attributes `TCSETS2` sets from `struct termios2` `b`: with
    /// `BOTHER` the speeds are taken as given.
    pub fn from_bytes2(&self, b: &[u8; Self::SIZE2]) -> Termios {
        let mut t = self.from_bytes(b[..Self::SIZE].try_into().expect("prefix"));
        let word = |i: usize| u32::from_le_bytes(b[i..i + 4].try_into().expect("four bytes"));
        if t.cflag & cflag::CBAUD == cflag::BOTHER {
            t.ospeed = word(40);
        }
        if (t.cflag & cflag::CIBAUD) >> cflag::IBSHIFT == cflag::BOTHER {
            t.ispeed = word(36);
        }
        t
    }

    /// The attributes `TCSETA` sets from `struct termio` `b`: the low 16
    /// bits of each flag word and the first `NCC` characters; the rest is
    /// kept.
    pub fn from_termio(&self, b: &[u8; Self::SIZE_TERMIO]) -> Termios {
        let half = |i: usize| u32::from(u16::from_le_bytes([b[i], b[i + 1]]));
        let mut t = *self;
        t.iflag = (self.iflag & 0xffff_0000) | half(0);
        t.oflag = (self.oflag & 0xffff_0000) | half(2);
        t.cflag = (self.cflag & 0xffff_0000) | half(4);
        t.lflag = (self.lflag & 0xffff_0000) | half(6);
        t.line = b[8];
        t.cc[..NCC].copy_from_slice(&b[9..9 + NCC]);
        t.speeds_from_cflag();
        t
    }

    /// Set the speeds from the baud codes in `c_cflag`.
    fn speeds_from_cflag(&mut self) {
        let out = self.cflag & cflag::CBAUD;
        if let Some(s) = cflag::speed(out) {
            self.ospeed = s;
        }
        let input = (self.cflag & cflag::CIBAUD) >> cflag::IBSHIFT;
        match input {
            0 => self.ispeed = self.ospeed,
            code => {
                if let Some(s) = cflag::speed(code) {
                    self.ispeed = s;
                }
            }
        }
    }
}

/// `struct winsize` (`asm-generic/termios.h`): rows and columns, and the
/// size in pixels (unused by the terminal itself).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Winsize {
    pub rows: u16,
    pub cols: u16,
    pub xpixel: u16,
    pub ypixel: u16,
}

impl Winsize {
    pub const SIZE: usize = 8;

    pub fn new(rows: u16, cols: u16) -> Winsize {
        Winsize { rows, cols, xpixel: 0, ypixel: 0 }
    }

    pub fn to_bytes(&self) -> [u8; Self::SIZE] {
        let mut b = [0u8; Self::SIZE];
        for (i, v) in [self.rows, self.cols, self.xpixel, self.ypixel].into_iter().enumerate() {
            b[2 * i..2 * i + 2].copy_from_slice(&v.to_le_bytes());
        }
        b
    }

    pub fn from_bytes(b: &[u8; Self::SIZE]) -> Winsize {
        let h = |i: usize| u16::from_le_bytes([b[2 * i], b[2 * i + 1]]);
        Winsize { rows: h(0), cols: h(1), xpixel: h(2), ypixel: h(3) }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn termios_round_trips_through_its_layouts() {
        let t = Termios::sane(115_200);
        assert_eq!(t.cflag & cflag::CBAUD, cflag::B115200);
        assert_eq!(t.ispeed, 115_200);
        let b = t.to_bytes();
        assert_eq!(&b[0..4], &t.iflag.to_le_bytes());
        assert_eq!(b[17 + cc::VINTR], 3);
        assert_eq!(b[17 + cc::VMIN], 1);
        assert_eq!(Termios::sane(9600).from_bytes(&b), t);
        let b2 = t.to_bytes2();
        assert_eq!(&b2[36..40], &115_200u32.to_le_bytes());
        assert_eq!(t.from_bytes2(&b2), t);
        let termio = t.to_termio();
        assert_eq!(t.from_termio(&termio), t);
    }

    #[test]
    fn speeds_follow_the_baud_codes() {
        let t = Termios::sane(38400);
        let mut b = t.to_bytes();
        let cflag = (t.cflag & !cflag::CBAUD & !cflag::CIBAUD) | cflag::B9600;
        b[8..12].copy_from_slice(&cflag.to_le_bytes());
        let n = t.from_bytes(&b);
        assert_eq!((n.ispeed, n.ospeed), (9600, 9600), "input code 0: the output speed");
        let mut b2 = t.to_bytes2();
        let other = (t.cflag & !cflag::CBAUD & !cflag::CIBAUD) | cflag::BOTHER | (cflag::BOTHER << cflag::IBSHIFT);
        b2[8..12].copy_from_slice(&other.to_le_bytes());
        b2[36..40].copy_from_slice(&1234u32.to_le_bytes());
        b2[40..44].copy_from_slice(&5678u32.to_le_bytes());
        let n = t.from_bytes2(&b2);
        assert_eq!((n.ispeed, n.ospeed), (1234, 5678));
    }

    #[test]
    fn winsize_layout() {
        let w = Winsize { rows: 24, cols: 80, xpixel: 640, ypixel: 480 };
        assert_eq!(w.to_bytes(), [24, 0, 80, 0, 0x80, 2, 0xe0, 1]);
        assert_eq!(Winsize::from_bytes(&w.to_bytes()), w);
    }

    #[test]
    fn disabled_characters_are_not_special() {
        let t = Termios::sane(38400);
        assert!(t.is(cc::VINTR, 3));
        assert_eq!(t.special(cc::VEOL), None);
        assert!(!t.is(cc::VEOL, 0));
    }
}

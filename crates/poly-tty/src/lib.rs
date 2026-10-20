//! Terminals without the machine (ADR 0046): the termios ABI, the line
//! discipline, terminals with sessions and job control, pseudo-terminals
//! and devpts. The kernel provides a [`Host`] (who is calling, signals,
//! blocking, user memory, the clock) and [`Driver`]s (the serial port,
//! the screen); everything else is here and tested on the host.
//!
//! ```text
//!   keys / serial bytes ──► Tty::receive ──► LineDiscipline (ISIG, ICANON, ECHO, IXON...)
//!                                   │ echo                │ signals ──► Host::signal_group(foreground)
//!   program ──► TtyFile::write ──► output processing (OPOST) ──► Driver::write (serial, screen, pty master)
//!   program ◄── TtyFile::read ◄── lines / MIN-TIME ◄─────┘   job control: SIGTTIN / SIGTTOU / EIO
//!
//!   /dev/ptmx (Ptmx) ──open──► PtyMaster ──write──► slave Tty input
//!                                        ◄──read─── slave Tty output
//!   /dev/pts/<n> (DevPts, SlaveDevice) ──open──► slave TtyFile
//!   /dev/tty (ControllingTty), /dev/console (Console), /dev/ttyS0 (TtyDevice) ──open──► TtyFile
//! ```
//!
//! - [`termios`]: `struct termios` / `termios2` / `termio` / `winsize`,
//!   flags and special characters (UAPI `asm-generic/termbits.h`).
//! - [`ioctl`]: the request numbers (`asm-generic/ioctls.h`).
//! - [`ldisc`]: the line discipline, plain data.
//! - [`Tty`] / [`TtyFile`] / [`Ttys`]: a terminal, an open file on it,
//!   and what terminals share (the controlling terminal of each session).
//! - [`pty`]: `/dev/ptmx`, masters and slaves; [`devpts`]: `/dev/pts`.
//! - [`device`]: the nodes that stand for terminals.
//!
//! Sources: termios(3), ioctl_tty(2) (tty_ioctl(4)), pty(7), pts(4),
//! credentials(7), tty(4), console(4), ttyS(4), grantpt(3), unlockpt(3),
//! posix_openpt(3), tcsetpgrp(3), tcgetpgrp(3), POSIX.1-2024 "General
//! Terminal Interface" and `_exit`; constants and layouts from the UAPI
//! headers named where they are defined. No Linux or C library source was
//! read and nothing was observed on a real system.

#![no_std]

extern crate alloc;
#[cfg(test)]
extern crate std;

pub mod device;
pub mod devpts;
pub mod ioctl;
pub mod ldisc;
pub mod pty;
pub mod termios;
mod tty;

#[cfg(test)]
mod tests;

pub use tty::{Caller, Driver, Host, Tty, TtyFile, Ttys};

/// The signals terminals send (`asm-generic/signal.h`, which x86-64 uses).
pub mod sig {
    pub const SIGHUP: u32 = 1;
    pub const SIGINT: u32 = 2;
    pub const SIGQUIT: u32 = 3;
    pub const SIGCONT: u32 = 18;
    pub const SIGTSTP: u32 = 20;
    pub const SIGTTIN: u32 = 21;
    pub const SIGTTOU: u32 = 22;
    pub const SIGWINCH: u32 = 28;
}

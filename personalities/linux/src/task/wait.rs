//! wait4(2) and waitid(2): their options, the status word and the
//! `siginfo_t` they report, and the `SIGCHLD` a parent gets.
//!
//! Option bits from the Linux UAPI header `linux/wait.h` (`WNOHANG`,
//! `WUNTRACED`/`WSTOPPED`, `WEXITED`, `WCONTINUED`, `WNOWAIT`,
//! `__WNOTHREAD`, `__WALL`, `__WCLONE`, and the `P_*` id types); the
//! `si_code` values (`CLD_*`) and the `_sigchld` layout from
//! `asm-generic/siginfo.h`. The status word is what the `W*` macros of
//! `<sys/wait.h>` (POSIX; the C library's public header) decode: exit
//! status in bits 8-15 with 0 below; the terminating signal in bits 0-6
//! with bit 7 for a core dump; `0x7f` with the stop signal in bits 8-15;
//! `0xffff` for a continued child.

use super::tree::{ChildEvent, Clones, Selector, Termination, WaitFilter, Waited};
use crate::abi::SigInfo;
use crate::abi::signal::SIGCHLD;
use crate::errno::Errno;

pub const WNOHANG: u32 = 0x0000_0001;
pub const WUNTRACED: u32 = 0x0000_0002;
pub const WSTOPPED: u32 = WUNTRACED;
pub const WEXITED: u32 = 0x0000_0004;
pub const WCONTINUED: u32 = 0x0000_0008;
pub const WNOWAIT: u32 = 0x0100_0000;
pub const WNOTHREAD: u32 = 0x2000_0000;
pub const WALL: u32 = 0x4000_0000;
pub const WCLONE: u32 = 0x8000_0000;

/// waitid(2) id types.
pub const P_ALL: u32 = 0;
pub const P_PID: u32 = 1;
pub const P_PGID: u32 = 2;
pub const P_PIDFD: u32 = 3;

/// `si_code` of `SIGCHLD`.
pub const CLD_EXITED: i32 = 1;
pub const CLD_KILLED: i32 = 2;
pub const CLD_DUMPED: i32 = 3;
pub const CLD_TRAPPED: i32 = 4;
pub const CLD_STOPPED: i32 = 5;
pub const CLD_CONTINUED: i32 = 6;

/// `sizeof(struct rusage)` on x86-64 (`linux/resource.h`: two
/// `struct timeval` and fourteen `long`s).
pub const RUSAGE_SIZE: usize = 144;

/// The options wait4 accepts (wait4(2) / wait(2)); anything else is
/// `EINVAL`.
const WAIT4_OPTIONS: u32 = WNOHANG | WUNTRACED | WCONTINUED | WNOTHREAD | WALL | WCLONE;
/// The options waitid accepts.
const WAITID_OPTIONS: u32 = WNOHANG | WSTOPPED | WEXITED | WCONTINUED | WNOWAIT | WNOTHREAD | WALL | WCLONE;

fn clones(options: u32) -> Clones {
    if options & WALL != 0 {
        Clones::Include
    } else if options & WCLONE != 0 {
        Clones::Only
    } else {
        Clones::Exclude
    }
}

/// What wait4(`pid`, ..., `options`) waits for, by the caller in process
/// group `caller_pgid`: `pid < -1` the group `-pid`, `-1` any child, `0`
/// the caller's group, `> 0` that child (wait(2)). `EINVAL` for unknown
/// options. Also says whether to return at once (`WNOHANG`).
pub fn wait4_filter(pid: i32, options: u32, caller_pgid: i32) -> Result<(WaitFilter, bool), Errno> {
    if options & !WAIT4_OPTIONS != 0 {
        return Err(Errno::EINVAL);
    }
    let selector = match pid {
        -1 => Selector::Any,
        0 => Selector::Group(caller_pgid),
        p if p < -1 => Selector::Group(p.checked_neg().ok_or(Errno::ESRCH)?),
        p => Selector::Pid(p),
    };
    let filter = WaitFilter {
        selector,
        ended: true,
        stopped: options & WUNTRACED != 0,
        continued: options & WCONTINUED != 0,
        consume: true,
        clones: clones(options),
    };
    Ok((filter, options & WNOHANG != 0))
}

/// What waitid(`idtype`, `id`, ..., `options`) waits for. `EINVAL` for an
/// unknown id type, a pidfd (not supported), unknown options, or none of
/// `WEXITED`, `WSTOPPED`, `WCONTINUED`. `P_PGID` with id 0 means the
/// caller's group (wait(2), "since Linux 5.4").
pub fn waitid_filter(idtype: u32, id: i32, options: u32, caller_pgid: i32) -> Result<(WaitFilter, bool), Errno> {
    if options & !WAITID_OPTIONS != 0 || options & (WEXITED | WSTOPPED | WCONTINUED) == 0 {
        return Err(Errno::EINVAL);
    }
    let selector = match idtype {
        P_ALL => Selector::Any,
        P_PID if id > 0 => Selector::Pid(id),
        P_PGID if id == 0 => Selector::Group(caller_pgid),
        P_PGID if id > 0 => Selector::Group(id),
        _ => return Err(Errno::EINVAL),
    };
    let filter = WaitFilter {
        selector,
        ended: options & WEXITED != 0,
        stopped: options & WSTOPPED != 0,
        continued: options & WCONTINUED != 0,
        consume: options & WNOWAIT == 0,
        clones: clones(options),
    };
    Ok((filter, options & WNOHANG != 0))
}

/// The status word wait4 stores for `event`.
pub const fn status_word(event: ChildEvent) -> u32 {
    match event {
        ChildEvent::Ended(Termination::Exited(code)) => (code as u32) << 8,
        ChildEvent::Ended(Termination::Killed { signal, core }) => (signal & 0x7f) | if core { 0x80 } else { 0 },
        ChildEvent::Stopped(signal) => ((signal & 0xff) << 8) | 0x7f,
        ChildEvent::Continued => 0xffff,
    }
}

/// `si_code` and `si_status` for `event` (sigaction(2), `SIGCHLD`
/// fills in "si_pid, si_uid, si_status, si_utime, si_stime": the exit
/// status, or the signal that killed, stopped or continued the child).
pub const fn code_and_status(event: ChildEvent) -> (i32, i32) {
    match event {
        ChildEvent::Ended(Termination::Exited(code)) => (CLD_EXITED, code as i32),
        ChildEvent::Ended(Termination::Killed { signal, core: false }) => (CLD_KILLED, signal as i32),
        ChildEvent::Ended(Termination::Killed { signal, core: true }) => (CLD_DUMPED, signal as i32),
        ChildEvent::Stopped(signal) => (CLD_STOPPED, signal as i32),
        ChildEvent::Continued => (CLD_CONTINUED, crate::abi::signal::SIGCONT as i32),
    }
}

/// The `siginfo_t` of a child's event, as signal `signo` (`SIGCHLD`, or
/// the child's exit signal) and as waitid(2) reports it: `_sigchld` with
/// `si_pid` at 16, `si_uid` at 20, `si_status` at 24, `si_utime` and
/// `si_stime` at 32 and 40 (0: CPU time is not accounted yet).
pub fn child_info(signo: u32, waited: &Waited) -> SigInfo {
    let (code, status) = code_and_status(waited.event);
    let mut info = SigInfo::from_process(signo, code, waited.pid, waited.uid);
    info.fields[8..12].copy_from_slice(&status.to_le_bytes());
    info
}

/// The `SIGCHLD`-style signal for a child's event.
pub fn sigchld(waited: &Waited) -> SigInfo {
    child_info(SIGCHLD, waited)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::abi::signal::{SIGCONT, SIGKILL, SIGSEGV, SIGTSTP};

    #[test]
    fn status_words_decode_with_the_w_macros() {
        let exited = status_word(ChildEvent::Ended(Termination::Exited(42)));
        assert_eq!(exited & 0x7f, 0, "WIFEXITED");
        assert_eq!((exited >> 8) & 0xff, 42, "WEXITSTATUS");
        let killed = status_word(ChildEvent::Ended(Termination::Killed { signal: SIGSEGV, core: true }));
        assert_eq!(killed & 0x7f, SIGSEGV, "WTERMSIG");
        assert_ne!(killed & 0x80, 0, "WCOREDUMP");
        assert!(((killed & 0x7f) + 1) >> 1 > 0, "WIFSIGNALED");
        let stopped = status_word(ChildEvent::Stopped(SIGTSTP));
        assert_eq!(stopped & 0xff, 0x7f, "WIFSTOPPED");
        assert_eq!(stopped >> 8, SIGTSTP, "WSTOPSIG");
        assert_eq!(status_word(ChildEvent::Continued), 0xffff, "WIFCONTINUED");
    }

    #[test]
    fn wait4_selectors_and_options() {
        let f = |pid, options| wait4_filter(pid, options, 7).map(|(f, nohang)| (f.selector, nohang));
        assert_eq!(f(-1, 0), Ok((Selector::Any, false)));
        assert_eq!(f(0, WNOHANG), Ok((Selector::Group(7), true)));
        assert_eq!(f(-5, 0), Ok((Selector::Group(5), false)));
        assert_eq!(f(12, 0), Ok((Selector::Pid(12), false)));
        assert_eq!(f(i32::MIN, 0), Err(Errno::ESRCH));
        assert_eq!(f(-1, WEXITED), Err(Errno::EINVAL));
        assert_eq!(f(-1, 0x10), Err(Errno::EINVAL));
        let (filter, _) = wait4_filter(-1, WUNTRACED | WCONTINUED | WALL, 7).unwrap();
        assert!(filter.ended && filter.stopped && filter.continued && filter.consume);
        assert_eq!(filter.clones, Clones::Include);
        assert_eq!(wait4_filter(-1, WCLONE, 7).unwrap().0.clones, Clones::Only);
    }

    #[test]
    fn waitid_selectors_and_options() {
        let f = |idtype, id, options| waitid_filter(idtype, id, options, 7).map(|(f, _)| f);
        assert_eq!(f(P_ALL, 0, 0), Err(Errno::EINVAL), "nothing to wait for");
        assert_eq!(f(P_PIDFD, 3, WEXITED), Err(Errno::EINVAL));
        assert_eq!(f(9, 3, WEXITED), Err(Errno::EINVAL));
        assert_eq!(f(P_PID, 0, WEXITED), Err(Errno::EINVAL));
        assert_eq!(f(P_PGID, 0, WEXITED).unwrap().selector, Selector::Group(7));
        let filter = f(P_PID, 12, WSTOPPED | WNOWAIT).unwrap();
        assert_eq!(filter.selector, Selector::Pid(12));
        assert!(!filter.ended && filter.stopped && !filter.consume);
    }

    #[test]
    fn sigchld_info() {
        let w = Waited { pid: 12, uid: 1000, event: ChildEvent::Ended(Termination::Exited(3)) };
        let info = sigchld(&w);
        assert_eq!((info.signal(), info.code, info.pid(), info.uid()), (SIGCHLD, CLD_EXITED, 12, 1000));
        assert_eq!(i32::from_le_bytes(info.fields[8..12].try_into().unwrap()), 3);
        let killed = Waited { event: ChildEvent::Ended(Termination::Killed { signal: SIGKILL, core: false }), ..w };
        assert_eq!(code_and_status(killed.event), (CLD_KILLED, SIGKILL as i32));
        assert_eq!(code_and_status(ChildEvent::Continued), (CLD_CONTINUED, SIGCONT as i32));
        assert_eq!(code_and_status(ChildEvent::Stopped(SIGTSTP)).0, CLD_STOPPED);
    }
}

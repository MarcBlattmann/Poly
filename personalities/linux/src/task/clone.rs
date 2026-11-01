//! clone(2), clone3(2), fork(2) and vfork(2): which flags exist, which
//! combinations are refused, and what the kernel is asked to do
//! ([`CloneRequest`]).
//!
//! Flag values are from the Linux UAPI header `linux/sched.h`; the
//! `struct clone_args` layout and its size constants (`CLONE_ARGS_SIZE_VER0`
//! to `VER2`) from the same header. The rules come from clone(2), fork(2)
//! and vfork(2):
//!
//! - `CLONE_SIGHAND` needs `CLONE_VM`, `CLONE_THREAD` needs
//!   `CLONE_SIGHAND` ("EINVAL ... was specified in the flags mask, but
//!   ... was not");
//! - `CLONE_FS` with `CLONE_NEWNS`, and `CLONE_CLEAR_SIGHAND` with
//!   `CLONE_SIGHAND`, are `EINVAL`;
//! - `CLONE_PARENT` from the init process (pid 1) is `EINVAL`;
//! - the namespace flags (`CLONE_NEWUSER`, `CLONE_NEWPID`, `CLONE_NEWNS`,
//!   `CLONE_NEWNET`, `CLONE_NEWUTS`, `CLONE_NEWIPC`) make namespaces for
//!   a new process ([`crate::ns`], ADR 0054) with clone(2)'s rules
//!   ([`crate::ns::check_clone_flags`]); Poly's namespaces belong to
//!   processes, so with `CLONE_THREAD` any of them is `EINVAL`;
//! - cgroup and time namespaces (`CLONE_NEWCGROUP`, `CLONE_NEWTIME`),
//!   cgroups (`CLONE_INTO_CGROUP`) and `set_tid` are not supported:
//!   `EINVAL`, as clone(2) documents for a kernel built without them;
//! - `CLONE_PIDFD` stores a pidfd for the child (pidfd_open(2)) at
//!   clone3's `pidfd`, or at clone(2)'s `parent_tid`, which is why clone(2)
//!   refuses it with `CLONE_PARENT_SETTID`; with `CLONE_DETACHED` or
//!   `CLONE_THREAD` it is `EINVAL` (clone(2));
//! - `CLONE_DETACHED` is ignored by clone(2) ("this flag is ignored" since
//!   2.6.2) and refused by clone3 (its spec sheet);
//!   `CLONE_PTRACE`, `CLONE_UNTRACED`, `CLONE_IO` and `CLONE_SYSVSEM` are
//!   accepted and have no effect yet (no tracing, I/O contexts or System V
//!   semaphores);
//! - the low byte of clone(2)'s flags (`CSIGNAL`) is the signal the parent
//!   gets when the child ends; clone3 has `exit_signal` for it and refuses
//!   those bits in `flags`.
//!
//! The order of the checks is Poly's; where the man pages list several
//! errors for one call without an order, the diff runner confirms it.

use crate::abi::signal::{NSIG, SIGCHLD};
use crate::errno::Errno;
use crate::kernel::UserPtr;

/// The signal-number byte of clone(2)'s flags.
pub const CSIGNAL: u64 = 0x0000_00ff;
pub const CLONE_NEWTIME: u64 = 0x0000_0080;
pub const CLONE_VM: u64 = 0x0000_0100;
pub const CLONE_FS: u64 = 0x0000_0200;
pub const CLONE_FILES: u64 = 0x0000_0400;
pub const CLONE_SIGHAND: u64 = 0x0000_0800;
pub const CLONE_PIDFD: u64 = 0x0000_1000;
pub const CLONE_PTRACE: u64 = 0x0000_2000;
pub const CLONE_VFORK: u64 = 0x0000_4000;
pub const CLONE_PARENT: u64 = 0x0000_8000;
pub const CLONE_THREAD: u64 = 0x0001_0000;
pub const CLONE_NEWNS: u64 = 0x0002_0000;
pub const CLONE_SYSVSEM: u64 = 0x0004_0000;
pub const CLONE_SETTLS: u64 = 0x0008_0000;
pub const CLONE_PARENT_SETTID: u64 = 0x0010_0000;
pub const CLONE_CHILD_CLEARTID: u64 = 0x0020_0000;
pub const CLONE_DETACHED: u64 = 0x0040_0000;
pub const CLONE_UNTRACED: u64 = 0x0080_0000;
pub const CLONE_CHILD_SETTID: u64 = 0x0100_0000;
pub const CLONE_NEWCGROUP: u64 = 0x0200_0000;
pub const CLONE_NEWUTS: u64 = 0x0400_0000;
pub const CLONE_NEWIPC: u64 = 0x0800_0000;
pub const CLONE_NEWUSER: u64 = 0x1000_0000;
pub const CLONE_NEWPID: u64 = 0x2000_0000;
pub const CLONE_NEWNET: u64 = 0x4000_0000;
pub const CLONE_IO: u64 = 0x8000_0000;
/// clone3 only.
pub const CLONE_CLEAR_SIGHAND: u64 = 0x1_0000_0000;
/// clone3 only.
pub const CLONE_INTO_CGROUP: u64 = 0x2_0000_0000;

/// Every namespace flag.
pub const CLONE_NAMESPACES: u64 =
    CLONE_NEWNS | CLONE_NEWCGROUP | CLONE_NEWUTS | CLONE_NEWIPC | CLONE_NEWUSER | CLONE_NEWPID | CLONE_NEWNET;

/// The flags clone3 knows (everything but the `CSIGNAL` byte, plus the
/// clone3-only ones; `CLONE_NEWTIME` sits inside that byte).
pub const CLONE3_KNOWN: u64 = (0xffff_ff00 | CLONE_NEWTIME) | CLONE_CLEAR_SIGHAND | CLONE_INTO_CGROUP;

/// `sizeof(struct clone_args)` of each version (`CLONE_ARGS_SIZE_VER0`,
/// `VER1` with `set_tid`, `VER2` with `cgroup`).
pub const CLONE_ARGS_SIZE_VER0: usize = 64;
pub const CLONE_ARGS_SIZE_VER1: usize = 80;
pub const CLONE_ARGS_SIZE_VER2: usize = 88;

/// What the kernel is asked to create: a thread of the calling process
/// (`CLONE_THREAD`) or a new process, and what it shares with the caller.
///
/// The child starts with the caller's user registers (and FPU state) as
/// they are at the system call, except: `rax` is 0, `rsp` is
/// [`stack`](Self::stack) if given, and the FS base is
/// [`tls`](Self::tls) if given. Before it runs, the kernel stores its
/// thread id at [`parent_tid`](Self::parent_tid) in the caller's memory and
/// at [`child_tid`](Self::child_tid) in the child's memory (failures are
/// ignored), and sets its `clear_child_tid` to
/// [`clear_child_tid`](Self::clear_child_tid) (set_tid_address(2)).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct CloneRequest {
    /// The `CLONE_*` flags, without the `CSIGNAL` byte.
    pub flags: u64,
    /// The signal the parent gets when a child process ends (0: none).
    /// Ignored for threads.
    pub exit_signal: u32,
    /// The child's stack pointer; `None`: the caller's.
    pub stack: Option<u64>,
    /// `CLONE_SETTLS`: the child's FS base.
    pub tls: Option<u64>,
    /// `CLONE_PARENT_SETTID`.
    pub parent_tid: Option<UserPtr>,
    /// `CLONE_CHILD_SETTID`.
    pub child_tid: Option<UserPtr>,
    /// `CLONE_CHILD_CLEARTID`.
    pub clear_child_tid: Option<UserPtr>,
    /// `CLONE_PIDFD`: where the caller gets the child's pidfd (an `int`).
    pub pidfd: Option<UserPtr>,
}

impl CloneRequest {
    /// fork(2): "a new process ... an exact duplicate of the calling
    /// process", the parent told with `SIGCHLD`.
    pub const fn fork() -> CloneRequest {
        CloneRequest {
            flags: 0,
            exit_signal: SIGCHLD,
            stack: None,
            tls: None,
            parent_tid: None,
            child_tid: None,
            clear_child_tid: None,
            pidfd: None,
        }
    }

    /// vfork(2): `CLONE_VM | CLONE_VFORK` with `SIGCHLD`: the child runs in
    /// the caller's memory while the caller waits for it to exec or end.
    pub const fn vfork() -> CloneRequest {
        CloneRequest { flags: CLONE_VM | CLONE_VFORK, ..CloneRequest::fork() }
    }

    const fn has(&self, flag: u64) -> bool {
        self.flags & flag != 0
    }

    /// `CLONE_THREAD`: a thread of the caller's process.
    pub const fn thread(&self) -> bool {
        self.has(CLONE_THREAD)
    }

    /// `CLONE_VM`: the child runs in the caller's memory, not a copy.
    pub const fn shares_memory(&self) -> bool {
        self.has(CLONE_VM)
    }

    /// `CLONE_VFORK`: the caller waits until the child execs or ends.
    pub const fn vfork_wait(&self) -> bool {
        self.has(CLONE_VFORK)
    }

    /// `CLONE_PARENT`: the child's parent is the caller's parent.
    pub const fn sibling(&self) -> bool {
        self.has(CLONE_PARENT)
    }

    /// `CLONE_FILES`: the descriptor table is shared, not copied.
    pub const fn shares_files(&self) -> bool {
        self.has(CLONE_FILES)
    }

    /// `CLONE_FS`: root, working directory and umask are shared.
    pub const fn shares_fs(&self) -> bool {
        self.has(CLONE_FS)
    }

    /// `CLONE_SIGHAND`: the signal dispositions are shared.
    pub const fn shares_handlers(&self) -> bool {
        self.has(CLONE_SIGHAND)
    }

    /// `CLONE_CLEAR_SIGHAND`: handled signals start at `SIG_DFL` in the
    /// child.
    pub const fn clears_handlers(&self) -> bool {
        self.has(CLONE_CLEAR_SIGHAND)
    }

    /// Whether the child's alternate signal stack starts disabled
    /// (sigaltstack(2): "CLONE_VM and do not include CLONE_VFORK").
    pub const fn drops_altstack(&self) -> bool {
        self.has(CLONE_VM) && !self.has(CLONE_VFORK)
    }
}

/// The checks every way of cloning shares, on flags without the `CSIGNAL`
/// byte. `caller_pid` decides the init rule of `CLONE_PARENT`.
fn check_flags(flags: u64, caller_pid: i32) -> Result<(), Errno> {
    let has = |f: u64| flags & f != 0;
    if has(CLONE_SIGHAND) && !has(CLONE_VM) {
        return Err(Errno::EINVAL);
    }
    if has(CLONE_THREAD) && !has(CLONE_SIGHAND) {
        return Err(Errno::EINVAL);
    }
    if has(CLONE_FS) && has(CLONE_NEWNS) {
        return Err(Errno::EINVAL);
    }
    if has(CLONE_CLEAR_SIGHAND) && has(CLONE_SIGHAND) {
        return Err(Errno::EINVAL);
    }
    if has(CLONE_NEWCGROUP | CLONE_NEWTIME | CLONE_INTO_CGROUP) {
        return Err(Errno::EINVAL);
    }
    if has(CLONE_PIDFD) && has(CLONE_DETACHED | CLONE_THREAD) {
        return Err(Errno::EINVAL);
    }
    if has(CLONE_THREAD) && has(crate::ns::NEW_NAMESPACES) {
        return Err(Errno::EINVAL);
    }
    crate::ns::check_clone_flags(flags)?;
    if has(CLONE_PARENT) && caller_pid == 1 {
        return Err(Errno::EINVAL);
    }
    Ok(())
}

/// Decode clone(2)'s arguments (x86-64 order: `flags, stack, parent_tid,
/// child_tid, tls`). A `stack` of 0 means the caller's stack.
pub fn decode_clone(
    flags: u64,
    stack: u64,
    parent_tid: UserPtr,
    child_tid: UserPtr,
    tls: u64,
    caller_pid: i32,
) -> Result<CloneRequest, Errno> {
    // clone(2)'s flags are an `unsigned long`, but the clone3-only bits
    // above 32 do not exist for it (to be confirmed by the diff runner:
    // Poly ignores them like the rest of the upper half).
    let flags = flags & 0xffff_ffff;
    let exit_signal = (flags & CSIGNAL) as u32;
    let flags = flags & !CSIGNAL;
    check_flags(flags, caller_pid)?;
    // clone(2): "CLONE_PIDFD ... together with CLONE_PARENT_SETTID" is
    // EINVAL for clone(), which stores the pidfd at parent_tid.
    if flags & CLONE_PIDFD != 0 && flags & CLONE_PARENT_SETTID != 0 {
        return Err(Errno::EINVAL);
    }
    if exit_signal > NSIG {
        return Err(Errno::EINVAL);
    }
    let mut r = request(flags, exit_signal, (stack != 0).then_some(stack), parent_tid, child_tid, tls);
    r.pidfd = (flags & CLONE_PIDFD != 0).then_some(parent_tid);
    Ok(r)
}

fn request(flags: u64, exit_signal: u32, stack: Option<u64>, ptid: UserPtr, ctid: UserPtr, tls: u64) -> CloneRequest {
    let flags = flags & !CLONE_DETACHED;
    let when = |f: u64, p: UserPtr| (flags & f != 0).then_some(p);
    CloneRequest {
        flags,
        exit_signal: if flags & CLONE_THREAD != 0 { 0 } else { exit_signal },
        stack,
        tls: (flags & CLONE_SETTLS != 0).then_some(tls),
        parent_tid: when(CLONE_PARENT_SETTID, ptid),
        child_tid: when(CLONE_CHILD_SETTID, ctid),
        clear_child_tid: when(CLONE_CHILD_CLEARTID, ctid),
        pidfd: None,
    }
}

/// `struct clone_args` (`linux/sched.h`): eleven `__aligned_u64` fields.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct CloneArgs {
    pub flags: u64,
    pub pidfd: u64,
    pub child_tid: u64,
    pub parent_tid: u64,
    pub exit_signal: u64,
    pub stack: u64,
    pub stack_size: u64,
    pub tls: u64,
    pub set_tid: u64,
    pub set_tid_size: u64,
    pub cgroup: u64,
}

impl CloneArgs {
    /// The structure from the first `bytes.len()` bytes user space passed
    /// (at least [`CLONE_ARGS_SIZE_VER0`]; missing later fields are 0).
    pub fn from_bytes(bytes: &[u8]) -> CloneArgs {
        let field =
            |i: usize| bytes.get(i * 8..i * 8 + 8).map_or(0, |b| u64::from_le_bytes(b.try_into().expect("8 bytes")));
        CloneArgs {
            flags: field(0),
            pidfd: field(1),
            child_tid: field(2),
            parent_tid: field(3),
            exit_signal: field(4),
            stack: field(5),
            stack_size: field(6),
            tls: field(7),
            set_tid: field(8),
            set_tid_size: field(9),
            cgroup: field(10),
        }
    }
}

/// The checks of clone3's `size` argument (clone(2): "EINVAL size is
/// smaller than CLONE_ARGS_SIZE_VER0"; "E2BIG size is larger than the
/// kernel's struct clone_args" with non-zero bytes beyond it, and larger
/// than a page).
pub fn check_clone3_size(size: u64) -> Result<usize, Errno> {
    if size < CLONE_ARGS_SIZE_VER0 as u64 {
        return Err(Errno::EINVAL);
    }
    if size > crate::abi::PAGE_SIZE {
        return Err(Errno::E2BIG);
    }
    Ok(size as usize)
}

/// Decode clone3(2)'s structure. `extra_nonzero`: bytes past the
/// structure Poly knows were not all zero (`E2BIG`).
pub fn decode_clone3(args: &CloneArgs, extra_nonzero: bool, caller_pid: i32) -> Result<CloneRequest, Errno> {
    if extra_nonzero {
        return Err(Errno::E2BIG);
    }
    if args.flags & !CLONE3_KNOWN != 0 || args.flags & CSIGNAL & !CLONE_NEWTIME != 0 {
        return Err(Errno::EINVAL);
    }
    // clone3 has no use for the ignored CLONE_DETACHED (its spec sheet).
    if args.flags & CLONE_DETACHED != 0 {
        return Err(Errno::EINVAL);
    }
    // clone(2): "exit_signal ... must be a valid signal number" (0 or
    // 1-64); "stack and stack_size ... must both be given or neither".
    if args.exit_signal > u64::from(NSIG) {
        return Err(Errno::EINVAL);
    }
    if (args.stack == 0) != (args.stack_size == 0) {
        return Err(Errno::EINVAL);
    }
    if args.set_tid != 0 || args.set_tid_size != 0 || args.cgroup != 0 {
        return Err(Errno::EINVAL);
    }
    check_flags(args.flags, caller_pid)?;
    // The stack grows down from its end.
    let stack = match args.stack {
        0 => None,
        s => Some(s.checked_add(args.stack_size).ok_or(Errno::EINVAL)?),
    };
    let mut r = request(
        args.flags,
        args.exit_signal as u32,
        stack,
        UserPtr(args.parent_tid),
        UserPtr(args.child_tid),
        args.tls,
    );
    // clone3 has a separate pidfd field, so CLONE_PARENT_SETTID uses
    // parent_tid alone; both tid pointers are what the structure says.
    r.parent_tid = (args.flags & CLONE_PARENT_SETTID != 0).then_some(UserPtr(args.parent_tid));
    r.pidfd = (args.flags & CLONE_PIDFD != 0).then_some(UserPtr(args.pidfd));
    Ok(r)
}

#[cfg(test)]
mod tests {
    use super::*;

    const THREAD_FLAGS: u64 = CLONE_VM
        | CLONE_FS
        | CLONE_FILES
        | CLONE_SIGHAND
        | CLONE_THREAD
        | CLONE_SYSVSEM
        | CLONE_SETTLS
        | CLONE_PARENT_SETTID
        | CLONE_CHILD_CLEARTID;

    #[test]
    fn a_pthread_create_clone() {
        let r = decode_clone(THREAD_FLAGS, 0x7000_0000, UserPtr(0x5000), UserPtr(0x6000), 0x8000, 100).unwrap();
        assert!(r.thread() && r.shares_memory() && r.shares_files() && r.shares_fs() && r.shares_handlers());
        assert_eq!(r.stack, Some(0x7000_0000));
        assert_eq!(r.tls, Some(0x8000));
        assert_eq!(r.parent_tid, Some(UserPtr(0x5000)));
        assert_eq!(r.clear_child_tid, Some(UserPtr(0x6000)));
        assert_eq!(r.child_tid, None);
        assert_eq!(r.exit_signal, 0, "threads do not signal their parent");
        assert!(r.drops_altstack());
    }

    #[test]
    fn a_fork_clone() {
        let flags = CLONE_CHILD_SETTID | CLONE_CHILD_CLEARTID | u64::from(SIGCHLD);
        let r = decode_clone(flags, 0, UserPtr(0), UserPtr(0x6000), 0, 100).unwrap();
        assert!(!r.thread() && !r.shares_memory());
        assert_eq!(r.exit_signal, SIGCHLD);
        assert_eq!(r.stack, None);
        assert_eq!(r.tls, None, "no CLONE_SETTLS: the tls argument is ignored");
        assert_eq!((r.child_tid, r.clear_child_tid), (Some(UserPtr(0x6000)), Some(UserPtr(0x6000))));
        assert_eq!(CloneRequest::fork().exit_signal, SIGCHLD);
        assert!(CloneRequest::vfork().vfork_wait() && CloneRequest::vfork().shares_memory());
        assert!(!CloneRequest::vfork().drops_altstack());
    }

    #[test]
    fn refused_combinations() {
        let p = |flags| decode_clone(flags, 0, UserPtr(0), UserPtr(0), 0, 100).err();
        assert_eq!(p(CLONE_SIGHAND), Some(Errno::EINVAL));
        assert_eq!(p(CLONE_VM | CLONE_THREAD), Some(Errno::EINVAL));
        assert_eq!(p(CLONE_FS | CLONE_NEWNS), Some(Errno::EINVAL));
        assert_eq!(p(CLONE_NEWPID), None);
        assert_eq!(p(CLONE_NEWUSER | CLONE_NEWPID | CLONE_NEWNET | CLONE_NEWNS | CLONE_NEWUTS | CLONE_NEWIPC), None);
        assert_eq!(p(CLONE_NEWCGROUP), Some(Errno::EINVAL));
        assert_eq!(p(CLONE_NEWUSER | CLONE_FS), Some(Errno::EINVAL));
        assert_eq!(p(CLONE_NEWIPC | CLONE_SYSVSEM), Some(Errno::EINVAL));
        assert_eq!(p(CLONE_VM | CLONE_SIGHAND | CLONE_THREAD | CLONE_NEWUTS), Some(Errno::EINVAL));
        assert_eq!(p(CLONE_PIDFD), None);
        assert_eq!(p(CLONE_PIDFD | CLONE_PARENT_SETTID), Some(Errno::EINVAL));
        assert_eq!(p(CLONE_PIDFD | CLONE_DETACHED), Some(Errno::EINVAL));
        assert_eq!(p(CLONE_PIDFD | CLONE_THREAD | CLONE_SIGHAND | CLONE_VM), Some(Errno::EINVAL));
        let r = decode_clone(CLONE_PIDFD | 17, 0, UserPtr(0x5000), UserPtr(0), 0, 100).unwrap();
        assert_eq!((r.pidfd, r.parent_tid), (Some(UserPtr(0x5000)), None), "clone() puts the pidfd at parent_tid");
        assert_eq!(p(CLONE_DETACHED | u64::from(SIGCHLD)), None, "CLONE_DETACHED is ignored");
        assert_eq!(p(CLONE_VM | CLONE_SIGHAND), None);
        assert_eq!(decode_clone(CLONE_PARENT, 0, UserPtr(0), UserPtr(0), 0, 1), Err(Errno::EINVAL));
        assert!(decode_clone(CLONE_PARENT, 0, UserPtr(0), UserPtr(0), 0, 2).is_ok());
        assert_eq!(p(1 << 40 | 17).map(|_| ()), None, "upper bits ignored by clone()");
    }

    #[test]
    fn clone3_arguments() {
        let mut a = CloneArgs { flags: THREAD_FLAGS, stack: 0x7000_0000, stack_size: 0x1_0000, ..CloneArgs::default() };
        a.tls = 0x9000;
        a.parent_tid = 0x5000;
        a.child_tid = 0x6000;
        let r = decode_clone3(&a, false, 100).unwrap();
        assert_eq!(r.stack, Some(0x7001_0000), "the stack pointer starts at the top");
        assert_eq!((r.parent_tid, r.clear_child_tid), (Some(UserPtr(0x5000)), Some(UserPtr(0x6000))));
        assert_eq!(decode_clone3(&a, true, 100), Err(Errno::E2BIG));
        let bad = |f: &dyn Fn(&mut CloneArgs)| {
            let mut b = CloneArgs { exit_signal: u64::from(SIGCHLD), ..CloneArgs::default() };
            f(&mut b);
            decode_clone3(&b, false, 100).err()
        };
        assert_eq!(bad(&|_| ()), None);
        assert_eq!(bad(&|b| b.flags = 17), Some(Errno::EINVAL), "CSIGNAL bits in flags");
        assert_eq!(bad(&|b| b.flags = 1 << 40), Some(Errno::EINVAL), "unknown flag");
        assert_eq!(bad(&|b| b.exit_signal = 65), Some(Errno::EINVAL));
        assert_eq!(bad(&|b| b.stack = 0x1000), Some(Errno::EINVAL), "stack without size");
        assert_eq!(bad(&|b| b.stack_size = 0x1000), Some(Errno::EINVAL), "size without stack");
        assert_eq!(bad(&|b| b.set_tid_size = 1), Some(Errno::EINVAL));
        assert_eq!(bad(&|b| b.flags = CLONE_CLEAR_SIGHAND | CLONE_SIGHAND | CLONE_VM), Some(Errno::EINVAL));
        assert_eq!(bad(&|b| b.flags = CLONE_CLEAR_SIGHAND), None);
        assert_eq!(bad(&|b| b.flags = CLONE_DETACHED), Some(Errno::EINVAL), "clone3 refuses CLONE_DETACHED");
        assert_eq!(bad(&|b| b.flags = CLONE_DETACHED | CLONE_PIDFD), Some(Errno::EINVAL));
        assert_eq!(bad(&|b| b.exit_signal = 0x111), Some(Errno::EINVAL), "bits above the low byte");
        let pidfd = CloneArgs {
            flags: CLONE_PIDFD | CLONE_PARENT_SETTID,
            pidfd: 0x7000,
            parent_tid: 0x7004,
            ..CloneArgs::default()
        };
        let r = decode_clone3(&pidfd, false, 100).unwrap();
        assert_eq!((r.pidfd, r.parent_tid), (Some(UserPtr(0x7000)), Some(UserPtr(0x7004))), "clone3 has both fields");
        assert_eq!(check_clone3_size(63), Err(Errno::EINVAL));
        assert_eq!(check_clone3_size(64), Ok(64));
        assert_eq!(check_clone3_size(4097), Err(Errno::E2BIG));
    }

    #[test]
    fn clone_args_from_short_and_long_structures() {
        let mut bytes = [0u8; 88];
        for i in 0..11 {
            bytes[i * 8] = i as u8 + 1;
        }
        let a = CloneArgs::from_bytes(&bytes[..64]);
        assert_eq!((a.flags, a.tls, a.set_tid, a.cgroup), (1, 8, 0, 0));
        let a = CloneArgs::from_bytes(&bytes);
        assert_eq!((a.set_tid, a.set_tid_size, a.cgroup), (9, 10, 11));
    }
}

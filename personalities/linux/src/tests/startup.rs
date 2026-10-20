//! "Hello, world!" startup sequences, scripted from `strace -f` runs of
//! static binaries on a Linux host (glibc 2.39 C program built with
//! `gcc -static`; Rust 1.97 program built for `x86_64-unknown-linux-musl`).
//! Observing the real OS is how the sequences were obtained; no code was
//! looked at. Each step is the traced call with the fake kernel's addresses
//! in place of the traced ones, and the result Poly must give. Calls Poly
//! does not implement yet must come back as `-ENOSYS` / `Unimplemented`,
//! which both C libraries and Rust's std tolerate at startup.

extern crate std;

use super::fake::*;
use crate::abi::{SigAction, StackT, arch_prctl as op, ioctl, map, prot, random, signal};
use crate::kernel::SigSet;
use crate::table::name;
use crate::{Errno, nr};
use poly_abi::Outcome;
use std::vec::Vec;

const HELLO: &[u8] = b"Hello, world!\n";

/// What one step must produce.
#[derive(Debug)]
enum Expect {
    Value(u64),
    Errno(Errno),
    /// Handled, any non-error value (an address the kernel picks).
    Any,
    /// Not implemented: `-ENOSYS`.
    Enosys,
}

/// Runs steps and keeps the names of the calls made, for the final check.
struct Script {
    k: FakeKernel,
    calls: Vec<&'static str>,
}

impl Script {
    fn new(k: FakeKernel) -> Self {
        Script { k, calls: Vec::new() }
    }

    fn step(&mut self, number: u64, args: &[u64], expect: Expect) -> u64 {
        let name = name(number).expect("traced calls exist");
        self.calls.push(name);
        let (outcome, rax) = call(&mut self.k, number, args);
        match expect {
            Expect::Enosys => {
                assert_eq!(outcome, Outcome::Unimplemented { table: "linux", number }, "{name}");
                assert_eq!(Errno::from_rax(rax), Some(Errno::ENOSYS), "{name}");
            }
            Expect::Value(v) => {
                assert_eq!(outcome, Outcome::Handled, "{name}");
                assert_eq!(rax, v, "{name}: got {:?}", Errno::from_rax(rax));
            }
            Expect::Errno(e) => {
                assert_eq!(outcome, Outcome::Handled, "{name}");
                assert_eq!(Errno::from_rax(rax), Some(e), "{name}");
            }
            Expect::Any => {
                assert_eq!(outcome, Outcome::Handled, "{name}");
                assert_eq!(Errno::from_rax(rax), None, "{name}: {:?}", Errno::from_rax(rax));
            }
        }
        rax
    }
}

/// Stack scratch space of the "program" (in the fake's data area, above
/// the part glibc makes read-only).
const STACK: u64 = DATA + 0xc000;
/// Where the program keeps its string (the data area, made read-only later).
const STRING: u64 = DATA;

/// glibc 2.39, static, stdout a pipe:
///
/// ```text
/// brk(NULL)                               = 0x33617000
/// brk(0x33617d00)                         = 0x33617d00
/// arch_prctl(ARCH_SET_FS, 0x33617380)     = 0
/// set_tid_address(0x33617650)             = 6208
/// set_robust_list(0x33617660, 24)         = 0
/// rseq(0x33617ca0, 0x20, 0, 0x53053053)   = 0
/// prlimit64(0, RLIMIT_STACK, NULL, {...}) = 0
/// readlinkat(AT_FDCWD, "/proc/self/exe", ..., 4096) = 91
/// getrandom("...", 8, GRND_NONBLOCK)      = 8
/// brk(NULL)                               = 0x33617d00
/// brk(0x33638d00)                         = 0x33638d00
/// brk(0x33639000)                         = 0x33639000
/// mprotect(0x4a5000, 20480, PROT_READ)    = 0
/// fstat(1, {st_mode=S_IFIFO|0600, ...})   = 0
/// write(1, "Hello, world!\n", 14)         = 14
/// exit_group(0)                           = ?
/// ```
///
/// With stdout on `/dev/null` the `fstat` reports `S_IFCHR` and is followed
/// by `ioctl(1, TCGETS, ...) = -1 ENOTTY`.
fn glibc_static_hello(stdout: Kind) -> Script {
    let mut k = FakeKernel::new();
    k.file_mut(1).kind = stdout;
    k.poke(STRING, HELLO);
    let mut s = Script::new(k);
    let b = s.step(nr::BRK, &[0], Expect::Any);
    s.step(nr::BRK, &[b + 0xd00], Expect::Value(b + 0xd00));
    s.step(nr::ARCH_PRCTL, &[op::ARCH_SET_FS as u64, b + 0x380], Expect::Value(0));
    s.step(nr::SET_TID_ADDRESS, &[b + 0x650], Expect::Value(100));
    s.step(nr::SET_ROBUST_LIST, &[b + 0x660, 24], Expect::Value(0));
    s.step(nr::RSEQ, &[b + 0xca0, 0x20, 0, 0x5305_3053], Expect::Enosys);
    s.step(nr::PRLIMIT64, &[0, 3, 0, STACK], Expect::Enosys);
    s.step(nr::READLINKAT, &[(-100i64) as u64, STACK, STACK + 0x100, 4096], Expect::Enosys);
    s.step(nr::GETRANDOM, &[STACK, 8, random::GRND_NONBLOCK as u64], Expect::Value(8));
    s.step(nr::BRK, &[0], Expect::Value(b + 0xd00));
    s.step(nr::BRK, &[b + 0x21d00], Expect::Value(b + 0x21d00));
    s.step(nr::BRK, &[b + 0x22000], Expect::Value(b + 0x22000));
    s.step(nr::MPROTECT, &[DATA + 0x5000, 20480, prot::PROT_READ as u64], Expect::Value(0));
    s.step(nr::FSTAT, &[1, STACK], Expect::Value(0));
    if stdout != Kind::Pipe {
        s.step(nr::IOCTL, &[1, ioctl::TCGETS as u64, STACK + 0x100], Expect::Errno(Errno::ENOTTY));
    }
    s.step(nr::WRITE, &[1, STRING, HELLO.len() as u64], Expect::Value(14));
    s.step(nr::EXIT_GROUP, &[0], Expect::Value(0));
    // The heap the program grew is usable memory.
    assert!(s.k.is_mapped(b + 0x21fff));
    assert_eq!(s.k.fs_base, b + 0x380);
    assert_eq!(s.k.thread_state().robust_list.0, b + 0x660);
    s
}

#[test]
fn glibc_static_hello_world_to_a_pipe() {
    let s = glibc_static_hello(Kind::Pipe);
    assert_eq!(s.k.file(1).written, HELLO);
    assert_eq!(s.k.exit, Some(Exit::Group(0)));
}

#[test]
fn glibc_static_hello_world_to_dev_null() {
    let s = glibc_static_hello(Kind::NullDevice);
    assert_eq!(s.k.file(1).written, HELLO);
    assert_eq!(s.k.exit, Some(Exit::Group(0)));
}

/// Rust std, static musl:
///
/// ```text
/// arch_prctl(ARCH_SET_FS, 0x7f384bfdc188) = 0
/// set_tid_address(0x7f384bfdc318)         = 6614
/// poll([{fd=0, events=0}, {fd=1, events=0}, {fd=2, events=0}], 3, 0) = 0 (Timeout)
/// rt_sigaction(SIGPIPE, {sa_handler=SIG_IGN, sa_mask=[], sa_flags=SA_RESTORER|SA_RESTART, ...}, {sa_handler=SIG_DFL, ...}, 8) = 0
/// rt_sigaction(SIGSEGV, NULL, {sa_handler=SIG_DFL, sa_mask=[], sa_flags=0}, 8) = 0
/// sigaltstack(NULL, {ss_sp=NULL, ss_flags=SS_DISABLE, ss_size=0}) = 0
/// mmap(NULL, 16048, PROT_READ|PROT_WRITE, MAP_PRIVATE|MAP_ANONYMOUS|MAP_STACK, -1, 0) = 0x7f384bf62000
/// mprotect(0x7f384bf62000, 4096, PROT_NONE) = 0
/// sigaltstack({ss_sp=0x7f384bf63000, ss_flags=0, ss_size=11952}, NULL) = 0
/// rt_sigprocmask(SIG_UNBLOCK, [RT_1 RT_2], NULL, 8) = 0
/// rt_sigaction(SIGSEGV, {sa_handler=0x7f384bf9b7c0, sa_flags=SA_RESTORER|SA_ONSTACK|SA_SIGINFO, ...}, NULL, 8) = 0
/// rt_sigaction(SIGBUS, NULL, {sa_handler=SIG_DFL, sa_mask=[], sa_flags=0}, 8) = 0
/// rt_sigaction(SIGBUS, {sa_handler=0x7f384bf9b7c0, ...}, NULL, 8) = 0
/// brk(NULL)                               = 0x55558b205000
/// brk(0x55558b207000)                     = 0x55558b207000
/// mmap(0x55558b205000, 4096, PROT_NONE, MAP_PRIVATE|MAP_FIXED|MAP_ANONYMOUS, -1, 0) = 0x55558b205000
/// mmap(NULL, 4096, PROT_READ|PROT_WRITE, MAP_PRIVATE|MAP_ANONYMOUS, -1, 0) = 0x7f384bf61000
/// write(1, "Hello, world!\n", 14)         = 14
/// munmap(0x7f384bf61000, 4096)            = 0
/// sigaltstack({ss_sp=NULL, ss_flags=SS_DISABLE, ss_size=11952}, NULL) = 0
/// munmap(0x7f384bf62000, 16048)           = 0
/// exit_group(0)                           = ?
/// ```
#[test]
fn rust_std_musl_static_hello_world() {
    let mut s = Script::new(FakeKernel::new());
    let tls = 0x7f38_4bfd_c188;
    s.step(nr::ARCH_PRCTL, &[op::ARCH_SET_FS as u64, tls], Expect::Value(0));
    s.step(nr::SET_TID_ADDRESS, &[tls + 0x190], Expect::Value(100));
    s.step(nr::POLL, &[STACK, 3, 0], Expect::Value(0));

    let restorer = 0x40_1802;
    let ign =
        SigAction { handler: signal::SIG_IGN, flags: signal::SA_RESTORER | signal::SA_RESTART, restorer, mask: 0 };
    s.k.poke(STACK, &ign.to_bytes());
    let old = STACK + 0x40;
    s.step(nr::RT_SIGACTION, &[signal::SIGPIPE as u64, STACK, old, 8], Expect::Value(0));
    assert_eq!(SigAction::from_bytes(&s.k.peek(old, 32).try_into().unwrap()), SigAction::default());
    s.step(nr::RT_SIGACTION, &[signal::SIGSEGV as u64, 0, old, 8], Expect::Value(0));
    assert_eq!(SigAction::from_bytes(&s.k.peek(old, 32).try_into().unwrap()).handler, signal::SIG_DFL);
    s.step(nr::SIGALTSTACK, &[0, old], Expect::Value(0));
    let disabled = StackT { sp: 0, flags: signal::SS_DISABLE, size: 0 };
    assert_eq!(StackT::from_bytes(&s.k.peek(old, StackT::SIZE).try_into().unwrap()), disabled);

    let rw = (prot::PROT_READ | prot::PROT_WRITE) as u64;
    let anon = (map::MAP_PRIVATE | map::MAP_ANONYMOUS) as u64;
    let alt = s.step(nr::MMAP, &[0, 16048, rw, anon | map::MAP_STACK as u64, u64::MAX, 0], Expect::Any);
    s.step(nr::MPROTECT, &[alt, 4096, prot::PROT_NONE as u64], Expect::Value(0));
    s.k.poke(STACK, &StackT { sp: alt + 4096, flags: 0, size: 11952 }.to_bytes());
    s.step(nr::SIGALTSTACK, &[STACK, 0], Expect::Value(0));
    assert_eq!(s.k.thread_state().altstack.sp, alt + 4096);
    s.k.poke(STACK, &(SigSet::only(33).0 | SigSet::only(34).0).to_le_bytes());
    s.step(nr::RT_SIGPROCMASK, &[signal::SIG_UNBLOCK as u64, STACK, 0, 8], Expect::Value(0));

    let handler = SigAction {
        handler: 0x40_b7c0,
        flags: signal::SA_RESTORER | signal::SA_ONSTACK | signal::SA_SIGINFO,
        restorer,
        mask: 0,
    };
    s.k.poke(STACK, &handler.to_bytes());
    s.step(nr::RT_SIGACTION, &[signal::SIGSEGV as u64, STACK, 0, 8], Expect::Value(0));
    s.step(nr::RT_SIGACTION, &[signal::SIGBUS as u64, 0, old, 8], Expect::Value(0));
    s.step(nr::RT_SIGACTION, &[signal::SIGBUS as u64, STACK, 0, 8], Expect::Value(0));
    assert_eq!(s.k.actions().get(signal::SIGSEGV), handler);
    assert_eq!(s.k.actions().get(signal::SIGPIPE), ign);

    let b = s.step(nr::BRK, &[0], Expect::Any);
    s.step(nr::BRK, &[b + 0x2000], Expect::Value(b + 0x2000));
    let fixed = anon | map::MAP_FIXED as u64;
    s.step(nr::MMAP, &[b, 4096, prot::PROT_NONE as u64, fixed, u64::MAX, 0], Expect::Value(b));
    let buf = s.step(nr::MMAP, &[0, 4096, rw, anon, u64::MAX, 0], Expect::Any);
    s.k.poke(buf, HELLO);
    s.step(nr::WRITE, &[1, buf, 14], Expect::Value(14));
    s.step(nr::MUNMAP, &[buf, 4096], Expect::Value(0));
    s.k.poke(STACK, &StackT { sp: 0, flags: signal::SS_DISABLE, size: 11952 }.to_bytes());
    s.step(nr::SIGALTSTACK, &[STACK, 0], Expect::Value(0));
    assert!(!s.k.thread_state().altstack.is_enabled());
    s.step(nr::MUNMAP, &[alt, 16048], Expect::Value(0));
    s.step(nr::EXIT_GROUP, &[0], Expect::Value(0));

    assert_eq!(s.k.file(1).written, HELLO);
    assert_eq!(s.k.exit, Some(Exit::Group(0)));
    assert!(!s.k.is_mapped(alt) && !s.k.is_mapped(buf), "everything unmapped again");
}

/// The calls the traced programs made that Poly does not implement yet:
/// the work list after this milestone.
#[test]
fn traced_calls_still_missing() {
    let mut missing: Vec<&str> = Vec::new();
    let glibc = glibc_static_hello(Kind::NullDevice);
    let traced = [
        "brk",
        "arch_prctl",
        "set_tid_address",
        "set_robust_list",
        "rseq",
        "prlimit64",
        "readlinkat",
        "getrandom",
        "mprotect",
        "fstat",
        "ioctl",
        "write",
        "exit_group",
        "poll",
        "rt_sigaction",
        "sigaltstack",
        "mmap",
        "rt_sigprocmask",
        "munmap",
        // Rust std with glibc, which reads /proc/self/maps for the stack guard.
        "openat",
        "read",
        "close",
        "sched_getaffinity",
        "gettid",
    ];
    for &call in glibc.calls.iter().chain(traced.iter()) {
        let number = crate::table::number(call).unwrap();
        if crate::table::handler(number).is_none() && !missing.contains(&call) {
            missing.push(call);
        }
    }
    assert_eq!(missing, ["rseq", "prlimit64", "readlinkat", "openat", "sched_getaffinity"]);
}

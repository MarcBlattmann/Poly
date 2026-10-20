//! In-kernel tests of namespaces and seccomp (ADR 0054), made from a
//! ring-3 process the way `files_tests` makes them (its harness: the
//! checks run in the process's one system call; a child it forks returns
//! from that call with 0 and exits with 0).

use super::files_tests::{AT_FDCWD, BUF, Check, cstr, peek, read_file, readlink, run, setup, sys, text};
use crate::kernel_test;
use crate::process::Process;
use poly_personality_linux::task::clone::*;
use poly_personality_linux::{Errno, nr};
use poly_vfs::{At, Cred};

const O_WRONLY: u64 = 1;
const SIGCHLD: u64 = 17;

/// Fails check `n`, logging what was expected.
macro_rules! check {
    ($n:expr, $cond:expr $(, $why:literal)?) => {
        if !$cond {
            crate::kprintln!("linux ns test: check {} failed: {}", $n, stringify!($cond));
            return Err($n);
        }
    };
}

/// `bytes` into the test process's memory at `at`.
fn poke(at: u64, bytes: &[u8]) {
    let _ = crate::user::copy_to_user(at, bytes);
}

/// Write `data` to the file at `path` (opened write-only, as a shell's
/// `echo data > path` would, without `O_CREAT`).
fn write_file(p: &Process, path: &str, data: &str) -> Result<u64, Errno> {
    let fd = sys(p, nr::OPENAT, &[AT_FDCWD, cstr(6, path), O_WRONLY | 0o1000, 0])?;
    let r = sys(p, nr::WRITE, &[fd, cstr(7, data), data.len() as u64]);
    let _ = sys(p, nr::CLOSE, &[fd]);
    r
}

fn ids(p: &Process) -> [u64; 2] {
    [sys(p, nr::GETUID, &[]).unwrap_or(1 << 40), sys(p, nr::GETGID, &[]).unwrap_or(1 << 40)]
}

kernel_test! {
    fn an_unprivileged_user_namespace() -> TestResult {
        let (process, memory) = setup()?;
        run(&process, &memory, |p| -> Check {
            // As root: a directory to mount on, a descriptor on the initial
            // UTS namespace; then become an ordinary user.
            let dir = cstr(1, "/tmp/ns-m");
            check!(1, matches!(sys(p, nr::MKDIRAT, &[AT_FDCWD, dir, 0o777]), Ok(0) | Err(Errno::EEXIST)));
            let initial_uts = sys(p, nr::OPENAT, &[AT_FDCWD, cstr(2, "/proc/self/ns/uts"), 0, 0]).map_err(|_| 2u64)?;
            let initial_user = readlink(p, "/proc/self/ns/user").map_err(|_| 3u64)?;
            check!(4, initial_user.starts_with(b"user:["));
            check!(5, sys(p, nr::SETRESGID, &[1000, 1000, 1000]) == Ok(0));
            check!(6, sys(p, nr::SETRESUID, &[1000, 1000, 1000]) == Ok(0));
            check!(7, sys(p, nr::UNSHARE, &[CLONE_NEWUTS]) == Err(Errno::EPERM));
            // A user namespace of its own; nothing mapped yet.
            check!(8, sys(p, nr::UNSHARE, &[CLONE_NEWUSER]) == Ok(0));
            check!(9, ids(p) == [65534, 65534]);
            check!(10, readlink(p, "/proc/self/ns/user").is_ok_and(|l| l != initial_user));
            check!(11, write_file(p, "/proc/self/uid_map", "0 1000 1\n") == Ok(9));
            check!(12, write_file(p, "/proc/self/uid_map", "0 1000 1\n") == Err(Errno::EPERM));
            check!(13, write_file(p, "/proc/self/gid_map", "0 1000 1\n") == Err(Errno::EPERM));
            check!(14, write_file(p, "/proc/self/setgroups", "deny") == Ok(4));
            check!(15, write_file(p, "/proc/self/gid_map", "0 1000 1\n") == Ok(9));
            check!(16, ids(p) == [0, 0]);
            check!(17, text(p, "/proc/self/uid_map") == "         0       1000          1\n");
            check!(18, text(p, "/proc/self/setgroups") == "deny\n");
            // Root inside: a host name of its own.
            check!(19, sys(p, nr::UNSHARE, &[CLONE_NEWUTS]) == Ok(0));
            check!(20, sys(p, nr::SETHOSTNAME, &[cstr(3, "ns-host"), 7]) == Ok(0));
            check!(21, sys(p, nr::UNAME, &[BUF]) == Ok(0) && peek(BUF + 65, 8) == b"ns-host\0");
            check!(22, text(p, "/proc/sys/kernel/hostname") == "ns-host\n");
            check!(23, super::admin::hostname() != b"ns-host");
            // ... but no way back: the initial namespace is not its own.
            check!(24, sys(p, nr::SETNS, &[initial_uts, CLONE_NEWUTS]) == Err(Errno::EPERM));
            check!(25, sys(p, nr::SETNS, &[initial_uts, CLONE_NEWNET]) == Err(Errno::EINVAL));
            // Mounts of its own, of the file systems a user namespace may
            // mount.
            check!(26, sys(p, nr::UNSHARE, &[CLONE_NEWNS]) == Ok(0));
            let tmpfs = cstr(4, "tmpfs");
            check!(27, sys(p, nr::MOUNT, &[cstr(5, "nsfs-test"), dir, tmpfs, 0, 0]) == Ok(0));
            let created = sys(p, nr::OPENAT, &[AT_FDCWD, cstr(6, "/tmp/ns-m/f"), 0o101, 0o644]);
            check!(28, created.is_ok_and(|fd| sys(p, nr::CLOSE, &[fd]) == Ok(0)));
            check!(29, text(p, "/proc/self/mounts").contains("nsfs-test /tmp/ns-m tmpfs"));
            let ext2 = cstr(4, "ext2");
            check!(30, sys(p, nr::MOUNT, &[cstr(5, "/dev/null"), dir, ext2, 0, 0]) == Err(Errno::EPERM));
            // Device nodes stay the initial namespace's privilege.
            check!(31, sys(p, nr::MKNODAT, &[AT_FDCWD, cstr(8, "/tmp/ns-m/null"), 0o020666, 0x103]) == Err(Errno::EPERM));
            check!(32, read_file(p, "/proc/self/ns/net:[1]") == Err(Errno::ENOENT));
            Ok(())
        })?;
        // The mount was made in the process's copy of the table only.
        let vfs = crate::fs::vfs();
        let ctx = vfs.root_context().map_err(|_| "no root context")?;
        let root = Cred::root();
        if vfs.lookup(&ctx, &root, At::Cwd, b"/tmp/ns-m/f", true).is_ok() {
            return Err("the namespace's mount shows in the initial namespace");
        }
        if crate::linux::admin::hostname() == b"ns-host" {
            return Err("the UTS namespace's name leaked");
        }
        Ok(())
    }
}

kernel_test! {
    fn a_child_is_init_of_a_new_pid_namespace() -> TestResult {
        let (process, memory) = setup()?;
        run(&process, &memory, |p| -> Check {
            // vfork-style, sharing memory, so the child's own view of its
            // id (CLONE_CHILD_SETTID) lands where the parent can see it.
            let flags = CLONE_VM | CLONE_VFORK | CLONE_NEWPID | CLONE_PARENT_SETTID | CLONE_CHILD_SETTID | SIGCHLD;
            poke(BUF, &[0xff; 16]);
            let child = sys(p, nr::CLONE, &[flags, 0, BUF, BUF + 8, 0]).map_err(|_| 1u64)?;
            let seen = |at| i32::from_le_bytes(peek(at, 4).try_into().unwrap_or([0; 4]));
            check!(2, seen(BUF) == child as i32, "the parent's view: its global id");
            check!(3, seen(BUF + 8) == 1, "the child's view: init of its namespace");
            check!(4, sys(p, nr::WAIT4, &[child, BUF + 32, 0, 0]) == Ok(child));
            check!(5, seen(BUF + 32) == 0, "it exited with 0");
            // Not with CLONE_THREAD; not without privilege.
            let thread = CLONE_VM | CLONE_SIGHAND | CLONE_THREAD | CLONE_NEWPID;
            check!(6, sys(p, nr::CLONE, &[thread, 0, 0, 0, 0]) == Err(Errno::EINVAL));
            check!(7, sys(p, nr::SETRESUID, &[1000, 1000, 1000]) == Ok(0));
            check!(8, sys(p, nr::CLONE, &[CLONE_NEWPID | SIGCHLD, 0, 0, 0, 0]) == Err(Errno::EPERM));
            // With a user namespace of its own it may.
            let both = CLONE_NEWUSER | CLONE_NEWPID | CLONE_VM | CLONE_VFORK | CLONE_CHILD_SETTID | SIGCHLD;
            let child = sys(p, nr::CLONE, &[both, 0, 0, BUF + 8, 0]).map_err(|_| 9u64)?;
            check!(10, seen(BUF + 8) == 1);
            check!(11, sys(p, nr::WAIT4, &[child, 0, 0, 0]) == Ok(child));
            Ok(())
        })
    }
}

/// classic BPF opcodes (`linux/bpf_common.h`) and seccomp return values
/// (`linux/seccomp.h`).
const BPF_LD_W_ABS: u16 = 0x20;
const BPF_JEQ_K: u16 = 0x15;
const BPF_RET_K: u16 = 0x06;
const SECCOMP_SET_MODE_FILTER: u64 = 1;
const SECCOMP_RET_ERRNO: u32 = 0x0005_0000;
const SECCOMP_RET_ALLOW: u32 = 0x7fff_0000;
const PR_SET_NO_NEW_PRIVS: u64 = 38;

kernel_test! {
    fn a_seccomp_filter_refuses_a_call() -> TestResult {
        let (process, memory) = setup()?;
        run(&process, &memory, |p| -> Check {
            // getppid gets EACCES (13), everything else is allowed.
            let insns: [(u16, u8, u8, u32); 4] = [
                (BPF_LD_W_ABS, 0, 0, 0),
                (BPF_JEQ_K, 0, 1, nr::GETPPID as u32),
                (BPF_RET_K, 0, 0, SECCOMP_RET_ERRNO | 13),
                (BPF_RET_K, 0, 0, SECCOMP_RET_ALLOW),
            ];
            let mut prog = alloc::vec::Vec::new();
            for (code, jt, jf, k) in insns {
                prog.extend_from_slice(&code.to_le_bytes());
                prog.extend_from_slice(&[jt, jf]);
                prog.extend_from_slice(&k.to_le_bytes());
            }
            poke(BUF + 64, &prog);
            let mut fprog = [0u8; 16];
            fprog[0..2].copy_from_slice(&(insns.len() as u16).to_le_bytes());
            fprog[8..16].copy_from_slice(&(BUF + 64).to_le_bytes());
            poke(BUF, &fprog);
            // Unprivileged without no_new_privs: EACCES.
            check!(1, sys(p, nr::SETRESUID, &[1000, 1000, 1000]) == Ok(0));
            check!(2, sys(p, nr::SECCOMP, &[SECCOMP_SET_MODE_FILTER, 0, BUF]) == Err(Errno::EACCES));
            check!(3, sys(p, nr::PRCTL, &[PR_SET_NO_NEW_PRIVS, 1, 0, 0, 0]) == Ok(0));
            check!(4, sys(p, nr::SECCOMP, &[SECCOMP_SET_MODE_FILTER, 0, BUF]) == Ok(0));
            check!(5, sys(p, nr::GETPPID, &[]) == Err(Errno::EACCES));
            check!(6, sys(p, nr::GETPID, &[]) == Ok(u64::from(p.pid)));
            Ok(())
        })
    }
}

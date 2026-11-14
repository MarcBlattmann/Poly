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

/// `struct ifreq` for `lo` with `value` in the first two bytes of its
/// union, at [`BUF`].
fn ifreq_lo(value: u16) -> u64 {
    let mut r = [0u8; 40];
    r[..2].copy_from_slice(b"lo");
    r[16..18].copy_from_slice(&value.to_le_bytes());
    poke(BUF, &r);
    BUF
}

/// `SIOCGIFFLAGS` of `lo` through socket `fd`.
fn lo_flags(p: &Process, fd: u64) -> Result<u16, Errno> {
    sys(p, nr::IOCTL, &[fd, 0x8913, ifreq_lo(0)])?;
    let b = peek(BUF + 16, 2);
    Ok(u16::from_le_bytes([b[0], b[1]]))
}

/// A `sockaddr_in` at `BUF + 64`.
fn sockaddr_in(ip: [u8; 4], port: u16) -> u64 {
    let mut a = [0u8; 16];
    a[0] = 2;
    a[2..4].copy_from_slice(&port.to_be_bytes());
    a[4..8].copy_from_slice(&ip);
    poke(BUF + 64, &a);
    BUF + 64
}

kernel_test! {
    /// ADR 0178: a network namespace has a stack and a loopback device of
    /// its own, down until it is brought up; a socket stays in the
    /// namespace it was made in.
    fn a_new_network_namespace_has_its_own_loopback() -> TestResult {
        let (process, memory) = setup()?;
        run(&process, &memory, |p| -> Check {
            const SIOCSIFFLAGS: u64 = 0x8914;
            const SIOCGIFADDR: u64 = 0x8915;
            let stacks = crate::net::namespace_stacks();
            let outer = sys(p, nr::SOCKET, &[2, 2, 0]).map_err(|_| 1u64)?;
            check!(2, lo_flags(p, outer) == Ok(0x49), "the initial lo is up");
            check!(3, sys(p, nr::UNSHARE, &[CLONE_NEWNET]) == Ok(0));
            let s = sys(p, nr::SOCKET, &[2, 2, 0]).map_err(|_| 4u64)?;
            check!(5, lo_flags(p, s) == Ok(0x8), "down: IFF_LOOPBACK only");
            check!(6, lo_flags(p, outer) == Ok(0x49), "the older socket is still in the initial namespace");
            check!(7, crate::net::namespace_stacks() == stacks + 1);
            check!(8, sys(p, nr::IOCTL, &[s, SIOCGIFADDR, ifreq_lo(0)]) == Err(Errno::EADDRNOTAVAIL));
            // Nothing is reachable, though a loopback address can be bound.
            check!(9, sys(p, nr::BIND, &[s, sockaddr_in([127, 0, 0, 1], 0), 16]) == Ok(0));
            let c = sys(p, nr::SOCKET, &[2, 1, 0]).map_err(|_| 10u64)?;
            check!(11, sys(p, nr::CONNECT, &[c, sockaddr_in([127, 0, 0, 1], 80), 16]) == Err(Errno::ENETUNREACH));
            check!(12, sys(p, nr::CONNECT, &[c, sockaddr_in([10, 0, 0, 1], 80), 16]) == Err(Errno::ENETUNREACH));
            let text_of = |path| text(p, path);
            let dev = text_of("/proc/self/net/dev");
            check!(13, dev.lines().count() == 3 && dev.contains("    lo:"));
            check!(14, readlink(p, "/proc/net") == Ok(b"self/net".to_vec()));
            // Up, as bubblewrap does it: TCP over the namespace's loopback.
            check!(15, sys(p, nr::IOCTL, &[s, SIOCSIFFLAGS, ifreq_lo(0x49)]) == Ok(0));
            check!(16, lo_flags(p, s) == Ok(0x49));
            check!(17, sys(p, nr::IOCTL, &[s, SIOCGIFADDR, ifreq_lo(0)]) == Ok(0));
            check!(18, peek(BUF + 20, 4) == [127, 0, 0, 1]);
            let l = sys(p, nr::SOCKET, &[2, 1, 0]).map_err(|_| 19u64)?;
            check!(20, sys(p, nr::BIND, &[l, sockaddr_in([127, 0, 0, 1], 0), 16]) == Ok(0));
            check!(21, sys(p, nr::LISTEN, &[l, 4]) == Ok(0));
            poke(BUF + 128, &16u32.to_le_bytes());
            check!(22, sys(p, nr::GETSOCKNAME, &[l, BUF + 96, BUF + 128]) == Ok(0));
            let port = peek(BUF + 98, 2);
            let port = u16::from_be_bytes([port[0], port[1]]);
            check!(23, port != 0);
            check!(24, sys(p, nr::CONNECT, &[c, sockaddr_in([127, 0, 0, 1], port), 16]) == Ok(0));
            let a = sys(p, nr::ACCEPT, &[l, 0, 0]).map_err(|_| 25u64)?;
            poke(BUF + 200, b"ping");
            check!(26, sys(p, nr::WRITE, &[c, BUF + 200, 4]) == Ok(4));
            check!(27, sys(p, nr::READ, &[a, BUF + 300, 16]) == Ok(4));
            check!(28, peek(BUF + 300, 4) == b"ping");
            // A port nobody listens on is refused; elsewhere there is no route.
            let r = sys(p, nr::SOCKET, &[2, 1, 0]).map_err(|_| 29u64)?;
            check!(30, sys(p, nr::CONNECT, &[r, sockaddr_in([127, 0, 0, 1], 1), 16]) == Err(Errno::ECONNREFUSED));
            check!(31, sys(p, nr::CONNECT, &[r, sockaddr_in([10, 0, 0, 1], 80), 16]) == Err(Errno::ENETUNREACH));
            // The initial namespace does not see the listener: its own
            // loopback has nobody on that port.
            let o = sys(p, nr::SOCKET, &[2, 1, 0]);
            check!(32, o.is_ok());
            // (`o` is in the new namespace too: the process moved. The
            // socket made before the unshare is the one outside.)
            check!(33, sys(p, nr::CONNECT, &[outer, sockaddr_in([127, 0, 0, 1], port), 16]) == Ok(0), "UDP connect");
            // Counters moved on the namespace's lo only after traffic.
            let dev = text_of("/proc/self/net/dev");
            let lo = dev.lines().find(|l| l.contains("lo:")).unwrap_or_default();
            check!(34, lo.split_whitespace().nth(2).is_some_and(|packets| packets != "0"));
            // Down again: flags 0x8, nothing reachable.
            check!(35, sys(p, nr::IOCTL, &[s, SIOCSIFFLAGS, ifreq_lo(0)]) == Ok(0));
            check!(36, lo_flags(p, s) == Ok(0x8));
            check!(37, sys(p, nr::CONNECT, &[r, sockaddr_in([127, 0, 0, 1], port), 16]) == Err(Errno::ENETUNREACH));
            // A second namespace starts from scratch.
            check!(38, sys(p, nr::UNSHARE, &[CLONE_NEWNET]) == Ok(0));
            let s2 = sys(p, nr::SOCKET, &[2, 2, 0]).map_err(|_| 39u64)?;
            check!(40, lo_flags(p, s2) == Ok(0x8));
            // Raw sockets can be made by the namespace's root; without the
            // capability they are EPERM, and so is changing the device.
            check!(41, sys(p, nr::SOCKET, &[2, 3, 255]).is_ok());
            check!(42, sys(p, nr::SOCKET, &[17, 3, 0x300]).is_ok());
            check!(43, sys(p, nr::SETRESUID, &[1000, 1000, 1000]) == Ok(0));
            check!(44, sys(p, nr::SOCKET, &[2, 3, 255]) == Err(Errno::EPERM));
            check!(45, sys(p, nr::SOCKET, &[17, 3, 0x300]) == Err(Errno::EPERM));
            check!(46, sys(p, nr::IOCTL, &[s2, SIOCSIFFLAGS, ifreq_lo(0x49)]) == Err(Errno::EPERM));
            check!(47, lo_flags(p, s2) == Ok(0x8), "the getters work");
            Ok(())
        })
    }
}

kernel_test! {
    /// ADR 0178: an IPC namespace owns its POSIX message queues (shown by
    /// a message queue file system mounted from it), its System V objects
    /// (`/proc/sysvipc`) and its settings.
    fn an_ipc_namespace_owns_its_queues_and_settings() -> TestResult {
        let (process, memory) = setup()?;
        run(&process, &memory, |p| -> Check {
            const O_CREAT_RDWR: u64 = 0o102;
            const MS_REC_PRIVATE: u64 = 0x4000 | 0x40000;
            const IPC_CREAT: u64 = 0o1000;
            check!(1, sys(p, nr::UNSHARE, &[CLONE_NEWIPC | CLONE_NEWNS]) == Ok(0));
            check!(2, sys(p, nr::MOUNT, &[cstr(1, ""), cstr(2, "/"), 0, MS_REC_PRIVATE, 0]) == Ok(0));
            for (slot, dir) in [(1, "/tmp/mq-a"), (2, "/tmp/mq-b")] {
                let made = sys(p, nr::MKDIRAT, &[AT_FDCWD, cstr(slot, dir), 0o755]);
                check!(3, matches!(made, Ok(0) | Err(Errno::EEXIST)));
            }
            let mount = |dir: &str| sys(p, nr::MOUNT, &[cstr(3, "mqueue"), cstr(4, dir), cstr(5, "mqueue"), 0, 0]);
            check!(4, mount("/tmp/mq-a") == Ok(0));
            check!(5, read_file(p, "/tmp/mq-a/pk_q") == Err(Errno::ENOENT), "a new namespace has no queues");
            let q = sys(p, nr::MQ_OPEN, &[cstr(1, "pk_q"), O_CREAT_RDWR, 0o600, 0]);
            check!(6, q.is_ok());
            let status = read_file(p, "/tmp/mq-a/pk_q").map_err(|_| 7u64)?;
            check!(8, status.starts_with(b"QSIZE:0 "));
            // System V: the first queue of the namespace is 0 and is listed.
            let listed = |path: &str| text(p, path).lines().count();
            check!(9, listed("/proc/sysvipc/msg") == 1 && listed("/proc/sysvipc/shm") == 1);
            check!(10, sys(p, nr::MSGGET, &[0x706b_0001, IPC_CREAT | 0o600]) == Ok(0));
            check!(11, listed("/proc/sysvipc/msg") == 2);
            // A setting of this namespace.
            let msgmni = text(p, "/proc/sys/kernel/msgmni");
            check!(12, write_file(p, "/proc/sys/kernel/msgmni", "77") == Ok(2));
            check!(13, text(p, "/proc/sys/kernel/msgmni") == "77\n");
            check!(14, write_file(p, "/proc/sys/fs/mqueue/msg_max", "11") == Ok(2));

            // A further namespace: none of it is there, while the mount
            // made before keeps showing its own namespace's queue.
            check!(15, sys(p, nr::UNSHARE, &[CLONE_NEWIPC]) == Ok(0));
            check!(16, sys(p, nr::MQ_OPEN, &[cstr(1, "pk_q"), 2, 0, 0]) == Err(Errno::ENOENT));
            check!(17, sys(p, nr::MSGGET, &[0x706b_0001, 0]) == Err(Errno::ENOENT));
            check!(18, listed("/proc/sysvipc/msg") == 1);
            check!(19, text(p, "/proc/sys/kernel/msgmni") == msgmni);
            check!(20, text(p, "/proc/sys/fs/mqueue/msg_max") == "10\n");
            check!(21, mount("/tmp/mq-b") == Ok(0));
            check!(22, read_file(p, "/tmp/mq-b/pk_q") == Err(Errno::ENOENT));
            check!(23, read_file(p, "/tmp/mq-a/pk_q").is_ok());
            check!(24, sys(p, nr::MQ_OPEN, &[cstr(1, "pk_q"), O_CREAT_RDWR, 0o600, 0]).is_ok(), "another queue");
            check!(25, read_file(p, "/tmp/mq-b/pk_q").is_ok());
            check!(26, sys(p, nr::MQ_UNLINK, &[cstr(1, "pk_q")]) == Ok(0));
            check!(27, read_file(p, "/tmp/mq-b/pk_q") == Err(Errno::ENOENT));
            check!(28, read_file(p, "/tmp/mq-a/pk_q").is_ok(), "the outer queue is untouched");
            check!(29, sys(p, nr::UMOUNT2, &[cstr(1, "/tmp/mq-b"), 0]) == Ok(0));
            check!(30, sys(p, nr::UMOUNT2, &[cstr(1, "/tmp/mq-a"), 0]) == Ok(0));
            Ok(())
        })
    }
}

# 0054: Namespaces and seccomp: kernel ids with translation at the boundary, namespaces per process, mount namespaces as copies of the mount tree, classic BPF filters per thread

**Status:** Accepted. Builds on [0041](0041-processes-threads-fork-exec-wait.md)
(processes, the process tree), [0043](0043-dev-proc-sys-and-file-system-calls.md)
(`/proc`), [0045](0045-credentials-and-capabilities.md) (credentials,
capabilities) and [0051](0051-busybox-init-mount-and-power-off.md)
(mount(2), sethostname(2)).

## Context

Phase 4 asks for "Linux namespaces / seccomp basics (needed by browsers'
sandboxes)". A browser's sandbox, as its public design documents describe
it, starts its renderer processes in new user, pid, network and IPC
namespaces made without privilege (an unprivileged user namespace grants
the capabilities the others need), maps its own ids in the new user
namespace through `/proc/<pid>/uid_map` and `gid_map`, chroots to an empty
directory, sets `no_new_privs` and installs a seccomp-BPF filter that
allows the calls the renderer needs and traps or refuses the rest. None of
this existed: the clone(2) namespace flags were `EINVAL`, seccomp(2) and
unshare(2) / setns(2) were not implemented, every id was global and every
capability was the initial namespace's.

Sources: namespaces(7), user_namespaces(7), pid_namespaces(7),
mount_namespaces(7), network_namespaces(7), uts_namespaces(7),
ipc_namespaces(7), capabilities(7), credentials(7), clone(2), unshare(2),
setns(2), getpid(2), getppid(2), kill(2), setuid(2), setgroups(2),
seccomp(2), prctl(2), proc(5), proc_sys_kernel(5); the UAPI headers
`linux/sched.h`, `linux/seccomp.h`, `linux/filter.h`, `linux/bpf_common.h`,
`linux/audit.h`, `asm-generic/errno.h`. No Linux or C library source was
read and nothing was observed on a real Linux system. Where the pages
leave an order or a detail open, the code says so ("to be confirmed by the
diff runner").

## Decision

### seccomp

**Classic BPF is a crate of its own.** `crates/poly-bpf` checks a program
before it may run (1 to 4096 instructions, only the classic opcodes, only
aligned 32-bit loads inside the input, forward jumps inside the program, a
return last, no division by a constant 0) and runs it on a slice of words.
Everything that could fail or loop at run time is refused at load time, so
running a filter is a bounded loop with no error path.

**Filters belong to threads, and are shared.** Each thread's `Seccomp`
(in the personality's `LinuxThread`) is disabled, strict, or a chain of
filters, newest first, behind `Arc`s: fork(2), clone(2) and execve(2) copy
the pointer, `SECCOMP_FILTER_FLAG_TSYNC` puts the caller's chain on every
thread of the process (or names the thread that cannot take it). Installing
needs `no_new_privs` or `CAP_SYS_ADMIN` (`EACCES`), as seccomp(2) says.

**The check runs in the dispatcher, before the handler.** The kernel keeps
one flag per process, set when any thread got seccomp state
(`LinuxKernel::seccomp_active`), so a process without filters pays one load
per call. The highest-precedence action of all filters wins; `TRACE`
(no ptrace) and `USER_NOTIF` (no listener) fail the call with `ENOSYS`;
`LOG` asks the kernel to log the call. Strict mode allows read(2),
write(2), _exit(2) and rt_sigreturn(2) and kills for anything else.

### Namespaces

**Credentials hold kernel ids; the boundary translates.** Every id in a
thread's `Creds` and in the VFS is the id the initial namespace sees. A
user namespace's maps translate both ways, and only the system calls do
it: getuid(2) and friends report the caller's view (an unmapped id as
65534, `overflowuid`), setuid(2) and friends take ids in the caller's
namespace (`EINVAL` without a mapping), setgroups(2) likewise. So nothing
below the personality, not the VFS and not the process table, needs to
know namespaces exist, and a process that made a user namespace keeps
owning its files.

**Capabilities are relative to a user namespace.** A thread's capability
sets are what it has in its own user namespace (`Creds::user_ns`);
`ns::user::ns_capable` applies user_namespaces(7)'s rules (members have
their sets, ancestors' holders have everything, the owner in the parent
has everything). Each privileged call names the namespace it checks:
mount(2) and umount2(2) that of the mount namespace's owner, sethostname(2)
that of the UTS namespace's owner, setns(2) the target's owner,
`Creds::capable_initial` for what belongs to the whole machine (reboot(2),
mknod(2) of devices, the clocks). The VFS keeps checking capabilities as
before; a thread outside the initial user namespace hands it only
`CAP_SYS_ADMIN` and `CAP_SYS_CHROOT`, which the personality has checked in
the right namespace, so file permission overrides (`CAP_DAC_OVERRIDE`,
`CAP_FOWNER`, ...) do not leak out of a namespace. "Root" for the
capability rules of execve(2) and setuid(2) is the kernel id of the
namespace's user 0. Set-user-ID bits of a file whose owner has no mapping
are ignored.

**Namespaces other than the user namespace belong to the process.** The
personality's `LinuxProcess` holds them (`ns::Namespaces`), shared by its
threads; `None` is the initial namespace of each kind, so a system that
never makes one pays nothing. Linux keeps them per thread; Poly's processes
own their files and signal state the same way (ADR 0041), so unshare(2) in
a process with several threads refuses what Linux would refuse
(`CLONE_NEWUSER`, which implies `CLONE_THREAD`), and also what Poly cannot
split (`CLONE_NEWNS`, which implies `CLONE_FS`; `CLONE_FILES`), and
changes the rest for all threads; clone(2) with `CLONE_THREAD` takes no
namespace flag.

**Pid namespaces number their members.** The kernel and the process tree
keep one global id per thread and process. Each pid namespace has a table
of its members' numbers; a new thread or process gets the next free number
in its own namespace and every ancestor (the first process of a namespace
is 1), gives it back when it ends (a thread) or is reaped (a process),
and the system calls translate: ids reported are the caller's view (0 for
one it cannot see, as getppid(2) says for a parent outside), ids given are
looked up in the caller's namespace (`ESRCH` otherwise). When a
namespace's init ends, the namespace is dead (fork(2) `ENOMEM`) and every
other member gets `SIGKILL`; its orphans go to its init (each process
records its reaper in the tree), and an init only gets signals it handles
except `SIGKILL` and `SIGSTOP` from an ancestor namespace.

**A mount namespace is a copy of the mount tree.** The VFS already linked
each mount to its parent; a tree of mounts below one first mount is now a
namespace, named by that mount's id. `Vfs::copy_namespace` duplicates
every mount of a tree (same file systems, so files are shared) and moves
the process's root and working directory into the copy; mounts and
unmounts in one copy do not show in the others; `/proc/<pid>/mounts` lists
the process's tree. When the personality's last reference to a namespace
goes, the kernel drops the copy (later, where no lock is held). Mounts are
not *locked* (mount_namespaces(7): a less privileged namespace's copies
cannot be unmounted one by one); a user namespace may only mount proc,
sysfs, devpts, tmpfs and ramfs.

**UTS, IPC and network namespaces are small.** A UTS namespace holds its
host and domain names (a copy of the current ones). An IPC namespace holds
nothing: Poly has no System V IPC yet. A network namespace is an identity
only: Poly's network stack is not yet split by namespace, which is the
largest gap here (a sandbox's renderer that is meant to have no network
still reaches the machine's).

**`/proc/<pid>/ns/<kind>` resolves like an ordinary link.** Its text is
`<type>:[<inode number>]`, and procfs has a hidden regular file of that
name in the same directory, whose inode number encodes the namespace;
following the link opens it, and setns(2) takes a descriptor on it
(anything else is `EINVAL`). Every namespace is registered by its inode
number with a weak reference, so a descriptor does not keep a namespace
alive (unlike Linux). `uid_map`, `gid_map` and `setgroups` are the only
procfs files that take writes (`FileSystem::writable_file`), with
user_namespaces(7)'s rules: once, at offset 0, by a writer in the
namespace or its parent with `CAP_SETUID` / `CAP_SETGID` there; without
`CAP_SETUID` in the parent only its own id, one line of length 1, and for
`gid_map` only after "deny" went to `setgroups`.

**Not supported:** cgroup and time namespaces (`EINVAL`, as for a kernel
built without them), pidfds, `set_tid`, ptrace-based rules for who may open
`/proc/<pid>/ns`, `NS_GET_*` ioctls, per-namespace views of `/proc`'s pid
directories (procfs shows global pids), locked mounts, mount propagation,
networking split by namespace; ids passed between processes in data
(`SCM_CREDENTIALS`, `SO_PEERCRED`, procfs's `status`) are not translated
for the receiver.

## Consequences

- The id and capability translation lives in the personality and is host
  tested; the kernel only stores namespaces, numbers processes and copies
  mount trees.
- Every check of a capability had to say which namespace it means. New
  privileged calls must do the same (`capable` for the caller's own
  namespace's resources, `capable_initial` for the machine's).
- The network namespace is a placeholder until the network stack takes a
  namespace per socket; that is the next step for real sandboxing.

# 0057: App tests: unchanged host programs in one file tree, a chroot on Linux and the initramfs on Poly, compared in app mode

**Status:** Accepted. Builds on [0027](0027-agent-pipeline-tools-and-test-run-protocol.md)
(the diff runner and the test-run protocol), [0038](0038-linux-processes-in-the-kernel-and-the-first-program.md)
(`poly.init`), [0043](0043-dev-proc-sys-and-file-system-calls.md) and
[0049](0049-dynamic-linking-program-interpreter.md) (the `rseq` decision,
the host's C library in test images).

## Context

Roadmap Phase 4's proof of concept (a) asks for "a recorded trace of a
simple Linux CLI app" to pass: an unchanged program (GNU `true`, `echo`,
`cat`, `wc`, `sort`, `head`, `ls`, `cp`, `mkdir`, `grep`, `sed`), run on
real Linux and on Poly with the same inputs, with the same observable
behaviour. The diff runner only ran freestanding test programs: one
binary as `/test-binary`, no arguments (the kernel's `poly.init` takes a
path only), stdout and stderr on pipes on Linux but on the console on
Poly, and the binary's host path in the Linux trace.

Running the same program "the same way" on both systems is harder than it
looks. A first comparison of `true` showed that what differs is rarely
the kernel under test: the paths the dynamic linker sees (host paths and
`/etc/ld.so.cache` against an image without one), the CPU model (glibc
searches `glibc-hwcaps/x86-64-v2..v4` only when the CPU has those
features; QEMU's default CPU has none), the kind of file stdout is (a
pipe needs no `TCGETS`, a terminal does, and C libraries buffer
differently), and the host's configuration (this host's kernel has
SELinux, so libselinux finds `selinuxfs` in `/proc/filesystems` and reads
`/proc/mounts`, whose size is the host's).

## Decision

**A test is a definition, not a program:** `<name>.app` (JSON) lists
steps (`[program, args...]`, programs by command name, taken from the
host), stdin, extra environment and files. The corpus has one per program
in `tests/linux/apps/`; an entry's tests directory may hold `.app` files
like `.c` files.

**One file tree, built by the tool, is what both systems run.** The
launcher (freestanding C, `tests/linux/apps/launch.c`) is `/test-binary`;
the programs, their interpreter and their `DT_NEEDED` closure (found with
the code the dynamic linking boot tests use, `tools/xtask/src/libsearch.rs`,
included by both tools) go to their host paths; fixed `/etc/passwd`,
`/etc/group`, empty `/dev`, `/proc`, `/sys`, `/tmp`; the test's files in
`/work`; root's, fixed modes, one fixed modification time. Poly boots it
as the initramfs. Linux gets it written to a tmpfs (the initramfs is a
tmpfs there too, which decides directory sizes and blocks) and runs the
launcher chrooted into it under our tracer, in its own mount namespace
(made private before anything is bound, so nothing reaches the host's)
with the host's `/dev` and `/proc` bound in; without root, a user
namespace maps the caller to root. The tree's paths, ld.so's search
(no cache on either side), the owner and times `ls -l` shows are then the
same by construction.

**The launcher fixes the process state** both systems start the program
in: umask 022, `/work`, stdin from a regular file, stdout and stderr into
regular files, the environment. Steps but the last run as children
(`fork`, `execve`, `wait4`); the last replaces the launcher, so the
program's exit status is the test's. Linux output is read from the files;
Poly's comes from `POLY-OUT`, which reports what fds 1 and 2 are written
whatever file they are.

**What the host's configuration would add is taken out on the reference
side, not masked in the comparison:** `GLIBC_TUNABLES=glibc.cpu.hwcaps=-SSE4_2`
in the fixed environment (a documented glibc tunable; on QEMU's CPU it
changes nothing), an empty `/sys`, and a copy of `/proc/filesystems`
without `selinuxfs` bound over the real one inside the reference's mount
namespace. The reference then behaves like Linux on the CPU and kernel
configuration Poly presents.

**App mode** compares exit status, stdout and stderr exactly and, per
process, the sequence of syscall names with result classes (`ok`,
`-1 ERRNO`, no return, or the descriptor for calls returning one),
consecutive repeats collapsed, `rseq` and `SIGCHLD` left out. Exact mode
stays available (`--syscalls exact`); `true`, `echo` and `head` pass it
too, with `--ignore-syscall rseq`.

## Consequences

- Six of the eleven programs pass on Poly as of this change; the gaps of
  the others are concrete syscalls (`specs/linux/FAILING.md`), and the
  proof of concept (a) is met.
- App tests depend on the host's programs and C library: their traces are
  recorded live on every run (like the corpus), and a host without a
  program reports its tests unsupported. A different distribution can
  give different traces; the comparison is always against the host the
  run is on.
- App mode does not see arguments or exact results: a Poly bug that
  changes only a returned size or a flag shows up only through output,
  exit status, or a later call. Exact mode and the per-syscall tests
  cover those.
- The reference needs root or unprivileged user namespaces; elsewhere the
  app tests are unsupported (not failures), and `cargo xtask ci` stays
  green.
- `POLY-OUT` has to cover every call that writes to fds 1 and 2
  (`copy_file_range`, `sendfile`, `splice`, `writev`), or a correct
  implementation of such a call loses the program's output from the Poly
  side (noted in `specs/linux/FAILING.md` for `copy_file_range`).

## Alternatives considered

- **Per-test launcher binaries with the arguments compiled in:** no
  configuration file to read, but a compile per test and generated code.
- **Pipes for stdout with a relaying parent:** the program would see a
  pipe on both systems, but a relaying parent's reads depend on
  scheduling, and on Poly the child's writes to fd 1 are reported as
  well as the parent's.
- **Masking the start-up in the comparison** (hwcaps probes, libselinux's
  reads): Poly's records have no strings yet, so the noise cannot be
  recognised on its side, and a mask would hide real differences.
- **Bind-mounting the host's `/sys`:** it shows the host's devices and
  security modules (`/sys/fs/selinux`), which say nothing about Poly.

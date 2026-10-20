# 0027: Agent pipeline tools: own ptrace tracer, per-test diff runner, a serial test-run protocol for Poly, held-out store outside the repository

**Status:** Accepted. The kernel side of the test-run protocol is implemented as described in [0038](0038-linux-processes-in-the-kernel-and-the-first-program.md).

## Context

Phase 3 asks for the first version of the agent pipeline from the project
brief (section 4): a syscall tracer on real Linux, a diff runner that runs
the same tests on real Linux and on Poly, a held-out test store outside the
repository and a logged hand-off between the spec agent and the
implementer agent (CLEAN_ROOM.md). The LLM agents themselves are run by the
owner (Claude Code or OpenCode on a home server); what the repository needs
are the deterministic tools around them, so every step that decides
something (what is held out, what was handed over, what passed) is
reproducible and checkable without a model.

Poly cannot run user programs yet. The parallel kernel work (user mode,
VFS, initramfs, ELF loader, first syscalls) will make it possible, so the
Poly side of the diff runner has to be written against an interface that
does not exist yet, and say clearly that it is not there.

## Decision

**One host tool, `tools/pipeline` (`cargo xtask pipeline <command>`).**
Like `tools/coverage`, a binary with no kernel code, using `serde`,
`serde_json` and `libc` (for ptrace). SHA-256, cpio and the strace parser
are small and written here. The workflow is in
[AGENT_PIPELINE.md](../AGENT_PIPELINE.md).

**Test corpus in `tests/linux/`, freestanding C.** Each program is one C
file with its own `_start` and raw `syscall` instructions
(`tests/linux/include/poly_test.h`), built static, non-PIE, without a C
library, stack protector, CET or build id by the host's `cc`. It makes
exactly the syscalls it asks for, so its trace is short and its first
calls are the ones Poly implements first. Output is one `ok <check>` /
`not ok <check> (got N)` line per check and a summary, with nothing that
varies between runs (no addresses, times, pids). Programs in
`tests/linux/libc/` link the host's static C library; their traces show a
real C library start-up, which orders the work queue. Builds are
reproducible and recorded in a manifest (compiler, flags, hashes). C over
Rust because a freestanding Rust program for Linux needs a target the
pinned toolchain does not install, and every Linux host has a C compiler.

**Our own ptrace tracer; strace only as a fallback.** `PTRACE_TRACEME`,
then `PTRACE_SYSCALL` stops, registers from `PTRACE_GETREGS`, memory from
`/proc/<pid>/mem`, forks and clones followed. The trace starts after the
test program's `execve` (Poly starts the program directly), the program
gets an empty environment, stdin on `/dev/null`, stdout and stderr on
pipes, and no address randomisation (raw traces of two runs are
identical). Arguments are decoded by a table of argument kinds (int, hex,
pointer, string, input buffer, output buffer, pid) for the calls programs
use most; names and numbers come from the coverage map. strace (`-f -q -X
raw`) is parsed into the same format where ptrace is not allowed; its
traces compare only with strace traces. Traces are JSON (`poly-trace/1`).

**Comparison: exact where the OS is deterministic, normalised elsewhere.**
Exit status (code or signal), stdout and stderr are compared byte for
byte. The syscall and signal sequence is compared after normalisation:
pointers become `<ptr>` (NULL stays), addresses returned by `mmap`/`brk`
become `<addr>`, pids `<pid>`, errors `-1 ENAME`; strings and buffers the
kernel reads are compared by content. The first divergence is reported
with context. Options: names only, off, syscalls to ignore, and a timing
envelope (target at most F times the reference, plus 50 ms), off by
default since Poly runs under QEMU's emulator. Linux against Linux must be
clean; `cargo xtask ci` checks that (`pipeline selfcheck`, under a second;
it skips itself without a C compiler or where ptrace is not permitted).
A diff run writes every trace, a per-test table and a JSON summary under
`target/pipeline/runs/`; it fails on any failure, or, given a baseline
summary, only on regressions (a test that passed and no longer does).

**The Poly test-run protocol.** This is the interface the kernel will
implement; the diff runner already speaks it.

- The test program is the only file of a cpio `newc` initramfs (plus
  empty `/dev` and `/tmp`), at `/test-binary`, passed with QEMU's
  `-initrd`. The tool writes the archive itself (reproducible: timestamps 0,
  owner root).
- Kernel command line: `poly.qemu_exit poly.init=/test-binary poly.test.capture`.
  `poly.init` names the first user program; `poly.test.capture` asks for
  the records below on the serial port.
- Records, each on its own serial line (kernel log lines may come in
  between):

  ```text
  POLY-INIT: exec /test-binary                     starting it
  POLY-INIT: unsupported <reason>                  cannot run it (e.g. not a static ELF)
  POLY-SYSCALL: pid=1 nr=1 args=0x1,0x401000,0xd,0x0,0x0,0x0 ret=13 t=81234 data1=68656c6c6f
  POLY-OUT: fd=1 hex=68656c6c6f0a                  bytes written to fd 1 or 2
  POLY-SIGNAL: pid=1 signal=11                     a signal delivered
  POLY-EXIT: pid=1 code=0 t=91234                  the program ended (or signal=N)
  ```

  Numbers are decimal or `0x` hex; `ret=?` means the call did not return;
  `t` (nanoseconds since the program started) is optional; `data<i>`
  optionally carries the bytes behind pointer argument `i` (without the
  terminating NUL for strings). An argument without data matches anything
  in the comparison, so the kernel can start without copying data out.
- After `POLY-EXIT` of the first process the kernel exits QEMU with its
  success status (33); the program's own status is in the record. Any
  other QEMU status after a `POLY-EXIT` is a kernel failure.
- A kernel that does not know `poly.init` boots as usual and never prints
  `POLY-INIT`. The diff runner then reports the test as **unsupported**
  (not failed) and does not boot again for the remaining tests. Today
  every test is unsupported this way.

**Held-out tests never enter the repository.** The spec agent writes its
tests outside the repository; `pipeline tests split` copies about 70%
into `specs/<os>/<component>/<symbol>.tests/` and the rest into the
held-out store (`$POLY_HELDOUT_DIR`, default `~/.poly-heldout`, refused if
it overlaps the repository), with a manifest of SHA-256 hashes of both
sides. The choice is deterministic (by a hash of entry and file name, not
content), holds out at least one and never all, and refuses identical
content on both sides.

**The hand-off log is the proof.** `pipeline handoff record` appends a row
to `specs/HANDOFF_LOG.md`: date (UTC), entry, the commit the spec and
visible tests are in (it refuses uncommitted files), both agent ids, and
every file with its SHA-256. It refuses when an agent would play both
roles for an entry, or when a file has a held-out test's content. `handoff
bundle` writes exactly those files, read from git at that commit, into the
implementer's input directory. `handoff verify` re-reads every logged file
from git and checks its hash, checks the roles again, and with the
held-out store at hand checks that nothing logged, nothing under `specs/`
and nothing the implementer received (`--received`) has a held-out test's
content, whatever its name.

**LLM roles are commands, not code.** `pipeline role` runs a configured
shell command per role (`POLY_SPEC_AGENT_CMD`, `POLY_IMPLEMENTER_CMD`,
or the `claude` / `opencode` presets) in the role's own directory with a
generated prompt, a fresh session each time, without `POLY_HELDOUT_DIR`
for the implementer, and with `POLY_ROLE` set. Commands that observe the
reference OS (`trace`, `diff`, `spec`, `tests`) refuse to run under
`POLY_ROLE=implementer`. A usage-limit message in the agent's output ends
the run with status 75 so a scheduler can pause and retry.

## Consequences

- The pipeline can be exercised today: corpus, tracer, Linux-against-Linux
  diffs, split, hand-off and verification all run and are tested; the Poly
  target reports "unsupported" with the boot log until the kernel speaks
  the protocol, and then needs no change on the tool side.
- The kernel work that makes `poly.init` real also has to produce the
  `POLY-*` records; syscall tracing in the kernel is a small addition to
  the dispatcher that already logs unimplemented calls.
- Comparisons are only as good as the argument table: calls outside it are
  compared as six raw values with address-like values masked, which can
  hide or invent differences. The table grows with the work queue.
- The oracle guard is a speed bump, not a sandbox: an implementer on a
  Linux machine can always observe Linux. Real separation is the
  implementer's container (no held-out store, no oracle VM, restricted
  tools), described in AGENT_PIPELINE.md.
- `pipeline selfcheck` in CI relies on ptrace of a child, which GitHub's
  runners allow; elsewhere it skips itself rather than fail.

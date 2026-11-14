# 0182: The macOS test oracle: C test programs of raw system calls, built on the owner's Mac for x86_64 and arm64, run there several times, results recorded as `macos.json` beside the tests

**Status:** Accepted. The first part of the Phase 9 item "macOS test oracle
(licensed Mac); extend the framework map with Objective-C methods and Swift
API". It is the macOS counterpart of the Linux corpus
([ADR 0027](0027-agent-pipeline-tools-and-test-run-protocol.md)) and the
Windows oracle ([ADR 0075](0075-windows-test-oracle.md),
[ADR 0097](0097-oracle-machine-over-ssh-and-wine-test-seeds.md)). The
framework map half of the item is not done here, so the item stays open.

## Context

Every macOS entry of the coverage map (`coverage/macos/syscall/{bsd,mach}.json`
from XNU's `syscalls.master` and trap table; the framework map of
[ADR 0157](0157-chrome-mac-import-map.md) and
[ADR 0163](0163-macos-map-credits-re-exports.md)) has waited for an oracle:
the XNU personality answers every call with `ENOSYS` or `KERN_FAILURE`,
"provisional results until spec sheets exist". The plan
([ADR 0151](0151-macos-plan-arm64-translator-and-aarch64-port.md)) has
Poly speak the x86-64 macOS system call convention first and arm64 later,
through a translator.

The only real macOS available is the owner's **MacBook Pro (Apple M1 Pro,
arm64), macOS 27.0 (build 26A428), Darwin 27.0.0**, with Xcode (Apple clang
21, macOS SDK 27.2), SIP enabled. A Claude Code session runs there with a
worktree of this repository. What that Mac allowed, checked before deciding:

- **Rosetta 2 is not installed.** An x86_64 program fails to start with
  `EBADARCH` ("Bad CPU type in executable"). Installing it
  (`softwareupdate --install-rosetta`) changes the system and needs the
  owner.
- **Static executables are refused on arm64.** A `-static -nostdlib`
  arm64 program is killed at `exec` (SIGKILL); macOS on Apple silicon runs
  only programs that go through `dyld`. A dynamically linked program that
  calls nothing in libSystem works.
- **`dtruss` needs SIP disabled** (DTrace's `syscall` provider), and
  `fs_usage` needs root. Neither is available to an unattended session.
- **The pipeline tool did not build on macOS**: its tracer is `ptrace(2)`
  with Linux's options and namespaces.

## Decision

### Test programs: one C file, raw system calls, nothing imported

A macOS test is one C file: `tests/macos/<name>.c` for the corpus and
`specs/macos/<component>/<symbol>.tests/<name>.c` for an entry (held-out
tests in the held-out store), named in lower case. It includes
`tests/macos/include/poly_mtest.h` and defines `int test_main(void)`.

The header makes BSD system calls and Mach traps itself, for both
architectures, with the conventions open-source XNU documents: x86_64
`syscall` with the class in the top byte of `rax` (2 BSD, 1 Mach) and
arguments in `rdi rsi rdx r10 r8 r9`; arm64 `svc #0x80` with the number in
`x16`, negative for Mach traps, arguments in `x0`..`x5`. A BSD call that
fails sets the carry flag and returns the errno; the header turns that into
`sysret {ret, ret2, err}`, so a test sees both result registers (some calls
use the second) and the error exactly as the kernel gives them. Numbers,
flags, errno values and the layouts of `struct stat64` and `struct dirent`
come from the macOS SDK headers and each group names its header. Output is
the Linux corpus's format (`ok`, `not ok ... (got N)`, `value <name> <v>`,
`summary`), written with the raw `write` call; the program ends with the
raw `exit` call: status 0 when every check passed, 1 otherwise.

Why raw calls: the specs are of the kernel. A libSystem wrapper hides the
carry flag, picks a different call (`open` may be `open_nocancel`,
`fstat` is `fstat64`), retries, or caches (`getpid` was cached for years).
And it is what Poly's XNU personality has to implement first; its own
libSystem comes later in Phase 9.

Why dynamically linked nonetheless: arm64 macOS starts nothing else. The
program links `/usr/lib/libSystem.B.dylib` and has `main`, but calls
nothing in it. **`mac build` refuses an executable that loads another
library or imports any symbol** (checked with the repository's own Mach-O
reader, `poly-macho`, ADR 0153): a test that calls `getpid()` from libc is
an error, `imports _getpid`. The header defines `memset` and `memcpy` for
the code the compiler generates. So a test's only contact with libSystem is
that dyld loads it before `main`.

Never print addresses, port names, pids, times or anything else that
changes between runs; a value may differ between the architectures (the
page size does) because each architecture's output is recorded on its own.
Each run starts in a fresh, empty scratch directory (its current
directory), with an empty environment and stdin at end of file.

### `cargo xtask pipeline mac build | run | show`

```sh
cargo xtask pipeline mac build [--tests-dir DIR]... [--tests a,b]
cargo xtask pipeline mac run   [--tests-dir DIR]... [--tests a,b] [--runs 3] [--timeout 10] [--record]
cargo xtask pipeline mac show  <results.json | macos.json> [--tests a,b]
```

- **`mac build`** compiles every test (default: `tests/macos/` and every
  `specs/macos/*/*.tests/`) with `xcrun --sdk macosx clang` for
  **x86_64** and **arm64** (`-O1 -g0 -std=c11 -Wall -Wextra -Werror
  -fno-builtin -fno-stack-protector -Wl,-S`, the repository prefix mapped
  to `.`, `ZERO_AR_DATE=1`) into `target/pipeline/macos/bin/<arch>/<test>`,
  checks each executable as above, and writes `manifest.json`
  (`poly-macbuild/1`: compiler, SDK, flags, per test the source's and each
  executable's SHA-256). Two builds give the same bytes (tested), so a hash
  names an executable. The linker signs arm64 executables ad hoc, which
  macOS requires; nothing else is signed.
- **`mac run`** builds, then runs every test on both architectures
  `--runs` times (3 by default) with a timeout, and writes
  `target/pipeline/macos/results/macos-<time>.json` (`poly-macrun/1`): the
  Mac (`sw_vers` product version and build, `uname -r`, `-v`, `-m`; never
  the host name), compiler, SDK, and per test and architecture the status
  (`ran`, `timeout`, `error`, or **`unavailable`** when the Mac cannot run
  the architecture: `EBADARCH`, no Rosetta 2), exit code or signal, stdout
  and stderr (text, plus base64 when not UTF-8), the executable's hash, the
  number of runs and **`stable`**: whether every run gave the same status,
  exit code, signal and bytes. It exits 1 if a test that could run timed
  out, did not start or was not stable.
- **`mac run --record`** merges the results into **`macos.json` in each
  test's own directory** (same format), per test and architecture: a
  stable result replaces the recorded one; an unavailable architecture or
  an unstable result keeps what was there and is listed as not recorded; a
  test whose source changed loses its old results. So a recording made
  without Rosetta has arm64 results only, and a later `mac run --record`
  on a Mac with Rosetta adds x86_64 without touching them. Visible tests
  record into `specs/.../<symbol>.tests/macos.json`, held-out tests into
  their directory in the store.
- **`mac show`** prints a result file's observations.

`mac run` and `mac show` observe real macOS, so they refuse
`POLY_ROLE=implementer` like `trace` and `win diff`; `mac build` observes
nothing. Building and running need macOS: on any other host they stop with
"`mac build` needs macOS (Apple clang, the macOS SDK and the kernel under
test); this host is linux". Parsing, summarizing, merging and the
executable check are host-tested on every host; the build and run test runs
only on a Mac.

### The pipeline tool builds on macOS

`tools/pipeline` (which also holds `spec new`, `tests split` and
`handoff`, needed on the Mac for the held-out store) now builds there: the
ptrace tracer is compiled on Linux only, and on other hosts
`ptrace_other.rs` keeps its interface and reports "the system call tracer
needs Linux (ptrace and namespaces); this host is not Linux" for `trace`,
the diff runner's reference runs and app tests. `identity.rs` uses
`getuid`/`geteuid` where `getresuid` does not exist, and the three host
tests that build Linux ELF programs are ignored off Linux with that reason.
Linux behaviour is unchanged.

### The held-out store and the split

Nothing new: `tests split` takes a macOS entry's generated tests (only
`.c` files with valid names, checked like the Windows entries'), keeps
about 30% in the held-out store and commits the rest. On the Mac the store
is `POLY_HELDOUT_DIR=~/poly-heldout`, so a macOS entry's held-out tests are
in `~/poly-heldout/macos/syscall/<component>/<symbol>/` with their
`MANIFEST.json`; after the split, `mac run --record` once with the visible
directory and once with the held-out directory records each side's
`macos.json` where its tests are.

### Poly's side: not yet

Poly cannot run a Mach-O program yet (the roadmap's "Mach-O loading and
our own `dyld`" comes after this item), so there is no `mac poly` or `mac
diff`. When there is, it compares Poly's run of the same x86_64 executable
(by hash) with the recorded x86_64 result, as `win diff` does with
`windows.json`; a test's stdout and exit code are already the whole
comparison. Until Rosetta 2 is installed on the oracle, recordings have
arm64 results only; the x86_64 ones are the reference Poly needs first.

## Considered and not chosen

- **Calling libSystem in tests** (`write()`, `getpid()`): the observed
  behaviour would be libSystem's as much as the kernel's, and Poly's
  first target is the kernel interface.
- **Static x86_64 executables** (which Intel Macs ran and Poly could load
  without a `dyld`): they cannot be observed here (no Rosetta, and arm64
  refuses static programs), and whether Rosetta runs them is unknown. An
  option for `mac poly` later, built from the same sources.
- **Tracing with `dtruss`**: needs SIP off, which is the owner's decision
  and lowers the Mac's security; the tests' own output is enough for the
  kernel-level specs. A boundary tracer for unchanged programs (the macOS
  counterpart of ADR 0090) is future work.

## Consequences

- Spec agents on the Mac can write macOS tests, run them on real macOS and
  record the results beside the tests; implementers get the spec sheet,
  the visible tests and their `macos.json`.
- New: `tools/pipeline/src/mac.rs` (`poly-macbuild/1`, `poly-macrun/1`),
  `tools/pipeline/src/ptrace_other.rs`, `tests/macos/include/poly_mtest.h`,
  `tests/macos/hello.c` with its `macos.json`.
- Open for the owner: installing Rosetta 2 on the oracle Mac, so that the
  x86_64 results (the convention Poly speaks) can be recorded with `mac run
  --record` again.

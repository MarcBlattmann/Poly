# 0075: The Windows test oracle: test programs built here in Rust against `ntdll` by name, an observation file beside the executable, a PowerShell runner on the owner's PC, bundles that travel as text, results compared with Poly

**Status:** Accepted. The Windows counterpart of [ADR 0027](0027-agent-pipeline-tools-and-test-run-protocol.md)
(the Linux test corpus, diff runner and test-run protocol). Builds on
[ADR 0071](0071-our-ntdll-a-real-pe-dll-built-from-rust.md) (our `ntdll.dll`,
built from Rust for `x86_64-pc-windows-msvc` with `-Zbuild-std` and `rust-lld`,
offline; the kernel starts every NT process in it) and
[ADR 0073](0073-windows-file-system-layout.md) (an executable outside `C:` is
named `Z:\...`). The workflow is in [AGENT_PIPELINE.md](../AGENT_PIPELINE.md#windows-test-oracle).
Refined by [ADR 0097](0097-oracle-machine-over-ssh-and-wine-test-seeds.md):
the oracle is now a dedicated machine reached over SSH (`win remote`),
references are kept one file per test, and Wine's tests enter the coverage
map as name-only seeds.

## Context

Phase 5 asks for a "Windows test oracle VM, Windows API tracer, Wine
conformance tests imported as spec seeds". On the Linux side a spec agent
writes small freestanding C programs, the tools run them on real Linux (this
machine) under a ptrace tracer and on Poly in QEMU, and compare outputs, exit
statuses and system calls. Every NT entry in the coverage map (ADR 0063 to
0073, `ntdll` in 0071) is waiting for that oracle before it can turn green.

The oracle machine is the owner's Windows 11 Home PC (build 26200, x64), not
a VM: Defender real-time protection on, Smart App Control off, git, Python
and Node installed, **no compilers**, no WSL or Hyper-V. A Claude Code
session runs there and can run commands; nothing may be installed without
the owner's OK. The repository is public, so the PC can clone and pull it
without credentials, but it cannot push. Nothing may be downloaded from
Microsoft to build test programs here (no SDK, no import libraries).

What this container has: the pinned Rust toolchain with `rust-src` and
`rust-lld` (what ADR 0071 builds `ntdll` with), and clang 18 with
`lld-link`. The home server that runs the pipeline loop has the Rust
toolchain but no clang (its packages are those of `setup.sh`).

## Decision

### Test programs: Rust, `no_std`, imports declared by name with `raw-dylib`

A Windows test is **one Rust file**, `#![no_std]` + `#![no_main]`:
`tests/windows/<name>.rs` for the corpus, and, for a spec entry,
`specs/windows/<component>/<symbol>.tests/<name>.rs` (visible) and the
held-out store (held out), exactly where the Linux tests go. It declares the
functions it tests itself, from Microsoft Learn's prototypes:

```rust
#[link(name = "ntdll", kind = "raw-dylib")]
unsafe extern "system" {
    fn NtQuerySystemTime(system_time: *mut i64) -> NTSTATUS;
}
poly_wtest::main!(test);
fn test(t: &mut Test) { /* t.check(..), t.expect(..), t.expect_status(..), t.value(..) */ }
```

`raw-dylib` makes the compiler write the import entries itself, so no
import library and no SDK are needed. `cargo xtask pipeline win build`
generates one Cargo package with a binary per test (in
`target/pipeline/windows/build`), builds it for `x86_64-pc-windows-msvc`
with `-Zbuild-std=core,compiler_builtins` from `rust-src` (offline, under
`RUSTC_BOOTSTRAP=1`, as ADR 0071), links with `rust-lld -flavor link`
(`/NODEFAULTLIB /ENTRY:mainCRTStartup /SUBSYSTEM:CONSOLE /Brepro
/DEBUG:NONE`, paths remapped), and writes `target/pipeline/windows/bin/<name>.exe`
and `manifest.json` (`poly-winbuild/1`: compiler, revision, source and
executable hashes, imports). Every executable is checked with `poly-pe`: an
x64 PE32+ console executable that **imports by name only, only from
`ntdll.dll` and `kernel32.dll`**, with no delay-load imports. Executables
are 6 to 8 KB; two builds give the same bytes (tested), so a hash names an
executable. The first build compiles `core` once (about 30 s, 120 MB under
`target/pipeline/windows/cargo`); later builds take seconds.

**The support library** `tests/windows/support` (crate `poly-wtest`, the
counterpart of `tests/linux/include/poly_test.h`, excluded from the host
workspace) has the entry point (`main!`), the checks, the output, a panic
handler, `__chkstk` (written from Learn's x64 stack-probe contract) and
`_fltused`, the constants and layouts tests need with the SDK header each
comes from (`ntstatus.h`, `winnt.h`, `winternl.h`, `wdm.h`, `ntdef.h`),
`UNICODE_STRING`, `OBJECT_ATTRIBUTES`, `IO_STATUS_BLOCK`, a compile-time
`utf16!` and `path_beside_image(name)` (the NT path of a file in the test's
own directory). Everything a test does is a documented call; no code is
copied from anywhere.

Why Rust and not C: C needs `clang --target=x86_64-pc-windows-msvc`,
`lld-link` and import libraries (`llvm-dlltool` and `.def` files we would
write) and declarations of every structure without the SDK headers. clang
is here but not on the home server, whose runner builds and runs the spec
agent's tests; the Rust toolchain with `rust-src` is on every machine that
builds Poly, and the `ntdll` build already depends on exactly this
cross-compilation. A spec agent writes `unsafe extern "system"` declarations
from Learn instead of `#include <winternl.h>`; that is the only difference.

### Output: an observation file beside the executable

A test writes its observations, in the Linux corpus's format (`ok <check>`,
`not ok <check> (got N)`, `value <name> <v>`, `summary <p> passed, <f>
failed`), into **`poly-test.out` in the executable's own directory**, with
`NtCreateFile`/`NtWriteFile`, and ends with `NtTerminateProcess(NtCurrentProcess(),
code)`: 0 when every check passed, 1 if one failed, 2 if the file could not
be made, 3 after a panic. The directory comes from
`RTL_USER_PROCESS_PARAMETERS.ImagePathName` (`winternl.h`: `TEB` + 0x60 is
the PEB, `PEB` + 0x20 the parameters, + 0x60 the image path).

Why not standard output: the standard handles are `kernel32`
(`GetStdHandle`) and console matters that Poly does not have yet (the
`kernelbase`/`kernel32` and console milestones), and the only native way to
them is an undocumented field of `RTL_USER_PROCESS_PARAMETERS`. The file
needs four documented `ntdll` calls that Poly implements (ADR 0067), so the
same executable runs on both systems today. A test may still write to its
standard handles once it imports `kernel32`; both runners capture stdout and
stderr too, and the comparison includes them.

### The Windows runner: `tools/pipeline/windows/poly-oracle.ps1`

Windows PowerShell 5.1 (as shipped), nothing installed, nothing changed
outside its work directory and the result file; started with a
process-scoped `-ExecutionPolicy Bypass`. It:

1. unpacks a bundle (below) into `%TEMP%\poly-oracle\bin` (emptied first),
   checking every file's SHA-256 and the bundle's end line, or takes a
   directory of `.exe` files;
2. sets its error mode to `SEM_FAILCRITICALERRORS | SEM_NOGPFAULTERRORBOX |
   SEM_NOOPENFILEERRORBOX`, which its children inherit, so a test that fails
   to load or crashes ends with its status instead of showing a dialog;
3. runs each test `-Runs` times (2 by default), each time as a copy of
   itself in a fresh scratch directory (also its current directory), with
   stdin at end of file, stdout and stderr captured as bytes, and a timeout
   (`-TimeoutSeconds`, 10) after which it is killed; the scratch directory
   is removed afterwards;
4. writes one JSON file, `poly-winrun/1`: the system (version, build with
   UBR, architecture, edition), the bundle id, and per test the
   executable's hash, `status` (`ran`, `timeout`, `error`), the 32-bit exit
   code, the observation file, stdout and stderr (base64, plus the
   observation file as text for reading), the time, and `stable`: whether
   every run gave the same status, exit code and bytes. `-PrintJson` also
   prints it, to copy it out of the session.

### How test programs get there: a text bundle, through the session

`cargo xtask pipeline win bundle` writes **one text file**,
`target/pipeline/windows/bundle-<id>.txt` (`poly-winbundle/1`):

```text
poly-winbundle/1
manifest <base64 of manifest.json>
file <name>.exe <sha256> <base64>
end <files>
```

`<id>` is the first 12 hex digits of the text's SHA-256. Line based, so a
bundle larger than one message is cut into parts at line ends
(`--part-size`, 45 000 bytes by default: `bundle-<id>.txt.part1of3`, ...)
that the runner concatenates in the order given; `win unbundle` checks a
bundle here the same way. The three demo tests make a 30 KB bundle.

**The transport is the Claude session on the PC**: the coordinator sends
the bundle's text in a message, the session saves it as a file and runs the
runner, and prints the result file, which the coordinator saves here. No
binary enters git, nothing needs credentials on the PC, and the hashes
catch any damage on the way. Considered and not chosen:

- *Executables committed to `main`*: binaries in the history of every clone,
  for files that are reproducible from the sources anyway.
- *An orphan branch* (`windows-oracle` with the latest bundle): `git clone`
  fetches every branch, so every clone downloads it, and its history grows
  with each push unless it is force-pushed; pushing needs credentials that
  only the runner has.
- *Building on the PC*: no compiler there, and installing one needs the
  owner's OK.
- *GitHub release assets*: no git objects, no credentials to download
  (public repository), any size. This is the way for the unattended loop
  later: the runner (which has `gh`) runs `gh release upload windows-oracle
  bundle-<id>.txt --clobber`, the PC fetches
  `https://github.com/<owner>/<repo>/releases/download/windows-oracle/bundle-<id>.txt`
  with `Invoke-WebRequest` and runs it unchanged (the runner copies each
  executable before it runs it; `Unblock-File` would clear a mark of the
  web if one were set). Not set up now: there is no unattended Windows step yet, and it
  needs a release the owner agrees to.

### Poly's side

`cargo xtask pipeline win poly` boots each executable as `/test-binary` of
the test initramfs (ADR 0027) with `poly.qemu_exit poly.init=/test-binary
poly.test.capture poly.test.stdout=/poly-test.out`. The kernel starts it
through our `ntdll` (ADR 0071), which binds its imports; its image path is
`Z:\test-binary` (ADR 0073), so its observation file is
`\??\Z:\poly-test.out`, the initramfs's `/poly-test.out`, which the kernel
reports at the end as `POLY-OUT: fd=1` records (an NT process has no
standard handles on Poly yet, so nothing else is reported as fd 1). The
results go into the same `poly-winrun/1` format (`target/pipeline/runs/<time>-windows-poly/poly.json`).
`win poly --build-kernel` builds the kernel with `cargo xtask build`, which
embeds `ntdll`; the pipeline's kernel build now does that for every target
(a bare `cargo build` in `kernel/` makes a kernel without `ntdll`).

**`POLY-EXIT` carries the full `NTSTATUS` of an NT program** (a change to
ADR 0027's protocol, compatible: an extra field): `POLY-EXIT: pid=2 code=57
ntstatus=0xc0000139`. A process's `ExitStatus` in the kernel keeps 8 bits,
like a Linux exit status; `run_init` asks the NT personality to remember the
status of the `poly.init` program (`nt::watch_exit_status`): the one
`NtTerminateProcess` gave, else its last thread's. Without the field (an
older kernel) the comparison uses the low 8 bits.

### The comparison: `pipeline win diff`

`win diff --reference <Windows results.json>` runs the tests on Poly (or
takes `--target <file>`) and compares per test: the exit code (32 bits, or
the bits both sides know), the observation file, stdout and stderr, byte
for byte; the first differing line is shown. Verdicts: **pass**, **fail**,
**unsupported** (Poly cannot run it: `POLY-INIT: unsupported`), **missing**
(no target result) and **error**: the reference timed out or did not start,
is not `stable` (a test must be deterministic, the Windows counterpart of
`selfcheck`), or the two runs are of different executables (hashes). It
writes `report.txt` and `summary.json` (`poly-windiff/1`) under
`target/pipeline/runs/<time>-windows-diff/` and fails on fail, missing or
error (`--strict`: also unsupported). `win show <results.json>` prints a
result file's observations. `win diff` and `win show` read observations of
real Windows, so they refuse to run under `POLY_ROLE=implementer` like
`diff` and `trace`; `win build`, `win bundle` and `win poly` do not observe
Windows.

There is no system-call or API trace on the Windows side yet: the
comparison is outputs and exit codes (the "app mode" of the Linux side
without its events).

### Recorded references

Linux is re-observed on every run because it is this machine; Windows is
not always reachable. A Windows result file is therefore kept: for the
corpus as `tests/windows/oracle/windows-<build>.json` once the PC has run
it (observations of our own programs, the same kind of fact as a spec
sheet), for a spec entry's visible tests next to them in
`<symbol>.tests/windows.json`, and for its held-out tests in the held-out
store. Then CI and the loop can compare Poly with the recorded reference
without the PC (`win diff --reference <recorded> --build-kernel`, a
`win polycheck` analogous to `polycheck`), and the PC is needed only when
tests change or a new Windows build is to be checked. Teaching `tests split`
and `handoff` about `.rs` tests and the result files, and the spec agent's
role prompt about the Windows workflow, is the next step; neither the loop
nor CI runs Windows tests yet.

### The demo

`tests/windows/`: `nt_query_system_time` (status, after 2020 and before
2100, a second query not earlier), `nt_virtual_memory`
(`NtAllocateVirtualMemory` of 0x1001 bytes: a 64 KiB-aligned base, the size
rounded to 0x2000, zeroed, reads back; `NtFreeVirtualMemory` with
`MEM_RELEASE` and size 0 reports base and size; a second release fails, its
status printed as a value) and `nt_file_rw` (`FILE_CREATE` then a
collision, a synchronous write and read at explicit offsets,
`STATUS_END_OF_FILE` at the end, `FILE_DELETE_ON_CLOSE`, then
`STATUS_OBJECT_NAME_NOT_FOUND`). On Poly, `nt_virtual_memory` and
`nt_file_rw` pass every check; `nt_query_system_time` ends with
`STATUS_ENTRYPOINT_NOT_FOUND` (0xc0000139) because our `ntdll` does not
export `NtQuerySystemTime` yet, which is the comparison doing its job. The
Windows run is the first step of using the oracle (commands in
AGENT_PIPELINE.md).

## Not built: a Windows API tracer

The comparison sees outputs and exit codes, not the calls a program makes.
For real applications (the 7-Zip milestone) a trace is needed: which
`ntdll`/`kernel32` exports an unchanged program calls, in what order, with
what results. Clean-room constraints: only the boundary between the
program and the system may be observed, never the inside of Windows
binaries. The way that fits:

- **An import-address-table logger of our own**: a DLL built here like the
  tests (Rust, `raw-dylib`), loaded into the program at start (started
  suspended with `CreateProcessW`, the DLL loaded through a documented
  route such as a remote `LoadLibraryW` thread), which replaces the
  program's own IAT entries with logging thunks that call the original and
  record name, arguments (by a per-function signature table like
  `tools/pipeline/src/syscalls.rs`) and result into a file. It observes only
  what the program asks of the system, like `ltrace`. Not ETW kernel
  system-call tracing: its events name kernel addresses that only Windows'
  internal symbols explain.
- What it needs: the logger and launcher (both cross-built here), a trace
  format next to `poly-trace/1`, the owner's OK to run an injecting program
  on the PC (Defender may flag it; an exclusion for the work directory is
  the owner's decision), and on Poly the same thunks (or our `ntdll`
  logging its own exports) for the other side of the comparison.

## Wine's conformance tests as spec seeds

Wine's tests (LGPL) are an oracle **for the spec agent only** (CLEAN_ROOM.md):
it may read them to learn which behaviours are worth observing, then writes
its own tests; nothing of them enters the repository, not even as
"imported" seeds. What the roadmap item means here is this list of where a
spec agent looks first for an entry, by Wine's file names: `dlls/ntdll/tests/`
`file.c`, `virtual.c`, `time.c`, `info.c`, `om.c`, `reg.c`, `sync.c`,
`exception.c`, `pipe.c`, `directory.c`, `path.c`, `rtl.c`, `rtlstr.c`,
`env.c`, `change.c`; `dlls/kernel32/tests/` `file.c`, `virtual.c`,
`time.c`, `process.c`, `thread.c`, `sync.c`, `heap.c`, `console.c`,
`path.c`, `environ.c`, `module.c`, `loader.c`, `pipe.c`.

## Consequences

- Spec agents can observe Windows: write `.rs` tests, `win build`,
  `win bundle`, have the PC run them, `win show` the results; the
  comparison with Poly is `win diff`. The tests run on Poly today.
- Two new formats (`poly-winbuild/1`, `poly-winbundle/1`) and two shared
  ones (`poly-winrun/1` from both runners, `poly-windiff/1`), parsed and
  tested in `tools/pipeline/src/windows.rs` (including a result file shaped
  like Windows PowerShell's `ConvertTo-Json` output with a byte-order mark).
- The PowerShell runner cannot be run in this container; its first run on
  the PC is its test. It is plain PowerShell 5.1 and ASCII.
- `-Zbuild-std` is now used by two builds (`ntdll` and the Windows tests),
  each with its own target directory (about 120 MB each).
- The kernel change is small: the exit-status watch in `nt/mod.rs` and the
  `ntstatus=` field.
- Defender may quarantine a test executable (small, unsigned, native API
  only); the runner then reports that test as `error` with the reason.
  An exclusion is the owner's decision, never the runner's.
- The roadmap item stays open: the oracle exists; the API tracer and the
  pipeline's Windows workflow (split, hand-off, recorded references in CI,
  the loop) do not yet.

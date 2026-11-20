# Agent pipeline

How coverage entries go from "missing" to "green" with AI agents, and
which parts are deterministic tools in this repository. The rules it
enforces are in [CLEAN_ROOM.md](../CLEAN_ROOM.md); the design decisions,
including the interface the kernel implements for test runs, are in
[ADR 0027](adr/0027-agent-pipeline-tools-and-test-run-protocol.md).

## The workflow

```
  tracer ──► spec agent ──► hand-off ──► implementer ──► diff runner ──┬─► green: PR, CI, merge
 (real OS)   (oracle)       (logged)     (no oracle)     (visible +    │
                                                          held-out)    └─► red: the visible tests'
                                                                            diff back to the implementer
```

1. **Trace.** Run programs on the real OS (a Linux VM, or this machine,
   which is real Linux) and record every syscall: `pipeline trace`.
2. **Claim.** Pick the next entry in roadmap order and claim it:
   `cargo xtask coverage claim <id>... --agent <your-agent-id>` (many ids in
   one call, or `--from-file FILE`; `release` and `set-status` take them the same way).
3. **Spec agent** (may observe the real OS): writes the spec sheet
   (`pipeline spec new` makes the skeleton) and test programs outside the
   repository; `pipeline tests split` keeps ~30% held out, outside the
   repository, and commits ~70% as visible tests.
4. **Hand-off.** `pipeline handoff record` logs who handed what (file
   hashes, git revision) to whom, in a file of its own under `specs/handoffs/`
   (ADR 0213; the legacy `specs/HANDOFF_LOG.md` is still read);
   `pipeline handoff bundle` writes exactly those files for the implementer.
5. **Implementer** (never sees the real OS or held-out tests):
   implements against the spec and visible tests, runs `cargo xtask ci`.
6. **Diff runner.** `pipeline diff --target poly` runs the corpus and the
   entry's tests (visible and held-out) on the real OS and on Poly and
   compares them per test. Green: open a PR; it merges when CI is green.
   Red: the visible tests' diff goes back to the implementer (held-out
   tests only as a count).
7. Any app-level failure later becomes a new entry.

Each role runs in its own agent session, started by hand. With
`POLY_ROLE=implementer` in its environment the pipeline refuses `trace`,
`diff`, `spec` and `tests`, and `cargo xtask ci` skips its Linux
self-check and the Poly check (both run the reference OS).

## Deterministic tools vs. LLM roles

| Step | Who | Command |
|---|---|---|
| Build the test corpus (`tests/linux/`) | tool | `cargo xtask pipeline build` |
| Trace on the real OS | tool | `cargo xtask pipeline trace <test\|path>... [--all] [--print]` |
| Observe behaviour, write spec and tests | **spec agent (LLM)** | its own session |
| Spec skeleton from `specs/TEMPLATE.yaml` | tool | `pipeline spec new <id> --agent A` |
| Visible / held-out split, manifest | tool | `pipeline tests split <id> <dir> --spec-agent A` |
| Hand-off log, bundle, verification | tool | `pipeline handoff record\|bundle\|verify` |
| Implement | **implementer (LLM)** | its own session, with the hand-off bundle |
| Compare real OS and Poly | tool | `pipeline diff [--target linux\|poly]` |
| Merge decision | CI + held-out result | PR, never a direct push |
| Windows tests: build, bundle, run on Poly, compare | tool | `pipeline win build\|bundle\|poly\|diff\|show` (runner on the PC: `poly-oracle.ps1`) |
| Windows tests on the oracle machine over SSH, record, compare | tool | `pipeline win remote --host homeserver2 [--record] [--diff]` (from a container host: `tools/pipeline/windows/poly-remote.sh`) |
| Where Wine's conformance tests exercise an entry (spec agents) | tool | `cargo xtask coverage show <id>`; refreshed by `cargo xtask coverage seed-wine` |
| macOS tests: build for x86_64 and arm64, run on the Mac, record `macos.json` | tool | `pipeline mac build\|run [--record]\|show` (on the oracle Mac only; ADR 0182) |

All of these are `cargo xtask pipeline <command>`; the full option list is
at the top of `tools/pipeline/src/main.rs`. Outputs go under
`target/pipeline/` (`bin/`, `traces/`, `runs/<time>-<target>/` with
`summary.json` and `report.txt`, `runs/latest`).

### Test corpus

`tests/linux/*.c` are freestanding static programs (no C library, raw
syscalls; `include/poly_test.h`), each printing `ok <check>` /
`not ok <check> (got N)` lines and a summary, exit status 0 when all pass:
`hello`, `stderr`, `exit42`, `brk`, `mmap` (with `munmap`/`mprotect`),
`arch_prctl`, `uname`, `clock`, `getrandom`, `efault` (also `EBADF` and
`ENOSYS`), `segv` (killed by a signal). `tests/linux/libc/hello.c` links
the host's static C library (`libc-hello`) for the start-up trace; it is
skipped where there is none. Toolchain: the host's `cc` (gcc 13 here;
clang works too, set `CC`), flags in `tools/pipeline/src/corpus.rs`;
`target/pipeline/bin/manifest.json` records the compiler version and the
hash of every source and binary. Two builds with the same compiler give
the same bytes (tested). `tests/linux/apps/*.app` are app tests, below.

### App tests

An app test runs an unchanged, dynamically linked program of this
machine (GNU coreutils, grep, sed, ...) with fixed arguments, stdin,
environment and files, on real Linux and on Poly, and compares the runs
like a corpus test ([ADR 0057](adr/0057-app-tests-host-programs-in-one-tree.md);
code in `tools/pipeline/src/apps.rs`). A test is a JSON file:

```json
{
  "description": "cp(1) copies a file; cat(1) shows the copy",
  "steps": [["cp", "src.txt", "dst.txt"], ["cat", "dst.txt"]],
  "stdin": "optional",
  "env": ["OPTIONAL=extra"],
  "files": [{"path": "src.txt", "text": "copied content\n"},
            {"path": "sub", "dir": true}, {"path": "run.sh", "text": "...", "mode": "755"},
            {"path": "link", "symlink": "src.txt"}]
}
```

- **One file tree for both systems.** The tool builds it from the
  definition: the launcher `tests/linux/apps/launch.c` (freestanding, like
  the corpus) as `/test-binary`, its configuration and stdin in
  `/poly-app/`, the programs in `/bin` (from the host's `/usr/bin`,
  `/bin`, `/usr/sbin`, `/sbin`), their interpreter and the shared libraries
  they need at their host paths (found as the dynamic linking boot tests
  find them, `tools/xtask/src/libsearch.rs`, shared by both tools),
  `/etc/passwd` and `/etc/group` with only root, empty `/dev`, `/proc`,
  `/sys` and `/tmp`, and the test's files under `/work`. Everything is
  root's, with fixed modes and the modification time 2023-11-14 22:13:20
  UTC (so `ls -l` shows the year on every system). The run directory
  keeps a listing with hashes (`trees/<test>.txt`).
- **Poly** boots the tree as its initramfs (`poly.init=/test-binary`);
  **Linux** gets it written to a tmpfs (`/dev/shm`, or
  `$POLY_APP_TREE_DIR`; an initramfs is a tmpfs on Linux too) and the
  tracer starts the launcher chrooted into it, in a mount namespace of its
  own with the host's `/dev` and `/proc` bound in. That needs root, or
  unprivileged user namespaces (the caller is then mapped to root); where
  neither is allowed, app tests are reported **unsupported**. Only the
  ptrace tracer runs them.
- **The launcher** (the kernel's `poly.init` takes no arguments) reads
  `/poly-app/launch`, sets umask 022, changes to `/work`, takes stdin from
  `/poly-app/stdin` and sends stdout and stderr into the regular files
  `/poly-app/stdout` and `/poly-app/stderr` (the same kind of file on both
  systems, so programs buffer and probe the same way), runs every step
  but the last as a child it waits for and the last one with `execve` in
  its place. On Linux the run's output is what those files hold; on Poly
  too: the kernel reports their contents (`poly.test.stdout=`,
  `poly.test.stderr=`) in `POLY-OUT` records when the program has ended.
- **Fixed environment:** `PATH=/bin HOME=/work LANG=C TZ=UTC
  GLIBC_TUNABLES=glibc.cpu.hwcaps=-SSE4_2`, plus the test's `env`. The
  tunable hides SSE4.2 from glibc, so its dynamic linker does not search
  `glibc-hwcaps/x86-64-v2..v4` directories on a newer host CPU: the search
  is the one QEMU's default CPU (under which Poly runs) gets.
- **Host configuration that is not the program's:** the reference's
  `/sys` stays empty, and a copy of the host's `/proc/filesystems`
  without `selinuxfs` is bound over the real one: Poly is a Linux without
  SELinux, and libselinux (linked by ls, cp, mkdir, sed) probes for it.
- **App mode** (`--syscalls app`, the default for app tests; `--syscalls`
  overrides it): exit status, stdout and stderr byte for byte, and per
  process (each process's own order, so a parent waiting for its child
  does not depend on scheduling) the sequence of syscall names with result
  classes: `ok`, `-1 ERRNO`, `?` (no return), or the descriptor for calls
  that return one; consecutive events of the same name and class are
  collapsed into one (a read loop over a host file of another size, a
  signal action set for many signals); `rseq` (`ENOSYS` on Poly until
  ADR 0111, ADR 0043/0049) and `SIGCHLD` are left out. Arguments are shown
  in the report but not compared: an unchanged program's arguments are its
  own, and where they follow from the system (a read size after a short
  read of `/proc/self/maps`) they are not about Poly.

Linux against Linux is clean for every app test, also with four runs in
parallel under load. The corpus has one per program (`app-true`,
`app-echo`, `app-cat`, `app-wc`, `app-sort`, `app-head`, `app-ls`,
`app-cp`, `app-mkdir`, `app-grep`, `app-sed`); each program's entry
`linux/api/apps/<program>` has its spec sheet and visible cases in
`specs/linux/apps/` and held-out cases in the store, and what fails on
Poly is in `specs/linux/FAILING.md`. To trace one: `cargo xtask pipeline
trace app-ls --print`. A QEMU that dies by a signal (an emulator crash
seen under load) is booted once more.

**Chromium's scenario tests** (`specs/linux/apps/chromium.tests/`, entry
`linux/api/apps/chromium`) use the same JSON format but run programs of
Chromium's own root disk (Debian packages pinned in
`tools/xtask/chromium.lock`, ADR 0130) rather than the host's, so the
diff runner does not run them: `cargo xtask chromium apptest
[--target linux|poly|both|linux-twice] [--tests-dir DIR]...` runs each
test's steps as a shell script on the reference (the Debian root entered
with `pivot_root`) and on Poly (one boot of the disk), and compares stdout
and exit status; stderr and system calls are not compared (Chromium is
about 15 processes and 250 threads, its log lines carry pids and times).
What Chromium needs call by call is in `docs/chromium-linux-gaps.md`.

### Diff runner

```sh
cargo xtask pipeline diff                       # real Linux against real Linux (must be clean)
cargo xtask pipeline diff --target poly         # real Linux against Poly in QEMU
cargo xtask pipeline diff --tests hello,brk --syscalls names --timing-envelope 50
cargo xtask pipeline diff --target poly --baseline target/pipeline/runs/<old>/summary.json
cargo xtask pipeline diff --target poly --tests-dir <entry>.tests --tests-dir <held-out dir of the entry>
cargo xtask pipeline diff --target poly --jobs 4 # four tests at a time (default 1)
cargo xtask pipeline selfcheck                  # what `cargo xtask ci` runs: Linux against Linux
cargo xtask pipeline polycheck                  # and Poly against Linux (poly-pass.txt, green entries)
cargo xtask pipeline polyrun cat-n --tests-dir specs/linux/apps/cat.tests --print
                                                # Poly only, no reference (implementers may run it)
```

`--tests-dir` adds an entry's test programs (visible or held-out, `.c`
files in the corpus format, or `.app` app tests) to the run.

Per test it compares the exit status, stdout and stderr exactly and the
syscall sequence after masking addresses, pids and times; the first
divergence is printed. Arguments are typed by the call's signature
(`tools/pipeline/src/syscalls.rs`), and for multiplexed calls by their
option or command too: `prctl(PR_GET_CHILD_SUBREAPER, &x)` compares its
address as `<ptr>` while `prctl(PR_SET_CHILD_SUBREAPER, 1)` compares the
1; likewise `fcntl` (lock and owner commands), `futex` (timeout or
`val2`), `arch_prctl` and `waitid` (`P_PIDFD`). It exits non-zero on any failure, or with
`--baseline` only on regressions; `--strict` also fails on unsupported
tests. A reference run that times out is run once more; a test whose
reference times out both times is an error, whatever the target did. A
rerun that finished is compared as usual and named on the report's last
lines (`the reference run timed out and was run again`, `summary.json`
`reference_timeouts`): the test can hang and wants fixing
([ADR 0085](adr/0085-reference-timeouts-are-hangs-rerun-once-kill-the-tree.md)).
On a timeout the runner kills the program's whole process tree, also
children that started a session of their own. `--tracer strace` uses
strace instead of our ptrace tracer.

A test can weaken the syscall comparison for itself with a line
`poly-diff: syscalls=off` (or `names`) in its source, usually in the
header comment with the reason: tests with several processes or threads
(their calls interleave by scheduling), and tests whose calls depend on
timing or read data carrying pids. Their exit status and output are
still compared exactly, and a directive never strengthens the mode the
run was given.

**The same environment on both sides**
([ADR 0081](adr/0081-test-runs-as-the-reference-user.md);
`tools/pipeline/src/identity.rs`). A test program starts with an empty
environment, only fds 0-2 open, in `/`, with umask 022, whatever the
tool's caller had, and as the
user, groups and supplementary groups of whoever runs the tool: root in
a container, a normal user on GitHub's CI. The diff runner records that identity (`summary.json`
`identity`, the report's last line: `test programs ran as uid 1000 gid
1000 groups 1000 umask 0022 on both sides`) and boots Poly with it
(`poly.test.uid=1000 poly.test.gid=1000 poly.test.groups=1000
poly.test.umask=0022`); the kernel's test init gives the program those
credentials (what root makes of itself with setgroups, setresgid and
setresuid, so a user other than root has no capabilities, as on Linux)
and umask before it runs. Root stays root. App tests run as root on both
sides (in the reference's jail, or mapped to root in a user namespace).

A test that only makes sense as root (mount, set*id to other users,
capabilities, root passing a permission check) says so with a line
`poly-diff: needs=root` in its header comment. Where the reference is not
root, `pipeline diff --target poly` does not compare it: it runs it on
Poly as root alone and passes it if it passes its own checks (exit 0),
`self_checked` in the summary, with the reason in the report. Linux
against Linux compares it as usual. Every other test must pass as any
user.

The ptrace tracer follows forks and clones, including children that
leave the test's process group, and keeps job control intact: a tracee
stopped by SIGSTOP stays stopped and its parent sees it (`WUNTRACED`).
The program starts with default signal actions and an empty signal mask.

**Poly:** the tool builds a one-program initramfs per test, boots
`kernel/target/x86_64-unknown-none/debug/poly-kernel` (built if missing,
`--build-kernel` to rebuild, or `--kernel`) with
`poly.qemu_exit poly.init=/test-binary poly.test.capture` and the
reference's identity (above), and turns the
kernel's `POLY-*` records (ADR 0027, kernel side in ADR 0038) into a trace;
about half a second per test. QEMU runs on KVM when `/dev/kvm` can be opened
and on its emulator (TCG) otherwise; `POLY_QEMU_ACCEL=tcg|kvm|auto` chooses (ADR 0077).
Records carry no `data<i>` fields yet, so
pointer arguments are compared as "anything". A kernel that cannot run a
test's file says `POLY-INIT: unsupported <reason>`, and the test is
reported **unsupported**. QEMU itself sometimes crashes (SIGSEGV) while
booting under TCG on a loaded host; a guest cannot cause that, so the boot
is retried twice and a crash that persists is an **error** of that test
alone (the logs of the crashed boots are kept as
`<test>.serial.crash-<n>.log`). Processes and threads are numbered in
creation order on both sides (on Poly by thread id, `tid=`, which the
kernel hands out in that order), so the program is always task 0; on Poly
it runs as pid 2, the child of a kernel init (ADR 0064), as under the
tracer it is the tracer's child. The kernel writes a `POLY-ENTER` record
when `clone`, `fork`, `vfork` or `clone3` is entered, so the call stands
before the child's calls as in a ptrace trace, and takes the program's
output from the files it started with on fds 1 and 2, wherever they are
duplicated (an app test's output files are reported at the end). `--jobs N` runs N tests at a time (the
reference run and the Poly boot of a test stay together; the report keeps
the corpus order).

**Keeping green entries green.** `pipeline polycheck` (in `cargo xtask ci`)
runs, on Poly against Linux, the corpus and the **visible** spec tests of
every coverage entry whose status is `green` and that has a
`specs/<os>/<component>/<symbol>.tests/` directory, as many at a time as
the machine has cores (at most four). It fails if a test listed in
`tests/linux/poly-pass.txt` does not pass, or if any visible test of a
green entry does not pass (`green entry <id>: visible test <t> fails`);
corpus tests that newly pass are named, to be added to the list. The green
entries come from the coverage map itself, so marking an entry green
(`coverage set-status <id> green ...`) is what puts its tests into CI; there
is no second list. The green entries' tests were verified as root, most of
them before `needs=root` existed: where the reference is not root
(GitHub's CI), one that differs is checked on Poly as root alone like a
`needs=root` test (`pipeline diff --root-fallback <tests>` does the same
by hand), and polycheck names it. A test that failed its own checks on
the reference needs root without saying so; one that passed them while
Poly differed is a real difference of Poly for users other than root,
printed as a warning (today `a_linkat_dirfd`: `linkat(fd, "", ...,
AT_EMPTY_PATH)` of a file the caller opened works on Linux 6.18 without
`CAP_DAC_READ_SEARCH`, Poly says `ENOENT`). Held-out tests are not in the repository and not on
GitHub, so CI cannot run them: they are checked before an entry is
marked green (`pipeline diff --target poly` with the entry's visible and
held-out directories). Today the 12 corpus programs
pass (`libc-hello` differed at `rseq` until Poly implemented it, ADR 0111); all 11 corpus app tests pass (app mode); and 139
green entries bring 245 visible tests. The spec tests take about a minute with four jobs
(the app tests run one at a time after them). (`write` was green
before its spec tests existed; its visible and held-out spec tests pass
since the runner's third pass.)

### Held-out store and hand-off log

```sh
export POLY_HELDOUT_DIR=~/poly/heldout          # default ~/.poly-heldout; never inside the repository
pipeline spec new linux/syscall/x86_64/brk --agent spec-1
pipeline tests split linux/syscall/x86_64/brk ~/poly/generated/brk --spec-agent spec-1
git commit ...                                   # spec sheet + visible tests
pipeline handoff record linux/syscall/x86_64/brk --spec-agent spec-1 --implementer impl-1
git add specs/handoffs && git commit ...         # one new file per record: PRs never conflict on it
pipeline handoff bundle linux/syscall/x86_64/brk --out ~/poly/implementer/inbox/brk
pipeline handoff verify linux/syscall/x86_64/brk --received ~/poly/implementer/inbox/brk
pipeline handoff verify                          # every record; run before merging (seconds)
```

`handoff record` writes `specs/handoffs/<UTC timestamp>-<spec agent>-<implementer>-<entry hash>.md`
with the fields of a log row (date, entry, spec revision, both agents, every
file with its SHA-256). Do not add rows to `specs/HANDOFF_LOG.md` any more:
two pull requests appending to it conflict, two new files never do. `verify`
and `bundle` read the legacy rows and the files alike, and `verify` reads
every handed-over file from git in one `git cat-file --batch` process (a
`git show` per file took 80 s on a Mac and over 30 minutes through a
container's file system). Agent ids in a record are letters, digits, `.`,
`_` and `-`.

The held-out store holds `<os>/<layer>/<component>/<symbol>/` with the
held-out files and a `MANIFEST.json` (hashes of visible and held-out
files, spec agent, revision). `verify` proves from git that each logged
file had the logged content at the logged revision, that no agent had both
roles for an entry, and that no held-out content is in the log, under
`specs/`, or in what the implementer received, whatever the file is called.

## Windows test oracle

The real Windows is a dedicated Windows 11 machine, **`homeserver2`**,
reached over SSH ([ADR 0097](adr/0097-oracle-machine-over-ssh-and-wine-test-seeds.md));
nothing is installed there. Test programs are built here, run there by a
PowerShell script and on Poly in QEMU, and the results compared
([ADR 0075](adr/0075-windows-test-oracle.md); code in
`tools/pipeline/src/windows.rs` and `remote.rs`, runner
`tools/pipeline/windows/poly-oracle.ps1`). Never run test executables on
the owner's corporate PC (its endpoint protection refuses unsigned ones).

**A test** is one Rust file, `#![no_std]` + `#![no_main]`, in
`tests/windows/` (the corpus) or `specs/windows/<component>/<symbol>.tests/`,
named in lower case (`nt_file_rw.rs`). It declares what it calls from
Learn's prototypes with `#[link(name = "ntdll", kind = "raw-dylib")]` (or
`kernel32`, which Poly does not have yet), and uses the support crate
`poly-wtest` (`tests/windows/support`): `poly_wtest::main!(test)`, then
`t.check(name, cond, got)`, `t.expect(name, got, want)`,
`t.expect_status(name, status, want)`, `t.value(name, v)`, the `ntstatus.h` /
`winnt.h` / `winternl.h` constants, `ObjectAttributes`, `IoStatusBlock`,
`utf16!("...")` and `path_beside_image(name)` for a file in the test's own
directory (on Windows a fresh scratch directory per run). Observations go
into `poly-test.out` beside the executable, one line each, in the Linux
corpus's format; the exit code is 0 (all passed), 1 (a check failed), 2 (no
output file) or 3 (panic). Never print addresses, handles or times: every
test runs twice on Windows and must give the same bytes.

```sh
cargo xtask pipeline win build [--tests-dir DIR]... [--tests a,b]   # target/pipeline/windows/bin/*.exe, manifest.json
cargo xtask pipeline win bundle                     # target/pipeline/windows/bundle-<id>.txt (+ .partNofM over 45 KB)
cargo xtask pipeline win unbundle <bundle> --out DIR   # check a bundle (what the PC will do)
cargo xtask pipeline win poly [--build-kernel] [--jobs 3]   # run them on Poly; poly.json in the run directory
cargo xtask pipeline win show <results.json>        # the observations in a result file
cargo xtask pipeline win diff --reference <windows results.json> [--build-kernel] [--target <file.json>]
```

A test that imports `dwrite.dll` is booted with the DejaVu fonts in
`C:\Windows\Fonts` (ADR 0170): `win poly` puts the files of `$POLY_FONT_DIR` or
`target/fonts/dejavu-*` (`cargo xtask fonts fetch`) in its initramfs. A test
that creates fonts through gdi32 (it imports `CreateFontW` or one of its
relatives) gets them too when they are there, so a font asked for by name is
an outline font of the height asked, as on any Windows.

`win build` cross-compiles with the pinned toolchain (`-Zbuild-std` from
`rust-src`, `rust-lld`; the first build compiles `core`, about 30 s) and
refuses an executable that imports from a DLL other than `ntdll`,
`kernel32`, `user32`, `gdi32`, `dwmapi`, `imm32`, `shcore`, `dwrite`, `advapi32`, `userenv`, `version` and the DLLs of Chrome's first stage
(`shlwapi`, `shell32`, `comctl32`, `comdlg32`, `powrprof`, `wtsapi32`, `netapi32`, `iphlpapi`, `oleacc`, `uiautomationcore`, `ole32`, `oleaut32`,
`kernelbase`, and Chrome's network stage: `ws2_32`, `mswsock`, `winhttp`, `crypt32`, `secur32`, `dhcpcsvc`, and `ucrtbase` for the C runtime entries, and `winmm` for the wave output entries; the list is `ALLOWED_DLLS` in `tools/pipeline/src/windows.rs`) (the GUI entries' tests are still console
programs; the DirectWrite entries' tests import `dwrite.dll`; `advapi32` and `userenv` are for the security model; `msimg32` is for `AlphaBlend`), or delay-loaded. An import by ordinal (`raw-dylib` with `#[link_ordinal(n)]`) is allowed and kept in the manifest as `#n`: it is how comctl32's `SetWindowSubclass` and its relatives (410..413) are bound by their real users. `win diff` compares per test the 32-bit exit code, the
observation file, stdout and stderr; a reference that timed out, did not
start or was not the same on both runs is an **error**, a test Poly cannot
start is **unsupported**. `win show`, `win diff` and `win remote` read real
Windows' observations, so implementers may not run them.

**GUI tests and the interactive session.** By default the runner runs
the tests in the SSH session, which is session 0 with a non-interactive
window station: windows and messages work there, but no window is
visible, nothing is painted through windows (no `WM_PAINT`, no update
regions, `IsWindowVisible` 0) and no input arrives. Tests of painting,
focus and activation, the cursor or real input (`SendInput`) run with
`--runner-arg -Interactive`: `poly-oracle.ps1 -Interactive` registers a
one-time scheduled task for the same user with the interactive logon type
(`New-ScheduledTaskPrincipal -LogonType Interactive`, no password stored),
runs itself through it with the same parameters in that user's desktop
session (session 1 on homeserver2, where `server` stays logged on), waits,
prints the inner run's log and unregisters the task. It refuses when the
user has no interactive session or it is locked. Inside, the runner keeps
a 16x16 topmost window of its own (the "keeper", at the screen's
bottom-right corner) as the foreground window between tests: before each
run it brings the keeper to the front and parks the cursor at (1500, 600).
So every test starts the same way and is started by the foreground
process, which lets it take the foreground; without the keeper, activation
fell to whatever the user had open when a test's window closed (on
homeserver2 an elevated console, which made `SetCursorPos` fail and
`SendInput` go nowhere). The result file's
`system.session` is `interactive` (absent for the SSH session), and a
spec entry's `windows.json` keeps it, so a reference says where it was
recorded; the screen there is 1920x1080 at 96 DPI (session 0: 1024x768).
Poly reproduces the session a test was recorded in: `win build` keeps
each test's `system.session` from the `windows.json` beside its source
in the manifest, and `win poly` boots those tests with the kernel
argument `poly.win32k.session=interactive` (ADR 0136), so a test
directory has to hold its `windows.json` when it is built.
A test recorded on another display than 1920x1080 at 100 percent carries
a `session` of the form `<kind>/<width>x<height>/<scale percent>` in its
`windows.json` entry (`interactive-bottom/2560x1600/200`, the DPI tests of
spec-win-dpi, recorded with the oracle's display switched dynamically to
2560x1600 at 200 percent and put back afterwards); `win poly` then also
passes `poly.win32k.display=2560x1600 poly.win32k.scale=200`.
The windows of such tests appear on the oracle's real desktop, so tests
keep to a few rules (spec-win-gui-b's sheets list them): windows in the
top-left 1000x500 pixels, closed within seconds; the cursor parked away
from them first and put back at the end; keyboard input only while a
window of the test is in the foreground, clicks only onto a window of the
test, never Alt, the Windows key or other shell shortcuts. Only one
interactive run at a time may use the machine.

```sh
POLY_RUN=~/poly-dev/polyd tools/pipeline/windows/poly-remote.sh --host homeserver2     --tests-dir specs/windows/user32.dll/InvalidateRect.tests --runner-arg -Interactive [--record]
```

**Tokens: standard and elevated** (`poly-oracle.ps1 -Token standard|elevated`, ADR 0171,
`docs/windows/chrome-installer-plan.md`). The oracle's SSH logon of an administrator has the
full token with no UAC split (`TokenElevationType` 1, no linked token); Windows' own programs
of a desktop user have **two** tokens of the same logon. `-Token` runs the tests through a
one-time task (as `-Interactive`, session 1, the user must be logged on) with run level
Limited (`standard`: medium integrity, Administrators deny-only, 5 privileges, `TokenElevationType` 3,
`TokenElevation` 0, `TokenLinkedToken` the full token) or Highest (`elevated`: the full token, high
integrity, 24 privileges, `TokenElevationType` 2, linked token the filtered one), so no consent
prompt is shown and no security setting changes. The result file's `system.token` says which;
a test's own `token` in `windows.json` overrides it (a directory can hold both), and `win poly`
boots Poly with `poly.win.token=standard|elevated` for it (nothing for a test without a token:
the pipeline's old default, the elevated administrator). A consent prompt that a test provokes (`ShellExecute` runas from a `standard` caller) is
**never answered** on the oracle: Windows ends it itself after about two minutes and the call returns as
declined (`ERROR_CANCELLED`), so those tests take about 140 s per run (give them `-TestTimeouts name=400`).
On Poly a `poly.win.token=standard` test run has nobody to click: the consent dialog must answer No at once.
Per-test `token` and `session` in `windows.json` rows are kept by `tests split` (`RunResult`).
Never click through or approve a real consent prompt on the oracle, and never change UAC settings there.

**Tests that need a sound card, or none.** The oracle has a sound card. `win build` notes in the manifest what a test
needs (`sound`): a test that imports a `winmm.dll` sound function (`waveOut*`, `waveIn*`, `midi*`, `mixer*`,
`PlaySound*`) gets `card`, and `win poly` (and `win polycheck`) boots Poly with a sound card whose output goes
nowhere (`-audiodev none`; virtio-sound, else Intel HDA). A test whose first line says `NO SOUND DEVICE` was recorded
with the oracle's card turned off; it gets `none` and boots without one. Every other test boots as before, without
a card.

**Tests with resources and dialogs** (menus, dialogs, string tables, bitmaps, icons,
accelerators; `specs/windows/user32.dll/LoadMenuW.tests` and its relatives). A test that
needs the resources of its own executable carries them as data: the resource directory
(the PE format of Learn's "PE Format"; the templates in the formats of "Resource File
Formats") is written in the test's source as a `core::arch::global_asm!` block in the
sections `.rsrc$01` (directories) and `.rsrc$02` (data, each blob with an image-relative
`.rva` entry), so the test stays one `.rs` file and `win build` needs no resource
compiler (the section is kept by the linker as it is). These tests run in the SSH session
(session 0): windows, menus, modal dialogs and message boxes work there without being
visible, input never arrives, so a test ends a modal loop itself (`EndDialog` from
`WM_INITDIALOG`, a posted message, or a window timer that calls `EndMenu` or finds the
`#32770` window of a message box with `EnumThreadWindows`). Common controls are those of
version 5.82 (no manifest); `comdlg32` is in `ALLOWED_DLLS` and the common dialogs are
ended by a hook that posts IDCANCEL.

**C++ tests** ([ADR 0098](adr/0098-cpp-windows-tests-and-the-cpp-exception-spec.md)):
a test may also be one `.cpp` file, for what Rust cannot do (C++
exceptions through `msvcrt.dll`). `win build` compiles it with `clang++
--target=x86_64-pc-windows-msvc -ffreestanding -nostdlib -fcxx-exceptions`
and links it with `lld-link` against import libraries made by
`llvm-dlltool` from `tests/windows/cxx/{kernel32,msvcrt}.def`; it includes
`tests/windows/cxx/poly_wtest.h` (the same observation file and lines as
`poly-wtest`, `POLY_TEST_MAIN(fn)`) and may import from `msvcrt.dll` too.
A line `// poly-cxx: handler=__CxxFrameHandler magic=0x19930520
ehflags=0 cflags=-fasync-exceptions` rewrites clang's exception tables the
way older compilers write them (see `tools/pipeline/src/wincxx.rs`).

**Tests built by Microsoft's compiler** (ADR 0199, spec-vcrt-1): what Visual C++ writes
(`__CxxFrameHandler4`'s compressed tables, RTTI records, the layouts of `msvcp140`'s classes) cannot
come from clang, and the container has no Microsoft compiler. A `.cpp` test with the line
`// poly-cxx: compiler=msvc [cl=/EHa,/O2]` (`cl=` takes `/EHsc /EHa /EHs /O1 /O2 /Od /GR /GR- /MD`; `/MD` with `/Od` makes the program call the members of the DLL's classes through their imports, as an application built with the DLL runtime does, which the stream and locale tests need) is built on the oracle machine (Visual Studio Build
Tools in `C:\BuildTools`, the Visual C++ Redistributable installed in System32) by
`tools/pipeline/windows/msvc-build.ps1` (`cl /EHsc /GS- /Zl /O1`, `link /NODEFAULTLIB
/ENTRY:mainCRTStartup /Brepro`, import libraries made from `tests/windows/cxx/*.def` and, for
`msvcp140`, from the DLL's own export table; `poly_wtest.h` takes `terminate`/`set_terminate` from
`<eh.h>` and defines its own `??_7type_info@@6B@`), and the resulting `<name>.exe` is checked in
beside `<name>.cpp` (a test artefact of about 10 KB; the pipeline's "no binary file" review rule
has this one exception, by owner decision 2026-11-15). `win build` takes that executable as it is
(it must import only from `ntdll`, `kernel32`, `msvcrt`, `vcruntime140`, `vcruntime140_1`,
`ucrtbase`, `msvcp140`, `msvcp140_1`); `tests split` keeps a `.exe` with its `.cpp`, visible or held out
together; `win remote`, `win diff`, `win poly` and `polycheck` need nothing else, so the same file
runs on Windows and on Poly. A symbol that is a Visual C++ decorated name (`?_Xbad_alloc@std@@YAXXZ`)
has file names with its `?` written `%3F` (any of `<>:"|?*%` is written `%XX`;
`EntryId::file_symbol`), the entry id stays the export name.

**Splitting a Windows entry's tests**: `pipeline tests split <entry> <dir>
--spec-agent A --windows-results <results.json from the PC>` accepts only
`.rs`/`.cpp` test files, checks that every test ran on Windows with the
same result on every run, and writes the visible tests' results to
`<symbol>.tests/windows.json` and the held-out tests' to the store, beside
the tests (`win diff --reference <symbol>.tests/windows.json` compares Poly
with them without the PC). The hand-off works as for Linux entries.

**Test names are unique** (`coverage check`, run by `ci coverage` and `ci lint`): `win build` builds the tests of every entry it is given, and the corpus, as one Cargo package with one binary per file, and results are looked up by name, so two files with the same name (`uc_basic` of two entries ended a CI run once) cannot live together. The check lists every name used by two entries among the visible tests and the corpus (an error); where the held-out store is present it also warns about held-out names shared with another entry (`--strict-heldout` makes those errors), reading only the manifests. Prefix a test's name with something of its entry.

**A test waits for its child before it reads the child's exit code** (`coverage check` warns): `GetExitCodeProcess` answers 259 (`STILL_ACTIVE`) until the process has ended, so a read that comes before any wait in its function (`WaitForSingleObject`, `wait_exit`, `.finish()`, ...) depends on timing and fails on a loaded CI runner. The check lists such reads (`tools/coverage/src/childwait.rs`); a read on purpose (a suspended or running process answering 259) goes into `tools/coverage/childwait-baseline.txt` as `path:function` once someone has looked at it. The check does not look at the length of the wait: a bound of 5 or 10 s for a child is short under CI load (`finish()` of the copied child helper waits 10 s, then ends the child and waits 5 s); give a wait for a child 30 s or more.

**Keeping green Windows entries green** ([ADR 0099](adr/0099-cpp-exception-handling-in-msvcrt.md)):
`cargo xtask pipeline win polycheck` (in `cargo xtask ci pipeline`) builds
the visible tests of every green Windows entry that has a `windows.json`,
runs them on Poly and compares them with those recorded results; every one
must pass. It skips itself without QEMU, without clang++/lld-link/
llvm-dlltool for C++ tests, and for implementer agents.

**One round on the oracle machine, over SSH** (ADR 0097). On the owner's PC,
where builds run in the container and SSH on the host, from the repository
or worktree root in Git Bash:

```sh
POLY_RUN=~/poly-dev/polyd tools/pipeline/windows/poly-remote.sh --host homeserver2 \
    [--tests a,b] [--tests-dir specs/windows/<component>/<symbol>.tests] [--record] [--diff]
```

That runs `cargo xtask pipeline win remote prepare` in the container (build,
bundle, stage in `.oracle-remote/homeserver2/` with a generated
`transfer.sh`), `transfer.sh` on the host (scp the bundle and the runner to
`poly-oracle-work/remote/<bundle id>/` on the machine, run them, scp
`results.json` back, remove the remote directory) and `win remote finish` in
the container: it checks the results against the bundle and keeps them as
`target/pipeline/windows/results/windows-<id>.json`; `--record` writes the
references (`tests/windows/oracle/<test>.windows.json` for the corpus,
`windows.json` beside a spec entry's tests); `--diff` runs the same
executables on Poly and compares (`--build-kernel`, `--jobs N`, `--strict`).
Where `cargo` and `ssh` are on one machine, `cargo xtask pipeline win remote
--host homeserver2 [...]` does all three. `--runner-arg` passes arguments to
the runner (`--runner-arg -Runs --runner-arg 3`). A test Defender takes away
is reported as `error`; stop and tell the owner (an exclusion is their call).

**One round through a session on a Windows PC** (the older way, ADR 0075:
the coordinator relays; the session there uses Git Bash). Here:

```sh
cargo xtask pipeline win build && cargo xtask pipeline win bundle
# -> bundle <id>: N tests, B bytes: target/pipeline/windows/bundle-<id>.txt
```

Send the session on the PC the bundle file's text, to be saved unchanged as
`~/poly-oracle/bundle-<id>.txt` (several parts: each as its own file), then
these commands:

```sh
git -C ~/poly pull --ff-only || git clone https://github.com/MarcBlattmann/Poly.git ~/poly
cd ~/poly-oracle
powershell.exe -NoProfile -ExecutionPolicy Bypass -File ../poly/tools/pipeline/windows/poly-oracle.ps1 \
    -Bundle bundle-<id>.txt -Out results-<id>.json
cat results-<id>.json
```

(`-Bundle a.part1of2,a.part2of2` for parts; `-Tests name1,name2` for a
subset; `-Runs`, `-TimeoutSeconds`.) The execution policy is bypassed for
that one process only; the runner works in `%TEMP%\poly-oracle` and writes
nothing else. Save the printed JSON here as
`target/pipeline/windows/results/windows-<id>.json` and compare:

```sh
cargo xtask pipeline win diff --reference target/pipeline/windows/results/windows-<id>.json --build-kernel
```

Compare with the executables of that bundle (the build is reproducible, so
rebuilding unchanged sources gives the same files; the diff reports
different executables as an error).

**Scenario drivers** ([ADR 0088](adr/0088-7-zip-scenario-driver.md)): a
test that runs an unchanged application step by step and records each
step (exit code, stdout and stderr bytes, every file with size, SHA-256,
time and attributes) in a `poly-scenario/1` observation file. The first is
`tests/windows/sevenzip_scenario.rs` (15 runs of the pinned `7za.exe`). Its
data file `7za.exe` travels beside it: `win bundle --data-dir DIR` adds
`DIR/<test>/<file>` as `data` lines, which `poly-oracle.ps1` puts beside
the test (or `-TestsDir ... -DataDir DIR` on the PC). Its Windows recording
is committed as `tests/windows/oracle/sevenzip_scenario.windows.json`.

```sh
cargo xtask win 7zip-diff [--markdown docs/windows/7zip-recording.md]   # build, run on Poly, compare per step
cargo xtask pipeline win scenario show tests/windows/oracle/sevenzip_scenario.windows.json   # what Windows did
cargo xtask pipeline win scenario poly --bin-dir DIR [--data-dir DIR]  # Poly only
# on the PC, to record again (after the driver or the support crate changed):
powershell.exe -NoProfile -ExecutionPolicy Bypass -File ../poly/tools/pipeline/windows/poly-oracle.ps1 \
    -Bundle bundle-<id>.txt -Tests sevenzip_scenario -TestTimeouts sevenzip_scenario=300 -Out results.json
```

**The traced scenario** ([ADR 0110](adr/0110-7-zip-traced-scenario-and-what-passing-its-trace-means.md)):
`tests/windows/sevenzip_trace.rs` runs the same steps through the API
tracer's kit (ADR 0090) in a work directory of a fixed path length and
records each step's trace. The Windows reference is committed in canonical
form as `tests/windows/oracle/sevenzip_trace.reference.json`; the
`wintrace` boot group compares Poly's with it, a required check since
ADR 0114 (`TRACE_REQUIRED`).

```sh
cargo xtask win 7zip-trace diff [--markdown docs/windows/7zip-trace.md] [--check]   # stage, run on Poly, compare per step
cargo xtask win 7zip-trace diff --target poly.json                                 # compare a saved Poly run
# to record again (after the driver, poly-wtest, the tracer or its signatures changed), from the host:
polyd 'cargo xtask win 7zip-trace stage'
POLY_RUN=~/poly-dev/polyd tools/pipeline/windows/poly-remote.sh --host homeserver2 --tests sevenzip_trace \
    --data-dir target/pipeline/windows/data --runner-arg -Runs --runner-arg 1 --runner-arg -TestTimeouts --runner-arg sevenzip_trace=1800
cp .oracle-remote/homeserver2/results.json .oracle-remote/windows-1.json    # repeat for -2, -3
polyd 'cargo xtask win 7zip-trace record .oracle-remote/windows-1.json .oracle-remote/windows-2.json .oracle-remote/windows-3.json'
```
### Where Windows behaviour is already tested: Wine seeds

For a spec agent: `cargo xtask coverage show <id>` lists an entry's
**seeds**, the Wine conformance-test functions that check something and
call that export (`wine:ntdll/tests/file.c:test_NtCreateFile`, best first,
at most 8; ADR 0097). They say where to read for behaviour worth observing
(Wine's tests are a spec agent's oracle, CLEAN_ROOM.md); the spec agent
then writes its own tests and runs them on the oracle machine. Nothing of
Wine's tests is copied; the map holds only these names and the Wine
revision (`coverage/index.json`, `seeds.wine`). Implementers see only how
many seeds an entry has. The seeds are refreshed from a Wine checkout
outside the repository with `cargo xtask coverage seed-wine <checkout>
--revision <commit>`.

## macOS test oracle

The real macOS is the owner's MacBook Pro (Apple M1 Pro, macOS 27.0, build
26A428), where a session runs with a worktree of this repository; tests are
built and run there ([ADR 0182](adr/0182-macos-test-oracle.md); code in
`tools/pipeline/src/mac.rs`). The pipeline tool builds on macOS; what needs
Linux (the ptrace tracer, the diff runner's reference runs, app tests) says
so there.

**A test** is one C file, `tests/macos/<name>.c` (the corpus) or
`specs/macos/<component>/<symbol>.tests/<name>.c`, named in lower case. It
includes `poly_mtest.h` (`tests/macos/include`) and defines `int
test_main(void)`; it makes BSD calls (`bsd3(SYS_write, 1, buf, n)`, which
returns `sysret {ret, ret2, err}`: `err` is the errno when the carry flag
was set) and Mach traps (`mach0(MACH_task_self_trap)`) directly and reports
with `check(name, cond, got)`, `check_ok(name, r)`, `check_err(name, r,
errno)`, `value(name, v)` and `finish()`, in the Linux corpus's output
format. It calls nothing in libSystem: `mac build` refuses an executable
that imports a symbol or loads a library other than libSystem. Never print
addresses, port names, pids or times: every test runs three times per
architecture and must print the same bytes each time. A value may differ
between x86_64 and arm64 (the page size); each architecture is recorded on
its own. A test starts in a fresh, empty scratch directory (use relative
paths), with an empty environment.

```sh
cargo xtask pipeline mac build [--tests-dir DIR]... [--tests a,b]   # target/pipeline/macos/bin/<arch>/, manifest.json
cargo xtask pipeline mac run [--tests-dir DIR]... [--tests a,b] [--runs 3] [--record]
cargo xtask pipeline mac show specs/macos/<component>/<symbol>.tests/macos.json
```

`mac run` builds and runs every test (default: the corpus and every
`specs/macos/*/*.tests/`) on both architectures and writes
`target/pipeline/macos/results/macos-<time>.json`; `--record` merges the
results into `macos.json` in each test's directory, per test and
architecture. x86_64 runs under Rosetta 2; on a Mac without it, x86_64 is
reported `unavailable` and left out of the recording (the arm64 results are
kept), so recording again once Rosetta is installed adds x86_64. A test
that timed out, did not start or was not the same on every run fails the
command and is not recorded.

**A macOS entry, end to end** (on the Mac; the held-out store is
`~/poly-heldout`):

```sh
export POLY_HELDOUT_DIR=~/poly-heldout
cargo xtask coverage claim macos/syscall/bsd/write --agent spec-macos-1
cargo xtask pipeline spec new macos/syscall/bsd/write --agent spec-macos-1 --oracle-os "macOS 27.0 (26A428)"
# write tests outside the repository, check them with mac run --tests-dir <that dir>, then:
cargo xtask pipeline tests split macos/syscall/bsd/write <dir> --spec-agent spec-macos-1
cargo xtask pipeline mac run --tests-dir specs/macos/bsd/write.tests --record
cargo xtask pipeline mac run --tests-dir ~/poly-heldout/macos/syscall/bsd/write --record
```

Poly cannot run Mach-O programs yet, so there is no comparison with Poly
(`mac poly`/`mac diff`) until Mach-O loading exists.

## On a Windows host

The pipeline tool builds and its tests pass on a Windows host with the same
`cargo xtask pipeline ...` commands (ADR 0124); `ci lint` checks it with
`clippy --target x86_64-pc-windows-msvc` next to xtask. What is Linux by
nature is behind `cfg(unix)` and says so at run time instead of failing to
build:

| Command | On a Windows host |
|---|---|
| `spec`, `tests`, `handoff`, `initramfs`, `win ...` | work (`win build` cross-builds the Windows test programs; `win remote` needs ssh) |
| `build`, `trace`, `diff`, `polyrun` | exit 1: `` `pipeline <cmd>` needs a Linux host (ADR 0124); run it through polyd `` |
| `selfcheck`, `polycheck` | skip themselves with the same reason (exit 0), like without a C compiler |
| `mac ...` | need a Mac, as before |

Repository paths the tool records (spec sheets, visible tests, hand-off
log, manifests) always use `/`, whatever the host.

## CI runs Chrome for Windows

Since [ADR 0214](adr/0214-ci-runs-chrome-for-windows-stage-1-and-the-window-check-on-every-pull-request.md)
the `chrome` job of CI boots Chrome for Windows on Poly on every pull request:
`chrome (s1-headless)` (stage 1, `--dump-dom`, required) and
`chrome (s3-window)` (Chrome's window on the desktop, QMP clicks, a typed URL).
Chrome for Testing is downloaded in the job by `cargo xtask chrome-win fetch`
(pinned version, SHA-256 checked) and kept only in the repository's Actions
cache: it is never committed, never an artifact; only the serial logs are
(`chrome-logs-<stage>`, on a failure). The stages use `data:` pages, no
network. There is no need to boot Chrome on the owner's PC to check a change:
push, and read the stage that failed. Locally `cargo xtask chrome-win fetch`
and `cargo xtask boot-tests chrome-win` still work; `POLY_CHROME_REQUIRED=1`
turns a missing package from a skip into a failure, as in CI.

## Safety rules

Reviewers check these on every agent PR; a violation stops the entry
(CLEAN_ROOM.md, red flags).

- Only changes that are **CI-green and held-out-green** merge, and only
  through pull requests reviewed by the owner; agents never push to
  `main` and never force-push.
- Agent-written code is **safe Rust only**: no `unsafe` blocks in agent
  changes (the existing kernel's `unsafe` is written and reviewed by
  humans). Reviews reject a PR that adds `unsafe`, `unsafe_code` or
  `global_asm!`, a build script, a proc-macro crate, a crate from a
  registry, or a binary file.
- The implementer's environment never contains the held-out store, the
  generated tests, the oracle VMs or third-party OS source; it receives
  only the bundle. `pipeline handoff verify` runs before every merge.
- One agent id never plays both roles for an entry (`handoff record` and
  `verify` enforce it).
- Agents do not edit `docs/ROADMAP.md`, the README progress section,
  `docs/progress/`, the ADR index table, `coverage/` (except through
  `claim`/`release`) or `CLEAN_ROOM.md`.
- The generated files (the README's `coverage:` and `roadmap:` sections,
  `docs/progress/*.svg` and the ADR index between its `adr-index:` markers)
  are written **on `main` only**, by `.github/workflows/progress.yml`, which
  runs `cargo xtask coverage readme` after every push to `main` (ADR 0213).
  A pull request must not change them: `cargo xtask ci coverage` fails if
  it does, and does not fail when they are out of date. Run
  `cargo xtask coverage readme` to preview them, then restore them
  (`git checkout README.md docs/progress docs/adr/README.md`) before you
  commit. If a merge of `main` conflicts in them, take main's version.
- CI also runs on any branch on demand: `gh workflow run ci.yml --ref <branch>`.
- Anything that looks like copied code, or a file of unknown origin,
  stops the entry (CLEAN_ROOM.md, red flags).

## What is not built yet

- `data<i>` fields and times in the kernel's `POLY-SYSCALL` records.
  (`POLY-SIGNAL` reports every signal a thread takes but `SIGKILL`, as the
  ptrace tracer's signal-delivery-stops: ADR 0062.)
- A macOS boundary tracer, and running macOS tests on Poly (after Mach-O
  loading; ADR 0182).

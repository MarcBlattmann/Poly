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
   `cargo xtask coverage claim <id> --agent <your-agent-id>`.
3. **Spec agent** (may observe the real OS): writes the spec sheet
   (`pipeline spec new` makes the skeleton) and test programs outside the
   repository; `pipeline tests split` keeps ~30% held out, outside the
   repository, and commits ~70% as visible tests.
4. **Hand-off.** `pipeline handoff record` logs who handed what (file
   hashes, git revision) to whom in `specs/HANDOFF_LOG.md`;
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
git commit specs/HANDOFF_LOG.md ...
pipeline handoff bundle linux/syscall/x86_64/brk --out ~/poly/implementer/inbox/brk
pipeline handoff verify linux/syscall/x86_64/brk --received ~/poly/implementer/inbox/brk
pipeline handoff verify                          # every record; run before merging
```

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

`win build` cross-compiles with the pinned toolchain (`-Zbuild-std` from
`rust-src`, `rust-lld`; the first build compiles `core`, about 30 s) and
refuses an executable that imports from another DLL, by ordinal, or
delay-loaded. `win diff` compares per test the 32-bit exit code, the
observation file, stdout and stderr; a reference that timed out, did not
start or was not the same on both runs is an **error**, a test Poly cannot
start is **unsupported**. `win show`, `win diff` and `win remote` read real
Windows' observations, so implementers may not run them.

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

**Splitting a Windows entry's tests**: `pipeline tests split <entry> <dir>
--spec-agent A --windows-results <results.json from the PC>` accepts only
`.rs`/`.cpp` test files, checks that every test ran on Windows with the
same result on every run, and writes the visible tests' results to
`<symbol>.tests/windows.json` and the held-out tests' to the store, beside
the tests (`win diff --reference <symbol>.tests/windows.json` compares Poly
with them without the PC). The hand-off works as for Linux entries.

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
`wintrace` boot group compares Poly's with it (report only until
`TRACE_REQUIRED`).

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
  `docs/progress/`, `coverage/` (except through `claim`/`release`) or
  `CLEAN_ROOM.md`.
- Anything that looks like copied code, or a file of unknown origin,
  stops the entry (CLEAN_ROOM.md, red flags).

## What is not built yet

- `data<i>` fields and times in the kernel's `POLY-SYSCALL` records.
  (`POLY-SIGNAL` reports every signal a thread takes but `SIGKILL`, as the
  ptrace tracer's signal-delivery-stops: ADR 0062.)
- The macOS oracle and tracer (later phases).

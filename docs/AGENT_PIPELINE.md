# Agent pipeline

How coverage entries go from "missing" to "green" with AI agents, which
parts are deterministic tools in this repository and which are LLM roles,
and how to run it unattended. The rules it enforces are in
[CLEAN_ROOM.md](../CLEAN_ROOM.md); the design decisions, including the
interface the kernel implements for test runs, are in
[ADR 0027](adr/0027-agent-pipeline-tools-and-test-run-protocol.md); the
scheduler that runs it all unattended is
[ADR 0056](adr/0056-pipeline-scheduler-loop-and-home-server-deployment.md)
(amended by [ADR 0074](adr/0074-pipeline-sessions-paid-once-and-spec-feedback-cycles.md)).

## The loop

```
  tracer ──► work queue ──► spec agent ──► hand-off ──► implementer ──► diff runner ──┬─► green: PR, CI, merge
 (real OS)   (`next`)       (LLM, oracle)   (logged)     (LLM, no oracle)  (visible +     │
                                                                           held-out)      └─► red: diff back to the
                                                                                               implementer; after ~5 red
                                                                                               cycles, a human's queue
```

1. **Trace.** Run programs on the real OS (a Linux VM, or this machine,
   which is real Linux) and record every syscall: `pipeline trace`.
2. **Queue.** `pipeline next` lists unclaimed coverage entries by weight,
   then by first use in the start-up traces. `pipeline claim` takes one.
3. **Spec agent** (LLM, may observe the real OS): writes the spec sheet
   (`pipeline spec new` makes the skeleton) and test programs outside the
   repository; `pipeline tests split` keeps ~30% held out, outside the
   repository, and commits ~70% as visible tests.
4. **Hand-off.** `pipeline handoff record` logs who handed what (file
   hashes, git revision) to whom in `specs/HANDOFF_LOG.md`;
   `pipeline handoff bundle` writes exactly those files for the implementer.
5. **Implementer** (LLM, never sees the real OS or held-out tests):
   implements against the spec and visible tests, runs `cargo xtask ci`.
6. **Diff runner.** `pipeline diff --target poly` runs the corpus and the
   entry's tests (visible and held-out) on the real OS and on Poly and
   compares them per test. Green: open a PR; it merges when CI is green.
   Red: the visible tests' diff goes back to the implementer (`pipeline
   role implementer --feedback <report>`; held-out tests only as a count);
   after five red cycles the entry goes to a human.
7. Any app-level failure later becomes a new entry at the top of the queue.

## Deterministic tools vs. LLM roles

| Step | Who | Command |
|---|---|---|
| Build the test corpus (`tests/linux/`) | tool | `cargo xtask pipeline build` |
| Trace on the real OS | tool | `cargo xtask pipeline trace <test\|path>... [--all] [--print]` |
| Pick the next entry, claim / release | tool | `pipeline next`, `pipeline claim <id> --agent A`, `pipeline release <id>` |
| Observe behaviour, write spec and tests | **spec agent (LLM)** | `pipeline role spec <id> ...` runs it |
| Spec skeleton from `specs/TEMPLATE.yaml` | tool | `pipeline spec new <id> --agent A` |
| Visible / held-out split, manifest | tool | `pipeline tests split <id> <dir> --spec-agent A` |
| Hand-off log, bundle, verification | tool | `pipeline handoff record\|bundle\|verify` |
| Implement | **implementer (LLM)** | `pipeline role implementer <id> ...` runs it |
| Compare real OS and Poly | tool | `pipeline diff [--target linux\|poly]` |
| Merge decision | CI + held-out result | PR, never a direct push |
| All of the above, unattended | tool | `pipeline loop [--once] [--dry-run]`, `loop status`, `loop pause\|resume`, `loop reset` |
| Windows tests: build, bundle, run on Poly, compare | tool | `pipeline win build\|bundle\|poly\|diff\|show` (runner on the PC: `poly-oracle.ps1`) |

All of these are `cargo xtask pipeline <command>`; the full option list is
at the top of `tools/pipeline/src/main.rs`. Outputs go under
`target/pipeline/` (`bin/`, `traces/`, `runs/<time>-<target>/` with
`summary.json` and `report.txt`, `runs/latest`, `roles/`).

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
  signal action set for many signals); `rseq` (`ENOSYS` on Poly by
  decision, ADR 0043/0049) and `SIGCHLD` are left out. Arguments are shown
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
tool's caller had (the loop's systemd unit has `UMask=0027`), and as the
user, groups and supplementary groups of whoever runs the tool: root in
this container, `server` (uid 1000) on the home server, a normal user on
GitHub's CI. The diff runner records that identity (`summary.json`
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
GitHub, so CI cannot run them: they are checked by the runner when it
marks an entry green (`pipeline diff --target poly` with the entry's
visible and held-out directories; the loop's verify step) and again
whenever the runner re-runs the entries. Today 11 of the 12 corpus programs
pass (`libc-hello` differs at `rseq`: Poly answers `ENOSYS`, with the same
output and exit status); all 11 corpus app tests pass (app mode); and 139
green entries bring 245 visible tests. The spec tests take about a minute with four jobs
(the app tests run one at a time after them). (`write` was green
before its spec tests existed; its visible and held-out spec tests pass
since the runner's third pass.)

### Held-out store and hand-off log

```sh
export POLY_HELDOUT_DIR=/srv/poly/heldout       # default ~/.poly-heldout; never inside the repository
pipeline spec new linux/syscall/x86_64/brk --agent spec-1
pipeline tests split linux/syscall/x86_64/brk /srv/poly/generated/brk --spec-agent spec-1
git commit ...                                   # spec sheet + visible tests
pipeline handoff record linux/syscall/x86_64/brk --spec-agent spec-1 --implementer impl-1
git commit specs/HANDOFF_LOG.md ...
pipeline handoff bundle linux/syscall/x86_64/brk --out /srv/poly/implementer/inbox/brk
pipeline handoff verify linux/syscall/x86_64/brk --received /srv/poly/implementer/inbox/brk
pipeline handoff verify                          # every record; run before merging
```

The held-out store holds `<os>/<layer>/<component>/<symbol>/` with the
held-out files and a `MANIFEST.json` (hashes of visible and held-out
files, spec agent, revision). `verify` proves from git that each logged
file had the logged content at the logged revision, that no agent had both
roles for an entry, and that no held-out content is in the log, under
`specs/`, or in what the implementer received, whatever the file is called.

## Windows test oracle

The real Windows is the owner's Windows 11 PC, reached through a Claude Code
session on it; nothing is installed there. Test programs are built here, run
there by a PowerShell script and on Poly in QEMU, and the results compared
([ADR 0075](adr/0075-windows-test-oracle.md); code in
`tools/pipeline/src/windows.rs`, runner `tools/pipeline/windows/poly-oracle.ps1`).

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
start is **unsupported**. `win show` and `win diff` read real Windows'
observations, so implementers may not run them.

**One round on the PC** (the coordinator relays; the session there uses Git
Bash). Here:

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

## Running the LLM roles

The pipeline never calls a model. Each role is a command, configured per
role, run by `pipeline role <spec|implementer> <entry> --workdir DIR
--agent ID` with a generated prompt:

```sh
export POLY_SPEC_AGENT_CMD='...'                # or --preset claude|opencode [--model M]
export POLY_IMPLEMENTER_CMD='...'
pipeline role spec linux/syscall/x86_64/brk --workdir /srv/poly/spec/repo --agent spec-1 \
    --generated /srv/poly/generated/brk
pipeline role implementer linux/syscall/x86_64/brk --workdir /srv/poly/impl/repo --agent impl-1 \
    [--feedback target/pipeline/runs/latest/report.txt] [--dry-run]
```

Placeholders in a command: `{prompt_file}`, `{workdir}`, `{entry}`,
`{agent}`, `{role}` (substituted shell-quoted). The command runs with
`sh -c` in the working directory with `POLY_ROLE`, `POLY_ENTRY`,
`POLY_AGENT_ID` and `POLY_PROMPT_FILE` set; for the implementer
`POLY_HELDOUT_DIR` is removed, and a working directory overlapping the
held-out store is refused. Every call is a new agent session (nothing is
resumed), so the two roles' contexts never mix, and neither do two
hand-offs. Prompts, the exact command and the output are kept in
`target/pipeline/roles/<time>-<role>-<symbol>/`. With `POLY_ROLE=implementer`
the pipeline refuses `trace`, `diff`, `spec` and `tests`, and `cargo xtask ci`
skips its Linux self-check and the Poly check (both run the reference OS).

### Preset 1: Claude Code (headless)

`--preset claude` runs:

```sh
claude -p --output-format text --permission-mode acceptEdits [--model M] \
    --allowedTools '<role tools>' [--disallowedTools '<denied>'] < {prompt_file}
```

- Spec agent: `Read Write Edit Glob Grep Bash`.
- Implementer: file tools plus `cargo build/test/clippy/fmt`, `cargo xtask
  ci/test/build` and local `git` commands; denied: `WebFetch`,
  `WebSearch`, `cargo xtask pipeline`, `strace`, `gdb`, `objdump`.

A **Claude subscription** (Pro/Max) is used through Claude Code itself:
log in once with `claude` on the server as the user that runs the role.
Do not use the subscription login through third-party tools; for those,
use an API key. Subscriptions have **usage limits** that reset in windows
(a rolling 5-hour window and a weekly limit). `pipeline role` looks for
"usage limit", "limit reached", "rate limit", "too many requests" or
"quota exceeded" in the agent's output (override with
`POLY_LIMIT_PATTERNS='a|b'`) and then exits with status **75**
(`cargo xtask pipeline` passes the tool's exit status on): the
scheduler pauses that role and retries later (for example every 30
minutes, or at the reset time the message gives) instead of counting a
red cycle. Work already committed on the branch is kept; the retry starts
a fresh session with the same prompt plus the latest diff report.

### Preset 2: OpenCode, any model

`--preset opencode --model <provider/model>` runs
`opencode run --model <provider/model> "$(cat {prompt_file})"`. Any
provider OpenCode supports works (a local model through Ollama on the home
server, OpenRouter, an API key). Restrict the implementer's tools in its
working directory's `opencode.json` (`permission`: edits allowed, `bash`
limited to cargo and local git, `webfetch` denied).

### Suggested split

- **Implementer and escalations on Claude** (Claude Code with the
  subscription): implementation quality decides how many red cycles an
  entry needs, and escalated entries are the hard ones.
- **Spec agent on Claude or a cheaper model** (OpenCode with a local or
  inexpensive model): observing and writing tests is more mechanical, and
  the diff runner and the held-out tests catch weak specs. Use Claude for
  the spec agent too when the cheaper model's specs cause red cycles.
- Use different agent ids per model and role (`spec-qwen-1`,
  `impl-claude-1`); the log then shows which model wrote what.

## A home server running 24/7

`pipeline loop` (ADR [0056](adr/0056-pipeline-scheduler-loop-and-home-server-deployment.md))
runs the whole cycle unattended, entry after entry, as a systemd service.
`tools/pipeline/deploy/` has everything to set it up.

### Users, directories, channels

Three OS users; nothing one role saw can reach the other through files,
settings, memory files or session history, and the only channels are the
inbox and git:

```
/srv/poly/runner/      user `server` (has `gh` and push access): runs the loop and is the diff runner
  repo/                  checkout of main, remotes `origin` (GitHub) and `hub`
  work/w0, w1/           worktrees where the runner checks branches out (CI, diff, verify)
  state/                 one state file per entry, locks, pauses, logs of every command
  pipeline.toml          the configuration;  human-queue.md;  log/pipeline.log
/srv/poly/git/poly.git server, world-readable bare "hub" (core.sharedRepository=all, so refs the loop pushes under UMask=0027 stay readable): the role checkouts fetch from it; never forced
/srv/poly/spec/        user `polyspec` (home): repo/ (remote `hub` only), its own Claude login, rustup
/srv/poly/impl/        user `polyimpl` (home): repo/ (remote `hub` only), its own Claude login, rustup
/srv/poly/heldout/     polyspec:server 2750, the held-out store (POLY_HELDOUT_DIR of the spec agent and the runner)
/srv/poly/generated/   polyspec:server 2750, the spec agent's generated tests
/srv/poly/inbox/       server:polyimpl 2750, hand-off bundles and feedback for the implementer
/srv/poly/spec-inbox/  server:polyspec 2750, feedback for the spec agent (a failed spec check)
/srv/poly/bin/poly-role root, the role wrapper: the only command `server` may run as the role users
/srv/poly/bin/deployed.sha256  root, the fingerprint of the deploy files setup.sh installed last
```

`polyimpl` cannot read `heldout/`, `generated/`, `runner/`, `spec/` or
`spec-inbox/` (the setup script checks it). The loop starts a role with
`sudo -n -u polyspec /srv/poly/bin/poly-role spec sync|run ...` (likewise
`polyimpl` / `implementer`); `/etc/sudoers.d/poly-pipeline` allows exactly
that, and four `systemctl` commands for the service, NOPASSWD:

```
server ALL=(polyspec) NOPASSWD: /srv/poly/bin/poly-role spec *
server ALL=(polyimpl) NOPASSWD: /srv/poly/bin/poly-role implementer *
Cmnd_Alias POLY_PIPELINE_SERVICE = /usr/bin/systemctl start poly-pipeline, \
    /usr/bin/systemctl stop poly-pipeline, \
    /usr/bin/systemctl restart poly-pipeline, \
    /usr/bin/systemctl status poly-pipeline
server ALL=(root) NOPASSWD: POLY_PIPELINE_SERVICE
```

The last two lines let `server` control the service without a password,
and nothing else as root: sudo matches those four commands exactly (no
other arguments, no other unit; `poly-pipeline.service` spelled out does
not match). `status` is left out on systemd older than 247, which starts
its pager as root without secure mode under sudo (CVE-2023-26604); newer
systemd runs `less` in secure mode there, so the pager cannot open a shell.

sudo resets the environment; the wrapper checks every argument, sets the
role's environment (the implementer gets `POLY_ROLE=implementer` and never
`POLY_HELDOUT_DIR`; the spec agent gets `POLY_HELDOUT_DIR=/srv/poly/heldout`
and an empty `/srv/poly/generated/<symbol>`, its held-out directory removed
too, unless the session is a feedback session, which continues the last
one with its tests), and runs `cargo xtask pipeline role <role> <entry>
--workdir <its repo> --preset claude [--model M] [--feedback F]` in the
role's checkout; feedback is taken only from the role's own inbox
(`inbox/` for the implementer, `spec-inbox/` for the spec agent). `sync
<branch> <base>` puts that checkout on the branch, from the hub, clean.

Code moves through git only. Agents commit on their branch
(`pipeline/spec/<symbol>`, `pipeline/impl/<symbol>`) and never push; the loop
fetches the branch from the role's checkout and pushes it to the hub without
`--force` (a rewritten history is refused, and the entry goes to the human
queue). Only the loop pushes to GitHub, and never to `main`.

### One cycle per entry

1. **claim** (runner): `pipeline next` order, skipping entries the loop has
   state for; on a new `pipeline/spec/<symbol>` from `origin/main`:
   `coverage claim <id> --agent <spec agent>`, committed, pushed to the hub.
2. **spec**: `poly-role spec sync`, `poly-role spec run` (the spec agent
   writes the spec sheet, tests in `generated/`, runs `tests split`,
   commits); the loop fetches and pushes the branch. A session that fails
   or commits nothing is tried again, up to `spec_attempts` (2) sessions.
3. **spec-check** (runner): the spec agent changed only its entry's spec
   sheet and tests directory; the sheet, visible tests and held-out tests
   exist; `cargo xtask ci` passes (`spec.ci`).
   - **Red** (CI fails, or the sheet, the visible or the held-out tests are
     missing): feedback into `/srv/poly/spec-inbox/<entry>/feedback-<n>.txt`
     and back to step 2, `poly-role spec run --feedback` on the spec branch
     as it is: the session continues the last one (its commits, generated
     and held-out tests stay) and commits a fix on top. The feedback is CI's
     last 80 lines, not redacted: the spec agent wrote the held-out tests and
     reads the held-out store, CI of a spec branch runs without it, and the
     spec inbox is not readable by the implementer. After `spec_cycles` (2)
     feedback sessions, the next failed check sends the entry to the human
     queue (`spec_cycles = 0`: at once).
   - **Red flag** (a path outside the entry's spec sheet and tests): the
     human queue at once.
4. **handoff** (runner): `pipeline handoff record` with the spec agent's head
   as the revision, committed and pushed on the spec branch;
   `pipeline handoff bundle` into `/srv/poly/inbox/<entry>/bundle`.
5. **implement**: `poly-role implementer sync pipeline/impl/<symbol>` (from
   the spec branch the first time, then from itself), `poly-role implementer
   run [--feedback /srv/poly/inbox/<entry>/feedback-<n>.txt]`; the loop
   fetches and pushes the branch.
6. **verify** (runner, on the implementer's branch), in this order, before
   anything from the branch runs: the safety checks on the session's diff;
   `pipeline handoff verify <id> --received <bundle>` (with the held-out
   store); `cargo xtask ci`; `pipeline diff --target poly --build-kernel
   --tests-dir <visible> --tests-dir <held-out> --tests <their names>`.
   Green: every visible and held-out test of the entry passes. The tests
   run as `server` on both sides (umask 022, not the unit's 0027), and
   `needs=root` tests as root on Poly alone (Diff runner, above).
   - **Red** (CI or the diff): feedback into the inbox and back to step 5.
     The feedback has the visible tests' results in full and the held-out
     tests only as "k of n pass"; CI output goes back as its last 80 lines
     with every line that names or quotes a held-out test withheld. An
     implementer session that fails or commits nothing is a red cycle too.
     After `max_cycles` (5) red cycles: the human queue.
   - **Red flag** (a forbidden path, added `unsafe`, `handoff verify`
     failing, a rewritten branch): the human queue at once.
7. **publish** (runner): the status commit (`coverage set-status <id> green
   --tests-visible V --tests-held-out H`, `coverage readme`), the whole
   branch checked against main once more, `coverage check` and `readme
   --check`, pushed to the hub and GitHub, `gh pr create`, and with
   `github.auto_merge` `gh pr merge --auto --merge` (GitHub merges once the
   required checks pass; without auto-merge the PR waits for the owner).
8. **awaiting-merge**: every `poll_secs` `gh pr view`: merged → done;
   closed → the human queue; conflicting with main → **refresh**: main is
   merged in (conflicts in the generated files, README progress,
   `docs/progress/`, the entry's coverage file and the hand-off log, are
   resolved by the loop; others go to the human), then verify and publish
   again.

**One PR per entry** carries the claim, the spec, the hand-off record, the
implementation and the status change. Merge pipeline PRs with a **merge
commit**, never squash or rebase: the hand-off log cites the spec agent's
commit by its hash, and `handoff verify` reads the files from it.

**Usage limits.** Exit status 75 from a role pauses that role until the
reset time in the message (`...limit reached|<unix time>`, `resets 3pm
(Europe/Berlin)`, `try again in 20 minutes`) plus two minutes, else for
`limit_pause_secs` (30 minutes); the entry stays at its step and the session
is retried then, without counting a red cycle. Other entries' runner steps
go on meanwhile.

**Errors** of the infrastructure (git, sudo, a tool that crashed) back off
2, 4, ... 60 minutes; after `max_errors` in a row the entry goes to the
human queue.

**An agent session is paid for once.** The state records a session
(`session`: the branch head it starts from) once the role's checkout is
synced, and its exit status when it ends. If anything after it fails (the
fetch of its branch from the role checkout, say git's "dubious ownership"),
or the loop stops, the retry fetches the role's branch first and neither
syncs the checkout nor starts the agent: a commit beyond the session's
start is its result, and an ended session without one counts as "committed
nothing". Only a session the loop never saw end, and that left no commit,
runs again (from a clean checkout; for the spec agent a new session also
removes that entry's generated and held-out tests, a feedback session
does not). A role paused by a usage limit does not hold up such a fetch.

**The human queue** is `/srv/poly/runner/human-queue.md`: one section per
stopped entry with the reason, the branches (pushed to GitHub too), the
state file and the logs. `notify.command` runs then (and when a PR is
opened) with `POLY_EVENT`, `POLY_ENTRY`, `POLY_MESSAGE`, e.g. a `curl` to an
ntfy topic for a push notification; also with `POLY_EVENT=deploy` and
`update` and an empty `POLY_ENTRY` for the loop's own notices (see "Updates
and service control"). `pipeline loop reset <id>` forgets an
entry (and deletes its hub branches) so the loop takes it again.

### Running the loop

```sh
cargo xtask pipeline loop [--config FILE]     # max_parallel workers, forever (the systemd service)
cargo xtask pipeline loop --once              # drive one entry until it waits or ends, then exit
cargo xtask pipeline loop --entry linux/syscall/x86_64/brk [--once]   # only this entry
cargo xtask pipeline loop --dry-run           # print the commands for the next entry; changes nothing
cargo xtask pipeline loop status [--json]     # every entry's step, cycle, PR or reason; paused roles; notices
cargo xtask pipeline loop watch [--once|--json]   # live dashboard (below, "Watching the loop")
cargo xtask pipeline loop pause [--role spec|implementer|all] [--for 4h] [--reason TEXT]
cargo xtask pipeline loop resume [--role spec|implementer|all]
cargo xtask pipeline loop reset <id> [--keep-branches]
```

`loop pause` writes the pause file the cycle reads before every agent
session (`state/pause-<role>.json`, `{until, reason}`, the same file a usage
limit writes): no new session of that role starts until then, or until
`loop resume` without `--for` (`--role` defaults to both, `--for` takes
`30m`, `4h`, `2d`, `1h30m`). A session that is running finishes, and the
runner's own steps (CI, diff, PRs) go on; `sudo systemctl stop
poly-pipeline` stops everything. A usage limit hit during an owner's
longer pause does not shorten it. `loop resume` removes the file (a usage
limit's too); the loop starts the next session within `idle_secs`.
`loop status` lists both roles with their pauses.

The configuration is TOML, looked up as `--config`, `$POLY_PIPELINE_CONFIG`,
`/etc/poly-pipeline.toml`, `/srv/poly/runner/pipeline.toml` (where setup.sh
installs it, so `loop status` and `loop pause` find it without
`--config`), `~/.config/poly/pipeline.toml`; every key has a
default for this layout, unknown keys are errors. The example with every
key and its meaning is
[`tools/pipeline/deploy/pipeline.example.toml`](../tools/pipeline/deploy/pipeline.example.toml):
`[loop]` (`max_parallel` 1, 2 on a 4-core machine; `max_cycles` 5;
`spec_attempts`; `spec_cycles` 2; `max_errors`; `max_open_prs`;
`entries`/`exclude` id prefixes; `idle_secs`, `limit_pause_secs`,
`poll_secs`; `self_update`), `[paths]` (with `spec_inbox`,
`deploy_stamp`), `[git]`,
`[github]` (`open_prs`, `auto_merge`, `push_human_branches`), `[spec]` and
`[implementer]` (agent id, the wrapper command, `model`, the checkout; the
implementer's `allowed_paths`), `[safety] extra_forbidden`,
`[notify] command`.

Logs: one line per event on stdout (journald priorities under systemd) and
in `log/pipeline.log`; every command's full output in
`state/logs/<entry>/`. With `max_parallel = 2` one entry's spec agent and
another's implementer can run at the same time (one session per role: one
checkout per role), and the runner verifies in its own worktree per worker.
`flock` locks keep two workers or two loops off the same entry; a crash or
reboot resumes every entry at its step (each step is idempotent, and an
agent session that ended is never run again, see above).

### Watching the loop

`loop watch` is a live dashboard for a plain terminal of 80 columns or
more (over SSH too): it redraws every 2 seconds on the alternate screen,
`q` or Ctrl-C quits and puts the terminal back (so do SIGTERM and a
dropped connection), `r` redraws at once. On the server, as the runner
user (the state, the log and the configuration are `server`'s, mode
0750/0640; no root needed), in one line to keep as an alias:

```sh
alias polywatch="sudo -u server -H bash -lc 'cd /srv/poly/runner/repo && exec cargo xtask pipeline loop watch'"
```

From another machine in one go (`-t`: a terminal, for the keys and for
sudo's password): `ssh -t homeserver "sudo -u server -H bash -lc 'cd
/srv/poly/runner/repo && exec cargo xtask pipeline loop watch'"`.

`cargo xtask` builds the tool first when the runner's checkout moved on
(the loop fast-forwards it to main); `sudo -u server
/srv/poly/runner/repo/target/debug/pipeline loop watch` starts at once
with the binary the last build left. Options:
`--interval SECS` (2), `--config FILE` (else found as the loop finds
it), `--lines N` of the log (8), `--width N`;
`--once` prints one snapshot as plain text and exits (colours only on a
terminal; also what you get when stdout is not a terminal), for a quick
look or a log: `ssh homeserver "sudo -u server ... loop watch --once"`;
`--json` prints the snapshot as JSON.

```
POLY PIPELINE  srv  2026-10-21 11:04:29 UTC   q quits, r refreshes
service active  loop pid 5766 up 3d04h  max_parallel 2
config /srv/poly/runner/pipeline.toml  state /srv/poly/runner/state
load 6.83 7.91 9.69  mem 14.2G/15.7G free  disk 112G/450G free on /srv/poly
PAUSED spec until 11:52 UTC (in 48m20s): usage limit (... resets 12pm (Europe/Berlin))
done today 2 (31 in all)  human queue 1  open PRs 1  in flight 3/2

ENTRIES (linux/syscall/x86_64/...)
  ENTRY        STEP       CYCLE ERR  IN STEP NOTE
* brk          implement  2/5   0/8  20m50s  running in w0
* mmap         verify     0/5   1/8  5m20s   running in w1
  rt_sigaction spec       s1/2  2/8  1h05m   error back-off, retry in 7m00s (11:
  openat       waiting    0/5   0/8  1h01m   github.com/marcbl/poly/pull/212, ch
  chdir        human      5/5   0/8  1h08m   5 red cycles; last: 2 of 6 held-out
  getpid       done       0/5   0/8  -       github.com/marcbl/poly/pull/205

ACTIVITY
  w0  brk          implementer session 20m40s  pid 5768
                   claude pid 5770 as polyimpl up 20m38s; log written 3s ago
  w1  mmap         CI 4m36s  pid 5769
                   log written 0s ago

LOG /srv/poly/runner/log/pipeline.log
10:43:49 w0 brk: [implement] implementer session starts (feedback-2.txt)
10:54:29 w1 rt_sigaction: [spec] error: git fetch: dubious ownership
...
```

- **Header**: `systemctl is-active poly-pipeline` ("unknown" without
  systemd), the `pipeline loop` process and its uptime, load, available
  memory and the free space of `/srv/poly` (or of the state directory's
  file system; red under 10%), roles paused (a usage limit or `loop
  pause`) with the time they resume, the loop's notices as `loop status`
  shows them (deploy files changed, a new version that does not build),
  and the totals: merged today (UTC), the human queue,
  the PRs the state knows to be open (no GitHub request), entries in flight
  against `max_parallel`.
- **Entries**: in flight first (`*`: a worker holds its lock now), then
  the human queue and what was merged in the last 24 hours. STEP is the
  state's step, with `waiting` for a PR waiting for its merge and `done`
  for merged; CYCLE is red cycles of `max_cycles` (`sN/M`: spec checks
  of `spec_cycles`); ERR infrastructure errors in a row of `max_errors`;
  IN STEP the time since the entry reached the step (from its history).
  NOTE: the PR, why it is in the human queue, the worker, or what it waits
  for (error back-off, a paused role, the next PR check).
- **Activity**: what each worker runs right now: the loop's child
  processes (an agent session through `poly-role` with the Claude Code
  process under it, its user and age; CI; the diff run; a `git` or `gh`
  command), each with its entry (from its arguments or from the command
  log it writes in `state/logs/<entry>/`) and how long ago that log was
  last written. The worker (`w0`, ...) comes from the log. Where the
  process list does not show the loop (another PID namespace, `hidepid`),
  the newest command log of each locked entry stands in, marked `~`.
- **Log**: the last lines of `log_file` (`log/pipeline.log`).

It only reads: the entry states (written by an atomic rename, so a file is
whole or not there; a `.tmp` file of a write in progress is skipped, a
file that does not parse is shown as a problem), the pause files, the
log, the command logs' names and times, and `/proc` (the process list,
`/proc/locks` for the loop's `flock` locks, load and memory). It writes
nothing and holds no lock; only where `/proc/locks` cannot be read does it
probe each lock file for an instant on a read-only descriptor (the loop
retries a lock for a second, so that never holds it up).

### Setting up the home server

On the server (Ubuntu 24.04), as an admin, from a checkout of the
repository. The runner user (`server`) needs rustup and `gh auth login`
first. The packages are what the setup script checks for: what `cargo
xtask ci` needs (QEMU for the boot tests, a static BusyBox for the BusyBox
and root-disk boots, e2fsprogs for the ext2/ext4 images, the static C
library for the `-static` test programs) and the tools the script uses:

```sh
sudo apt install git gcc libc6-dev qemu-system-x86 busybox-static e2fsprogs python3 curl
tools/pipeline/deploy/setup.sh --dry-run          # read what it would do (no root needed; with sudo it sees more)
sudo tools/pipeline/deploy/setup.sh               # [--runner-user server] [--repo-url URL] [--no-install-tools]
```

It is idempotent and changes nothing but: the users `polyspec` and
`polyimpl` (system users with locked passwords and their own groups, homes
`/srv/poly/spec` and `/srv/poly/impl`), `/srv/poly` (directories with the
owners and modes above, the wrapper, the clones, the hub, the
configuration; rustup, the toolchain `rust-toolchain.toml` pins and Claude
Code in the role homes), the pinned toolchain in `server`'s rustup,
`/etc/sudoers.d/poly-pipeline` (checked with `visudo` first) and
`/etc/systemd/system/poly-pipeline.service` (enabled, not started; `--start`
starts it), and last the fingerprint of the deploy files in
`/srv/poly/bin/deployed.sha256`. The toolchain is installed explicitly (`rustup toolchain
install` in the checkout, then checked with `RUSTUP_AUTO_INSTALL=0`), not
by rustup's deprecated install on first use. The unit runs `cargo xtask
pipeline loop` as `server` in `/srv/poly/runner/repo` with
`Restart=always`, `SuccessExitStatus=10` (the self-update exit, see
below), `Nice=10`, a low IO priority, `CPUQuota=300%` (3 of 4
cores), `MemoryHigh=11G`, `MemoryMax=12G`, and no `NoNewPrivileges` (the
loop needs sudo for the wrapper).

Before it changes anything, the script checks the packages, `server`'s
cargo (it must be rustup's) and the role users. Each role's home is fixed:
`setup.sh`, `poly-role` (which refuses to run when the passwd entry says
otherwise), `pipeline.toml` and the Claude Code logins all use
`/srv/poly/spec` and `/srv/poly/impl`. A user left from an earlier attempt
stops the script, with nothing changed, and it prints the commands that
fix it; the script never moves, deletes or re-owns an existing user's
files itself:

- home elsewhere (e.g. `/home/polyspec`): `sudo usermod -d /srv/poly/spec
  polyspec` when `/srv/poly/spec` exists already (this moves nothing;
  `usermod -m` refuses an existing directory and rewrites the passwd entry
  anyway), else `sudo usermod -d /srv/poly/spec -m polyspec` (moves the
  old home);
- another primary group: `sudo usermod -g polyspec polyspec`;
- `polyimpl` in any group but its own: `sudo gpasswd -d polyimpl <group>`;
- files in a role home that are not the role user's (`chown -R`), or a
  `repo/` that is not a git checkout (`rm -rf` it; it is cloned again).

Entries of `/srv/poly` it did not make (an old `mirror.git`) and extra
groups of `polyspec` are only warnings. `--dry-run` goes through the same
checks and then prints every command instead of running it; it works as an
ordinary user, but then it cannot see into the role homes or the sudoers
file and shows them as missing.

Then, once:

1. Log both role users in to Claude Code with the subscription, and trust
   their checkouts. `poly-role <role> login` starts `claude` with exactly
   the environment the sessions get (`HOME=/srv/poly/spec`, its `PATH`, in
   `repo/`), so the login lands where the agent reads it:
   `sudo -u polyspec /srv/poly/bin/poly-role spec login`, `/login` (Claude
   account with subscription), accept "trust this folder", `/exit`; the
   same with `sudo -u polyimpl /srv/poly/bin/poly-role implementer login`.
   (`login` needs a terminal; the loop never uses it.)
2. Review `/srv/poly/runner/pipeline.toml`.
3. On GitHub: allow merge commits and auto-merge; protect `main` with CI as
   a required check.
4. Try it: `sudo -u server -H bash -lc 'cd /srv/poly/runner/repo && cargo
   xtask pipeline loop --dry-run'`, then `... loop --once`.
5. `sudo systemctl start poly-pipeline`; follow it with
   `journalctl -u poly-pipeline -f` and `cargo xtask pipeline loop status`.

### Updates and service control

Routine operation needs no commands. The loop fast-forwards
`/srv/poly/runner/repo` to `origin/main` before it picks an entry, and at
least every `poll_secs` while none can start
(`tools/pipeline/src/scheduler/update.rs`):

- **The loop's own code** (`tools/pipeline/` but `deploy/`,
  `tools/xtask/`, `Cargo.toml`, `Cargo.lock`, `rust-toolchain.toml`,
  `.cargo/config.toml`) changed since the commit it was started from: it
  builds the new code first (`cargo build -p xtask`, then `-p
  poly-pipeline`, in the checkout, logged under `state/logs/loop/`). If
  that passes, it starts nothing new, lets every step in progress end (an
  agent session runs to its end; every step resumes from its state), and
  exits with status **10**. The unit lists 10 in `SuccessExitStatus=`, and
  `Restart=always` starts `cargo xtask pipeline loop` again after
  `RestartSec=60`, which runs the new code (built already). If the build
  fails, the loop does not exit: it keeps running the code it started
  with, logs the error, writes it to the human queue, runs the notify
  command (`POLY_EVENT=update`) once, shows it in `loop status`, and builds
  again when main moves. So a broken main never makes systemd spin. Other
  changes (the crates, the kernel, the coverage map) do not restart the
  loop: the tools that use them run as `cargo xtask ...` in the runner's
  worktrees, built from the branch every time.
- **The deploy files** (`tools/pipeline/deploy/`: the wrapper, `setup.sh`
  with the sudoers rule and the unit; not the example configuration)
  need root and are never applied by the loop. `setup.sh` records the
  fingerprint of what it installed in `/srv/poly/bin/deployed.sha256`
  (`[paths] deploy_stamp`); while the runner's checkout has other deploy
  files, `loop status` shows `deploy files changed: run sudo
  /srv/poly/runner/repo/tools/pipeline/deploy/setup.sh`, and the loop logs
  it, writes it to the human queue and runs the notify command
  (`POLY_EVENT=deploy`) once per change. Running `setup.sh` again clears
  it. A changed unit applies when the loop next starts (its next
  self-update, or `sudo systemctl restart poly-pipeline`, which stops the
  steps in progress; they resume from their state). `setup.sh` keeps
  `pipeline.toml`; compare it with the example for new keys (every key
  has a default).

`loop.self_update = false` turns the restart off; it is off anyway for
`--once`, `--dry-run` and a loop that does not run from `paths.repo`.
For the service itself, `server` may run `sudo systemctl
start|stop|restart|status poly-pipeline` without a password (the sudoers
rule above), and nothing else as root.

The runner builds and tests agent-written code (CI) as `server`, which can
read the held-out store and push: the checks before CI (paths, no build
scripts, no new crates) and the redacted feedback narrow that, and running
CI and the diff as a separate user or in a container is the next hardening
step (ADR 0056).

## Safety rules

`pipeline loop` enforces these by checking, not by trusting prompts: a
violation stops the entry and puts it in the human queue.

- Only changes that are **CI-green and held-out-green** merge, and only
  through pull requests reviewed by the owner or an automated merge rule;
  agents never push to `main` and never force-push. (The loop opens the PR
  only when green; agents never push at all, and the loop never forces.)
- Agent-written code is **safe Rust only**: no `unsafe` blocks in agent
  changes (the existing kernel's `unsafe` is written and reviewed by
  humans). Reviews reject a PR that adds `unsafe`; the loop refuses any
  agent diff that adds `unsafe`, `unsafe_code` or `global_asm!`, a build
  script, a proc-macro crate, a crate from a registry, or a binary file.
- The implementer's environment never contains the held-out store, the
  generated tests, the oracle VMs or third-party OS source; it receives
  only the bundle. `pipeline handoff verify` runs before every merge (the
  loop runs it on every implementer session, before CI).
- One agent id never plays both roles for an entry (`handoff record` and
  `verify` enforce it).
- Agents do not edit `docs/ROADMAP.md`, the README progress section,
  `docs/progress/`, `coverage/` (except through `claim`/`release`) or
  `CLEAN_ROOM.md`. The loop checks every agent diff against the role's
  allowed paths (the spec agent: its entry's spec sheet and tests; the
  implementer: `implementer.allowed_paths`) and a list of reserved files
  (also the hand-off log, `tools/`, `tests/`, CI and agent configuration),
  and the whole PR once more: only the loop's own status change, hand-off
  rows of this entry and the generated progress files may come on top.
- Anything that looks like copied code, or a file of unknown origin, stops
  the loop for that entry and goes to the human queue (CLEAN_ROOM.md, red
  flags).

## What is not built yet

- Running CI and the diff of agent branches as a user without GitHub
  credentials, or in a container (today the runner, `server`, does both).
- `data<i>` fields and times in the kernel's `POLY-SYSCALL` records.
  (`POLY-SIGNAL` reports every signal a thread takes but `SIGKILL`, as the
  ptrace tracer's signal-delivery-stops: ADR 0062.)
- A Windows API tracer and the Windows workflow in the pipeline (`tests
  split` and `handoff` for `.rs` tests and recorded Windows results, a
  `win polycheck` in CI against recorded references, the loop's Windows
  steps): ADR 0075 says what they need. The macOS oracle and tracer (later
  phases).

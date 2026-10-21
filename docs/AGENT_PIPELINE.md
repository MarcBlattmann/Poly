# Agent pipeline

How coverage entries go from "missing" to "green" with AI agents, which
parts are deterministic tools in this repository and which are LLM roles,
and how to run it unattended. The rules it enforces are in
[CLEAN_ROOM.md](../CLEAN_ROOM.md); the design decisions, including the
interface the kernel implements for test runs, are in
[ADR 0027](adr/0027-agent-pipeline-tools-and-test-run-protocol.md); the
scheduler that runs it all unattended is
[ADR 0056](adr/0056-pipeline-scheduler-loop-and-home-server-deployment.md).

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
| All of the above, unattended | tool | `pipeline loop [--once] [--dry-run]`, `loop status`, `loop reset` |

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
  it is what `POLY-OUT` reports for fds 1 and 2.
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
cargo xtask pipeline selfcheck                  # what `cargo xtask ci` runs: Linux against Linux
cargo xtask pipeline polycheck                  # and Poly against Linux (tests/linux/poly-pass.txt)
```

`--tests-dir` adds an entry's test programs (visible or held-out, `.c`
files in the corpus format, or `.app` app tests) to the run.

Per test it compares the exit status, stdout and stderr exactly and the
syscall sequence after masking addresses, pids and times; the first
divergence is printed. It exits non-zero on any failure, or with
`--baseline` only on regressions; `--strict` also fails on unsupported
tests. A test whose reference run times out is an error, whatever the
target did. `--tracer strace` uses strace instead of our ptrace tracer.

A test can weaken the syscall comparison for itself with a line
`poly-diff: syscalls=off` (or `names`) in its source, usually in the
header comment with the reason: tests with several processes or threads
(their calls interleave by scheduling), and tests whose calls depend on
timing or read data carrying pids. Their exit status and output are
still compared exactly, and a directive never strengthens the mode the
run was given.

The ptrace tracer follows forks and clones, including children that
leave the test's process group, and keeps job control intact: a tracee
stopped by SIGSTOP stays stopped and its parent sees it (`WUNTRACED`).
The program starts with default signal actions and an empty signal mask.

**Poly:** the tool builds a one-program initramfs per test, boots
`kernel/target/x86_64-unknown-none/debug/poly-kernel` (built if missing,
`--build-kernel` to rebuild, or `--kernel`) with
`poly.qemu_exit poly.init=/test-binary poly.test.capture`, and turns the
kernel's `POLY-*` records (ADR 0027, kernel side in ADR 0038) into a trace;
about half a second per test. Records carry no `data<i>` fields yet, so
pointer arguments are compared as "anything". A kernel that cannot run a
test's file says `POLY-INIT: unsupported <reason>`, and the test is
reported **unsupported**. `pipeline polycheck` (in `cargo xtask ci`) runs
the corpus this way and fails if a test listed in
`tests/linux/poly-pass.txt` does not pass; tests that newly pass are named,
to be added to the list. Today 11 of the 12 corpus programs pass;
`libc-hello` differs at `rseq` (Poly answers `ENOSYS`), with the same
output and exit status. Of the 11 corpus app tests, `app-true`,
`app-echo`, `app-head`, `app-grep`, `app-sed` and `app-mkdir` pass (app
mode); the rest are in `specs/linux/FAILING.md`.

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
`POLY_LIMIT_PATTERNS='a|b'`) and then exits with status **75**: the
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
/srv/poly/git/poly.git server, world-readable bare "hub": the role checkouts fetch from it; never forced
/srv/poly/spec/        user `polyspec` (home): repo/ (remote `hub` only), its own Claude login, rustup
/srv/poly/impl/        user `polyimpl` (home): repo/ (remote `hub` only), its own Claude login, rustup
/srv/poly/heldout/     polyspec:server 2750, the held-out store (POLY_HELDOUT_DIR of the spec agent and the runner)
/srv/poly/generated/   polyspec:server 2750, the spec agent's generated tests
/srv/poly/inbox/       server:polyimpl 2750, hand-off bundles and feedback for the implementer
/srv/poly/bin/poly-role root, the role wrapper: the only command `server` may run as the role users
```

`polyimpl` cannot read `heldout/`, `generated/`, `runner/` or `spec/` (the
setup script checks it). The loop starts a role with
`sudo -n -u polyspec /srv/poly/bin/poly-role spec sync|run ...` (likewise
`polyimpl` / `implementer`); `/etc/sudoers.d/poly-pipeline` allows exactly
that, NOPASSWD:

```
server ALL=(polyspec) NOPASSWD: /srv/poly/bin/poly-role spec *
server ALL=(polyimpl) NOPASSWD: /srv/poly/bin/poly-role implementer *
```

sudo resets the environment; the wrapper checks every argument, sets the
role's environment (the implementer gets `POLY_ROLE=implementer` and never
`POLY_HELDOUT_DIR`; the spec agent gets `POLY_HELDOUT_DIR=/srv/poly/heldout`
and an empty `/srv/poly/generated/<symbol>`), and runs `cargo xtask pipeline
role <role> <entry> --workdir <its repo> --preset claude [--model M]
[--feedback F]` in the role's checkout. `sync <branch> <base>` puts that
checkout on the branch, from the hub, clean.

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
   commits); the loop fetches and pushes the branch.
3. **spec-check** (runner): the spec agent changed only its entry's spec
   sheet and tests directory; the sheet, visible tests and held-out tests
   exist; `cargo xtask ci` passes (`spec.ci`).
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
   Green: every visible and held-out test of the entry passes.
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

**The human queue** is `/srv/poly/runner/human-queue.md`: one section per
stopped entry with the reason, the branches (pushed to GitHub too), the
state file and the logs. `notify.command` runs then (and when a PR is
opened) with `POLY_EVENT`, `POLY_ENTRY`, `POLY_MESSAGE`, e.g. a `curl` to an
ntfy topic for a push notification. `pipeline loop reset <id>` forgets an
entry (and deletes its hub branches) so the loop takes it again.

### Running the loop

```sh
cargo xtask pipeline loop [--config FILE]     # max_parallel workers, forever (the systemd service)
cargo xtask pipeline loop --once              # drive one entry until it waits or ends, then exit
cargo xtask pipeline loop --entry linux/syscall/x86_64/brk [--once]   # only this entry
cargo xtask pipeline loop --dry-run           # print the commands for the next entry; changes nothing
cargo xtask pipeline loop status [--json]     # every entry's step, cycle, PR or reason; paused roles
cargo xtask pipeline loop reset <id> [--keep-branches]
```

The configuration is TOML, looked up as `--config`, `$POLY_PIPELINE_CONFIG`,
`/etc/poly-pipeline.toml`, `~/.config/poly/pipeline.toml`; every key has a
default for this layout, unknown keys are errors. The example with every
key and its meaning is
[`tools/pipeline/deploy/pipeline.example.toml`](../tools/pipeline/deploy/pipeline.example.toml):
`[loop]` (`max_parallel` 1, 2 on a 4-core machine; `max_cycles` 5;
`spec_attempts`; `max_errors`; `max_open_prs`; `entries`/`exclude` id
prefixes; `idle_secs`, `limit_pause_secs`, `poll_secs`), `[paths]`, `[git]`,
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
reboot resumes every entry at its step (each step is idempotent).

### Setting up the home server

On the server (Ubuntu 24.04), as an admin, from a checkout of the
repository:

```sh
sudo apt install git gcc qemu-system-x86 curl     # what the setup script checks for
sudo tools/pipeline/deploy/setup.sh --dry-run     # read what it would do
sudo tools/pipeline/deploy/setup.sh               # [--runner-user server] [--repo-url URL] [--no-install-tools]
```

It is idempotent and changes nothing but: the users `polyspec` and
`polyimpl` (system users with locked passwords, homes in `/srv/poly`),
`/srv/poly` (directories with the owners and modes above, the wrapper, the
clones, the hub, the configuration, rustup and Claude Code in the role
homes), `/etc/sudoers.d/poly-pipeline` (checked with `visudo` first) and
`/etc/systemd/system/poly-pipeline.service` (enabled, not started; `--start`
starts it). The unit runs `cargo xtask pipeline loop` as `server` in
`/srv/poly/runner/repo` with `Restart=always`, `Nice=10`, a low IO priority,
`CPUQuota=300%` (3 of 4 cores), `MemoryHigh=11G`, `MemoryMax=12G`, and no
`NoNewPrivileges` (the loop needs sudo for the wrapper).

Then, once:

1. Log both role users in to Claude Code with the subscription, and trust
   their checkouts: `sudo -iu polyspec`, `cd repo && claude`, `/login`
   (Claude account with subscription), accept "trust this folder", `/exit`,
   `exit`; the same as `polyimpl`.
2. Review `/srv/poly/runner/pipeline.toml`.
3. On GitHub: allow merge commits and auto-merge; protect `main` with CI as
   a required check.
4. Try it: `sudo -u server -H bash -lc 'cd /srv/poly/runner/repo && cargo
   xtask pipeline loop --dry-run'`, then `... loop --once`.
5. `sudo systemctl start poly-pipeline`; follow it with
   `journalctl -u poly-pipeline -f` and `cargo xtask pipeline loop status`.

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
- `data<i>` fields and times in the kernel's `POLY-SYSCALL` records, and
  `POLY-SIGNAL` for signals a handler takes (only faults and fatal signals
  are reported).
- Windows and macOS oracles and tracers (later phases).

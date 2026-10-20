# Agent pipeline

How coverage entries go from "missing" to "green" with AI agents, which
parts are deterministic tools in this repository and which are LLM roles,
and how to run it unattended. The rules it enforces are in
[CLEAN_ROOM.md](../CLEAN_ROOM.md); the design decisions, including the
interface the kernel implements for test runs, are in
[ADR 0027](adr/0027-agent-pipeline-tools-and-test-run-protocol.md).

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
   Red: the diff goes back to the implementer (`pipeline role implementer
   --feedback <report>`); after about five red cycles the entry goes to a
   human.
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
the same bytes (tested).

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
files in the corpus format) to the run.

Per test it compares the exit status, stdout and stderr exactly and the
syscall sequence after masking addresses, pids and times; the first
divergence is printed. It exits non-zero on any failure, or with
`--baseline` only on regressions; `--strict` also fails on unsupported
tests. `--tracer strace` uses strace instead of our ptrace tracer.

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
output and exit status.

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

Two agent instances, one per role, with separate contexts and
directories, and a small scheduler (a systemd timer or a shell loop):

```
/srv/poly/spec/        OS user `polyspec`: repository checkout, the oracle (a Linux VM, later Windows/macOS VMs),
                       POLY_HELDOUT_DIR=/srv/poly/heldout, generated tests in /srv/poly/generated
/srv/poly/impl/        container or OS user `polyimpl`: its own repository checkout and the hand-off inbox only;
                       no access to /srv/poly/heldout, /srv/poly/generated or the oracle VMs, no POLY_HELDOUT_DIR
/srv/poly/runner/      OS user `polyrun`: checkout for the diff runner, read access to the held-out store and the VMs
```

One cycle per entry:

1. `pipeline next` → `pipeline claim <id> --agent spec-1`.
2. `pipeline role spec <id> --workdir /srv/poly/spec/repo ...`; then
   `cargo xtask ci` and a PR for the spec and visible tests.
3. `pipeline handoff record` (commit the log), `pipeline handoff bundle`
   into the implementer's inbox.
4. `pipeline role implementer <id> --workdir /srv/poly/impl/repo ...`
   (in the implementer's container).
5. In the runner's checkout of the implementer's branch: `cargo xtask ci`,
   `pipeline diff --target poly --tests-dir specs/linux/x86_64/brk.tests
   --tests-dir $POLY_HELDOUT_DIR/linux/syscall/x86_64/brk`, and
   `pipeline handoff verify <id> --received <inbox>`.
6. Green: PR; it merges when CI is green. Red: back to step 4 with
   `--feedback runs/latest/report.txt`; after about five red cycles,
   release the claim and put the entry in the human queue. Exit status 75
   from step 2 or 4: sleep, then repeat the same step.

Each role's instance keeps its own agent configuration and history (one
OS user or container per role), so nothing one role saw can reach the
other through settings, memory files or session history.

## Safety rules

- Only changes that are **CI-green and held-out-green** merge, and only
  through pull requests reviewed by the owner or an automated merge rule;
  agents never push to `main` and never force-push.
- Agent-written code is **safe Rust only**: no `unsafe` blocks in agent
  changes (the existing kernel's `unsafe` is written and reviewed by
  humans). Reviews reject a PR that adds `unsafe`.
- The implementer's environment never contains the held-out store, the
  generated tests, the oracle VMs or third-party OS source; it receives
  only the bundle. `pipeline handoff verify` runs before every merge.
- One agent id never plays both roles for an entry (`handoff record` and
  `verify` enforce it).
- Agents do not edit `docs/ROADMAP.md`, the README progress section,
  `docs/progress/`, `coverage/` (except through `claim`/`release`) or
  `CLEAN_ROOM.md`.
- Anything that looks like copied code, or a file of unknown origin, stops
  the loop for that entry and goes to the human queue (CLEAN_ROOM.md, red
  flags).

## What is not built yet

- The scheduler loop itself (the steps above, as a script or service).
- `data<i>` fields and times in the kernel's `POLY-SYSCALL` records, and
  `POLY-SIGNAL` for signals a handler takes (only faults and fatal signals
  are reported).
- Windows and macOS oracles and tracers (later phases).

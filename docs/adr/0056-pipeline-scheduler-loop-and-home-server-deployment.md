# 0056: The pipeline scheduler: a state machine per entry in `pipeline loop`, three OS users, git between them, one PR per entry

**Status:** Accepted. Builds on [0027](0027-agent-pipeline-tools-and-test-run-protocol.md)
(the deterministic pipeline tools and the role commands) and implements the
"scheduler loop" it left to the owner. The workflow is in
[AGENT_PIPELINE.md](../AGENT_PIPELINE.md). Amended by
[0074](0074-pipeline-sessions-paid-once-and-spec-feedback-cycles.md): an
agent session is recorded in the state and never run again once it ended
(a retry fetches its branch), and a failed spec check goes back to the
spec agent with feedback up to `spec_cycles` times.
CI's QEMU acceleration: see [ADR 0077](0077-ci-in-parallel-jobs-and-qemu-on-kvm.md).

## Context

ADR 0027 built every deterministic step of the agent pipeline (queue,
claim, spec skeleton, held-out split, hand-off log, bundle, verify, diff
runner) and made the two LLM roles commands (`pipeline role`), but nothing
ran the steps in order, unattended. The owner runs the pipeline on a home
server (Ubuntu 24.04, 4 cores, 15 GB, no KVM yet) with Claude Code on a Max
subscription, `gh` logged in as the owner under the user `server`. The loop
has to:

- run the cycle of "A home server running 24/7" for one entry after another,
  for weeks, surviving restarts, reboots and usage limits;
- keep the clean-room separation real on one machine: the implementer never
  reaches the held-out tests, the generated tests, the reference OS traces
  or the spec agent's history;
- enforce the safety rules (no `unsafe` from agents, no edits to the
  roadmap, README progress, coverage map, CLEAN_ROOM.md; CI-green and
  held-out-green before anything merges; agents never push to `main` or
  force-push) by checking, not by trusting prompts;
- hand hard entries to a human instead of looping on them.

## Decision

**One host command, `pipeline loop`, with the logic host-tested.** The
scheduler is a module of `tools/pipeline` (`src/scheduler/`), not a shell
script: every decision (what to do next, red or green, what is allowed in a
diff, when to retry) is Rust with unit tests. Everything outside its own
state goes through one trait, `Ops` (git, `cargo xtask ...`, the role
commands, `gh`, `date`, the clock). `RealOps` runs commands through a
`Shell`; the tests drive the cycle with a fake world (branches, scripted
agents, CI and diff results, PRs, crashes), and `--dry-run` runs `RealOps`
with a `DryShell` that prints each command and pretends it succeeds (CI runs
it). Configuration is a TOML file (a small subset parsed in the tool, no new
dependency), unknown keys are errors.

**A state machine per entry, saved after every step.** `claim → spec →
spec-check → handoff → implement → verify → publish → awaiting-merge →
merged`, with `refresh` (main merged into a conflicting PR, then verify
again) and `human` (stopped). Each entry has a JSON state file written by
atomic rename; a restart resumes at the step it was at. Steps are idempotent:
those whose result is a pushed branch look at the hub first (a claim or
hand-off commit pushed before a crash is recognised by its subject), a head
fetched from a role checkout but not yet pushed is kept in the state
(`pending_head`), an agent session cut by a crash simply runs again (its
checkout is reset first), and an existing PR is found before one is
created.

**Concurrency with `flock` locks.** `max_parallel` workers (1 by default,
2 on the home server); a worker holds the entry's lock while it drives it,
so two workers or two loop processes never take the same entry; a `pick`
lock serialises choosing new entries. One checkout per role means one agent
session per role at a time (a role lock); the runner checks branches out in
worktrees `w0..` (a slot lock each), so with two workers one entry's spec
agent and another's implementer run side by side while the runner verifies.
Locks die with the process: no stale locks after a crash.

**Three OS users; files and git are the only channels.**

| | `server` (the loop, runner) | `polyspec` | `polyimpl` |
|---|---|---|---|
| checkout | `/srv/poly/runner/repo` + worktrees | `/srv/poly/spec/repo` | `/srv/poly/impl/repo` |
| GitHub | `gh` login, push | none | none |
| held-out store, generated tests | reads (group) | writes (owner) | no access |
| inbox (bundle, feedback) | writes | none | reads (group) |
| Claude login | none needed | own | own |

The loop starts the roles with `sudo -n -u <role user> /srv/poly/bin/poly-role
<role> sync|run ...`; the sudoers rule allows that wrapper and nothing else,
NOPASSWD, and sudo resets the environment. The wrapper (root-owned) checks
every argument, sets the role's environment itself (the implementer gets
`POLY_ROLE=implementer` and never `POLY_HELDOUT_DIR`), and runs
`cargo xtask pipeline role ... --preset claude` in the role's checkout. The
role homes are group-readable by `server` (setgid group), never by the other
role.

**Git: agents commit, only the loop pushes, nothing is forced.** A bare
repository, the hub (`/srv/poly/git/poly.git`, owned by `server`,
world-readable, `receive.denyNonFastForwards`), is where the role checkouts
fetch from; they have no GitHub remote and no credentials. The wrapper's
`sync` puts the role checkout on `pipeline/spec/<symbol>` or
`pipeline/impl/<symbol>` from the hub. After a session the loop fetches the
branch from the role's checkout (as `server`, `safe.directory` for that
fetch only) and pushes it to the hub without `--force`: a rewritten history
is refused and is a red flag. The loop alone pushes to GitHub and opens PRs.

**One PR per entry, merged with a merge commit.** The entry's branch
carries, in order: the claim (coverage status `claimed`), the spec agent's
commits (spec sheet, visible tests), the hand-off record (made and committed
by the loop with the spec agent's head as the revision), the implementer's
commits, merges of main if it conflicted, and the loop's status commit
(`green` with `tests_visible` / `tests_held_out`, README progress and
`docs/progress/` regenerated by `coverage readme`). The PR is opened with
`gh pr create` only when CI and the held-out tests are green; with
`auto_merge` the loop runs `gh pr merge --auto --merge`, so GitHub merges it
once the required checks pass; otherwise it waits for the owner. A squash
or rebase merge would lose the commit the hand-off log cites, so pipeline
PRs are merged with merge commits only. One PR instead of a spec PR and an
implementation PR: the owner reviews an entry once, nothing half-done lands
on `main`, and the hand-off log row still points at the spec agent's own
commit. The claim is visible in the branch (and on GitHub once the PR is
open); on this machine the state files keep two loops off the same entry.

**Verify before anything from the branch runs.** In the runner, for each
implementer session: the safety checks on the session's diff; then
`pipeline handoff verify <entry> --received <bundle>` with the held-out
store (must pass: a failure is a red flag, not a red cycle); then
`cargo xtask ci`; then `pipeline diff --target poly --build-kernel` with the
entry's visible and held-out test directories, restricted to those tests
(the corpus is `polycheck`'s job inside CI). Green means every visible and
held-out test of the entry passes; an "unsupported" test is not green.

**Safety checks are code.** On every agent diff (`git diff -U0`): changed
paths must be the role's own (the spec agent: its entry's spec sheet and
tests directory; the implementer: `personalities/`, `crates/`,
`kernel/src/`, the Cargo manifests and locks, configurable), never the
reserved files (`docs/ROADMAP.md`, `README.md`, `docs/progress/`,
`coverage/`, `CLEAN_ROOM.md`, `CLAUDE.md`, `specs/HANDOFF_LOG.md`, `tools/`,
`tests/`, CI and agent configuration, licences); no added `unsafe`,
`unsafe_code` or `global_asm!`; no build scripts or proc-macro crates (they
would run on the runner); no new registry or git crates in a `Cargo.lock`;
no binary files or names git has to quote. Before the PR the whole branch
against main is checked again: agent paths plus only the loop's own changes
(this entry's line of its coverage file, changed only in its status fields
and green; added hand-off rows of this entry; README and `docs/progress/`,
which `coverage readme --check` proves generated). Any problem stops the
entry and queues it for the human (CLEAN_ROOM.md, red flags).

**What goes back to the implementer.** A red cycle writes feedback into the
inbox for the next session (`--feedback`): the visible tests' diff results
in full, held-out tests only as "k of n pass", never their names or output.
CI output goes back as its last lines with every line that names a held-out
test or contains a line of one withheld. An implementer session that fails
or commits nothing is a red cycle too, so every entry ends after at most
`max_cycles` (5) sessions. After the last red cycle the entry goes to the
human queue (a Markdown file next to the state), its branches are pushed to
GitHub for the owner, and an optional notify command runs; the loop does
not take it again until `pipeline loop reset <entry>`.

**Usage limits pause the role, never count.** Exit status 75 from a role
(`pipeline role` saw a usage-limit message) pauses that role until the
reset time the message gives (a Unix time, "resets 3pm (Europe/Berlin)"
resolved with `date`, "try again in 20 minutes") plus two minutes, else for
`limit_pause_secs` (30 minutes), at most eight days. The entry keeps its
step; other entries' runner work goes on. Infrastructure errors (a failed
`git fetch`, sudo, a tool that crashed) back off 2, 4, 8 ... 60 minutes and
reach the human queue only after `max_errors` in a row.

**Deployment files in the repository** (`tools/pipeline/deploy/`): an
idempotent `setup.sh` that creates the two users, `/srv/poly` with owners
and modes, the hub and the clones, installs rustup and Claude Code in the
role homes, the sudoers drop-in (validated with `visudo` before it is
installed) and a systemd unit (`Restart=always`, as `server`, `Nice=10`,
idle-class-like IO priority, 3 of 4 cores, memory capped at 12 GB, no
`NoNewPrivileges` because of sudo); it touches nothing else and installs no
packages. The role wrapper, an example configuration with every key, and
the owner's one-time steps (log the role users in to Claude, GitHub
settings) are documented there and in AGENT_PIPELINE.md.

## Consequences

- The pipeline can run unattended on the home server; its behaviour is
  pinned by unit tests (green path, red then green, five reds to the human,
  usage-limit pause and resume, crashes at several points, unsafe and
  forbidden paths, a failed hand-off verification, rewritten history,
  conflicts with main, two workers) and by the dry run in CI.
- The runner executes agent-written code (CI builds and tests the branch)
  as `server`, which can read the held-out store and push to GitHub. The
  checks before CI (paths, no build scripts, no new crates) and the redacted
  feedback narrow this, but a test that reads the held-out store and fails
  with its content is only caught by the redaction. Running CI and the diff
  as a fourth user without GitHub credentials, or in a container, is the
  next hardening step.
- Two entries in flight edit the same coverage file, README progress and
  hand-off log, so the second PR often conflicts with main once the first
  merges. The loop resolves conflicts in those generated files (main's
  version, regenerated; hand-off rows appended) and verifies again; any
  other conflict goes to the human.
- Branch names use the entry's symbol (`pipeline/spec/brk`): unique among
  Linux system calls; an entry whose symbol is not a safe ref name or is
  already in use is skipped.
- Both roles use the same subscription, so a limit usually hits both; each
  pauses on its own when it sees it.
- Without KVM, QEMU runs the diff and CI's boot tests under TCG: a verify
  step takes tens of minutes. The loop does not care how long a step takes.

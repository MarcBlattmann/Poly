# 0213: Landing work on main: hand-off records one file each, progress files written on main, CI on any branch, a fast `handoff verify`

**Status:** Accepted. Tooling only. Amends [0027](0027-agent-pipeline-tools-and-test-run-protocol.md)
(the hand-off log) and [0077](0077-ci-in-parallel-jobs-and-qemu-on-kvm.md) (CI in jobs).

## Context

About ten agents work at once, each opening pull requests. Every merge to
`main` made the next pull request conflict in files that are shared or
generated, and each conflict costs a re-merge and another CI run (about ten
minutes):

- `specs/HANDOFF_LOG.md`: every hand-off appended a row at the end of one
  table, so two pull requests that each hand something over conflict.
- The README's progress section and phase table and `docs/progress/*.svg`:
  `cargo xtask coverage readme` rewrote them in every pull request that
  touched `coverage/` or the roadmap, and CI failed when they were stale, so
  every pull request had to be regenerated after the one before it landed.
- The ADR index `docs/adr/README.md`: every ADR added a row at the end of one
  table, so two pull requests that each add an ADR conflict.

Two more costs: `pipeline handoff verify` took 80 seconds on a Mac (about 13
minutes on the Windows PC, over 30 inside the Docker wrapper). It ran one
`git show` per handed-over file (6,744 of them for 1,887 records); a process
start is cheap on a host and slow where the git directory is read over a
network file system (9p). And CI ran only for pull requests and pushes to
`main`, so a branch could not be tested before there was a pull request.

## Decision

**Hand-off records are one file each.** `pipeline handoff record` writes
`specs/handoffs/<UTC timestamp>-<spec agent>-<implementer>-<entry hash>.md`
(the entry hash is the first 12 hex digits of the SHA-256 of the entry id),
holding the fields of a log row: date, entry id, spec revision, both agents,
and every file with its SHA-256. Two pull requests add different files and do
not conflict. `handoff verify`, `bundle` and `latest` read the legacy rows of
`specs/HANDOFF_LOG.md` (in their order) and then the files (by date), and
check both the same way: hashes against git at the recorded revision, held-out
content, role separation across both (an agent that implemented an entry in
the legacy log cannot write its spec in a new file). The legacy log is neither
rewritten nor migrated: open pull requests still append to it and must keep
merging. A file whose name does not match its content is reported by `verify`;
agent ids are limited to letters, digits, `.`, `_` and `-` because they are in
the file name. Nothing else parses the log (polycheck and the coverage tool
never did), so no other reader changes.

**`handoff verify` reads git once.** All files of all records at their
revisions come out of one `git cat-file --batch` (`util::git_blobs`, a feeder
thread writes the requests while the answers are read, so the pipes never
fill), each blob is hashed as it arrives and dropped, and the roles are
checked against an index by entry instead of against every record. The
held-out scans (of the store and of `specs/`) hash on several threads. The
result is the same: the same problems for the same inputs. Time on a Mac:
79.5 seconds before, 0.6 seconds after.

**Generated files are written on `main` only.**

- The README's progress section (`coverage:begin/end`) and phase table
  (`roadmap:begin/end`), `docs/progress/*.svg` and, now, the ADR index are
  rewritten by `.github/workflows/progress.yml` after every push to `main`: it
  runs `cargo xtask coverage readme` and pushes a commit `Regenerate progress`
  with `[skip ci]` if something changed.
- `cargo xtask ci coverage` no longer fails when they are out of date.
  Instead, in a pull request it **fails if the change rewrites them**, compared
  with the base the pull request is merged into (`HEAD^1` of the merge commit CI
  checks out, which stays right when `main` moves after the pull request was
  opened; locally the merge base with `origin/main`, or nothing is checked where
  there is no `origin/main`). Only the generated parts count: the rest of
  `README.md` and `docs/adr/README.md` stays editable. The message explains
  that `main` regenerates them. A part that the base does not have yet (the ADR
  index's markers, in the change that adds them) is not compared.
- `cargo xtask coverage readme` stays for local previews; agents must not
  commit its output. `CLAUDE.md`, `CONTRIBUTING.md` and `docs/AGENT_PIPELINE.md`
  say so, replacing "run `coverage readme` after changing `coverage/`".

**The ADR index is generated, from the ADR files.** `docs/adr/README.md` keeps
its text above and below, and the table between `<!-- adr-index:begin -->` and
`<!-- adr-index:end -->` is one row per `NNNN-*.md`, in number order, with the
title of the file's first line (`# NNNN: title`) and the first words of its
`**Status:**` line (up to the first `. ` or `;`). The alternative, to show that
two pull requests cannot conflict in a hand-written table, does not hold: both
append a row at the same place. Generating it also fixes the index lagging the
files (169 rows for 177 ADRs when this was written). It costs the hand-written
one-line summaries, which were longer than the titles; the ADR itself has the
detail. Two pull requests that pick the same number get different file names,
both appear in the index, and nothing conflicts; picking the next free number
stays the author's job.

**CI runs on any branch on demand.** `ci.yml` has `workflow_dispatch:`, so
`gh workflow run ci.yml --ref <branch>` runs the whole CI on a branch without a
pull request. For that event the `changes` job reports `code=true` (it is the
same path as a push to `main`: not a `pull_request`, so nothing is skipped), the
concurrency group is `ci-refs/heads/<branch>` and `cancel-in-progress` is false:
it does not cancel a run of another branch or of a pull request (their groups
differ) and is not cancelled by one; a second manual run of the same branch
queues. The coverage step has no pull request to compare with and says so.

## What the owner must allow

The ruleset on `main` requires the `ci` status and has no bypass for the
workflow's push of the `[skip ci]` commit (a commit nobody ran CI on). GitHub
does not let the Actions app bypass a ruleset on a user-owned repository, so
the bypass is a **deploy key**: a write deploy key "progress workflow" is in the
ruleset's bypass list, and its private key is the Actions secret
`PROGRESS_DEPLOY_KEY`. `progress.yml` checks `main` out with that key
(`ssh-key:`) and pushes over SSH. `permissions:` is `contents: read` (the
token is not used to write). If the push is refused (the key or the secret is
missing, or the ruleset changed), the job ends with an error and a job summary
that says what to allow; the progress files then simply stay as they are until
it is fixed. The deploy key can push to any branch it is allowed to; the
workflow only pushes `HEAD:main`, runs only on `push` to `main` (never on a
pull request, so no pull request's code ever sees the key) and the key is
used for nothing else.

## Consequences

- Landing a pull request no longer makes another conflict in hand-off records,
  the README's generated parts, the graphics or the ADR index. Merging `main`
  into an old pull request that regenerated them conflicts once: take main's
  version of those parts.
- Pull requests opened before this lands and that contain regenerated files fail
  the new check until they restore them.
- The progress section is as old as the last `Regenerate progress` commit (a few
  minutes after a merge), not as new as the pull request.
- Two `main` pushes in a row make two workflow runs; they queue (one at a time,
  the newest waiting), and each regenerates from the latest `main`.

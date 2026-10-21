# 0086: The pipeline loop keeps its PRs mergeable: it merges main before the first push and whenever GitHub reports a PR dirty

**Status:** Accepted. Amends [0056](0056-pipeline-scheduler-loop-and-home-server-deployment.md)
(the `refresh` step: main merged into a conflicting PR, then verify again).
The workflow is in [AGENT_PIPELINE.md](../AGENT_PIPELINE.md), "One cycle
per entry".

## Context

The loop's first PR (#28, `pipeline/impl/open`) sat `dirty` on GitHub: it
conflicted with main in README.md's generated progress section only. A PR
that conflicts gets no CI run, so auto-merge never fires; the coordinator
fixed it by hand (merge main, take either side of README.md, `cargo xtask
coverage readme`, commit, push).

This is the normal case, not an accident. Every coverage change on main
regenerates README.md and `docs/progress/`, and every pipeline PR changes
them too, so each merged pipeline PR makes every other open one conflict.
ADR 0056's `refresh` handled conflicts, but only after a poll noticed them,
through `gh pr view` (GraphQL, which not every token may use), and then sent
the entry back through `verify`: `cargo xtask ci` and the diff run again on
the runner before the push, a long window in which main moves again.

## Decision

**Publish merges main first.** Before the status commit and the push, the
publish step merges `origin/main` into the branch (a merge commit, never a
rebase: the hand-off log cites the spec commit by its hash), so a PR starts
out mergeable. The whole PR is then checked as before (`check_pr` against
the new merge base, `coverage check`, `readme --check`) and pushed to the
hub and GitHub without force.

**Only generated and append-only files are resolved by the loop.**

| Conflict in | Resolution |
|---|---|
| `README.md`, `docs/progress/*` | main's version, then `cargo xtask coverage readme` |
| the entry's coverage file | main's version, if main did not change this entry's own object; the publish step then sets the entry's status again (`coverage set-status <id> green --tests-visible V --tests-held-out H`, then `coverage readme`) in the usual status commit |
| `specs/HANDOFF_LOG.md` | main's rows, then the branch's rows main lacks (the log is append-only; `check_pr` checks the result) |
| anything else, or main changed this entry's coverage object | the merge is aborted and the entry goes to the human queue with the files named |

`coverage readme` runs after every merge, conflicts or not: a textual merge
of a generated section is not what the generator makes of the merged map.
Coverage files are canonical JSON with one object per line, so "main's
version plus this entry's status set again" is the coverage tool's own
write, not a hand-made union.

**A PR GitHub reports dirty goes through publish again.** `awaiting-merge`
reads the PR through the REST API (`gh api repos/{owner}/{repo}/pulls?head=…`
for the newest PR of the branch, then `…/pulls/N` for `mergeable` and
`mergeable_state`). `mergeable_state` `dirty` sends the entry to `refresh`,
which counts it against `max_refresh` and continues with publish (which
merges main). `mergeable: null` (GitHub still computing; asking for the PR
starts the computation) is treated like a mergeable PR: the next look is
`poll_secs` later, never at once. `max_refresh` defaults to 10 (was 3): with
`max_open_prs` PRs open, the last one to merge can conflict once per PR
merged before it.

**The runner does not verify again after a merge of main.** The PR's CI on
GitHub runs `cargo xtask ci` on the merge and is the required check for
auto-merge; its `pipeline polycheck` runs the visible tests of every green
entry, this one included (the branch marks it green). The held-out tests
ran green on the branch before the merge; main's own changes went through
their own CI.

## Consequences

- Pipeline PRs merge without the owner, however often main moves, unless a
  conflict touches real code or this entry's coverage object.
- A change from main that breaks only this entry's held-out tests is not
  caught on the PR (CI has no held-out store). Verifying again on the runner
  would cost a CI run and a diff run per merge of main, and keep the PR out
  of main long enough to conflict again.
- The PR branch gains one merge commit per merge of main. GitHub needs no
  force push and no branch deletion; the loop does neither.
- A commit pushed to a pipeline branch by hand (as for #28) is not in the
  hub: the loop's next push of that branch is refused as not a fast-forward
  and the entry goes to the human queue. Let the loop do the merge.
- No GraphQL is needed to watch a PR; creating it and enabling auto-merge
  still use `gh pr create` and `gh pr merge --auto`.

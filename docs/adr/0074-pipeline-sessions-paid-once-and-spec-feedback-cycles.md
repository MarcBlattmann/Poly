# 0074: Pipeline agent sessions are recorded and never paid for twice; failed spec checks go back to the spec agent

**Status:** Accepted. Amends [0056](0056-pipeline-scheduler-loop-and-home-server-deployment.md)
(how the loop resumes the spec and implement steps, and what happens when
the spec check fails). The workflow is in [AGENT_PIPELINE.md](../AGENT_PIPELINE.md).

## Context

The loop's first runs on the home server showed two weak spots of ADR 0056.

1. **An infrastructure error after a successful agent session paid for the
   session again.** In the spec step, the fetch of the agent's branch from
   the role checkout failed once (git's "dubious ownership"). The error
   ended the step before the fetched head was kept in the state, so the
   retry synced the role checkout again, which resets the branch to the
   claim commit and discards the agent's commit, and started a new
   session (about 15 minutes), whose `poly-role spec run` also removed the
   generated and held-out tests of the first one. Up to `max_errors`
   sessions per entry could be burned this way; the implement step had the
   same pattern. ADR 0056 said "an agent session cut by a crash simply runs
   again", which was meant for a crash during the session but also covered
   every failure after it.
2. **A spec check that failed went straight to the human queue.** The
   implementer gets `max_cycles` sessions with redacted feedback; the spec
   agent got none, although a visible test that does not build, or a
   `tests split` it forgot, is the kind of mistake it fixes in one more
   session.

## Decision

**A session is recorded in the entry's state, and the steps after it
resume from the record.** Once the role checkout is synced, before the
agent starts, the state gets `session` with the branch head the session
starts from; when the session ends, its exit status is added and saved
before anything else runs. A retry of the step (after an error, a crash or
a reboot) that finds a session in the state does not sync the checkout and
does not start the agent: it fetches the role's branch first. A commit
beyond the recorded start is the session's result (pushed to the hub as
before); an ended session without one is "committed nothing" or "failed",
counted as before. Only a session the loop never saw end, and that left no
commit, runs again, from a clean checkout. A role paused by a usage limit
does not hold up an entry that only needs that fetch. A session cut by a
usage limit is not recorded: it runs again after the pause, as before.

**Failed spec checks get `spec_cycles` feedback sessions** (`[loop]
spec_cycles`, default 2; 0 keeps the old behaviour). A red spec check (CI
fails, or the sheet, the visible tests or the held-out tests are missing)
writes feedback into the spec agent's own inbox,
`/srv/poly/spec-inbox/<entry>/feedback-<n>.txt` (`[paths] spec_inbox`), and
sends the entry back to the spec step; the next failed check after
`spec_cycles` of them is the human queue. Red flags (paths outside the
entry's spec sheet and tests) stay the human queue at once.

- The feedback session **continues** the last one: the checkout is synced
  to the spec branch as the loop pushed it, the wrapper keeps the entry's
  generated and held-out tests when `--feedback` is given (a new session
  still starts from nothing), and the prompt tells the agent to fix the
  problem, re-split with `--replace` if it changes tests, and commit on
  top. The spec check then checks the whole branch from the claim again.
- The feedback is CI's last 80 lines, **not redacted**. The redaction of
  implementer feedback exists because CLEAN_ROOM.md hides the held-out
  tests from the implementer; the spec agent wrote them and reads the
  held-out store, and CI of a spec branch runs without the store, so there
  is nothing to hide from it. Redacting would hurt: it withholds every line
  that contains a line of a held-out test, and the tests share lines
  (includes, `test_main`) with the visible ones whose build errors the
  agent has to see.
- The spec inbox is a new directory, `server:polyspec 2750`: the loop
  writes, the spec role reads, the implementer cannot (setup.sh checks it).
  The wrapper takes a role's feedback only from that role's inbox, so
  neither role can be handed the other's. The configuration refuses a
  `spec_inbox` that overlaps the implementer's inbox or checkout.

`spec_attempts` still bounds spec sessions that fail or commit nothing, so
an entry gets at most `spec_attempts + spec_cycles` spec sessions.

## Consequences

- An infrastructure failure after a session costs a retry of the fetch,
  never a second session, and never the generated or held-out tests of a
  session that worked. The behaviour is pinned by scheduler tests: a fetch
  failing after a spec session and after an implementer session (no second
  session, the commit kept, one sync, the tests removed only once), a crash
  while the session runs (it runs again) and after it (it does not).
- A session the loop never saw end but that left a commit (the loop was
  stopped while the agent was committing) is taken as it is; the spec check
  or the verify step judges it like any other result.
- A spec check that fails costs at most `spec_cycles` more spec sessions
  before the owner sees it. Entries already in the human queue for a red
  spec check stay there; `pipeline loop reset` starts them again from
  scratch.
- The deployment changes (the wrapper, the spec inbox) take effect when
  `setup.sh` runs again. Until then the spec feedback cannot be written (no
  `/srv/poly/spec-inbox`: an infrastructure error, retried up to
  `max_errors` times), and the old wrapper would refuse `--feedback` for
  the spec role, so such an entry still ends in the human queue, only
  later.

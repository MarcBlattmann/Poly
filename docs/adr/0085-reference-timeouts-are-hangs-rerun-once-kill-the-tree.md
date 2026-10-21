# 0085: A reference run that times out is a hang: run it once more, and kill the whole tree

**Status:** Accepted. Amends the test-run protocol of
[ADR 0027](0027-agent-pipeline-tools-and-test-run-protocol.md). The workflow
is in [AGENT_PIPELINE.md](../AGENT_PIPELINE.md#diff-runner).

## Context

`cargo xtask ci pipeline` (polycheck) failed three times, and passed when
run again, with

    b_ioctl_ctty  ERROR  timeout / exit 0  -  the reference run timed out (a broken test, or the tracer)

on a loaded machine (4 CPUs, polycheck's four jobs with a QEMU each, several
CI runs at once). The reference is real Linux under our ptrace tracer.

**It is not a tight timeout.** The test normally takes 33 ms under the
tracer (30 ms of it a `sleep_ms(30)`); the limit is 10 s. In 950 runs under
CPU load (8 to 16 busy loops, four tracers at a time) the slowest took
274 ms and none timed out.

**It is a hang in the test, a race with the kernel.** The reference trace of
a failed CI run shows it. The parent writes the interrupt character to the
pty master, writes `g` to the child, sleeps 30 ms and closes the master. The
child reads `g` and spins on `sched_yield` until its SIGINT handler has run.
The SIGINT never came: the trace has the child's SIGHUP and SIGCONT from the
hang-up, no SIGINT, and 426,322 `sched_yield` calls until the watchdog killed
it at 10 s. Linux hands bytes written to a pty master to the line discipline
in a kernel worker (the tty flip-buffer work), not in `write`. That run's
host was stalled badly (one of the child's `ioctl`s started 280 ms after the
previous call returned), so the worker had not run when the master was
closed 30 ms later. The hang-up clears the terminal's foreground group,
so the late `^C` signals nobody. Closing the master right after the write
loses the SIGINT 9 times in 30 without added load (a stand-alone program); a 10 µs
gap is already enough, which is why only a stall of the whole host loses it
at 30 ms. Whether the stall is CPU steal on the VM, memory reclaim or
anything else does not matter: no timeout is long enough for a test that
waits forever.

So the cause is in the test's synchronisation (a 30 ms sleep standing in for
"the child has seen SIGINT"), not in the tracer: no job-control stop,
SIGTTOU/SIGTTIN or tracer race is involved. The test belongs to the spec
agent (CLEAN_ROOM.md) and the tooling may not change what it checks.

**The timeout also leaked the hung child, outside the ptrace tracer.**
`runner::run_command` (untraced and strace runs, QEMU) killed only the
process it started. The child of a hung `b_ioctl_ctty` has a session of its
own, so it lived on, kept the output pipes open, and the runner waited for
them for as long as it lived. On this machine one such child from a manual
run had spun for 14 hours at 80% of a CPU, adding to the very load that makes
the race lose. (The ptrace tracer was already right: it kills every tracee.)

## Decision

1. **A reference run that times out is run once more**
   (`runner::rerun_on_timeout`, `TIMEOUT_RERUNS` = 1), for test programs and
   app tests. The reference is the real OS and always finishes, so a timeout
   is a test that hangs on a race; a race lost under load is very likely won
   on the next run, and a test that hangs every time still times out twice
   and stays an ERROR ("timed out 2 times in a row"). The timeout is not
   raised or scaled with the load: a hang outlasts any limit, and a larger
   one only makes every real hang slower to report. A rerun costs time only
   when a run has already timed out.
2. **A rerun is never silent.** `summary.json` records
   `reference_timeouts` per test, and the report's last lines name the tests
   whose reference was run again, so the test can still be fixed.
3. **A timeout kills the whole process tree** (`runner::kill_tree`): every
   process descended from the one started, found through `/proc` one
   generation at a time and stopped before any is killed, so nothing forks
   past the kill and no pid is reused. The output is waited for until the
   deadline plus one second at most, so a process outside the tree that
   still holds the pipes cannot hang the runner either.
4. **`b_ioctl_ctty` is reported to its spec agent (spec-b)** to wait for the
   child's acknowledgement of SIGINT (a byte on a pipe) instead of
   sleeping 30 ms before closing the master. Until then the rerun keeps CI
   green.

## Consequences

- A test that hangs only under load no longer blocks a merge; one that
  hangs every time is reported as before, 10 s later.
- `a_timer_delete_pending` (exit 1 on Poly once) and the epoll timing test
  failed on the target (Poly under QEMU), not on the reference, so this does
  not touch them: the rerun is only for the reference. They did not
  reproduce here.
- Untraced and strace runs, and QEMU runs, leave no processes behind on
  timeout.

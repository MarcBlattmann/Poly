# 0079: A test program runs as the reference's user on both sides, with umask 022 in `/`; tests that need root say so

**Status:** Accepted. Amends the test-run protocol of
[ADR 0027](0027-agent-pipeline-tools-and-test-run-protocol.md) and the test
init of [ADR 0064](0064-test-runs-as-a-tracer-sees-them-and-spec-a-behaviours.md).
The workflow is in [AGENT_PIPELINE.md](../AGENT_PIPELINE.md#diff-runner).

## Context

The home server's first real loop run compared every entry's tests in two
different environments before any behaviour was compared. The diff runner
there is the systemd service of user `server` (uid 1000) with `UMask=0027`;
its reference runs inherited both. Poly's test init started the program as
root with umask 022. So a test calling `umask` diverged at its first event
(`reference umask(0x12) = 23, target umask(0x12) = 18`), one calling
`getuid` too (`1000` against `0`), and permission checks differed (a test
expecting `EACCES` for a user other than root failed on Poly). In this
container the runner is root with umask 022 and nothing showed; on GitHub's
CI (a normal user) `polycheck` papered over it by accepting any failing
green-entry test that passed its own checks on Poly.

Both sides could be forced into one fixed identity instead (always root,
or always a fixed uid): the reference then needs root to change users, or a
user namespace, which the CI runner and the home server do not grant the
diff runner, and a fixed root reference hides every rule that applies to
other users.

## Decision

1. **The reference's environment is fixed where it can be.** The tracer's
   child, the strace run and the untraced run start the program with
   umask 022 (Linux's usual default), in `/`, with an empty environment
   and only fds 0-2: as Poly's test init starts it.
2. **The identity is whoever runs the tool, on both sides.** The diff
   runner records its own real, effective and saved user and group ids and
   supplementary groups (`identity.rs`, `summary.json` `identity`) and
   boots Poly with `poly.test.uid=R[,E,S] poly.test.gid=R[,E,S]
   poly.test.groups=G1,... poly.test.umask=0022`. The kernel's test init
   makes the credentials as root would make them for a program
   (setgroups, setresgid, setresuid: `Creds::login` in the Linux
   personality), so a user other than root has no capabilities, gives
   them and the umask to the program before its first instruction (also
   the ids of its auxiliary vector), and gives the same credentials to
   the init, which stands for the tracer, the program's parent on Linux.
   Without the arguments the program runs as root as before. Bad
   arguments make the run unsupported (`POLY-INIT: unsupported`). The
   command line must fit Poly's 511 bytes; the runner refuses a longer
   one. App tests stay root on both sides.
3. **Tests that need root say so.** A test that only makes sense as root
   carries `poly-diff: needs=root`. Where the reference is not root, `pipeline
   diff --target poly` does not compare it but runs it on Poly as root and
   passes it if it passes its own checks; Linux against Linux compares it
   as usual. Spec agents are told (specs/README.md, the role prompt) that
   tests run as any user.
4. **Green entries recorded as root keep CI green.** `polycheck` gives the
   same root self-check to a green entry's test that differs where the
   reference is not root, and names it: as "needs root" when the
   reference failed its own checks, as a warning when the reference passed
   and Poly differed (a real difference for users other than root).

## Consequences

- The loop's verify on the home server compares like for like: a test
  runs as `server` with umask 022 on Linux and on Poly. Entries whose
  tests failed only because of the environment pass now; the operator
  resets them (`pipeline loop reset <id>`).
- Rules for users other than root are now compared wherever the runner
  is not root, and show differences root hid (today `setgroups` checks
  `EPERM` before `EINVAL` on Linux, `linkat` with `AT_EMPTY_PATH` of a
  file the caller opened needs no capability on Linux 6.18).
- A test's result can depend on the runner's user; a test meant for any
  user must not assume one, and the summary says who ran it.

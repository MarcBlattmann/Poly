# 0076: The pipeline loop updates itself; deploy changes are announced, not applied; narrow sudo for the service

**Status:** Accepted. Amends [0056](0056-pipeline-scheduler-loop-and-home-server-deployment.md)
(how the loop on the home server gets new code, and what `server` may run
as root). The workflow is in [AGENT_PIPELINE.md](../AGENT_PIPELINE.md),
"Updates and service control".

## Context

The loop fast-forwards its own checkout (`/srv/poly/runner/repo`) to
`origin/main` before it picks an entry, and the systemd unit builds and
runs `cargo xtask pipeline loop` from that checkout. A merged change to the
loop itself therefore reached the disk but not the running process: the
owner had to notice it and restart the service by hand, with sudo,
interrupting whatever was in flight. Pausing a role meant writing
`state/pause-<role>.json` by hand. And a change to the deploy files (the
role wrapper, `setup.sh`, which writes the sudoers rule and the unit) went
unnoticed unless the owner read the history.

## Decision

**The loop restarts itself for its own code, only once the new code
builds, and only between steps.** After every fast-forward the loop asks
git which paths differ from the commit it was started from. Paths that make
up the loop's binaries (`tools/pipeline/` except `deploy/`, `tools/xtask/`,
the workspace `Cargo.toml` and `Cargo.lock`, `rust-toolchain.toml`,
`.cargo/config.toml`) mean a restart. The loop first builds the new code
in the checkout, exactly as `cargo xtask` will (`xtask`, then
`poly-pipeline`, each on its own so features unify the same way, without
rustup's `RUSTUP_TOOLCHAIN` so the new toolchain pin applies). When the
build passes it picks nothing new, lets each worker finish the step it is
in (every step is resumable from the saved state, and an agent session is
not cut short), and exits with status 10, which the unit lists in
`SuccessExitStatus=`; `Restart=always` starts the new code. When the build
fails, the loop does not exit: a broken main cannot make systemd spin. It
keeps running its old code, tells the owner once (log, human queue,
notify command, `loop status`) and builds again when main moves.

The crates the binaries link and the files they embed (the syscall table
from the coverage map) are deliberately not restart paths: the loop
process never runs that code, the tools that do run from the runner's
worktrees and are built from the branch each time, and the coverage map
changes with every merged entry.

**Deploy files are never applied by the loop, but it says when they
changed.** They need root. `setup.sh` records the SHA-256 fingerprint of
what it installed (`/srv/poly/bin/deployed.sha256`, the same function in
bash and Rust, checked against each other by a test). While the runner's
checkout has other deploy files, `loop status` says `deploy files changed:
run sudo .../setup.sh`, and the loop tells the owner once per change.
Content, not history, decides: a notice survives restarts, also appears
when the loop was down during the change, and goes away once `setup.sh`
has run, whichever checkout it ran from. The example configuration is not
a deploy file (`setup.sh` never overwrites `pipeline.toml`).

**`server` may control the service without a password, and nothing else
as root.** The sudoers rule `setup.sh` writes allows exactly
`/usr/bin/systemctl start|stop|restart|status poly-pipeline` (sudo
matches the arguments exactly). `status` is left out on systemd older than
247, whose pager would run as root without secure mode under sudo
(CVE-2023-26604).

**`loop pause` and `loop resume`** write and remove the pause files the
cycle already reads; a pause without `--for` lasts until `loop resume`, and
a usage limit hit during a longer pause does not shorten it.

`cargo xtask pipeline` now passes the tool's exit status on (it turned
every failure into 1, which also hid the usage-limit status 75 of
`pipeline role` from the loop).

## Consequences

- Merged pipeline changes go live within one pass of the loop plus the
  current steps, without the owner. A restart can wait for an agent
  session in progress (up to its own time limit).
- After a merge that changes `tools/pipeline/deploy/`, the owner runs
  `setup.sh` once; the loop says so. The first time, that includes this
  change itself: the stamp, the service rule and `SuccessExitStatus=` come
  from it.
- Self-update is off for `--once`, `--dry-run`, a loop not run from
  `paths.repo`, and with `loop.self_update = false`.

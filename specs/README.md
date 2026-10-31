# Specs

Behaviour specifications written by the **spec agent** from observing the
real OS, one file per entry in the coverage map (`coverage/`):

```
specs/<os>/<component>/<symbol>.yaml      spec sheet (see TEMPLATE.yaml)
specs/<os>/<component>/<symbol>.tests/    visible tests (~70%)
```

Linux tests are freestanding C programs (`tests/linux/include/`); Windows
tests are `no_std` Rust files (`tests/windows/support`, ADR 0075) or
freestanding C++ files (`tests/windows/cxx`, ADR 0098) built by `cargo
xtask pipeline win build` and run on a real Windows PC. A Windows entry's
`<symbol>.tests/windows.json` holds the recorded Windows results of its
visible tests (`tests split --windows-results`).

**Who a Linux test runs as.** A test runs as whatever user runs the
pipeline: root in some environments, a normal user in others (the home
server's runner, GitHub's CI), with the same user and groups on Poly
(ADR 0081). It always starts with umask 022, in `/`, with an empty
environment and only fds 0-2 open. So a test must pass as any user: do not
assume uid 0, a particular uid, or that permission checks are bypassed, and
use absolute paths (under `/tmp`). A test that only makes sense as root
(mount, set*id to other users, capabilities, root passing a permission
check) says so in its header comment, on a line of its own:

```c
/* mount(2) of a tmpfs: flags and statfs.
 * poly-diff: needs=root (only root may mount)
 */
```

Where the reference is not root, such a test is not compared: it runs on
Poly as root alone and passes if it passes its own checks. (Other
directives: `poly-diff: syscalls=off` or `names` for tests whose syscall
order depends on scheduling; see `docs/AGENT_PIPELINE.md`.)

Held-out tests (~30%) are **not** stored in this repository. They live in
the held-out store (`$POLY_HELDOUT_DIR`, default `~/.poly-heldout`) with the
diff runner, so implementer agents can never see them. See `../CLEAN_ROOM.md`.

Every hand-off from spec agent to implementer is appended to `HANDOFF_LOG.md`.

The tools for all of this are `cargo xtask pipeline spec new`, `tests split`
and `handoff record|bundle|verify`; see `../docs/AGENT_PIPELINE.md`.

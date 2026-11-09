# 0156: CI builds once, boots many times

**Status:** Accepted. Refines ADR 0150 (CI in jobs of equal length).

## Context

After ADR 0150 a pull request's CI took about 7.5 minutes (run 42599765798).
Every boot and pipeline job spent about 35 s on setup and then 100 s
building the same things: the native programs (70 s) and the kernel (100 s,
both on the same four cores), the traced 7-Zip scenario's kit and driver
(30 s), the pipeline tool (70 s in the pipeline jobs), the corpus (55 s,
one compiler after the other), and `cargo xtask build` once more inside
`polycheck`. The pole, `wintrace`, was 373 s: 114 s of that building, and
one 259 s boot.

## Decision

- **Two `build` jobs** (`cargo xtask prebuild kernel|tools`) make what the
  other jobs boot and upload it as artifacts (34 MB and 28 MB): `kernel`
  the kernel ELF; `tools` the native programs, xtask itself, the pipeline
  tool, the Linux hello programs, and the Windows programs of `wintrace`,
  `7zip` and `desktop-win32` with the trace kit. The boot and pipeline jobs
  unpack them (`.github/actions/prebuilt`) and run with `POLY_PREBUILT=1`:
  `tools/xtask/src/prebuilt.rs` makes each build step whose outputs are
  among the prebuilt files say so and skip, and runs the pipeline tool from
  its binary. Each artifact has a stamp with the commit it was built from;
  a job refuses files of another commit. Without the variable nothing
  changes. The boot jobs need no Rust at all; only the `extras` pipeline job
  (win polycheck builds Windows programs) installs it, in the background
  while apt installs.
- Both build jobs compare checksums, not modification times, in cargo
  (`CARGO_UNSTABLE_CHECKSUM_FRESHNESS`), so the 50 crates a fresh checkout
  touches are not rebuilt from the cache (40 s).
- **The traced 7-Zip scenario runs on TCG.** The 15 steps take seconds, but
  the observation file (8.2 MB of trace) leaves the guest as hex on the
  serial port, one port I/O exit per byte: 50 KB/s through a hypervisor
  inside a virtual machine (GitHub's runners), 3 minutes. QEMU's emulator
  does the same in 15 s (measured on a runner: 8.2 MB in 12 s). Unless
  `POLY_QEMU_ACCEL` names an accelerator, the scenario boots on TCG. The
  comparison with Windows' trace, `TRACE_REQUIRED` and the driver (whose
  SHA-256 the reference records) are untouched.
- The 7-Zip boot test (`7zip`, one boot per step) is two groups, `7zip` and
  `7zip-b`, every second step each.
- Boot shards are cut finer (eleven) and combined into seven jobs of 80 to
  105 s of boots; `polycheck` runs in four slices and the sliceless checks
  (`selfcheck`, win polycheck) in an `extras` job (`ci pipeline --shard
  extras`), so no job's tests take more than about two minutes. The corpus
  is compiled on every core. `polycheck --build-kernel` no longer builds
  xtask to find the kernel up to date.
- `ci lint` checks that the workflow has the build parts, the shards, the
  slices and the extras.

## Not done

- The boot jobs could start their setup while the build runs (they would
  poll for the artifact), saving 25 s. Jobs waiting for a job that is itself
  queued can deadlock when the repository's 20 concurrent jobs are taken
  (several pull requests at once), so they wait for `needs`.
- A faster serial path in the kernel (`rep outsb` bursts of 16 bytes were
  tried: 38 to 52 KB/s through a nested hypervisor) or a different way to
  hand a test's output file to the host would speed up every boot test.

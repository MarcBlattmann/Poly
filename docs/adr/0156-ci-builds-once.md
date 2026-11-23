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

## Addendum (2026-11-23): slices that fit again, no Rust in the Chrome jobs, scopes for pull requests

By November a pull request's CI took 20 to 23 minutes (run 45156193996 on
`main`): `pipeline (extras)` ran win polycheck's 1 790 tests one after the
other (22 minutes: 3.5 minutes of cargo for the Windows test programs, then
17.5 of boots), `boot (userland)` held `macho-specs` (832 boots, 11.5
minutes), and `chrome (s3-window)` three window checks (10 minutes). Every
`chrome` job also installed Rust and built `ntdll.dll`, the Win32 modules and
the launcher (about 95 s) before it booted.

- **win polycheck in slices.** `pipeline win polycheck --shard I/N` builds and
  checks only the tests whose name hashes (FNV-1a) to slice I, the
  vcruntime140 parity pass included, so every test is in exactly one slice and
  stays there when others are added. CI runs four (`ci pipeline --shard
  win-I/4`, xtask's `WIN_PIPELINE_SHARDS`) beside polycheck's four (`I/4`,
  with selfcheck in the first). The `extras` slice is gone. Each `win-*`
  slice keeps its cargo target directory in a cache that `main` saves, and
  cargo compares checksums there (`CARGO_UNSTABLE_CHECKSUM_FRESHNESS`), so a
  pull request rebuilds only the test programs it changed.
- **`macho-specs` per architecture.** The group is two, `macho-specs`
  (x86-64) and `macho-specs-a64`, each its own shard and job (the ratchet
  file is read per architecture); `boot-tests macho-specs` still runs both.
- **The window checks in jobs of their own.** `chrome-try` and
  `chrome-try-max` left the `chrome-win-window` shard for shards and `chrome`
  jobs of their own (`s3-try`, `s3-try-max`). `x11` and `desktop-shell`
  share a job.
- **No Rust in the `chrome` jobs.** `prebuild tools` also builds Chrome's
  launcher and probe and the desktop's Windows GUI test program
  (`winapp::prebuild`), and `prebuild kernel` packs the caches of the modules
  the kernel embeds (`target/ntdll-cache`, `target/win32-cache`,
  `target/wow64-cache`), so `polystub.dll` is made from their exports without
  building them. `winapp::build` takes the prebuilt programs in prebuilt mode.
  The boot jobs that built them (`desktop`, `desktop-shell`, `sevenzip-b`) no
  longer do either.
- **Scopes for pull requests.** `changes` lists the pull request's files
  (a renamed file with its old path too; a list of 3 000 files or none means
  everything) and sets four scopes: `host` (anything but prose), `heavy`
  (anything but prose and `tools/coverage/`: build, boot, chrome),
  `pipeline` (anything but those and `crates/poly-pkg`, `crates/poly-files`,
  `userland/poly-files-app`, `userland/poly-installer-app`, which neither the
  kernel nor the pipeline tool nor its test programs use) and `aarch64` (the
  kernel tree, the crates, xtask, the toolchain, Cargo's files, the
  workflows). A push to `main` and a manual run set them all. The `ci` job
  lets a job be skipped only when `changes` succeeded and that job's scope is
  false; any other result but success fails it.

Not done: a narrower scope for the desktop crates (only the `apps` boot job)
was left out, because xtask itself uses `poly-pkg` and `poly-files` (the
`chromium` and `linux-installer-2` groups) and the native programs they make
are on every desktop the boot and chrome jobs start. The `chrome` jobs still
spend 1.5 to 2 minutes in `chrome-win fetch`: a debug build of xtask checks
and inflates the 200 MiB archive (moving SHA-256 and inflate into a crate
built with optimizations, as `poly-coverage` is, would take that to seconds).

# 0150: CI in jobs of equal length

**Status:** Accepted. Refines ADR 0077 (CI in parallel jobs).

## Context

A pull request's CI took 15 to 27 minutes. The jobs run in parallel, so
the slowest one is the wall-clock time, and measured over the runs of
2026-11-05 and 2026-11-06 two jobs were far slower than the rest:

- `boot (userland)`: 12 to 27 minutes. Its boots take 10 to 12 minutes
  (the traced 7-Zip scenario, `wintrace`, alone 3 to 4); the rest was the
  apt installs (up to 7 minutes when the mirror is slow) and the build.
- `pipeline`: 8 to 19 minutes, 4 to 5 of them `polycheck`'s one QEMU boot
  per corpus test.

Every other job took 3 to 4 minutes, of which the boots were a minute or
less: the time of a boot job is mostly the two minutes it needs to build
the Win32 modules (48 s), the native programs (56 s, beside them) and the
kernel (83 s), and about 50 seconds to install QEMU, Rust and the cache.
That build is the same in every job, and does not get faster by sharing it
(a job that builds and uploads, with the others downloading, takes as long
as each building for itself).

## Decision

- Cut the work so every job takes about the same, three to four minutes of
  boots (`polycheck`) or less on top of the build:
  - `CI_BOOT_SHARDS` in xtask has seven small shards (selftest, userland,
    wintrace, desktop, x11, devices, faults); `.github/workflows/ci.yml`
    puts them into the boot jobs (`core` runs four of them), and `ci lint`
    checks that every shard is in exactly one job. `--shard` takes a comma
    list. The TCG self-test job is unchanged.
  - `polycheck --shard I/N` runs the I-th of N slices of the corpus (the
    sorted test names, every N-th one) and checks the pass list and the
    green entries' tests that are in it; `ci pipeline --shard I/N` runs a
    slice, the first also `selfcheck` and `win polycheck`. Three slices.
- Cache what is the same on every run:
  - `target/win32-cache` like `target/ntdll-cache`, under a hash of the
    inputs (`cargo xtask win32 --cache-key`): saves 48 s of the build in
    every job.
  - The apt packages, as .deb files in a cache keyed by the runner image
    and the package list (`.github/actions/apt-install`): a hit installs
    with dpkg and no network, so a slow mirror cannot hold a job up.
- A pull request that only changes prose (ADRs, root and `docs/` Markdown
  files, the generated charts) skips the host tests, boot tests and
  pipeline; `lint` and `coverage` always run. The required check `ci`
  counts those skips as passed only then, and still fails for a failed or
  cancelled job.
- Each boot job's summary lists the time of the build and of every group
  (`$GITHUB_STEP_SUMMARY`), and `boot-tests` prints each group's time as it
  ends, so the shards can be rebalanced from the next run's numbers.

Nothing runs less: every boot test group is in exactly one shard, every
corpus test in exactly one slice, TCG and KVM boots as before, and the
tests that skip (Chromium, toolkits, virgl) skip as before.

## Consequences

- Job names changed: `boot (<job>, <accel>)` with the jobs `core`,
  `userland`, `desktop`, `wintrace` and `selftest` (TCG), and
  `pipeline (<i>/3, ...)`. The required check is `ci`, unchanged.
- More, shorter jobs: 13 instead of 10 per run, about the same minutes of
  runner time in total (the build is paid once per job).
- When a group gets slow, move it to another shard in `CI_BOOT_SHARDS` and
  the workflow's `shards:` lines; `ci lint` tells if one is lost.

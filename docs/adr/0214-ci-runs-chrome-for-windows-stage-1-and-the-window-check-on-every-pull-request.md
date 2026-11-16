# 0214: CI runs Chrome for Windows, stage 1 and the window check, on every pull request

**Status:** Accepted. Tooling (track `chrome-ci`). Builds on ADR 0155 (Chrome
for Windows on Poly), ADR 0156 (the build jobs and prebuilt files), ADR 0194
(MinGit's cache, the pattern copied here), ADR 0207 and ADR 0209 (the window check).
Inputs: GitHub's documentation of Actions caches and workflow commands. Chrome
stays a black box (CLEAN_ROOM.md): nothing of it is read, it runs.

## What was there

The boot test group `chrome-win` (stage 1: headless `--dump-dom`, required
since ADR 0155 step 13; then the stage 3 window check) skipped itself in CI
because Chrome for Testing was never downloaded there. Every agent that
touched the NT personality booted Chrome on the owner's one PC to see whether
it still ran, and a pull request that broke Chrome was found only afterwards.

## Decisions

1. **CI downloads Chrome for Testing**, in its own job, with
   `cargo xtask chrome-win fetch` (the pinned version in `chromewin.rs`: URL, size and SHA-256
   checked, unpacked by the pure-Rust ZIP reader). It comes from Google's public
   bucket (`storage.googleapis.com/chrome-for-testing-public`) at job time.
   **It is never committed, never published as an artifact, never released**:
   the only copy kept is the archive in this repository's Actions cache, keyed by
   version (`chrome-win-<version>`, checked by `ci lint` against `chromewin::VERSION`).
   Only the archive is cached (210 MB), because xtask checks it against the
   pinned SHA-256 on every run: a bad cache entry is never used. `main`
   saves the entry; a pull request restores main's (caches of `main` are
   readable by pull requests, those of a pull request are not shared) or
   downloads it once. A new version in `chromewin.rs` means a new key.
2. **The `chrome` job runs the group in two parallel matrix entries,** each a
   boot-test shard (`CI_BOOT_SHARDS`), so Chrome adds no more than one entry's time to a
   pull request, next to the other jobs, and a flaky window check can never
   hide stage 1:
   - `s1-headless`: shard `chrome-win`, stage 1 (`--dump-dom` of the spec's page `s1.html`,
     stdout compared with the SHA-256 the Windows oracle recorded). **Required.**
   - `s3-window`: shard `chrome-win-window`, the stage 3 check: Chrome's window
     on the desktop, QMP clicks, the green page painted at (200, 400), a URL
     typed into the address field (ADR 0209), the blue page painted.
   The group `chrome-win` used to end with this check; it is now the group
   `chrome-win-window` (locally `cargo xtask boot-tests chrome-win` still runs both).
   Both stages use `data:` pages: **the CI stages need no network.** Stage 4
   (example.com, iana.org) is not run in CI, and a later stage that needs
   a network page must serve it from the runner.
3. **A missing Chrome is a failure, not a skip.** The job sets
   `POLY_CHROME_REQUIRED=1`; with it the groups fail when the package is not
   fetched, so a failed download can no longer pass as "skipped". The job
   is part of the aggregate `ci` check (skipped, like the boot jobs, only when a pull
   request changes prose alone). `ci lint` checks that the workflow fetches
   Chrome into a cache keyed by the version, sets the variable and that `ci` needs the job.
4. **A failure names its stage and links the log.** The job's name is the stage
   (`chrome (s3-window)`); xtask prints a workflow error annotation
   (`Chrome for Windows, stage N (...) failed`, with the run's address) on a failed stage; the
   job summary says which stage failed and links the artifact with the serial logs.
   **Only logs are uploaded** (`target/chrome-win/*.log`, `target/win-app/s3-boot/*.log`),
   never screenshots or pixels of a page.
5. **The job is built like the boot job:** the kernel and the native programs
   come from the `build` jobs (prebuilt, ADR 0156); the Windows launcher and probe
   (small Rust programs) are built in the job, with Rust installed while the packages come in
   (the way the pipeline's `extras` slice does it).
6. **Stage 2 (sandbox) and the GPU stages are not run yet.** Each joins when it
   is on `main` and passes reliably: a stage is one more entry of the matrix
   and one more shard, nothing else changes. (Stage 3 is the window check; ADR 0155's
   sandbox stage, S2, becomes required the same way, with `S1_REQUIRED`'s rule.)

## Flakiness

See "Results" below.

## Consequences

- A pull request that breaks Chrome's start-up or its window fails the check
  of the pull request; nobody has to boot Chrome by hand to find out.
- Time: the `chrome` entries start when the `build` jobs end, like the boot
  jobs, and run next to them.
- The cache of Chrome is 210 MB of the repository's 10 GB Actions cache; it is
  evicted when not used for a week, and fetched again (a download of seconds).
- A Google outage while the cache is cold fails the job (three retries first);
  re-run it.
- Where the window check turns out flaky on the runners, it can be made
  informational (`continue-on-error` on its entry) without touching stage 1.

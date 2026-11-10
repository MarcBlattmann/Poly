# 0174: The agents' build loop: a Linux-side mirror of the checkout, a quick lint and batch coverage edits

**Status:** Accepted. Tooling only: the wrapper the agents build through
(`polyd`, outside this repository), `cargo xtask ci lint`, the coverage tool
and one build profile. Builds on [ADR 0077](0077-ci-in-parallel-jobs-and-qemu-on-kvm.md),
[ADR 0150](0150-ci-in-equal-sized-jobs.md) and [ADR 0156](0156-ci-builds-once.md)
(CI in parallel jobs that build once) and keeps [ADR 0124](0124-native-desktop-alpine-shell-look-and-builds-on-windows-and-linux.md)
(everything builds on a Windows and a Linux host).

## Context

About eight agents work on Poly at once on one Windows 11 PC (32 CPUs,
96 GB). Each has a git worktree on the Windows disk and builds and boots in
a Linux container (`polyd '<command>'`) that mounts the worktree at `/repo`
and keeps `target/` on a Docker volume. The transcripts of 55 agent sessions
(170 h) showed 74% of an agent's time is waiting for a build, a lint or a
test: `cargo xtask ci lint` a median of 10 minutes (CI: 2), a
`cargo xtask coverage` call 2.3 minutes, the boot tests 3.4.

Measured on the loaded machine (Docker VM load 8 to 18), the cause is not
compiling. The Windows worktree reaches the container over 9p, and every
file opened there is scanned by Windows Defender:

| what | time |
|---|---|
| `stat` of the 2383 tracked `.rs` files, in the container | 60 s (25 ms a file) |
| reading them (38 MB) | 39 to 52 s |
| `tar` of the tree's 6748 files natively, in Git Bash | 164 s |
| `git diff --name-only <base>` in the container | more than 10 minutes |
| `cargo xtask ci lint`, cold `target/` | 1170 s |
| the same again, warm `target/` | 1196 s |

A warm lint is no faster than a cold one: cargo's freshness check stats
every source file for each of the lint's twelve cargo runs, rustfmt reads
every file six times, the conflict-marker check reads every tracked file,
and the kernel's, ntdll's and the Win32 modules' own `target/` directories
(inside the tree) are written through the mount. Starting a program on the
Windows side costs 0.3 s and more (`docker` 1.2 to 1.8 s before it does
anything), and the wrapper started ten.

## Decision

### `polyd` builds from a mirror of the checkout on a Linux volume

In **mirror mode** `/repo` is not the Windows worktree but a copy of it on
the Docker volume `poly-snap-<worktree>`, with `target/` and the other
build directories in it. Around each command the wrapper's `snap.py` (in
the container) keeps the copy and the checkout equal:

- **The manifest.** On the host, `git add -A` into an index of the
  wrapper's own (never the worktree's) and `git ls-files -s -z`: the blob
  hash of every tracked and every untracked, not ignored file as the work
  tree has it now. Git hashes only files whose time or size changed, so
  this is two short git runs; the blobs land in the object store.
- **Before the command**, each path is compared three ways, by content:
  the mirror (hashed, with a cache keyed by change time, size and inode),
  the manifest, and `have`, what both had after the last sync. Only the
  checkout changed: the blob is read from the object store (pack files
  read well over 9p) into the mirror. Only the mirror changed (a command
  that was killed before it could copy back): it goes to the checkout.
  Both changed: the checkout wins, the mirror's file is kept aside and
  named. A file written to the mirror gets a new modification time, so
  cargo rebuilds what changed and nothing else.
- **After the command**, what it changed in the mirror goes back to the
  checkout the same way: `cargo fmt`, the coverage map, generated sources,
  new files that are not ignored; deletions too, unless there are more
  than 20 (a wiped mirror must not wipe a checkout). Ignored files stay in
  the mirror.
- The sync never looks under `target/` or into another volume mounted
  inside the tree (the 7-Zip and ntdll caches all worktrees share). The
  first version did, and emptied those two caches until they were put back
  from the copies it had made: now the mounts are read from
  `/proc/self/mountinfo` and the ignore rules are only applied once the
  mirror has the checkout's `.gitignore`.
- A command sees the checkout as it was when the command started. Git
  inside the container runs on the copy and never rewrites the index.
- **A worktree chooses** with `POLYD_SNAP=1` or `0`, and the choice sticks.
  A worktree the wrapper sees for the first time is in mirror mode; one
  that already has a `poly-target-<worktree>` volume stays as it was until
  asked. A spec agent's worktree (`POLYD_SPEC=1`) never gets a manifest or
  a mirror: the manifest puts untracked files' blobs in the object store
  every worktree shares.

### New mirrors start warm, from a template

`polyd-template` builds a mirror of `origin/main` on a volume of its own
(`poly-snap-tpl-<time>`): the tree from the object store, then
`cargo xtask ci lint`, `cargo test --workspace --no-run` and
`cargo xtask prebuild kernel` and `tools`. A worktree's first mirror-mode
run copies that volume. The copy keeps the template's modification times
and the sync then rewrites exactly the files the worktree has differently,
so cargo's ordinary freshness check is right by construction: no
`RUSTC_WRAPPER`, no unstable cargo flag. The next template starts as a
copy of the previous one, so making it is an incremental build; the
previous one is kept while mirrors may still be copying it.

### The wrapper starts fewer programs

- `/dev/kvm` and `/dev/dri` are looked for with one container that sees
  the Docker VM's `/dev`, and the answer is kept for 30 minutes
  (`POLYD_PROBE=1` asks again): before, two `docker run --device` probes on
  every call.
- Paths are converted in bash, not by `cygpath`; `.git` is read with
  `read`; nothing else is started but git (mirror mode) and docker.

### `cargo xtask ci lint --quick`

`ci lint` runs rustfmt's checks beside the three clippy runs, not before
them (rustfmt is one thread per workspace: 13 s for the host workspace).
`ci lint --quick` lints what changed against the merge base with
`origin/main` (committed, modified or untracked): fmt and clippy of the
host packages that have such a file, and of the kernel, the native
programs, the aarch64 stub, ntdll, the Win32 modules and the tracer's
sources only when one of theirs changed; then the source checks, with the
conflict-marker check on the changed files. A change to a file that
configures every build (`Cargo.toml`, `Cargo.lock`, `rust-toolchain.toml`,
`rustfmt.toml`, `clippy.toml`, `.cargo/config.toml`) runs the full lint.
Under `polyd` the changed files come from the manifest compared with
`git ls-tree` of the merge base, not from `git diff`, which reads every
file. The quick lint does not look at the crates that depend on a changed
crate: the full `ci lint`, which CI runs, stays the gate. The planning
(`quicklint.rs`) is host-tested.

### Coverage edits in one call

`coverage claim`, `release` and `set-status` take many entry ids, as
arguments and from `--from-file FILE` (one id per line, `#` comments, `-`
for standard input). A call changes all its entries or none, and names
every id that is wrong, not only the first. The map has 874 000 entries and
is read whole on every call: `poly-coverage`, `serde` and `serde_json` are
built with `opt-level = 3` in the dev profile (`coverage check` 21 s to
6.5 s, `coverage readme --check` 14 s to 4 s).

## Measured

Same machine, same commands, Docker VM load 8 to 19 throughout; "before" is
the old wrapper on the Windows worktree, "after" the mirror.

| command | before | after |
|---|---|---|
| `polyd true` | 10.2 s | 6.0 s (4.7 s without the mirror) |
| `cargo xtask boot-tests --list` | 23.5 s | 5.6 s |
| `cargo xtask ci lint`, nothing built | 1170 s | 181 s |
| `cargo xtask ci lint`, built | 1196 s | 38 s |
| `cargo xtask ci lint --quick` (two crates changed) | none | 11 s |
| `cargo xtask ci coverage` | 243 s | 19 s |
| `cargo xtask coverage show <id>` | 65 s | 10 s |
| claim 150 entries | 150 calls | one call, 13 s |
| `cargo fmt --all --check` | 65 s | 13 s |
| cargo's check that xtask is built | 10 s | 0.3 s |

## Considered and dropped

- **sccache** as `RUSTC_WRAPPER` with a shared volume: it caches no
  incremental compile, and every crate that matters here is a path
  dependency built incrementally; turning incremental builds off to feed
  the cache would slow the loop that is fast today. The template gives a
  new worktree the same warm state with incremental builds kept.
- **Seeding a `target/` volume alone**: a new worktree's files all have
  the time of its checkout, so cargo rebuilds every crate; cargo's
  `-Zchecksum-freshness` (which CI's build jobs use) would fix that but
  reads every file, which is the slow part here.
- **A launcher that runs the built xtask without cargo**: in the mirror
  cargo's own check takes 0.3 s; the launcher (keyed by the manifest) was
  built, measured and removed.
- **`cargo fmt` on the Windows host**: 15 to 39 s there (Defender again),
  13 s in the mirror, and it would have split the lint in two.
- **One long-lived container per worktree with `docker exec`**: 1.2 to
  1.6 s a call against 2.5 to 8 s for `docker run`, but a command outlives
  a killed wrapper either way and the idle containers need a reaper;
  not worth it next to the minutes above.

## Consequences

- An agent's lint is a minute, not twenty; the wrapper's own cost per
  command is about 5 s, most of it `docker run` on the loaded host.
- A mirror is a second copy of the tree and holds every `target/`: about
  10 GB a worktree. Volumes of worktrees that are gone have to be removed
  by the owner (`docker volume ls`, `poly-snap-*` and the older
  `poly-target-*`).
- Files a command writes to an ignored path (`*.img`, `*.iso`, `target/`)
  do not appear in the checkout; before, the ones outside `target/` did.
- The template is as fresh as its last `polyd-template` run; an old one
  only costs compile time.
- The real fix for the 25 ms file opens is the owner's: a Defender
  exclusion for the worktrees, or a Dev Drive. With it the wrapper's
  non-mirror mode would be fast as well.

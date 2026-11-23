# 0270: Faster agent build loops on the Windows PC

**Status:** Accepted. Track `speed-up-worktree-builds`. Changes [ADR 0071](0071-our-ntdll-a-real-pe-dll-built-from-rust.md)
and [ADR 0079](0079-our-kernelbase-and-kernel32.md) (their build directories were deleted after every build), builds on
[ADR 0124](0124-native-desktop-alpine-shell-look-and-builds-on-windows-and-linux.md) (no host C toolchain) and
[ADR 0174](0174-agent-build-loop-a-linux-side-mirror-quick-lint-and-batch-coverage-edits.md) (`ci lint --quick`).

## Context

A speed test on the owner's Windows PC (one small `msvcp140.dll` task in a fresh worktree) took 17 minutes, of which
about 40 s was test and lint work. The rest was building, and four things made it worse than it had to be:

1. `pipeline win build --tests-dir <dir>` built `<dir>` *and* the whole `tests/windows` corpus.
2. `ci lint --quick` failed on Windows in `fmt (win32)`: `cargo fmt --all` hands rustfmt the root files of all 123
   local packages of the win32 workspace in one command line, longer than Windows takes (`os error 206`).
3. `pipeline win build` refused to run without `clang++` even when every C++ test was built by Microsoft's compiler
   and checked in as an `.exe`, and it made its import libraries with `llvm-dlltool`, a host LLVM tool.
4. Every build of the Win32 modules, `ntdll.dll` and the WOW64 DLLs that missed their content cache (any edit) built
   from nothing: the build directory was deleted afterwards, a choice made when the Win32 workspace had four crates
   (about 40 s). It has 85 now.

## Measurements (this PC: 32 threads, sccache 0.19 as the global `rustc-wrapper`)

`cargo xtask build` (ntdll, Win32 modules and WOW64 DLLs from their caches or built, the native programs, then the
kernel) after a one-line change to `msvcp140`:

| Case | Win32 modules | Kernel | Whole command | sccache |
|---|---:|---:|---:|---|
| Brand-new worktree, first build (before and after this ADR) | 165 s | 162 s | 346 s (+52 s building xtask) | 12 hits, 107 misses |
| Same worktree, next edit, build directory deleted (before) | 180 s | 40 s | 221 s | |
| Same worktree, next edit, build directory kept (after) | 4 s | 33 s | 38 s | |
| Brand-new worktree, sccache 0.20 with both worktrees as `basedirs`, warm cache | 140 s | 155 s | not finished (see below) | xtask: 50 hits, 0 misses |
| Brand-new worktree, `target/win32` copied from a warm worktree | 200 s (+290 s to copy) | | | 123 crates recompiled |

What a first build costs is work no cache can skip:
- **The Win32 workspace links 78 images with fat LTO** (`lto = true`, `codegen-units = 1`, `opt-level = "s"`). sccache
  caches no linked crate (`cdylib`, `bin`), and the LTO link is most of each one's time.
- **The kernel is one large `bin` crate**, compiled whole on a first build. Its dependencies are a small part.
- **Cargo cannot reuse another worktree's artifacts.** The crates in `crates/` are outside the win32 and kernel
  workspaces' roots, so cargo hashes their absolute paths into their metadata and fingerprints. A copied target
  directory rebuilt all 123 Poly crates, and the copy alone (Defender scans every file) took longer than the build.
- **Microsoft Defender scans every new executable when it first starts**: a build script that has run before starts in
  100 ms, a fresh copy of the same file takes 6.5 to 10.7 s. A cold Win32 build (191 s, `cargo build --timings`) starts
  its 78 build scripts at once at about 89 s, and each takes about 60 s to *run*, though about 60 of them only print
  `cargo:rustc-link-arg=/ENTRY:<name>`. That is about a third of the build, and proc macros and the other
  workspaces' build scripts pay it too. Real-time protection is tamper-protected on this PC, and exclusions need an
  administrator.

## Decision

1. **`--tests-dir` without `--tests` builds only those directories' tests** (`windows::build_runtime`, used by
   `win build` and `win remote`). With `--tests`, names may still come from the corpus or the directories.
   `win polycheck` always names its tests and is unchanged.
2. **`fmt (win32)` runs rustfmt itself, in chunks** (`fmt_all_chunked` in `tools/xtask/src/main.rs`): the root files of
   every local package of `cargo metadata` (members and the path dependencies they reach, what `cargo fmt --all`
   checks), grouped by edition, at most 16,000 characters of file names per rustfmt. Two rustfmt runs check the same
   files the one `cargo fmt` did.
3. **clang++ is needed only for C++ tests built here** (`wincxx::needs_compiler`). Tests built by Microsoft's compiler
   need no tool, and `win polycheck` no longer skips itself for them. When one is compiled here, the error names it.
   **The import libraries come from the toolchain's `rust-lld -flavor link /lib /def:`**, or from `$LLVM_DLLTOOL` when
   that is set. For all four `.def` files the libraries are byte-for-byte those `llvm-dlltool` makes, so the test
   executables and their recorded hashes stay the same. `lld-link` still links them, because a different linker
   version could change the bytes the Windows results were recorded with.
4. **The DLL build directories stay** (`target/ntdll`, `target/win32`, `target/win32-clippy`, `target/wow64`;
   `ntdll::keep_build`). The next build after an edit compiles only what changed: 221 s became 38 s. `target/win32` is
   about 360 MB. CI (`CI` set) still deletes them, so its caches do not take them. `POLY_*_KEEP_BUILD=0` deletes and
   `=1` keeps, everywhere.

## Rejected

- **sccache 0.20's `basedirs` for sharing crates between worktrees.** sccache 0.20.0 (released 2026-11-23) strips
  basedirs from Rust compiles' keys. It would have to be the server's setting, though: clients take the basedirs from
  the server's handshake, even in client-side mode. They are plain prefixes, so every worktree root would have to be
  listed and the shared server restarted for each new worktree. The gain measured is small (Win32 165 → 140 s, kernel
  162 → 155 s) because of the links above. Crates that read `env!("CARGO_MANIFEST_DIR")` (`tools/pipeline` among
  them) would also need `SCCACHE_BASEDIRS_VERBATIM_ENV_DEPS`, or they would bake in another worktree's path. And two
  runs on the private test server ended with `rust-lld` failing to start (`0xc0000142`) or hanging. Both came around a
  restart of the agent host, so the cause is not clear. Worth trying again once 0.20 has settled, together with an
  xtask step that keeps the server's basedirs current.
- **Seeding a new worktree's target directory** from a warm one: no gain, measured above. Hard links would be fast,
  but rustc writes outputs in place and would change the other worktree's files.
- **A shared target directory** for all worktrees: the `crates/` dependencies differ by path, so they would be
  separate artifacts anyway. The workspace members would share file names and rebuild each other's output, under
  cargo's lock, which would also serialise the agents.

## Not decided here

- **The Defender scans of new build-script executables.** There are two ways out: a Defender exclusion for the
  worktrees' `target` directories (the owner's or IT's decision, not a repository change), or fewer build scripts. A
  DLL whose entry point is named `_DllMainCRTStartup`, the linker's default, needs no `/ENTRY` and no `build.rs`, but
  that touches about 60 crates and their 32-bit builds.

- **The release profile of the Win32 workspace** (fat LTO, one codegen unit) decides most of a first build. A lighter
  profile for agents' loops would change the DLLs that tests run against (sizes, inlining, stack use). That is the
  owner's call.
- **A stable build path per worktree slot** with `-Zchecksum-freshness`, which would let cargo reuse another worktree's
  artifacts by content, is possible but a larger change. It needs its own ADR.

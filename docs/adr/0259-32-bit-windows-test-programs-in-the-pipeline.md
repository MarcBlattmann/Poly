# 0259: 32-bit Windows test programs in the pipeline

**Status:** Accepted. Builds on [ADR 0075](0075-windows-test-oracle.md) (the Windows test
programs and `poly-wtest`) and [ADR 0186](0186-32-bit-windows-programs-compatibility-mode-poly-s-own-i686-ntdll-and-converted-system-calls.md)
(32-bit programs on Poly). Track `spec-win-wow64-registry`.

## Context

Some Windows behaviour exists only in a 32-bit process: the registry views of WOW64
(`HKLM\Software` redirected to `Software\Wow6432Node`, `Software\Classes\CLSID` to
`Classes\Wow6432Node\CLSID`, the view a handle inherits), `IsWow64Process`, the file system
redirector. Google's 32-bit updater creates a class key one level at a time below a handle
opened with `KEY_WOW64_64KEY`; Poly had no recorded reference for what Windows does. The
pipeline built x64 programs only, so no test could record it.

## Decision

1. **A Rust test with the line `// poly-arch: x86` is a 32-bit program.** `win build` puts all
   such tests into a second generated package (`target/pipeline/windows/build32`, target
   directory `cargo32`) and builds it for `i686-pc-windows-msvc` with the flags of the x64
   build plus `/safeseh:no` (lld marks an x86 image with no handler table `NO_SEH`, and the
   system then refuses its frame handlers; ADR 0186 does the same for `tests/windows/wow64`).
   The image check is `check_image_arch`: an I386 PE32 console executable with the same
   import rules. The manifest's `Built.arch` is `x86` (absent: x64).
2. **`poly-wtest` builds for both targets.** The TEB/PEB reads use `fs:[0x30]` and the 32-bit
   offsets on x86, the ntdll declarations use `import_name_type = "undecorated"` there, the entry
   point is `extern "C"` (the x86 `system` convention would decorate it), and on x86 the crate links
   `poly-msvc32` (`__chkstk`, 64-bit arithmetic helpers) instead of the x64 `__chkstk`.
3. **Nothing else changes.** The runner on Windows runs any executable; the split, the hand-off
   records and the diff runner see tests, not architectures. A 32-bit test that Poly cannot run
   yet fails like any other test of a claimed entry.

## Consequences

- A 32-bit test needs `-Zbuild-std` for a second target: the first build of the x86 package
  takes about a minute longer.
- Green entries' visible tests run on Poly; a 32-bit test added to a green entry would turn
  its CI red until Poly runs it. New 32-bit tests go to new or claimed entries.

# 0098: C++ Windows test programs (clang, our own import libraries, a `poly-cxx:` line for older exception tables), the Windows split with recorded results, and the spec of C++ exception handling

**Status:** Accepted. Extends [ADR 0075](0075-windows-test-oracle.md) (the
Windows test oracle) and works toward the C++ exception handling that
[ADR 0087](0087-7-zip-command-line-on-poly.md) left as a stub.

## Context

7-Zip's `7za.exe` imports `__CxxFrameHandler` and `_CxxThrowException` from
`msvcrt.dll`. Poly's `__CxxFrameHandler` continues every search (ADR 0087),
so no `catch` ever takes an exception and no destructor runs during an
unwind. Before anyone implements it, the behaviour has to be observed on
real Windows and written down (CLEAN_ROOM.md). The Windows tests of ADR 0075
are Rust, which cannot throw or catch a C++ exception; ADR 0075 chose Rust
over C only because the home server has no clang, and listed the Windows
`tests split` and `handoff` as not built.

## Decision

### C++ tests next to the Rust ones

A Windows test may be one `.cpp` file (in `tests/windows/` or a spec entry's
`<symbol>.tests/`, held-out ones in the store), named like a Rust test.
`cargo xtask pipeline win build` builds it (`tools/pipeline/src/wincxx.rs`):

- `clang++ --target=x86_64-pc-windows-msvc -std=c++17 -fms-extensions
  -fexceptions -fcxx-exceptions -ffreestanding -nostdlib -fno-builtin -O1`,
  `-I tests/windows/cxx`, `-ffile-prefix-map` for the repository;
- import libraries made by `llvm-dlltool` from our own
  `tests/windows/cxx/{kernel32,msvcrt}.def` (names from the DLLs' export
  tables and Learn: ABI facts, no Microsoft library involved);
- `lld-link /NODEFAULTLIB /ENTRY:mainCRTStartup /SUBSYSTEM:CONSOLE /Brepro
  /DEBUG:NONE`; the executable must import by name from `ntdll.dll`,
  `kernel32.dll` and `msvcrt.dll` only. Builds are reproducible (same bytes
  twice), so a recorded Windows result names its executable by hash.
- Tools from `CLANGXX`, `LLD_LINK`, `LLVM_DLLTOOL` (default names);
  `manifest.json` records `clang++ --version`. Only C++ tests need them; a
  machine without clang still builds the Rust tests.

`tests/windows/cxx/poly_wtest.h` is the C++ side of `poly-wtest`: the
observation file beside the executable (`GetModuleFileNameW`,
`CreateFileW`, `WriteFile`), `ok` / `not ok` / `value` / `summary` lines,
`ExitProcess` with 0/1/2, an event log for orders of constructors and
destructors, and `POLY_TEST_MAIN`. It also gives the program what the
compiler refers to and `msvcrt.dll` does not export: `??_7type_info@@6B@`
(type descriptors start with it; nothing calls through it) and
`__std_terminate` (clang's terminate scope; ours counts the call and calls
`msvcrt`'s `terminate()`), plus no-op `operator delete` for deleting
destructors (tests never allocate). No C runtime runs, so globals must be
constant-initialised.

### Older exception tables: the `poly-cxx:` line

clang emits the newest x64 tables: `__CxxFrameHandler3`, FuncInfo magic
`0x19930522`, EH flags 1. Programs of older compilers (7-Zip) name
`__CxxFrameHandler` and may carry `0x19930520` or `0x19930521`. A test asks
for those with one line, `// poly-cxx: handler=__CxxFrameHandler
magic=0x19930520 ehflags=0 cflags=-fasync-exceptions`: the build compiles to
assembly, renames the handler, replaces the fields clang's assembly marks
`# MagicNumber` and `# EHFlags` (LLVM's `WinException.cpp` layout), and
assembles that; `cflags` takes a short allowed list. (clang prints
`sym@IMGREL` unquoted for decorated names containing `@`, which its own
assembler misreads; the build quotes them.) Written so rather than as
hand-made tables because the compiler's tables are the ones real code has;
only the fields that differ between versions are changed.

### Running them

On Windows nothing changes: `poly-oracle.ps1` runs any executable of a
bundle. On Poly `win poly`/`win diff` boot them like the Rust tests; they
load our `msvcrt.dll`.

### The Windows split: `tests split` with `--windows-results`

For a Windows entry `pipeline tests split` accepts only test sources
(`.rs`, `.cpp`, valid names, no subdirectories: support files belong in the
repository). With `--windows-results <poly-winrun/1 file>` every test must
have run on Windows with the same result on every run, and the results are
split with the tests: the visible tests' into `<symbol>.tests/windows.json`,
the held-out tests' into `windows.json` in the entry's held-out directory
(ADR 0075's "recorded references"). `win diff --reference
<symbol>.tests/windows.json` then compares Poly without the PC. `handoff
record|bundle|verify` needed no change: they hand over the spec sheet and
everything in the tests directory, the results file included, and check
held-out contents by hash as before.

### The spec: three sheets, 48 tests

`specs/windows/msvcrt.dll/{__CxxFrameHandler3,__CxxFrameHandler,_CxxThrowException}.yaml`,
observed with the system's `msvcrt.dll` on Windows 11 Home 25H2 (build
26200.9711, the owner's home PC, whose results are recorded) and Windows 11
Enterprise 25H2 (build 26200.9448), byte for byte the same; 48 tests (33
visible, 15 held out), all passing on both in three runs each. Sources: our observations; the table layouts as LLVM
writes them (`WinException.cpp`, `MicrosoftCXXABI.cpp`,
`X86FrameLowering.cpp`, Apache-2.0 with LLVM exception); Learn's "x64
exception handling"; `winnt.h`; Raymond Chen's article on 0xE06D7363;
Wine's `dlls/msvcrt` for which questions to ask (behaviour only). No
Microsoft binary was disassembled. Findings that were not obvious:

- A by-value catch object is copied before the frames below are unwound;
  the exception object lives in the throwing frame, which stays in place
  while the catch block runs, and is destroyed after it.
- With EH flags bit 0 (`/EHs`, clang's default) a non-C++ exception is
  ignored by the frame entirely: no `catch(...)`, and no destructors when an
  outer `__except` takes it. With flags 0 or the older magic numbers (which
  have no flags field) the destructors run, but `catch(...)` with adjective
  `0x40` still does not take it; only `catch(...)` without `0x40`
  (`-fasync-exceptions`, `/EHa`) does.
- When an `__except` takes a C++ exception, the object is destroyed first,
  even with no C++ frame between: the throw's own frame does it.
- A rethrow record (parameters 1..3 zero) is completed in place by the frame
  handler of the catch block it leaves; a rethrow with no exception being
  handled is matched by nothing and ends unhandled (no `terminate`).
- `RaiseException` sets flag `0x80` on this build, so a throw shows `0x81`.

## Consequences

- Spec agents can write C++ Windows tests; the pipeline needs clang,
  lld-link and llvm-dlltool only for them (the dev container has them).
- `win build` keeps refusing anything but `ntdll`/`kernel32` imports for
  Rust tests; C++ tests may also import `msvcrt.dll`.
- New code: `wincxx.rs` (directives, assembly rewrite, build) and the split
  helpers in `windows.rs`, host-tested; `tests/windows/cxx/`.
- Not built: a `win polycheck` for green Windows entries in CI (it can now
  use the recorded `windows.json`), and a way to run the C++ build where
  there is no clang (the home server).

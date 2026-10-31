# 0099: C++ exception handling in msvcrt: a host-tested decision crate, catch blocks run through a consolidating unwind, and a CI check for green Windows entries

**Status:** Accepted. Implements the spec of [ADR 0098](0098-cpp-windows-tests-and-the-cpp-exception-spec.md)
(`specs/windows/msvcrt.dll/{__CxxFrameHandler3,__CxxFrameHandler,_CxxThrowException}.yaml`,
handed from `spec-cxxeh` to `impl-cxxeh`) on the SEH of
[ADR 0068](0068-nt-structured-exception-handling.md) and our `ntdll`
([ADR 0071](0071-our-ntdll-a-real-pe-dll-built-from-rust.md)); replaces the
stubs of [ADR 0087](0087-7-zip-command-line-on-poly.md).

## Context

Poly's `__CxxFrameHandler` continued every search, so a C++ `throw` ended
the process with `0xE06D7363`; 7-Zip's error paths throw. The spec says what
the three functions do; its inputs were our own programs on Windows 11, the
table layouts as the compiler writes them, Learn's "x64 exception handling"
and `winnt.h`. The implementation reads only those, the 33 visible tests and
their recorded Windows results (CLEAN_ROOM.md); no other implementation of
these handlers was read.

Two facts shape the design:

- **The catch block runs on top of the stack it unwound.** The exception
  object lives in the throwing frame, and a by-value catch object is copied
  before anything is unwound, yet the frames between are unwound (their
  destructors run) before the catch block, and the object is destroyed after
  it (`stack_during_catch`, `order_of_events`). The frames' memory must stay
  in place until the catch block ends.
- **A catch block can throw, rethrow and catch again**, any number of
  levels deep, and the new exception's stack walk must pass from the catch
  block into the frame that holds its try block, not into the dispatcher
  frames of the first exception.

## Decision

### The decisions in `crates/poly-cxxeh`

Everything that does not touch the process is a `no_std` crate, tested on
the host with images built in byte buffers (the shapes of the visible tests'
tables): `tables` (FuncInfo by magic number, reading only the fields the
magic has; unwind map, try block map, handler arrays, IP-to-state lookup),
`throw` (the record, ThrowInfo and catchable types, matching by decorated
name with the throw's qualifiers, the `mdisp`/`pdisp`/`vdisp` adjustment,
the catch object as a value, a byte copy or a copy-function call), `frame`
(the kind of a frame, the try blocks that apply, the search decision —
continue, terminate, or this clause — and the unwind actions between two
states) and `catches` (the catch blocks in progress). No input panics it;
a fuzz test throws random tables at every entry point. It reads memory
through `poly_seh::Memory`, the SEH crate's trait (nothing in `poly-seh`
changed).

**A frame is the function, one of its catch funclets, or an unwind
funclet**, told apart by its state: catch blocks' code runs in the states
`TryHigh+1..=CatchHigh` of their try block (the innermost such try block),
which the function's own code never has; a frame whose function starts at
an unwind map action is an unwind funclet and takes part in nothing. In a
catch funclet's frame only the try blocks *inside* its catch block apply,
and its unwind stops when the state leaves that catch block's states; the
try blocks around the whole try statement are looked at in the frame that
holds it. So every try block is matched in the frame whose code it is, and
the catch's continuation is in that frame. Catch objects and RDX for every
funclet are the function's establisher frame, read through
`ParentFrameOffset` (of the clause whose funclet the frame is).

**The state of a frame whose catch block runs** is not the state of its
control PC (an address in the try block) but the state around the try
statement, `ToState` of `TryLow`: the per-thread list of catch blocks in
progress keeps it, keyed by establisher frame and function, so a later
exception neither matches the same try block again nor destroys the try
block's objects twice. The list also gives the exception a rethrow means
(the innermost catch block still running) and decides when an exception
object dies: at the end of its catch block unless another running catch
block holds it (a rethrow caught inside the catch block); when its catch
block is left by an unwind, unless that unwind goes into a C++ catch block
that takes the same object (a rethrow caught further out). An abandoned
entry stays until its target frame is unwound or a later catch block's
target frame is at or above it, because the target frame's handler runs
after the catch block's frames and still needs the state.

### The glue in msvcrt (`personalities/nt/win32/msvcrt/src/cxx.rs`)

- `_CxxThrowException` is assembly with an unwind handler: it raises
  `0xE06D7363` (flags `EXCEPTION_NONCONTINUABLE`, parameters 0x19930520,
  object, ThrowInfo, image base by `RtlPcToFileHeader`, 0 for a rethrow) and
  keeps the object in its frame. An unwind of its exception that is not the
  one into a C++ catch block (an `__except` took it) destroys the object
  before any other frame is unwound.
- `__CxxFrameHandler3` and `__CxxFrameHandler` are one handler. During the
  search it completes a rethrow record in place, decides, and for a catch
  makes the catch object and calls `ntdll`'s `RtlUnwindEx` to its frame with
  a `STATUS_UNWIND_CONSOLIDATE` record (`ntstatus.h`) whose first parameter
  is msvcrt's callback and second a description of the catch, left in the
  searching handler's frame. During an unwind it runs the frame's actions
  down to -1, or for the target frame to the try block's `TryLow`, or for a
  foreign target (`longjmp`) to the state of the target address; a
  synchronous-only frame ignores non-C++ exceptions also here.
- The callback runs the catch funclet through `poly_cxx_call_catch`, an
  assembly frame whose handler notices the catch block being left by an
  unwind; at the end it destroys the object and returns the continuation.
- Unwind actions, copy functions and destructors are called through
  `poly_cxx_call_guarded`, whose handler calls `terminate()` when an
  exception leaves them during an unwind or a catch-object copy.

### What `ntdll` and `kernelbase` gained

- **`RtlUnwindEx` with `STATUS_UNWIND_CONSOLIDATE`:** after the unwind,
  instead of resuming the target context, `ExceptionInformation[0]` is called
  as `callback(record)` on the stack below the unwound frames, and the
  address it returns becomes the target's RIP, which is then resumed with
  `NtContinue`. The callback is entered from `poly_consolidate_frame`, whose
  unwind data is a machine frame (`UWOP_PUSH_MACHFRAME`) holding the target's
  RIP and RSP and whose nonvolatile registers are the target's: a stack walk
  from inside the catch block continues in the target frame. Exceptions in a
  catch block are therefore ordinary new exceptions; no nested-exception or
  collided-unwind dispositions are needed.
- **The top-level filter for unhandled exceptions:**
  `RtlSetUnhandledExceptionFilter` (an `ntdll` export name) registers a
  function the dispatcher asks, with `EXCEPTION_POINTERS`, before an
  exception no frame took goes second chance; kernelbase registers its
  `UnhandledExceptionFilter` when the program calls
  `SetUnhandledExceptionFilter`. Nothing is unwound first, as the spec
  requires.
- **`RaiseException` sets `EXCEPTION_SOFTWARE_ORIGINATE` (0x80)** on every
  record, as observed (`record_flags`).

### CI: `pipeline win polycheck`

ADR 0098 left a CI check for green Windows entries unbuilt. `cargo xtask
pipeline win polycheck` builds the visible tests of every green Windows
entry that has a `windows.json`, runs them on Poly and compares them with the
recorded Windows results; every one must pass. `cargo xtask ci pipeline`
runs it after `polycheck`; the CI job installs clang, lld and llvm for the
C++ tests (Ubuntu 24.04's clang 18.1.3, as in the dev container, so the
executables are byte for byte those of the recording). It skips itself
without QEMU, without the C++ tools, and for implementer agents
(`POLY_ROLE=implementer`).

## Results

- All 33 visible tests pass on Poly against the recorded Windows results
  (`__CxxFrameHandler3` 17, `__CxxFrameHandler` 11, `_CxxThrowException` 5).
- 7-Zip scenario (ADR 0088): the three C++ error steps (`missing-archive`,
  `bad-switch`, `bad-command`) now exit 2, 7, 7 as on Windows instead of
  `0xE06D7363`; `bad-switch` and `bad-command` match byte for byte, and 15
  of 16 steps pass. `missing-archive` differs only in the text of
  `0x80070002` on stderr ("Error #80070002" against "The system cannot find
  the file specified."): `FormatMessageW`'s system messages, not C++
  exceptions.

## To be confirmed by the spec agent

1. **The state of a frame while one of its catch blocks runs.** We use
   `ToState` of the try block's `TryLow` (the state around the try
   statement), kept per thread rather than in the frame's `UnwindHelp`
   slot. Observable only through nesting the visible tests already cover.
2. **A catch funclet's frame looks only at try blocks inside its catch
   block**; the enclosing try blocks are matched in the frame that holds the
   try statement. The spec's `in_catch_block` allows both readings; ours
   gives the same clause, and the same order of destructors in the visible
   tests.
3. **`throw;` inside a try block inside the catch block, caught there**: we
   do not destroy the object when that inner catch block ends (the outer one
   still has it), only at the end of the outer one.
4. **`throw;` while the exception being handled is not a C++ one** (inside
   `catch(...)` that took an SEH exception under `/EHa`): we leave the record
   as it is (it matches nothing). Not observed.
5. **A foreign target unwind** (`longjmp` into a frame with C++ tables)
   unwinds the frame to the state of the target address; a non-C++ unwind
   record with `STATUS_UNWIND` counts as a non-C++ exception for the
   synchronous-only flag. Not observed.
6. **Exceptions leaving a catch-object copy constructor or the ThrowInfo
   destructor** call `terminate()`, like those leaving a destructor during
   an unwind. Not observed.
7. **`__CxxFrameHandler2`** (an export name of msvcrt) is not provided; no
   test imports it.

## Consequences

- C++ programs built against msvcrt (7-Zip's `7za.exe`, the C++ tests) catch,
  rethrow and destroy as on Windows; the coverage entries are green with
  33 visible and 15 held-out tests (the held-out ones run by the
  coordinator).
- `__C_specific_handler`'s `__finally` blocks run in the same unwind, in
  frame order with the destructors, unchanged.
- The CRT's `setjmp`/`longjmp` slice can unwind through C++ frames: a foreign
  target unwind is handled (point 5).
- New code: `crates/poly-cxxeh` (33 host tests), `msvcrt/src/cxx.rs`, the
  consolidation and top-level filter in `ntdll/src/seh.rs`, two lines in
  kernelbase, `pipeline win polycheck` and `green::windows_entries`.

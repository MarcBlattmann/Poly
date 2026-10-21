# 0068: NT structured exception handling: kernel-to-user dispatch, x64 unwinding, and Poly's own dispatcher registration

**Status:** Accepted. Builds on [ADR 0063](0063-nt-processes-object-manager-and-memory.md)
(NT processes, the service layer, Poly's own service numbers, the ring-3
test PE), [ADR 0065](0065-nt-threads-synchronisation-and-apcs.md) (threads,
blocking, suspension, the APC trampoline and `PolyResumeApc` as a stand-in
for `KiUserApcDispatcher`), [ADR 0029](0029-pe-loader-logic-in-poly-pe.md)
(`poly-pe`: PE parsing, the exception directory) and
[ADR 0055](0055-microsoft-redistributables-run-apps-own-copies.md) (Poly's
side is `ntdll`, which owns exception dispatch and unwinding).

## Context

Phase 5's structured-exception-handling item asks for kernel-to-user
exception dispatch and unwinding. On x64 there are no FS-based exception
registration records (that is x86); the model is **table-based**: every
non-leaf function describes its stack frame with an `UNWIND_INFO` structure
reached through a `RUNTIME_FUNCTION` in the image's `.pdata` (exception
directory). When an exception is raised the kernel builds an
`EXCEPTION_RECORD` and a `CONTEXT` and transfers to `ntdll`'s
`KiUserExceptionDispatcher`, which walks the stack with `RtlVirtualUnwind`,
offering the exception to each frame's language-specific handler
(`RtlDispatchException`); a handler that takes it unwinds the stack to its
frame (`RtlUnwindEx`), and resumes with `NtContinue`. An exception nobody
handles ends the process.

Two facts shape the work:

- **We have no `ntdll` yet** (the next roadmap item). So the kernel must do
  what `ntdll` will later do — build the record and context, and hand them
  to a user-mode dispatcher — and there is no `KiUserExceptionDispatcher`
  export to transfer to. Poly needs its own way for a process to register a
  dispatcher, exactly as ADR 0065 gave threads a trampoline and
  `PolyResumeApc` instead of `RtlUserThreadStart` and
  `KiUserApcDispatcher`.
- **Clean room** ([CLEAN_ROOM.md](../../CLEAN_ROOM.md)): only public
  Microsoft documentation and public SDK headers. Everything here is from
  Learn ("x64 exception handling", "Structured Exception Handling (C/C++)",
  the `RtlVirtualUnwind` / `RtlLookupFunctionEntry` / `RtlUnwindEx` /
  `RtlCaptureContext` / `RaiseException` pages, the `EXCEPTION_RECORD`,
  `CONTEXT` and `EXCEPTION_POINTERS` pages), the PE Format specification's
  ".pdata section", and the `winnt.h` / `excpt.h` / `ntstatus.h` layouts and
  constants. Nothing is disassembled, decompiled or taken from Wine or
  ReactOS. There is no Windows oracle VM, so behaviours the documentation
  leaves open are labelled guesses (listed below) for the spec agent.

## Decision

**The hardware-free SEH logic is a host-tested `no_std` crate,
`crates/poly-seh`.** It knows nothing of handles, processes or the kernel;
it reads memory through a `Memory` trait and calls language-specific
handlers through a `Handlers` trait, so it is tested on the host with byte
maps and closures, and our `ntdll` will drive it over user memory and real
handler routines. It holds:

- `pdata`: `RUNTIME_FUNCTION` entries and `RtlLookupFunctionEntry` by binary
  search over the sorted table.
- `info`: `UNWIND_INFO` and its `UNWIND_CODE` array, decoded into operations
  — every documented `UWOP_*` (`PUSH_NONVOL`, `ALLOC_SMALL`, `ALLOC_LARGE`
  in both forms, `SET_FPREG`, `SAVE_NONVOL(_FAR)`, `SAVE_XMM128(_FAR)`,
  `PUSH_MACHFRAME`) — with the exception handler and its language-specific
  data, or chained unwind info (`UNW_FLAG_CHAININFO`).
- `unwind`: the virtual unwinder (`RtlVirtualUnwind`): it takes a `CONTEXT`
  in a function to its caller's, handling the prolog (undo only the codes
  already executed), the body (undo all, then the chained info's, and report
  the handler), and an epilog (decode and simulate the remaining
  `add/lea rsp`, `pop`s and the terminator). Restored registers' stack
  addresses are reported (`KNONVOLATILE_CONTEXT_POINTERS`).
- `context`, `record`: the x64 `CONTEXT` (0x4D0 bytes), `EXCEPTION_RECORD`
  (0x98) and `DISPATCHER_CONTEXT` (0x50) as values and byte layouts.
- `dispatch`: the two passes over a stack — `dispatch_exception`
  (`RtlDispatchException`) and `unwind` (`RtlUnwindEx`) — and a plain stack
  walk for backtraces.

**No input makes the crate panic.** Malformed or truncated tables and unwind
data, unreadable memory, loops in chained info, and address arithmetic that
overflows are all errors, never panics; a fuzz test throws random and
truncated bytes at every entry point.

**Kernel-to-user dispatch happens in `personalities/nt` and `kernel/src/nt`
(`exception` modules).** When an NT thread faults (any CPU exception) or
calls `NtRaiseException`, the service layer classifies it into an
`EXCEPTION_RECORD` with the documented `STATUS_*` code and parameters, the
kernel fills a `CONTEXT` from the trap frame and the FPU legacy state, and
the crate-free `exception::deliver` writes both to the thread's user stack
and points the thread at the process's dispatcher. `NtContinue` restores a
`CONTEXT`; an unhandled exception (no dispatcher, a stack the frame cannot
be written to, a fail-fast, or `NtRaiseException` second chance) ends the
process with the exception code as its exit status, after logging a
`.pdata`-walked backtrace of the user stack.

**Poly's own dispatcher registration (`PolySetExceptionDispatcher`,
`table 3` service `0x3010`).** Until our `ntdll` exports
`KiUserExceptionDispatcher`, a process names its user-mode dispatcher with
this Poly service (0: none). It is the SEH counterpart of ADR 0065's
thread-start and APC trampolines. The dispatcher is entered as
`dispatcher(EXCEPTION_RECORD *record, CONTEXT *context)` (RCX, RDX), with
the record and context on the thread's own stack below a 16-byte-aligned,
shadow-spaced frame and a zero return address, and RFLAGS.TF/DF cleared; it
ends with `NtContinue` or `NtRaiseException`. When `ntdll` arrives it will
provide `KiUserExceptionDispatcher`, and the kernel will take its export
address instead of this service, which then becomes a tool/test aid like
`PolyResumeApc`.

**New services (Poly's numbering, `services.rs`).** In the processes/threads
group: `NtRaiseException` (`0x0A0`), `NtContinue` (`0x0A1`). One of Poly's
own: `PolySetExceptionDispatcher` (`0x3010`). The WDK does not document
`NtRaiseException`/`NtContinue`'s parameters; they are Poly's reading of the
documented `RaiseException` / `EXCEPTION_RECORD` / `CONTEXT` model.

## To be confirmed by the spec agent against real Windows

1. The exact `STATUS_*` each CPU vector raises, and the
   `ExceptionInformation` of each (we map #DE→`STATUS_INTEGER_DIVIDE_BY_ZERO`
   or `STATUS_INTEGER_OVERFLOW` by decoding the `div`/`idiv` divisor,
   #UD/#NM→`STATUS_ILLEGAL_INSTRUCTION`, #GP on a privileged instruction→
   `STATUS_PRIVILEGED_INSTRUCTION` else `STATUS_ACCESS_VIOLATION`,
   #PF→`STATUS_ACCESS_VIOLATION`/`_IN_PAGE_ERROR`/`_GUARD_PAGE_VIOLATION`/
   `_STACK_OVERFLOW`, #AC→`STATUS_DATATYPE_MISALIGNMENT`, #MF/#XM→the x87/SSE
   status-word mapping). A #GP with no faulting address reports
   `ExceptionInformation = [0, 0xFFFFFFFFFFFFFFFF]`.
2. `NtRaiseException`'s and `NtContinue`'s exact argument order and the
   statuses they return, and whether `NtContinue` validates RIP/RSP (we
   refuse a non-user RIP or RSP with `STATUS_INVALID_PARAMETER`).
3. The dispatcher-frame layout `KiUserExceptionDispatcher` is entered with
   (ours: record and context below a call-aligned frame; Windows' exact
   offsets and whether the machine frame is included).
4. `__fastfail`/`int 0x29` semantics (we raise a non-continuable
   `STATUS_STACK_BUFFER_OVERRUN` with the code from RCX and never dispatch
   it).
5. Collided unwinds and nested exceptions (handler dispositions 2 and 3),
   which we currently reject as `STATUS_INVALID_DISPOSITION`.
6. Whether a direct `jmp` that stays in the function ever ends an epilog
   (we treat only a `jmp` out of the function — a tail call — or a memory
   `jmp` as an epilog terminator, besides `ret`/`ret imm`/`rep ret`).
7. Segment selector values in the `CONTEXT` of a faulting user thread (we
   report CS/SS from the trap frame and DS/ES/FS/GS equal to SS).

## Consequences

- `poly-seh` is fully host-tested (parsing, the unwinder against
  hand-assembled prologs/epilogs/frame-pointer/machine-frame/chained
  functions, the two dispatch passes over a three-frame stack, backtraces,
  and fuzzing); `personalities/nt/src/exception.rs` is host-tested against
  the ADR 0063/0065 fake kernel (classification of every vector, the frame
  layout, `NtRaiseException`/`NtContinue`/`PolySetExceptionDispatcher`, and a
  full fault→dispatch→fix-up→`NtContinue` round trip). The kernel side proves
  the real transfer with two ring-3 `kernel_test!`s
  (`kernel/src/nt/test_seh.rs`): one registers a dispatcher, takes an access
  violation and a divide-by-zero, inspects the records, fixes the context up
  and continues; one leaves an exception unhandled and checks the process
  ends with `STATUS_ACCESS_VIOLATION`.
- The kernel stays thin: the trap frame and FPU state become a `CONTEXT` and
  back, and the process is ended for an unhandled exception; everything else
  is the crates'.
- The `CONTEXT` here carries the full floating-point/XMM legacy area (unlike
  ADR 0065's register-only `NtContext` for `NtGetContextThread`), because
  `NtContinue` must restore it. `NtGetContextThread`/`SetContextThread` keep
  their smaller context for now.
- What is left for the `ntdll` milestone: `KiUserExceptionDispatcher`,
  `RtlUnwindEx`/`RtlDispatchException`/`RtlVirtualUnwind` as exports over
  `poly-seh`, `__C_specific_handler` and the C/C++ `__try`/`__except`
  language handlers, vectored exception handlers, and debugger first-chance
  ports. `PolySetExceptionDispatcher` is then retired in favour of the
  `ntdll` export.
- The coverage map's `NtRaiseException` and `NtContinue` entries are claimed
  but stay ungreen until the Windows oracle exists (no held-out tests yet).

# 0063: NT processes in the kernel: the object manager, memory services and the first PE

**Status:** Accepted. Builds on [ADR 0001](0001-multiple-personalities-one-kernel.md)
(one kernel, three personalities, each choosing its own service numbering),
[ADR 0029](0029-pe-loader-logic-in-poly-pe.md) (`poly-pe`: PE parsing and
load plans), [ADR 0030](0030-nt-object-manager-and-dispatcher-in-a-crate.md)
(`poly-ntobj`: the object manager and dispatcher as host-tested logic),
[ADR 0022](0022-user-mode-address-spaces-and-system-calls.md) and
[ADR 0036](0036-demand-paging-and-exec.md) (user address spaces, the syscall
path, `user::Memory` and demand paging), and [ADR 0055](0055-microsoft-redistributables-run-apps-own-copies.md)
(Poly runs apps' own redistributables; our side is `ntdll`, `kernelbase`, ...).

## Context

Phase 5's first two kernel items are the NT object manager (handles, the
object namespace, reference counts) and the NT memory services (sections,
`NtAllocateVirtualMemory`, `NtMapViewOfSection`, protections). `poly-ntobj`
(ADR 0030) and `poly-pe` (ADR 0029) already hold the hardware-free logic,
host-tested; nothing in the kernel used them. A PE executable detected by
`poly-binfmt` was tagged `Personality::Nt` but refused at exec.

There is no Windows oracle VM yet, so the clean-room rules
([CLEAN_ROOM.md](../../CLEAN_ROOM.md)) allow only public Microsoft
documentation (learn.microsoft.com, the Windows Driver Kit `Nt*`/`Zw*`
reference), public SDK/WDK header *names, layouts and constants*
(`ntstatus.h`, `winnt.h`, `ntdef.h`, `wdm.h`, `winternl.h`), and the PE/COFF
specification. Nothing is disassembled, decompiled or debugged, and no Wine
or ReactOS source is read. Where documentation does not settle a behaviour,
the code picks one and says "to be confirmed by the spec agent against real
Windows"; ADR 0030 already lists the object-manager ones and this ADR adds
the memory and ABI ones below.

## Decision

**The NT personality grows a service layer (`personalities/nt`).** On top of
the ABI shell (ADR 0001) it gains:

- `services`: Poly's own NT system service numbers (see below).
- `abi`: the x64 call convention of `ntdll`'s stubs and the structures the
  services read and write in user memory (`OBJECT_ATTRIBUTES`,
  `UNICODE_STRING`, `MEMORY_BASIC_INFORMATION`, the `NtQueryObject` and
  `NtQueryDirectoryObject` information structures), captured through an
  `NtKernel` trait so the services are host-tested against a fake.
- `objects`: the handle, directory, symbolic-link and event services, over
  `poly-ntobj`.
- `vm`: an NT process's virtual-memory model — allocations (reserve ranges),
  per-page state (reserved or committed with a `PAGE_*` protection), the
  64 KiB allocation granularity and 4 KiB page, top-down placement, zero
  bits, guard pages and stack growth — carried out through a `Backend` the
  kernel implements over `user::Memory`.
- `protect`: checking a `PAGE_*` value and mapping it to hardware rights and
  to the view protections a section allows.
- `section` / `image`: sections (pagefile-backed, data-file, `SEC_IMAGE`
  using `poly-pe`'s load plan) and views.
- `system`: the machine-wide `NtSystem` (object manager + section metadata)
  and the per-process `NtProcess` (its `vm`, process/thread objects).
- `exec`: starting an NT process from a PE (image section mapped at its
  preferred base or relocated with `poly_pe::relocate`, the PEB, the first
  TEB, the stack with its guard page), its end, and exception codes.
- `pe_writer`: a minimal PE32+ writer, so the tests build their own
  executables with no Microsoft toolchain.

**Our own service numbering (`services::SERVICES`).** Windows renumbers its
system services with almost every build, and only `ntdll`/`win32u` call
them, so the numbers are not an application ABI. Poly's own `ntdll` is the
only caller of ours, so **the numbering is ours** and stable within Poly: a
number is never reused for another service. The layout is the one `ntdll`'s
x64 stubs use — `table << 12 | index`, table in bits 12–13 (0 = executive,
1 = win32k, 3 = Poly's own services for tests and tools). The executive
table groups the services: `0x000`–`0x03F` object manager, `0x040`–`0x07F`
memory and sections, `0x080`–`0x0BF` processes and threads. `cargo xtask`
will generate our `ntdll`'s stubs from this table.

**The kernel keeps one lock (`kernel/src/nt`).** Following ADR 0030, the
object manager, the per-process memory models and the section store sit
behind one `Mutex`; each system call locks it once and runs the service with
the calling process's `user::Memory` wrapped as a `Backend`. The per-process
`NtProcess` is keyed by pid, so `process.rs` keeps only a `new_nt`
constructor (no new field) and the NT state lives entirely in `kernel/src/nt`.

**The `Backend` maps, it does not decide.** The `vm`/`section` model decides
what every page is; the kernel's `MemBackend` carries it out on
`user::Memory` (ADR 0036): committed private memory is demand-zero anonymous
memory; a pagefile section's pages are an `AnonShared` object so every
shared view maps the same frames (and a copy-on-write view forks on write);
an image or data-file section's pages are a `MemoryImage` the kernel holds
(views are private copies). Section pages outlive the section object as long
as a view maps them, because each view holds its own `FileRef`.

**The first PE runs without `ntdll` yet.** `nt::start` maps the image as a
section, sets up the PEB, the first TEB and the stack, and starts the thread
at the entry point with GS based at the TEB; `program.rs` reaches it for a
PE file through the VFS, and a `kernel_test!` drives it from a PE Poly builds
itself. An executable that imports anything is refused
(`STATUS_INVALID_IMAGE_FORMAT`) until our `ntdll` binds imports — the next
Phase 5 milestones (processes/threads, TEB/PEB, synchronisation, our
`ntdll`).

### The TEB and PEB this milestone fills in

Only fields whose offsets public headers give; everything else is zero (the
full TEB/PEB is the next milestone's):

| Field | Offset | Header | Value |
|---|---|---|---|
| `NT_TIB.StackBase` | TEB + 0x08 | `winnt.h` (`NT_TIB`) | top of the stack |
| `NT_TIB.StackLimit` | TEB + 0x10 | `winnt.h` | lowest committed stack page (grows with the stack) |
| `NT_TIB.Self` | TEB + 0x30 | `winnt.h` (`NtCurrentTeb` reads `gs:[0x30]`) | the TEB |
| `TEB.ProcessEnvironmentBlock` | TEB + 0x60 | `winternl.h` | the PEB |
| `PEB.BeingDebugged` | PEB + 0x02 | `winternl.h` | 0 |

## To be confirmed by the spec agent against real Windows

In addition to ADR 0030's object-manager list:

1. The `NTSTATUS` each memory/section failure returns (the code uses the
   documented family — `STATUS_INVALID_PARAMETER_n`,
   `STATUS_CONFLICTING_ADDRESSES`, `STATUS_MEMORY_NOT_ALLOCATED`,
   `STATUS_FREE_VM_NOT_AT_BASE`, `STATUS_SECTION_PROTECTION`,
   `STATUS_INVALID_PAGE_PROTECTION`, ... — but which one for each argument
   error is not all documented).
2. Zero-bits semantics on x64 (counted from bit 63, less than 21), and the
   highest usable user address (`lpMaximumApplicationAddress`).
3. `MEM_RESET`/`MEM_RESET_UNDO` alone vs. combined; which `AllocationType`
   and `FreeType` bit combinations are valid.
4. The exact `MEMORY_BASIC_INFORMATION` a free or reserved region reports
   (`AllocationBase`, `AllocationProtect`, `Protect` of a reserved page).
5. Guard-page semantics: `STATUS_GUARD_PAGE_VIOLATION` on first touch, the
   page's guard bit cleared, and a thread stack's growth to the last page
   then `STATUS_STACK_OVERFLOW`.
6. Section rules: `SEC_RESERVE` cross-view commit visibility, the offset
   rounding to 64 KiB, `STATUS_IMAGE_NOT_AT_BASE` for an image mapped away
   from its base, the view protections a section protection allows
   (`view_allowed`), and `STATUS_INVALID_IMAGE_FORMAT` vs. more specific
   image statuses.
7. The `OBJECT_ATTRIBUTES.Length` check, an odd or over-long
   `UNICODE_STRING`, and the order in which the services probe their output
   buffers.
8. The exception `NTSTATUS` each CPU vector raises (`exec::exception_status`).

## Consequences

- The object, memory and section logic is host-tested in `personalities/nt`
  against a fake kernel (user memory as pages with rights, section pages as
  shared buffers); the kernel side proves only mapping and locking, with one
  `kernel_test!` that runs a self-built PE from ring 3 through real `syscall`
  instructions: a named event created, opened, duplicated and closed to
  zero references (then gone), a directory queried, memory
  reserved/committed/protected/queried/freed, and a pagefile section mapped
  twice with a write through one view visible through the other.
- `kernel/src/nt` holds the only NT kernel state; the edits to shared files
  (`process.rs`, `user/mod.rs`, `syscall.rs`, `program.rs`) are small and
  local, as Phase 5 runs in parallel with Linux work.
- `SEC_RESERVE` views commit coherently within one view now; cross-view
  immediate visibility of a newly committed page needs a fault upgrade, left
  for the synchronisation/threads milestones. The host tests already pin the
  full reserve semantics against the model.
- The coverage map is unchanged: these services cannot go green without the
  Windows oracle, which does not exist yet.

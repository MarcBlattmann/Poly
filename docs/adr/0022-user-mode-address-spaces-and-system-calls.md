# 0022: User mode: per-process address spaces, the syscall path, user-memory access and FPU state

**Status:** Accepted. Builds on [0010](0010-kernel-address-space.md) (the
kernel half every address space shares),
[0012](0012-scheduler-policy-in-a-crate-context-switch-in-the-kernel.md) and
[0016](0016-interrupt-dispatch-tick-and-kernel-locks.md) (threads, the
switch, interrupt dispatch).

## Context

Phase 3 runs the first user programs. The kernel needs address spaces of
their own, a way into ring 3 and back (`syscall`, interrupts, exceptions),
a way to read and write user memory without trusting user pointers, and the
user-visible CPU state (SSE/AVX registers, FS and GS bases) kept per thread.
All three personalities enter through the same `syscall` instruction and
dispatch on `poly_abi::SyscallRegs`. The machine runs on one CPU, QEMU's
TCG in CI, whose default CPU model has no PCID, SMEP, SMAP or XSAVE, and
whose `max` model has all but PCID.

## Decision

**One PML4 per process, the higher half shared.** A user address space
copies PML4 slots 256-511 from the kernel PML4 when it is created; they
point at the kernel's own PDPTs, so kernel mappings are shared, not copied.
The kernel creates every one of those PML4 entries at boot (an empty PDPT
for the MMIO window too), so no copy ever lacks a later kernel mapping. User
mappings are 4 KiB pages between 64 KiB and the last page below the
canonical hole; both ends stay unmapped (null pointers; no `syscall` at the
very end of the lower half). A space owns the frames it allocated
(`map_zeroed`), marked with a bit the CPU ignores in each leaf
(`PageFlags::SOFTWARE_9`), and frees them, its page tables and its PML4
when the last reference goes (`poly_paging::PageTables::clear`, host-tested).
`map_frames` maps frames it does not own (shared memory).

**Kernel threads run on the kernel PML4.** Switching to a kernel thread
loads it, switching to a user thread loads that thread's space. A kernel
thread therefore never sees, and can never be tricked into touching, some
process's memory, and a space is never loaded once its last thread is gone.
The cost is a CR3 load (and, without PCIDs, a TLB flush) per switch between
a user and a kernel thread; lazy switching can come later, as an
optimisation, if profiles ask for it.

**PCIDs when the CPU has them.** PCID 0 is the kernel PML4's; spaces take
1-255 round robin, and a table remembers which space each was last given
to. A load whose PCID is still the space's keeps the TLB (CR3 bit 63);
otherwise the space takes the next PCID and loads without bit 63, which
flushes whatever the previous owner left. A space changed while not loaded
forgets its PCID, so its next load flushes too. Kernel pages are global
either way. Nothing needs INVPCID. TCG cannot run this path (it does not
emulate PCIDs); a test checks the bookkeeping wherever it can run.

**syscall/SYSRET with a per-CPU area at GS.** SYSCALL masks IF, TF, DF, AC,
NT and IOPL. The entry stub does `swapgs`, parks the user RSP in the
per-CPU area, loads the thread's kernel stack top from it and pushes the
same `TrapFrame` an interrupt from ring 3 leaves, so there is one format of
"user registers". Interrupts and exceptions from ring 3 arrive on
TSS.rsp0, the same stack; `isr_common` swaps GS when the saved CS is ring 3,
and the common exit swaps back. Only assembly uses GS; Rust reads the
per-CPU static, so the NMI and machine check, which can hit the
instructions between `syscall` and `swapgs`, need no GS discipline. They
get an IST stack of their own, so they never run on the user's RSP.
FSGSBASE stays off: user mode sets its GS and FS bases only through the
kernel.

**SYSRET only to the lower half, IRETQ otherwise.** On Intel CPUs a SYSRET
to a non-canonical RCX faults in ring 0 *with the user's RSP already
loaded*. The handler uses SYSRET only when the return address is below
`0x8000_0000_0000`, and the common IRETQ exit otherwise; IRETQ checks
before it changes privilege, so a bad return faults on the kernel stack, at
a known instruction, which the #GP handler recognises (swap GS back, kill
the thread). TCG checks neither, so a test switch
(`make_next_return_fail`, an invalid SS selector) makes that path run in CI.

**System calls go through a per-thread hook.** A user thread carries a
`SyscallHook`, `fn(&Process, &mut SyscallRegs)`, called with interrupts
enabled; the default routes to `syscall::dispatch` by personality. Tests
(and later tracers, or a process whose personality is decided late) give
their own. The hook can end the thread (`user::exit`) or redirect the
return (`user::set_return_address`, for signal delivery later). RFLAGS is
sanitised on every return.

**A fault in ring 3 kills the thread, never the kernel.** It is logged in
one line, and the thread's exit code is `KILLED | vector`. Killing the rest
of the process waits for a process-wide thread list (exit_group).

**User memory through an exception table.** `copy_from_user`,
`copy_to_user` and `strncpy_from_user` check that the range lies in the
user half, then copy in small assembly routines; the page-fault handler
resumes a fault on a user address inside one of them at its fixup label,
and the call returns `Efault`. With SMAP the routines run between STAC and
CLAC, and interrupt entry clears AC; SMEP is on when present. Without them
the range checks are the protection. `AddressSpace::read`/`write` go through
the direct map instead, for spaces that are not loaded.

**FPU state: eager switching with an owner, no #NM.** The kernel is
soft-float and never touches x87/SSE/AVX, so the registers always hold one
user thread's state. Switching to a different user thread saves the
owner's (XSAVE64 with XCR0 = x87 | SSE | AVX as supported, else FXSAVE64)
and restores the new one's; switching to kernel threads or back to the
owner costs nothing. CR0.TS stays clear: no lazy trap, no state left
behind for another thread to observe. Areas are heap-allocated, 64-byte
aligned; a dying owner's area is simply forgotten. FS base and the user GS
base (in IA32_KERNEL_GS_BASE while in the kernel) are saved and restored
with the thread.

## Consequences

- The integration step builds processes from these pieces: an
  `AddressSpace`, filled by the ELF loader with `map_zeroed` + `write` +
  `protect`, a `Process`, and `user::spawn` with the default hook; the
  Linux personality's services use `copy_*_user`, `set_fs_base`,
  `user::exit`, and `AddressSpace::map_zeroed`/`unmap`/`protect` for brk and
  mmap.
- Demand paging needs one hook in the ring-3 page-fault path (and before
  the user-copy fixup): both are marked TODO(vm).
- `map_zeroed` zeroes and maps with the space's lock (and `FRAMES`) held,
  so interrupts are off for the length of the call; large mappings should
  be made in pieces until that is restructured.
- SMP needs a `PerCpu` (and GS base, TSS, PCID table) per CPU, TLB
  shootdowns when a space loaded elsewhere changes, and the FPU owner per
  CPU.
- On AMD CPUs, SYSRET can leave SS unusable if the kernel's SS was loaded
  with a null selector by an interrupt; the kernel never reloads SS, and
  QEMU does not model this, but hardware bring-up must check it.

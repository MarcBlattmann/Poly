# 0036: Demand paging and exec: a per-process memory map, faults resolved against it, ELF segments mapped lazily

**Status:** Accepted. Builds on [0022](0022-user-mode-address-spaces-and-system-calls.md)
(address spaces, the fault and user-copy paths), [0024](0024-elf-loader.md)
(load plans and the initial stack) and [0026](0026-user-address-space-regions.md)
(the region map and what a fault means).

## Context

Phase 3 runs a static Linux "hello world". Until now a user address space
was filled eagerly by hand (`map_zeroed`, `write`) and every page fault in
ring 3 killed the thread. `poly-vm` (ADR 0026) decides what a fault on a
region means and `poly-elf` (ADR 0024) plans an executable's mappings and
initial stack, but nothing in the kernel used them. The next agent writes
the first Linux system calls (`brk`, `mmap`, `munmap`, `mprotect`, ...),
and the VFS is being wired into the kernel in parallel, so files cannot be
assumed yet but must plug in later.

## Decision

**A `user::Memory` per process, shared by its threads.** It is what Linux
calls an `mm`: one `AddressSpace` (the page tables) plus the `poly_vm::VmMap`
of its regions, behind a sleeping `Mutex`. `user::spawn_in(process, memory,
entry, hook)` starts a thread in it; the thread keeps an `Arc` to it next to
its address space, so the fault handlers find it through the running
thread. Bare address spaces (`user::spawn`) still exist for the low-level
tests; a fault there is fatal, as before. The system calls will reach the
memory through `user::current_memory()`; it never lives in `Process`, so
`execve` can swap it without touching the process record.

**Faults are resolved in the kernel's two fault paths, with interrupts on.**
A page fault from ring 3 enables interrupts (the thread holds no lock),
calls `user::page_fault(addr, access)` and returns to the faulting
instruction if it succeeded; otherwise the thread is killed with the reason
(`SEGV_MAPERR`, `SEGV_ACCERR`, `SIGBUS`) in the log line, which becomes a
signal later. A fault inside a user-copy routine does the same before the
fixup is considered, but only if the copy ran with interrupts on: a copy
made under a spinlock cannot block, so it does not fault pages in and fails
with `Efault` on a page that is not present. Resolving may block on the
memory map's mutex now and on a disk read once files come from the VFS,
which is why it is a mutex and not a spinlock; nothing may copy user
memory while holding it.

**What a resolution does.** The kernel tells `VmMap::fault` whether the
page is present (every frame has one user until `fork`, so `refs` is 1) and
carries out the answer: map a fresh zeroed frame; map a fresh frame filled
from the file (always a private copy: there is no page cache, so the
"shared page-cache frame" case of ADR 0026 does not arise yet); or update
the entry's rights (a page first read from a private file mapping is
mapped read-only, as `poly-vm` asks, and a later write only upgrades it).
Copy on write is reported as unsupported until frames are shared. Frames
move between the space and the code filling them as `UserFrame`s, which
free themselves when dropped, so no path leaks a frame on error.

**Backing is a trait.** A file-backed region holds a `FileRef`: an
`Arc<dyn PageSource>` (length, `read_at`, optionally the bytes in memory)
and a limit past which pages read as zero. `MemoryImage` wraps bytes the
kernel holds; a VFS inode implements the trait later. The limit is how an
ELF segment's last page gets its zero tail without copying the file: the
segment's mapping is a window that ends at `p_offset + p_filesz`. A page
wholly past the end of the file is `SIGBUS`, as mmap(2) says. Shared file
mappings need a page cache and are refused (`ENODEV`) until there is one.

**`PROT_NONE` parks frames.** x86-64 has no present, inaccessible user
page, and a non-present entry carrying a frame would need `poly-paging` to
report such entries when a space is torn down. Instead, populated pages of
a region that becomes `PROT_NONE` are taken out of the page tables and kept
in the memory map (`parked`), mapped back when the region becomes
accessible, and freed when it is unmapped or the memory goes. A fault on
them is `SEGV_ACCERR` without looking at them, so a user copy fails cleanly.

**Exec builds a `Memory` and nothing else.** `user::exec::load(file,
params)` parses the file with `poly-elf`, refuses `PT_INTERP` (dynamic
executables come with `ld.so` support), creates one region per planned
mapping at its fixed address (`FixedNoReplace`, so a broken plan cannot
overwrite anything), starts the break at the plan's `brk`, creates a stack
region of 128 KiB (more if the arguments need it) under the top of user
space, which then grows on demand to the 8 MiB limit with `poly-vm`'s guard
gap, and writes the initial stack built by `poly_elf::build_initial_stack`
into it. Arguments and environment may take a quarter of the stack limit
(`E2BIG`, execve(2)). `AT_RANDOM` gets RDRAND output when the CPU has it,
otherwise TSC readings through a 64-bit mixer; `AT_HWCAP` is CPUID leaf 1
EDX. The thread starts at the entry point with every register 0 but RSP.
`exec::exec` adds the thread; replacing a running process's memory, ending
its other threads and closing files are `execve`'s, later.

**Test executables are assembled in the kernel.** The exec tests need real
ELF files that run in ring 3 without any C library and build the same way
everywhere. `user/test_elf.rs` writes them byte by byte with the kernel's
own assembler: ELF header, program headers (text, data with `.bss`,
optionally `PT_INTERP`, `PT_GNU_STACK`) and a program that walks its initial
stack and reports `argc`, `argv`, `envp` and every auxiliary vector entry,
then probes its data, zero tails, `.bss` and stack growth, through a test
system call. One macro produces the `ET_EXEC`, static-PIE and `PT_INTERP`
variants. `readelf -lW` reads them like any linker output.

## Consequences

- The Linux system calls are thin: `brk` is `Memory::brk`, `mmap` of
  anonymous or private file memory is `Memory::map`, `munmap` and
  `mprotect` are `Memory::unmap` and `Memory::protect`, each returning the
  errno `poly-vm` chose; frames are freed before they return.
- `mprotect` and `munmap` visit every page of their range in the page
  tables. Large mostly-empty ranges are slow; a walk that skips absent
  tables can replace it without changing the API.
- `fork` will clone the `VmMap`, share frames with reference counts and
  write-protect private pages; `CopyOnWrite` then needs carrying out, and
  parked frames need the same reference counts.
- A memory-mapped file read from a block device blocks inside the fault
  handler with interrupts on, on the faulting thread's kernel stack, which
  the design already allows.
- Faults are serialised per process by the mutex. With SMP and threads
  faulting in parallel this may become a reader-writer lock.

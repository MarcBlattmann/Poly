# 0026: User address spaces: a region map in a host-tested crate, page state in the page tables

**Status:** Accepted.

## Context

Phase 3 needs per-process address spaces: `mmap`, `munmap`, `mprotect` and
`brk` for the first Linux programs, a stack that grows on demand, and a
page-fault handler that fills pages lazily. Later, `fork` needs copy on
write and the NT personality needs `NtAllocateVirtualMemory` and section
views on the same machinery. Most of this is bookkeeping (which ranges exist,
where a new one goes, what a fault means) that does not touch hardware and is
easy to get subtly wrong at the edges: partial unmaps, protection changes
that split regions, `MAP_FIXED` over several regions, map count limits, the
end of the address space. That calls for a crate with a randomized test
against a reference model, like `poly-sched` (ADR 0012).

## Decision

**`crates/poly-vm` owns the regions, the kernel owns the pages.** A
`VmMap<F>` holds the virtual memory areas of one address space in a
`BTreeMap` keyed by start address (the crate uses `alloc`; region counts
range from a few dozen to the 65 530 limit, so no fixed table fits). Each
region has a protection, a protection ceiling (`max_prot`, so a shared
mapping of a read-only file can never become writable), a backing (private
anonymous memory, or a file or memory object at an offset, private or
shared) and a kind (ordinary, heap, stack). `F` is the kernel's file handle;
the crate clones it on splits and compares it to merge neighbours.

Nothing is stored per page in the crate. The page tables are the only record
of which pages are present. On a fault the kernel passes the page's state in
one word (absent, or present with its frame's reference count) and the crate
returns an action: zero-fill, read a file page (with whether the frame must
be a private copy), copy on write, update the entry's permissions, or
`SIGSEGV` with `SEGV_MAPERR` / `SEGV_ACCERR`. Frame reference counts,
including the page cache's own reference, live in the kernel. A private page
is writable in the page tables only while its frame has a single user
(`Vma::page_prot`), which is all copy on write needs.

**Semantics follow the man pages.** `map` takes a hint, `MAP_FIXED`
(replacing, and returning the replaced parts so the kernel can unmap their
pages) or `MAP_FIXED_NOREPLACE` (`EEXIST`); `unmap` splits and ignores holes;
`protect` fails with `ENOMEM` on any hole and `EACCES` above `max_prot`;
`brk` returns the old break on failure, as the system call does. Every
operation is checked completely before it changes anything, including the
exact region count after merging, so a failed call leaves the map as it was
(Linux may leave a partial `mprotect`; we do not). Adjacent compatible
regions are always merged.

**Layout.** User space is `0x1_0000` to `0x7fff_ffff_f000`: nothing below
64 KiB (null-pointer offsets always fault; the same floor as NT), and the
last page of the 48-bit lower half stays unmapped so a `syscall` there cannot
make `sysret` return to a non-canonical address. The stack is at the top
with a 1 MiB guard gap below it that searches, `brk` and stack growth
respect (`MAP_FIXED` may still use it). Mappings without a fixed address go
top-down from `mmap_base`, which is the stack limit (at least 128 MiB) plus
the gap below the stack top; if nothing fits there, the lowest hole anywhere
is used. A bottom-up policy exists for a classic layout. Randomisation is
the kernel's: it adjusts `stack_top`, `mmap_base` and the break start.

**Shared anonymous memory** is a shared mapping of a memory object the
kernel creates, not a separate backing kind: its pages must be found by
every process sharing it after `fork`, so they cannot live in one process's
page tables.

## Consequences

- The kernel's page-fault handler is short: translate the error code, read
  the PTE, call `VmMap::fault`, carry out the action. `mmap`, `munmap`,
  `mprotect` and `brk` are a call each plus unmapping returned regions or
  downgrading PTEs in the range, then a TLB flush.
- A page whose protection becomes `PROT_NONE` must keep its frame behind a
  non-present PTE (a software bit), so the kernel needs that encoding before
  `mprotect(PROT_NONE)` on populated memory works.
- The free-space search is linear in the number of regions in the worst
  case (it stops at the first fit going down from `mmap_base`, which is
  usually near). An augmented tree can replace it without changing the API.
- `fork` will clone the `VmMap` (it is `Clone`) and write-protect the
  private pages in the page tables; nothing in the crate changes for that.
- `MAP_GROWSDOWN`, `mremap`, `madvise`, `mlock` and `RLIMIT_AS` are not
  modelled yet.

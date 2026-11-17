# 0217: Chrome's sandbox stage failed on a full host disk; the buffer cache reads past a block it cannot write

**Status:** Accepted. Kernel block layer (`poly-block`) and tooling (track `impl-chrome-s2-r4`).
Builds on ADR 0014 (the block layer and its buffer cache), ADR 0155 (Chrome for Windows on
Poly), ADR 0206 (Chrome's sandboxed children) and ADR 0214 (Chrome in CI). Inputs: QEMU's
`blkdebug` documentation (QEMU used as a black box), Chrome's own behaviour on Poly (its
logs and Poly's exception log). No Chrome source was read.

## What was seen

After ADR 0206, stage 2 (`--dump-dom` with the sandbox on) passed 3 of 6 runs in the
integrator's gates. The failing runs looked like a race in the sandboxed children: a utility
child (`--type=utility --utility-sub-type=network.mojom.NetworkService`) died in `ntdll`'s
`KiUserExceptionDispatcher` writing below its stack, then the crashpad handler
(`chrome.exe+0x2e12e0`) and the fallback handler (`chrome.exe+0x8e002`) died too, and the
browser waited until the 300 s timeout.

## What it was

**Not a race, and not Poly's TLB handling.** Every failing run's serial log has kernel lines
`fs: writeback to /mnt/root (virtio0) failed: EIO`; no passing run has one. The host's disk
was 99 % full (Docker's disk lives in a growing file on `C:`), so writes to the guest's disk
image failed. The chain, reproduced on demand with QEMU's `blkdebug` failing every write after
the first 300 (`POLY_DISK_WRITE_ERRORS_AFTER=300`, below):

1. A database of Chrome's network service cannot be flushed (the exception log's stack text
   names `RazeNullDB` and `dbsc_session_tbl`), and Chrome ends the service on purpose with a
   write to a small address.
2. Handling that exception faults again, every time, until the thread's stack reservation is
   used up (hundreds of nested exceptions, all at `chrome.dll+0xae01df0`); at the bottom
   Poly raises `STATUS_STACK_OVERFLOW` and the dispatcher has no room left: the unhandled
   `0xc0000005` at `KiUserExceptionDispatcher+0x36` of the integrator's logs (its `r11` is
   that same `chrome.dll+0xae01df0`).
3. The crash handlers fail on the dead child and the browser waits for its network service.

The "more often under load" was the disk filling up while ~10 agents built and booted on the
same PC. Under heavy CPU load with a disk that can be written, stage 2 passed every run (QEMU
on 1, 2 or 4 shared host cores, busy loops beside it: 22 of 22, with no exception at all).
The TLB-shootdown lead of the brief was checked: changes to another process's page tables go
through `AddressSpace::sync_tlb`, which shoots down every CPU that has the space loaded and
drops its PCIDs elsewhere; nothing in the failing runs pointed there.

## Decisions

1. **The buffer cache reads past a block it cannot write.** Until now, when the cache needed
   a slot and the least recently used block was dirty and its write-back failed, the operation
   failed with the write's error, even a read of an unrelated block, while clean slots were
   free: one unwritable block at the head of the LRU list made every cache miss fail. Now the
   failed block stays dirty and cached, moves to the most recently used end (so it is tried
   again by `sync` and once per trip round the list, not on every miss), and the least
   recently used clean or empty slot is taken instead. Only when every unpinned slot is dirty
   does the operation fail. Host tests: `io_errors` (changed) and
   `an_unwritable_block_does_not_stop_reads` (fails without the change). This does not make
   Chrome survive a full disk (its database still cannot be flushed), but reads of files
   Poly already has, and of the files and pages of other programs, keep working.
2. **The Chrome stages say when the disk failed.** `chrome-win run` and the boot test groups
   count the kernel's disk `EIO` lines and print `DISK ERRORS: ...` with the reason, so a run
   on a full host is not taken for a Poly regression again.
3. **`POLY_DISK_WRITE_ERRORS_AFTER=N`** (xtask, every root-disk boot through
   `rootdisk::drive_args`): the disk image sits behind QEMU's `blkdebug`, which lets the first
   N writes through and fails every later one with `EIO`, to reproduce this.
4. **Stage 2 is required in CI**: the boot test group `chrome-win-sandbox` (shard of the same
   name, `s2-sandbox` in the `chrome` job's matrix, ADR 0214), next to stage 1 and the window
   check.
5. `console_stream` (the launcher's markers in a serial log) reads the log once per kernel-line
   prefix instead of once per kernel line: the 10 MB log of an `--exceptions` run of the failure
   took more than ten minutes to read.

## Not done

- Chrome's reaction to a database it cannot flush (step 1) and the nested fault in its exception
  handling (step 2) are Chrome's; on Windows a full disk fails the write itself
  (`ERROR_DISK_FULL`), while a guest cannot see that its host's disk is full until the write-back.
- Keep at least a few tens of GB free on the host (see the hand-back of this track).

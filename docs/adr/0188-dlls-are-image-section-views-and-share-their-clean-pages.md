# 0188: DLLs are image section views, and processes share their clean pages

**Status:** Accepted. Changes how `ntdll`'s loader maps DLLs
([ADR 0063](0063-nt-processes-object-manager-and-memory.md), `poly-ldr`) and
how the kernel backs image sections
([ADR 0155](0155-chrome-for-windows-bring-up-stubs-lazy-images-and-the-stage-ladder.md),
section 3). Found in Chrome's stage 1 (ADR 0155 step 13).

## Context

Stage 1 of Chrome for Windows (`--headless --dump-dom`) starts about ten Chrome
processes. In a 4 GiB guest the kernel ran out of frames in about half the runs
("memory allocation of 1232 bytes failed": the kernel heap takes its pages from
the same frame allocator as user memory), so step 12 raised the stages to 8 GiB.
Windows runs the same processes in far less.

A memory report (`poly.nt.mem`, below) showed where the frames went: every
Chrome process owned about 77,300 pages (302 MB) of its own from its start, the
size of `chrome.dll`'s image (`SizeOfImage` 0x12b09000). User pages reached
3.3 GiB of the 4 GiB before the panic; the page cache, the kernel heap and the
page tables were small (under 400 MiB together).

The cause was the loader, not the image sections. `poly-ldr` loaded every DLL
by reading the whole file (`Process::read_file`), laying the image out in a
second buffer, and copying it into private memory it allocated. A 310 MB
`chrome.dll` therefore cost each process a full private copy (and twice that
again while it loaded). The lazy image sections of ADR 0155 were used only for
the executable, `ntdll` and an application's own `NtCreateSection(SEC_IMAGE)`,
and even there every touched page was a private copy per process.

On Windows a DLL is a view of an image section, and views of an image are copy
on write (Learn, "Memory Protection Constants", `PAGE_WRITECOPY`: a write makes
a private copy of the page for the process), so a page nobody wrote is one
frame for every process that maps the file.

## Decision

1. **The loader maps a DLL as an image section view.** `poly_ldr::Process`
   gets `open_image(nt_path) -> ImageFile`: `Mapped { base, size, relocate }`
   or `Bytes(file)` (the default, which reads the file as before; the host
   tests' fake process uses both). `ntdll` opens the file
   (`FILE_READ_DATA | FILE_EXECUTE`), makes a `SEC_IMAGE` section of it and maps
   one view (`NtMapViewOfSection`, base 0, `ViewShare`), then closes both
   handles (the view keeps the section, the section the file). The kernel puts
   the view at the image's preferred base or, if that is taken, elsewhere with
   `STATUS_IMAGE_NOT_AT_BASE` (unchanged); the loader then makes the view
   writable (copy on write), applies the relocations in place, binds the
   imports, assigns TLS and sets each section's protection, as it did for its
   own copy. A module from a view is unmapped (`NtUnmapViewOfSection`) when it
   is unloaded or its load fails. A file that does not make an image section or
   a view is read whole as before, so a bad image fails with the same status.
   Only the pages with relocations, the import address table and written data
   become the process's own: for `chrome.dll`, 5,398 of 76,553 pages carry
   relocations.
2. **The kernel shares an image's clean pages across views**
   (`poly_personality_nt::imageshare`, `kernel/src/nt/backend.rs`). An image
   section made from a file names it by a `FileIdentity` (device, inode, size,
   modification time: a changed file is a new identity) when its bytes are the
   file's at its preferred base (`ImageBytes::share_key`: `Lazy` without
   fix-ups, and `HeldFile` for an image up to 1 MiB laid out in memory; a view
   the kernel relocated, and an image the kernel was handed as bytes, have
   none). The kernel keeps, per identity, a table of the frames of that image's
   pages that some view maps (`Registry` and `SharedPages`, both holding their
   objects weakly). The section's page source answers `shared_frame(offset)`
   from that table, or reads the page into a new frame (no lock held; a page
   another view made meanwhile wins). Memory maps it read-only in the private
   region (the `SharedFrame` path that `fork` uses, ADR 0041): a write copies
   it, or the last user takes the frame over; a frame goes with the last
   mapping, as a private copy did. Nothing is kept for an image nobody maps.
3. **`poly.nt.mem[=SECONDS]`** (off by default; `chrome-win run --mem[=N]`):
   a kernel thread logs every N seconds (default 5) the frames by user (free,
   user pages, page cache, kernel heap, the rest: page tables and kernel stacks)
   and per NT process its own, page-cache and shared pages, with the number of
   images sharing pages and how many faults found a shared page.
4. **An exception the kernel cannot allocate a `CONTEXT` for** ends the
   faulting process with `STATUS_NO_MEMORY` instead of panicking the kernel
   (`nt::exception::fault`, `try_reserve`).

## Measurements (stage 1, 4 GiB guest, `poly.nt.mem`)

| | Before | After |
|---|---|---|
| user pages, highest report | 3,372 MiB (then the kernel panicked) | 363 MiB |
| a renderer's own pages | 79,700 | 7,300-14,200 |
| a utility process's own pages at start | 77,333 | 6,200 |
| stage 1 | kernel out of memory | passed, about 15 s |

## Consequences

- Every process that loads the same DLL file shares its code and read-only data;
  a new process maps what is there instead of reading the file. Loading a big
  DLL no longer needs the file in memory.
- DLL pages are now `MEM_IMAGE` views (`NtQueryVirtualMemory`), with the file's
  name (`MemoryMappedFilenameInformation`), as on Windows; before they were
  `MEM_PRIVATE`.
- Poly's own DLLs all have the preferred base `0x180000000`, so all but the first
  are relocated (small: 3.5 MB together); so is `chrome.dll` (preferred base
  `0x180000000` too). A system-wide base per image (as Windows' ASLR keeps one)
  would let relocated pages be shared too; not needed for now.
- The kernel's exec path (the executable and `ntdll`, mapped from bytes) still
  makes private copies; they are small.

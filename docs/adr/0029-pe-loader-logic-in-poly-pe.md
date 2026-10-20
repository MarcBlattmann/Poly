# 0029: PE loader logic in a host-tested crate (`poly-pe`), working on mapped images in place

**Status:** Accepted

## Context

Phase 5 needs our own `ntdll` loader: map an executable and its DLLs,
apply base relocations, fill import address tables, set up TLS, run TLS
callbacks and entry points. On Windows the kernel maps an image section
(`NtMapViewOfSection` with `SEC_IMAGE`) and the user-mode loader in `ntdll`
then works only on the mapped view. Our kernel will need the same layout
decisions when it maps image sections, and our `ntdll` will need the
parsing, relocation and binding logic. Neither can afford a parser that
panics or reads out of bounds on a hostile file, and both run `no_std`.

The clean-room rules (`CLEAN_ROOM.md`) allow public documentation only:
Microsoft's PE Format specification, the "x64 exception handling" page and
public SDK headers. No other loader's source is read.

## Decision

- **`crates/poly-pe`** (`no_std`, no dependencies, no allocation) holds all
  PE/COFF logic that does not touch an address space: header and section
  parsing, the load plan, base relocations, imports (including delay-load
  and bound imports), exports and forwarders, TLS, load config, x64
  exception data, resources and debug directories. The kernel and the
  future `ntdll` call it; neither reimplements it.
- **One parser, two layouts.** `Pe::parse` reads a file as stored (RVAs are
  translated through the section table); `Pe::parse_mapped` reads an image
  already laid out in memory (an RVA is an offset). Every directory parser
  works on both, so the loader can parse the view the kernel mapped.
- **The plan is data.** `Pe::load_plan` validates the layout (alignments,
  ordering, overlap, sizes) once and returns regions with RVA, size, bytes
  to copy and protection; `LoadPlan::map_into` fills a caller buffer. Who
  allocates address space and applies protections stays outside the crate.
- **Changing an image works in place, without a borrow.** `relocate`,
  `bind_imports` and `bind_delay_imports` take the mutable mapped image and
  an `ImageInfo` (a `Copy` of the headers), not a `Pe` borrowing the bytes.
  `relocate` validates the whole table before writing anything, rejects
  relocation types the machine does not use, and stores the new base in
  the header so the mapped image stays self-describing.
- **Imports are resolved through a callback** that gets the DLL, the symbol
  (name with hint, or ordinal) and the slot, and returns an address or an
  error. Loading DLLs, search paths, forwarder chains and the loader lock
  are the caller's business; the crate only reports what is needed and
  writes what it is given. Names come from the lookup tables, so bound
  images are always rebound.
- **Hostile input is bounded explicitly**: at most 96 sections in an image,
  names up to 4096 bytes, at most 2^18 slots per bind call, at most 2^20
  resource entries visited and a resource depth of 8. Every loop advances
  through bounded data or a counter.

## Consequences

- The loader's decisions are unit-tested on the host against images built
  byte by byte in the tests and against real `lld-link` output (fixtures
  built from our own C sources, without any Windows SDK).
- The kernel's image-section mapping and `ntdll` share one validated view
  of what a PE image means.
- Things deliberately left out: x86 `HIGHADJ` and ARM-specific relocations,
  ARM64 `.pdata`, CLR images, certificate validation, and applying bound
  imports without rebinding. Each fails with a clear error or is simply not
  parsed.

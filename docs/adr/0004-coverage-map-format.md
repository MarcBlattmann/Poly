# 0004: Coverage map format

**Status:** Accepted; storage layout superseded by [0006](0006-coverage-map-one-file-per-component.md)

## Decision

`coverage/coverage.json` is the single source of truth. Each entry is keyed by
`<os>/<layer>/<component>/<symbol>` (for example
`windows/api/kernel32.dll/CreateFileW`) and holds a status (`missing`,
`claimed`, `stub`, `green`, `blocked-by-design`), the claiming agent, an
app-usage weight, visible and held-out test counts, and its sources.

On disk there is a small pretty-printed header and then one entry per line,
sorted by id, so every change is a one-line diff. Fields with default values
are omitted. `cargo xtask coverage check` (run in CI) enforces sorting,
uniqueness, canonical formatting and the invariants (a claim needs an agent;
green needs tests).

## Seed sources (names only)

| Source | Gives | Notes |
|---|---|---|
| Wine `dlls/*/*.spec` | Windows DLL exports; `-syscall` entries in ntdll/win32u give NT and win32k services | Skips Wine-internal modules, 16-bit modules and api-ms-win-* contract DLLs |
| Linux `syscall_64.tbl` | x86-64 syscalls (common + 64 ABIs) | x32 excluded |
| XNU `syscalls.master`, `syscall_sw.c` | BSD syscalls and Mach traps | `nosys` / `kern_invalid` slots excluded |

Each import records the upstream commit it came from.

## Not yet seeded

- macOS framework API: needs the macOS SDK `.tbd` files (from a licensed Mac).
- Real Windows export tables, to catch exports Wine does not list.
- The per-entry generated doc pages from the brief (generated in CI, not committed).

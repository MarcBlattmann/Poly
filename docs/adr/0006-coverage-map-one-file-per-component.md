# 0006: Coverage map stored as one file per component

**Status:** Accepted. Supersedes the storage layout in [0004](0004-coverage-map-format.md);
the entry fields and statuses of 0004 stay as they are.

## Context

The first layout kept every entry in one `coverage/coverage.json`. With the
Windows exports alone it was 18 MB, and mapping the macOS SDK (hundreds of
thousands of symbols, many of them long Swift names) would have pushed it past
GitHub's 50 MB warning size. One file is also a merge hotspot: every agent
that claims or finishes an entry edits the same file.

## Decision

```text
coverage/index.json                        schema version, sources, traces
coverage/<os>/<layer>/<component>.json     JSON array, one entry per line, sorted by symbol
```

For example `coverage/windows/api/kernel32.dll.json` or
`coverage/macos/api/AppKit.framework.json`. An entry stores its `symbol`; its
os, layer and component come from the path, and its id is still
`<os>/<layer>/<component>/<symbol>`. Fields with default values, including the
status `missing`, are omitted.

`cargo xtask coverage check` (run in CI) fails if any file is not in canonical
form, or if a component file exists that the map does not contain.

## Consequences

- No single file comes close to GitHub's size limits; the largest is under 1 MB.
- Agents working on different libraries change different files, so parallel
  claims rarely conflict.
- The tooling reads the whole directory, so commands and the README are unchanged.
- *Note, 2026-10-20:* [0055](0055-microsoft-redistributables-run-apps-own-copies.md)
  adds a sixth status to those of 0004, `app-provided`: "supplied by the
  application or its installer, per ADR 0055" (Microsoft's redistributable
  C++ and DirectX runtimes). It is stored like the others (`"status":"app-provided"`),
  always covers a whole component file, cannot be claimed or carry tests, and
  is left out of the README percentages and the treemap.
  `coverage set-status --component <os>/<layer>/<component> <status>` sets a
  status for every entry of a component.

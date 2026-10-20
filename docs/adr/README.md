# Architecture decision records

One file per decision: context, decision, consequences. Never rewrite an
accepted ADR. Supersede it with a new one and link both ways.

| # | Decision | Status |
|---|---|---|
| [0001](0001-multiple-personalities-one-kernel.md) | Three personalities on one Rust kernel, chosen by binary format | Accepted |
| [0002](0002-pvh-boot-first.md) | Boot via PVH under QEMU first; UEFI later | Accepted |
| [0003](0003-workspace-layout.md) | Host-testable crates + thin kernel, kernel in its own workspace | Accepted |
| [0004](0004-coverage-map-format.md) | `coverage.json` format, seeded from public name lists | Accepted |
| [0005](0005-license.md) | Apache-2.0 | Accepted |
| [0006](0006-coverage-map-one-file-per-component.md) | Coverage map stored as one file per component | Accepted |
| [0007](0007-buddy-allocator-out-of-band-metadata.md) | Buddy frame allocator with its metadata outside the memory it manages | Accepted |
| [0008](0008-in-kernel-tests-and-backtraces.md) | In-kernel tests in a linker section; frame-pointer backtraces symbolised on the host | Accepted |
| [0009](0009-drivers-behind-access-traits.md) | Device drivers as host-tested crates behind access traits | Accepted |

## Open decisions

These are also marked ⚖️ in [ROADMAP.md](../ROADMAP.md) at the point where they block work.

- **Global menu bar vs. per-window menus.** Needed before compositor work (roadmap step 4).
- **Window frames:** native per app (macOS traffic lights, Windows frames) or one neutral frame.
- **MSVC / DirectX redistributables:** 62% of Windows entries are versioned C++
  runtime DLLs (`msvcp*`, `msvcr*`, `vcruntime*`) that apps normally ship
  themselves. Decide whether we reimplement them or run the app's own copies.
- **aarch64:** port the kernel to 64-bit ARM, or stay x86-64 only. Modern Mac apps
  are increasingly arm64-only, so this decides how much of macOS is reachable.

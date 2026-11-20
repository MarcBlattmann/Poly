# macOS: the path to Chrome

The macOS work aims at one app first: Chrome for Testing Stable
157.0.8103.38 for macOS (pinned by the owner; downloaded at build time,
never committed). Order of the whole project: Chrome on Linux (works),
Chrome on Windows (in progress), then Chrome on macOS.

- [`chrome-mac-imports.md`](chrome-mac-imports.md): what the pinned build
  links and imports, by name, per library and area, against the SDK map in
  `coverage/macos/` (generated: `cargo xtask macos imports chrome-x64
  chrome-arm64 --report docs/macos/chrome-mac-imports.md`; ADR 0157).

## What the import map decides

- **x86-64 first, no translator.** The `mac-x64` build is 17 thin x86_64
  Mach-O files. Chrome on x86 PCs needs the XNU personality, our `dyld` and
  the frameworks, not the arm64 translator (ADR 0151's translator stays the
  route for arm64-only apps; ADR 0152, 0162).
- **macOS 13 and later.** Every image asks for at least macOS 13.0 (Widevine:
  12.0). Version checks (`NSProcessInfo`, `sysctl kern.osproductversion`)
  should answer 13 or later.
- **Chained fixups, plus classic binds for Widevine.** `dyld` v0 (ADR 0160)
  handles both (poly-macho reads both).
- **66 system libraries, 2,751 symbols.** Six of the libraries are linked
  without any import and only have to load.

- **Poly's own `libSystem.B.dylib`** (ADR 0258): `cargo xtask libsystem`
  builds it and the six sub-libraries a C program needs from Rust
  (`personalities/xnu/libsystem`, logic in `crates/poly-libsystem`) and
  links them with `rust-lld`'s Mach-O flavour, so any host builds them.
- **Poly's own `CoreFoundation.framework`** (ADR 0260): `cargo xtask
  corefoundation` builds it the same way (`personalities/xnu/corefoundation`,
  logic in `crates/poly-cf`); the boot test group `corefoundation` runs the
  sheets' visible tests and a C program with it.

## The order of work

| Step | ADR | What it unlocks |
|---|---|---|
| Import map | 0157 | this plan |
| A static x86_64 Mach-O runs (loader, initial stack, first BSD calls) | 0237 | the XNU personality runs code (done) |
| BSD system calls, first slice (Darwin errno, carry flag, `stat64`, `O_*`) | 0159 | a C "hello" and Chrome's early start-up |
| Our `dyld` v0 (done, ADR 0237) and a Poly-built `libSystem.B.dylib` (v0 done, ADR 0258: an SDK-linked C program runs) | 0160 | dynamically linked Mach-Os, the 66 libraries as Poly dylibs |
| Mach IPC core (ports, rights, `mach_msg`, bootstrap) | 0161 | the 85 Mach / XPC imports, Chrome's process model |
| arm64 hello through the interpreter | 0162 | arm64-only apps (not Chrome on x86) |

After these, the areas of the map in the order Chrome's start-up meets
them: libSystem / BSD (619 symbols) and the Objective-C and C++ runtimes
(61), CoreFoundation (219), Foundation (81), then Security (131),
IOSurface / CoreGraphics / CoreText (452), AppKit (384), Metal (32) and
the rest (687, mostly media, IOKit and system configuration).

## Clean room

Implementers use public documentation, SDK headers for constants and
layouts, and import and export *names* (CLEAN_ROOM.md). The Mac a
contributor builds on is a build host only: observing macOS behaviour is
the spec agent's role.

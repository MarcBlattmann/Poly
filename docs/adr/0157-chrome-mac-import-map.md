# 0157: The Chrome for macOS import map: what the pinned build links and imports, by name

**Status:** Accepted. The plan for the macOS path to Chrome (ADR 0151); it
builds on the Mach-O reader of [ADR 0153](0153-macho-reader-crate.md).

## Context

The goal of the macOS work is Chrome for macOS on Poly, after Chrome on
Linux (works) and Chrome on Windows (in progress). Before building the XNU
personality, our `dyld` and the frameworks, we need to know what Chrome
asks of macOS: which libraries it links, which symbols it imports from
each, and which of them the coverage map knows.

The owner pinned the app: Chrome for Testing Stable 157.0.8103.38 for macOS,
the `mac-x64` and `mac-arm64` builds. Like 7-Zip (ADR 0087) it is
downloaded at build time, checked against its SHA-256, and never committed
or redistributed.

The Windows side has the same tool for PE files (`cargo xtask win imports`,
ADR 0087). Import and export *names* are an allowed input for implementers
(CLEAN_ROOM.md); nothing else of the binaries is read.

## Decision

- `cargo xtask macos fetch-chrome [x64|arm64]` downloads the pinned zips
  into `target/chrome-mac/157.0.8103.38/` of the main checkout (shared by
  its worktrees), checked against size and SHA-256.
- `cargo xtask macos imports <input>... [--report <file.md>]` reads every
  Mach-O file of each input (an app bundle or directory, a zip of one, a
  single file, or `chrome-x64` / `chrome-arm64`) with `poly-macho`, every
  slice of a universal file. Per image it records the architecture, file
  type, minimum macOS and SDK (`LC_BUILD_VERSION`), the fixup format, the
  libraries it links (`LC_LOAD_DYLIB` and kin), and every imported symbol
  name with the library its ordinal names (chained fixups' import table,
  or the bind and lazy-bind streams). Imports are compared with the macOS
  framework map in `coverage/macos/api/`.
- The zip is read in memory with xtask's own ZIP reader, which gains a
  member list with Unix modes (`unzip::members`, `unzip::read`), so the
  tool runs on Windows and Linux hosts without `unzip` (ADR 0124).
- The report, `docs/macos/chrome-mac-imports.md`, is generated
  (`cargo xtask macos imports chrome-x64 chrome-arm64 --report
  docs/macos/chrome-mac-imports.md`) and committed. `docs/macos/README.md`
  draws the conclusions for the plan. The report is not checked by CI: the
  download is 400 MB and the report changes only with the pinned version.
- Symbols are grouped into areas: libSystem / BSD, Mach / XPC (libSystem's
  `mach_*`, `task_*`, `thread_*`, `vm_*`, `host_*`, `bootstrap_*`,
  `semaphore_*`, `mig_*`, `launch_*` and libxpc), the C++ and Objective-C
  runtimes, CoreFoundation, Foundation, AppKit, CoreGraphics / CoreText /
  IOSurface (with QuartzCore, CoreVideo, ImageIO, CoreImage), Security,
  Metal / OpenGL, and the rest.

## What the map says

- **The `mac-x64` build is x86-64 only**: all 17 Mach-O files are thin
  x86_64 images, no universal files. A first run of Chrome for macOS on x86
  PCs needs no arm64 translator. The `mac-arm64` build is arm64 only.
- **Minimum macOS 13.0** (`LC_BUILD_VERSION` of the app, its framework and
  helpers), built with SDK 27.0; the Widevine CDM says 12.0.
- **Chained fixups everywhere but in Widevine**, which uses the classic
  bind opcodes: our `dyld` needs both from the start.
- **66 system libraries and frameworks, 2,751 distinct imported symbols**
  (arm64: 2,755; the two builds differ in 44 symbols). The framework
  dylib imports 2,697 of them; the helpers, the crashpad handler and the
  bundled libraries (`libvk_swiftshader`, `libvulkan`, `libaperitif`,
  Widevine) the rest.
- **By area** (x64): libSystem / BSD 619, Mach / XPC 85, C++ and
  Objective-C runtimes 61, CoreFoundation 219, Foundation 81, AppKit 384,
  CoreGraphics / CoreText / IOSurface 452, Security 131, Metal / OpenGL 32,
  other frameworks 687 (IOKit 78, VideoToolbox 57, CoreMedia 55, libcups
  51, SystemConfiguration 36, ...). Six libraries are linked but nothing is
  imported from them (Cocoa, MetalKit, Quartz, ServiceManagement,
  libpmenergy, libpmsample); they still have to load.

## The map's own gaps

119 imports (x64) are "not in the map" for the library they come from,
and 7 more are in the map under another library. Apart from one weak
import from the private SkyLight framework, these are gaps of the map's
seed (`tools/coverage`, `import.rs`), not of macOS:

- `sdk_component` drops private frameworks. That is right for private API,
  but the SDK's `.tbd` stubs inline the private libraries a public
  framework re-exports, and apps link those symbols through the public
  framework: AppKit's UIFoundation (`NSFontAttributeName`, ...),
  AudioToolbox's AudioToolboxCore (`AudioConverterNew`, ...),
  AVFoundation's AVFCore and AVFCapture (no `AVFoundation.framework`
  component exists at all), AuthenticationServices.
- The seed reads the `exports:` lists only, not `reexports:`: libsystem_c
  re-exports `memcpy`, `strlen` and the other string functions.

Fixing the seed (credit a re-exported private library's symbols to the
public framework that re-exports it, read `reexports:`) changes the macOS
counts in the README, so it is its own change, done next.

## Consequences

- The macOS work has a measured target: the report lists, per area and
  library, every symbol Chrome needs, and becomes the checklist for the
  personality, `dyld`, libSystem, libobjc and the frameworks.
- Chrome on x86 PCs takes the x86_64 path first (ADR 0158 to 0161); the
  arm64 translator (ADR 0152, 0162) is not on Chrome's critical path.
- When the pinned version changes, the report is regenerated with the same
  command.

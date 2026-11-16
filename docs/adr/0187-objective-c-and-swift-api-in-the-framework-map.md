# 0187: The Objective-C and Swift API of the macOS frameworks in the coverage map: `clang -extract-api` for selectors, `.swiftinterface` files for Swift, names only, next to the `.tbd` exports

**Status:** Accepted (the owner told the coordinator to go ahead). The second
half of the Phase 9 item "macOS test oracle (licensed Mac); extend the
framework map with Objective-C methods and Swift API"; the oracle half is
[ADR 0182](0182-macos-test-oracle.md), so the item is ticked. Builds on the
framework map of [ADR 0157](0157-chrome-mac-import-map.md) and
[ADR 0163](0163-macos-map-credits-re-exports.md) (`tools/scripts/import-macos-sdk.sh`,
`coverage import macos-sdk`).

## Context

The framework layer of the macOS map (`coverage/macos/api/`) holds what the
SDK's `.tbd` stubs export: C functions and globals, the mangled names of
Swift symbols (`$s10Foundation...`) and the names of exported Objective-C
classes (`objc:NSString`). That is the linker's view. An Objective-C program
does not link against `-[NSString substringFromIndex:]`: it sends a message,
so nothing in a `.tbd` says which methods a framework has, and a mangled
Swift name cannot be read back as `URL.appendingPathComponent(_:)`. Poly has
to implement `AppKit`, `Foundation` and the Swift overlays themselves (Phase
9 and 10), and the owner and the agents need the list of what exists, by the
names a developer knows, to claim and tick it off.

Two public, supported sources describe that API without opening any binary:

- **The public headers** of each framework. `clang -extract-api` (Apple
  clang, run as a black box like the compilers of CLEAN_ROOM.md) writes
  Apple's documented JSON *symbol graph* of the declarations in the headers
  it is given: classes, protocols, methods with their selectors, properties,
  enum cases, with availability. On this Mac (Apple clang 21) Foundation
  gives 7,307 symbols in 2.7 s and AppKit 12,153 in 4 s.
- **The `.swiftinterface` files** the SDK ships beside each Swift module
  (`Foundation.swiftmodule/arm64e-apple-macos.swiftinterface`): the textual,
  documented format for a module's public API (`public func`, `public
  struct`, ...). 265 of them for macOS in the SDK, about 19 MB.

Both are public SDK material (CLEAN_ROOM.md: exported names, SDK headers);
neither is a decompiled binary. The binary `.swiftmodule` and `.swiftdoc`
files, the dynamic libraries and the shared cache are not read.

## Decision

### Only names, as everywhere in the map

An entry says that a thing exists. No declaration, type, documentation or
header text is stored; `coverage/` keeps ids and the key of the source. What
the tools read from the headers (a symbol graph) and the interfaces (text)
stays in a scratch directory or in the SDK.

### Entry ids

Below `macos/api/<component>/`, where the component is the framework that
declares the API (`AppKit.framework`):

```text
objc:NSString                            a class or protocol (classes the .tbd lists are the same entries)
objc:NSString.substringFromIndex:        an instance method: owner, ".", selector
objc:NSString+stringWithFormat:          a class method: owner, "+", selector
objc:NSString.@length                    an instance property: owner, ".@", name
objc:NSString+@defaultCStringEncoding    a class property: owner, "+@", name
swift:Foundation.URL                     a Swift type, nested types by path: Foundation.URL.Bookmark
swift:Foundation.URL.appendingPathComponent(_:)   a function: name and argument labels
swift:Foundation.URL.init(fileURLWithPath:)       an initializer
swift:Foundation.URL.path                a property; subscript(_:), typealiases, enum cases the same way
swift:Swift.min(_:_:)                    a free function
```

- Objective-C: the owner is the class or protocol the selector belongs to.
  The methods of a **category** are credited to the class it extends, in the
  framework whose headers declare the category (`-[NSString
  drawAtPoint:withAttributes:]` is in `AppKit.framework`). A property's
  getter and setter selectors are not listed separately: the property is
  the entry. Enum cases, macros, C functions, global variables and ivars of
  the headers are **not** imported: constants and C symbols that exist at
  run time are already in the map from the `.tbd`, the others are compile
  time facts. Symbols the SDK marks unconditionally unavailable on macOS
  are left out.
- Swift: the id starts with the module that **declares** the API, so what
  `Foundation` adds to `Swift.String` is `swift:Foundation.String.<name>`.
  The member is its name with the argument labels, as Swift writes them
  (`_` for no label). Overloads that differ only in types are one entry (the
  map holds names, not signatures). A subscript's argument is a label only
  when it is written twice. Declarations that are not `public`/`open`, that
  carry `@_spi`, or that are `@available(macOS, unavailable)` or
  `@available(*, unavailable)` are left out; every member of a public
  protocol counts, the cases of a public enum too.

### How they sit next to the `.tbd` entries

They are new entries in the same component files, written by new importers
through the same `Coverage::merge`: an existing entry keeps its status,
claim and tests and gains the new source in `src` (the 4,449 classes that
both the `.tbd` and the headers know now list `macos-sdk` and
`macos-objc-api`), new entries are `missing`. Two new sources in
`coverage/index.json`, `macos-objc-api` and `macos-swift-interface`, with the
SDK revision. A module with no framework of its own is placed like this
(`swift::component`): the Swift modules in `usr/lib/swift` go to the Swift
library of that name that the map already has (`Metal` to `libswiftMetal.dylib`,
`Swift` to `libswiftCore.dylib`), else to a component `<Module>.swiftmodule`
(two, `hvf` and `MobileGestaltPrivate`). Frameworks that only the new
sources know (`WebKit`, `JavaScriptCore`, `SafariServices`,
`AuthenticationServices`, `CoreAudioTypes`, `LiveExecutionResultsRuntime`
and two SwiftUI cross-import modules) become components of their own.

### The importers

```sh
tools/scripts/import-macos-api.sh [<MacOSX.sdk>]    # on a Mac; one command for both
cargo xtask coverage import macos-objc <dir of symbol graphs> --revision REV
cargo xtask coverage import macos-swift <MacOSX.sdk> --revision REV
```

- `tools/scripts/import-macos-api.sh` is the Mac-only wrapper, next to
  `import-macos-sdk.sh`. It takes the SDK (default `xcrun --sdk macosx
  --show-sdk-path`), reads its version and build from the SDK's
  `SystemVersion.plist` (so the revision is `macOS SDK 27.0 (26A425)` as
  the `.tbd` import wrote it), runs `tools/scripts/extract-objc-api.sh` for
  every framework in `System/Library/Frameworks` with public headers, ten
  at a time (`xcrun clang -extract-api -x objective-c-header -target
  arm64-apple-macosX.Y -isysroot SDK -F SDK/System/Library/Frameworks
  -fobjc-arc`, the umbrella header first, then every other header directly
  in `Headers/`, since the tool describes only the files it is given), into
  a scratch directory of `<Framework>.framework.json` symbol graphs that is
  removed on exit; then both imports, then `coverage readme`.
- `tools/coverage/src/objc.rs` reads the graphs (kind and path components
  of each symbol, `availability`), `tools/coverage/src/swift.rs` reads the
  interfaces: a comment and string sanitiser, then one pass over the
  declaration lines that tracks braces (types and extensions open a scope
  whose members are read, bodies and non-public types are skipped,
  `#if` branches are all read, a declaration may wrap over lines). Both are
  plain Rust with unit tests on fixtures (a symbol graph, an interface that
  has generics, operators, subscripts, nested types, extensions with `where`
  clauses, protocol requirements, `@available`, `@_spi`, `#if`); the
  importers need only the files, not a Mac, but the files come from one.
- **When a new SDK ships:** run the script again with the new SDK (both
  `import-macos-sdk.sh` and this one, the same SDK for both, so the
  revisions match), review `git diff --stat coverage`, commit. Entries that
  disappeared from the SDK stay in the map (imports only add and refresh,
  as for every source); removing them is a decision for the owner.

### Scale

Measured on the SDK 27.0 import (the existing map had 874,150 entries,
85 MB on disk):

| | |
|---|---|
| Objective-C entries | 66,172 (5,747 classes and protocols, 26,401 instance and 6,289 class methods, 26,849 instance and 886 class properties), in 178 components |
| Swift entries | 77,617, in 215 components (from 265 interfaces) |
| New entries in the map | 139,340 (the other 4,449 Objective-C entries were already there as classes): 1,013,491 in all, 98 MB on disk |
| Largest file | 5.5 MB (`SwiftUICore.framework.json`), as before |
| `cargo xtask ci coverage` / `coverage readme` | 9 s / 5.5 s (before: 3 to 9 s) |
| the whole import script | 2 min 51 s, of which `clang -extract-api` 2.5 min |
| `cargo xtask ci lint` | unchanged (10 s warm) |

The macOS framework row of the README goes from 741,217 to 880,557 entries.
No sharding and no cut to the first GUI frameworks was needed. Rerunning an
import changes nothing (0 new, the same files).

### What is not in the map (and why)

- **Frameworks `clang -extract-api` could not read** (4): `DriverKit`
  (C++ headers), `Kernel` (kernel headers that need a kernel SDK), `Tk`
  (X11 headers), `OpenGL` (C only). None has Objective-C API an app uses.
  The script lists the failures when it runs.
- **Frameworks with public headers and no Objective-C symbols** (45):
  umbrellas (`Cocoa`, `Carbon`, `ApplicationServices`, `Quartz`,
  `CoreServices`, `Accelerate`) and C or Swift-only frameworks
  (`ImageIO`, `OpenAL`, `SwiftUI`, `RealityKit`, ...). The API of their
  sub-frameworks is in those components.
- **Private frameworks**, as for the `.tbd` import.
- **Swift**: only `arm64e-apple-macos` interfaces are read (the `x86_64`
  ones describe the same API; `#if` branches that differ by architecture
  are all taken), the macCatalyst/iOS-support interfaces (`System/iOSSupport`,
  one) are skipped. An extension's *conformances* and a declaration's types
  are not recorded; neither is which overload exists.
- **Objective-C relationships** (inheritance, protocol conformance, optional
  and required methods): the symbol graph has them, the map does not keep
  them. A spec agent reads the headers for that.

## Consequences

- An implementer or spec agent can see exactly what `NSWindow`,
  `NSString` or `URL` should have (names only), claim `objc:NSWindow.makeKeyAndOrderFront:`
  or a whole component, and a test names the same id. `coverage set-status
  --component macos/api/AppKit.framework ...` works as before.
- The framework layer is 16% larger; every `cargo xtask coverage` command
  still takes seconds.
- New: `tools/coverage/src/objc.rs`, `swift.rs`, `tools/scripts/import-macos-api.sh`,
  `extract-objc-api.sh`, the sources `macos-objc-api` and
  `macos-swift-interface`, 10 new component files.
- The roadmap item is ticked: the oracle (ADR 0182, with its recorded
  specs) and the framework map with Objective-C and Swift API are both done.

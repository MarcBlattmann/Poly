# 0260: Poly's own CoreFoundation.framework, v0

**Status:** Accepted. Track `impl-macos-cf`, the CoreFoundation step of `docs/macos/README.md` (ADR
0157's plan). Builds on Poly's dyld (ADR 0237), the Mach core (ADR 0238), the BSD layer (ADR 0246) and
Poly's own libSystem (ADR 0258). Inputs: the 46 CoreFoundation spec sheets and their visible tests
(`specs/macos/CoreFoundation.framework`, handed over by `spec-macos-cf`), Apple's public CoreFoundation
documentation, the SDK headers `CoreFoundation/*.h` for constants, callback tables and the `CFSTR`
layout, the install name, versions and export names of the SDK's `CoreFoundation.tbd`
(`coverage/macos/api/CoreFoundation.framework.json`), the Unicode Consortium's public mapping tables
(Mac OS Roman, Windows-1252, NeXTSTEP) and character data, RFC 3986 (URLs) and RFC 4648 (base64). No
part of Apple's CoreFoundation, CF-Lite or swift-corelibs-foundation was read, disassembled or run; clang,
Apple's ld and `rust-lld` were used as black boxes.

## What was there

Programs linked against the SDK's CoreFoundation could load through Poly's dyld only if Apple's
framework were present, which Poly may not ship. Chrome imports 219 CoreFoundation symbols (ADR 0157).

## Decisions

1. **The same install name, Poly's code.** One Mach-O dylib,
   `/System/Library/Frameworks/CoreFoundation.framework/Versions/A/CoreFoundation`, compatibility
   version 150, current version 5027.0.69 (the SDK's `.tbd`), exporting 296 names, every one a name of
   the SDK's CoreFoundation: the 46 entries of the sheets and what their visible tests and Chrome's
   start-up need around them (mutable strings and collections, `CFURL`, `CFDate`, `CFAllocator`, run loop
   observers, the remaining standard callback tables). It links only Poly's `libSystem.B.dylib`.
2. **Built like libSystem.** `personalities/xnu/corefoundation` is a `no_std` crate with `alloc` (its
   heap is libSystem's `malloc`), its own workspace, built for `x86_64-apple-darwin` with
   `-Zbuild-std=core,compiler_builtins,alloc` and linked by `rust-lld -flavor darwin` against Poly's
   libSystem from a small system root (`cargo xtask corefoundation`), on any host. Cached by content
   hash and read back with `poly-macho` (install name, versions, only libSystem loaded, exactly the
   listed exports, all SDK names). The logic is in the host-tested `crates/poly-cf`: encodings,
   comparison options, the format engine, number conversions, XML property lists, URL text, bundle
   paths and the bundle version number, hashes and description text.
3. **Objects keep a class word first, the type in CoreFoundation's own header.** Every object starts
   with a pointer-sized class word (a class record of this framework with room for an Objective-C
   class structure), then a header word with the `CFTypeID` and flags, the retain count and the
   allocator. `CFGetTypeID` reads the header, never the class word, so the Objective-C runtime (ADR 0259)
   can later put bridged classes there and treat CF objects as objects (toll-free bridging) without
   changing CoreFoundation's functions. A `CFSTR` constant is laid out by the compiler (sheet
   `__CFConstantStringClassReference`: class word, flags 0x07C8 or 0x07D0, characters, length) and is
   recognised by its class word; exported string constants (`kCFRunLoopDefaultMode`, the preferences
   and bundle keys) use the same layout.
4. **Reference counts as the sheets describe.** Constants (`kCFNull`, the Booleans, `CFSTR` strings,
   `kCFNumberNaN` and the infinities, the standard allocators) are never counted or freed and report
   0x0FFFFFFFFFFFFFFF; other objects start at 1 and are freed through the allocator they were made with,
   one allocation per object (variable storage comes from `malloc`), after a collection released its
   elements through its callbacks. NULL arguments where the sheets say the process dies end it with an
   invalid instruction (SIGILL, the x86-64 observation); what the sheets call an Objective-C exception
   prints the reason and ends the process (there is no runtime to catch it yet).
5. **Strings are UTF-16 code units.** Decoding is strict for UTF-8 (overlong forms, surrogates, values
   past U+10FFFF and truncation give NULL; a leading byte order mark is dropped), UTF-16 and UTF-32 follow
   the byte-order rules of the `CFStringCreateWithBytes` sheet, and the single-byte encodings use the
   public tables. `CFStringGetCString` is all or nothing. `CFStringCompare` compares units as unsigned
   numbers, with case folding, digit runs, canonical decomposition, diacritics and full-width forms on
   request (Latin, Greek and Cyrillic tables). `CFStringCreateWithFormat` uses libSystem's `printf`
   engine for numbers (`poly_libsystem::printf`) and adds `%@`, `%C`, `%S`, `%s` as Mac Roman,
   positional arguments of every kind, unknown letters written as themselves and a trailing `%` dropped;
   the variadic entry points build a `va_list` the way libSystem's `printf` does.
6. **Numbers keep their fixed-size type** and compare, hash and convert by value as the sheets say:
   integers hash to |v| × 2654435761, a whole double like the integer, a Boolean like 0 or 1; narrowing
   wraps and answers exact, a fraction truncates and answers inexact, an out-of-range double gives the
   x86-64 conversion's smallest integer.
7. **The run loop sleeps on a Mach port of its own.** Each loop allocates a receive right
   (`mach_port_allocate`) and waits in `mach_msg` with a receive timeout until the next timer or the
   run's deadline. A run follows the order the sheets record (Entry, BeforeTimers, BeforeSources,
   BeforeWaiting, AfterWaiting, Exit): signalled version 0 sources by their order value, then the wait,
   then due timers; it ends when a source was handled (if asked), on `CFRunLoopStop`, at the timeout, or
   when the mode has nothing left. The main loop's default mode counts as never empty. v0 has one thread,
   so `CFRunLoopWakeUp` comes from the loop's own thread and only makes its next wait return at once;
   waking from another thread will send to the port once the kernel has a send path for programs
   (`mach_msg2_trap`, not done in ADR 0238).
8. **Bundles from the executable's path.** The main bundle is `X.app` for an executable at
   `X.app/Contents/MacOS/name`, else the executable's directory with an empty Info dictionary.
   `Info.plist` is read as an XML property list (integers become 64-bit numbers) and gains
   `CFBundleNumericVersion`, the documented `CFBundleVersion` packing (two BCD digits, minor and bug-fix
   nibbles, stage, revision). Resources are looked up in `Resources[/sub]`, then `Base`, `en` and
   `English` `.lproj` directories.
9. **Preferences live in the process for v0.** Domains are dictionaries of deep immutable copies; the
   global domain answers `AppleLanguages` = ("en") for every application; `kCFPreferencesCurrentApplication`
   is the main bundle's identifier; synchronising answers true. Writing
   `~/Library/Preferences/<domain>.plist` comes with a preferences service of Poly's own (the sheets
   leave the file and its timing out of the behaviour).
10. **A clock for CoreFoundation.** The XNU personality now answers BSD call 116 (`gettimeofday`) in
    Poly's form: a `struct timeval` (64-bit seconds, 32-bit microseconds) through the first argument,
    the time zone argument ignored, result 0. No sheet covers it yet; Poly's own libraries are its only
    callers, programs reach the clock through them. `CFAbsoluteTimeGetCurrent` is that time minus
    978307200 s.

## Tests

- `crates/poly-cf`: encodings and their tables, every comparison option, the format engine (integers,
  floats, `%@`, positional arguments), number conversions, hashes, property lists, URL text, bundle paths
  and version numbers.
- `personalities/xnu`: `gettimeofday` against the fake kernel.
- `tools/xtask/src/corefoundation.rs`: the export list against the SDK, the test list and recordings,
  the runner's output parsing, the build's inputs.
- Boot test group `corefoundation` (shard `userland`): all 75 visible tests of the sheets, built on a Mac
  by `tests/macos/corefoundation/generate.sh` with clang and Apple's ld against the SDK, run on Poly
  through Poly's dyld, libSystem and CoreFoundation by `/bin/cf-runner`; each must exit 0 and print
  exactly what macOS printed on x86-64. The four tests that build `T.app` and re-execute themselves find
  the application laid out in the image and are started inside it (the kernel has no `mkdir` or
  `execve` for Mach-O programs yet). `cfdemo.c`, a program of our own, prints through `printf` what it
  does with CFString, CFArray, `CFStringCreateWithFormat` and a repeating run loop timer that stops its
  loop.

## Waiting for other work

- **libSystem (PR #434, ADR 0258):** this branch builds on it; its `posix_spawn` passes the six C
  arguments to BSD call 244, which takes five (pid, path, one descriptor, argv, envp), so a child gets
  an empty argv; the boot test's runner calls `__posix_spawn` directly until that is fixed.
- **The Objective-C runtime (ADR 0259):** Apple's CoreFoundation re-exports `libobjc.A.dylib`; ours does
  not yet. Bridged classes in the class word, `CFAutorelease` with real pools, and Objective-C exceptions
  for the cases the sheets mark come with it.
- **Threads** (`bsdthread_create` in libSystem): run loops per thread, locks, waking another thread's
  loop over its port (with `mach_msg2_trap`).
- **The kernel:** `mkdir` and `execve` for Mach-O programs, so the bundle tests can build and enter
  `T.app` themselves.
- The 296 entries stay `claimed` by `impl-macos-cf`; the 46 with sheets go green through the
  pipeline's held-out tests, not here.

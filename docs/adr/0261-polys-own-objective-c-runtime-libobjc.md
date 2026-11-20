# 0261: Poly's own Objective-C runtime, `libobjc.A.dylib`

**Status:** Accepted. Track `impl-macos-objc`, Roadmap Phase 9 "Objective-C runtime (`libobjc`)
and Swift runtime" (the Objective-C half, v0). Builds on ADR 0237 (Poly's dyld), ADR 0246 (the BSD
layer) and ADR 0258 (Poly's own `libSystem.B.dylib`, whose build it follows). Inputs: the spec
sheets and visible tests of `specs/macos/libobjc.A.dylib` (PR #431), the SDK headers
`<objc/runtime.h>`, `<objc/message.h>` and `<objc/NSObject.h>` (constants, function prototypes),
the export names of the SDK's `libobjc.tbd` (`coverage/macos/api/libobjc.A.dylib.json`), the
clang Blocks ABI document, and the output of clang and the linkers for our own Objective-C sources,
run as black boxes (see "The metadata layout"). No source of Apple's runtime (objc4), of clang or
LLVM, or of any other Objective-C runtime was read; no Apple binary was disassembled.

## The metadata layout

The compiler writes classes, methods and categories into the image in records the runtime must
read. Neither the public headers nor the sheets give their layout, so it was learned by compiling
our own `@implementation`s with `clang -S` (the emitted data, with its symbol names) and reading the
linked files back with `otool`, `nm` and `llvm-objdump` (`crates/poly-objc/src/abi.rs` documents
each record). One observation of the linkers matters: Apple's `ld` rewrites method lists into a
relative form in `__TEXT,__objc_methlist` (flags word with bit 31, 12-byte entries of three `i32`
offsets from their own fields; the name offset reaches a selector reference), and it merges a
category without `+load` into its class when both are in one image; `rust-lld` does neither. The
runtime reads both forms. The runtime's own state (tables, caches, counts) is Poly's design and
never kept in the compiled records.

## Decisions

1. **Logic in a host-tested crate, `crates/poly-objc`;** the dylib (`personalities/xnu/libobjc`) is
   a thin shell. The crate reads metadata through a `Mem` trait (raw pointers in the dylib, a
   simulated address space in tests) and keeps the tables: selectors (a selector is the address of
   its canonical name; `sel_getName` is the identity), classes by class-object address, methods as
   stable handles (`Method`), categories, protocols, images, the method cache, the resolver's
   once-per-(class, selector) set, and the `+load` order. Memory management tables (counts, weak
   references, associations, pools, `@synchronized` locks) are in `poly_objc::arc`.
2. **Built like libSystem:** `no_std` Rust (with `alloc`, over libSystem's `malloc`) for
   `x86_64-apple-darwin` with `-Zbuild-std`, linked by `rust-lld -flavor darwin` against Poly's
   libSystem (through a sysroot of its libraries), with Apple's install name and versions
   (`/usr/lib/libobjc.A.dylib`, 228 / 1) and an export list that is a subset of the SDK's
   (`cargo xtask libobjc`, cached by content hash, verified with `poly-macho`).
3. **Start-up:** an initializer (Poly's dyld runs it after libSystem's) maps every image of
   libdyld's image table, libraries first (the reverse of the load order), and runs each image's
   `+load` methods. `NSObject` is ordinary compiled metadata of this library: static class objects
   in the compiler's layout, exported as `_OBJC_CLASS_$_NSObject` and `_OBJC_METACLASS_$_NSObject`
   and listed in the library's own `__objc_classlist`, so `class_getImageName` names libobjc.
4. **Mapping an image:** selector references are uniqued in place; protocols, classes (with their
   metaclasses) and categories are registered; a category's methods go in front of the class's (the
   last category first: it wins dispatch, and both copies stay in `class_copyMethodList`); a
   category whose class is in an image not mapped yet waits for it. `+load` follows the
   `_objc_load_image` sheet, including its exception (a superclass without `+load` runs its
   categories' `+load` in the superclass chain).
5. **Message sending (x86-64):** `objc_msgSend`, `_stret`, `_fpret`, `objc_msgSendSuper`,
   `objc_msgSendSuper2` and their `_stret` forms are assembly that saves every argument register,
   `rax` and `xmm0`–`xmm7`, looks the method up and jumps to it. nil short-circuits (zero in `rax`,
   `rdx`, `xmm0`, `xmm1`; the stret buffer untouched; `st0` 0.0 for `_fpret`) except for super
   sends. The first message to a class sends `+initialize` (superclasses first, once each); a
   selector the class lacks asks `+resolveInstanceMethod:` / `+resolveClassMethod:` once; then the
   unrecognized-selector line and SIGABRT. Every change to a method list empties the cache.
6. **Reference counting:** the compiler's entry points send `-retain`, `-release`, `-autorelease`
   (overrides see each call); NSObject keeps counts beyond the first in a side table, sends
   `-dealloc` when the last goes, and `-dealloc` runs `.cxx_destruct` of the class chain, zeroes the
   weak references, releases the associations and frees the object. The return-value handshake of
   the sheets is done by reading the caller's code: `mov %rax, %rdi` followed by a call (direct
   through a stub, or through the GOT) to `objc_retainAutoreleasedReturnValue` or a claim sibling
   hands the object over unpooled.
7. **Pools, weak references, associations, `@synchronized`** as the sheets record them: pools are
   per thread, tokens are stack positions (nested pools differ, a push after a pop reuses the
   popped token), a pop releases newest first and continues with objects its releases add; weak
   locations are registered per object and zeroed at death, storing one to a dying object aborts;
   associations by pointer key with the five policies (atomic ones retain-and-autorelease on get);
   recursive per-object locks with `-1` for an exit without an enter.
8. **Exceptions, v0:** `objc_exception_throw` ends the process the way an uncaught exception does
   (`libc++abi: terminating due to uncaught exception of type NAME`, SIGABRT). Catching needs an
   unwinder (C++'s), which Poly's libSystem does not have yet; the personality routine and
   `OBJC_EHTYPE_id` are exported so programs with `@try` load.
9. **One lock** guards the tables and is never held across a call into the program. Threads work
   (thread ids from `thread_selfid`) but contention spins.

## Tests

- `crates/poly-objc`: unit tests of the tables, and `tests/images.rs`: our `classes.m` linked by
  Apple's `ld` (relative lists, merged categories) and by `rust-lld` (plain lists), loaded by Poly's
  dyld into a simulated memory and mapped: the three `+load` order examples of the sheet in one
  image, superclass and metaclass links, category dispatch and method list order, selector
  uniquing.
- Boot test group `objc` (shard `userland`): `tests/macos/libobjc/hello.m`, compiled with ARC and
  linked the ordinary way against the SDK, runs on Poly with Poly's dyld, libSystem and libobjc
  (23 checks: classes, a category, a super send, `+load`/`+initialize`, ARC strong and weak, the
  return-value handshake, pools, run-time methods and classes, associations, the sync locks).
- Boot test group `objc-spec` (shard `userland`): every visible test of the libobjc sheets (98),
  built for x86-64 as `pipeline mac build` builds them (`tests/macos/libobjc/spec`), run on Poly in
  one boot by a small runner that `posix_spawn`s each (ADR 0246); each must print the protocol lines
  macOS printed (the arm64 recording). 85 pass: 80 equal the recording, 5 are x86-64 parts (the arm64
  struct-return checks become the x86-64 `_stret` ones, BOOL is `c`) whose own checks all pass. The
  other 13 are listed with their reason and wait for pieces outside libobjc: an unwinder (caught
  exceptions, `_Unwind_Resume` in cleanups), threads (`pthread_create`), the Blocks runtime,
  `dlopen`/`dlsym`.

## Not in v0

Caught exceptions; `imp_implementationWithBlock` and blocks (no `libsystem_blocks` yet); tagged
pointers; properties and ivar API beyond the basics; `objc_copyClassNamesForImage` and the
other image queries; `dlopen` of images with Objective-C metadata; arm64 entry points (they come
with the translator's path, PR #429); the Swift runtime.

# Clean-room rules

These rules are **non-negotiable**. They decide whether this project can be
distributed at all. Every contributor, human or agent, follows them. If a
rule is unclear, stop and ask; do not guess.

## Why

Code derived from decompiled or disassembled Windows or macOS binaries is a
derivative work of those binaries and cannot be distributed. Code copied from
GPL projects would force the kernel under the GPL. The only safe input to our
implementation is **observed behaviour** written up as a spec.

## Allowed and forbidden inputs

| Source | Spec agent | Implementer agent |
|---|---|---|
| Running the real OS in a VM, observing inputs/outputs/errors/ordering/timing | ✅ | ❌ |
| Public documentation (MSDN/Learn, Apple developer docs, man pages, format specs) | ✅ | ✅ |
| Exported *names* (DLL export tables, SDK headers, `.tbd` files, Wine `.spec` lists) | ✅ | ✅ |
| Public SDK headers for constants, struct layouts and error codes (`ntstatus.h`, `errno.h`, ...) | ✅ | ✅ |
| Spec sheets and **visible** tests in `specs/` | writes them | ✅ reads them |
| **Held-out** tests | writes them | ❌ never |
| Open-source code under permissive licences (XNU/APSL, BSD, MIT) for *behaviour* | ✅ read | ❌ |
| Linux source (GPL), Wine source (LGPL), ReactOS (GPL) for *behaviour* | ✅ read | ❌ |
| Microsoft's STL **headers** (`github.com/microsoft/STL`, `stl/inc`, Apache-2.0 WITH LLVM-exception) for the class layouts and member behaviour behind `msvcp140*.dll` (ADR 0199; see "Microsoft's STL headers") | ✅ read; only facts go into sheets | ❌ never |
| Running third-party tools as black boxes: compilers, linkers and assemblers (clang, lld, gcc) to build test programs and fixtures; emulators (QEMU, qemu-user) to run them; comparing only their inputs and outputs | ✅ | ✅ (outputs only, never their source) |
| Third-party apps under test (Chrome, 7-Zip, ...): running them, their logs, their import/export *names*; downloaded at build time, never committed or redistributed | ✅ | ✅ (running on Poly; not their source) |
| Decompiling, disassembling or debugging Microsoft/Apple binaries' internals | ❌ | ❌ |
| Leaked source code of any kind | ❌ | ❌ |
| Copying code from any project into this repo | ❌ | ❌ |

Tools used as black boxes are not inputs to our code: what they produce is
either a test artefact (a fixture, a test program) or an observation (an
output to compare against), never something copied into Poly. Their licences
(GPL for QEMU and gcc) do not reach Poly because nothing of them is linked or
copied in. Their *source* stays off limits to implementers like any other
third-party source.

## Role separation

1. The **spec agent** observes the real OS and writes `specs/<os>/<component>/<symbol>.yaml`
   plus tests, split about 70% visible and 30% held out.
2. The **implementer agent** runs in a separate context and only ever receives the
   spec sheet and the visible tests. It never sees the real OS, the held-out
   tests, or any third-party source.
3. Every hand-off is logged, one file per record under `specs/handoffs/` (and,
   for older ones, in the legacy `specs/HANDOFF_LOG.md`): who, when, which spec
   revision, which files, so the separation can be proven later.
4. One agent identity never plays both roles for the same entry.

## Licences of what we read

- **Linux (GPL-2.0)**: reading for behaviour is fine; no code, comments or
  structure copied into this repo. Syscall *names and numbers* are ABI facts.
- **Wine (LGPL-2.1)**: its conformance tests are a great oracle *for the spec
  agent*, and its `.spec` files are used as export *name* lists. Nothing else.
- **XNU (APSL-2.0)**: syscall and trap *names and numbers* are ABI facts. Read
  for behaviour only.
- **Licensed copies only** of Windows and macOS are used as test oracles.
- **Microsoft's STL (Apache-2.0 WITH LLVM-exception)**: see the next section.
  None of it is reused in Poly (owner decision, ADR 0199).

## Microsoft's STL headers (owner decision, 2026-11-15, ADR 0199)

Poly's `msvcp140*.dll` is clean-room Rust, but programs compiled against
Visual C++'s headers read the fields of the library's classes and call
through their virtual tables directly, so Poly has to match those layouts
exactly. Therefore:

1. **Spec agents only** may read the headers of Microsoft's STL
   (`stl/inc` of `github.com/microsoft/STL`, or the same headers as Visual
   Studio installs them) to learn the layout and behaviour of what
   `msvcp140*.dll` exports.
2. What goes into a spec sheet are **facts**: sizes and alignments, field
   offsets and their types, the order of virtual functions, the values of
   constants and flags, exported data and its initial value, and what a member
   does, in the spec agent's own words, checked where possible by probe
   programs compiled on the Windows oracle (`sizeof`, `offsetof`, observed
   calls). **Never code**: no function bodies, no declarations or class
   definitions copied, no comments, no identifiers beyond the exported names
   and the member names a layout needs.
3. **Implementers never read these headers**, nor any other part of
   Microsoft's STL; they get the sheets, as for every other entry.
4. The decision covers the headers only; nothing of the STL is compiled
   into, copied into or shipped with Poly. Anything beyond the headers
   (its sources, `stl/src`) goes to the owner first.

## Red flags: stop and escalate

- A spec or test that contains code rather than behaviour.
- An implementation that reproduces undocumented internal structure that
  could only have come from looking inside a binary.
- Any file whose origin you cannot explain.

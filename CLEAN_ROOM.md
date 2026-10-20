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
| Decompiling, disassembling or debugging Microsoft/Apple binaries' internals | ❌ | ❌ |
| Leaked source code of any kind | ❌ | ❌ |
| Copying code from any project into this repo | ❌ | ❌ |

## Role separation

1. The **spec agent** observes the real OS and writes `specs/<os>/<component>/<symbol>.yaml`
   plus tests, split about 70% visible and 30% held out.
2. The **implementer agent** runs in a separate context and only ever receives the
   spec sheet and the visible tests. It never sees the real OS, the held-out
   tests, or any third-party source.
3. Every hand-off is logged in `specs/HANDOFF_LOG.md` (who, when, which spec
   revision, which files), so the separation can be proven later.
4. One agent identity never plays both roles for the same entry.

## Licences of what we read

- **Linux (GPL-2.0)**: reading for behaviour is fine; no code, comments or
  structure copied into this repo. Syscall *names and numbers* are ABI facts.
- **Wine (LGPL-2.1)**: its conformance tests are a great oracle *for the spec
  agent*, and its `.spec` files are used as export *name* lists. Nothing else.
- **XNU (APSL-2.0)**: syscall and trap *names and numbers* are ABI facts. Read
  for behaviour only.
- **Licensed copies only** of Windows and macOS are used as test oracles.

## Red flags: stop and escalate

- A spec or test that contains code rather than behaviour.
- An implementation that reproduces undocumented internal structure that
  could only have come from looking inside a binary.
- Any file whose origin you cannot explain.

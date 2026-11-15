# 0206: Random numbers come from a Poly system call, not from a device file

**Status:** Accepted. Works toward stage S2 of the Chrome-for-Windows ladder
([0155](0155-chrome-for-windows-bring-up-stubs-lazy-images-and-the-stage-ladder.md)). Follows the
security model ([0171](0171-windows-security-model-tokens-descriptors-and-access-checks.md)) and the
sandbox rules ([0192](0192-chrome-s2-window-stations-per-process-mitigations-and-known-dlls-under-a-token.md),
[0196](0196-appcontainers-lowbox-tokens-two-pass-access-checks-and-image-views-under-prohibit-dynamic-code.md)).
Nothing here was read from Chrome or from Microsoft's binaries; the cause was found with Poly's own
call trail.

## Context

`RtlGenRandom` (`SystemFunction036`), `ProcessPrng` (what `BCryptGenRandom` and Chromium's `RandBytes`
reach), `BCryptGenRandom` and the C run time's `rand_s` all drew their bytes by opening
`Z:\dev\urandom` (the kernel's generator, which Poly shows as a file of the virtual file system) and
reading it. A sandboxed Chrome child lowers its token after it starts: from then on its restricted
token (one restricting SID, Low integrity) or its lowbox token cannot open a file whose descriptor does
not name one of its SIDs, and the random device is such a file. The first thread that wanted a random
number after the lowering (Chrome seeds its own generator on a new thread) got `STATUS_ACCESS_DENIED`
from the open, and Chrome stopped with a failed check (a handled breakpoint at `chrome.dll+0x420b2`, in
every sandboxed child: the S2 blocker after the null calls of ADR 0196's follow-up were fixed).
On Windows the system-preferred generator is a driver reached by a system call, never a file the token
has to be allowed to open.

## Decision

* **A Poly system call** `PolyRandomBytes(Buffer, Length)` (the Poly table, number 3 after
  `PolyResumeApc` and `PolyDebugPrint`) fills the caller's buffer from the kernel's generator
  (`NtKernel::random_bytes`; the kernel's `random::fill`), in pieces of 4 KiB; no access check but the
  buffer's own (`STATUS_ACCESS_VIOLATION`), `STATUS_NOT_IMPLEMENTED` from a kernel that has no
  generator. The 32-bit table has a row for it (two arguments).
* `ntdll` exports the stub like `PolyDebugPrint`, for Poly's own DLLs only (no program knows it).
  `advapi32!SystemFunction036` and the C run time's `rand_s` call it; `bcryptprimitives!ProcessPrng` and
  `bcrypt` reach it through `SystemFunction036` as before. No file is opened any more, so a process
  with any token (restricted, lowbox, impersonating) draws random numbers.
* `Z:\dev\urandom` stays a file for programs that open it by name (it is Poly's Linux side).

## Consequences

* The sandboxed children get past the failed check at `chrome.dll+0x420b2`; the next blocker is
  recorded in the hand-off of the round.
* A Poly-only entry in `ntdll`'s export table (like `PolyDebugPrint`), and a service no Windows program
  can call by accident (the Poly table).
* Nothing else of `\Device` or the object namespace changes: the descriptor of the device file is not
  special-cased.

# 0222: UAC on Poly: split tokens and a consent prompt

**Status:** Accepted (owner decision 2026-11-17). Track `impl-chrome-installer` (batch B0 of
`docs/windows/chrome-installer-plan.md`). Builds on ADR 0171 (tokens, descriptors and access
checks), ADR 0128/0131 (win32k's desktop link to the compositor), ADR 0186 (32-bit programs) and
ADR 0155 (fail-only exports). Inputs (clean room, `CLEAN_ROOM.md`): the spec sheets and visible
tests of `spec-win-elevation` (`specs/windows/advapi32.dll/{UacStandardToken,UacElevatedToken,
ObjectIntegrityLabel}`, `kernel32.dll/CreateProcessElevation`, `shell32.dll/{IsUserAnAdmin,
ShellExecuteExW.Elevation,ShellExecuteW,ShellExecuteA}`); Microsoft Learn ("How User Account
Control works", "Access Tokens", "Mandatory Integrity Control", "Application Manifests",
`GetTokenInformation`, `TOKEN_LINKED_TOKEN`, `CheckTokenMembership`, `IsUserAnAdmin`,
`ShellExecuteExW`, `CreateProcessW`, "System Error Codes"); the PE format specification ("The
Attribute Certificate Table"); RFC 2315/5652 (PKCS #7 `SignedData`); `winnt.h`, `winerror.h`,
`ntstatus.h`, `shellapi.h`, `wintrust.h`. Nothing of Wine, ReactOS or Chromium was read; nothing
was run on Windows.

## Context

Until now every Windows program on Poly ran as the elevated administrator of ADR 0171: the token
the older spec sheets were recorded with (the SSH logon's, which has no split). Google's Chrome
installers ask `IsUserAnAdmin` and, when it is FALSE, start themselves again with the verb
`runas`; Windows then shows its consent prompt. The owner decided (2026-11-17) that Poly does
what Windows does: **ordinary programs run without administrator rights, and a program that asks
for them gets Poly's own consent prompt.**

## Decision

### The split token (`poly-security`)

- `Token::uac_filtered()` is what a program of an administrator runs with: Medium integrity
  (`S-1-16-8192`, `0x60`), `BUILTIN\Administrators` and `S-1-5-114` deny-only (`0x10`), the five
  privileges of a standard user (`SeChangeNotify` enabled, `SeIncreaseWorkingSet`,
  `SeShutdown`, `SeTimeZone`, `SeUndock` present), `TokenElevation` 0, `TokenElevationType` 3
  (Limited), `TokenHasRestrictions` 1, virtualization allowed. It belongs to the user (owner) and
  its default DACL gives the user and SYSTEM all and the logon SID read and execute.
- `Token::uac_full()` is what an elevated program has: High integrity, Administrators enabled
  and owner (`0xF`), 24 privileges with only `SeChangeNotify`, `SeImpersonate` and
  `SeCreateGlobal` enabled and `SeDebug` at 2, `TokenElevation` 1, type 2 (Full).
- Each half carries the other (`Token::linked`). `TokenLinkedToken` (class 19) makes a new token
  object, the other half as an **impersonation token at identification level** linked back
  (`Token::linked_token`). It can be queried and set on a thread. It cannot be used to act:
  `DuplicateTokenEx` to a higher level fails with 1346, and `IsUserAnAdmin` is FALSE under it.
  The full token as the filtered one's link shows `SeDebug` 0 (the sheet's linked token).
- `Logon` names the logon the programs run in: `standard` (the filtered token, **the default**),
  `elevated` (the full token) and `admin` (the ADR 0171 administrator without a split). The kernel
  reads `poly.win.token=standard|elevated|admin` and keeps it in `NtSystem::logon`. A process no
  process made (the first one, a program the desktop starts) gets `logon.token()`. Children
  inherit their creator's token as before (ADR 0171).

### Elevation without the user is refused

`NtCreateUserProcess` with a given primary token (`CreateProcessAsUserW`) of a **higher**
integrity level than the creator's own token is `STATUS_PRIVILEGE_NOT_HELD`. Two exceptions:
- the creator holds `SeAssignPrimaryTokenPrivilege` or `SeTcbPrivilege` enabled (a service of
  the system);
- the token carries the **consent mark** (`Token::consented`). Only the consent broker sets that
  mark, and starting a process uses it up. A copy of a token never has it.

So a filtered program cannot reach the full token: the linked token is identification-level, and
an elevated process's token is labelled High (no-write-up, ADR 0171).

### Integrity labels

As ADR 0171 made them, now visible with two levels:
- a process and its threads get `S:AI(ML;;NWNR;;;ME|HI)` at their creator's level;
- the token and a job get `S:AI(ML;;NW;;;ME|HI)`;
- events, mutexes, semaphores, sections, pipes, files and keys a program creates get no label
  (ObjectIntegrityLabel.yaml).

`GetFileSecurityW` and `RegGetKeySecurity` now return the descriptors through
`NtQuerySecurityObject`.

### Asking for administrator rights

- **The manifest** (`poly_kernelbase::elevation`): the level is `requestedExecutionLevel` of the
  image's `RT_MANIFEST` 1. An image without one uses the external `<image>.manifest`. Neither
  means `asInvoker`.
- **`CreateProcessW`** of an image that requests `requireAdministrator`, or `highestAvailable`
  when the user is an administrator (a Limited token), from a token that is not elevated fails
  with `ERROR_ELEVATION_REQUIRED` (740) and creates nothing (`kernelbase/elevate.rs`).
  `CreateProcessW` never asks the user.
- **`ShellExecuteExW` / `ShellExecuteW`** are ADR 0223's (`shell32/exec.rs`: programs,
  documents by association, folders through the desktop). Its program path now also takes the
  verb `runas` and calls `shell32/elevate.rs` (`caller_elevated`, `start_elevated`) for these
  rules; `ShellExecuteA` and `IsUserAnAdmin` are `elevate.rs`'s:
  - First the call is checked: a missing file is 2 / `SE_ERR_FNF`, a verb other than `open`,
    `runas` or none is 1155 / `SE_ERR_NOASSOC`, a wrong `cbSize` is 5. No prompt comes before
    these pass.
  - `runas` from a caller that is not elevated asks the **consent broker**. So does `open` that
    `CreateProcessW` refuses with 740.
  - Yes: the program starts with the full token the broker handed out (`CreateProcessAsUserW`).
  - No: `ERROR_CANCELLED` (1223) and `hInstApp` 5, and nothing starts. `ShellExecuteW` returns 5.
  - An elevated caller starts the program without a prompt.
  - `IsUserAnAdmin` is TRUE when the effective token has Administrators enabled. An
    impersonation token below the impersonation level answers FALSE. The last error is kept.
  - A document opened with `runas` goes by its association as before (the verb's command).

### The consent broker (`personalities/nt/src/consent.rs`)

`PolyRequestElevation(Path, PathLength, Token*)` is Poly's own service (`TABLE_POLY` 6, exported
by both `ntdll`s). The calling thread waits on an event until an answer comes. Three things can
answer, in this order:

1. **The test hook `poly.uac.answer=yes|no`.** It answers at once. It is never on by default; the
   pipeline sets `no` for tests recorded with the standard token, because the oracle's prompt
   went unanswered.
2. **The desktop.** The kernel sends `ToCompositor::Consent { request, pid, program, path, file }`
   over the desktop link, and the compositor answers `FromCompositor::Consent { request, allow }`.
3. **Nobody.** Without a compositor on the link the answer is No. A link that closes answers its
   open prompts No, and a process that ends drops its requests.

A Yes gives the caller a handle to a primary full token with the consent mark
(`Token::elevated_for_consent`). An elevated caller gets its own token without a prompt, and a
token that has no full token gets `STATUS_ACCESS_DENIED`. Every request and its answer is logged
(`uac: request N for <path>: yes|no`).

### The prompt (`poly-shell::consent`, the compositor)

- **Drawn by the compositor, never by the app.** The whole screen is dimmed (black at 55 %). A
  card in the middle, in Alpine's bar colour with its ring and radius, asks "Allow this app to
  make changes to your device?". It names the program (the file name of the path the kernel
  read), the publisher and the full path, with **Yes** and **No**.
- **The texts come from the kernel's record.** The app's windows or messages play no part.
- **Only the real devices answer.** While a prompt shows, the compositor's input routing gives
  every key and button to the prompt and none to any client (`Desktop::consent_input`), so an
  app's synthetic input (win32k's `SendInput` goes to win32k's queues, not to the compositor)
  cannot answer it.
- **Keys.** No has the focus first, so Enter on an unexpected prompt declines. Tab and the arrows
  move the focus, Enter answers, Escape is No. Prompts asked while one shows wait their turn.
- **The publisher** is read by the compositor from the file in Poly's file system
  (`IoState::vfs_path_of_dos`). It is the signer of the Authenticode certificate table
  (`poly_x509::authenticode`: the PE security directory, PKCS #7 `SignedData`, the first signer's
  certificate subject, CN else O), shown as "Publisher: <name> (signature not verified)". An
  unsigned or unreadable file shows "Publisher: Unknown".

### 32-bit fail-only exports pop their arguments

A 32-bit `stdcall` function pops its own arguments. The fail-only exports of Poly's i686 modules
(`fail_exports!`) took none, so a call left the stack wrong. That is how Google's 32-bit stub
jumped to a stack address (survey stop 1). Now:
- on i686, `fail_exports!` makes a naked stub that calls the body, then pops the bytes
  `x86_signatures.txt` gives the function (`poly_stubs::x86slots::stack_bytes`, a `const fn`;
  the same file is compiled into `poly-win32-rt` by path);
- a function the table does not know logs and ends the process with `STATUS_NOT_IMPLEMENTED`, as
  the 32-bit `polystub.dll` does, instead of returning wrong;
- the installers' functions (SCM, COM server side, locale and time, WTS, shell) were added to
  the table from their Learn prototypes;
- the `win32`/`wow64` module caches hash the table and the lookup.

### Tests and boot tests

- **`win poly`** gives each visible test the token it was recorded with:
  - `standard` (plus `poly.uac.answer=no`) or `elevated` from `windows.json`;
  - `admin` for tests with no token kind (all older recordings, the SSH logon).
- **Recorded as admin.** The kernel self-tests (`a32test`, `b35test`, `s1test`, `sys32` and the
  NT file, layout and registry programs), the 7-Zip steps and the recorded scenarios
  (`wintrace`) run with `poly.win.token=admin`. They check API behaviour as the elevated
  administrator they were recorded and written for. Under the filtered token they write HKLM,
  `C:\Windows` and `Program Files` and are refused (5, 1300), as on Windows.
- **Un-elevated.** Chrome for Windows (every `chrome-win*` group), the desktop groups,
  `desktop-apps` and `git-win` run with the default, the filtered token.

## Effects on today's apps

| What | Before | Now |
|---|---|---|
| Windows programs started by the desktop or as the first process | elevated admin | filtered (Medium) token; `IsUserAnAdmin` FALSE |
| A program asking for admin (`runas`, `requireAdministrator`) | ran elevated | Poly's consent prompt; No → 1223 / `hInstApp` 5 |
| Kernel self-test programs, 7-Zip steps, `wintrace` | elevated admin | the same, by `poly.win.token=admin` |
| Chrome for Windows (all groups), desktop groups, `desktop-apps`, `git-win` | elevated admin | un-elevated; results in the hand-back |

## Not done

- **The publisher's signature is not verified.** The image digest and the chain to a trusted root
  are not checked yet; the prompt says "not verified".
- **The COM elevation moniker** (`Elevation:Administrator!new:`) is not done; it belongs with
  batch B2 (out-of-process COM).
- **Missing cases.** Credentials for a user who is not an administrator, the legacy installer
  name rule for 32-bit images without a manifest, and the session id of the interactive logon
  (all tokens keep session 0).
- **`cmd.exe`.** `CreateProcessElevation`'s `cmd.exe /c exit 3` lines fail: Poly has no
  `cmd.exe`.

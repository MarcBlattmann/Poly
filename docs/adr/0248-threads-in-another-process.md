# 0248: Threads in another process

**Status:** Accepted. Track `chrome-feel` (round 3); sheets `specs/windows/kernel32.dll/CreateRemoteThread.yaml`,
`CreateRemoteThreadEx.yaml` and `specs/windows/ntdll.dll/NtCreateThreadEx.yaml` (PR #389). Builds on
[ADR 0065](0065-nt-threads-synchronisation-and-apcs.md) (NT threads, `NtCreateThreadEx` in the calling process),
[ADR 0071](0071-our-ntdll-a-real-pe-dll-built-from-rust.md) (`LdrInitializeThunk`) and
[ADR 0089](0089-createprocess-handle-inheritance-startupinfo-create-suspended.md) (processes created suspended). Inputs: the sheets, Microsoft
Learn ("CreateRemoteThread", "CreateRemoteThreadEx", "Thread Stack Size", "EnumProcessModules",
"GetProcessIdOfThread"), `winternl.h` for the loader-list fields, and traces of Chrome for Windows running on Poly.
No third-party source.

## Context

Chrome for Windows' browser calls `kernel32!CreateRemoteThread` 15 to 19 times per run (the sheet's recorded
Windows trace, all successful). On Poly it was a stub that failed with `ERROR_CALL_NOT_IMPLEMENTED`, and
`NtCreateThreadEx` refused any process but the caller's (`STATUS_NOT_IMPLEMENTED`).

What Chrome does with it, logged on Poly with a temporary replacement of the stub:

- The **browser** makes the thread in a **renderer it has just created**, each time from a fresh browser thread.
- The start address is always the same **internal `chrome.dll` function** (`chrome.dll+0xbff650`, not an export).
- The parameter is an address in the target; flags 0; default stack.
- When the call failed, Chrome went on.

So the address the creator passes is one it found in its own process and expects at the same place in the child.

## Decisions

1. **`NtCreateThreadEx` makes a thread in the process its handle names.**
   - The kernel lays out the thread's stack and TEB in that process's memory map, puts the thread among that
     process's threads, and opens the handle in the caller's table.
   - It needs exactly `PROCESS_CREATE_THREAD` on the handle.
   - A process that has ended or is ending is `STATUS_PROCESS_IS_TERMINATING`.
   - A 32-bit caller cannot make a thread in a 64-bit process (`STATUS_ACCESS_DENIED`).
   - The sheet's other rules are kept: `DesiredAccess` (0 refused; generic mapping and the limited rights that
     come with full ones), the `CreateFlags` bits (0x1 suspended, 0x2 skip attach, 0x4 hidden from debuggers,
     0x8–0x40 accepted, others `STATUS_INVALID_PARAMETER_7`), stack sizes (decision 5), `OBJ_INHERIT`, the
     attribute-list length, and a `ThreadHandle` that cannot be written (`STATUS_ACCESS_VIOLATION` before
     anything is made).
2. **A process created suspended keeps a hold while its first thread waits.**
   - A remote thread may come and go before the first thread starts (the sheet's `process_not_started`; Chrome's
     renderers are created suspended).
   - The kernel process therefore does not end when its last *live* thread leaves while the hold is there.
   - Starting the first thread gives the hold up. Ending the process gives it up too: `NtTerminateProcess` of
     it, a group exit, or `ExitProcess` run by a remote thread.
3. **Whichever thread runs first starts the loader.**
   - `LdrInitializeThunk` now gets the executable's base in RDX for every thread, so a remote thread in a process
     that has not started initialises it.
   - A later thread waits until that initialisation is done, then gets `DLL_THREAD_ATTACH`.
   - Bit 0 of RDX is Poly's mark for `SKIP_THREAD_ATTACH`. That thread gets neither attach nor detach callouts.
     Its TLS is still set up.
4. **A remote thread's stack is reserved from the top of the address space, as its TEB is.**
   - In a process whose loader has not run, a stack reserved low down took the space where the loader then put
     the DLLs it relocates (`kernelbase.dll`).
   - Those DLLs then sat at other addresses than in the creator, and the creator's start address (`ExitThread`
     in the test; `chrome.dll` code for Chrome) pointed elsewhere.
5. **Stack sizes follow the sheets and Learn's "Thread Stack Size".**
   - `StackSize` is the commit and `MaximumStackSize` the reserve.
   - The maximum counts only from 64 KiB and above the commit, and is rounded up to 64 KiB.
   - A commit at or above the reserve makes the reserve the commit rounded up to 1 MiB, all of it committed,
     with no guard page.
   - A size of all ones is `STATUS_INVALID_PARAMETER`.
   - `CreateThread` gets the same rules.
6. **Threads have records in the NT state** (`NtSystem::thread_records`: process, exit status, hidden flag),
   kept while the thread object exists.
   - `NtQueryInformationThread` answers a thread of another process from its process, or from the record once
     that process is gone. `GetProcessIdOfThread` and `GetExitCodeThread` on a remote thread therefore work,
     also after its process has ended.
   - `ThreadHideFromDebugger` (class 0x11, one byte) reads the hidden flag.
   - `NtResumeThread`, `NtSuspendThread` and `NtTerminateThread` act on the thread's own process.
7. **kernel32's rights are its own.**
   - `CreateRemoteThread(Ex)` uses a handle with `PROCESS_CREATE_THREAD` and either `PROCESS_QUERY_INFORMATION`
     or both VM rights as it is.
   - Otherwise it duplicates the handle with `PROCESS_CREATE_THREAD | PROCESS_QUERY_INFORMATION`, which the
     process's descriptor allows or refuses (refused: `ERROR_ACCESS_DENIED`, `STATUS_ACCESS_DENIED`).
   - A handle to something other than a process is `STATUS_OBJECT_TYPE_MISMATCH`.
   - Every failure sets the thread's last NT status too.
   - `CreateRemoteThreadEx` takes the thread attributes: the group affinity is applied, the ideal processor is
     accepted, and a process attribute is `ERROR_INVALID_PARAMETER`.
8. **`GetProcessIdOfThread` and `K32EnumProcessModules` are implemented.** The sheets' tests need them.
   `K32EnumProcessModules` reads the target's `PEB.Ldr` in-memory-order list (`winternl.h` fields).

## Consequences

- Chrome's `CreateRemoteThread` calls succeed.
- Visible tests: all of `CreateRemoteThread`, `CreateRemoteThreadEx` and `NtCreateThreadEx` pass except
  `rmt_sb_appcontainer`. That test needs `SetFileSecurityW` (a stub without a sheet) to make its image
  readable for AppContainers.
- Not done:
  - `GetThreadIdealProcessorEx` and `GetThreadGroupAffinity` stay stubs; the ideal processor is accepted and not
    stored.
  - The contents of `PS_ATTRIBUTE_LIST` are not used.
  - A thread made in another process is not added to a job's thread count.
- The 64-bit-caller-into-32-bit-target case goes through the same path (the TEB, the stack and the start
  convention of the target's layout). It was observed on Windows but has no recorded test.

## Addendum (chrome-feel-4): a thread's descriptor, ideal processor and affinity

Integration round A found two held-out failures: `rmt_rights_dacl` ("a rights check against a thread's security
descriptor") and `rmte_attrs` (a thread attribute not read back). The sheets of `GetThreadIdealProcessorEx` and
`GetThreadGroupAffinity` (PR #399) say what the attributes must read back as.

- **Every new thread has a descriptor.**
  - Learn, "CreateRemoteThread": "The access control lists (ACL) in the default security descriptor for a thread
    come from the primary token of the creator".
  - Threads made by `NtCreateThreadEx` had none unless the caller gave one, so nothing could be read from or
    checked against their descriptor. They now get the creator's default, after the creator's own handle is
    opened, also in another process.
  - The sheet's process-rights rules were checked separately on Poly: 5 descriptors × 36 handle-right
    combinations, all 180 results as the sheet says.
- **A thread's record keeps its affinity and its ideal processor**, so a thread that is suspended, not started
  or finished has both.
  - `ThreadAffinityMask` accepts only processors the system has (Learn, "SetThreadAffinityMask"). The kernel
    applies a mask set before the thread starts when it starts.
  - `ThreadIdealProcessorEx` (class 33, a `PROCESSOR_NUMBER`) reads the set one while it lies in the mask,
    else one spread by thread id inside the mask. Setting it returns the previous one in the same buffer
    (Poly's own convention with kernelbase).
- **kernelbase:**
  - `GetThreadIdealProcessorEx` and `GetThreadGroupAffinity` need a query right;
  - `SetThreadIdealProcessorEx` needs `THREAD_SET_INFORMATION`;
  - `SetThreadGroupAffinity` needs `THREAD_SET_INFORMATION` and a query right; mask 0 means the process mask;
  - `CreateRemoteThreadEx` refuses an ideal processor or group affinity the system does not have (no thread is
    made: `ERROR_INVALID_PARAMETER`, `STATUS_INVALID_PARAMETER`) and applies valid ones to the new thread.

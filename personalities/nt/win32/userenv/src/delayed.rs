//! The names Chrome's `chrome.dll` delay-loads from `userenv.dll` that Poly does not implement yet (ADR
//! 0155, step 9): each is an export that logs `stub: userenv!name` and fails. A delay load of a name
//! the DLL does not export ends the browser, whatever the call would have been used for; a
//! failing call it copes with (the fault-injection table of the gap map). An implementation
//! replaces a name here by defining it elsewhere in the crate and deleting it from the list.

poly_win32_rt::fail_exports!(
    "userenv",
    win32,
    [EnterCriticalPolicySection, LeaveCriticalPolicySection, CreateEnvironmentBlock, DestroyEnvironmentBlock]
);

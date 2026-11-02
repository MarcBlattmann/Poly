//! Poly's win32k system calls (ADR 0128, Decision 2).
//!
//! Windows renumbers its win32k services with every build and documents
//! none of its `NtUser*` / `NtGdi*` entry points; applications call the
//! documented `user32` and `gdi32` functions. So, as for the executive
//! services (ADR 0063), Poly owns the table: `win32u.dll`'s stubs are
//! generated from [`WIN32K_SERVICES`] (its `build.rs` reads this file), the
//! NT personality dispatches table 1 by it, and `cargo xtask win32` checks
//! that every stub carries its number.
//!
//! - A number is `TABLE_WIN32K << 12 | index` (`0x1000 | index`), the layout
//!   of every NT service number; only the low 14 bits count.
//! - Names start with `PolyUser` or `PolyGdi`, never Windows' `NtUser` /
//!   `NtGdi`: a program that calls a Windows `win32u` export directly must
//!   fail to bind rather than reach a Poly service with arguments it reads
//!   differently. Windows names are added to `win32u` only as adapters, once
//!   a spec sheet documents one (ADR 0128).
//! - Each service takes the parameters of the documented function it serves,
//!   in the documented order (noted on each variant); results that are not
//!   an `NTSTATUS` are written through a pointer, as `Nt*` services do.
//!
//! | Indices | Group |
//! |---|---|
//! | `0x000`–`0x0FF` | classes and windows |
//! | `0x100`–`0x1FF` | messages and queues |
//! | `0x200`–`0x2FF` | input: focus, activation, capture, keyboard state, cursor |
//! | `0x300`–`0x3FF` | timers and hooks |
//! | `0x400`–`0x4FF` | painting and surfaces |
//! | `0x800`–`0xFFF` | GDI services that need the kernel |
//!
//! This list is the first slice's starting point: until a service ships
//! with an implementation its arguments may still be reshaped; after that
//! its number is never reused.

/// The table number of win32k services (bits 12–13 of a service number).
pub const TABLE_WIN32K: u32 = 1;

/// One win32k service Poly knows.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Win32kService {
    // ----- classes and windows
    /// `RegisterClassExW(const WNDCLASSEXW*)`, plus whether the class's
    /// procedure is Unicode; the atom through a pointer.
    RegisterClass,
    /// `UnregisterClassW(lpClassName, hInstance)`.
    UnregisterClass,
    /// `GetClassInfoExW(hInstance, lpszClass, WNDCLASSEXW*)`.
    GetClassInfo,
    /// `CreateWindowExW(dwExStyle, lpClassName, lpWindowName, dwStyle, X, Y,
    /// nWidth, nHeight, hWndParent, hMenu, hInstance, lpParam)`; the window
    /// handle through a pointer.
    CreateWindow,
    /// `DestroyWindow(hWnd)`.
    DestroyWindow,
    /// `ShowWindow(hWnd, nCmdShow)`.
    ShowWindow,
    /// `SetWindowPos(hWnd, hWndInsertAfter, X, Y, cx, cy, uFlags)`.
    SetWindowPos,
    /// What a window is (rectangles, styles, parent, owner, thread, text
    /// length, ...): `(hWnd, class, buffer, length, return length*)`, an
    /// information class of Poly's, like `NtQueryInformation*`.
    QueryWindow,
    /// `GetWindowLongPtrW(hWnd, nIndex)`, the value through a pointer.
    GetWindowLong,
    /// `SetWindowLongPtrW(hWnd, nIndex, dwNewLong)`, the previous value
    /// through a pointer.
    SetWindowLong,
    /// `SetWindowTextW(hWnd, lpString)`.
    SetWindowText,

    // ----- messages and queues
    /// `PostMessageW(hWnd, Msg, wParam, lParam)`.
    PostMessage,
    /// `PostThreadMessageW(idThread, Msg, wParam, lParam)`.
    PostThreadMessage,
    /// `SendMessageTimeoutW(hWnd, Msg, wParam, lParam, fuFlags, uTimeout,
    /// lpdwResult)` to a window of another thread (same-thread sends never
    /// reach the kernel).
    SendMessage,
    /// `GetMessageW(lpMsg, hWnd, wMsgFilterMin, wMsgFilterMax)`, with the
    /// caller's callout record.
    GetMessage,
    /// `PeekMessageW(lpMsg, hWnd, wMsgFilterMin, wMsgFilterMax, wRemoveMsg)`,
    /// with the caller's callout record.
    PeekMessage,
    /// `ReplyMessage(lResult)`.
    ReplyMessage,
    /// `PostQuitMessage(nExitCode)`.
    PostQuitMessage,
    /// `WaitMessage()`.
    WaitMessage,
    /// `MsgWaitForMultipleObjectsEx(nCount, pHandles, dwMilliseconds,
    /// dwWakeMask, dwFlags)`.
    MsgWaitForMultipleObjects,
    /// `GetQueueStatus(flags)`, the result through a pointer.
    GetQueueStatus,
    /// The end of a callout (ADR 0128, Decision 1): `(callout, result)`.
    CalloutReturn,

    // ----- input
    /// `SetFocus(hWnd)`, the previous focus through a pointer.
    SetFocus,
    /// `SetActiveWindow(hWnd)`, the previous active window through a pointer.
    SetActiveWindow,
    /// `SetForegroundWindow(hWnd)`.
    SetForegroundWindow,
    /// `SetCapture(hWnd)`, the previous capture through a pointer.
    SetCapture,
    /// `ReleaseCapture()`.
    ReleaseCapture,
    /// `GetKeyState(nVirtKey)`, the state through a pointer.
    GetKeyState,
    /// `GetAsyncKeyState(vKey)`, the state through a pointer.
    GetAsyncKeyState,
    /// `GetKeyboardState(lpKeyState)`.
    GetKeyboardState,
    /// `GetCursorPos(lpPoint)`.
    GetCursorPos,
    /// `SetCursor(hCursor)`, the previous cursor through a pointer.
    SetCursor,

    // ----- timers and hooks
    /// `SetTimer(hWnd, nIDEvent, uElapse, lpTimerFunc)`, the timer's id
    /// through a pointer.
    SetTimer,
    /// `KillTimer(hWnd, uIDEvent)`.
    KillTimer,
    /// `SetWindowsHookExW(idHook, lpfn, hmod, dwThreadId)`, the hook handle
    /// through a pointer.
    SetWindowsHook,
    /// `UnhookWindowsHookEx(hhk)`.
    UnhookWindowsHook,

    // ----- painting and surfaces
    /// `BeginPaint(hWnd, lpPaint)`, plus where the window's surface is.
    BeginPaint,
    /// `EndPaint(hWnd, lpPaint)`.
    EndPaint,
    /// `InvalidateRect(hWnd, lpRect, bErase)`.
    InvalidateRect,
    /// `ValidateRect(hWnd, lpRect)`.
    ValidateRect,
    /// `GetUpdateRect(hWnd, lpRect, bErase)`, the answer through a pointer.
    GetUpdateRect,
    /// `GetDCEx(hWnd, hrgnClip, flags)`'s kernel part: the window's surface
    /// section, its origin in it and its visible region.
    GetDC,
    /// `ReleaseDC(hWnd, hDC)`'s kernel part.
    ReleaseDC,
    /// Poly's own: `(hWnd, rects, count)`, the rectangles drawn through a
    /// window DC since the last commit, which become `Damage` records of
    /// the desktop link (ADR 0128, Decision 5).
    CommitSurface,

    // ----- GDI
    /// The display as GDI reports it (`GetDeviceCaps` of a screen DC,
    /// monitors): `(class, buffer, length, return length*)`.
    QueryDisplay,
}

/// Every win32k service with its number and its name as `win32u.dll`
/// exports its stub. The numbers are written out (`0x1000 | index`) so
/// `win32u`'s `build.rs` can read them as they are. Sorted by number.
pub const WIN32K_SERVICES: [(Win32kService, u32, &str); 45] = [
    (Win32kService::RegisterClass, 0x1000, "PolyUserRegisterClass"),
    (Win32kService::UnregisterClass, 0x1001, "PolyUserUnregisterClass"),
    (Win32kService::GetClassInfo, 0x1002, "PolyUserGetClassInfo"),
    (Win32kService::CreateWindow, 0x1003, "PolyUserCreateWindow"),
    (Win32kService::DestroyWindow, 0x1004, "PolyUserDestroyWindow"),
    (Win32kService::ShowWindow, 0x1005, "PolyUserShowWindow"),
    (Win32kService::SetWindowPos, 0x1006, "PolyUserSetWindowPos"),
    (Win32kService::QueryWindow, 0x1007, "PolyUserQueryWindow"),
    (Win32kService::GetWindowLong, 0x1008, "PolyUserGetWindowLong"),
    (Win32kService::SetWindowLong, 0x1009, "PolyUserSetWindowLong"),
    (Win32kService::SetWindowText, 0x100A, "PolyUserSetWindowText"),
    (Win32kService::PostMessage, 0x1100, "PolyUserPostMessage"),
    (Win32kService::PostThreadMessage, 0x1101, "PolyUserPostThreadMessage"),
    (Win32kService::SendMessage, 0x1102, "PolyUserSendMessage"),
    (Win32kService::GetMessage, 0x1103, "PolyUserGetMessage"),
    (Win32kService::PeekMessage, 0x1104, "PolyUserPeekMessage"),
    (Win32kService::ReplyMessage, 0x1105, "PolyUserReplyMessage"),
    (Win32kService::PostQuitMessage, 0x1106, "PolyUserPostQuitMessage"),
    (Win32kService::WaitMessage, 0x1107, "PolyUserWaitMessage"),
    (Win32kService::MsgWaitForMultipleObjects, 0x1108, "PolyUserMsgWaitForMultipleObjects"),
    (Win32kService::GetQueueStatus, 0x1109, "PolyUserGetQueueStatus"),
    (Win32kService::CalloutReturn, 0x110A, "PolyUserCalloutReturn"),
    (Win32kService::SetFocus, 0x1200, "PolyUserSetFocus"),
    (Win32kService::SetActiveWindow, 0x1201, "PolyUserSetActiveWindow"),
    (Win32kService::SetForegroundWindow, 0x1202, "PolyUserSetForegroundWindow"),
    (Win32kService::SetCapture, 0x1203, "PolyUserSetCapture"),
    (Win32kService::ReleaseCapture, 0x1204, "PolyUserReleaseCapture"),
    (Win32kService::GetKeyState, 0x1205, "PolyUserGetKeyState"),
    (Win32kService::GetAsyncKeyState, 0x1206, "PolyUserGetAsyncKeyState"),
    (Win32kService::GetKeyboardState, 0x1207, "PolyUserGetKeyboardState"),
    (Win32kService::GetCursorPos, 0x1208, "PolyUserGetCursorPos"),
    (Win32kService::SetCursor, 0x1209, "PolyUserSetCursor"),
    (Win32kService::SetTimer, 0x1300, "PolyUserSetTimer"),
    (Win32kService::KillTimer, 0x1301, "PolyUserKillTimer"),
    (Win32kService::SetWindowsHook, 0x1302, "PolyUserSetWindowsHook"),
    (Win32kService::UnhookWindowsHook, 0x1303, "PolyUserUnhookWindowsHook"),
    (Win32kService::BeginPaint, 0x1400, "PolyUserBeginPaint"),
    (Win32kService::EndPaint, 0x1401, "PolyUserEndPaint"),
    (Win32kService::InvalidateRect, 0x1402, "PolyUserInvalidateRect"),
    (Win32kService::ValidateRect, 0x1403, "PolyUserValidateRect"),
    (Win32kService::GetUpdateRect, 0x1404, "PolyUserGetUpdateRect"),
    (Win32kService::GetDC, 0x1405, "PolyUserGetDC"),
    (Win32kService::ReleaseDC, 0x1406, "PolyUserReleaseDC"),
    (Win32kService::CommitSurface, 0x1407, "PolyUserCommitSurface"),
    (Win32kService::QueryDisplay, 0x1800, "PolyGdiQueryDisplay"),
];

impl Win32kService {
    /// The service number `win32u`'s stub puts in `eax`.
    pub fn number(self) -> u32 {
        WIN32K_SERVICES.iter().find(|(s, _, _)| *s == self).map(|&(_, n, _)| n).expect("every service is in the table")
    }

    /// The name `win32u.dll` exports its stub under.
    pub fn name(self) -> &'static str {
        WIN32K_SERVICES.iter().find(|(s, _, _)| *s == self).map(|&(_, _, n)| n).expect("every service is in the table")
    }

    /// The service a number selects, if it is one of win32k's. Only the low
    /// 14 bits count (table and index), as for every NT service number.
    pub fn from_number(number: u32) -> Option<Win32kService> {
        let number = number & 0x3fff;
        WIN32K_SERVICES.iter().find(|&&(_, n, _)| n == number).map(|&(s, _, _)| s)
    }
}

/// The bytes of a `win32u` stub for service `number` as LLVM's assembler
/// encodes it: `mov r10, rcx` (`49 89 CA`), `mov eax, imm32` (`B8` + the
/// number), `syscall` (`0F 05`), `ret` (`C3`) — what `cargo xtask win32`
/// expects at every export.
pub fn stub_bytes(number: u32) -> [u8; 11] {
    let n = number.to_le_bytes();
    [0x49, 0x89, 0xCA, 0xB8, n[0], n[1], n[2], n[3], 0x0F, 0x05, 0xC3]
}

#[cfg(test)]
mod tests {
    extern crate std;
    use super::*;
    use std::collections::HashSet;

    #[test]
    fn numbers_and_names_are_unique_and_round_trip() {
        let numbers: HashSet<u32> = WIN32K_SERVICES.iter().map(|s| s.1).collect();
        let names: HashSet<&str> = WIN32K_SERVICES.iter().map(|s| s.2).collect();
        assert_eq!(numbers.len(), WIN32K_SERVICES.len());
        assert_eq!(names.len(), WIN32K_SERVICES.len());
        for &(s, n, name) in &WIN32K_SERVICES {
            assert_eq!(Win32kService::from_number(n), Some(s));
            assert_eq!(s.number(), n);
            assert_eq!(s.name(), name);
        }
        // Bits above the table are ignored; table 0 is not win32k's.
        assert_eq!(Win32kService::from_number(0x1_0000 | 0x1004), Some(Win32kService::DestroyWindow));
        assert_eq!(Win32kService::from_number(0x0004), None);
    }

    #[test]
    fn every_service_is_in_table_one_sorted_and_named_as_polys() {
        let mut last = 0;
        for &(_, n, name) in &WIN32K_SERVICES {
            assert_eq!(n >> 12, TABLE_WIN32K, "{name}");
            assert!(n > last, "{name} out of order");
            last = n;
            assert!(name.starts_with("PolyUser") || name.starts_with("PolyGdi"), "{name}");
            assert!(name.chars().all(|c| c.is_ascii_alphanumeric()), "{name}");
        }
        let gdi = WIN32K_SERVICES.iter().filter(|s| s.2.starts_with("PolyGdi"));
        assert!(gdi.clone().all(|s| (0x1800..0x2000).contains(&s.1)));
    }

    #[test]
    fn stubs_carry_their_number() {
        assert_eq!(stub_bytes(0x1003), [0x49, 0x89, 0xCA, 0xB8, 0x03, 0x10, 0, 0, 0x0F, 0x05, 0xC3]);
    }
}

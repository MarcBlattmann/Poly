//! A 32-bit message loop on a second thread, and what `user32` writes into a 32-bit caller's
//! structures (ADR 0186; the `wow64-gui-overrun` hunt). Every structure the program hands
//! `user32` is followed in memory by a guard word, as a `/GS` stack cookie follows a frame's
//! buffers: `user32` must write the structure's 32-bit size (`winuser.h`, `wingdi.h` on x86) and
//! not one byte more.
//!
//! - `loop`: a second thread makes a popup window and runs a `PeekMessageW(PM_REMOVE)` loop
//!   into a 28-byte `MSG` (`winuser.h` on x86: no `lPrivate`, which is `_MAC` only). The main
//!   thread sends the window a message whose handler destroys it (`WM_DESTROY` posts the quit),
//!   so the window procedure runs inside the loop's `PeekMessageW`, and the next `PeekMessageW`
//!   returns `WM_QUIT`; the same with a `GetMessageW` loop.
//! - the structures: `WINDOWPLACEMENT`, `MONITORINFO`, `MONITORINFOEXW`,
//!   `GUITHREADINFO`, `CURSORINFO`, `WNDCLASSEXW` (`GetClassInfoExW`),
//!   `LASTINPUTINFO`, `PAINTSTRUCT`, `RECT`, `POINT[2]` (`MapWindowPoints`), the key state,
//!   `NONCLIENTMETRICSW`, `DEVMODEW`, `SCROLLINFO`.
//! - `SECURITY_DESCRIPTOR`: 20 bytes on x86 (`winnt.h`: four pointers at 4, 8, 12, 16).
//!   `InitializeSecurityDescriptor` and `SetSecurityDescriptorDacl` stay inside it, and a named
//!   event made with that absolute descriptor in its `SECURITY_ATTRIBUTES` gets its DACL (read
//!   back with `GetKernelObjectSecurity`): the kernel reads the descriptor in the caller's layout.
//! - `TRUSTEE_W` (20 bytes on x86) and `EXPLICIT_ACCESS_W` (32): the `Build*` functions stay
//!   inside them, `SetEntriesInAclW` reads an array of them with the 32-bit stride and
//!   `GetExplicitEntriesFromAclW` writes one.
//! - `MENUITEMINFOW` (48 bytes on x86, 44 without `hbmpItem`) and `MENUINFO` (28): items in and
//!   out of a popup menu; `SECURITY_ATTRIBUTES` (12) in `CreateFileW` (descriptor at 4, inherit at
//!   8); comctl32's `LVITEMW`, `LVCOLUMNW`, `TVINSERTSTRUCTW`, `TVITEMW`, `TCITEMW`, `IMAGEINFO`,
//!   `IMAGELISTDRAWPARAMS` and the `ICONINFO` it trades with `user32` (all with pointers inside,
//!   so the fields after the first pointer sit earlier than on x64).
//!
//! One line per check on standard output; the exit code is the number of checks that failed.

#![no_std]
#![no_main]

extern crate poly_msvc32;

use core::sync::atomic::{AtomicU32, Ordering};

type Handle = isize;

/// The value behind every structure; any other value after a call is an overrun.
const GUARD: u32 = 0xC0C0_C0C0;
/// The filler before a call.
const FILL: u8 = 0xAB;

#[repr(C)]
struct WndClassEx {
    size: u32,
    style: u32,
    wndproc: extern "system" fn(Handle, u32, usize, isize) -> isize,
    cls_extra: i32,
    wnd_extra: i32,
    instance: Handle,
    icon: Handle,
    cursor: Handle,
    background: Handle,
    menu_name: *const u16,
    class_name: *const u16,
    icon_sm: Handle,
}

/// `MSG` on x86: 28 bytes.
#[repr(C)]
#[derive(Clone, Copy)]
struct Msg {
    hwnd: Handle,
    message: u32,
    wparam: usize,
    lparam: isize,
    time: u32,
    pt: [i32; 2],
}

/// A buffer of `N` bytes followed by the guard.
#[repr(C)]
struct Guarded<const N: usize> {
    bytes: [u8; N],
    guard: u32,
}

impl<const N: usize> Guarded<N> {
    fn new() -> Self {
        Guarded { bytes: [FILL; N], guard: GUARD }
    }
    /// The same with the first four bytes the structure's size (`cbSize`).
    fn sized(size: u32) -> Self {
        let mut g = Self::new();
        g.bytes[..4].copy_from_slice(&size.to_le_bytes());
        g
    }
    fn ptr(&mut self) -> *mut u8 {
        self.bytes.as_mut_ptr()
    }
}

#[link(name = "user32", kind = "raw-dylib", import_name_type = "undecorated")]
unsafe extern "system" {
    fn RegisterClassExW(class: *const WndClassEx) -> u16;
    fn GetClassInfoExW(instance: Handle, class: *const u16, out: *mut u8) -> i32;
    fn CreateWindowExW(
        ex_style: u32,
        class: *const u16,
        title: *const u16,
        style: u32,
        x: i32,
        y: i32,
        w: i32,
        h: i32,
        parent: Handle,
        menu: Handle,
        instance: Handle,
        param: usize,
    ) -> Handle;
    fn DefWindowProcW(hwnd: Handle, msg: u32, w: usize, l: isize) -> isize;
    fn ShowWindow(hwnd: Handle, cmd: i32) -> i32;
    fn SendMessageW(hwnd: Handle, msg: u32, w: usize, l: isize) -> isize;
    fn PostMessageW(hwnd: Handle, msg: u32, w: usize, l: isize) -> i32;
    fn PeekMessageW(msg: *mut Msg, hwnd: Handle, min: u32, max: u32, remove: u32) -> i32;
    fn GetMessageW(msg: *mut Msg, hwnd: Handle, min: u32, max: u32) -> i32;
    fn TranslateMessage(msg: *const Msg) -> i32;
    fn DispatchMessageW(msg: *const Msg) -> isize;
    fn MsgWaitForMultipleObjects(n: u32, handles: *const Handle, all: i32, ms: u32, mask: u32) -> u32;
    fn DestroyWindow(hwnd: Handle) -> i32;
    fn PostQuitMessage(code: i32);
    fn GetWindowPlacement(hwnd: Handle, wp: *mut u8) -> i32;
    fn MonitorFromWindow(hwnd: Handle, flags: u32) -> Handle;
    fn GetMonitorInfoW(monitor: Handle, info: *mut u8) -> i32;
    fn GetGUIThreadInfo(tid: u32, info: *mut u8) -> i32;
    fn GetCursorInfo(info: *mut u8) -> i32;
    fn GetLastInputInfo(info: *mut u8) -> i32;
    fn BeginPaint(hwnd: Handle, ps: *mut u8) -> Handle;
    fn EndPaint(hwnd: Handle, ps: *const u8) -> i32;
    fn GetClientRect(hwnd: Handle, rect: *mut u8) -> i32;
    fn GetWindowRect(hwnd: Handle, rect: *mut u8) -> i32;
    fn MapWindowPoints(from: Handle, to: Handle, points: *mut u8, n: u32) -> i32;
    fn GetKeyboardState(keys: *mut u8) -> i32;
    fn SystemParametersInfoW(action: u32, param: u32, pv: *mut u8, ini: u32) -> i32;
    fn EnumDisplaySettingsW(device: *const u16, mode: u32, dm: *mut u8) -> i32;
    fn GetScrollInfo(hwnd: Handle, bar: i32, si: *mut u8) -> i32;
    fn SetScrollInfo(hwnd: Handle, bar: i32, si: *const u8, redraw: i32) -> i32;
    fn CreatePopupMenu() -> Handle;
    fn DestroyMenu(menu: Handle) -> i32;
    fn InsertMenuItemW(menu: Handle, item: u32, by_position: i32, mii: *const u8) -> i32;
    fn GetMenuItemInfoW(menu: Handle, item: u32, by_position: i32, mii: *mut u8) -> i32;
    fn SetMenuItemInfoW(menu: Handle, item: u32, by_position: i32, mii: *const u8) -> i32;
    fn GetMenuInfo(menu: Handle, mi: *mut u8) -> i32;
    fn SetMenuInfo(menu: Handle, mi: *const u8) -> i32;
    fn LoadIconW(instance: Handle, name: usize) -> Handle;
    fn GetDC(hwnd: Handle) -> Handle;
    fn ReleaseDC(hwnd: Handle, dc: Handle) -> i32;
}

#[link(name = "comctl32", kind = "raw-dylib", import_name_type = "undecorated")]
unsafe extern "system" {
    fn InitCommonControlsEx(init: *const u32) -> i32;
    fn ImageList_Create(cx: i32, cy: i32, flags: u32, initial: i32, grow: i32) -> Handle;
    fn ImageList_Destroy(list: Handle) -> i32;
    fn ImageList_ReplaceIcon(list: Handle, index: i32, icon: Handle) -> i32;
    fn ImageList_GetImageInfo(list: Handle, index: i32, info: *mut u8) -> i32;
    fn ImageList_GetIcon(list: Handle, index: i32, flags: u32) -> Handle;
    fn ImageList_DrawIndirect(params: *const u8) -> i32;
}

#[link(name = "advapi32", kind = "raw-dylib", import_name_type = "undecorated")]
unsafe extern "system" {
    fn InitializeSecurityDescriptor(sd: *mut u8, revision: u32) -> i32;
    fn SetSecurityDescriptorDacl(sd: *mut u8, present: i32, acl: *mut u8, defaulted: i32) -> i32;
    fn InitializeAcl(acl: *mut u8, length: u32, revision: u32) -> i32;
    fn AddAccessAllowedAce(acl: *mut u8, revision: u32, mask: u32, sid: *const u8) -> i32;
    fn GetKernelObjectSecurity(h: Handle, info: u32, sd: *mut u8, length: u32, needed: *mut u32) -> i32;
    fn GetSecurityDescriptorDacl(sd: *mut u8, present: *mut i32, acl: *mut *mut u8, defaulted: *mut i32) -> i32;
    fn BuildTrusteeWithSidW(trustee: *mut u8, sid: *const u8);
    fn BuildExplicitAccessWithNameW(ea: *mut u8, name: *const u16, access: u32, mode: u32, inherit: u32);
    fn SetEntriesInAclW(count: u32, entries: *const u8, old: *const u8, new: *mut *mut u8) -> u32;
    fn GetExplicitEntriesFromAclW(acl: *const u8, count: *mut u32, entries: *mut *mut u8) -> u32;
}

#[link(name = "kernel32", kind = "raw-dylib", import_name_type = "undecorated")]
unsafe extern "system" {
    fn CreateEventW(sa: *const u8, manual: i32, initial: i32, name: *const u16) -> Handle;
    fn CloseHandle(h: Handle) -> i32;
    fn LocalFree(p: *mut u8) -> *mut u8;
    fn GetStdHandle(which: u32) -> Handle;
    fn WriteFile(file: Handle, buffer: *const u8, length: u32, written: *mut u32, overlapped: *mut u8) -> i32;
    fn GetModuleHandleW(name: *const u16) -> Handle;
    fn CreateThread(sa: *const u8, stack: usize, start: extern "system" fn(usize) -> u32, param: usize, flags: u32, tid: *mut u32) -> Handle;
    fn WaitForSingleObject(h: Handle, ms: u32) -> u32;
    fn GetExitCodeThread(h: Handle, code: *mut u32) -> i32;
    fn Sleep(ms: u32);
    fn ExitProcess(code: u32) -> !;
    fn CreateFileW(name: *const u16, access: u32, share: u32, sa: *const u8, disposition: u32, flags: u32, template: Handle) -> Handle;
    fn DeleteFileW(name: *const u16) -> i32;
    fn GetHandleInformation(h: Handle, flags: *mut u32) -> i32;
}

/// `  name: v1 v2 ...` for a check that failed, so the log shows how far it got.
fn detail(values: &[u32]) {
    out(b"  values:");
    for v in values {
        out(b" ");
        out(&hex(*v));
    }
    out(b"
");
}

fn u32_at(b: &[u8], at: usize) -> u32 {
    u32::from_le_bytes([b[at], b[at + 1], b[at + 2], b[at + 3]])
}

fn put_u32(b: &mut [u8], at: usize, v: u32) {
    b[at..at + 4].copy_from_slice(&v.to_le_bytes());
}

/// `MENUITEMINFOW` (x86: 48 bytes, 44 without `hbmpItem`: `wID` 16, `dwItemData` 32, `dwTypeData`
/// 36, `cch` 40) and `MENUINFO` (x86: 28 bytes, `dwContextHelpID` 20, `dwMenuData` 24).
unsafe fn menus() {
    const MIIM_ID: u32 = 0x2;
    const MIIM_DATA: u32 = 0x20;
    const MIIM_STRING: u32 = 0x40;
    // SAFETY: user32 calls on a menu of this program with buffers of the 32-bit sizes.
    unsafe {
        let menu = CreatePopupMenu();
        let open: [u16; 8] = wide("Open");
        let mut mii = [0u8; 48];
        put_u32(&mut mii, 0, 48);
        put_u32(&mut mii, 4, MIIM_ID | MIIM_DATA | MIIM_STRING);
        put_u32(&mut mii, 16, 0x1234);
        put_u32(&mut mii, 32, 0xBEEF_0001);
        put_u32(&mut mii, 36, open.as_ptr() as u32);
        check(b"MENUITEMINFOW (48): InsertMenuItemW", InsertMenuItemW(menu, 0, 1, mii.as_ptr()) != 0, GUARD);
        for size in [48u32, 44] {
            let mut text = Guarded::<16>::new();
            let mut g = Guarded::<48>::sized(size);
            put_u32(&mut g.bytes, 4, MIIM_ID | MIIM_DATA | MIIM_STRING);
            put_u32(&mut g.bytes, 36, text.ptr() as u32);
            put_u32(&mut g.bytes, 40, 8);
            if size == 44 {
                // The guard right after the 44-byte structure.
                put_u32(&mut g.bytes, 44, GUARD);
            }
            let ok = GetMenuItemInfoW(menu, 0, 1, g.ptr()) != 0
                && u32_at(&g.bytes, 16) == 0x1234
                && u32_at(&g.bytes, 32) == 0xBEEF_0001
                && u32_at(&g.bytes, 40) == 4
                && text.bytes[..10] == [b'O', 0, b'p', 0, b'e', 0, b'n', 0, 0, 0];
            let guard = if size == 44 { u32_at(&g.bytes, 44) } else { g.guard };
            let name: &[u8] = if size == 48 { b"MENUITEMINFOW (48): GetMenuItemInfoW" } else { b"MENUITEMINFOW (44): GetMenuItemInfoW" };
            check(name, ok, guard);
            if text.guard != GUARD {
                check(b"MENUITEMINFOW: the item's text", false, text.guard);
            }
        }
        let mut set = [0u8; 48];
        put_u32(&mut set, 0, 48);
        put_u32(&mut set, 4, MIIM_DATA);
        put_u32(&mut set, 32, 0xBEEF_0002);
        let changed = SetMenuItemInfoW(menu, 0x1234, 0, set.as_ptr()) != 0;
        let mut g = Guarded::<48>::sized(48);
        put_u32(&mut g.bytes, 4, MIIM_DATA);
        let ok = changed && GetMenuItemInfoW(menu, 0x1234, 0, g.ptr()) != 0 && u32_at(&g.bytes, 32) == 0xBEEF_0002;
        check(b"MENUITEMINFOW (48): SetMenuItemInfoW", ok, g.guard);

        // MIM_HELPID | MIM_MENUDATA.
        let mut mi = [0u8; 28];
        put_u32(&mut mi, 0, 28);
        put_u32(&mut mi, 4, 0x4 | 0x8);
        put_u32(&mut mi, 20, 77);
        put_u32(&mut mi, 24, 0xCAFE);
        let set = SetMenuInfo(menu, mi.as_ptr()) != 0;
        let mut g = Guarded::<28>::sized(28);
        put_u32(&mut g.bytes, 4, 0x4 | 0x8);
        let ok = set && GetMenuInfo(menu, g.ptr()) != 0 && u32_at(&g.bytes, 20) == 77 && u32_at(&g.bytes, 24) == 0xCAFE;
        check(b"MENUINFO (28): SetMenuInfo/GetMenuInfo", ok, g.guard);
        DestroyMenu(menu);
    }
}

/// `CreateFileW` with a 32-bit `SECURITY_ATTRIBUTES` (12 bytes: `lpSecurityDescriptor` at 4,
/// `bInheritHandle` at 8): the descriptor is taken from 4 and the handle is inheritable.
unsafe fn create_file(sd: *mut u8) {
    const GENERIC_WRITE: u32 = 0x4000_0000;
    const CREATE_ALWAYS: u32 = 2;
    const FILE_ATTRIBUTE_NORMAL: u32 = 0x80;
    let name: [u16; 40] = wide("C:\\Windows\\Temp\\polysa32.tmp");
    let sa: [u32; 3] = [12, sd as u32, 1];
    // SAFETY: kernel32 calls with a NUL-terminated name and a 32-bit SECURITY_ATTRIBUTES.
    unsafe {
        let h = CreateFileW(name.as_ptr(), GENERIC_WRITE, 0, sa.as_ptr() as *const u8, CREATE_ALWAYS, FILE_ATTRIBUTE_NORMAL, 0);
        let mut flags = 0u32;
        let ok = h != -1 && GetHandleInformation(h, &mut flags) != 0 && flags & 1 != 0;
        check(b"SECURITY_ATTRIBUTES (12): CreateFileW", ok, GUARD);
        if h != -1 {
            CloseHandle(h);
            DeleteFileW(name.as_ptr());
        }
    }
}

/// comctl32's item structures in their 32-bit layouts: `LVITEMW` (`pszText` 20, `cchTextMax` 24),
/// `LVCOLUMNW` (`pszText` 12), `TVINSERTSTRUCTW` (handles at 0 and 4, the item at 8),
/// `TVITEMW` (`hItem` 4, `pszText` 16, `cchTextMax` 20), `TCITEMW` (28 bytes: `pszText` 12,
/// `cchTextMax` 16, `iImage` 20, `lParam` 24), `IMAGEINFO` (32 bytes: `rcImage` 16) and
/// `IMAGELISTDRAWPARAMS` (56 bytes without the Windows XP fields).
unsafe fn controls(parent: Handle) {
    const WS_CHILD: u32 = 0x4000_0000;
    // SAFETY: comctl32 controls of this program, with buffers of the 32-bit sizes.
    unsafe {
        let icc: [u32; 2] = [8, 0xFF];
        InitCommonControlsEx(icc.as_ptr());
        let inst = GetModuleHandleW(core::ptr::null());
        let make = |class: &str| {
            let c: [u16; 20] = wide(class);
            CreateWindowExW(0, c.as_ptr(), core::ptr::null(), WS_CHILD | 1, 0, 0, 200, 100, parent, 0, inst, 0)
        };

        // List view: a column, an item by text, its text read back.
        let list = make("SysListView32");
        let head: [u16; 8] = wide("Name");
        let mut col = [0u8; 44];
        put_u32(&mut col, 0, 0x2 | 0x4);
        put_u32(&mut col, 8, 80);
        put_u32(&mut col, 12, head.as_ptr() as u32);
        let col_ok = SendMessageW(list, 0x1061, 0, col.as_ptr() as isize) == 0;
        let row: [u16; 8] = wide("Row0");
        let mut item = [0u8; 60];
        put_u32(&mut item, 0, 1);
        put_u32(&mut item, 20, row.as_ptr() as u32);
        let at = SendMessageW(list, 0x104D, 0, item.as_ptr() as isize);
        let mut text = Guarded::<16>::new();
        let mut get = [0u8; 60];
        put_u32(&mut get, 20, text.ptr() as u32);
        put_u32(&mut get, 24, 8);
        let n = SendMessageW(list, 0x1073, 0, get.as_mut_ptr() as isize);
        let ok = list != 0 && col_ok && at == 0 && n == 4 && text.bytes[..10] == [b'R', 0, b'o', 0, b'w', 0, b'0', 0, 0, 0];
        check(b"LVITEMW/LVCOLUMNW: LVM_INSERTITEMW, LVM_GETITEMTEXTW", ok, text.guard);
        if !ok {
            detail(&[list as u32, u32::from(col_ok), at as u32, n as u32, u32_at(&text.bytes, 0)]);
        }

        // Tree view: an item under TVI_ROOT after TVI_LAST, its text read back by handle.
        let tree = make("SysTreeView32");
        let leaf: [u16; 8] = wide("Leaf");
        let mut ins = [0u8; 48];
        put_u32(&mut ins, 0, 0xFFFF_0000);
        put_u32(&mut ins, 4, 0xFFFF_0002);
        put_u32(&mut ins, 8, 1);
        put_u32(&mut ins, 8 + 16, leaf.as_ptr() as u32);
        let hitem = SendMessageW(tree, 0x1132, 0, ins.as_ptr() as isize);
        let mut text = Guarded::<16>::new();
        let mut tvi = [0u8; 40];
        put_u32(&mut tvi, 0, 1);
        put_u32(&mut tvi, 4, hitem as u32);
        put_u32(&mut tvi, 16, text.ptr() as u32);
        put_u32(&mut tvi, 20, 8);
        let got = SendMessageW(tree, 0x113E, 0, tvi.as_mut_ptr() as isize) != 0;
        let ok = tree != 0 && hitem != 0 && got && text.bytes[..10] == [b'L', 0, b'e', 0, b'a', 0, b'f', 0, 0, 0];
        check(b"TVINSERTSTRUCTW/TVITEMW: TVM_INSERTITEMW, TVM_GETITEMW", ok, text.guard);
        if !ok {
            detail(&[tree as u32, hitem as u32, u32::from(got), u32_at(&text.bytes, 0)]);
        }

        // Tab control: a tab with text, image and lParam read back into a guarded TCITEMW.
        let tabs = make("SysTabControl32");
        let name: [u16; 8] = wide("Tab0");
        let mut tci = [0u8; 28];
        put_u32(&mut tci, 0, 0x1 | 0x2 | 0x8);
        put_u32(&mut tci, 12, name.as_ptr() as u32);
        put_u32(&mut tci, 20, 3);
        put_u32(&mut tci, 24, 0x5151);
        let at = SendMessageW(tabs, 0x133E, 0, tci.as_ptr() as isize);
        let mut text = Guarded::<16>::new();
        let mut g = Guarded::<28>::new();
        put_u32(&mut g.bytes, 0, 0x1 | 0x2 | 0x8);
        put_u32(&mut g.bytes, 12, text.ptr() as u32);
        put_u32(&mut g.bytes, 16, 8);
        let got = SendMessageW(tabs, 0x133C, 0, g.ptr() as isize) != 0;
        let ok = tabs != 0
            && at == 0
            && got
            && u32_at(&g.bytes, 20) == 3
            && u32_at(&g.bytes, 24) == 0x5151
            && text.bytes[..10] == [b'T', 0, b'a', 0, b'b', 0, b'0', 0, 0, 0];
        check(b"TCITEMW (28): TCM_INSERTITEMW, TCM_GETITEMW", ok, g.guard);
        if !ok {
            detail(&[tabs as u32, at as u32, u32::from(got), u32_at(&g.bytes, 20), u32_at(&g.bytes, 24), u32_at(&text.bytes, 0)]);
        }
        if text.guard != GUARD {
            check(b"TCITEMW: the tab's text", false, text.guard);
        }

        // Image list: an icon in (ICONINFO through user32), IMAGEINFO, an icon out, a draw.
        let il = ImageList_Create(16, 16, 0x20 | 0x1, 1, 1);
        let icon = LoadIconW(0, 32512);
        let index = ImageList_ReplaceIcon(il, -1, icon);
        check(b"ICONINFO (20): ImageList_ReplaceIcon", il != 0 && icon != 0 && index == 0, GUARD);
        let mut g = Guarded::<32>::new();
        let ok = ImageList_GetImageInfo(il, 0, g.ptr()) != 0 && u32_at(&g.bytes, 16 + 8) == 16 && u32_at(&g.bytes, 16 + 12) == 16;
        check(b"IMAGEINFO (32): ImageList_GetImageInfo", ok, g.guard);
        check(b"ICONINFO (20): ImageList_GetIcon", ImageList_GetIcon(il, 0, 0) != 0, GUARD);
        let dc = GetDC(parent);
        let mut p = [0u8; 56];
        put_u32(&mut p, 0, 56);
        put_u32(&mut p, 4, il as u32);
        put_u32(&mut p, 12, dc as u32);
        put_u32(&mut p, 24, 16);
        put_u32(&mut p, 28, 16);
        put_u32(&mut p, 40, 0xFFFF_FFFF);
        put_u32(&mut p, 44, 0xFF00_0000);
        check(b"IMAGELISTDRAWPARAMS (56): ImageList_DrawIndirect", ImageList_DrawIndirect(p.as_ptr()) != 0, GUARD);
        ReleaseDC(parent, dc);
        ImageList_Destroy(il);
    }
}

const WM_DESTROY: u32 = 0x0002;
const WM_QUIT: u32 = 0x0012;
const WM_USER: u32 = 0x0400;
const WS_POPUP: u32 = 0x8000_0000;
const WS_OVERLAPPEDWINDOW: u32 = 0x00CF_0000;
const WS_VSCROLL: u32 = 0x0020_0000;
const SW_SHOWNOACTIVATE: i32 = 4;
const PM_REMOVE: u32 = 1;
const QS_ALLINPUT: u32 = 0x04FF;

static FAILED: AtomicU32 = AtomicU32::new(0);
static WINDOW: AtomicU32 = AtomicU32::new(0);
static READY: AtomicU32 = AtomicU32::new(0);
static QUIT_CODE: AtomicU32 = AtomicU32::new(0);

fn out(text: &[u8]) {
    let mut written = 0u32;
    // SAFETY: plain kernel32 calls with valid buffers.
    unsafe { WriteFile(GetStdHandle(-11i32 as u32), text.as_ptr(), text.len() as u32, &mut written, core::ptr::null_mut()) };
}

fn hex(v: u32) -> [u8; 8] {
    core::array::from_fn(|i| b"0123456789abcdef"[(v >> (28 - 4 * i)) as usize & 0xF])
}

fn wide<const N: usize>(text: &str) -> [u16; N] {
    let mut b = [0u16; N];
    for (d, s) in b.iter_mut().zip(text.encode_utf16()) {
        *d = s;
    }
    b
}

/// One check: `name ok` or `name OVERRUN guard <value>` / `name failed`.
fn check(name: &[u8], ok: bool, guard: u32) {
    out(name);
    if guard != GUARD {
        out(b": OVERRUN, the guard is ");
        out(&hex(guard));
        out(b"\n");
        FAILED.fetch_add(1, Ordering::SeqCst);
    } else if !ok {
        out(b": the call failed\n");
        FAILED.fetch_add(1, Ordering::SeqCst);
    } else {
        out(b": ok\n");
    }
}

extern "system" fn wndproc(hwnd: Handle, msg: u32, w: usize, l: isize) -> isize {
    // SAFETY: user32's documented calls with the arguments it passed.
    unsafe {
        match msg {
            m if m == WM_USER + 1 => {
                DestroyWindow(hwnd);
                0
            }
            WM_DESTROY => {
                PostQuitMessage(7);
                0
            }
            _ => DefWindowProcW(hwnd, msg, w, l),
        }
    }
}

fn register(name: &[u16]) {
    let wc = WndClassEx {
        size: core::mem::size_of::<WndClassEx>() as u32,
        style: 0,
        wndproc,
        cls_extra: 0,
        wnd_extra: 0,
        // SAFETY: NULL names the executable.
        instance: unsafe { GetModuleHandleW(core::ptr::null()) },
        icon: 0,
        cursor: 0,
        background: 0,
        menu_name: core::ptr::null(),
        class_name: name.as_ptr(),
        icon_sm: 0,
    };
    // SAFETY: a filled class.
    unsafe { RegisterClassExW(&wc) };
}

/// The splash thread: its window, then a message loop into a guarded 28-byte `MSG`
/// (`use_get`: `GetMessageW`, else `PeekMessageW` with `MsgWaitForMultipleObjects`).
extern "system" fn splash(use_get: usize) -> u32 {
    let class: [u16; 16] = wide("PolySplash32");
    register(&class);
    // SAFETY: plain user32 calls on this thread's window; `frame` is this frame's buffer.
    unsafe {
        let h = CreateWindowExW(0, class.as_ptr(), core::ptr::null(), WS_POPUP, 580, 340, 120, 120, 0, 0, GetModuleHandleW(core::ptr::null()), 0);
        if h == 0 {
            return 1;
        }
        ShowWindow(h, SW_SHOWNOACTIVATE);
        WINDOW.store(h as u32, Ordering::SeqCst);
        READY.store(1, Ordering::SeqCst);
        #[repr(C)]
        struct Frame {
            msg: Msg,
            guard: u32,
        }
        let mut frame = Frame { msg: core::mem::zeroed(), guard: GUARD };
        let mut quit = false;
        let mut spins = 0;
        while !quit && spins < 2000 {
            if use_get != 0 {
                if GetMessageW(&mut frame.msg, 0, 0, 0) <= 0 {
                    QUIT_CODE.store(frame.msg.wparam as u32, Ordering::SeqCst);
                    quit = true;
                    continue;
                }
            } else if PeekMessageW(&mut frame.msg, 0, 0, 0, PM_REMOVE) == 0 {
                MsgWaitForMultipleObjects(0, core::ptr::null(), 0, 10, QS_ALLINPUT);
                spins += 1;
                continue;
            } else if frame.msg.message == WM_QUIT {
                QUIT_CODE.store(frame.msg.wparam as u32, Ordering::SeqCst);
                quit = true;
                continue;
            }
            TranslateMessage(&frame.msg);
            DispatchMessageW(&frame.msg);
        }
        if frame.guard != GUARD {
            return 0x100 | (frame.guard & 0xFF);
        }
        if !quit {
            return 2;
        }
    }
    0
}

fn run_loop(name: &[u8], use_get: usize) {
    READY.store(0, Ordering::SeqCst);
    let mut tid = 0u32;
    // SAFETY: a thread of this program.
    unsafe {
        let t = CreateThread(core::ptr::null(), 0, splash, use_get, 0, &mut tid);
        for _ in 0..500 {
            if READY.load(Ordering::SeqCst) != 0 {
                break;
            }
            Sleep(10);
        }
        // The main thread asks the splash window to go: its procedure runs in the loop's call.
        SendMessageW(WINDOW.load(Ordering::SeqCst) as Handle, WM_USER + 1, 0, 0);
        WaitForSingleObject(t, 10_000);
        let mut code = 0xFFFF_FFFFu32;
        GetExitCodeThread(t, &mut code);
        let guard = if code & 0x100 != 0 { code } else { GUARD };
        check(name, code == 0 && QUIT_CODE.load(Ordering::SeqCst) == 7, guard);
        if code != 0 && code & 0x100 == 0 {
            out(b"  thread exit ");
            out(&hex(code));
            out(b"\n");
        }
    }
}

#[unsafe(no_mangle)]
pub extern "C" fn mainCRTStartup() -> u32 {
    run_loop(b"loop PeekMessageW", 0);
    run_loop(b"loop GetMessageW", 1);

    let class: [u16; 16] = wide("PolyGuard32");
    register(&class);
    // SAFETY: user32 calls on this program's window with guarded buffers of the 32-bit sizes.
    unsafe {
        let inst = GetModuleHandleW(core::ptr::null());
        let h = CreateWindowExW(0, class.as_ptr(), class.as_ptr(), WS_OVERLAPPEDWINDOW | WS_VSCROLL, 100, 100, 400, 300, 0, 0, inst, 0);

        let mut g = Guarded::<44>::sized(44);
        check(b"WINDOWPLACEMENT (44)", GetWindowPlacement(h, g.ptr()) != 0, g.guard);
        let mon = MonitorFromWindow(h, 2);
        let mut g = Guarded::<40>::sized(40);
        check(b"MONITORINFO (40)", GetMonitorInfoW(mon, g.ptr()) != 0, g.guard);
        let mut g = Guarded::<104>::sized(104);
        check(b"MONITORINFOEXW (104)", GetMonitorInfoW(mon, g.ptr()) != 0, g.guard);
        let mut g = Guarded::<48>::sized(48);
        check(b"GUITHREADINFO (48)", GetGUIThreadInfo(0, g.ptr()) != 0, g.guard);
        let mut g = Guarded::<20>::sized(20);
        check(b"CURSORINFO (20)", GetCursorInfo(g.ptr()) != 0, g.guard);
        let mut g = Guarded::<48>::sized(48);
        check(b"WNDCLASSEXW (48)", GetClassInfoExW(inst, class.as_ptr(), g.ptr()) != 0, g.guard);
        let mut g = Guarded::<8>::sized(8);
        check(b"LASTINPUTINFO (8)", GetLastInputInfo(g.ptr()) != 0, g.guard);
        let mut g = Guarded::<64>::new();
        let dc = BeginPaint(h, g.ptr());
        let guard = g.guard;
        EndPaint(h, g.ptr());
        check(b"PAINTSTRUCT (64)", dc != 0, guard);
        let mut g = Guarded::<16>::new();
        check(b"GetClientRect RECT (16)", GetClientRect(h, g.ptr()) != 0, g.guard);
        let mut g = Guarded::<16>::new();
        check(b"GetWindowRect RECT (16)", GetWindowRect(h, g.ptr()) != 0, g.guard);
        let mut g = Guarded::<16>::new();
        g.bytes = [0; 16];
        MapWindowPoints(h, 0, g.ptr(), 2);
        check(b"MapWindowPoints POINT[2] (16)", true, g.guard);
        let mut g = Guarded::<256>::new();
        check(b"GetKeyboardState (256)", GetKeyboardState(g.ptr()) != 0, g.guard);
        let mut g = Guarded::<504>::sized(504);
        check(b"NONCLIENTMETRICSW (504)", SystemParametersInfoW(0x29, 504, g.ptr(), 0) != 0, g.guard);
        let mut g = Guarded::<500>::sized(500);
        check(b"NONCLIENTMETRICSW (500)", SystemParametersInfoW(0x29, 500, g.ptr(), 0) != 0, g.guard);
        let mut g = Guarded::<220>::new();
        g.bytes[68..70].copy_from_slice(&220u16.to_le_bytes());
        g.bytes[70..72].copy_from_slice(&0u16.to_le_bytes());
        check(b"DEVMODEW (220)", EnumDisplaySettingsW(core::ptr::null(), 0xFFFF_FFFF, g.ptr()) != 0, g.guard);
        // SCROLLINFO: cbSize, fMask (SIF_ALL), nMin, nMax, nPage, nPos, nTrackPos.
        let mut si = [0u32; 7];
        si[0] = 28;
        si[1] = 0x17;
        si[3] = 100;
        si[4] = 10;
        si[5] = 40;
        let set = SetScrollInfo(h, 1, si.as_ptr() as *const u8, 0) == 40;
        let mut g = Guarded::<28>::sized(28);
        g.bytes[4..8].copy_from_slice(&0x17u32.to_le_bytes());
        let ok = set
            && GetScrollInfo(h, 1, g.ptr()) != 0
            && u32_at(&g.bytes, 12) == 100
            && u32_at(&g.bytes, 16) == 10
            && u32_at(&g.bytes, 20) == 40
            && u32_at(&g.bytes, 24) == 40;
        check(b"SCROLLINFO (28): SetScrollInfo/GetScrollInfo", ok, g.guard);

        // A posted message through this thread's own loop, into a guarded MSG.
        #[repr(C)]
        struct Frame {
            msg: Msg,
            guard: u32,
        }
        let mut f = Frame { msg: core::mem::zeroed(), guard: GUARD };
        PostMessageW(h, WM_USER + 5, 1, 2);
        let got = PeekMessageW(&mut f.msg, h, WM_USER + 5, WM_USER + 5, PM_REMOVE) != 0;
        check(b"PeekMessageW MSG (28)", got && f.msg.wparam == 1, f.guard);
        PostMessageW(h, WM_USER + 5, 3, 4);
        let got = GetMessageW(&mut f.msg, h, WM_USER + 5, WM_USER + 5) > 0;
        check(b"GetMessageW MSG (28)", got && f.msg.lparam == 4, f.guard);
        menus();
        controls(h);
        DestroyWindow(h);

        // SECURITY_DESCRIPTOR (x86: 20 bytes) and the kernel's reading of it.
        let mut sd = Guarded::<20>::new();
        let init = InitializeSecurityDescriptor(sd.ptr(), 1) != 0;
        let guard_init = sd.guard;
        // An ACL that grants Everyone (S-1-1-0) EVENT_ALL_ACCESS (0x1F0003).
        let everyone: [u8; 12] = [1, 1, 0, 0, 0, 0, 0, 1, 0, 0, 0, 0];
        let mut acl = [0u8; 64];
        let acl_ok = InitializeAcl(acl.as_mut_ptr(), 64, 2) != 0
            && AddAccessAllowedAce(acl.as_mut_ptr(), 2, 0x1F_0003, everyone.as_ptr()) != 0;
        let set = SetSecurityDescriptorDacl(sd.ptr(), 1, acl.as_mut_ptr(), 0) != 0;
        check(b"SECURITY_DESCRIPTOR (20): InitializeSecurityDescriptor", init, guard_init);
        check(b"SECURITY_DESCRIPTOR (20): SetSecurityDescriptorDacl", acl_ok && set, sd.guard);
        // SECURITY_ATTRIBUTES (x86): nLength 12, the descriptor, bInheritHandle.
        let sa: [u32; 3] = [12, sd.ptr() as u32, 0];
        let name: [u16; 20] = wide("PolyAbsoluteSd32");
        let ev = CreateEventW(sa.as_ptr() as *const u8, 1, 0, name.as_ptr());
        let mut back = [0u8; 256];
        let mut needed = 0u32;
        let got = ev != 0 && GetKernelObjectSecurity(ev, 4, back.as_mut_ptr(), 256, &mut needed) != 0;
        let (mut present, mut dacl, mut defaulted) = (0i32, core::ptr::null_mut::<u8>(), 0i32);
        let read = got && GetSecurityDescriptorDacl(back.as_mut_ptr(), &mut present, &mut dacl, &mut defaulted) != 0;
        // ACL: AceCount at 4; the first ACE's mask at 8 + 4.
        let ok = read
            && present != 0
            && !dacl.is_null()
            && u16::from_le_bytes([*dacl.add(4), *dacl.add(5)]) == 1
            && u32::from_le_bytes([*dacl.add(12), *dacl.add(13), *dacl.add(14), *dacl.add(15)]) == 0x1F_0003;
        check(b"an event made with an absolute 32-bit descriptor has its DACL", ok, GUARD);
        if ev != 0 {
            CloseHandle(ev);
        }
        create_file(sd.ptr());

        // TRUSTEE_W (x86 20 bytes: form at 8, name at 16) and EXPLICIT_ACCESS_W (32: trustee at 12).
        let mut t = Guarded::<20>::new();
        BuildTrusteeWithSidW(t.ptr(), everyone.as_ptr());
        let form = u32::from_le_bytes([t.bytes[8], t.bytes[9], t.bytes[10], t.bytes[11]]);
        let sid_at = u32::from_le_bytes([t.bytes[16], t.bytes[17], t.bytes[18], t.bytes[19]]);
        check(b"TRUSTEE_W (20): BuildTrusteeWithSidW", form == 0 && sid_at == everyone.as_ptr() as u32, t.guard);
        let me: [u16; 16] = wide("CURRENT_USER");
        // Two entries: Everyone (read access) and Administrators (all access), by SID; the second is
        // first built by name (`CURRENT_USER`) to check the builder.
        #[repr(C)]
        struct Entries {
            ea: [u8; 64],
            guard: u32,
        }
        let mut list = Entries { ea: [FILL; 64], guard: GUARD };
        BuildExplicitAccessWithNameW(list.ea.as_mut_ptr().add(32), me.as_ptr(), 0x1F_0003, 2, 0);
        let built_guard = list.guard;
        list.ea[..12].copy_from_slice(&[0x01, 0, 0x12, 0, 2, 0, 0, 0, 0, 0, 0, 0]);
        BuildTrusteeWithSidW(list.ea.as_mut_ptr().add(12), everyone.as_ptr());
        let name_form = u32::from_le_bytes([list.ea[32 + 20], list.ea[32 + 21], list.ea[32 + 22], list.ea[32 + 23]]);
        check(b"EXPLICIT_ACCESS_W (32): BuildExplicitAccessWithNameW", name_form == 1, built_guard);
        // The second entry for the ACL: Administrators (S-1-5-32-544) by SID.
        let admins: [u8; 16] = [1, 2, 0, 0, 0, 0, 0, 5, 0x20, 0, 0, 0, 0x20, 2, 0, 0];
        BuildTrusteeWithSidW(list.ea.as_mut_ptr().add(32 + 12), admins.as_ptr());
        let mut acl: *mut u8 = core::ptr::null_mut();
        let rc = SetEntriesInAclW(2, list.ea.as_ptr(), core::ptr::null(), &mut acl);
        let aces = if rc == 0 && !acl.is_null() { u16::from_le_bytes([*acl.add(4), *acl.add(5)]) } else { 0 };
        check(b"SetEntriesInAclW reads 32-byte entries", rc == 0 && aces == 2, list.guard);
        let (mut n, mut got) = (0u32, core::ptr::null_mut::<u8>());
        let rc = if acl.is_null() { 1 } else { GetExplicitEntriesFromAclW(acl, &mut n, &mut got) };
        // The second entry starts 32 bytes in; its trustee form (12 + 8) is a SID.
        let second_form = if rc == 0 && n == 2 && !got.is_null() {
            u32::from_le_bytes([*got.add(32 + 20), *got.add(32 + 21), *got.add(32 + 22), *got.add(32 + 23)])
        } else {
            9
        };
        check(b"GetExplicitEntriesFromAclW writes 32-byte entries", rc == 0 && n == 2 && second_form == 0, GUARD);
        if !got.is_null() {
            LocalFree(got);
        }
        if !acl.is_null() {
            LocalFree(acl);
        }
        // The special trustee name CURRENT_USER (Learn, `TRUSTEE`): the process token's user, an
        // S-1-5-... SID in the ACE (ACL header 8, ACE header 4, mask 4, then the SID).
        let mut one = Guarded::<32>::new();
        BuildExplicitAccessWithNameW(one.ptr(), me.as_ptr(), 0x1F_0003, 2, 0);
        let mut acl: *mut u8 = core::ptr::null_mut();
        let rc = SetEntriesInAclW(1, one.ptr(), core::ptr::null(), &mut acl);
        let ok = rc == 0
            && !acl.is_null()
            && u16::from_le_bytes([*acl.add(4), *acl.add(5)]) == 1
            && *acl.add(16) == 1
            && *acl.add(16 + 7) == 5;
        check(b"SetEntriesInAclW resolves CURRENT_USER", ok, one.guard);
        if !ok {
            detail(&[rc]);
        }
        if !acl.is_null() {
            LocalFree(acl);
        }
        let failed = FAILED.load(Ordering::SeqCst);
        out(b"failed: ");
        out(&hex(failed));
        out(b"\n");
        ExitProcess(failed)
    }
}

/// MSVC-style code references this once floating point is used.
#[unsafe(no_mangle)]
#[used]
static _fltused: i32 = 0;

#[panic_handler]
fn panic(_: &core::panic::PanicInfo) -> ! {
    out(b"panic\n");
    // SAFETY: ends the process.
    unsafe { ExitProcess(0xDEAD) }
}

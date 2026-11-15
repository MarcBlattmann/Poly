//! What the dialogs call in other DLLs. The dialog manager's functions
//! (`DialogBoxIndirectParamW`, `EndDialog`, `GetDlgItem` ...) are looked up
//! when first used (`late!`): a program that never opens a common dialog
//! does not need them, and the DLL loads where user32 does not have them yet.

#![allow(non_snake_case, dead_code)]

use core::sync::atomic::{AtomicUsize, Ordering};

pub type H = usize;
pub type WndProc = unsafe extern "system" fn(H, u32, usize, isize) -> isize;

#[cfg_attr(target_arch = "x86", link(name = "kernel32", kind = "raw-dylib", import_name_type = "undecorated"))]
#[cfg_attr(not(target_arch = "x86"), link(name = "kernel32", kind = "raw-dylib"))]
unsafe extern "system" {
    pub fn GetModuleHandleW(name: *const u16) -> H;
    pub fn LoadLibraryW(name: *const u16) -> H;
    pub fn GetProcAddress(module: H, name: *const u8) -> usize;
    pub fn GetCurrentThreadId() -> u32;
    pub fn GetCurrentDirectoryW(n: u32, buf: *mut u16) -> u32;
    pub fn SetCurrentDirectoryW(path: *const u16) -> i32;
    pub fn GetFileAttributesW(path: *const u16) -> u32;
    pub fn FindFirstFileW(name: *const u16, data: *mut u8) -> H;
    pub fn FindNextFileW(find: H, data: *mut u8) -> i32;
    pub fn FindClose(find: H) -> i32;
    pub fn GetLogicalDrives() -> u32;
    pub fn FindResourceW(module: H, name: *const u16, ty: *const u16) -> H;
    pub fn MultiByteToWideChar(cp: u32, flags: u32, s: *const u8, n: i32, out: *mut u16, cap: i32) -> i32;
    pub fn WideCharToMultiByte(
        cp: u32,
        flags: u32,
        s: *const u16,
        n: i32,
        out: *mut u8,
        cap: i32,
        def: *const u8,
        used: *mut i32,
    ) -> i32;
}

#[cfg_attr(target_arch = "x86", link(name = "user32", kind = "raw-dylib", import_name_type = "undecorated"))]
#[cfg_attr(not(target_arch = "x86"), link(name = "user32", kind = "raw-dylib"))]
unsafe extern "system" {
    pub fn IsWindow(hwnd: H) -> i32;
    pub fn EnableWindow(hwnd: H, enable: i32) -> i32;
    pub fn GetParent(hwnd: H) -> H;
    pub fn PostMessageW(hwnd: H, msg: u32, w: usize, l: isize) -> i32;
    pub fn SendMessageW(hwnd: H, msg: u32, w: usize, l: isize) -> isize;
    pub fn GetWindowLongPtrW(hwnd: H, index: i32) -> isize;
    pub fn SetWindowLongPtrW(hwnd: H, index: i32, value: isize) -> isize;
    pub fn SetWindowTextW(hwnd: H, text: *const u16) -> i32;
    pub fn GetClientRect(hwnd: H, rect: *mut i32) -> i32;
    pub fn InvalidateRect(hwnd: H, rect: *const i32, erase: i32) -> i32;
    pub fn BeginPaint(hwnd: H, ps: *mut u8) -> H;
    pub fn EndPaint(hwnd: H, ps: *const u8) -> i32;
    pub fn FillRect(hdc: H, rect: *const i32, brush: H) -> i32;
    pub fn GetDC(hwnd: H) -> H;
    pub fn ReleaseDC(hwnd: H, hdc: H) -> i32;
    pub fn DefWindowProcW(hwnd: H, msg: u32, w: usize, l: isize) -> isize;
    pub fn RegisterClassW(class: *const u8) -> u16;
    pub fn LoadCursorW(inst: H, name: *const u16) -> H;
    pub fn MessageBeep(kind: u32) -> i32;
    pub fn GetSysColorBrush(index: i32) -> H;
    pub fn SetFocus(hwnd: H) -> H;
    pub fn DrawTextW(hdc: H, text: *const u16, n: i32, rect: *mut i32, format: u32) -> i32;
    pub fn GetSystemMetrics(index: i32) -> i32;
}

#[cfg_attr(target_arch = "x86", link(name = "gdi32", kind = "raw-dylib", import_name_type = "undecorated"))]
#[cfg_attr(not(target_arch = "x86"), link(name = "gdi32", kind = "raw-dylib"))]
unsafe extern "system" {
    pub fn CreateSolidBrush(color: u32) -> H;
    pub fn DeleteObject(obj: H) -> i32;
    pub fn SelectObject(hdc: H, obj: H) -> H;
    pub fn CreateFontIndirectW(lf: *const u8) -> H;
    pub fn GetDeviceCaps(hdc: H, index: i32) -> i32;
    pub fn EnumFontFamiliesExW(
        hdc: H,
        lf: *const u8,
        proc_: unsafe extern "system" fn(*const u8, *const u8, u32, isize) -> i32,
        lparam: isize,
        flags: u32,
    ) -> i32;
}

/// A function in a DLL, found when first called; `$default` when it is not there.
macro_rules! late {
    ($dll:literal, $name:literal, fn $f:ident($($a:ident: $t:ty),*) -> $r:ty = $default:expr) => {
        #[allow(clippy::missing_safety_doc, clippy::too_many_arguments)]
        pub unsafe fn $f($($a: $t),*) -> $r {
            static P: AtomicUsize = AtomicUsize::new(0);
            let mut p = P.load(Ordering::Relaxed);
            if p == 0 {
                p = resolve($dll, concat!($name, "\0").as_bytes());
                P.store(if p == 0 { 1 } else { p }, Ordering::Relaxed);
            }
            if p <= 1 {
                return $default;
            }
            // SAFETY: the export has this signature (Learn's prototype of `$name`).
            let f: unsafe extern "system" fn($($t),*) -> $r = unsafe { core::mem::transmute(p) };
            // SAFETY: the caller's arguments.
            unsafe { f($($a),*) }
        }
    };
}

fn resolve(dll: &str, name: &[u8]) -> usize {
    let mut w: [u16; 24] = [0; 24];
    for (i, b) in dll.bytes().enumerate().take(23) {
        w[i] = u16::from(b);
    }
    // SAFETY: NUL-terminated names.
    unsafe {
        let mut m = GetModuleHandleW(w.as_ptr());
        if m == 0 {
            m = LoadLibraryW(w.as_ptr());
        }
        if m == 0 { 0 } else { GetProcAddress(m, name.as_ptr()) }
    }
}

late!("user32.dll", "DialogBoxIndirectParamW",
    fn DialogBoxIndirectParamW(inst: H, template: *const u8, parent: H, proc_: WndProc, param: isize) -> isize = -1);
late!("user32.dll", "DialogBoxParamW",
    fn DialogBoxParamW(inst: H, name: *const u16, parent: H, proc_: WndProc, param: isize) -> isize = -1);
late!("user32.dll", "CreateDialogIndirectParamW",
    fn CreateDialogIndirectParamW(inst: H, template: *const u8, parent: H, proc_: WndProc, param: isize) -> H = 0);
late!("user32.dll", "CreateDialogParamW",
    fn CreateDialogParamW(inst: H, name: *const u16, parent: H, proc_: WndProc, param: isize) -> H = 0);
late!("user32.dll", "EndDialog", fn EndDialog(hdlg: H, result: isize) -> i32 = 0);
late!("user32.dll", "GetDlgItem", fn GetDlgItem(hdlg: H, id: i32) -> H = 0);
late!("user32.dll", "SetDlgItemTextW", fn SetDlgItemTextW(hdlg: H, id: i32, text: *const u16) -> i32 = 0);
late!("user32.dll", "GetDlgItemTextW", fn GetDlgItemTextW(hdlg: H, id: i32, buf: *mut u16, max: i32) -> u32 = 0);
late!("user32.dll", "SendDlgItemMessageW",
    fn SendDlgItemMessageW(hdlg: H, id: i32, msg: u32, w: usize, l: isize) -> isize = 0);
late!("user32.dll", "CheckDlgButton", fn CheckDlgButton(hdlg: H, id: i32, check: u32) -> i32 = 0);
late!("user32.dll", "IsDlgButtonChecked", fn IsDlgButtonChecked(hdlg: H, id: i32) -> u32 = 0);
late!("gdi32.dll", "SetBkMode", fn SetBkMode(hdc: H, mode: i32) -> i32 = 0);
late!("gdi32.dll", "SetTextColor", fn SetTextColor(hdc: H, color: u32) -> u32 = 0);
late!("user32.dll", "MessageBoxW", fn MessageBoxW(owner: H, text: *const u16, caption: *const u16, kind: u32) -> i32 = 0);
late!("user32.dll", "SetDlgItemInt", fn SetDlgItemInt(hdlg: H, id: i32, value: u32, signed: i32) -> i32 = 0);
late!("user32.dll", "GetDlgItemInt",
    fn GetDlgItemInt(hdlg: H, id: i32, translated: *mut i32, signed: i32) -> u32 = 0);

/// A NUL-terminated copy of `s`.
pub fn wz(s: &str) -> alloc::vec::Vec<u16> {
    let mut v: alloc::vec::Vec<u16> = s.encode_utf16().collect();
    v.push(0);
    v
}

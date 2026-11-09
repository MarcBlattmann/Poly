//! Our `imm32.dll` (ADR 0172): input contexts without any IME. Contexts
//! and the window association are win32k's (`poly_win32k::ime`); this DLL
//! maps the `Imm*` calls onto them, sends the `WM_IME_SETCONTEXT` pair an
//! association change makes in the active window, and keeps each thread's
//! default IME window. Sheets: `specs/windows/imm32.dll/Imm*.yaml`.

#![no_std]
#![allow(clippy::missing_safety_doc)]

extern crate alloc;

use alloc::vec;
use alloc::vec::Vec;
use poly_win32_gui::{SpinLock, set_last_error};
use poly_win32_rt as _;
use poly_win32k::ime::{Assoc, ImeOp, ImeRes};
use poly_win32k::proto::{Reply, Request, WinInfo};
use poly_win32k::wire;

#[link(name = "win32u", kind = "raw-dylib")]
unsafe extern "system" {
    fn PolyUserCall(req: *const u8, len: u64, out: *mut u8, cap: u64, out_len: *mut u64) -> u32;
}

#[link(name = "user32", kind = "raw-dylib")]
unsafe extern "system" {
    fn SendMessageW(hwnd: usize, msg: u32, w: usize, l: isize) -> isize;
    fn GetGUIThreadInfo(tid: u32, gti: *mut u8) -> i32;
    fn PrivateLayoutInstalled(hkl: usize) -> i32;
    fn CreateWindowExW(
        ex: u32,
        class: *const u16,
        name: *const u16,
        style: u32,
        x: i32,
        y: i32,
        w: i32,
        h: i32,
        parent: usize,
        menu: usize,
        instance: usize,
        param: *mut u8,
    ) -> usize;
    fn RegisterClassExW(wc: *const u8) -> u16;
    fn DefWindowProcW(hwnd: usize, msg: u32, w: usize, l: isize) -> isize;
    fn IsWindow(hwnd: usize) -> i32;
    fn EnumThreadWindows(tid: u32, f: Option<unsafe extern "system" fn(usize, isize) -> i32>, l: isize) -> i32;
}

#[link(name = "kernel32", kind = "raw-dylib")]
unsafe extern "system" {
    fn GetCurrentThreadId() -> u32;
}

const ERROR_INVALID_HANDLE: u32 = 6;
const ERROR_ACCESS_DENIED: u32 = 5;
const ERROR_INVALID_WINDOW_HANDLE: u32 = 1400;
const WM_IME_SETCONTEXT: u32 = 0x0281;
const WM_IME_NOTIFY: u32 = 0x0282;
const ISC_SHOWUIALL: isize = 0xC000_000Fu32 as i32 as isize;
const IMN_SETCOMPOSITIONWINDOW: usize = 0xB;
const IMM_ERROR_GENERAL: i32 = -2;

fn call(req: Request) -> Reply {
    let bytes = wire::encode(&req);
    let mut out = vec![0u8; 1024];
    let mut len = 0u64;
    // SAFETY: the buffers are ours and sized as passed.
    let s = unsafe { PolyUserCall(bytes.as_ptr(), bytes.len() as u64, out.as_mut_ptr(), out.len() as u64, &mut len) };
    if s != 0 {
        return Reply::Error { code: 87 };
    }
    wire::decode(&out[..len as usize]).unwrap_or(Reply::Error { code: 87 })
}

fn ime(op: ImeOp) -> ImeRes {
    match call(Request::Ime { op }) {
        Reply::Ime { res } => res,
        Reply::Error { code } => ImeRes::Err { code },
        _ => ImeRes::Err { code: 87 },
    }
}

fn info(hwnd: usize) -> Option<WinInfo> {
    if hwnd > u32::MAX as usize {
        return None;
    }
    match call(Request::Window { hwnd: hwnd as u64 }) {
        Reply::Window { info } => Some(info),
        _ => None,
    }
}

/// The context a window has (0: none), resolving the association.
fn context_of(i: &WinInfo) -> u32 {
    match ime(ImeOp::Resolve { code: i.ime, pid: i.pid, tid: i.tid }) {
        ImeRes::Val { value } => value,
        _ => 0,
    }
}

/// `ImmGetContext`.
#[unsafe(no_mangle)]
pub extern "system" fn ImmGetContext(hwnd: usize) -> usize {
    if hwnd == 0 {
        return 0;
    }
    let Some(i) = info(hwnd) else {
        set_last_error(ERROR_INVALID_WINDOW_HANDLE);
        return 0;
    };
    if i.desktop {
        return 0;
    }
    context_of(&i) as usize
}

#[unsafe(no_mangle)]
pub extern "system" fn ImmReleaseContext(_hwnd: usize, _himc: usize) -> i32 {
    1
}

/// Whether `hwnd` is the active window of its thread.
fn is_active(i: &WinInfo) -> bool {
    let mut gti = [0u8; 72];
    gti[0] = 72;
    // SAFETY: a GUITHREADINFO of the right size.
    if unsafe { GetGUIThreadInfo(i.tid, gti.as_mut_ptr()) } == 0 {
        return false;
    }
    let active = u64::from_le_bytes(gti[8..16].try_into().unwrap_or([0; 8]));
    active as u32 == i.hwnd
}

/// The `WM_IME_SETCONTEXT` pair of an association change of the active window.
fn announce(i: &WinInfo) -> bool {
    if !is_active(i) {
        return false;
    }
    // SAFETY: messages to a live window.
    unsafe {
        SendMessageW(i.hwnd as usize, WM_IME_SETCONTEXT, 0, ISC_SHOWUIALL);
        SendMessageW(i.hwnd as usize, WM_IME_SETCONTEXT, 1, ISC_SHOWUIALL);
    }
    true
}

/// Check that `himc` may go to a window of `i`'s thread: the new
/// association, or the error.
fn assoc_for(i: &WinInfo, himc: usize) -> Result<Assoc, u32> {
    if himc == 0 {
        return Ok(Assoc::None);
    }
    if himc > u32::MAX as usize {
        return Err(ERROR_INVALID_HANDLE);
    }
    match ime(ImeOp::MayAssociate { himc: himc as u32, pid: i.pid, tid: i.tid }) {
        ImeRes::Val { value: 1 } => Ok(Assoc::Default),
        ImeRes::Val { .. } => Ok(Assoc::Own(himc as u32)),
        ImeRes::Err { code } => Err(code),
        _ => Err(ERROR_INVALID_HANDLE),
    }
}

#[unsafe(no_mangle)]
pub extern "system" fn ImmAssociateContext(hwnd: usize, himc: usize) -> usize {
    if hwnd == 0 {
        return 0;
    }
    let Some(i) = info(hwnd) else {
        set_last_error(ERROR_INVALID_WINDOW_HANDLE);
        return 0;
    };
    if i.desktop {
        set_last_error(ERROR_ACCESS_DENIED);
        return 0;
    }
    let new = match assoc_for(&i, himc) {
        Ok(a) => a,
        Err(e) => {
            set_last_error(e);
            return 0;
        }
    };
    let prev = context_of(&i);
    let old_code = i.ime;
    call(Request::ImeAssoc { hwnd: i.hwnd, code: new.code() });
    let _ = old_code;
    let now = info(hwnd).map(|n| context_of(&n)).unwrap_or(0);
    if now != prev {
        announce(&i);
        set_last_error(0);
    }
    prev as usize
}

const IACE_CHILDREN: u32 = 0x1;
const IACE_DEFAULT: u32 = 0x10;
const IACE_IGNORENOCONTEXT: u32 = 0x20;

fn children_of(hwnd: u32) -> Vec<u32> {
    match call(Request::Relatives { hwnd }) {
        Reply::Relatives { children, .. } => children,
        _ => Vec::new(),
    }
}

fn apply_ex(i: &WinInfo, himc: usize, flags: u32, changed: &mut Vec<WinInfo>) {
    let skip = flags & IACE_IGNORENOCONTEXT != 0 && Assoc::from_code(i.ime) == Assoc::None;
    if !skip {
        let new = if flags & IACE_DEFAULT != 0 { Assoc::Default } else { assoc_for(i, himc).unwrap_or(Assoc::None) };
        let before = context_of(i);
        call(Request::ImeAssoc { hwnd: i.hwnd, code: new.code() });
        if let Some(n) = info(i.hwnd as usize)
            && context_of(&n) != before
        {
            changed.push(n);
        }
    }
    if flags & IACE_CHILDREN != 0 {
        for c in children_of(i.hwnd) {
            if let Some(ci) = info(c as usize) {
                apply_ex(&ci, himc, flags, changed);
            }
        }
    }
}

#[unsafe(no_mangle)]
pub extern "system" fn ImmAssociateContextEx(hwnd: usize, himc: usize, flags: u32) -> i32 {
    let Some(i) = (if hwnd == 0 { None } else { info(hwnd) }) else {
        set_last_error(ERROR_INVALID_WINDOW_HANDLE);
        return 0;
    };
    if i.desktop {
        set_last_error(ERROR_ACCESS_DENIED);
        return 0;
    }
    // A context of another thread is refused unless the default is asked for.
    if flags & IACE_DEFAULT == 0
        && himc != 0
        && let Err(e) = assoc_for(&i, himc)
    {
        set_last_error(e);
        return 0;
    }
    let mut changed = Vec::new();
    apply_ex(&i, himc, flags, &mut changed);
    for c in &changed {
        announce(c);
    }
    set_last_error(0);
    1
}

#[unsafe(no_mangle)]
pub extern "system" fn ImmCreateContext() -> usize {
    match ime(ImeOp::Create {}) {
        ImeRes::Val { value } => value as usize,
        _ => 0,
    }
}

#[unsafe(no_mangle)]
pub extern "system" fn ImmDestroyContext(himc: usize) -> i32 {
    if himc > u32::MAX as usize {
        set_last_error(ERROR_INVALID_HANDLE);
        return 0;
    }
    match ime(ImeOp::Destroy { himc: himc as u32 }) {
        ImeRes::Ok {} => 1,
        ImeRes::Err { code } => {
            if code != 0 {
                set_last_error(code);
            }
            0
        }
        _ => 0,
    }
}

/// Whether `himc` is a context: NULL is not (and is no error to callers
/// that allow it); a bad handle sets ERROR_INVALID_HANDLE.
fn valid(himc: usize) -> bool {
    if himc > u32::MAX as usize {
        set_last_error(ERROR_INVALID_HANDLE);
        return false;
    }
    match ime(ImeOp::Check { himc: himc as u32 }) {
        ImeRes::Ok {} => true,
        _ => {
            set_last_error(ERROR_INVALID_HANDLE);
            false
        }
    }
}

#[unsafe(no_mangle)]
pub extern "system" fn ImmGetOpenStatus(himc: usize) -> i32 {
    if himc == 0 || !valid(himc) {
        return 0;
    }
    set_last_error(0);
    0
}

/// The twelve `GCS_*` values (`imm.h`); an index must be exactly one.
fn single_gcs(index: u32) -> bool {
    matches!(index, 0x1 | 0x2 | 0x4 | 0x8 | 0x10 | 0x20 | 0x80 | 0x100 | 0x200 | 0x400 | 0x800 | 0x1000)
}

#[unsafe(no_mangle)]
pub extern "system" fn ImmGetCompositionStringW(himc: usize, index: u32, _buf: *mut u8, _len: u32) -> i32 {
    if himc == 0 || !valid(himc) {
        return 0;
    }
    set_last_error(0);
    if single_gcs(index) { 0 } else { IMM_ERROR_GENERAL }
}

#[unsafe(no_mangle)]
pub unsafe extern "system" fn ImmGetIMEFileNameW(hkl: usize, buf: *mut u16, len: u32) -> u32 {
    // SAFETY: a user32 query without preconditions.
    if unsafe { PrivateLayoutInstalled(hkl) } != 0 && !buf.is_null() && len >= 1 {
        // SAFETY: at least one unit.
        unsafe { *buf = 0 };
    }
    0
}

#[unsafe(no_mangle)]
pub extern "system" fn ImmIsIME(hkl: usize) -> i32 {
    // SAFETY: a user32 query without preconditions.
    unsafe { PrivateLayoutInstalled(hkl) }
}

#[unsafe(no_mangle)]
pub extern "system" fn ImmNotifyIME(himc: usize, action: u32, index: u32, _value: u32) -> i32 {
    if himc == 0 {
        set_last_error(0);
        return 0;
    }
    if !valid(himc) {
        return 0;
    }
    set_last_error(0);
    // NI_COMPOSITIONSTR (0x15) with CPS_COMPLETE, CPS_CONVERT or CPS_CANCEL.
    i32::from(action == 0x15 && matches!(index, 1 | 2 | 4))
}

/// COMPOSITIONFORM (28 bytes) and CANDIDATEFORM (32 bytes).
#[unsafe(no_mangle)]
pub unsafe extern "system" fn ImmSetCompositionWindow(himc: usize, form: *const u8) -> i32 {
    if himc == 0 || !valid(himc) || form.is_null() {
        return 0;
    }
    // SAFETY: a COMPOSITIONFORM.
    let bytes = unsafe { core::slice::from_raw_parts(form, 28) }.to_vec();
    ime(ImeOp::Form { himc: himc as u32, kind: 0, set: true, bytes });
    set_last_error(0);
    notify_window(himc, IMN_SETCOMPOSITIONWINDOW);
    1
}

/// `WM_IME_NOTIFY` to the windows of the calling thread that use `himc`
/// (the thread's active window when it has the default context).
fn notify_window(himc: usize, code: usize) {
    // SAFETY: a user32 query and a message to a live window.
    unsafe {
        let mut gti = [0u8; 72];
        gti[0] = 72;
        if GetGUIThreadInfo(GetCurrentThreadId(), gti.as_mut_ptr()) == 0 {
            return;
        }
        let active = u64::from_le_bytes(gti[8..16].try_into().unwrap_or([0; 8])) as usize;
        if active != 0
            && let Some(i) = info(active)
            && context_of(&i) as usize == himc
        {
            SendMessageW(active, WM_IME_NOTIFY, code, 0);
        }
    }
}

#[unsafe(no_mangle)]
pub unsafe extern "system" fn ImmGetCompositionWindow(himc: usize, form: *mut u8) -> i32 {
    get_form(himc, 0, form, 28)
}

#[unsafe(no_mangle)]
pub unsafe extern "system" fn ImmSetCandidateWindow(himc: usize, form: *const u8) -> i32 {
    if himc == 0 || !valid(himc) || form.is_null() {
        return 0;
    }
    // SAFETY: a CANDIDATEFORM (dwIndex first).
    let bytes = unsafe { core::slice::from_raw_parts(form, 32) }.to_vec();
    let index = u32::from_le_bytes([bytes[0], bytes[1], bytes[2], bytes[3]]);
    if index >= 4 {
        // Indices past the four windows fail with the error untouched.
        return 0;
    }
    ime(ImeOp::Form { himc: himc as u32, kind: 1 + index as u8, set: true, bytes });
    set_last_error(0);
    1
}

#[unsafe(no_mangle)]
pub unsafe extern "system" fn ImmGetCandidateWindow(himc: usize, index: u32, form: *mut u8) -> i32 {
    if index >= 4 {
        return 1;
    }
    get_form(himc, 1 + index as u8, form, 32)
}

fn get_form(himc: usize, kind: u8, form: *mut u8, size: usize) -> i32 {
    if himc == 0 || !valid(himc) || form.is_null() {
        return 0;
    }
    match ime(ImeOp::Form { himc: himc as u32, kind, set: false, bytes: Vec::new() }) {
        ImeRes::Bytes { bytes } if bytes.len() >= size => {
            // SAFETY: the caller's structure of `size` bytes.
            unsafe { core::ptr::copy_nonoverlapping(bytes.as_ptr(), form, size) };
            1
        }
        _ => 0,
    }
}

// ----- the default IME window of a thread

/// (thread, its IME window).
static IME_WINDOWS: SpinLock<Vec<(u32, usize)>> = SpinLock::new(Vec::new());

const WS_POPUP: u32 = 0x8000_0000;
const WS_DISABLED: u32 = 0x0800_0000;
const WS_CLIPSIBLINGS: u32 = 0x0400_0000;

static IME_CLASS: [u16; 4] = [b'I' as u16, b'M' as u16, b'E' as u16, 0];

/// Called by `user32` when a thread creates a window: its first one gets
/// the hidden "IME" window, owned by it.
#[unsafe(no_mangle)]
pub extern "system" fn PolyImmWindowCreated(first: usize) {
    // SAFETY: no preconditions.
    let tid = unsafe { GetCurrentThreadId() };
    let have = IME_WINDOWS.with(|w| match w.iter().position(|e| e.0 == tid) {
        Some(k) => {
            // SAFETY: a user32 query. (An entry of 0 is the window being created.)
            if w[k].1 == 0 || unsafe { IsWindow(w[k].1) } != 0 {
                true
            } else {
                w.remove(k);
                false
            }
        }
        None => false,
    });
    if have {
        return;
    }
    // Reserve the slot first: creating the window calls back here.
    IME_WINDOWS.with(|w| w.push((tid, 0)));
    // WNDCLASSEXW (80 bytes): cbSize, style, wndproc, extra, extra, instance, icon, cursor, brush, menu name, class name, small icon.
    let mut wc = [0u8; 80];
    wc[0..4].copy_from_slice(&80u32.to_le_bytes());
    wc[8..16].copy_from_slice(&(DefWindowProcW as *const () as usize as u64).to_le_bytes());
    wc[64..72].copy_from_slice(&(IME_CLASS.as_ptr() as u64).to_le_bytes());
    // SAFETY: a valid class description; the class may exist already.
    unsafe { RegisterClassExW(wc.as_ptr()) };
    // SAFETY: creating a hidden popup owned by the thread's first window.
    let h = unsafe {
        CreateWindowExW(
            0,
            IME_CLASS.as_ptr(),
            IME_CLASS.as_ptr(),
            WS_POPUP | WS_DISABLED | WS_CLIPSIBLINGS,
            0,
            0,
            0,
            0,
            first,
            0,
            0,
            core::ptr::null_mut(),
        )
    };
    IME_WINDOWS.with(|w| {
        if let Some(e) = w.iter_mut().find(|e| e.0 == tid) {
            e.1 = h;
        }
    });
}

unsafe extern "system" fn any_window(_hwnd: usize, l: isize) -> i32 {
    // SAFETY: `l` is our counter.
    unsafe { *(l as *mut u32) += 1 };
    1
}

#[unsafe(no_mangle)]
pub extern "system" fn ImmGetDefaultIMEWnd(hwnd: usize) -> usize {
    let tid = if hwnd == 0 {
        // SAFETY: no preconditions.
        unsafe { GetCurrentThreadId() }
    } else {
        match info(hwnd) {
            Some(i) if !i.desktop => i.tid,
            _ => return 0,
        }
    };
    let found = IME_WINDOWS.with(|w| w.iter().find(|e| e.0 == tid).map(|e| e.1));
    match found {
        // SAFETY: a user32 query.
        Some(h) if h != 0 && unsafe { IsWindow(h) } != 0 => h,
        _ => {
            // The thread's windows may have ended with its IME window.
            let mut n = 0u32;
            // SAFETY: counting callback on our counter.
            unsafe { EnumThreadWindows(tid, Some(any_window), &mut n as *mut u32 as isize) };
            let _ = n;
            0
        }
    }
}

/// The entry point (`DllMain`): nothing to set up.
#[unsafe(no_mangle)]
pub extern "system" fn imm32_main(_module: *mut u8, _reason: u32, _reserved: *mut u8) -> i32 {
    1
}

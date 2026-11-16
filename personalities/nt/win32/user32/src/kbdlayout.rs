//! Keyboard layouts of a process (ADR 0212): the list of loaded layouts, the layout of each
//! thread, `LoadKeyboardLayout`, `UnloadKeyboardLayout`, `ActivateKeyboardLayout` and
//! `VkKeyScan`. The state is `poly_win32k::kbdlist::LayoutSet`; the desktop's layout (which
//! win32k reads keys with, ADR 0209) is its default and the list's middle entry.

use super::{GetFocus, H, PostMessageW, SYS, SendMessageW, call, set_last_error};
use poly_win32_gui::SpinLock;
use poly_win32k::kbd::{self, Layout};
use poly_win32k::kbdlist::{self, LayoutSet};
use poly_win32k::proto::{Reply, Request};
use poly_win32k::user::Env;

/// This process's layouts, made on the first keyboard call.
static LAYOUTS: SpinLock<Option<LayoutSet>> = SpinLock::new(None);

/// `WM_INPUTLANGCHANGE` and `WM_KEYUP` (winuser.h).
const WM_INPUTLANGCHANGE: u32 = 0x0051;
const WM_KEYUP: u32 = 0x0101;

/// Run `f` with the process's layouts, updated to the desktop's current layout.
fn with_layouts<R>(f: impl FnOnce(&mut LayoutSet) -> R) -> R {
    let desktop = match call(Request::KeyboardLayout {}) {
        Reply::Value { value } => Layout::from_id(value),
        _ => Layout::German,
    }
    .hkl();
    LAYOUTS.with(|slot| {
        let set = slot.get_or_insert_with(|| LayoutSet::new(desktop));
        set.set_desktop(desktop);
        f(set)
    })
}

/// The layout of the calling thread (an `HKL`).
pub(crate) fn thread_hkl() -> u32 {
    let tid = SYS.tid();
    with_layouts(|s| s.thread_hkl(tid))
}

/// The layout of thread `tid` of this process (the process's own for one that chose none).
pub(crate) fn hkl_of_thread(tid: u32) -> u32 {
    with_layouts(|s| s.thread_hkl(tid))
}

/// The keyboard layout the calling thread types with, made this process's for the keyboard
/// functions that take no layout.
pub(crate) fn refresh_layout() -> Layout {
    let l = Layout::for_hkl(thread_hkl());
    kbd::set_active(l);
    l
}

/// The layouts of the list (`GetKeyboardLayoutList`).
pub(crate) fn list() -> alloc::vec::Vec<u32> {
    with_layouts(|s| s.list().to_vec())
}

/// Whether `hkl` is in the list.
pub(crate) fn installed(hkl: u32) -> bool {
    with_layouts(|s| s.contains(hkl))
}

/// The table an `HKL` types with: the thread's for NULL, `None` for a layout that is not loaded.
pub(crate) fn layout_of(hkl: usize) -> Option<Layout> {
    if hkl == 0 {
        return Some(Layout::for_hkl(thread_hkl()));
    }
    let h = hkl as u32;
    installed(h).then(|| Layout::for_hkl(h))
}

/// Tell the focus window the thread's layout changed (`WM_INPUTLANGCHANGE`, character set 0), as
/// observed with the Ctrl key's release that follows it on every layout but Swiss German
/// (`ActivateKeyboardLayout.yaml` `message`).
fn announce(hkl: u32) {
    // SAFETY: no preconditions.
    let focus = unsafe { GetFocus() };
    if focus == 0 {
        return;
    }
    SendMessageW(focus, WM_INPUTLANGCHANGE, 0, hkl as isize);
    if Layout::for_hkl(hkl) != Layout::SwissGerman {
        PostMessageW(focus, WM_KEYUP, kbd::VK_CONTROL as usize, 0xC01D_0001u32 as i32 as isize);
    }
}

/// The KLID in `LoadKeyboardLayoutW`'s string, `None` if it is not 8 hexadecimal digits.
unsafe fn read_klid(p: *const u16) -> Option<u32> {
    let mut v = alloc::vec::Vec::new();
    let mut i = 0;
    loop {
        // SAFETY: the caller's NUL-terminated string.
        let c = unsafe { *p.add(i) };
        if c == 0 || i > 16 {
            break;
        }
        v.push(c);
        i += 1;
    }
    kbdlist::parse_klid(&v)
}

fn load(klid: Option<u32>, flags: u32) -> usize {
    let hkl = kbdlist::klid_to_hkl(klid, flags);
    let tid = SYS.tid();
    let act = with_layouts(|s| {
        s.load(hkl);
        if flags & kbdlist::KLF_ACTIVATE != 0 { s.activate(tid, hkl, flags) } else { None }
    });
    if let Some(a) = act
        && a.changed()
    {
        announce(a.current);
    }
    hkl as usize
}

/// `LoadKeyboardLayoutW` (`LoadKeyboardLayoutW.yaml`): the HKL of the KLID, which a name this
/// table does not know gives as the US layout's.
///
/// # Safety
/// `klid` is NULL or a NUL-terminated string.
#[unsafe(no_mangle)]
pub unsafe extern "system" fn LoadKeyboardLayoutW(klid: *const u16, flags: u32) -> usize {
    if klid.is_null() {
        set_last_error(87);
        return 0;
    }
    // SAFETY: the caller's string.
    load(unsafe { read_klid(klid) }, flags)
}

/// `LoadKeyboardLayoutA`: as the wide form.
///
/// # Safety
/// `klid` is NULL or a NUL-terminated string.
#[unsafe(no_mangle)]
pub unsafe extern "system" fn LoadKeyboardLayoutA(klid: *const u8, flags: u32) -> usize {
    if klid.is_null() {
        set_last_error(87);
        return 0;
    }
    let mut v = alloc::vec::Vec::new();
    let mut i = 0;
    // SAFETY: the caller's NUL-terminated string.
    while i <= 16 && unsafe { *klid.add(i) } != 0 {
        // SAFETY: as above.
        v.push(u16::from(unsafe { *klid.add(i) }));
        i += 1;
    }
    load(kbdlist::parse_klid(&v), flags)
}

/// `UnloadKeyboardLayout` (`UnloadKeyboardLayout.yaml`): 1 for a layout this process loaded,
/// 0 (last error unchanged) for any other number; a thread that had it falls back to the list's
/// first layout.
#[unsafe(no_mangle)]
pub extern "system" fn UnloadKeyboardLayout(hkl: usize) -> i32 {
    // NULL and HKL_NEXT are no layouts here (the oracle took them for every installed one; no test does).
    if hkl <= 1 {
        return 0;
    }
    let tid = SYS.tid();
    match with_layouts(|s| s.unload(tid, hkl as u32)) {
        None => 0,
        Some(fell_back) => {
            if let Some(h) = fell_back {
                announce(h);
            }
            1
        }
    }
}

/// `ActivateKeyboardLayout` (`ActivateKeyboardLayout.yaml`): the thread's previous layout, NULL
/// (last error 0) for one that is not loaded.
#[unsafe(no_mangle)]
pub extern "system" fn ActivateKeyboardLayout(hkl: usize, flags: u32) -> usize {
    let tid = SYS.tid();
    let Some(a) = with_layouts(|s| s.activate(tid, hkl as u32, flags)) else {
        set_last_error(0);
        return 0;
    };
    if a.changed() {
        announce(a.current);
    }
    a.previous as usize
}

/// `VkKeyScanExW` (`VkKeyScanExW.yaml`).
#[unsafe(no_mangle)]
pub extern "system" fn VkKeyScanExW(ch: u16, hkl: usize) -> i16 {
    match layout_of(hkl) {
        Some(l) => kbd::vk_key_scan_in(l, ch),
        None => -1,
    }
}

/// `VkKeyScanW`: in the thread's layout.
#[unsafe(no_mangle)]
pub extern "system" fn VkKeyScanW(ch: u16) -> i16 {
    kbd::vk_key_scan_in(refresh_layout(), ch)
}

#[allow(dead_code)]
fn unused(_: H) {}

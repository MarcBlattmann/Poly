//! Showing menus: the modal loop of `TrackPopupMenu(Ex)` with the messages its sheets
//! record (`TrackPopupMenu.yaml`, `TrackPopupMenuEx.yaml`), `EndMenu`, and the drawing
//! of a window's menu bar.

use super::menu::{self, h32, with};
use super::{H, SYS, info};
use core::sync::atomic::{AtomicBool, AtomicU32, Ordering};
use poly_win32_gui::set_last_error;
use poly_win32k::user::{self as u, CMsg};

const ERROR_INVALID_PARAMETER: u32 = 87;
const ERROR_INVALID_HANDLE: u32 = 6;
const ERROR_INVALID_WINDOW_HANDLE: u32 = 1400;

pub const TPM_NONOTIFY: u32 = 0x0080;
pub const TPM_RETURNCMD: u32 = 0x0100;

const WM_COMMAND: u32 = 0x0111;
const WM_INITMENU: u32 = 0x0116;
const WM_INITMENUPOPUP: u32 = 0x0117;
const WM_MENUSELECT: u32 = 0x011F;
const WM_UNINITMENUPOPUP: u32 = 0x0125;
const WM_ENTERMENULOOP: u32 = 0x0211;
const WM_EXITMENULOOP: u32 = 0x0212;
const WM_QUIT: u32 = 0x0012;

/// Set by `EndMenu`: the running menu loop ends.
static ENDED: AtomicBool = AtomicBool::new(false);
/// How many menu loops run (this process).
static LOOPS: AtomicU32 = AtomicU32::new(0);

/// `EndMenu`: ends the menu loop the calling thread runs (a menu loop is modal, so this
/// is called from a window procedure the loop dispatches to).
#[unsafe(no_mangle)]
pub extern "system" fn EndMenu() -> i32 {
    if LOOPS.load(Ordering::Relaxed) == 0 {
        return 0;
    }
    ENDED.store(true, Ordering::Relaxed);
    1
}

fn send(hwnd: H, msg: u32, w: u64, l: u64) -> u64 {
    u::send(&SYS, hwnd as u64, msg, w, l)
}

/// `MAKEWPARAM(lo, hi)`.
fn mk(lo: u32, hi: u32) -> u64 {
    u64::from(lo & 0xFFFF) | (u64::from(hi & 0xFFFF) << 16)
}

/// Run messages until the menu ends; the id the user chose (0: none).
fn run_loop() -> u32 {
    let chosen = 0;
    while !ENDED.load(Ordering::Relaxed) {
        let mut m = CMsg::default();
        match u::get_message(&SYS, &mut m, 0, 0, 0) {
            0 => {
                // WM_QUIT ends the loop and is passed on.
                super::PostQuitMessage(m.w as i32);
                break;
            }
            n if n < 0 => break,
            _ => {}
        }
        if m.message == WM_QUIT {
            break;
        }
        u::translate_message(&SYS, &m);
        u::dispatch_message(&SYS, &m);
    }
    chosen
}

fn track(menu: H, flags: u32, _x: i32, _y: i32, hwnd: H) -> i32 {
    let h = h32(menu);
    let kind = with(|m| m.get(h).map(|x| (x.popup, x.items.len())));
    let Some((popup, count)) = kind else {
        set_last_error(menu::ERROR_INVALID_MENU_HANDLE);
        return 0;
    };
    if info(hwnd).is_none() {
        set_last_error(ERROR_INVALID_WINDOW_HANDLE);
        return 0;
    }
    if !popup {
        set_last_error(ERROR_INVALID_HANDLE);
        return 0;
    }
    if count == 0 {
        return 0;
    }
    let notify = flags & TPM_NONOTIFY == 0;
    ENDED.store(false, Ordering::Relaxed);
    LOOPS.fetch_add(1, Ordering::Relaxed);
    if notify {
        send(hwnd, WM_ENTERMENULOOP, 1, 0);
        send(hwnd, WM_INITMENU, u64::from(h), 0);
        send(hwnd, WM_INITMENUPOPUP, u64::from(h), 0);
    }
    let chosen = run_loop();
    LOOPS.fetch_sub(1, Ordering::Relaxed);
    ENDED.store(false, Ordering::Relaxed);
    if notify {
        send(hwnd, WM_UNINITMENUPOPUP, u64::from(h), 0);
    }
    send(hwnd, WM_MENUSELECT, mk(0, 0xFFFF), 0);
    if notify {
        send(hwnd, WM_EXITMENULOOP, 1, 0);
    }
    if flags & TPM_RETURNCMD != 0 {
        chosen as i32
    } else {
        if chosen != 0 {
            super::PostMessageW(hwnd, WM_COMMAND, chosen as usize, 0);
        }
        1
    }
}

#[unsafe(no_mangle)]
pub unsafe extern "system" fn TrackPopupMenu(
    menu: H,
    flags: u32,
    x: i32,
    y: i32,
    _reserved: i32,
    hwnd: H,
    _rect: *const i32,
) -> i32 {
    track(menu, flags, x, y, hwnd)
}

#[unsafe(no_mangle)]
pub unsafe extern "system" fn TrackPopupMenuEx(menu: H, flags: u32, x: i32, y: i32, hwnd: H, tpm: *const u8) -> i32 {
    if !tpm.is_null() {
        // SAFETY: a TPMPARAMS (cbSize 4 bytes, then the exclusion RECT: 20 bytes).
        let size = unsafe { (tpm as *const u32).read_unaligned() };
        if size != 20 {
            set_last_error(ERROR_INVALID_PARAMETER);
            return 0;
        }
    }
    track(menu, flags, x, y, hwnd)
}

/// Draw the menu bar of window `hwnd` (a window that has none draws nothing).
pub(crate) fn draw_bar(_hwnd: H) {}

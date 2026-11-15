//! Scroll bars: `SetScrollInfo`, `GetScrollInfo`, `SetScrollRange`, `GetScrollRange`,
//! `SetScrollPos`, `GetScrollPos`, `EnableScrollBar`, `ShowScrollBar`, and the messages of the
//! `ScrollBar` control (`SBM_*`). The model and its rules are win32k's (`poly_win32k::scroll`);
//! this file keeps one table of bars per window (state of the process, gone with
//! `WM_NCDESTROY`), sets the `WS_HSCROLL` / `WS_VSCROLL` style bit and the frame when a bar
//! appears or goes, and gives the functions their Win32 shapes.
//!
//! Sources: Learn, "Scroll Bars", the functions' pages, `SCROLLINFO`; the sheets
//! `specs/windows/user32.dll/*Scroll*.yaml`.

use super::{H, SYS, SpinLock, bad_window, call, ctl, info, set_last_error, u};
use alloc::collections::BTreeMap;
use poly_win32k::dlg::sysproc;
use poly_win32k::metrics::{WS_HSCROLL, WS_VSCROLL};
use poly_win32k::proto::{Request, WinInfo};
use poly_win32k::scroll::{self as s, Bar};

const ERROR_NO_SCROLLBARS: u32 = 1447;
const ERROR_INVALID_SCROLLBAR_RANGE: u32 = 1448;
const ERROR_NOACCESS: u32 = 998;
const ERROR_INVALID_PARAMETER: u32 = 87;
const WS_DISABLED: u32 = 0x0800_0000;
const SW_HIDE: i32 = 0;
const SW_SHOWNA: i32 = 8;

/// One bar of a window: its state, and whether the window has it (the style at creation, or a
/// call that gave the window a bar).
#[derive(Clone, Copy)]
struct Slot {
    bar: Bar,
    exists: bool,
}

#[derive(Clone, Copy)]
struct WinBars {
    /// Horizontal, vertical.
    bars: [Slot; 2],
    ctl: Bar,
}

impl WinBars {
    fn new() -> WinBars {
        let slot = Slot { bar: Bar::window(), exists: false };
        WinBars { bars: [slot, slot], ctl: Bar::control() }
    }
}

static BARS: SpinLock<BTreeMap<u32, WinBars>> = SpinLock::new(BTreeMap::new());

/// A window went: its bars go.
pub fn forget(h: H) {
    BARS.with(|m| m.remove(&(h as u32)));
}

/// Forgets the window's bars when dropped, if asked to (the end of a `WM_NCDESTROY`).
pub struct Forget(pub H, pub bool);

impl Drop for Forget {
    fn drop(&mut self) {
        if self.1 {
            forget(self.0);
        }
    }
}

fn is_control(i: &WinInfo) -> bool {
    i.wndproc == sysproc::SCROLLBAR
}

fn style_bit(idx: usize) -> u32 {
    if idx == 0 { WS_HSCROLL } else { WS_VSCROLL }
}

/// Run `f` on the table entry of `h`, made if need be; a bar the style says the window has exists.
fn with_win<R>(h: H, style: u32, f: impl FnOnce(&mut WinBars) -> R) -> R {
    BARS.with(|m| {
        let w = m.entry(h as u32).or_insert_with(WinBars::new);
        for idx in 0..2 {
            if style & style_bit(idx) != 0 {
                w.bars[idx].exists = true;
            }
        }
        f(w)
    })
}

/// The bars a `SB_*` value names for a window (`SB_BOTH`: both; the vertical one answers).
fn indices(bar: i32) -> &'static [usize] {
    match bar {
        s::SB_HORZ => &[0],
        s::SB_VERT => &[1],
        s::SB_BOTH => &[0, 1],
        _ => &[],
    }
}

/// Make the window's style carry (or not carry) bar `idx`, with the frame following:
/// `WM_WINDOWPOSCHANGING`, `WM_NCCALCSIZE`, `WM_WINDOWPOSCHANGED` and `WM_SIZE`, no
/// `WM_STYLECHANGING` (`SetScrollInfo.yaml` `visibility`).
fn set_style_bits(h: H, i: &WinInfo, want: [Option<bool>; 2]) {
    let mut style = i.style;
    for (idx, w) in want.iter().enumerate() {
        match w {
            Some(true) => style |= style_bit(idx),
            Some(false) => style &= !style_bit(idx),
            None => {}
        }
    }
    if style == i.style {
        return;
    }
    call(Request::SetStyle { hwnd: i.hwnd, which: -16, value: style });
    let flags = u::SWP_NOSIZE | u::SWP_NOMOVE | u::SWP_NOZORDER | u::SWP_NOACTIVATE | u::SWP_FRAMECHANGED;
    u::set_window_pos(&SYS, h as u64, 0, 0, 0, 0, 0, flags);
}

// ---- SCROLLINFO

/// `SCROLLINFO` (`winuser.h`).
#[repr(C)]
#[derive(Clone, Copy)]
pub struct ScrollInfo {
    cb_size: u32,
    mask: u32,
    min: i32,
    max: i32,
    page: u32,
    pos: i32,
    track_pos: i32,
}

/// Write the fields of `mask` of `bar` into `si`.
fn write_info(si: &mut ScrollInfo, bar: &Bar, mask: u32) {
    if mask & s::SIF_RANGE != 0 {
        si.min = bar.min;
        si.max = bar.max;
    }
    if mask & s::SIF_PAGE != 0 {
        si.page = bar.page;
    }
    if mask & s::SIF_POS != 0 {
        si.pos = bar.pos;
    }
    if mask & s::SIF_TRACKPOS != 0 {
        si.track_pos = bar.pos;
    }
}

// ---- the functions

#[unsafe(no_mangle)]
pub unsafe extern "system" fn SetScrollInfo(hwnd: H, bar: i32, psi: *const ScrollInfo, _redraw: i32) -> i32 {
    let Some(i) = info(hwnd) else { return bad_window() };
    if !(0..=3).contains(&bar) {
        set_last_error(ERROR_INVALID_PARAMETER);
        return 0;
    }
    if psi.is_null() {
        set_last_error(ERROR_NOACCESS);
        return 0;
    }
    // SAFETY: the caller's SCROLLINFO; cbSize is not checked.
    let si = unsafe { *psi };
    let ctl_window = is_control(&i);
    if bar == s::SB_CTL {
        if !ctl_window {
            return 0;
        }
        return with_win(hwnd, i.style, |w| w.ctl.set_info(si.mask, si.min, si.max, si.page, si.pos));
    }
    if ctl_window {
        set_last_error(ERROR_NO_SCROLLBARS);
        return 0;
    }
    let mut want = [None, None];
    let mut ret = 0;
    for &idx in indices(bar) {
        let (pos, keep) = with_win(hwnd, i.style, |w| {
            let slot = &mut w.bars[idx];
            let created = !slot.exists;
            slot.exists = true;
            let pos = slot.bar.set_info(si.mask, si.min, si.max, si.page, si.pos);
            let has = i.style & style_bit(idx) != 0;
            let keep = if created || si.mask & (s::SIF_RANGE | s::SIF_PAGE) != 0 {
                Some(slot.bar.wants_style(si.mask & s::SIF_DISABLENOSCROLL != 0))
            } else {
                Some(has)
            };
            (pos, keep)
        });
        want[idx] = keep;
        ret = pos;
    }
    set_style_bits(hwnd, &i, want);
    ret
}

#[unsafe(no_mangle)]
pub unsafe extern "system" fn GetScrollInfo(hwnd: H, bar: i32, psi: *mut ScrollInfo) -> i32 {
    let Some(i) = info(hwnd) else { return bad_window() };
    if !(0..=2).contains(&bar) {
        set_last_error(ERROR_INVALID_PARAMETER);
        return 0;
    }
    if psi.is_null() {
        set_last_error(ERROR_NOACCESS);
        return 0;
    }
    // SAFETY: the caller's SCROLLINFO.
    let si = unsafe { &mut *psi };
    if si.cb_size != s::SCROLLINFO_SIZE {
        return 0;
    }
    let ctl_window = is_control(&i);
    let found: Option<Bar> = if bar == s::SB_CTL {
        if !ctl_window {
            return 1;
        }
        Some(with_win(hwnd, i.style, |w| w.ctl))
    } else if ctl_window {
        None
    } else {
        let idx = bar as usize;
        with_win(hwnd, i.style, |w| w.bars[idx].exists.then_some(w.bars[idx].bar))
    };
    let mask = si.mask;
    // Only undefined bits: nothing to say about the bar, not even that there is none.
    if mask & s::SIF_ALL == 0 && mask & !0x1F != 0 {
        return 0;
    }
    let Some(b) = found else {
        set_last_error(ERROR_NO_SCROLLBARS);
        return 0;
    };
    if mask & s::SIF_ALL == 0 {
        return 0;
    }
    write_info(si, &b, mask);
    1
}

#[unsafe(no_mangle)]
pub unsafe extern "system" fn SetScrollRange(hwnd: H, bar: i32, min: i32, max: i32, _redraw: i32) -> i32 {
    let Some(i) = info(hwnd) else { return bad_window() };
    let ctl_window = is_control(&i);
    if bar == s::SB_CTL || ctl_window {
        if !ctl_window {
            return 1;
        }
        let ok = with_win(hwnd, i.style, |w| w.ctl.set_range(min, max));
        if !ok {
            set_last_error(ERROR_INVALID_SCROLLBAR_RANGE);
        }
        return i32::from(ok);
    }
    if !(0..=3).contains(&bar) {
        return 1;
    }
    let mut want = [None, None];
    let mut bad = false;
    // `SB_BOTH` changes the vertical bar only (the sheet's `sb_both`).
    let bars: &[usize] = if bar == s::SB_BOTH { &[1] } else { indices(bar) };
    for &idx in bars {
        let keep = with_win(hwnd, i.style, |w| {
            let slot = &mut w.bars[idx];
            if !slot.bar.set_range(min, max) {
                bad = true;
                return None;
            }
            slot.exists = true;
            Some(slot.bar.wants_style(false))
        });
        want[idx] = keep;
    }
    set_style_bits(hwnd, &i, want);
    if bad {
        set_last_error(ERROR_INVALID_SCROLLBAR_RANGE);
        return 0;
    }
    1
}

#[unsafe(no_mangle)]
pub unsafe extern "system" fn GetScrollRange(hwnd: H, bar: i32, pmin: *mut i32, pmax: *mut i32) -> i32 {
    let Some(i) = info(hwnd) else { return bad_window() };
    if !(0..=2).contains(&bar) {
        set_last_error(ERROR_INVALID_PARAMETER);
        return 0;
    }
    let ctl_window = is_control(&i);
    let (min, max) = if ctl_window {
        with_win(hwnd, i.style, |w| (w.ctl.min, w.ctl.max))
    } else if bar == s::SB_CTL {
        return 1;
    } else {
        with_win(hwnd, i.style, |w| {
            let slot = w.bars[bar as usize];
            if slot.exists { (slot.bar.min, slot.bar.max) } else { (0, 0) }
        })
    };
    // SAFETY: the caller's outputs (Learn: both required; a NULL one is not written).
    unsafe {
        if !pmin.is_null() {
            *pmin = min;
        }
        if !pmax.is_null() {
            *pmax = max;
        }
    }
    1
}

#[unsafe(no_mangle)]
pub extern "system" fn SetScrollPos(hwnd: H, bar: i32, pos: i32, _redraw: i32) -> i32 {
    let Some(i) = info(hwnd) else { return bad_window() };
    if !(0..=3).contains(&bar) {
        set_last_error(ERROR_INVALID_PARAMETER);
        return 0;
    }
    let ctl_window = is_control(&i);
    if bar == s::SB_CTL {
        if !ctl_window {
            return 0;
        }
        return with_win(hwnd, i.style, |w| w.ctl.set_pos(pos));
    }
    if ctl_window {
        set_last_error(ERROR_NO_SCROLLBARS);
        return 0;
    }
    let mut ret = 0;
    let mut missing = false;
    for &idx in indices(bar) {
        match with_win(hwnd, i.style, |w| {
            let slot = &mut w.bars[idx];
            slot.exists.then(|| slot.bar.set_pos(pos))
        }) {
            Some(old) => ret = old,
            None => missing = true,
        }
    }
    if missing && ret == 0 {
        set_last_error(ERROR_NO_SCROLLBARS);
    }
    ret
}

#[unsafe(no_mangle)]
pub extern "system" fn GetScrollPos(hwnd: H, bar: i32) -> i32 {
    let Some(i) = info(hwnd) else { return bad_window() };
    if !(0..=2).contains(&bar) {
        set_last_error(ERROR_INVALID_PARAMETER);
        return 0;
    }
    let ctl_window = is_control(&i);
    if bar == s::SB_CTL {
        if !ctl_window {
            return 0;
        }
        return with_win(hwnd, i.style, |w| w.ctl.pos);
    }
    if ctl_window {
        set_last_error(ERROR_NO_SCROLLBARS);
        return 0;
    }
    match with_win(hwnd, i.style, |w| w.bars[bar as usize].exists.then_some(w.bars[bar as usize].bar.pos)) {
        Some(p) => p,
        None => {
            set_last_error(ERROR_NO_SCROLLBARS);
            0
        }
    }
}

#[unsafe(no_mangle)]
pub extern "system" fn EnableScrollBar(hwnd: H, bar: u32, arrows: u32) -> i32 {
    let Some(i) = info(hwnd) else { return bad_window() };
    if bar > 3 {
        set_last_error(ERROR_INVALID_PARAMETER);
        return 0;
    }
    let bar = bar as i32;
    if bar == s::SB_CTL {
        if !is_control(&i) {
            return 0;
        }
        return control_arrows(hwnd, &i, arrows);
    }
    if is_control(&i) {
        return 0;
    }
    let mut changed = false;
    for &idx in indices(bar) {
        changed |= with_win(hwnd, i.style, |w| w.bars[idx].bar.enable_arrows(arrows));
    }
    i32::from(changed)
}

#[unsafe(no_mangle)]
pub extern "system" fn ShowScrollBar(hwnd: H, bar: i32, show: i32) -> i32 {
    let Some(i) = info(hwnd) else { return bad_window() };
    if !(0..=3).contains(&bar) {
        set_last_error(ERROR_INVALID_PARAMETER);
        return 0;
    }
    if bar == s::SB_CTL {
        if is_control(&i) {
            super::ShowWindow(hwnd, if show != 0 { SW_SHOWNA } else { SW_HIDE });
        }
        return 1;
    }
    if is_control(&i) {
        return 1;
    }
    let mut want = [None, None];
    for &idx in indices(bar) {
        with_win(hwnd, i.style, |w| w.bars[idx].exists = true);
        want[idx] = Some(show != 0);
    }
    set_style_bits(hwnd, &i, want);
    1
}

// ---- the control

/// `SBM_ENABLE_ARROWS` and `EnableScrollBar(SB_CTL)`: both arrows disabled disables the control.
/// What `EnableScrollBar` returns for a change that disables both arrows is 8
/// (`EnableScrollBar.yaml` `control`), for any other change 1, for none 0.
fn control_arrows(hwnd: H, i: &WinInfo, arrows: u32) -> i32 {
    let (changed, state) = with_win(hwnd, i.style, |w| {
        let c = w.ctl.enable_arrows(arrows);
        (c, w.ctl.arrows)
    });
    if !changed {
        return 0;
    }
    super::EnableWindow(hwnd, i32::from(state != s::ESB_DISABLE_BOTH));
    if state == s::ESB_DISABLE_BOTH { 8 } else { 1 }
}

/// The window procedure of the `ScrollBar` class: the `SBM_` messages, then the other controls'
/// defaults.
pub fn control_proc(h: H, msg: u32, w: usize, l: isize) -> isize {
    let Some(i) = info(h) else { return 0 };
    match msg {
        s::SBM_SETPOS => with_win(h, i.style, |x| x.ctl.set_pos(w as i32)) as isize,
        s::SBM_GETPOS => with_win(h, i.style, |x| x.ctl.pos) as isize,
        s::SBM_SETRANGE | s::SBM_SETRANGEREDRAW => {
            with_win(h, i.style, |x| x.ctl.set_info(s::SIF_RANGE, w as i32, l as i32, 0, 0)) as isize
        }
        s::SBM_GETRANGE => {
            let (min, max) = with_win(h, i.style, |x| (x.ctl.min, x.ctl.max));
            // SAFETY: the caller's two integers (NULL ones are skipped).
            unsafe {
                if w != 0 {
                    *(w as *mut i32) = min;
                }
                if l != 0 {
                    *(l as *mut i32) = max;
                }
            }
            (((max as u32 & 0xFFFF) << 16) | (min as u32 & 0xFFFF)) as isize
        }
        s::SBM_ENABLE_ARROWS => {
            let c = with_win(h, i.style, |x| x.ctl.enable_arrows(w as u32));
            if c {
                let state = with_win(h, i.style, |x| x.ctl.arrows);
                if (i.style & WS_DISABLED != 0) != (state == s::ESB_DISABLE_BOTH) {
                    super::EnableWindow(h, i32::from(state != s::ESB_DISABLE_BOTH));
                }
            }
            isize::from(c)
        }
        s::SBM_SETSCROLLINFO => {
            if l == 0 {
                return 0;
            }
            // SAFETY: the caller's SCROLLINFO.
            let si = unsafe { *(l as *const ScrollInfo) };
            with_win(h, i.style, |x| x.ctl.set_info(si.mask, si.min, si.max, si.page, si.pos)) as isize
        }
        s::SBM_GETSCROLLINFO => {
            if l == 0 {
                return 0;
            }
            // SAFETY: the caller's SCROLLINFO.
            let si = unsafe { &mut *(l as *mut ScrollInfo) };
            if si.cb_size != s::SCROLLINFO_SIZE {
                return 0;
            }
            let b = with_win(h, i.style, |x| x.ctl);
            write_info(si, &b, si.mask);
            1
        }
        _ => ctl::other(h, msg, w, l),
    }
}

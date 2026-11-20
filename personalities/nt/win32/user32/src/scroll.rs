//! Scroll bars: `GetScrollInfo`, `SetScrollInfo`, `GetScrollPos`, `SetScrollPos`,
//! `GetScrollRange`, `SetScrollRange`, `ShowScrollBar`, `EnableScrollBar` and the scroll bar
//! control's `SBM_*` messages (`specs/windows/user32.dll/` sheets of those names; the numbers'
//! rules are `poly_win32k::scroll`'s).
//!
//! A window has a vertical and a horizontal bar once it was made with `WS_VSCROLL` /
//! `WS_HSCROLL` or a call gave it one (`SetScrollInfo`, `SetScrollRange`, `ShowScrollBar`); before
//! that, the bar does not exist (`ERROR_NO_SCROLLBARS`). A window's bar is shown (its style bit
//! set and its width taken off the client area) while it has something to scroll; the change
//! sends `WM_WINDOWPOSCHANGING`, `WM_NCCALCSIZE`, `WM_WINDOWPOSCHANGED` and `WM_SIZE` (a frame
//! change), and no style messages. A scroll bar control (class `ScrollBar`) has its own bar
//! (`SB_CTL`) and is never removed. The state is this process's: the functions run on the
//! window's own thread.

use super::{H, SYS, SpinLock, call, info, set_last_error, u};
use alloc::collections::BTreeMap;
use alloc::string::String;
use poly_win32k::metrics::{WS_HSCROLL, WS_VSCROLL};
use poly_win32k::proto::{Reply, Request, WinInfo};
use poly_win32k::scroll::{
    self as sc, Bar, ERROR_NO_SCROLLBARS, SB_BOTH, SB_CTL, SB_HORZ, SB_VERT, SCROLLINFO_SIZE, SIF_ALL,
    SIF_DISABLENOSCROLL, SIF_PAGE, SIF_POS, SIF_RANGE, SIF_TRACKPOS,
};

const ERROR_INVALID_PARAMETER: u32 = 87;
const ERROR_NOACCESS: u32 = 998;
const ERROR_INVALID_WINDOW_HANDLE: u32 = 1400;

/// The bars of one window: `[SB_HORZ, SB_VERT]` (`None`: the window has no such bar), the
/// control's own, and the disabled arrows of each (`[horizontal, vertical, control]`, kept even
/// for a bar that does not exist: `EnableScrollBar.yaml` `window_without_bar`).
#[derive(Clone, Copy)]
struct Bars {
    window: [Option<Bar>; 2],
    ctl: Option<Bar>,
    arrows: [u8; 3],
}

static BARS: SpinLock<BTreeMap<u32, Bars>> = SpinLock::new(BTreeMap::new());

/// Whether `hwnd` is a scroll bar control.
fn is_control(hwnd: H) -> bool {
    match call(Request::ClassName { hwnd: hwnd as u64 }) {
        Reply::Text { text } => String::from_utf16_lossy(&text).eq_ignore_ascii_case("ScrollBar"),
        _ => false,
    }
}

/// Runs `f` on the window's bars, made on first use from its style (a bar for each scroll style
/// bit, at the window defaults; a control's own bar at the control's).
fn with<R>(hwnd: H, i: &WinInfo, control: bool, f: impl FnOnce(&mut Bars) -> R) -> R {
    BARS.with(|m| {
        let b = m.entry(hwnd as u32).or_insert_with(|| Bars {
            window: [
                (i.style & WS_HSCROLL != 0).then_some(Bar::WINDOW),
                (i.style & WS_VSCROLL != 0).then_some(Bar::WINDOW),
            ],
            ctl: control.then_some(Bar::CONTROL),
            arrows: [0; 3],
        });
        if control && b.ctl.is_none() {
            b.ctl = Some(Bar::CONTROL);
        }
        f(b)
    })
}

/// Forgets the bars of windows that are gone (after a `DestroyWindow`).
pub fn forget_gone() {
    let held: alloc::vec::Vec<u32> = BARS.with(|m| m.keys().copied().collect());
    for h in held {
        if info(h as H).is_none() {
            BARS.with(|m| m.remove(&h));
        }
    }
}

/// Shows or hides a window's bars (`Some(true)`: shown); a change of the style is a frame
/// change.
fn show(hwnd: H, horz: Option<bool>, vert: Option<bool>) {
    let Some(i) = info(hwnd) else { return };
    let mut style = i.style;
    for (want, bit) in [(horz, WS_HSCROLL), (vert, WS_VSCROLL)] {
        match want {
            Some(true) => style |= bit,
            Some(false) => style &= !bit,
            None => {}
        }
    }
    if style != i.style {
        call(Request::SetStyle { hwnd: hwnd as u32, which: -16, value: style });
        let flags = u::SWP_NOMOVE | u::SWP_NOSIZE | u::SWP_NOZORDER | u::SWP_NOACTIVATE | u::SWP_FRAMECHANGED;
        u::set_window_pos(&SYS, hwnd as u64, 0, 0, 0, 0, 0, flags);
    }
}

/// The window's information, or `ERROR_INVALID_WINDOW_HANDLE`.
fn window(hwnd: H) -> Option<WinInfo> {
    let i = if hwnd == 0 { None } else { info(hwnd) };
    if i.is_none() {
        set_last_error(ERROR_INVALID_WINDOW_HANDLE);
    }
    i
}

fn fail<T>(code: u32, v: T) -> T {
    set_last_error(code);
    v
}

/// `SCROLLINFO`: cbSize, fMask, nMin, nMax, nPage, nPos, nTrackPos (seven 32-bit fields).
unsafe fn read_info(p: *const u8) -> [u32; 7] {
    // SAFETY: the caller's SCROLLINFO of 28 bytes.
    core::array::from_fn(|k| unsafe { (p.add(4 * k) as *const u32).read_unaligned() })
}

/// Writes the fields `mask` names (the track position is the position: no drag goes on).
unsafe fn write_info(p: *mut u8, mask: u32, b: &Bar) {
    let put = |k: usize, v: u32| {
        // SAFETY: the caller's SCROLLINFO of 28 bytes.
        unsafe { (p.add(4 * k) as *mut u32).write_unaligned(v) }
    };
    if mask & SIF_RANGE != 0 {
        put(2, b.min as u32);
        put(3, b.max as u32);
    }
    if mask & SIF_PAGE != 0 {
        put(4, b.page);
    }
    if mask & SIF_POS != 0 {
        put(5, b.pos as u32);
    }
    if mask & SIF_TRACKPOS != 0 {
        put(6, b.pos as u32);
    }
}

/// The index of a window bar in [`Bars::window`]; `SB_BOTH` is the vertical bar where a call
/// changes one bar (`SetScrollRange.yaml` `sb_both`, `SetScrollPos.yaml` `bars`).
fn slot(bar: i32) -> usize {
    if bar == SB_HORZ { 0 } else { 1 }
}

#[unsafe(no_mangle)]
pub unsafe extern "system" fn GetScrollInfo(hwnd: H, bar: i32, si: *mut u8) -> i32 {
    // A NULL structure crashes on Windows (not specified); it fails here.
    // SAFETY: the caller's SCROLLINFO when non-null.
    if si.is_null() || unsafe { read_info(si) }[0] != SCROLLINFO_SIZE {
        return 0;
    }
    let Some(i) = window(hwnd) else { return 0 };
    if !(SB_HORZ..=SB_CTL).contains(&bar) {
        return fail(ERROR_INVALID_PARAMETER, 0);
    }
    // SAFETY: as above.
    let mask = unsafe { read_info(si) }[1];
    let found = if bar == SB_CTL {
        if !is_control(hwnd) {
            // SB_CTL on an ordinary window: TRUE, nothing written.
            return 1;
        }
        with(hwnd, &i, true, |b| b.ctl)
    } else {
        with(hwnd, &i, false, |b| b.window[slot(bar)])
    };
    let Some(b) = found else {
        // Only undefined bits: FALSE with the last error as it was; otherwise no bar.
        if mask != 0 && mask & 0x1F == 0 {
            return 0;
        }
        return fail(ERROR_NO_SCROLLBARS, 0);
    };
    if mask & SIF_ALL == 0 {
        return 0;
    }
    // SAFETY: as above.
    unsafe { write_info(si, mask, &b) };
    1
}

#[unsafe(no_mangle)]
pub unsafe extern "system" fn SetScrollInfo(hwnd: H, bar: i32, si: *const u8, _redraw: i32) -> i32 {
    let Some(i) = window(hwnd) else { return 0 };
    if !(SB_HORZ..=SB_BOTH).contains(&bar) {
        return fail(ERROR_INVALID_PARAMETER, 0);
    }
    if si.is_null() {
        return fail(ERROR_NOACCESS, 0);
    }
    // SAFETY: the caller's SCROLLINFO (its cbSize is not checked).
    let f = unsafe { read_info(si) };
    let (mask, min, max, page, pos) = (f[1], f[2] as i32, f[3] as i32, f[4], f[5] as i32);
    if bar == SB_CTL {
        if !is_control(hwnd) {
            return 0;
        }
        return with(hwnd, &i, true, |b| b.ctl.get_or_insert(Bar::CONTROL).set_info(mask, min, max, page, pos));
    }
    let s = slot(bar);
    let (r, shown) = with(hwnd, &i, false, |b| {
        let bar = b.window[s].get_or_insert(Bar::WINDOW);
        let r = bar.set_info(mask, min, max, page, pos);
        let idle = bar.idle();
        if mask & SIF_DISABLENOSCROLL != 0 {
            b.arrows[s] = if idle { sc::ESB_DISABLE_BOTH } else { 0 };
        }
        (r, !idle || mask & SIF_DISABLENOSCROLL != 0)
    });
    if s == 0 {
        show(hwnd, Some(shown), None)
    } else {
        show(hwnd, None, Some(shown))
    }
    r
}

#[unsafe(no_mangle)]
pub extern "system" fn GetScrollPos(hwnd: H, bar: i32) -> i32 {
    let Some(i) = window(hwnd) else { return 0 };
    if !(SB_HORZ..=SB_CTL).contains(&bar) {
        return fail(ERROR_INVALID_PARAMETER, 0);
    }
    if bar == SB_CTL {
        if !is_control(hwnd) {
            return 0;
        }
        return with(hwnd, &i, true, |b| b.ctl.map_or(0, |c| c.pos));
    }
    match with(hwnd, &i, false, |b| b.window[slot(bar)]) {
        Some(b) => b.pos,
        None => fail(ERROR_NO_SCROLLBARS, 0),
    }
}

#[unsafe(no_mangle)]
pub extern "system" fn SetScrollPos(hwnd: H, bar: i32, pos: i32, _redraw: i32) -> i32 {
    let Some(i) = window(hwnd) else { return 0 };
    if !(SB_HORZ..=SB_BOTH).contains(&bar) {
        return fail(ERROR_INVALID_PARAMETER, 0);
    }
    if bar == SB_CTL {
        if !is_control(hwnd) {
            return 0;
        }
        return with(hwnd, &i, true, |b| b.ctl.get_or_insert(Bar::CONTROL).set_pos(pos));
    }
    // The position is stored also on a hidden bar, which stays hidden; no bar: nothing is made.
    match with(hwnd, &i, false, |b| b.window[slot(bar)].as_mut().map(|w| w.set_pos(pos))) {
        Some(old) => old,
        None => fail(ERROR_NO_SCROLLBARS, 0),
    }
}

#[unsafe(no_mangle)]
pub unsafe extern "system" fn GetScrollRange(hwnd: H, bar: i32, min: *mut i32, max: *mut i32) -> i32 {
    let Some(i) = window(hwnd) else { return 0 };
    if !(SB_HORZ..=SB_CTL).contains(&bar) {
        return fail(ERROR_INVALID_PARAMETER, 0);
    }
    let range = if bar == SB_CTL {
        if !is_control(hwnd) {
            // TRUE with the caller's numbers untouched.
            return 1;
        }
        with(hwnd, &i, true, |b| b.ctl)
    } else {
        with(hwnd, &i, false, |b| b.window[slot(bar)])
    }
    .map_or((0, 0), |b| (b.min, b.max));
    // SAFETY: the caller's two ints when non-null.
    unsafe {
        if !min.is_null() {
            min.write_unaligned(range.0);
        }
        if !max.is_null() {
            max.write_unaligned(range.1);
        }
    }
    1
}

#[unsafe(no_mangle)]
pub extern "system" fn SetScrollRange(hwnd: H, bar: i32, min: i32, max: i32, _redraw: i32) -> i32 {
    let Some(i) = window(hwnd) else { return 0 };
    // Undefined bars and SB_CTL on an ordinary window: TRUE, nothing changes.
    if !(SB_HORZ..=SB_BOTH).contains(&bar) || (bar == SB_CTL && !is_control(hwnd)) {
        return 1;
    }
    if bar == SB_CTL {
        return match with(hwnd, &i, true, |b| b.ctl.get_or_insert(Bar::CONTROL).set_range(min, max)) {
            Ok(()) => 1,
            Err(e) => fail(e, 0),
        };
    }
    let s = slot(bar);
    let r = with(hwnd, &i, false, |b| {
        let mut w = b.window[s].unwrap_or(Bar::WINDOW);
        w.set_range(min, max)?;
        b.window[s] = Some(w);
        Ok::<bool, u32>(!w.idle())
    });
    match r {
        Ok(shown) => {
            if s == 0 {
                show(hwnd, Some(shown), None)
            } else {
                show(hwnd, None, Some(shown))
            }
            1
        }
        Err(e) => fail(e, 0),
    }
}

#[unsafe(no_mangle)]
pub extern "system" fn ShowScrollBar(hwnd: H, bar: i32, show_it: i32) -> i32 {
    let Some(i) = window(hwnd) else { return 0 };
    if !(SB_HORZ..=SB_BOTH).contains(&bar) {
        return fail(ERROR_INVALID_PARAMETER, 0);
    }
    let on = show_it != 0;
    if bar == SB_CTL {
        if is_control(hwnd) {
            // SW_SHOW / SW_HIDE of the control itself.
            super::ShowWindow(hwnd, if on { 5 } else { 0 });
        }
        return 1;
    }
    let horz = (bar != SB_VERT).then_some(on);
    let vert = (bar != SB_HORZ).then_some(on);
    if on {
        // A bar that is shown exists (at the window defaults when it is new).
        with(hwnd, &i, false, |b| {
            for (k, want) in [horz, vert].into_iter().enumerate() {
                if want.is_some() && b.window[k].is_none() {
                    b.window[k] = Some(Bar::WINDOW);
                }
            }
        });
    }
    show(hwnd, horz, vert);
    1
}

#[unsafe(no_mangle)]
pub extern "system" fn EnableScrollBar(hwnd: H, which: u32, arrows: u32) -> i32 {
    let Some(i) = window(hwnd) else { return 0 };
    if which > SB_BOTH as u32 {
        return fail(ERROR_INVALID_PARAMETER, 0);
    }
    if which == SB_CTL as u32 {
        if !is_control(hwnd) {
            return 0;
        }
        let (now, changed) = with(hwnd, &i, true, |b| {
            let (now, changed) = sc::enable_arrows(b.arrows[2], arrows);
            b.arrows[2] = now;
            (now, changed)
        });
        if !changed {
            return 0;
        }
        // Both arrows disabled disables the control (the result is 8, as observed); enabling
        // both enables it.
        return match now {
            sc::ESB_DISABLE_BOTH => {
                super::EnableWindow(hwnd, 0);
                8
            }
            0 => {
                super::EnableWindow(hwnd, 1);
                1
            }
            _ => 1,
        };
    }
    let bars: &[usize] = match which as i32 {
        SB_HORZ => &[0],
        SB_VERT => &[1],
        _ => &[0, 1],
    };
    let changed = with(hwnd, &i, false, |b| {
        let mut any = false;
        for &k in bars {
            let (now, c) = sc::enable_arrows(b.arrows[k], arrows);
            b.arrows[k] = now;
            any |= c;
        }
        any
    });
    i32::from(changed)
}

/// The scroll bar control's messages; `None` for the others (the default procedure's).
pub fn control_message(h: H, msg: u32, w: usize, l: isize) -> Option<isize> {
    let i = info(h)?;
    let r = match msg {
        sc::SBM_SETPOS => with(h, &i, true, |b| b.ctl.get_or_insert(Bar::CONTROL).set_pos(w as i32)) as isize,
        sc::SBM_GETPOS => with(h, &i, true, |b| b.ctl.map_or(0, |c| c.pos)) as isize,
        sc::SBM_SETRANGE | sc::SBM_SETRANGEREDRAW => {
            // The page and position are kept (the position moved into the range); the result is
            // the position.
            match with(h, &i, true, |b| {
                let c = b.ctl.get_or_insert(Bar::CONTROL);
                c.set_range(w as i32, l as i32).map(|()| c.pos)
            }) {
                Ok(p) => p as isize,
                Err(_) => 0,
            }
        }
        sc::SBM_GETRANGE => {
            let c = with(h, &i, true, |b| b.ctl).unwrap_or(Bar::CONTROL);
            // SAFETY: wParam and lParam point at the caller's ints when non-zero.
            unsafe {
                if w != 0 {
                    (w as *mut i32).write_unaligned(c.min);
                }
                if l != 0 {
                    (l as *mut i32).write_unaligned(c.max);
                }
            }
            (((c.max as u32 & 0xFFFF) << 16) | (c.min as u32 & 0xFFFF)) as i32 as isize
        }
        sc::SBM_ENABLE_ARROWS => with(h, &i, true, |b| {
            let (now, changed) = sc::enable_arrows(b.arrows[2], w as u32);
            b.arrows[2] = now;
            isize::from(changed)
        }),
        sc::SBM_SETSCROLLINFO => {
            if l == 0 {
                return Some(0);
            }
            // SAFETY: lParam is the caller's SCROLLINFO.
            let f = unsafe { read_info(l as *const u8) };
            with(h, &i, true, |b| {
                b.ctl.get_or_insert(Bar::CONTROL).set_info(f[1], f[2] as i32, f[3] as i32, f[4], f[5] as i32)
            }) as isize
        }
        sc::SBM_GETSCROLLINFO => {
            if l == 0 {
                return Some(0);
            }
            // SAFETY: lParam is the caller's SCROLLINFO.
            let mask = unsafe { read_info(l as *const u8) }[1];
            let c = with(h, &i, true, |b| b.ctl).unwrap_or(Bar::CONTROL);
            // SAFETY: as above.
            unsafe { write_info(l as *mut u8, mask, &c) };
            1
        }
        super::ctl::WM_NCDESTROY => {
            BARS.with(|m| m.remove(&(h as u32)));
            return None;
        }
        _ => return None,
    };
    Some(r)
}

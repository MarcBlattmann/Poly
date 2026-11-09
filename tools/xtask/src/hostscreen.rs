//! The host's main screen, for the size, scale and place of the desktop's
//! QEMU window (`cargo xtask desktop`): on Windows its work area in
//! physical pixels and its DPI, and QEMU's window moved onto it once it has
//! the guest's size (user32, declared here: no crate for a few calls);
//! elsewhere unknown, and the defaults apply.

/// The main screen: its work area (left, top, width, height) in physical
/// pixels, and its scale in 1/96ths (96 is 100%).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Screen {
    pub work: (i32, i32, u32, u32),
    pub dpi: u32,
}

/// The screen size and integer scale for a desktop of `base` logical
/// pixels on `screen`. The scale is the screen's (rounded), less while
/// `base` at it would not fit (with `title` logical pixels of title bar);
/// at scale 1 a screen too small gets the largest size of `base`'s shape
/// that fits.
pub fn fit(base: (u32, u32), screen: Screen, title: u32) -> ((u32, u32), u32) {
    let (_, _, ww, wh) = screen.work;
    let avail = (ww, wh.saturating_sub(title * screen.dpi / 96));
    let mut scale = ((screen.dpi + 48) / 96).clamp(1, 4);
    while scale > 1 && (base.0 * scale > avail.0 || base.1 * scale > avail.1) {
        scale -= 1;
    }
    let mut size = (base.0 * scale, base.1 * scale);
    if size.0 > avail.0 || size.1 > avail.1 {
        let k = f64::min(f64::from(avail.0) / f64::from(base.0), f64::from(avail.1) / f64::from(base.1));
        size = (((f64::from(base.0) * k) as u32).max(320), ((f64::from(base.1) * k) as u32).max(240));
    }
    (size, scale)
}

/// Where a window of `outer` size (title bar and borders included) goes on
/// `screen`: centred in the work area, its top edge never above it.
// Used by `place_window`, which only Windows has.
#[cfg_attr(not(windows), allow(dead_code))]
pub fn centre(outer: (u32, u32), screen: Screen) -> (i32, i32) {
    let (wl, wt, ww, wh) = screen.work;
    let left = wl + (i64::from(ww) - i64::from(outer.0)).max(0) as i32 / 2;
    let top = wt + (i64::from(wh) - i64::from(outer.1)).max(0) as i32 / 2;
    (left, top)
}

#[cfg(windows)]
mod win {
    #[repr(C)]
    pub struct Rect {
        pub left: i32,
        pub top: i32,
        pub right: i32,
        pub bottom: i32,
    }
    #[repr(C)]
    pub struct MonitorInfo {
        pub size: u32,
        pub monitor: Rect,
        pub work: Rect,
        pub flags: u32,
    }
    #[repr(C)]
    pub struct Point {
        pub x: i32,
        pub y: i32,
    }
    pub type EnumProc = unsafe extern "system" fn(hwnd: isize, lparam: isize) -> i32;
    #[link(name = "user32")]
    unsafe extern "system" {
        pub fn SetThreadDpiAwarenessContext(context: isize) -> isize;
        pub fn MonitorFromPoint(pt: Point, flags: u32) -> isize;
        pub fn GetMonitorInfoW(monitor: isize, info: *mut MonitorInfo) -> i32;
        pub fn GetDpiForSystem() -> u32;
        pub fn EnumWindows(proc: EnumProc, lparam: isize) -> i32;
        pub fn GetWindowThreadProcessId(hwnd: isize, pid: *mut u32) -> u32;
        pub fn IsWindowVisible(hwnd: isize) -> i32;
        pub fn GetClientRect(hwnd: isize, rect: *mut Rect) -> i32;
        pub fn GetWindowRect(hwnd: isize, rect: *mut Rect) -> i32;
        pub fn SetWindowPos(hwnd: isize, after: isize, x: i32, y: i32, cx: i32, cy: i32, flags: u32) -> i32;
    }
    // WinUser.h: DPI_AWARENESS_CONTEXT_PER_MONITOR_AWARE_V2 is (HANDLE)-4,
    // MONITOR_DEFAULTTOPRIMARY 1; SWP_NOSIZE 0x1, SWP_NOZORDER 0x4,
    // SWP_NOACTIVATE 0x10.
    pub const PER_MONITOR_AWARE_V2: isize = -4;
    pub const DEFAULT_TO_PRIMARY: u32 = 1;
    pub const SWP_MOVE_ONLY: u32 = 0x1 | 0x4 | 0x10;

    pub fn empty() -> Rect {
        Rect { left: 0, top: 0, right: 0, bottom: 0 }
    }

    /// Run `f` with this thread per-monitor DPI aware (sizes in physical
    /// pixels), then put the thread's awareness back.
    pub fn physical<T>(f: impl FnOnce() -> T) -> T {
        // SAFETY: changes only this thread's DPI awareness.
        let old = unsafe { SetThreadDpiAwarenessContext(PER_MONITOR_AWARE_V2) };
        let r = f();
        if old != 0 {
            // SAFETY: restores the awareness this thread had.
            unsafe { SetThreadDpiAwarenessContext(old) };
        }
        r
    }
}

/// The main screen, where it can be found out.
#[cfg(windows)]
pub fn main_screen() -> Option<Screen> {
    use win::*;
    physical(|| {
        // SAFETY: a plain value argument; DEFAULT_TO_PRIMARY returns the
        // primary monitor whatever the point.
        let monitor = unsafe { MonitorFromPoint(Point { x: 0, y: 0 }, DEFAULT_TO_PRIMARY) };
        let mut info =
            MonitorInfo { size: std::mem::size_of::<MonitorInfo>() as u32, monitor: empty(), work: empty(), flags: 0 };
        // SAFETY: `info` is a MONITORINFO with its size set, as
        // GetMonitorInfoW requires. GetDpiForSystem takes nothing: the main
        // screen's DPI (the system DPI, which per-monitor aware threads also
        // get).
        let (ok, dpi) = unsafe { (GetMonitorInfoW(monitor, &mut info) != 0, GetDpiForSystem()) };
        let w = &info.work;
        let size = (u32::try_from(w.right - w.left).ok()?, u32::try_from(w.bottom - w.top).ok()?);
        (ok && dpi > 0 && size.0 > 0).then_some(Screen { work: (w.left, w.top, size.0, size.1), dpi })
    })
}

#[cfg(not(windows))]
pub fn main_screen() -> Option<Screen> {
    None
}

/// Centre the window of process `pid` on `screen` once its client area is
/// `client` wide (QEMU's window starts small and grows when the guest sets
/// its mode, and SDL does not keep a start position through that). Runs on
/// a thread of its own and gives up after two minutes.
#[cfg(windows)]
pub fn place_window(pid: u32, client: (u32, u32), screen: Screen) {
    use win::*;
    struct Search {
        pid: u32,
        width: i32,
        found: isize,
    }
    unsafe extern "system" fn each(hwnd: isize, lparam: isize) -> i32 {
        // SAFETY: `lparam` is the `Search` that `EnumWindows` was given
        // below, alive and not otherwise borrowed during the call.
        let s = unsafe { &mut *(lparam as *mut Search) };
        let mut owner = 0u32;
        let mut r = empty();
        // SAFETY: `hwnd` comes from EnumWindows; the outputs are locals.
        let ours = unsafe {
            GetWindowThreadProcessId(hwnd, &mut owner);
            owner == s.pid && IsWindowVisible(hwnd) != 0 && GetClientRect(hwnd, &mut r) != 0
        };
        if ours && (r.right - r.left - s.width).abs() <= 2 {
            s.found = hwnd;
            return 0;
        }
        1
    }
    std::thread::spawn(move || {
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(120);
        while std::time::Instant::now() < deadline {
            let done = physical(|| {
                let mut s = Search { pid, width: client.0 as i32, found: 0 };
                // SAFETY: `each` only reads `s` through the pointer, during
                // this call.
                unsafe { EnumWindows(each, &mut s as *mut Search as isize) };
                if s.found == 0 {
                    return false;
                }
                let mut r = empty();
                // SAFETY: a window handle just found; `r` is a local.
                if unsafe { GetWindowRect(s.found, &mut r) } == 0 {
                    return false;
                }
                let outer = ((r.right - r.left).max(0) as u32, (r.bottom - r.top).max(0) as u32);
                let (x, y) = centre(outer, screen);
                // SAFETY: moves the window only (no size, order or focus
                // change).
                unsafe { SetWindowPos(s.found, 0, x, y, 0, 0, SWP_MOVE_ONLY) };
                true
            });
            if done {
                return;
            }
            std::thread::sleep(std::time::Duration::from_millis(250));
        }
    });
}

#[cfg(not(windows))]
pub fn place_window(_pid: u32, _client: (u32, u32), _screen: Screen) {}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_200_percent_laptop_gets_twice_the_size_at_scale_2() {
        let s = Screen { work: (0, 0, 2880, 1704), dpi: 192 };
        assert_eq!(fit((1280, 800), s, 32), ((2560, 1600), 2));
        let (x, y) = centre((2586, 1671), s);
        assert_eq!((x, y), (147, 16));
    }

    #[test]
    fn a_screen_too_small_for_scale_2_drops_to_1() {
        // 1920x1080 at 150%: 2560x1600 does not fit.
        let s = Screen { work: (0, 0, 1920, 1032), dpi: 144 };
        assert_eq!(fit((1280, 800), s, 32), ((1280, 800), 1));
    }

    #[test]
    fn a_tiny_screen_gets_the_largest_size_of_the_same_shape() {
        let s = Screen { work: (0, 0, 1366, 728), dpi: 96 };
        let (size, scale) = fit((1280, 800), s, 32);
        assert_eq!(scale, 1);
        assert!(size.0 <= 1366 && size.1 <= 728 - 32, "{size:?}");
        assert!((f64::from(size.0) / f64::from(size.1) - 1.6).abs() < 0.01, "{size:?}");
    }

    #[test]
    fn a_100_percent_screen_keeps_the_base_size() {
        let s = Screen { work: (-659, -1440, 2560, 1400), dpi: 96 };
        assert_eq!(fit((1280, 800), s, 32), ((1280, 800), 1));
        assert_eq!(centre((1306, 871), s), (-659 + 627, -1440 + 264));
    }

    #[test]
    fn a_window_larger_than_the_screen_starts_at_its_top_left() {
        let s = Screen { work: (0, 0, 1000, 700), dpi: 96 };
        assert_eq!(centre((1306, 871), s), (0, 0));
    }
}

//! SystemParametersInfoForDpi at 96, 144, 192 and 288 next to SystemParametersInfoW, on a display
//! scaled to 200 percent; the program is a process that never sets its awareness (it starts DPI
//! unaware).
#![no_std]
#![no_main]
#![allow(dead_code, non_camel_case_types, clippy::all)]

// The declarations below are from Microsoft Learn (prototypes) and the
// public SDK headers winuser.h, wingdi.h, windef.h, winbase.h and
// winerror.h (constants, layouts); the helpers are this test's own.

use core::ffi::c_void;
use core::fmt::Write as _;
use poly_wtest::*;

type BOOL = i32;
const WM_NULL: u32 = 0x0000;
const WM_CREATE: u32 = 0x0001;
const WM_DESTROY: u32 = 0x0002;
const WM_MOVE: u32 = 0x0003;
const WM_SIZE: u32 = 0x0005;
const WM_ENABLE: u32 = 0x000A;
const WM_SETTEXT: u32 = 0x000C;
const WM_GETTEXT: u32 = 0x000D;
const WM_GETTEXTLENGTH: u32 = 0x000E;
const WM_PAINT: u32 = 0x000F;
const WM_CLOSE: u32 = 0x0010;
const WM_QUIT: u32 = 0x0012;
const WM_ERASEBKGND: u32 = 0x0014;
const WM_SHOWWINDOW: u32 = 0x0018;
const WM_GETMINMAXINFO: u32 = 0x0024;
const WM_WINDOWPOSCHANGING: u32 = 0x0046;
const WM_WINDOWPOSCHANGED: u32 = 0x0047;
const WM_STYLECHANGING: u32 = 0x007C;
const WM_STYLECHANGED: u32 = 0x007D;
const WM_NCCREATE: u32 = 0x0081;
const WM_NCDESTROY: u32 = 0x0082;
const WM_NCCALCSIZE: u32 = 0x0083;
const WM_NCPAINT: u32 = 0x0085;
const WM_KEYDOWN: u32 = 0x0100;
const WM_KEYUP: u32 = 0x0101;
const WM_CHAR: u32 = 0x0102;
const WM_DEADCHAR: u32 = 0x0103;
const WM_SYSKEYDOWN: u32 = 0x0104;
const WM_SYSKEYUP: u32 = 0x0105;
const WM_SYSCHAR: u32 = 0x0106;
const WM_COMMAND: u32 = 0x0111;
const WM_SYSCOMMAND: u32 = 0x0112;
const WM_TIMER: u32 = 0x0113;
const WM_PARENTNOTIFY: u32 = 0x0210;
const WM_CONTEXTMENU: u32 = 0x007B;
const WM_CHILDACTIVATE: u32 = 0x0022;
const WM_USER: u32 = 0x0400;
/// A fixed-size text buffer for formatting lines.
struct Buf<const N: usize> {
    b: [u8; N],
    n: usize,
}

impl<const N: usize> Buf<N> {
    const fn new() -> Self {
        Buf { b: [0; N], n: 0 }
    }
    fn as_str(&self) -> &str {
        core::str::from_utf8(&self.b[..self.n]).unwrap_or("?")
    }
}

impl<const N: usize> core::fmt::Write for Buf<N> {
    fn write_str(&mut self, s: &str) -> core::fmt::Result {
        for &c in s.as_bytes() {
            if self.n < N {
                self.b[self.n] = c;
                self.n += 1;
            }
        }
        Ok(())
    }
}

/// `t.line` with formatting.
macro_rules! say {
    ($t:expr, $($a:tt)*) => {{
        let mut b = Buf::<300>::new();
        let _ = write!(b, $($a)*);
        $t.line(b.as_str());
    }};
}

/// The documented messages the sequence tests record; everything else
/// (undocumented and environment-dependent traffic) is left out, and so are
/// the painting messages (WM_PAINT, WM_ERASEBKGND, WM_NCPAINT), which come
/// only for windows on a visible desktop.
fn documented(msg: u32) -> bool {
    matches!(
        msg,
        WM_NCCREATE
            | WM_CREATE
            | WM_NCCALCSIZE
            | WM_GETMINMAXINFO
            | WM_SIZE
            | WM_MOVE
            | WM_SHOWWINDOW
            | WM_WINDOWPOSCHANGING
            | WM_WINDOWPOSCHANGED
            | WM_DESTROY
            | WM_NCDESTROY
            | WM_PARENTNOTIFY
            | WM_SETTEXT
            | WM_GETTEXT
            | WM_GETTEXTLENGTH
            | WM_STYLECHANGING
            | WM_STYLECHANGED
            | WM_CLOSE
            | WM_TIMER
            | WM_KEYDOWN
            | WM_KEYUP
            | WM_CHAR
            | WM_SYSKEYDOWN
            | WM_SYSKEYUP
            | WM_SYSCHAR
            | WM_DEADCHAR
            | WM_SYSCOMMAND
            | WM_CONTEXTMENU
            | WM_COMMAND
            | WM_ENABLE
            | WM_CHILDACTIVATE
            | WM_QUIT
            | WM_NULL
    ) || (WM_USER..0xC000).contains(&msg)
}

/// SetLastError.
fn set_error(code: u32) {
    // SAFETY: sets this thread's last-error value.
    unsafe { SetLastError(code) }
}

/// GetLastError.
fn last_error() -> u32 {
    // SAFETY: reads this thread's last-error value.
    unsafe { GetLastError() }
}

const CTX_UNAWARE: isize = -1;
const CTX_SYSTEM: isize = -2;
const CTX_PM: isize = -3;
const CTX_PMV2: isize = -4;
const CTX_GDI: isize = -5;
const SPI_GETNONCLIENTMETRICS: u32 = 0x29;
const SPI_GETICONMETRICS: u32 = 0x2D;
const SPI_GETICONTITLELOGFONT: u32 = 0x1F;
/// The label of a probe's mode letter: u none, U unaware, s system, p per-monitor v1, v per-monitor v2,
/// g unaware GDI scaled (a letter in capitals M: the awareness comes from the manifest).
fn mode_label(m: u8) -> &'static str {
    match m {
        b'u' => "no call",
        b'U' => "unaware",
        b's' => "system",
        b'p' => "per-monitor",
        b'v' => "per-monitor-v2",
        b'g' => "unaware-gdi-scaled",
        _ => "manifest",
    }
}

/// Puts the process into the probe's mode with SetProcessDpiAwarenessContext (modes u and M: nothing).
fn enter(t: &mut Test, m: u8) {
    let ctx = match m {
        b'U' => CTX_UNAWARE,
        b's' => CTX_SYSTEM,
        b'p' => CTX_PM,
        b'v' => CTX_PMV2,
        b'g' => CTX_GDI,
        _ => 0,
    };
    if ctx == 0 {
        say!(t, "mode {}: no SetProcessDpiAwarenessContext call", mode_label(m));
        return;
    }
    set_error(0);
    // SAFETY: plain call.
    let r = unsafe { SetProcessDpiAwarenessContext(ctx) };
    say!(t, "mode {}: SetProcessDpiAwarenessContext -> {} error {}", mode_label(m), r != 0, last_error());
}

fn i32_at(b: &[u8], o: usize) -> i32 {
    i32::from_le_bytes([b[o], b[o + 1], b[o + 2], b[o + 3]])
}

/// The non-client metrics, icon metrics and fonts the system parameters give.
fn sec_spi(t: &mut Test) {
    // SAFETY: buffers of the documented sizes with cbSize set.
    unsafe {
        let mut ncm = [0u8; 504];
        ncm[0] = 504u32 as u8;
        ncm[1] = (504u32 >> 8) as u8;
        let r = SystemParametersInfoW(SPI_GETNONCLIENTMETRICS, 504, ncm.as_mut_ptr() as *mut c_void, 0);
        say!(t, "SPI_GETNONCLIENTMETRICS -> {} border {} scroll {}x{} caption {}x{} smcaption {}x{} menu {}x{} padded {}", r != 0, i32_at(&ncm, 4), i32_at(&ncm, 8), i32_at(&ncm, 12), i32_at(&ncm, 16), i32_at(&ncm, 20), i32_at(&ncm, 116), i32_at(&ncm, 120), i32_at(&ncm, 216), i32_at(&ncm, 220), i32_at(&ncm, 500));
        say!(t, "  font heights caption {} smcaption {} menu {} status {} message {}", i32_at(&ncm, 24), i32_at(&ncm, 124), i32_at(&ncm, 224), i32_at(&ncm, 316), i32_at(&ncm, 408));
        for dpi in [96u32, 144, 192, 288] {
            let mut n2 = [0u8; 504];
            n2[0] = (504u32 & 0xFF) as u8;
            n2[1] = (504u32 >> 8) as u8;
            let r = SystemParametersInfoForDpi(SPI_GETNONCLIENTMETRICS, 504, n2.as_mut_ptr() as *mut c_void, 0, dpi);
            say!(t, "SystemParametersInfoForDpi({dpi}) -> {} border {} scroll {}x{} caption {}x{} smcaption {}x{} menu {}x{} padded {} caption font {} menu font {}", r != 0, i32_at(&n2, 4), i32_at(&n2, 8), i32_at(&n2, 12), i32_at(&n2, 16), i32_at(&n2, 20), i32_at(&n2, 116), i32_at(&n2, 120), i32_at(&n2, 216), i32_at(&n2, 220), i32_at(&n2, 500), i32_at(&n2, 24), i32_at(&n2, 224));
        }
        let mut im = [0u8; 108];
        im[0] = 108;
        let r = SystemParametersInfoW(SPI_GETICONMETRICS, 108, im.as_mut_ptr() as *mut c_void, 0);
        say!(t, "SPI_GETICONMETRICS -> {} horz {} vert {} wrap {} font height {}", r != 0, i32_at(&im, 4), i32_at(&im, 8), i32_at(&im, 12), i32_at(&im, 16));
        let mut lf = [0u8; 92];
        let r = SystemParametersInfoW(SPI_GETICONTITLELOGFONT, 92, lf.as_mut_ptr() as *mut c_void, 0);
        say!(t, "SPI_GETICONTITLELOGFONT -> {} height {}", r != 0, i32_at(&lf, 0));
        let mut ncm = [0u8; 504];
        ncm[0] = (504u32 & 0xFF) as u8;
        ncm[1] = (504u32 >> 8) as u8;
        let r = SystemParametersInfoForDpi(SPI_GETICONMETRICS, 108, im.as_mut_ptr() as *mut c_void, 0, 192);
        say!(t, "SystemParametersInfoForDpi(SPI_GETICONMETRICS, 192) -> {} horz {} vert {} font height {}", r != 0, i32_at(&im, 4), i32_at(&im, 8), i32_at(&im, 16));
        let r = SystemParametersInfoForDpi(SPI_GETICONTITLELOGFONT, 92, lf.as_mut_ptr() as *mut c_void, 0, 192);
        say!(t, "SystemParametersInfoForDpi(SPI_GETICONTITLELOGFONT, 192) -> {} height {}", r != 0, i32_at(&lf, 0));
    }
}

#[link(name = "kernel32", kind = "raw-dylib")]
unsafe extern "system" {
    fn GetLastError() -> u32;
    fn SetLastError(code: u32);
}

#[link(name = "user32", kind = "raw-dylib")]
unsafe extern "system" {
    fn SetProcessDpiAwarenessContext(ctx: isize) -> BOOL;
    fn SystemParametersInfoW(action: u32, param: u32, pv: *mut c_void, ini: u32) -> BOOL;
    fn SystemParametersInfoForDpi(action: u32, param: u32, pv: *mut c_void, ini: u32, dpi: u32) -> BOOL;
}

const MODE: u8 = b'u';

poly_wtest::main!(test);

fn test(t: &mut Test) {
    enter(t, MODE);
    sec_spi(t);
}

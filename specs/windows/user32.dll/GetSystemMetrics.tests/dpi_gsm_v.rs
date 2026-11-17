//! GetSystemMetrics next to GetSystemMetricsForDpi, on a display scaled to 200 percent; the program
//! is a per-monitor aware v2 process (SetProcessDpiAwarenessContext(PER_MONITOR_AWARE_V2)).
#![no_std]
#![no_main]
#![allow(dead_code, non_camel_case_types, clippy::all)]

// The declarations below are from Microsoft Learn (prototypes) and the
// public SDK headers winuser.h, wingdi.h, windef.h, winbase.h and
// winerror.h (constants, layouts); the helpers are this test's own.

use core::fmt::Write as _;
use poly_wtest::*;

type BOOL = i32;
const SM_CXSCREEN: i32 = 0;
const SM_CYSCREEN: i32 = 1;
const SM_CYCAPTION: i32 = 4;
const SM_CXBORDER: i32 = 5;
const SM_CYBORDER: i32 = 6;
const SM_CXFRAME: i32 = 32;
const SM_CYFRAME: i32 = 33;
const SM_CXMIN: i32 = 28;
const SM_CYMIN: i32 = 29;
const SM_CXMINTRACK: i32 = 34;
const SM_CYMINTRACK: i32 = 35;
const SM_CXPADDEDBORDER: i32 = 92;
const SM_CXEDGE: i32 = 45;
const SM_CYMENU: i32 = 15;
const SM_CXVSCROLL: i32 = 2;
const SM_CYHSCROLL: i32 = 3;
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

const SM_CXICON: i32 = 11;
const SM_CYICON: i32 = 12;
const SM_CXSMICON: i32 = 49;
const SM_CYSMICON: i32 = 50;
const SM_CXDOUBLECLK: i32 = 36;
const SM_CXDRAG: i32 = 68;
const SM_CXHTHUMB: i32 = 10;
const SM_CYVTHUMB: i32 = 9;
const SM_CXSIZE: i32 = 30;
const SM_CYSIZE: i32 = 31;
const SM_CXSMSIZE: i32 = 52;
const SM_CYSMCAPTION: i32 = 51;
const SM_CXMENUSIZE: i32 = 54;
const SM_CXMENUCHECK: i32 = 71;
const SM_CXICONSPACING: i32 = 38;
const CTX_UNAWARE: isize = -1;
const CTX_SYSTEM: isize = -2;
const CTX_PM: isize = -3;
const CTX_PMV2: isize = -4;
const CTX_GDI: isize = -5;
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

/// GetSystemMetrics against GetSystemMetricsForDpi.
fn sec_gsm(t: &mut Test) {
    // SAFETY: plain calls.
    unsafe {
        let dpi = GetDpiForSystem();
        say!(t, "GetDpiForSystem {} SM_CXSCREEN x SM_CYSCREEN {}x{}", dpi, GetSystemMetrics(SM_CXSCREEN), GetSystemMetrics(SM_CYSCREEN));
        let items = [
            ("CXVSCROLL", SM_CXVSCROLL),
            ("CYHSCROLL", SM_CYHSCROLL),
            ("CYCAPTION", SM_CYCAPTION),
            ("CXBORDER", SM_CXBORDER),
            ("CYBORDER", SM_CYBORDER),
            ("CYVTHUMB", SM_CYVTHUMB),
            ("CXHTHUMB", SM_CXHTHUMB),
            ("CXICON", SM_CXICON),
            ("CYICON", SM_CYICON),
            ("CYMENU", SM_CYMENU),
            ("CXMIN", SM_CXMIN),
            ("CYMIN", SM_CYMIN),
            ("CXSIZE", SM_CXSIZE),
            ("CYSIZE", SM_CYSIZE),
            ("CXFRAME", SM_CXFRAME),
            ("CYFRAME", SM_CYFRAME),
            ("CXMINTRACK", SM_CXMINTRACK),
            ("CYMINTRACK", SM_CYMINTRACK),
            ("CXDOUBLECLK", SM_CXDOUBLECLK),
            ("CXICONSPACING", SM_CXICONSPACING),
            ("CXEDGE", SM_CXEDGE),
            ("CXSMICON", SM_CXSMICON),
            ("CYSMICON", SM_CYSMICON),
            ("CYSMCAPTION", SM_CYSMCAPTION),
            ("CXSMSIZE", SM_CXSMSIZE),
            ("CXMENUSIZE", SM_CXMENUSIZE),
            ("CXDRAG", SM_CXDRAG),
            ("CXMENUCHECK", SM_CXMENUCHECK),
            ("CXPADDEDBORDER", SM_CXPADDEDBORDER),
        ];
        for (n, i) in items {
            let a = GetSystemMetrics(i);
            let f = GetSystemMetricsForDpi(i, dpi);
            say!(t, "{n}: {} for the system dpi {} for 96 {} for 192 {}", a, f, GetSystemMetricsForDpi(i, 96), GetSystemMetricsForDpi(i, 192));
        }
    }
}

#[link(name = "kernel32", kind = "raw-dylib")]
unsafe extern "system" {
    fn GetLastError() -> u32;
    fn SetLastError(code: u32);
}

#[link(name = "user32", kind = "raw-dylib")]
unsafe extern "system" {
    fn GetSystemMetrics(index: i32) -> i32;
    fn GetDpiForSystem() -> u32;
    fn GetSystemMetricsForDpi(index: i32, dpi: u32) -> i32;
    fn SetProcessDpiAwarenessContext(ctx: isize) -> BOOL;
}

const MODE: u8 = b'v';

poly_wtest::main!(test);

fn test(t: &mut Test) {
    enter(t, MODE);
    sec_gsm(t);
}

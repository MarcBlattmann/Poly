//! QueryDisplayConfig(QDC_ONLY_ACTIVE_PATHS) and the same with
//! QDC_VIRTUAL_MODE_AWARE: the shape of the paths and modes it returns, which
//! does not depend on the monitor: how many modes there are for a path, which
//! mode index each path points at, the status flags, the identity rotation and
//! scaling, the pixel format, how the source and target modes relate to the
//! primary monitor's size (GetSystemMetrics), the refresh rate of the path and
//! the vertical sync of the target mode, the desktop image mode of the
//! virtual-mode-aware form and the way the indexes are packed. Machine values
//! (the monitor id, the pixel clock) are never printed.
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

/// An all-zero value of a C struct.
fn zeroed<T>() -> T {
    // SAFETY: only used for the C structs above, for which zero is valid.
    unsafe { core::mem::zeroed() }
}

/// winnt.h
#[repr(C)]
#[derive(Clone, Copy, Default, PartialEq, Eq)]
struct LUID {
    low: u32,
    high: i32,
}

/// wingdi.h
#[repr(C)]
#[derive(Clone, Copy, Default)]
struct DISPLAYCONFIG_RATIONAL {
    numerator: u32,
    denominator: u32,
}

/// wingdi.h
#[repr(C)]
#[derive(Clone, Copy, Default)]
struct DISPLAYCONFIG_PATH_SOURCE_INFO {
    adapter_id: LUID,
    id: u32,
    mode_info_idx: u32,
    status_flags: u32,
}

/// wingdi.h
#[repr(C)]
#[derive(Clone, Copy, Default)]
struct DISPLAYCONFIG_PATH_TARGET_INFO {
    adapter_id: LUID,
    id: u32,
    mode_info_idx: u32,
    output_technology: u32,
    rotation: u32,
    scaling: u32,
    refresh_rate: DISPLAYCONFIG_RATIONAL,
    scan_line_ordering: u32,
    target_available: BOOL,
    status_flags: u32,
}

/// wingdi.h
#[repr(C)]
#[derive(Clone, Copy, Default)]
struct DISPLAYCONFIG_PATH_INFO {
    source_info: DISPLAYCONFIG_PATH_SOURCE_INFO,
    target_info: DISPLAYCONFIG_PATH_TARGET_INFO,
    flags: u32,
}

/// wingdi.h (the union of DISPLAYCONFIG_MODE_INFO is as large as the target mode: 48 bytes)
#[repr(C)]
#[derive(Clone, Copy)]
struct DISPLAYCONFIG_MODE_INFO {
    info_type: u32,
    id: u32,
    adapter_id: LUID,
    u: [u32; 12],
}

const QDC_ONLY_ACTIVE_PATHS: u32 = 0x2;
const QDC_VIRTUAL_MODE_AWARE: u32 = 0x10;
/// The active paths and modes of the machine (the first `p` and `m` entries are valid).
fn qdc(flags: u32, paths: &mut [DISPLAYCONFIG_PATH_INFO; 8], modes: &mut [DISPLAYCONFIG_MODE_INFO; 16]) -> (i32, u32, u32) {
    let mut p = 8u32;
    let mut m = 16u32;
    // SAFETY: the counts are the lengths of the arrays; no topology is asked for.
    let r = unsafe { QueryDisplayConfig(flags, &mut p, paths.as_mut_ptr(), &mut m, modes.as_mut_ptr(), core::ptr::null_mut()) };
    (r, p, m)
}

#[link(name = "user32", kind = "raw-dylib")]
unsafe extern "system" {
    fn GetSystemMetrics(index: i32) -> i32;
    fn QueryDisplayConfig(
        flags: u32,
        num_paths: *mut u32,
        paths: *mut DISPLAYCONFIG_PATH_INFO,
        num_modes: *mut u32,
        modes: *mut DISPLAYCONFIG_MODE_INFO,
        topology: *mut u32,
    ) -> i32;
}

poly_wtest::main!(test);

fn test(t: &mut Test) {
    let mut paths = [DISPLAYCONFIG_PATH_INFO::default(); 8];
    // SAFETY: plain data.
    let mut modes = unsafe { [core::mem::zeroed::<DISPLAYCONFIG_MODE_INFO>(); 16] };
    let (sw, sh) = (unsafe { GetSystemMetrics(SM_CXSCREEN) } as u32, unsafe { GetSystemMetrics(SM_CYSCREEN) } as u32);
    for vma in [false, true] {
        let flags = QDC_ONLY_ACTIVE_PATHS | if vma { QDC_VIRTUAL_MODE_AWARE } else { 0 };
        let tag = if vma { "VIRTUAL_MODE_AWARE" } else { "plain" };
        let (r, p, m) = qdc(flags, &mut paths, &mut modes);
        say!(t, "[{tag}] returns {r}, paths {}, modes per path {}", if p >= 1 { "1 or more" } else { "none" }, if p > 0 { m / p } else { 0 });
        // clean the packed halves for the lookups below
        for i in 0..p as usize {
            let pi = paths[i];
            let (src_idx, tgt_idx, desk_idx) = if vma {
                (pi.source_info.mode_info_idx >> 16, pi.target_info.mode_info_idx >> 16, pi.target_info.mode_info_idx & 0xFFFF)
            } else {
                (pi.source_info.mode_info_idx, pi.target_info.mode_info_idx, 0)
            };
            let clone_group = pi.source_info.mode_info_idx & 0xFFFF;
            say!(t, "[{tag}] path {i}: flags {:#x}, source status {:#x}, target status {:#x}, target available {}", pi.flags, pi.source_info.status_flags, pi.target_info.status_flags, pi.target_info.target_available);
            say!(t, "[{tag}] path {i}: rotation {}, scaling {}, scan line ordering {}, adapter of source and target equal {}", pi.target_info.rotation, pi.target_info.scaling, pi.target_info.scan_line_ordering, pi.source_info.adapter_id == pi.target_info.adapter_id);
            if vma {
                say!(t, "[{tag}] path {i}: clone group {:#x}, source mode index in the high half {}, target index in the high half {}, desktop image index in the low half {}", clone_group, src_idx < m, tgt_idx < m, desk_idx < m);
            }
            let (s, tg) = (modes[src_idx as usize], modes[tgt_idx as usize]);
            say!(t, "[{tag}] path {i}: source mode is type {} with id equal to the source id {} and the adapter of the path {}", s.info_type, s.id == pi.source_info.id, s.adapter_id == pi.source_info.adapter_id);
            say!(t, "[{tag}] path {i}: target mode is type {} with id equal to the target id {} and the adapter of the path {}", tg.info_type, tg.id == pi.target_info.id, tg.adapter_id == pi.target_info.adapter_id);
            // source mode: width, height, pixel format, position
            let primary_at_origin = s.u[3] == 0 && s.u[4] == 0;
            say!(t, "[{tag}] path {i}: source mode size is the primary monitor's {} (when at 0,0: {primary_at_origin}), pixel format {}", s.u[0] == sw && s.u[1] == sh, s.u[2]);
            // target mode (DISPLAYCONFIG_TARGET_MODE = video signal info): pixel clock u64, hsync, vsync, active, total, standard, scan line ordering
            let pixel_rate = tg.u[0] as u64 | ((tg.u[1] as u64) << 32);
            let (vs_n, vs_d) = (tg.u[4] as u64, tg.u[5] as u64);
            let (act_w, act_h, tot_w, tot_h) = (tg.u[6], tg.u[7], tg.u[8], tg.u[9]);
            say!(t, "[{tag}] path {i}: target active size equals the source mode's {}, total size at least the active size {}, standard {}, scan line ordering {}", act_w == s.u[0] && act_h == s.u[1], tot_w >= act_w && tot_h >= act_h, tg.u[10], tg.u[11]);
            say!(t, "[{tag}] path {i}: the path's refresh rate equals the target mode's vertical sync {}, the rate is between 24 and 500 Hz {}, pixel rate non zero {}", (pi.target_info.refresh_rate.numerator as u64, pi.target_info.refresh_rate.denominator as u64) == (vs_n, vs_d), vs_d != 0 && vs_n / vs_d >= 24 && vs_n / vs_d <= 500, pixel_rate != 0);
            if vma {
                let d = modes[desk_idx as usize];
                say!(t, "[{tag}] path {i}: desktop image mode is type {} with id equal to the target id {}, path source size {} by {} equals the source mode {}, region {},{},{},{} clip {},{},{},{}", d.info_type, d.id == pi.target_info.id, d.u[0] == s.u[0], d.u[1] == s.u[1], d.u[0] == s.u[0] && d.u[1] == s.u[1], d.u[2], d.u[3], d.u[4] == s.u[0], d.u[5] == s.u[1], d.u[6], d.u[7], d.u[8] == s.u[0], d.u[9] == s.u[1]);
            }
        }
        // the three modes are of three different types, in the order target, source, desktop image
        let mut types = Buf::<64>::new();
        for i in 0..m as usize {
            let _ = write!(types, "{} ", modes[i].info_type);
        }
        say!(t, "[{tag}] mode types in order: {}", types.as_str().trim_end());
    }
}

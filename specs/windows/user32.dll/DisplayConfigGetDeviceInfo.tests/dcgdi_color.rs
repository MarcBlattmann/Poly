//! DisplayConfigGetDeviceInfo with DISPLAYCONFIG_DEVICE_INFO_GET_ADVANCED_COLOR_INFO
//! (9), GET_SDR_WHITE_LEVEL (11), GET_SUPPORT_VIRTUAL_RESOLUTION (7) and
//! GET_TARGET_PREFERRED_MODE (3) for the active target, the ones Chrome's
//! display code asks for: which output fields are written (advanced color
//! writes only the low four bits of the value), values on a monitor without
//! HDR (no advanced color, 8 bits per channel, white level 1000 meaning 80
//! nits), the support-virtual-resolution answer, the preferred mode being
//! non empty, and that these answers are identical when asked twice.
#![no_std]
#![no_main]
#![allow(dead_code, non_camel_case_types, clippy::all)]

// The declarations below are from Microsoft Learn (prototypes) and the
// public SDK headers winuser.h, wingdi.h, windef.h, winbase.h and
// winerror.h (constants, layouts); the helpers are this test's own.

use core::fmt::Write as _;
use poly_wtest::*;

type BOOL = i32;
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

/// wingdi.h
#[repr(C)]
#[derive(Clone, Copy, Default)]
struct DISPLAYCONFIG_DEVICE_INFO_HEADER {
    kind: u32,
    size: u32,
    adapter_id: LUID,
    id: u32,
}

/// wingdi.h (DISPLAYCONFIG_TARGET_BASE_TYPE)
#[repr(C)]
#[derive(Clone, Copy)]
struct DISPLAYCONFIG_TARGET_BASE_TYPE {
    header: DISPLAYCONFIG_DEVICE_INFO_HEADER,
    base_output_technology: u32,
}

/// wingdi.h (DISPLAYCONFIG_GET_ADVANCED_COLOR_INFO)
#[repr(C)]
#[derive(Clone, Copy)]
struct DISPLAYCONFIG_GET_ADVANCED_COLOR_INFO {
    header: DISPLAYCONFIG_DEVICE_INFO_HEADER,
    value: u32,
    color_encoding: u32,
    bits_per_color_channel: u32,
}

/// wingdi.h (DISPLAYCONFIG_SDR_WHITE_LEVEL)
#[repr(C)]
#[derive(Clone, Copy)]
struct DISPLAYCONFIG_SDR_WHITE_LEVEL {
    header: DISPLAYCONFIG_DEVICE_INFO_HEADER,
    sdr_white_level: u32,
}

/// wingdi.h (DISPLAYCONFIG_TARGET_PREFERRED_MODE)
#[repr(C)]
#[derive(Clone, Copy)]
struct DISPLAYCONFIG_TARGET_PREFERRED_MODE {
    header: DISPLAYCONFIG_DEVICE_INFO_HEADER,
    width: u32,
    height: u32,
    target_mode: [u64; 6],
}

const QDC_ONLY_ACTIVE_PATHS: u32 = 0x2;
const DISPLAYCONFIG_DEVICE_INFO_GET_TARGET_PREFERRED_MODE: u32 = 3;
const DISPLAYCONFIG_DEVICE_INFO_GET_SUPPORT_VIRTUAL_RESOLUTION: u32 = 7;
const DISPLAYCONFIG_DEVICE_INFO_GET_ADVANCED_COLOR_INFO: u32 = 9;
const DISPLAYCONFIG_DEVICE_INFO_GET_SDR_WHITE_LEVEL: u32 = 11;
/// The active paths and modes of the machine (the first `p` and `m` entries are valid).
fn qdc(flags: u32, paths: &mut [DISPLAYCONFIG_PATH_INFO; 8], modes: &mut [DISPLAYCONFIG_MODE_INFO; 16]) -> (i32, u32, u32) {
    let mut p = 8u32;
    let mut m = 16u32;
    // SAFETY: the counts are the lengths of the arrays; no topology is asked for.
    let r = unsafe { QueryDisplayConfig(flags, &mut p, paths.as_mut_ptr(), &mut m, modes.as_mut_ptr(), core::ptr::null_mut()) };
    (r, p, m)
}

#[link(name = "kernel32", kind = "raw-dylib")]
unsafe extern "system" {
    fn GetLastError() -> u32;
    fn SetLastError(code: u32);
}

#[link(name = "user32", kind = "raw-dylib")]
unsafe extern "system" {
    fn QueryDisplayConfig(
        flags: u32,
        num_paths: *mut u32,
        paths: *mut DISPLAYCONFIG_PATH_INFO,
        num_modes: *mut u32,
        modes: *mut DISPLAYCONFIG_MODE_INFO,
        topology: *mut u32,
    ) -> i32;
    fn DisplayConfigGetDeviceInfo(packet: *mut DISPLAYCONFIG_DEVICE_INFO_HEADER) -> i32;
}

poly_wtest::main!(test);

fn test(t: &mut Test) {
    let mut paths = [DISPLAYCONFIG_PATH_INFO::default(); 8];
    // SAFETY: plain data and our own buffers.
    unsafe {
        let mut modes = [core::mem::zeroed::<DISPLAYCONFIG_MODE_INFO>(); 16];
        qdc(QDC_ONLY_ACTIVE_PATHS, &mut paths, &mut modes);
        let pi = paths[0];

        let mut c: DISPLAYCONFIG_GET_ADVANCED_COLOR_INFO = core::mem::zeroed();
        c.header.kind = DISPLAYCONFIG_DEVICE_INFO_GET_ADVANCED_COLOR_INFO;
        c.header.size = core::mem::size_of::<DISPLAYCONFIG_GET_ADVANCED_COLOR_INFO>() as u32;
        c.header.adapter_id = pi.target_info.adapter_id;
        c.header.id = pi.target_info.id;
        c.value = 0xFFFF_FFFF;
        c.color_encoding = 77;
        c.bits_per_color_channel = 77;
        set_error(0xDEAD);
        let r = DisplayConfigGetDeviceInfo(&mut c.header);
        say!(t, "advanced color: returns {r}, size {}, last error left alone {}", core::mem::size_of::<DISPLAYCONFIG_GET_ADVANCED_COLOR_INFO>(), last_error() == 0xDEAD);
        say!(t, "advanced color: bits 4 and up of the value kept as passed {}, bits 0 to 3 (supported, enabled, wide color enforced, force disabled) written: {:#x}, encoding {} (RGB), bits per channel {}", c.value >> 4 == 0x0FFF_FFFF, c.value & 0xF, c.color_encoding, c.bits_per_color_channel);
        c.header.id = pi.source_info.id;
        say!(t, "advanced color with the source id: returns {}", DisplayConfigGetDeviceInfo(&mut c.header));

        let mut w: DISPLAYCONFIG_SDR_WHITE_LEVEL = core::mem::zeroed();
        w.header.kind = DISPLAYCONFIG_DEVICE_INFO_GET_SDR_WHITE_LEVEL;
        w.header.size = core::mem::size_of::<DISPLAYCONFIG_SDR_WHITE_LEVEL>() as u32;
        w.header.adapter_id = pi.target_info.adapter_id;
        w.header.id = pi.target_info.id;
        w.sdr_white_level = 5;
        let r = DisplayConfigGetDeviceInfo(&mut w.header);
        say!(t, "SDR white level: returns {r}, size {}, level {}", core::mem::size_of::<DISPLAYCONFIG_SDR_WHITE_LEVEL>(), w.sdr_white_level);

        let mut v: DISPLAYCONFIG_TARGET_BASE_TYPE = core::mem::zeroed();
        v.header.kind = DISPLAYCONFIG_DEVICE_INFO_GET_SUPPORT_VIRTUAL_RESOLUTION;
        v.header.size = 24;
        v.header.adapter_id = pi.target_info.adapter_id;
        v.header.id = pi.target_info.id;
        v.base_output_technology = 0x55;
        let r = DisplayConfigGetDeviceInfo(&mut v.header);
        say!(t, "support virtual resolution: returns {r}, field (disabled flag) {}", v.base_output_technology);
        v.header.size = 28;
        say!(t, "support virtual resolution with header size 28: returns {}", DisplayConfigGetDeviceInfo(&mut v.header));

        let mut pm: DISPLAYCONFIG_TARGET_PREFERRED_MODE = core::mem::zeroed();
        pm.header.kind = DISPLAYCONFIG_DEVICE_INFO_GET_TARGET_PREFERRED_MODE;
        pm.header.size = core::mem::size_of::<DISPLAYCONFIG_TARGET_PREFERRED_MODE>() as u32;
        pm.header.adapter_id = pi.target_info.adapter_id;
        pm.header.id = pi.target_info.id;
        let r = DisplayConfigGetDeviceInfo(&mut pm.header);
        say!(t, "target preferred mode: returns {r}, non empty size {}", pm.width != 0 && pm.height != 0);

        // asked twice, the same answers
        let mut c2 = c;
        c2.header.id = pi.target_info.id;
        c.header.id = pi.target_info.id;
        DisplayConfigGetDeviceInfo(&mut c.header);
        DisplayConfigGetDeviceInfo(&mut c2.header);
        say!(t, "advanced color asked twice gives the same answer: {}", c.value == c2.value && c.color_encoding == c2.color_encoding && c.bits_per_color_channel == c2.bits_per_color_channel);
    }
}

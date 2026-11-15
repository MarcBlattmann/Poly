//! DisplayConfigGetDeviceInfo asking for names: DISPLAYCONFIG_DEVICE_INFO_GET_SOURCE_NAME
//! (the GDI device name of the source), GET_TARGET_NAME (the monitor's name
//! and device path), GET_ADAPTER_NAME (the adapter's device path) and
//! GET_TARGET_BASE_TYPE (the connector type), for the active path of
//! QueryDisplayConfig. Prints how the answers are shaped (prefix and suffix of
//! the paths, the format of the GDI name, equality with GetMonitorInfoW and with
//! the output technology of the path), never the machine's own names.
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
/// windef.h
#[repr(C)]
#[derive(Clone, Copy, Default, PartialEq, Eq)]
struct POINT {
    x: i32,
    y: i32,
}

/// windef.h
#[repr(C)]
#[derive(Clone, Copy, Default, PartialEq, Eq)]
struct RECT {
    left: i32,
    top: i32,
    right: i32,
    bottom: i32,
}

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

/// An all-zero value of a C struct.
fn zeroed<T>() -> T {
    // SAFETY: only used for the C structs above, for which zero is valid.
    unsafe { core::mem::zeroed() }
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

type HMONITOR = *mut c_void;
/// winuser.h
#[repr(C)]
struct MONITORINFOEXW {
    cb_size: u32,
    monitor: RECT,
    work: RECT,
    flags: u32,
    device: [u16; 32],
}

const MONITOR_DEFAULTTOPRIMARY: u32 = 1;
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

/// wingdi.h (DISPLAYCONFIG_SOURCE_DEVICE_NAME)
#[repr(C)]
#[derive(Clone, Copy)]
struct DISPLAYCONFIG_SOURCE_DEVICE_NAME {
    header: DISPLAYCONFIG_DEVICE_INFO_HEADER,
    view_gdi_device_name: [u16; 32],
}

/// wingdi.h (DISPLAYCONFIG_TARGET_DEVICE_NAME)
#[repr(C)]
#[derive(Clone, Copy)]
struct DISPLAYCONFIG_TARGET_DEVICE_NAME {
    header: DISPLAYCONFIG_DEVICE_INFO_HEADER,
    flags: u32,
    output_technology: u32,
    edid_manufacture_id: u16,
    edid_product_code_id: u16,
    connector_instance: u32,
    monitor_friendly_device_name: [u16; 64],
    monitor_device_path: [u16; 128],
}

/// wingdi.h (DISPLAYCONFIG_ADAPTER_NAME)
#[repr(C)]
#[derive(Clone, Copy)]
struct DISPLAYCONFIG_ADAPTER_NAME {
    header: DISPLAYCONFIG_DEVICE_INFO_HEADER,
    adapter_device_path: [u16; 128],
}

/// wingdi.h (DISPLAYCONFIG_TARGET_BASE_TYPE)
#[repr(C)]
#[derive(Clone, Copy)]
struct DISPLAYCONFIG_TARGET_BASE_TYPE {
    header: DISPLAYCONFIG_DEVICE_INFO_HEADER,
    base_output_technology: u32,
}

const QDC_ONLY_ACTIVE_PATHS: u32 = 0x2;
const DISPLAYCONFIG_DEVICE_INFO_GET_SOURCE_NAME: u32 = 1;
const DISPLAYCONFIG_DEVICE_INFO_GET_TARGET_NAME: u32 = 2;
const DISPLAYCONFIG_DEVICE_INFO_GET_ADAPTER_NAME: u32 = 4;
const DISPLAYCONFIG_DEVICE_INFO_GET_TARGET_BASE_TYPE: u32 = 6;
fn wide_str(buf: &[u16], out: &mut Buf<300>) {
    let n = buf.iter().position(|&c| c == 0).unwrap_or(buf.len());
    for &c in &buf[..n] {
        let _ = write!(out, "{}", if (32..127).contains(&c) { c as u8 as char } else { '?' });
    }
}

fn w_len(buf: &[u16]) -> usize {
    buf.iter().position(|&c| c == 0).unwrap_or(buf.len())
}

fn w_ascii(buf: &[u16]) -> Buf<300> {
    let mut b = Buf::<300>::new();
    wide_str(buf, &mut b);
    b
}

fn w_starts(buf: &[u16], s: &str) -> bool {
    w_ascii(buf).as_str().starts_with(s)
}

fn w_ends(buf: &[u16], s: &str) -> bool {
    w_ascii(buf).as_str().ends_with(s)
}

fn w_has(buf: &[u16], s: &str) -> bool {
    w_ascii(buf).as_str().contains(s)
}

fn w_eq(buf: &[u16], s: &str) -> bool {
    w_ascii(buf).as_str() == s
}

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
    fn MonitorFromPoint(pt: POINT, flags: u32) -> HMONITOR;
    fn GetMonitorInfoW(monitor: HMONITOR, info: *mut MONITORINFOEXW) -> BOOL;
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
        let (r, p, _) = qdc(QDC_ONLY_ACTIVE_PATHS, &mut paths, &mut modes);
        say!(t, "active paths: returns {r}, at least one {}", p >= 1);
        let pi = paths[0];

        // source name
        let mut s: DISPLAYCONFIG_SOURCE_DEVICE_NAME = core::mem::zeroed();
        s.view_gdi_device_name = [0x2222; 32];
        s.header.kind = DISPLAYCONFIG_DEVICE_INFO_GET_SOURCE_NAME;
        s.header.size = core::mem::size_of::<DISPLAYCONFIG_SOURCE_DEVICE_NAME>() as u32;
        s.header.adapter_id = pi.source_info.adapter_id;
        s.header.id = pi.source_info.id;
        set_error(0xDEAD);
        let r = DisplayConfigGetDeviceInfo(&mut s.header);
        let n = w_len(&s.view_gdi_device_name);
        say!(t, "source name: returns {r}, size {}, header unchanged {}, last error left alone {}", core::mem::size_of::<DISPLAYCONFIG_SOURCE_DEVICE_NAME>(), s.header.kind == 1 && s.header.size == 84 && s.header.id == pi.source_info.id, last_error() == 0xDEAD);
        say!(t, "source name: begins with \\\\.\\DISPLAY {}, then the decimal number {} (source id + 1 {}), nothing else {}, remainder of the buffer zeroed {}", w_starts(&s.view_gdi_device_name, "\\\\.\\DISPLAY"), n > 11, w_eq(&s.view_gdi_device_name, &{
            let mut b = Buf::<40>::new();
            let _ = write!(b, "\\\\.\\DISPLAY{}", pi.source_info.id + 1);
            b
        }.as_str()), n < 32, s.view_gdi_device_name[n..].iter().all(|&c| c == 0));
        // equal to the device name of the monitor at the source's position
        let mut mi: MONITORINFOEXW = core::mem::zeroed();
        mi.cb_size = core::mem::size_of::<MONITORINFOEXW>() as u32;
        let hm = MonitorFromPoint(POINT { x: 0, y: 0 }, MONITOR_DEFAULTTOPRIMARY);
        GetMonitorInfoW(hm, &mut mi);
        say!(t, "source name equals the device name of the primary monitor from GetMonitorInfoW: {}", mi.device[..w_len(&mi.device)] == s.view_gdi_device_name[..n]);

        // target name
        let mut tn: DISPLAYCONFIG_TARGET_DEVICE_NAME = core::mem::zeroed();
        tn.header.kind = DISPLAYCONFIG_DEVICE_INFO_GET_TARGET_NAME;
        tn.header.size = core::mem::size_of::<DISPLAYCONFIG_TARGET_DEVICE_NAME>() as u32;
        tn.header.adapter_id = pi.target_info.adapter_id;
        tn.header.id = pi.target_info.id;
        set_error(0xDEAD);
        let r = DisplayConfigGetDeviceInfo(&mut tn.header);
        say!(t, "target name: returns {r}, size {}, last error left alone {}", core::mem::size_of::<DISPLAYCONFIG_TARGET_DEVICE_NAME>(), last_error() == 0xDEAD);
        say!(t, "target name: output technology equals the path's {}, friendly name shorter than 64 characters {}, flags use only the three defined bits {}", tn.output_technology == pi.target_info.output_technology, w_len(&tn.monitor_friendly_device_name) < 64, tn.flags & !7 == 0);
        let path = &tn.monitor_device_path;
        say!(t, "target name: device path begins with \\\\?\\DISPLAY# {}, ends with #{{e6f07b5f-ee97-4a90-b076-33f57bf4eaa7}} {}, holds UID<target id> {}", w_starts(path, "\\\\?\\DISPLAY#"), w_ends(path, "#{e6f07b5f-ee97-4a90-b076-33f57bf4eaa7}"), w_has(path, &{
            let mut b = Buf::<40>::new();
            let _ = write!(b, "UID{}#", pi.target_info.id);
            b
        }.as_str()));
        // when the monitor has no EDID name the friendly name is empty and the 'forced' bit is set
        say!(t, "target name: an empty friendly name goes with flag bit 1 (friendlyNameForced) set {}", w_len(&tn.monitor_friendly_device_name) != 0 || tn.flags & 2 != 0);
        say!(t, "target name: edid ids valid (flag bit 2) agrees with non zero edid ids {}", (tn.flags & 4 != 0) == (tn.edid_manufacture_id != 0 || tn.edid_product_code_id != 0));

        // adapter name
        let mut an: DISPLAYCONFIG_ADAPTER_NAME = core::mem::zeroed();
        an.header.kind = DISPLAYCONFIG_DEVICE_INFO_GET_ADAPTER_NAME;
        an.header.size = core::mem::size_of::<DISPLAYCONFIG_ADAPTER_NAME>() as u32;
        an.header.adapter_id = pi.target_info.adapter_id;
        an.header.id = 0;
        set_error(0xDEAD);
        let r = DisplayConfigGetDeviceInfo(&mut an.header);
        say!(t, "adapter name: returns {r}, size {}, device path begins with \\\\?\\ {}, ends with #{{5b45201d-f2f2-4f3b-85bb-30ff1f953599}} {}", core::mem::size_of::<DISPLAYCONFIG_ADAPTER_NAME>(), w_starts(&an.adapter_device_path, "\\\\?\\"), w_ends(&an.adapter_device_path, "#{5b45201d-f2f2-4f3b-85bb-30ff1f953599}"));
        // the id of an adapter request is ignored
        let mut an2 = an;
        an2.adapter_device_path = [0; 128];
        an2.header.id = 99;
        let r = DisplayConfigGetDeviceInfo(&mut an2.header);
        say!(t, "adapter name with another id: returns {r}, the same path {}", an2.adapter_device_path == an.adapter_device_path);

        // target base type
        let mut bt: DISPLAYCONFIG_TARGET_BASE_TYPE = core::mem::zeroed();
        bt.header.kind = DISPLAYCONFIG_DEVICE_INFO_GET_TARGET_BASE_TYPE;
        bt.header.size = core::mem::size_of::<DISPLAYCONFIG_TARGET_BASE_TYPE>() as u32;
        bt.header.adapter_id = pi.target_info.adapter_id;
        bt.header.id = pi.target_info.id;
        let r = DisplayConfigGetDeviceInfo(&mut bt.header);
        say!(t, "target base type: returns {r}, size {}, equals the path's output technology {}", core::mem::size_of::<DISPLAYCONFIG_TARGET_BASE_TYPE>(), bt.base_output_technology == pi.target_info.output_technology);
        // output technology values are the documented ones
        let ot = pi.target_info.output_technology as i32;
        say!(t, "the output technology is one of the documented values (-1, 0 to 16, 0x80000000): {}", ot == -1 || (0..=16).contains(&ot) || ot as u32 == 0x8000_0000);
    }
}

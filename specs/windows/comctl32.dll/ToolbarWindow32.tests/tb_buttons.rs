//! ToolbarWindow32 (comctl32 5.82): creation (CCS_TOP is added to the style),
//! TB_BUTTONSTRUCTSIZE, TB_ADDBITMAP with a bitmap the test makes (the index
//! of the first new image), TB_ADDBUTTONS with buttons and separators,
//! TB_BUTTONCOUNT, TB_GETBUTTON, TB_COMMANDTOINDEX, TB_INSERTBUTTON,
//! TB_DELETEBUTTON, the rectangle of every button (width of a button and of
//! a separator, row height), TB_GETBUTTONSIZE / SETBUTTONSIZE / SETBITMAPSIZE,
//! TB_AUTOSIZE (the toolbar's own size after it) and the image list calls.
#![no_std]
#![no_main]
#![allow(dead_code, non_camel_case_types, clippy::all)]

// The declarations below are from Microsoft Learn (prototypes) and the
// public SDK headers winuser.h, wingdi.h, windef.h, winbase.h and
// winerror.h (constants, layouts); the helpers are this test's own.

use core::cell::UnsafeCell;
use core::ffi::c_void;
use core::fmt::Write as _;
use poly_wtest::*;

type HWND = *mut c_void;
type HINSTANCE = *mut c_void;
type HMENU = *mut c_void;
type HBRUSH = *mut c_void;
type HDC = *mut c_void;
type HBITMAP = *mut c_void;
type WPARAM = usize;
type LPARAM = isize;
type LRESULT = isize;
type BOOL = i32;
type WNDPROC = Option<unsafe extern "system" fn(HWND, u32, WPARAM, LPARAM) -> LRESULT>;
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

/// windef.h
#[repr(C)]
#[derive(Clone, Copy, Default)]
struct SIZE {
    cx: i32,
    cy: i32,
}

/// winuser.h
#[repr(C)]
struct WNDCLASSEXW {
    cb_size: u32,
    style: u32,
    wnd_proc: WNDPROC,
    cls_extra: i32,
    wnd_extra: i32,
    instance: HINSTANCE,
    icon: *mut c_void,
    cursor: *mut c_void,
    background: HBRUSH,
    menu_name: *const u16,
    class_name: *const u16,
    icon_sm: *mut c_void,
}

/// winuser.h
#[repr(C)]
#[derive(Clone, Copy)]
struct MSG {
    hwnd: HWND,
    message: u32,
    wparam: WPARAM,
    lparam: LPARAM,
    time: u32,
    pt: POINT,
    private: u32,
}

const WM_COMMAND: u32 = 0x0111;
const WS_OVERLAPPED: u32 = 0x0000_0000;
const WS_CHILD: u32 = 0x4000_0000;
const WS_VISIBLE: u32 = 0x1000_0000;
const WS_CAPTION: u32 = 0x00C0_0000;
const GWL_STYLE: i32 = -16;
const GWLP_USERDATA: i32 = -21;
const PM_REMOVE: u32 = 0x0001;
/// A global the window procedures and the test share (one thread at a
/// time unless a test says otherwise).
struct Global<T>(UnsafeCell<T>);

unsafe impl<T> Sync for Global<T> {}
impl<T> Global<T> {
    const fn new(v: T) -> Self {
        Global(UnsafeCell::new(v))
    }
    #[allow(clippy::mut_from_ref)]
    fn get(&self) -> &mut T {
        // SAFETY: see the Sync impl: no two live references at once.
        unsafe { &mut *self.0.get() }
    }
}

/// The message log: lines appended by window procedures, printed by the test.
struct Log {
    buf: [u8; 32768],
    len: usize,
    on: bool,
}

static LOG: Global<Log> = Global::new(Log { buf: [0; 32768], len: 0, on: true });
impl core::fmt::Write for Log {
    fn write_str(&mut self, s: &str) -> core::fmt::Result {
        for &b in s.as_bytes() {
            if self.len < self.buf.len() {
                self.buf[self.len] = b;
                self.len += 1;
            }
        }
        Ok(())
    }
}

/// Append one line to the log (when logging is on).
fn logf(args: core::fmt::Arguments) {
    let log = LOG.get();
    if log.on {
        let _ = log.write_fmt(args);
        let _ = log.write_str("\n");
    }
}

macro_rules! logln {
    ($($a:tt)*) => { logf(format_args!($($a)*)) };
}

/// Print every logged line (prefixed) and empty the log.
fn flush_log(t: &mut Test, prefix: &str) {
    let log = LOG.get();
    let mut start = 0;
    for i in 0..log.len {
        if log.buf[i] == b'\n' {
            let line = core::str::from_utf8(&log.buf[start..i]).unwrap_or("?");
            let mut b = Buf::<300>::new();
            let _ = write!(b, "{prefix}{line}");
            t.line(b.as_str());
            start = i + 1;
        }
    }
    log.len = 0;
}

fn log_clear() {
    LOG.get().len = 0;
}

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

fn lo(v: isize) -> i32 {
    (v as u32 & 0xFFFF) as u16 as i16 as i32
}

fn hi(v: isize) -> i32 {
    ((v as u32 >> 16) & 0xFFFF) as u16 as i16 as i32
}

fn makelparam(lo: i32, hi: i32) -> LPARAM {
    ((lo as u32 & 0xFFFF) | ((hi as u32 & 0xFFFF) << 16)) as i32 as isize
}

/// This executable's instance handle.
fn instance() -> HINSTANCE {
    // SAFETY: NULL names the executable.
    unsafe { GetModuleHandleW(core::ptr::null()) }
}

/// Register a class with `proc_` (and no menu, icon or cursor).
fn register(name: &[u16], proc_: WNDPROC, style: u32, wnd_extra: i32, brush: HBRUSH) -> u16 {
    let wc = WNDCLASSEXW {
        cb_size: core::mem::size_of::<WNDCLASSEXW>() as u32,
        style,
        wnd_proc: proc_,
        cls_extra: 0,
        wnd_extra,
        instance: instance(),
        icon: core::ptr::null_mut(),
        cursor: core::ptr::null_mut(),
        background: brush,
        menu_name: core::ptr::null(),
        class_name: name.as_ptr(),
        icon_sm: core::ptr::null_mut(),
    };
    // SAFETY: a filled WNDCLASSEXW; the name is NUL-terminated and static.
    unsafe { RegisterClassExW(&wc) }
}

/// CreateWindowExW with this executable's instance.
fn create(
    ex: u32,
    class: &[u16],
    name: &[u16],
    style: u32,
    x: i32,
    y: i32,
    w: i32,
    h: i32,
    parent: HWND,
    menu: usize,
) -> HWND {
    // SAFETY: NUL-terminated names; the parent is a window or NULL.
    unsafe {
        CreateWindowExW(
            ex,
            class.as_ptr(),
            if name.is_empty() { core::ptr::null() } else { name.as_ptr() },
            style,
            x,
            y,
            w,
            h,
            parent,
            menu as HMENU,
            instance(),
            core::ptr::null_mut(),
        )
    }
}

fn rect_of(hwnd: HWND) -> RECT {
    let mut r = RECT::default();
    // SAFETY: r is writable.
    unsafe { GetWindowRect(hwnd, &mut r) };
    r
}

fn client_of(hwnd: HWND) -> RECT {
    let mut r = RECT::default();
    // SAFETY: r is writable.
    unsafe { GetClientRect(hwnd, &mut r) };
    r
}

/// Dispatch every waiting message.
fn pump() -> u32 {
    let mut msg: MSG = zeroed();
    let mut n = 0;
    // SAFETY: msg is writable; it is the message just removed.
    while unsafe { PeekMessageW(&mut msg, core::ptr::null_mut(), 0, 0, PM_REMOVE) } != 0 {
        unsafe {
            TranslateMessage(&msg);
            DispatchMessageW(&msg);
        }
        n += 1;
    }
    n
}

/// An all-zero value of a C struct.
fn zeroed<T>() -> T {
    // SAFETY: only used for the C structs above, for which zero is valid.
    unsafe { core::mem::zeroed() }
}

/// A NUL-terminated UTF-16 string from an ASCII literal, at compile time.
macro_rules! wz {
    ($s:literal) => {{
        const S: &str = concat!($s, "\0");
        const W: [u16; S.len()] = ascii_utf16::<{ S.len() }>(S);
        &W
    }};
}

/// The UTF-16 text in `buf` up to its first NUL, as ASCII (others as '?').
fn ascii_of(buf: &[u16], out: &mut Buf<300>) {
    for &c in buf {
        if c == 0 {
            break;
        }
        let _ = out.write_char(if (0x20..0x7f).contains(&c) { c as u8 as char } else { '?' });
    }
}

const WM_PRINT: u32 = 0x0317;
/// How a window handle in a message is printed: "null", the tag letter of
/// a window of this process (GWLP_USERDATA), "ours" (this process, no tag),
/// "other" (another process: the desktop's windows, the shell) or
/// "invalid".
struct W(HWND);

impl core::fmt::Display for W {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        if self.0.is_null() {
            return f.write_str("null");
        }
        let mut pid = 0u32;
        // SAFETY: plain queries; pid is writable.
        let (tid, me) = unsafe { (GetWindowThreadProcessId(self.0, &mut pid), GetCurrentProcessId()) };
        if tid == 0 {
            return f.write_str("invalid");
        }
        if pid != me {
            return f.write_str("other");
        }
        // SAFETY: a window of this process.
        let tag = unsafe { GetWindowLongPtrW(self.0, GWLP_USERDATA) };
        if (0x41..=0x5A).contains(&tag) || (0x61..=0x7A).contains(&tag) {
            f.write_char(tag as u8 as char)
        } else {
            f.write_str("ours")
        }
    }
}

type HIMAGELIST = *mut c_void;
const WM_NOTIFY: u32 = 0x004E;
/// commctrl.h
#[repr(C)]
#[derive(Clone, Copy)]
struct INITCOMMONCONTROLSEX {
    dw_size: u32,
    dw_icc: u32,
}

/// commctrl.h
#[repr(C)]
#[derive(Clone, Copy)]
struct NMHDR {
    hwnd_from: HWND,
    id_from: usize,
    code: u32,
}

const CCS_TOP: u32 = 1;
const CCS_BOTTOM: u32 = 3;

#[link(name = "kernel32", kind = "raw-dylib")]
unsafe extern "system" {
    fn GetModuleHandleW(name: *const u16) -> HINSTANCE;
    fn GetCurrentProcessId() -> u32;
}

#[link(name = "user32", kind = "raw-dylib")]
unsafe extern "system" {
    fn RegisterClassExW(class: *const WNDCLASSEXW) -> u16;
    fn GetClassNameW(hwnd: HWND, buf: *mut u16, max: i32) -> i32;
    fn CreateWindowExW(
        ex_style: u32,
        class: *const u16,
        name: *const u16,
        style: u32,
        x: i32,
        y: i32,
        w: i32,
        h: i32,
        parent: HWND,
        menu: HMENU,
        instance: HINSTANCE,
        param: *mut c_void,
    ) -> HWND;
    fn DestroyWindow(hwnd: HWND) -> BOOL;
    fn GetWindowLongPtrW(hwnd: HWND, index: i32) -> isize;
    fn GetWindowLongW(hwnd: HWND, index: i32) -> i32;
    fn GetWindowRect(hwnd: HWND, rect: *mut RECT) -> BOOL;
    fn GetClientRect(hwnd: HWND, rect: *mut RECT) -> BOOL;
    fn AdjustWindowRectEx(rect: *mut RECT, style: u32, menu: BOOL, ex_style: u32) -> BOOL;
    fn GetWindowThreadProcessId(hwnd: HWND, pid: *mut u32) -> u32;
    fn PeekMessageW(msg: *mut MSG, hwnd: HWND, min: u32, max: u32, remove: u32) -> BOOL;
    fn TranslateMessage(msg: *const MSG) -> BOOL;
    fn DispatchMessageW(msg: *const MSG) -> LRESULT;
    fn DefWindowProcW(hwnd: HWND, msg: u32, w: WPARAM, l: LPARAM) -> LRESULT;
    fn SendMessageW(hwnd: HWND, msg: u32, w: WPARAM, l: LPARAM) -> LRESULT;
    fn GetSystemMetrics(index: i32) -> i32;
}

#[link(name = "gdi32", kind = "raw-dylib")]
unsafe extern "system" {
    fn CreateBitmap(w: i32, h: i32, planes: u32, bpp: u32, bits: *const c_void) -> HBITMAP;
}

#[link(name = "comctl32", kind = "raw-dylib")]
unsafe extern "system" {
    fn InitCommonControlsEx(icc: *const INITCOMMONCONTROLSEX) -> BOOL;
    fn ImageList_Create(cx: i32, cy: i32, flags: u32, initial: i32, grow: i32) -> HIMAGELIST;
}

// ---- common control helpers of these tests (comctl32 version 5.82) ----

#[repr(C)]
#[derive(Clone, Copy)]
struct NMHDR5 {
    hwnd_from: HWND,
    id_from: usize,
    code: u32,
}

#[repr(C)]
#[derive(Clone, Copy)]
struct LVITEMW {
    mask: u32,
    item: i32,
    sub_item: i32,
    state: u32,
    state_mask: u32,
    text: *mut u16,
    text_max: i32,
    image: i32,
    param: isize,
    indent: i32,
}

#[repr(C)]
#[derive(Clone, Copy)]
struct LVCOLUMNW {
    mask: u32,
    fmt: i32,
    cx: i32,
    text: *mut u16,
    text_max: i32,
    sub_item: i32,
    image: i32,
    order: i32,
}

#[repr(C)]
#[derive(Clone, Copy)]
struct NMLISTVIEW {
    hdr: NMHDR5,
    item: i32,
    sub_item: i32,
    new_state: u32,
    old_state: u32,
    changed: u32,
    action: POINT,
    param: isize,
}

#[repr(C)]
#[derive(Clone, Copy)]
struct NMLVDISPINFOW {
    hdr: NMHDR5,
    item: LVITEMW,
}

#[repr(C)]
#[derive(Clone, Copy)]
struct LVHITTESTINFO {
    pt: POINT,
    flags: u32,
    item: i32,
    sub_item: i32,
}

#[repr(C)]
#[derive(Clone, Copy)]
struct TVITEMW {
    mask: u32,
    item: isize,
    state: u32,
    state_mask: u32,
    text: *mut u16,
    text_max: i32,
    image: i32,
    sel_image: i32,
    children: i32,
    param: isize,
}

#[repr(C)]
#[derive(Clone, Copy)]
struct TVINSERTSTRUCTW {
    parent: isize,
    after: isize,
    item: TVITEMW,
}

#[repr(C)]
#[derive(Clone, Copy)]
struct NMTREEVIEWW {
    hdr: NMHDR5,
    action: u32,
    old: TVITEMW,
    new: TVITEMW,
    pt: POINT,
}

#[repr(C)]
#[derive(Clone, Copy)]
struct NMTVDISPINFOW {
    hdr: NMHDR5,
    item: TVITEMW,
}

#[repr(C)]
#[derive(Clone, Copy)]
struct TVHITTESTINFO {
    pt: POINT,
    flags: u32,
    item: isize,
}

#[repr(C)]
#[derive(Clone, Copy)]
struct TBBUTTON {
    bitmap: i32,
    command: i32,
    state: u8,
    style: u8,
    reserved: [u8; 6],
    data: usize,
    string: isize,
}

#[repr(C)]
#[derive(Clone, Copy)]
struct TBADDBITMAP {
    instance: HINSTANCE,
    id: usize,
}

const LVM_GETITEMCOUNT: u32 = 0x1004;
const LVM_INSERTITEMW: u32 = 0x104D;
const LVM_DELETEITEM: u32 = 0x1008;
const LVM_DELETEALLITEMS: u32 = 0x1009;
const LVM_GETITEMW: u32 = 0x104B;
const LVM_SETITEMW: u32 = 0x104C;
const LVM_GETNEXTITEM: u32 = 0x100C;
const LVM_GETITEMRECT: u32 = 0x100E;
const LVM_HITTEST: u32 = 0x1012;
const LVM_ENSUREVISIBLE: u32 = 0x1013;
const LVM_GETCOLUMNW: u32 = 0x105F;
const LVM_SETCOLUMNW: u32 = 0x1060;
const LVM_INSERTCOLUMNW: u32 = 0x1061;
const LVM_DELETECOLUMN: u32 = 0x101C;
const LVM_GETCOLUMNWIDTH: u32 = 0x101D;
const LVM_SETCOLUMNWIDTH: u32 = 0x101E;
const LVM_GETHEADER: u32 = 0x101F;
const LVM_GETTOPINDEX: u32 = 0x1027;
const LVM_SETITEMSTATE: u32 = 0x102B;
const LVM_GETITEMSTATE: u32 = 0x102C;
const LVM_GETITEMTEXTW: u32 = 0x1073;
const LVM_SETITEMTEXTW: u32 = 0x1074;
const LVM_SETITEMCOUNT: u32 = 0x102F;
const LVM_GETSELECTEDCOUNT: u32 = 0x1032;
const LVM_SETEXTENDEDLISTVIEWSTYLE: u32 = 0x1036;
const LVM_GETEXTENDEDLISTVIEWSTYLE: u32 = 0x1037;
const LVM_GETSUBITEMRECT: u32 = 0x1038;
const LVM_GETSELECTIONMARK: u32 = 0x1042;
const LVM_SETSELECTIONMARK: u32 = 0x1043;
const LVM_GETVIEWRECT: u32 = 0x1022;
const LVM_GETSTRINGWIDTHW: u32 = 0x1057;
const LVM_SETBKCOLOR: u32 = 0x1001;
const LVM_GETBKCOLOR: u32 = 0x1000;
const LVM_SETTEXTCOLOR: u32 = 0x1024;
const LVM_GETTEXTCOLOR: u32 = 0x1023;
const LVM_SETTEXTBKCOLOR: u32 = 0x1026;
const LVM_GETTEXTBKCOLOR: u32 = 0x1025;
const LVM_FINDITEMW: u32 = 0x1053;
const LVM_GETCALLBACKMASK: u32 = 0x100A;
const LVM_SETCALLBACKMASK: u32 = 0x100B;
const LVM_SETIMAGELIST: u32 = 0x1003;
const LVM_GETIMAGELIST: u32 = 0x1002;
const LVIF_TEXT: u32 = 1;
const LVIF_IMAGE: u32 = 2;
const LVIF_PARAM: u32 = 4;
const LVIF_STATE: u32 = 8;
const LVIS_FOCUSED: u32 = 1;
const LVIS_SELECTED: u32 = 2;
const LVS_ICON: u32 = 0;
const LVS_REPORT: u32 = 1;
const LVS_SMALLICON: u32 = 2;
const LVS_LIST: u32 = 3;
const LVS_SINGLESEL: u32 = 4;
const LVS_SHOWSELALWAYS: u32 = 8;
const LVS_NOCOLUMNHEADER: u32 = 0x4000;
const LVS_OWNERDATA: u32 = 0x1000;
const LVS_EX_FULLROWSELECT: u32 = 0x20;
const LVS_EX_GRIDLINES: u32 = 1;
const LVS_EX_CHECKBOXES: u32 = 4;
const LVCF_FMT: u32 = 1;
const LVCF_WIDTH: u32 = 2;
const LVCF_TEXT: u32 = 4;
const LVCF_SUBITEM: u32 = 8;
const LVNI_ALL: u32 = 0;
const LVNI_FOCUSED: u32 = 1;
const LVNI_SELECTED: u32 = 2;
const LVNI_ABOVE: u32 = 0x100;
const LVNI_BELOW: u32 = 0x200;
const LVIR_BOUNDS: i32 = 0;
const LVIR_ICON: i32 = 1;
const LVIR_LABEL: i32 = 2;
const LVIR_SELECTBOUNDS: i32 = 3;
const LPSTR_TEXTCALLBACK_: isize = -1;

const TVM_INSERTITEMW: u32 = 0x1132;
const TVM_DELETEITEM: u32 = 0x1101;
const TVM_EXPAND: u32 = 0x1102;
const TVM_GETITEMRECT: u32 = 0x1104;
const TVM_GETCOUNT: u32 = 0x1105;
const TVM_GETINDENT: u32 = 0x1106;
const TVM_SETINDENT: u32 = 0x1107;
const TVM_GETNEXTITEM: u32 = 0x110A;
const TVM_SELECTITEM: u32 = 0x110B;
const TVM_GETITEMW: u32 = 0x113E;
const TVM_SETITEMW: u32 = 0x113F;
const TVM_GETVISIBLECOUNT: u32 = 0x1110;
const TVM_HITTEST: u32 = 0x1111;
const TVM_ENSUREVISIBLE: u32 = 0x1114;
const TVM_SORTCHILDREN: u32 = 0x1113;
const TVM_GETITEMHEIGHT: u32 = 0x111C;
const TVM_SETITEMHEIGHT: u32 = 0x111B;
const TVM_GETITEMSTATE: u32 = 0x1127;
const TVM_SETBKCOLOR: u32 = 0x111D;
const TVM_GETBKCOLOR: u32 = 0x111F;
const TVM_SETTEXTCOLOR: u32 = 0x111E;
const TVM_GETTEXTCOLOR: u32 = 0x1120;
const TVM_SETIMAGELIST: u32 = 0x1109;
const TVM_GETIMAGELIST: u32 = 0x1108;
const TVI_ROOT: isize = -0x10000;
const TVI_FIRST: isize = -0xFFFF;
const TVI_LAST: isize = -0xFFFE;
const TVI_SORT: isize = -0xFFFD;
const TVIF_TEXT: u32 = 1;
const TVIF_IMAGE: u32 = 2;
const TVIF_PARAM: u32 = 4;
const TVIF_STATE: u32 = 8;
const TVIF_HANDLE: u32 = 0x10;
const TVIF_SELECTEDIMAGE: u32 = 0x20;
const TVIF_CHILDREN: u32 = 0x40;
const TVIS_SELECTED: u32 = 2;
const TVIS_EXPANDED: u32 = 0x20;
const TVIS_EXPANDEDONCE: u32 = 0x40;
const TVIS_BOLD: u32 = 0x10;
const TVGN_ROOT: usize = 0;
const TVGN_NEXT: usize = 1;
const TVGN_PREVIOUS: usize = 2;
const TVGN_PARENT: usize = 3;
const TVGN_CHILD: usize = 4;
const TVGN_FIRSTVISIBLE: usize = 5;
const TVGN_NEXTVISIBLE: usize = 6;
const TVGN_PREVIOUSVISIBLE: usize = 7;
const TVGN_DROPHILITE: usize = 8;
const TVGN_CARET: usize = 9;
const TVGN_LASTVISIBLE: usize = 10;
const TVE_COLLAPSE: usize = 1;
const TVE_EXPAND: usize = 2;
const TVE_TOGGLE: usize = 3;
const TVE_COLLAPSERESET: usize = 0x8000;
const TVS_HASBUTTONS: u32 = 1;
const TVS_HASLINES: u32 = 2;
const TVS_LINESATROOT: u32 = 4;
const TVS_SHOWSELALWAYS: u32 = 0x20;
const TVS_DISABLEDRAGDROP: u32 = 0x10;
const TVC_UNKNOWN: u32 = 0;
const TVC_BYMOUSE: u32 = 1;
const TVC_BYKEYBOARD: u32 = 2;

const TB_ENABLEBUTTON: u32 = 0x401;
const TB_CHECKBUTTON: u32 = 0x402;
const TB_PRESSBUTTON: u32 = 0x403;
const TB_HIDEBUTTON: u32 = 0x404;
const TB_INDETERMINATE: u32 = 0x405;
const TB_ISBUTTONENABLED: u32 = 0x409;
const TB_ISBUTTONCHECKED: u32 = 0x40A;
const TB_ISBUTTONPRESSED: u32 = 0x40B;
const TB_ISBUTTONHIDDEN: u32 = 0x40C;
const TB_ISBUTTONINDETERMINATE: u32 = 0x40D;
const TB_SETSTATE: u32 = 0x411;
const TB_GETSTATE: u32 = 0x412;
const TB_ADDBITMAP: u32 = 0x413;
const TB_DELETEBUTTON: u32 = 0x416;
const TB_GETBUTTON: u32 = 0x417;
const TB_BUTTONCOUNT: u32 = 0x418;
const TB_COMMANDTOINDEX: u32 = 0x419;
const TB_GETITEMRECT: u32 = 0x41D;
const TB_BUTTONSTRUCTSIZE: u32 = 0x41E;
const TB_SETBUTTONSIZE: u32 = 0x41F;
const TB_SETBITMAPSIZE: u32 = 0x420;
const TB_AUTOSIZE: u32 = 0x421;
const TB_GETTOOLTIPS: u32 = 0x423;
const TB_SETPARENT: u32 = 0x425;
const TB_SETROWS: u32 = 0x427;
const TB_GETROWS: u32 = 0x428;
const TB_GETBITMAPFLAGS: u32 = 0x429;
const TB_SETCMDID: u32 = 0x42A;
const TB_SETIMAGELIST: u32 = 0x430;
const TB_GETIMAGELIST: u32 = 0x431;
const TB_SETHOTIMAGELIST: u32 = 0x434;
const TB_GETHOTIMAGELIST: u32 = 0x435;
const TB_SETDISABLEDIMAGELIST: u32 = 0x436;
const TB_GETDISABLEDIMAGELIST: u32 = 0x437;
const TB_SETSTYLE: u32 = 0x438;
const TB_GETSTYLE: u32 = 0x439;
const TB_GETBUTTONSIZE: u32 = 0x43A;
const TB_SETBUTTONWIDTH: u32 = 0x43B;
const TB_INSERTBUTTONW: u32 = 0x443;
const TB_ADDBUTTONSW: u32 = 0x444;
const TB_HITTEST: u32 = 0x445;
const TB_GETMAXSIZE: u32 = 0x453;
const TB_SETEXTENDEDSTYLE: u32 = 0x454;
const TB_GETEXTENDEDSTYLE: u32 = 0x455;
const TB_GETPADDING: u32 = 0x456;
const TB_SETPADDING: u32 = 0x457;
const TB_GETRECT: u32 = 0x433;
const TB_SETINDENT: u32 = 0x42F;
const TB_ADDSTRINGW: u32 = 0x44D;
const TB_GETBUTTONTEXTW: u32 = 0x44B;
const TB_GETTEXTROWS: u32 = 0x43D;
const TB_SETMAXTEXTROWS: u32 = 0x43C;
const TBSTATE_CHECKED: u8 = 1;
const TBSTATE_PRESSED: u8 = 2;
const TBSTATE_ENABLED: u8 = 4;
const TBSTATE_HIDDEN: u8 = 8;
const TBSTATE_INDETERMINATE: u8 = 0x10;
const TBSTATE_WRAP: u8 = 0x20;
const TBSTYLE_BUTTON: u8 = 0;
const TBSTYLE_SEP: u8 = 1;
const TBSTYLE_CHECK: u8 = 2;
const TBSTYLE_GROUP: u8 = 4;
const TBSTYLE_CHECKGROUP: u8 = 6;
const TBSTYLE_DROPDOWN: u8 = 8;
const TBSTYLE_AUTOSIZE: u8 = 0x10;
const TBSTYLE_TOOLTIPS: u32 = 0x100;
const TBSTYLE_WRAPABLE: u32 = 0x200;
const TBSTYLE_FLAT: u32 = 0x800;
const TBSTYLE_LIST: u32 = 0x1000;
const HINST_COMMCTRL: isize = -1;
const IDB_STD_SMALL_COLOR: usize = 0;
const IDB_STD_LARGE_COLOR: usize = 1;

const SB_SETTEXTW: u32 = 0x40B;
const SB_GETTEXTW: u32 = 0x40D;
const SB_GETTEXTLENGTHW: u32 = 0x40C;
const SB_SETPARTS: u32 = 0x404;
const SB_GETPARTS: u32 = 0x406;
const SB_GETBORDERS: u32 = 0x407;
const SB_SETMINHEIGHT: u32 = 0x408;
const SB_SIMPLE: u32 = 0x409;
const SB_GETRECT: u32 = 0x40A;
const SB_ISSIMPLE: u32 = 0x40E;
const SB_SETICON: u32 = 0x40F;
const SB_GETICON: u32 = 0x414;
const SBARS_SIZEGRIP: u32 = 0x100;
const SBT_NOBORDERS: u32 = 0x100;
const SBT_POPOUT: u32 = 0x200;
const SB_SIMPLEID: usize = 0xFF;

const HDM_GETITEMCOUNT: u32 = 0x1200;

const NM_CLICK_: i32 = -2;

const ICC_ALL5: u32 = 0xFF;

fn init_controls() -> BOOL {
    let icc = INITCOMMONCONTROLSEX { dw_size: 8, dw_icc: ICC_ALL5 };
    // SAFETY: a filled INITCOMMONCONTROLSEX.
    unsafe { InitCommonControlsEx(&icc) }
}

/// Names of notification codes the tests log.
fn nm_name(c: i32) -> &'static str {
    match c {
        -1 => "NM_OUTOFMEMORY",
        -2 => "NM_CLICK",
        -3 => "NM_DBLCLK",
        -4 => "NM_RETURN",
        -5 => "NM_RCLICK",
        -6 => "NM_RDBLCLK",
        -7 => "NM_SETFOCUS",
        -8 => "NM_KILLFOCUS",
        -12 => "NM_CUSTOMDRAW",
        -13 => "NM_HOVER",
        -14 => "NM_NCHITTEST",
        -15 => "NM_KEYDOWN",
        -16 => "NM_RELEASEDCAPTURE",
        -17 => "NM_SETCURSOR",
        -18 => "NM_CHAR",
        -19 => "NM_TOOLTIPSCREATED",
        -100 => "LVN_ITEMCHANGING",
        -101 => "LVN_ITEMCHANGED",
        -102 => "LVN_INSERTITEM",
        -103 => "LVN_DELETEITEM",
        -104 => "LVN_DELETEALLITEMS",
        -105 => "LVN_BEGINLABELEDITA",
        -106 => "LVN_ENDLABELEDITA",
        -108 => "LVN_COLUMNCLICK",
        -109 => "LVN_BEGINDRAG",
        -111 => "LVN_BEGINRDRAG",
        -113 => "LVN_ODCACHEHINT",
        -115 => "LVN_ODSTATECHANGED",
        -179 => "LVN_ODFINDITEMW",
        -116 => "LVN_ODFINDITEMA",
        -114 => "LVN_ITEMACTIVATE",
        -121 => "LVN_HOTTRACK",
        -150 => "LVN_GETDISPINFOA",
        -151 => "LVN_SETDISPINFOA",
        -155 => "LVN_KEYDOWN",
        -175 => "LVN_BEGINLABELEDITW",
        -176 => "LVN_ENDLABELEDITW",
        -177 => "LVN_GETDISPINFOW",
        -178 => "LVN_SETDISPINFOW",
        -401 => "TVN_SELCHANGINGA",
        -402 => "TVN_SELCHANGEDA",
        -403 => "TVN_GETDISPINFOA",
        -405 => "TVN_ITEMEXPANDINGA",
        -406 => "TVN_ITEMEXPANDEDA",
        -412 => "TVN_KEYDOWN",
        -450 => "TVN_SELCHANGINGW",
        -451 => "TVN_SELCHANGEDW",
        -452 => "TVN_GETDISPINFOW",
        -453 => "TVN_SETDISPINFOW",
        -454 => "TVN_ITEMEXPANDINGW",
        -455 => "TVN_ITEMEXPANDEDW",
        -458 => "TVN_DELETEITEMW",
        -456 => "TVN_BEGINDRAGW",
        -880 => "SBN_SIMPLEMODECHANGE",
        -20 => "NM_LDOWN",
        -300 => "HDN_ITEMCHANGINGA",
        -301 => "HDN_ITEMCHANGEDA",
        -320 => "HDN_ITEMCHANGINGW",
        -321 => "HDN_ITEMCHANGEDW",
        -306 => "HDN_BEGINTRACKA",
        -326 => "HDN_BEGINTRACKW",
        -700 => "TBN_GETBUTTONINFOA",
        -701 => "TBN_BEGINDRAG",
        -702 => "TBN_ENDDRAG",
        -703 => "TBN_BEGINADJUST",
        -704 => "TBN_ENDADJUST",
        -705 => "TBN_RESET",
        -706 => "TBN_QUERYINSERT",
        -707 => "TBN_QUERYDELETE",
        -708 => "TBN_TOOLBARCHANGE",
        -709 => "TBN_CUSTHELP",
        -710 => "TBN_DROPDOWN",
        -712 => "TBN_GETOBJECT",
        -713 => "TBN_HOTITEMCHANGE",
        -714 => "TBN_DRAGOUT",
        -715 => "TBN_DELETINGBUTTON",
        -716 => "TBN_GETDISPINFOA",
        -717 => "TBN_GETDISPINFOW",
        -718 => "TBN_GETINFOTIPA",
        -719 => "TBN_GETINFOTIPW",
        -720 => "TBN_GETBUTTONINFOW",
        -721 => "TBN_RESTORE",
        -722 => "TBN_SAVE",
        -723 => "TBN_INITCUSTOMIZE",
        _ => "notification?",
    }
}

/// What the parent window procedure of a test does with WM_NOTIFY of a
/// control: `CB_TEXT` is returned for text callbacks, `NOTIFY_LOG` says
/// which codes are logged (empty: all).
static CB_TEXT: Global<[u16; 16]> = Global::new([0; 16]);
static CB_ON: Global<bool> = Global::new(false);
static CB_RET: Global<isize> = Global::new(0);
static CMDS: Global<u32> = Global::new(0);

fn set_cb_text(s: &str) {
    let a = CB_TEXT.get();
    *a = [0; 16];
    for (i, u) in s.encode_utf16().take(15).enumerate() {
        a[i] = u;
    }
}

unsafe extern "system" fn cc_parent(h: HWND, m: u32, w: WPARAM, l: LPARAM) -> LRESULT {
    match m {
        WM_NOTIFY => {
            // SAFETY: lParam is an NMHDR (the first member of every notification structure).
            let hdr = unsafe { &*(l as *const NMHDR5) };
            let code = hdr.code as i32;
            let mut b = Buf::<300>::new();
            if nm_name(code) == "notification?" {
                let _ = write!(b, "WM_NOTIFY code {} from control id {}", code, hdr.id_from);
            } else {
                let _ = write!(b, "WM_NOTIFY {} from control id {}", nm_name(code), hdr.id_from);
            }
            match code {
                -100 | -101 | -102 | -103 | -108 | -2 | -3 | -5 | -114 => {
                    // SAFETY: an NMLISTVIEW for these codes (NM_CLICK has a different structure: only the header is read there).
                    if code <= -100 {
                        let n = unsafe { &*(l as *const NMLISTVIEW) };
                        let _ = write!(b, ": item {} subitem {} new state {:#x} old state {:#x} changed {:#x}", n.item, n.sub_item, n.new_state, n.old_state, n.changed);
                    }
                }
                -177 => {
                    // SAFETY: an NMLVDISPINFOW.
                    let n = unsafe { &mut *(l as *mut NMLVDISPINFOW) };
                    let _ = write!(b, ": item {} subitem {} mask {:#x} state {:#x} text buffer {} chars", n.item.item, n.item.sub_item, n.item.mask, n.item.state, n.item.text_max);
                    if *CB_ON.get() && n.item.mask & LVIF_TEXT != 0 && !n.item.text.is_null() {
                        let mut i = 0;
                        while CB_TEXT.get()[i] != 0 && (i as i32) + 1 < n.item.text_max {
                            // SAFETY: the buffer has text_max units.
                            unsafe { *n.item.text.add(i) = CB_TEXT.get()[i] };
                            i += 1;
                        }
                        // SAFETY: the buffer has at least one unit.
                        unsafe { *n.item.text.add(i) = 0 };
                    }
                }
                -450 | -451 | -454 | -455 | -458 => {
                    // SAFETY: an NMTREEVIEWW.
                    let n = unsafe { &*(l as *const NMTREEVIEWW) };
                    let _ = write!(b, ": action {} old item {} (mask {:#x} state {:#x}) new item {} (mask {:#x} state {:#x})", n.action, if n.old.item == 0 { 0 } else { 1 }, n.old.mask, n.old.state, if n.new.item == 0 { 0 } else { 1 }, n.new.mask, n.new.state);
                }
                -452 => {
                    // SAFETY: an NMTVDISPINFOW.
                    let n = unsafe { &mut *(l as *mut NMTVDISPINFOW) };
                    let _ = write!(b, ": mask {:#x} state {:#x}", n.item.mask, n.item.state);
                    if *CB_ON.get() && n.item.mask & TVIF_TEXT != 0 && !n.item.text.is_null() {
                        let mut i = 0;
                        while CB_TEXT.get()[i] != 0 && (i as i32) + 1 < n.item.text_max {
                            // SAFETY: the buffer has text_max units.
                            unsafe { *n.item.text.add(i) = CB_TEXT.get()[i] };
                            i += 1;
                        }
                        // SAFETY: the buffer has at least one unit.
                        unsafe { *n.item.text.add(i) = 0 };
                    }
                    if n.item.mask & TVIF_CHILDREN != 0 {
                        n.item.children = 1;
                    }
                }
                _ => {}
            }
            logln!("{}", b.as_str());
            *CB_RET.get()
        }
        WM_COMMAND => {
            *CMDS.get() += 1;
            logln!("WM_COMMAND id {} notify code {}", w & 0xFFFF, (w >> 16) & 0xFFFF);
            0
        }
        // SAFETY: default processing.
        _ => unsafe { DefWindowProcW(h, m, w, l) },
    }
}

/// A visible top-level window that logs WM_NOTIFY and WM_COMMAND (client 400 x 300).
fn cc_window() -> HWND {
    static DONE: Global<bool> = Global::new(false);
    if !*DONE.get() {
        register(wz!("CcTestParent"), Some(cc_parent), 0, 0, core::ptr::null_mut());
        *DONE.get() = true;
    }
    let style = WS_OVERLAPPED | WS_CAPTION | WS_VISIBLE;
    let mut r = RECT { left: 0, top: 0, right: 400, bottom: 300 };
    // SAFETY: r is writable.
    unsafe { AdjustWindowRectEx(&mut r, style, 0, 0) };
    let h = create(0, wz!("CcTestParent"), wz!("controls"), style, 20, 20, r.right - r.left, r.bottom - r.top, core::ptr::null_mut(), 0);
    pump();
    log_clear();
    h
}

/// A child control of a window: class name (NUL-terminated UTF-16), style, position, size, id.
fn cc_child(parent: HWND, class: &[u16], style: u32, x: i32, y: i32, w: i32, h: i32, id: usize) -> HWND {
    create(0, class, wz!(""), WS_CHILD | WS_VISIBLE | style, x, y, w, h, parent, id)
}

fn sm(h: HWND, m: u32, w: usize, l: isize) -> isize {
    // SAFETY: a message to a window of the test.
    unsafe { SendMessageW(h, m, w, l) }
}

fn smp<T>(h: HWND, m: u32, w: usize, p: &mut T) -> isize {
    // SAFETY: a message with a pointer to a live structure.
    unsafe { SendMessageW(h, m, w, p as *mut T as isize) }
}

/// Print and clear the log (notifications the parent got).
fn cc_log(t: &mut Test, label: &str) {
    pump();
    say!(t, "{label}:");
    flush_log(t, "    ");
}

fn text16(s: &str, out: &mut [u16; 64]) -> *mut u16 {
    for (i, u) in s.encode_utf16().take(63).enumerate() {
        out[i] = u;
    }
    out.as_mut_ptr()
}

/// The text read back into a buffer as ASCII.
fn ascii16(buf: &[u16; 64]) -> Buf<300> {
    let mut b = Buf::<300>::new();
    ascii_of(buf, &mut b);
    b
}

fn cls_name(h: HWND) -> Buf<300> {
    let mut b = [0u16; 64];
    // SAFETY: b has 64 units.
    unsafe { GetClassNameW(h, b.as_mut_ptr(), 64) };
    ascii16(&b)
}

/// A colour bitmap that is not selected into a DC (for toolbars and image lists).
fn tb_bitmap(w: i32, h: i32) -> HBITMAP {
    // SAFETY: plain call.
    unsafe { CreateBitmap(w, h, 1, 32, core::ptr::null()) }
}

/// Print and clear the log summarised: for each kind of line (the text up to " from") its count and the first line.
fn cc_log_short(t: &mut Test, label: &str) {
    pump();
    let n = LOG.get().len;
    let text = core::str::from_utf8(&LOG.get().buf[..n]).unwrap_or("");
    let mut kinds: [(&str, usize, &str); 12] = [("", 0, ""); 12];
    let mut nk = 0;
    for line in text.lines() {
        let key = match line.find(" from") {
            Some(i) => &line[..i],
            None => line,
        };
        let mut found = false;
        for k in kinds.iter_mut().take(nk) {
            if k.0 == key {
                k.1 += 1;
                found = true;
                break;
            }
        }
        if !found && nk < 12 {
            kinds[nk] = (key, 1, line);
            nk += 1;
        }
    }
    say!(t, "{label}:");
    for k in kinds.iter().take(nk) {
        say!(t, "    {} x {}; first: {}", k.0, k.1, k.2);
    }
    LOG.get().len = 0;
}

/// The control drawn into a memory bitmap with WM_PRINT (client area, background, children).
fn print_to(h: HWND, dc: HDC) -> isize {
    // SAFETY: a message to our window with our memory DC.
    unsafe { SendMessageW(h, 0x0317, dc as usize, 0x1C) }
}

poly_wtest::main!(test);

fn btn(bitmap: i32, command: i32, state: u8, style: u8) -> TBBUTTON {
    TBBUTTON { bitmap, command, state, style, reserved: [0; 6], data: 0, string: 0 }
}

fn rects(t: &mut Test, tb: HWND, label: &str) {
    let n = sm(tb, TB_BUTTONCOUNT, 0, 0) as usize;
    let mut line = Buf::<250>::new();
    for i in 0..n {
        let mut r = RECT::default();
        let ok = smp(tb, TB_GETITEMRECT, i, &mut r);
        let _ = write!(line, " [{i}] {ok} ({}, {}, {}, {})", r.left, r.top, r.right, r.bottom);
    }
    say!(t, "{label}: item rectangles{}", line.as_str());
}

fn size_of_window(t: &mut Test, tb: HWND, label: &str) {
    let r = rect_of(tb);
    let c = client_of(tb);
    say!(t, "{label}: window {} x {}, client {} x {}", r.right - r.left, r.bottom - r.top, c.right, c.bottom);
}

fn test(t: &mut Test) {
    // SAFETY: plain calls.
    unsafe { GetSystemMetrics(0) };
    init_controls();
    let p = cc_window();
    let tb = cc_child(p, wz!("ToolbarWindow32"), 0, 0, 0, 0, 0, 201);
    say!(t, "toolbar made: {}, class '{}'", !tb.is_null(), cls_name(tb).as_str());
    // SAFETY: plain call.
    let st = unsafe { GetWindowLongW(tb, GWL_STYLE) } as u32;
    say!(t, "style: CCS_TOP {} CCS_BOTTOM {} WS_CHILD {} WS_VISIBLE {} TBSTYLE_TOOLTIPS {}", st & 3 == 1, st & 3 == 3, st & WS_CHILD != 0, st & WS_VISIBLE != 0, st & TBSTYLE_TOOLTIPS != 0);
    say!(t, "TB_BUTTONCOUNT = {}", sm(tb, TB_BUTTONCOUNT, 0, 0));
    say!(t, "TB_BUTTONSTRUCTSIZE({}) = {}", core::mem::size_of::<TBBUTTON>(), sm(tb, TB_BUTTONSTRUCTSIZE, core::mem::size_of::<TBBUTTON>(), 0));
    size_of_window(t, tb, "empty toolbar");
    say!(t, "TB_GETBUTTONSIZE = {:#x}", sm(tb, TB_GETBUTTONSIZE, 0, 0));
    say!(t, "TB_GETROWS = {}", sm(tb, TB_GETROWS, 0, 0));
    say!(t, "TB_GETBITMAPFLAGS = {}", sm(tb, TB_GETBITMAPFLAGS, 0, 0));
    // a bitmap of three 16 x 15 images
    let bmp = tb_bitmap(48, 15);
    let mut ab = TBADDBITMAP { instance: core::ptr::null_mut(), id: bmp as usize };
    say!(t, "TB_ADDBITMAP(3 images) = {}", smp(tb, TB_ADDBITMAP, 3, &mut ab));
    let bmp2 = tb_bitmap(32, 15);
    let mut ab2 = TBADDBITMAP { instance: core::ptr::null_mut(), id: bmp2 as usize };
    say!(t, "TB_ADDBITMAP(2 more images) = {}", smp(tb, TB_ADDBITMAP, 2, &mut ab2));
    // buttons
    let mut bs = [btn(0, 101, TBSTATE_ENABLED, TBSTYLE_BUTTON), btn(0, 0, 0, TBSTYLE_SEP), btn(1, 102, TBSTATE_ENABLED, TBSTYLE_CHECK), btn(2, 103, TBSTATE_ENABLED | TBSTATE_CHECKED, TBSTYLE_CHECK), btn(3, 104, TBSTATE_ENABLED | TBSTATE_HIDDEN, TBSTYLE_BUTTON), btn(4, 105, 0, TBSTYLE_BUTTON)];
    say!(t, "TB_ADDBUTTONSW(6) = {}", sm(tb, TB_ADDBUTTONSW, 6, bs.as_mut_ptr() as isize));
    say!(t, "TB_BUTTONCOUNT = {}", sm(tb, TB_BUTTONCOUNT, 0, 0));
    for i in 0..7 {
        let mut b: TBBUTTON = zeroed();
        b.bitmap = -7;
        let r = smp(tb, TB_GETBUTTON, i, &mut b);
        say!(t, "TB_GETBUTTON({i}) = {r}: bitmap {} command {} state {:#x} style {:#x}", b.bitmap, b.command, b.state, b.style);
    }
    for id in [101, 102, 105, 999, 0] {
        say!(t, "TB_COMMANDTOINDEX({id}) = {}", sm(tb, TB_COMMANDTOINDEX, id, 0));
    }
    rects(t, tb, "after adding");
    size_of_window(t, tb, "toolbar");
    say!(t, "TB_GETBUTTONSIZE = {:#x}", sm(tb, TB_GETBUTTONSIZE, 0, 0));
    let mut sz = SIZE { cx: 0, cy: 0 };
    say!(t, "TB_GETMAXSIZE = {} ({} x {})", smp(tb, TB_GETMAXSIZE, 0, &mut sz), sz.cx, sz.cy);
    say!(t, "TB_GETROWS = {}", sm(tb, TB_GETROWS, 0, 0));
    say!(t, "TB_GETPADDING = {:#x}", sm(tb, TB_GETPADDING, 0, 0));
    say!(t, "TB_AUTOSIZE = {}", sm(tb, TB_AUTOSIZE, 0, 0));
    size_of_window(t, tb, "after TB_AUTOSIZE");
    // inserting and deleting
    let mut ins = btn(2, 110, TBSTATE_ENABLED, TBSTYLE_BUTTON);
    say!(t, "TB_INSERTBUTTONW(1) = {}", smp(tb, TB_INSERTBUTTONW, 1, &mut ins));
    say!(t, "TB_COMMANDTOINDEX(110) = {}", sm(tb, TB_COMMANDTOINDEX, 110, 0));
    let mut ins2 = btn(2, 111, TBSTATE_ENABLED, TBSTYLE_BUTTON);
    say!(t, "TB_INSERTBUTTONW(-1) = {}", smp(tb, TB_INSERTBUTTONW, usize::MAX, &mut ins2));
    say!(t, "TB_INSERTBUTTONW(100) = {}", smp(tb, TB_INSERTBUTTONW, 100, &mut ins2));
    say!(t, "TB_BUTTONCOUNT = {}", sm(tb, TB_BUTTONCOUNT, 0, 0));
    say!(t, "TB_DELETEBUTTON(1) = {}", sm(tb, TB_DELETEBUTTON, 1, 0));
    say!(t, "TB_DELETEBUTTON(50) = {}", sm(tb, TB_DELETEBUTTON, 50, 0));
    say!(t, "TB_BUTTONCOUNT = {}", sm(tb, TB_BUTTONCOUNT, 0, 0));
    cc_log(t, "notifications and commands so far");
    // sizes
    say!(t, "TB_SETBUTTONSIZE(30, 28) = {}", sm(tb, TB_SETBUTTONSIZE, 0, makelparam(30, 28)));
    say!(t, "TB_GETBUTTONSIZE = {:#x}", sm(tb, TB_GETBUTTONSIZE, 0, 0));
    rects(t, tb, "after TB_SETBUTTONSIZE(30, 28)");
    say!(t, "TB_SETBITMAPSIZE(20, 18) = {}", sm(tb, TB_SETBITMAPSIZE, 0, makelparam(20, 18)));
    say!(t, "TB_GETBUTTONSIZE = {:#x}", sm(tb, TB_GETBUTTONSIZE, 0, 0));
    // image lists
    say!(t, "TB_GETIMAGELIST = {}", if sm(tb, TB_GETIMAGELIST, 0, 0) == 0 { "NULL" } else { "set" });
    // SAFETY: an image list of ours.
    let il = unsafe { ImageList_Create(16, 15, 0x21, 4, 4) };
    let old = sm(tb, TB_SETIMAGELIST, 0, il as isize);
    say!(t, "TB_SETIMAGELIST gives the old list: {}", if old == 0 { "NULL" } else { "set" });
    say!(t, "TB_GETIMAGELIST is the list we set: {}", sm(tb, TB_GETIMAGELIST, 0, 0) == il as isize);
    say!(t, "TB_GETHOTIMAGELIST = {}", if sm(tb, TB_GETHOTIMAGELIST, 0, 0) == 0 { "NULL" } else { "set" });
    say!(t, "TB_GETDISABLEDIMAGELIST = {}", if sm(tb, TB_GETDISABLEDIMAGELIST, 0, 0) == 0 { "NULL" } else { "set" });
    say!(t, "TB_SETSTYLE/GETSTYLE: {} then {:#x} (the TBSTYLE_ bits)", sm(tb, TB_SETSTYLE, 0, (TBSTYLE_FLAT | TBSTYLE_TOOLTIPS) as isize), sm(tb, TB_GETSTYLE, 0, 0));
    say!(t, "TB_SETEXTENDEDSTYLE(TBSTYLE_EX_DRAWDDARROWS) = {:#x}", sm(tb, TB_SETEXTENDEDSTYLE, 0, 1));
    say!(t, "TB_GETEXTENDEDSTYLE = {:#x}", sm(tb, TB_GETEXTENDEDSTYLE, 0, 0));
    // SAFETY: our window.
    unsafe { DestroyWindow(p) };
}

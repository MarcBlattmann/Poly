//! `user32`'s side of win32k (ADR 0128): the message sequences that run
//! window procedures, which the kernel never calls. `user32.dll` runs these
//! functions with an [`Env`] that makes the `PolyUserCall` system call and
//! calls procedures; host tests run them against a [`crate::wm::Wm`] in
//! memory with procedures written in Rust.
//!
//! The orders, flags and parameters come from the spec sheets of the first
//! slice (`specs/windows/user32.dll/`): `CreateWindowExW.yaml`,
//! `DestroyWindow.yaml`, `ShowWindow.yaml`, `SetWindowPos.yaml`,
//! `DefWindowProcW.yaml`, the message functions' sheets. Structures passed
//! to procedures are the `winuser.h` layouts, in this process's memory.

use crate::frame::Rect;
use crate::metrics::{self, *};
use crate::proto::*;
use crate::wm::{ERROR_INVALID_WINDOW_HANDLE, PM_REMOVE, WM_QUIT, WM_TIMER};
use alloc::vec::Vec;

// Messages (`winuser.h`).
pub const WM_NULL: u32 = 0x0000;
pub const WM_CREATE: u32 = 0x0001;
pub const WM_DESTROY: u32 = 0x0002;
pub const WM_MOVE: u32 = 0x0003;
pub const WM_SIZE: u32 = 0x0005;
pub const WM_SETTEXT: u32 = 0x000C;
pub const WM_GETTEXT: u32 = 0x000D;
pub const WM_GETTEXTLENGTH: u32 = 0x000E;
pub const WM_CLOSE: u32 = 0x0010;
pub const WM_QUERYENDSESSION: u32 = 0x0011;
pub const WM_ERASEBKGND: u32 = 0x0014;
pub const WM_SHOWWINDOW: u32 = 0x0018;
pub const WM_MOUSEACTIVATE: u32 = 0x0021;
pub const WM_CHILDACTIVATE: u32 = 0x0022;
pub const WM_GETMINMAXINFO: u32 = 0x0024;
pub const WM_WINDOWPOSCHANGING: u32 = 0x0046;
pub const WM_WINDOWPOSCHANGED: u32 = 0x0047;
pub const WM_NOTIFYFORMAT: u32 = 0x0055;
pub const WM_CONTEXTMENU: u32 = 0x007B;
pub const WM_STYLECHANGING: u32 = 0x007C;
pub const WM_STYLECHANGED: u32 = 0x007D;
pub const WM_NCCREATE: u32 = 0x0081;
pub const WM_NCDESTROY: u32 = 0x0082;
pub const WM_NCCALCSIZE: u32 = 0x0083;
pub const WM_NCHITTEST: u32 = 0x0084;
pub const WM_KEYDOWN: u32 = 0x0100;
pub const WM_KEYUP: u32 = 0x0101;
pub const WM_CHAR: u32 = 0x0102;
pub const WM_SYSKEYDOWN: u32 = 0x0104;
pub const WM_SYSKEYUP: u32 = 0x0105;
pub const WM_SYSCHAR: u32 = 0x0106;
pub const WM_SYSCOMMAND: u32 = 0x0112;
pub const WM_RBUTTONUP: u32 = 0x0205;
pub const WM_MOUSEWHEEL: u32 = 0x020A;
pub const WM_PARENTNOTIFY: u32 = 0x0210;

// SetWindowPos flags (`winuser.h`); 0x800 and 0x1000 are the "client
// keeps its size / position" bits the sheets observed in WM_WINDOWPOSCHANGED.
pub const SWP_NOSIZE: u32 = 0x0001;
pub const SWP_NOMOVE: u32 = 0x0002;
pub const SWP_NOZORDER: u32 = 0x0004;
pub const SWP_NOREDRAW: u32 = 0x0008;
pub const SWP_NOACTIVATE: u32 = 0x0010;
pub const SWP_FRAMECHANGED: u32 = 0x0020;
pub const SWP_SHOWWINDOW: u32 = 0x0040;
pub const SWP_HIDEWINDOW: u32 = 0x0080;
pub const SWP_NOSENDCHANGING: u32 = 0x0400;
pub const SWP_NOCLIENTSIZE: u32 = 0x0800;
pub const SWP_NOCLIENTMOVE: u32 = 0x1000;
pub const SWP_STATECHANGED: u32 = 0x8000;

// ShowWindow commands.
pub const SW_HIDE: i32 = 0;
pub const SW_SHOWNORMAL: i32 = 1;
pub const SW_SHOWMINIMIZED: i32 = 2;
pub const SW_SHOWMAXIMIZED: i32 = 3;
pub const SW_SHOWNOACTIVATE: i32 = 4;
pub const SW_SHOW: i32 = 5;
pub const SW_MINIMIZE: i32 = 6;
pub const SW_SHOWMINNOACTIVE: i32 = 7;
pub const SW_SHOWNA: i32 = 8;
pub const SW_RESTORE: i32 = 9;
pub const SW_SHOWDEFAULT: i32 = 10;
pub const SW_FORCEMINIMIZE: i32 = 11;

pub const SIZE_RESTORED: u64 = 0;
pub const SIZE_MINIMIZED: u64 = 1;
pub const SIZE_MAXIMIZED: u64 = 2;

pub const SC_CLOSE: u64 = 0xF060;
pub const ERROR_ACCESS_DENIED: u32 = 5;

/// `RECT` (`windef.h`).
#[repr(C)]
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct CRect {
    pub left: i32,
    pub top: i32,
    pub right: i32,
    pub bottom: i32,
}

impl From<Rect> for CRect {
    fn from(r: Rect) -> CRect {
        CRect { left: r.left, top: r.top, right: r.right, bottom: r.bottom }
    }
}

impl From<CRect> for Rect {
    fn from(r: CRect) -> Rect {
        Rect::new(r.left, r.top, r.right, r.bottom)
    }
}

/// `POINT`.
#[repr(C)]
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct CPoint {
    pub x: i32,
    pub y: i32,
}

/// `CREATESTRUCTW`.
#[repr(C)]
#[derive(Clone, Copy, Debug, Default)]
pub struct CreateStructW {
    pub create_params: u64,
    pub instance: u64,
    pub menu: u64,
    pub parent: u64,
    pub cy: i32,
    pub cx: i32,
    pub y: i32,
    pub x: i32,
    pub style: u32,
    pub name: u64,
    pub class: u64,
    pub ex_style: u32,
}

/// `MINMAXINFO`.
#[repr(C)]
#[derive(Clone, Copy, Debug, Default)]
pub struct MinMaxInfo {
    pub reserved: CPoint,
    pub max_size: CPoint,
    pub max_position: CPoint,
    pub min_track: CPoint,
    pub max_track: CPoint,
}

/// `WINDOWPOS`.
#[repr(C)]
#[derive(Clone, Copy, Debug, Default)]
pub struct WindowPos {
    pub hwnd: u64,
    pub insert_after: u64,
    pub x: i32,
    pub y: i32,
    pub cx: i32,
    pub cy: i32,
    pub flags: u32,
}

/// `NCCALCSIZE_PARAMS`.
#[repr(C)]
#[derive(Clone, Copy, Debug)]
pub struct NcCalcSizeParams {
    pub rgrc: [CRect; 3],
    pub pos: *mut WindowPos,
}

/// `STYLESTRUCT`.
#[repr(C)]
#[derive(Clone, Copy, Debug, Default)]
pub struct StyleStruct {
    pub old: u32,
    pub new: u32,
}

/// `MSG`.
#[repr(C)]
#[derive(Clone, Copy, Debug, Default)]
pub struct CMsg {
    pub hwnd: u64,
    pub message: u32,
    pub w: u64,
    pub l: u64,
    pub time: u32,
    pub pt: CPoint,
    pub private: u32,
}

/// What these functions need from the process they run in.
pub trait Env {
    /// The `PolyUserCall` system call.
    fn call(&self, req: Request) -> Reply;
    /// Call window procedure `proc_` (0: the default procedure).
    fn call_proc(&self, proc_: u64, hwnd: u32, msg: u32, w: u64, l: u64) -> u64;
    /// The calling thread's id.
    fn tid(&self) -> u32;
    fn set_last_error(&self, e: u32);
    /// `GetTickCount`.
    fn tick(&self) -> u32;
    /// Wait until the thread's queue event is set or `ms` pass (`None`:
    /// no limit).
    fn wait_queue(&self, ms: Option<u64>);
    /// `WaitForMultipleObjects` over `handles` plus, when `queue`, the
    /// queue's event last: the index woken or `None` on a timeout.
    fn wait_handles(&self, handles: &[u64], wait_all: bool, ms: Option<u64>, queue: bool) -> Option<u32>;
    /// Fill `rect` of DC `hdc` with brush `brush` (`FillRect`).
    fn fill_rect(&self, hdc: u64, rect: CRect, brush: u64);
    /// Delete a window's own DC when the window goes.
    fn delete_dc(&self, hdc: u64);
    /// A DC for painting window `hwnd`'s client area, clipped to `clip`
    /// (client coordinates; `None`: not clipped), freed by
    /// [`Env::release_dc`].
    fn paint_dc(&self, hwnd: u32, clip: Option<&[Rect]>) -> u64;
    fn release_dc(&self, hwnd: u32, hdc: u64);
    /// A GDI region of `rects`, freed with [`Env::delete_object`].
    fn make_region(&self, rects: &[Rect]) -> u64;
    fn delete_object(&self, h: u64);
    /// Make `cursor` the calling thread's cursor shape (what DefWindowProc's
    /// `WM_SETCURSOR` does with the class cursor).
    fn set_cursor(&self, _cursor: u64) {}
    /// The rectangles of GDI region `rgn` (`None`: not a region: deleted, or
    /// another kind of object).
    fn region_rects(&self, _rgn: u64) -> Option<Vec<Rect>> {
        None
    }
    /// Replace GDI region `rgn`'s contents with `rects`: its kind
    /// (NULLREGION 1, SIMPLEREGION 2, COMPLEXREGION 3), 0 when `rgn` is not
    /// a region.
    fn set_region_rects(&self, _rgn: u64, _rects: &[Rect]) -> i32 {
        0
    }
}

/// `MAKELPARAM(lo, hi)`.
pub fn makelparam(lo: i32, hi: i32) -> u64 {
    u64::from(lo as u16) | (u64::from(hi as u16) << 16)
}

fn info<E: Env + ?Sized>(env: &E, hwnd: u64) -> Option<WinInfo> {
    match env.call(Request::Window { hwnd }) {
        Reply::Window { info } => Some(info),
        _ => None,
    }
}

/// The session's screen.
pub fn screen<E: Env + ?Sized>(env: &E) -> Display {
    match env.call(Request::Desktop {}) {
        Reply::Desktop { width, height, work, interactive, .. } => Display { width, height, work, interactive },
        _ => Display::SERVICE,
    }
}

/// The desktop window.
pub fn desktop<E: Env + ?Sized>(env: &E) -> u32 {
    match env.call(Request::Desktop {}) {
        Reply::Desktop { desktop, .. } => desktop,
        _ => 0,
    }
}

fn origin_of(i: &WinInfo) -> (i32, i32) {
    (i.window.left - i.window_rel.left, i.window.top - i.window_rel.top)
}

/// The origin and horizontal direction of a window's client coordinates.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct MapOrigin {
    pub x: i32,
    pub y: i32,
    pub rtl: bool,
}

/// Map one point between two client coordinate spaces (`MapWindowPoints`,
/// including the `WS_EX_LAYOUTRTL` direction from `winuser.h`).
pub fn map_point(from: MapOrigin, to: MapOrigin, point: (i32, i32)) -> (i32, i32) {
    let screen_x = if from.rtl { from.x - point.0 } else { from.x + point.0 };
    let x = if to.rtl { to.x - screen_x } else { screen_x - to.x };
    (x, from.y + point.1 - to.y)
}

/// `MapWindowPoints`' packed translation. An RTL source reverses the
/// horizontal translation; the destination's direction is reflected in the
/// mapped point itself.
pub fn map_offset(from: MapOrigin, to: MapOrigin) -> i32 {
    let dx = if from.rtl { to.x - from.x } else { from.x - to.x };
    let dy = from.y - to.y;
    (dx as u16 as u32 | ((dy as u16 as u32) << 16)) as i32
}

/// Map the supplied points and keep a two-point rectangle left-to-right when
/// either coordinate space is RTL.
pub fn map_points(from: MapOrigin, to: MapOrigin, points: &mut [(i32, i32)]) -> i32 {
    for point in points.iter_mut() {
        *point = map_point(from, to, *point);
    }
    if points.len() == 2 && (from.rtl || to.rtl) {
        let left = points[0].0;
        points[0].0 = points[1].0;
        points[1].0 = left;
    }
    map_offset(from, to)
}

/// `ClientToScreen` (`to_screen`) and `ScreenToClient` of one point of
/// window `hwnd` (ClientToScreen.yaml, ScreenToClient.yaml): the client
/// origin's screen position added or taken off; for a `WS_EX_LAYOUTRTL`
/// window x runs leftwards from the client area's right edge. `None` for
/// NULL or a made-up handle (ERROR_INVALID_WINDOW_HANDLE).
pub fn convert_point<E: Env + ?Sized>(env: &E, hwnd: u64, point: (i32, i32), to_screen: bool) -> Option<(i32, i32)> {
    let Some(i) = (hwnd != 0).then(|| info(env, hwnd)).flatten() else {
        env.set_last_error(ERROR_INVALID_WINDOW_HANDLE);
        return None;
    };
    let rtl = i.ex_style & metrics::WS_EX_LAYOUTRTL != 0;
    let own = MapOrigin { x: if rtl { i.client.right } else { i.client.left }, y: i.client.top, rtl };
    Some(if to_screen {
        map_point(own, MapOrigin::default(), point)
    } else {
        map_point(MapOrigin::default(), own, point)
    })
}

fn ptr<T>(v: &mut T) -> u64 {
    v as *mut T as u64
}

// ----- sending

/// Run a message another thread sent to us, and report its result.
pub fn run_incoming<E: Env + ?Sized>(env: &E, inc: &Incoming) {
    let r = env.call_proc(inc.wndproc, inc.hwnd, inc.msg, inc.w, inc.l);
    env.call(Request::SendDone { id: inc.id, result: r });
}

/// `SendMessageW`: a procedure of this thread is called directly; another
/// thread's runs there while we wait (receiving what is sent to us).
pub fn send<E: Env + ?Sized>(env: &E, hwnd: u64, msg: u32, w: u64, l: u64) -> u64 {
    match env.call(Request::SendStart { hwnd, msg, w, l }) {
        Reply::SameThread { wndproc, hwnd, .. } => env.call_proc(wndproc, hwnd, msg, w, l),
        Reply::Started { id } => loop {
            match env.call(Request::SendWait { id }) {
                Reply::Done { result } => return result,
                Reply::Incoming { incoming } => run_incoming(env, &incoming),
                _ => env.wait_queue(None),
            }
        },
        Reply::Error { code } => {
            env.set_last_error(code);
            0
        }
        _ => 0,
    }
}

// ----- creation

/// `CreateWindowExW` (`CreateWindowExW.yaml`).
pub fn create_window<E: Env + ?Sized>(env: &E, mut spec: CreateSpec, name: u64, param: u64) -> u32 {
    let style = spec.style;
    // WS_VISIBLE is the show at the end of the sequence, and so are
    // WS_MINIMIZE / WS_MAXIMIZE: the window is made at its rectangle and
    // put into that state then.
    spec.style &= !(WS_VISIBLE | WS_MINIMIZE | WS_MAXIMIZE);
    let instance = spec.instance;
    let menu = spec.menu;
    let c = match env.call(Request::CreateWindow { spec }) {
        Reply::Created { created } => created,
        Reply::Error { code } => {
            env.set_last_error(code);
            return 0;
        }
        _ => return 0,
    };
    let hwnd = c.hwnd;
    let h = u64::from(hwnd);
    let (mut x, mut y, mut cx, mut cy) = (c.x, c.y, c.cx, c.cy);
    if c.minmax {
        let [ms, mp, mt, xt] = metrics::minmax(style, c.ex_style, &screen(env));
        let mut mmi = MinMaxInfo {
            reserved: CPoint::default(),
            max_size: CPoint { x: ms.0, y: ms.1 },
            max_position: CPoint { x: mp.0, y: mp.1 },
            min_track: CPoint { x: mt.0, y: mt.1 },
            max_track: CPoint { x: xt.0, y: xt.1 },
        };
        send(env, h, WM_GETMINMAXINFO, 0, ptr(&mut mmi));
        if metrics::track_limited(style) {
            cx = cx.max(mmi.min_track.x).min(mmi.max_track.x.max(mmi.min_track.x));
            cy = cy.max(mmi.min_track.y).min(mmi.max_track.y.max(mmi.min_track.y));
        }
    }
    if cx < 0 {
        cx = 0;
    }
    if cy < 0 {
        cy = 0;
    }
    let _ = (&mut x, &mut y);
    let Some(i) = info(env, h) else { return 0 };
    if i.style & WS_CHILD != 0
        && let Some(parent) = info(env, u64::from(i.parent))
        && parent.ex_style & WS_EX_LAYOUTRTL != 0
    {
        x = parent.client.width() - x - cx;
    }
    let win_rel = Rect::new(x, y, x + cx, y + cy);
    env.call(Request::SetRects { hwnd, window_rel: Some(win_rel), client_rel: None });
    let Some(i) = info(env, h) else { return 0 };
    let (ox, oy) = origin_of(&i);
    let mut class_name = c.class_name.clone();
    class_name.push(0);
    let (cs_cx, cs_cy) = if c.default_size {
        (-1, if c.asked_cy == crate::wm::CW_USEDEFAULT { -1 } else { c.asked_cy })
    } else {
        (cx, cy)
    };
    let mut cs = CreateStructW {
        create_params: param,
        instance,
        menu,
        parent: c.cs_parent,
        cy: cs_cy,
        cx: cs_cx,
        y,
        x,
        style,
        name,
        class: class_name.as_ptr() as u64,
        ex_style: c.ex_style,
    };
    if send(env, h, WM_NCCREATE, 0, ptr(&mut cs)) == 0 {
        send(env, h, WM_NCDESTROY, 0, 0);
        env.call(Request::Free { hwnd });
        env.set_last_error(0);
        return 0;
    }
    let mut rect = CRect::from(i.window);
    send(env, h, WM_NCCALCSIZE, 0, ptr(&mut rect));
    let client_rel = Rect::new(rect.left - ox, rect.top - oy, rect.right - ox, rect.bottom - oy);
    env.call(Request::SetRects { hwnd, window_rel: None, client_rel: Some(client_rel) });
    let child = style & WS_CHILD != 0;
    let notify = child && c.ex_style & WS_EX_NOPARENTNOTIFY == 0;
    if send(env, h, WM_CREATE, 0, ptr(&mut cs)) as i64 == -1 {
        if notify {
            send(env, u64::from(i.parent), WM_PARENTNOTIFY, u64::from(WM_DESTROY) | ((menu & 0xFFFF) << 16), h);
        }
        send(env, h, WM_DESTROY, 0, 0);
        send(env, h, WM_NCDESTROY, 0, 0);
        env.call(Request::Free { hwnd });
        env.set_last_error(0);
        return 0;
    }
    let i = match env.call(Request::FinishCreate { hwnd }) {
        Reply::Window { info } => info,
        _ => return 0,
    };
    if !i.size_move_pending {
        send_size_move(env, &i);
    }
    if notify {
        send(env, u64::from(i.parent), WM_PARENTNOTIFY, u64::from(WM_CREATE) | ((menu & 0xFFFF) << 16), h);
    }
    if style & (WS_MINIMIZE | WS_MAXIMIZE) != 0 {
        // Created minimized or maximized: the rectangle asked for is its
        // restored one (MonitorFromWindow.yaml `state`: a window created
        // with WS_MINIMIZE at -3000,200 is judged by that position).
        if style & WS_VISIBLE != 0 {
            if style & WS_MINIMIZE != 0 {
                minimize(env, &i, SW_SHOWMINIMIZED);
            } else {
                maximize(env, &i);
            }
        } else {
            let min = style & WS_MINIMIZE != 0;
            env.call(Request::SetState {
                hwnd,
                visible: None,
                minimized: Some(min),
                maximized: Some(!min),
                normal: Some(i.window_rel),
                size_move_pending: None,
            });
        }
    } else if style & WS_VISIBLE != 0 {
        show(env, &i, ShowKind::Create);
        discharge_size_move(env, hwnd);
    }
    hwnd
}

/// The WM_SIZE and WM_MOVE held back at creation, if they still are
/// (ShowWindow.yaml `hide_size_move`).
fn discharge_size_move<E: Env + ?Sized>(env: &E, hwnd: u32) {
    let Some(now) = info(env, u64::from(hwnd)) else { return };
    if !now.size_move_pending {
        return;
    }
    env.call(Request::SetState {
        hwnd,
        visible: None,
        minimized: None,
        maximized: None,
        normal: None,
        size_move_pending: Some(false),
    });
    send_size_move(env, &now);
}

/// `WM_SIZE` then `WM_MOVE` from the window's current rectangles.
fn send_size_move<E: Env + ?Sized>(env: &E, i: &WinInfo) {
    let h = u64::from(i.hwnd);
    let kind = size_kind(i);
    send(env, h, WM_SIZE, kind, makelparam(i.client.width(), i.client.height()));
    send(env, h, WM_MOVE, 0, makelparam(i.client_rel.left, i.client_rel.top));
}

fn size_kind(i: &WinInfo) -> u64 {
    if i.minimized {
        SIZE_MINIMIZED
    } else if i.maximized {
        SIZE_MAXIMIZED
    } else {
        SIZE_RESTORED
    }
}

// ----- destruction

/// `DestroyWindow` (`DestroyWindow.yaml`).
pub fn destroy_window<E: Env + ?Sized>(env: &E, hwnd: u64) -> bool {
    let Some(i) = info(env, hwnd) else {
        env.set_last_error(ERROR_INVALID_WINDOW_HANDLE);
        return false;
    };
    if i.desktop || i.tid != env.tid() {
        env.set_last_error(ERROR_ACCESS_DENIED);
        return false;
    }
    destroy_tree(env, i.hwnd, true);
    true
}

fn destroy_tree<E: Env + ?Sized>(env: &E, hwnd: u32, alone: bool) {
    let h = u64::from(hwnd);
    let Some(i) = info(env, h) else { return };
    if alone && i.style & WS_CHILD != 0 && i.ex_style & WS_EX_NOPARENTNOTIFY == 0 {
        send(env, u64::from(i.parent), WM_PARENTNOTIFY, u64::from(WM_DESTROY) | ((i.id & 0xFFFF) << 16), h);
    }
    if i.style & WS_VISIBLE != 0 {
        if i.style & WS_CHILD != 0 {
            send(env, h, WM_SHOWWINDOW, 0, 0);
        }
        focus::DESTROYING.store(hwnd, core::sync::atomic::Ordering::Relaxed);
        set_window_pos(env, h, 0, 0, 0, 0, 0, SWP_HIDEWINDOW | SWP_NOACTIVATE | SWP_NOZORDER | SWP_NOMOVE | SWP_NOSIZE);
        focus::DESTROYING.store(0, core::sync::atomic::Ordering::Relaxed);
    } else if i.style & WS_CHILD == 0 && focus::input(env).active == hwnd {
        // A hidden window can be the active one (session 0): destroying it
        // moves the activation as for a visible one.
        focus::DESTROYING.store(hwnd, core::sync::atomic::Ordering::Relaxed);
        focus::after_hide(env, &i, true);
        focus::DESTROYING.store(0, core::sync::atomic::Ordering::Relaxed);
    }
    // The capture of the window or of one below it ends without a message
    // (SetCapture.yaml `destroy`; a deactivation above cancelled it already).
    focus::drop_capture_within(env, hwnd);
    if let Reply::Relatives { owned, .. } = env.call(Request::Relatives { hwnd }) {
        for o in owned {
            destroy_tree(env, o, false);
        }
    }
    send_destroy(env, hwnd);
    nc_destroy(env, hwnd);
}

/// WM_DESTROY to `hwnd`, then to its children. A window that has the
/// focus hands it to its parent before its own WM_DESTROY, and again after
/// its children's if one of them passed it up (GetFocus.yaml `destroy_child`).
fn send_destroy<E: Env + ?Sized>(env: &E, hwnd: u32) {
    focus::focus_to_parent(env, hwnd);
    send(env, u64::from(hwnd), WM_DESTROY, 0, 0);
    if let Reply::Relatives { children, .. } = env.call(Request::Relatives { hwnd }) {
        for c in children {
            send_destroy(env, c);
        }
    }
    focus::focus_to_parent(env, hwnd);
}

fn nc_destroy<E: Env + ?Sized>(env: &E, hwnd: u32) {
    if let Reply::Relatives { children, .. } = env.call(Request::Relatives { hwnd }) {
        for c in children {
            nc_destroy(env, c);
        }
    }
    send(env, u64::from(hwnd), WM_NCDESTROY, 0, 0);
    if let Some(i) = info(env, u64::from(hwnd))
        && i.own_dc != 0
    {
        env.delete_dc(i.own_dc);
    }
    env.call(Request::Free { hwnd });
}

// ----- showing and positions

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum ShowKind {
    /// `WS_VISIBLE` at creation.
    Create,
    Command(i32),
}

/// What a `swp` call does besides what its flags say.
#[derive(Clone, Copy, Debug, Default)]
struct Opts {
    /// Bits for the flags of `WM_WINDOWPOSCHANGED` only.
    extra: u32,
    /// The call activates the window, between `WM_NCCALCSIZE` and
    /// `WM_WINDOWPOSCHANGED`.
    activate: bool,
    /// The z order stays as it is whatever the flags ask (a child's state
    /// changes: ShowWindow.yaml `child_min_max`).
    keep_z: bool,
    /// A child changes its state: `WM_CHILDACTIVATE` where a top-level
    /// window would be activated.
    child_activate: bool,
    /// The rectangle the window gets if its procedure leaves
    /// `WM_WINDOWPOSCHANGING`'s as it is (maximizing).
    fit: Option<Rect>,
}

/// `SWP_NOCOPYBITS` (winuser.h).
const SWP_NOCOPYBITS: u32 = 0x0100;
/// `HWND_BOTTOM` (winuser.h).
const HWND_BOTTOM: u64 = 1;

/// `ShowWindow` (`ShowWindow.yaml`): the previous `WS_VISIBLE` state.
///
/// A command for the state the window is in sends nothing (`same_state`),
/// but `SW_SHOWNA`. The commands that minimize, maximize or restore send
/// no `WM_SHOWWINDOW`, also when they show a hidden window. `WM_SIZE` and
/// `WM_MOVE` held back at creation come after the first call that sends
/// `WM_SHOWWINDOW` (`hide_size_move`).
pub fn show_window<E: Env + ?Sized>(env: &E, hwnd: u64, cmd: i32) -> bool {
    let Some(i) = info(env, hwnd) else {
        env.set_last_error(ERROR_INVALID_WINDOW_HANDLE);
        return false;
    };
    let was = i.style & WS_VISIBLE != 0;
    // Whether the call sent WM_SHOWWINDOW (the held-back WM_SIZE / WM_MOVE follow it).
    let mut shown_message = false;
    match cmd {
        SW_HIDE => {
            if was {
                hide(env, &i);
                shown_message = true;
            }
        }
        SW_FORCEMINIMIZE => {
            // On a visible window of the calling thread: WS_VISIBLE goes
            // without a message, and the window is not minimized (`commands`).
            if was && !i.minimized && i.tid == env.tid() {
                env.call(Request::SetState {
                    hwnd: i.hwnd,
                    visible: Some(false),
                    minimized: None,
                    maximized: None,
                    normal: None,
                    size_move_pending: None,
                });
            }
        }
        SW_SHOWMINIMIZED | SW_MINIMIZE | SW_SHOWMINNOACTIVE => {
            if !i.minimized || !was {
                minimize(env, &i, cmd);
            }
        }
        SW_SHOWMAXIMIZED => {
            if !i.maximized || i.minimized || !was {
                maximize(env, &i);
            }
        }
        SW_SHOWNA => {
            if was {
                // Also on a visible window: the message, and the window is
                // raised if it was not on top (`same_state`, `z_order`).
                send(env, hwnd, WM_SHOWWINDOW, 1, 0);
                let flags = if i.style & WS_CHILD != 0 {
                    SWP_SHOWWINDOW | SWP_NOACTIVATE | SWP_NOZORDER | SWP_NOMOVE | SWP_NOSIZE
                } else {
                    SWP_SHOWWINDOW | SWP_NOACTIVATE | SWP_NOMOVE | SWP_NOSIZE
                };
                swp(env, i.hwnd, HWND_TOP, 0, 0, 0, 0, flags, Opts::default());
            } else {
                show(env, &i, ShowKind::Command(cmd));
            }
            shown_message = true;
        }
        SW_SHOW => {
            // SW_SHOW does not restore a minimized window.
            if !was {
                show(env, &i, ShowKind::Command(cmd));
                shown_message = true;
            }
        }
        _ => {
            // SW_SHOWNORMAL, SW_RESTORE, SW_SHOWDEFAULT, SW_SHOWNOACTIVATE.
            if i.minimized || i.maximized {
                restore(env, &i, cmd);
            } else if !was {
                show(env, &i, ShowKind::Command(cmd));
                shown_message = true;
            }
        }
    }
    if shown_message && i.size_move_pending {
        discharge_size_move(env, i.hwnd);
    }
    was
}

/// Whether showing top-level window `i` reports the "shown" bit 0x10000000
/// in `WM_WINDOWPOSCHANGED` on the interactive desktop: every show of a
/// hidden window but a tool window's or a `WS_EX_NOACTIVATE` window's
/// without `WS_EX_APPWINDOW` (GetActiveWindow.yaml `minimized_position`).
fn shown_bit<E: Env + ?Sized>(env: &E, i: &WinInfo) -> u32 {
    let plain = i.ex_style & WS_EX_APPWINDOW != 0 || i.ex_style & (WS_EX_TOOLWINDOW | WS_EX_NOACTIVATE) == 0;
    if screen(env).interactive && i.style & WS_CHILD == 0 && i.style & WS_VISIBLE == 0 && plain { 0x1000_0000 } else { 0 }
}

fn show<E: Env + ?Sized>(env: &E, i: &WinInfo, kind: ShowKind) {
    let h = u64::from(i.hwnd);
    send(env, h, WM_SHOWWINDOW, 1, 0);
    if i.style & WS_CHILD != 0 {
        let parent_visible = info(env, u64::from(i.parent)).is_some_and(|p| p.visible);
        if parent_visible {
            set_window_pos(
                env,
                h,
                0,
                0,
                0,
                0,
                0,
                SWP_SHOWWINDOW | SWP_NOACTIVATE | SWP_NOZORDER | SWP_NOMOVE | SWP_NOSIZE,
            );
        } else {
            env.call(Request::SetState {
                hwnd: i.hwnd,
                visible: Some(true),
                minimized: None,
                maximized: None,
                normal: None,
                size_move_pending: None,
            });
        }
        return;
    }
    let interactive = screen(env).interactive;
    let mut flags = SWP_SHOWWINDOW | SWP_NOMOVE | SWP_NOSIZE;
    let mut extra = shown_bit(env, i);
    match kind {
        // Creating a visible window activates it, also a WS_EX_NOACTIVATE
        // one (which reports no "shown" bit then).
        ShowKind::Create => {
            if i.ex_style & WS_EX_NOACTIVATE != 0 {
                extra = 0;
            }
        }
        ShowKind::Command(SW_SHOWNA) => flags |= SWP_NOACTIVATE,
        ShowKind::Command(SW_SHOWNOACTIVATE) => {
            flags |= SWP_NOACTIVATE | SWP_NOZORDER;
            // Session 0 reports the bit for an owned window (sw_toplevel).
            if !interactive && i.owner != 0 {
                extra = 0x1000_0000;
            }
        }
        // WS_EX_NOACTIVATE and tool windows are activated by a show all
        // the same (GetActiveWindow.yaml `show_activates`).
        ShowKind::Command(_) => {}
    }
    if flags & SWP_NOZORDER == 0
        && let Reply::Relatives { owned, .. } = env.call(Request::Relatives { hwnd: i.hwnd })
    {
        // Owned windows come along above their owner.
        for o in owned {
            let mut wp = WindowPos {
                hwnd: u64::from(o),
                flags: SWP_NOACTIVATE | SWP_NOMOVE | SWP_NOSIZE,
                ..WindowPos::default()
            };
            send(env, u64::from(o), WM_WINDOWPOSCHANGING, 0, ptr(&mut wp));
        }
    }
    let activate = flags & SWP_NOACTIVATE == 0;
    swp(env, i.hwnd, HWND_TOP, 0, 0, 0, 0, flags, Opts { extra, activate, ..Opts::default() });
}

fn hide<E: Env + ?Sized>(env: &E, i: &WinInfo) {
    let h = u64::from(i.hwnd);
    send(env, h, WM_SHOWWINDOW, 0, 0);
    // Hiding the thread's active window is not SWP_NOACTIVATE / SWP_NOZORDER:
    // the call raises the window while the activation is handed on
    // (ShowWindow.yaml `hide`, GetActiveWindow.yaml `hide_flags`).
    let active = i.style & WS_CHILD == 0 && focus::input(env).active == i.hwnd;
    let flags = if active {
        SWP_HIDEWINDOW | SWP_NOMOVE | SWP_NOSIZE
    } else {
        SWP_HIDEWINDOW | SWP_NOACTIVATE | SWP_NOZORDER | SWP_NOMOVE | SWP_NOSIZE
    };
    set_window_pos(env, h, 0, 0, 0, 0, 0, flags);
}

/// The size and position a window's procedure answers to `WM_GETMINMAXINFO`.
fn ask_minmax<E: Env + ?Sized>(env: &E, i: &WinInfo) -> MinMaxInfo {
    let display = screen(env);
    let [ms, mp, mt, xt] = metrics::minmax(i.style, i.ex_style, &display);
    let mut mmi = MinMaxInfo {
        reserved: CPoint::default(),
        max_size: CPoint { x: ms.0, y: ms.1 },
        max_position: CPoint { x: mp.0, y: mp.1 },
        min_track: CPoint { x: mt.0, y: mt.1 },
        max_track: CPoint { x: xt.0, y: xt.1 },
    };
    send(env, u64::from(i.hwnd), WM_GETMINMAXINFO, 0, ptr(&mut mmi));
    mmi
}

/// The three minimizing commands (ShowWindow.yaml `minimize`,
/// `show_minimized_from_hidden`, `z_order`; GetActiveWindow.yaml
/// `minimize_active`, `minimized_position`; GetForegroundWindow.yaml
/// `minimize_foreground`).
///
/// Session 0: every one of them makes the window the thread's active one,
/// but SW_MINIMIZE / SW_SHOWMINNOACTIVE of the visible active window, which
/// put it at the bottom of the z order. The interactive desktop:
/// SW_SHOWMINIMIZED activates the window; the others do not (a window that
/// is not the active one goes below it), and SW_MINIMIZE of the active
/// window takes its focus away first and hands the activation on after.
fn minimize<E: Env + ?Sized>(env: &E, i: &WinInfo, cmd: i32) {
    let display = screen(env);
    let interactive = display.interactive;
    let child = i.style & WS_CHILD != 0;
    let hidden = i.style & WS_VISIBLE == 0;
    let st = focus::input(env);
    let was_active = !child && st.active == i.hwnd;
    let (w, h) = MINIMIZED;
    let (x, y) = if child {
        let ph = info(env, u64::from(i.parent)).map_or(0, |p| p.client.height());
        (0, ph - h)
    } else {
        metrics::minimized_origin(i.ex_style, i.owner != 0, &display)
    };
    if i.minimized {
        // A hidden minimized window shown minimized again.
        let flags = SWP_SHOWWINDOW | SWP_NOACTIVATE | SWP_NOMOVE | SWP_NOSIZE;
        swp(env, i.hwnd, HWND_TOP, 0, 0, 0, 0, flags, Opts { extra: shown_bit(env, i), ..Opts::default() });
        return;
    }
    let activating = cmd == SW_SHOWMINIMIZED;
    if interactive && was_active && st.focus != 0 {
        send(env, u64::from(st.focus), focus::WM_KILLFOCUS, 0, 0);
        env.call(Request::SetInput { active: 0, focus: 0, capture: 0, which: crate::wm::input::SET_FOCUS });
    }
    // Who takes the activation from a minimized active window is settled
    // by the z order before it moves.
    let successor =
        if interactive && was_active && cmd == SW_MINIMIZE { focus::successor(env, i, true) } else { None };
    let extra = shown_bit(env, i);
    env.call(Request::SetState {
        hwnd: i.hwnd,
        visible: None,
        minimized: Some(true),
        maximized: Some(false),
        normal: if i.maximized { None } else { Some(i.window_rel) },
        size_move_pending: None,
    });
    let base = SWP_STATECHANGED | SWP_NOCOPYBITS | SWP_SHOWWINDOW | SWP_FRAMECHANGED;
    if child {
        swp(
            env,
            i.hwnd,
            HWND_TOP,
            x,
            y,
            w,
            h,
            base | SWP_NOZORDER,
            Opts { child_activate: true, keep_z: true, ..Opts::default() },
        );
        return;
    }
    // Where it goes in the z order, and whether the call activates it.
    let (flags, after, activate) = if interactive {
        if activating {
            (base, HWND_TOP, !was_active)
        } else {
            (base | SWP_NOACTIVATE, HWND_BOTTOM, false)
        }
    } else if was_active && !hidden && !activating {
        (base, HWND_BOTTOM, false)
    } else {
        (base, HWND_TOP, !was_active)
    };
    swp(env, i.hwnd, after, x, y, w, h, flags, Opts { extra, activate, ..Opts::default() });
    if interactive && cmd == SW_MINIMIZE {
        if was_active {
            focus::hand_on(env, i, successor, true);
        } else {
            // "Minimizes the specified window and activates the next
            // top-level window" (Learn) also when the minimized window was
            // not the active one: the window that was active before the
            // active one takes the activation if it can (SetCapture.yaml
            // `hide`: observed, "not understood beyond the observation").
            focus::activate_previous(env, i.hwnd);
        }
    }
}

fn maximize<E: Env + ?Sized>(env: &E, i: &WinInfo) {
    let display = screen(env);
    let child = i.style & WS_CHILD != 0;
    let hidden = i.style & WS_VISIBLE == 0;
    if i.maximized && !i.minimized {
        // A hidden maximized window shown again.
        let flags = SWP_SHOWWINDOW | SWP_NOMOVE | SWP_NOSIZE;
        swp(
            env,
            i.hwnd,
            HWND_TOP,
            0,
            0,
            0,
            0,
            flags,
            Opts { extra: shown_bit(env, i), activate: !child, ..Opts::default() },
        );
        return;
    }
    // WM_GETMINMAXINFO first: the maximized size and position are the
    // procedure's answer.
    let mmi = ask_minmax(env, i);
    let r = Rect::new(
        mmi.max_position.x,
        mmi.max_position.y,
        mmi.max_position.x + mmi.max_size.x,
        mmi.max_position.y + mmi.max_size.y,
    );
    let fit = (!child).then(|| {
        metrics::maximized_rect(
            i.style,
            i.ex_style,
            &display,
            (mmi.max_size.x, mmi.max_size.y),
            (mmi.max_position.x, mmi.max_position.y),
        )
    });
    let extra = shown_bit(env, i);
    env.call(Request::SetState {
        hwnd: i.hwnd,
        visible: None,
        minimized: Some(false),
        maximized: Some(true),
        normal: if i.minimized || i.maximized { None } else { Some(i.window_rel) },
        size_move_pending: None,
    });
    let mut flags = SWP_STATECHANGED | SWP_FRAMECHANGED;
    if hidden {
        flags |= SWP_SHOWWINDOW;
    }
    if i.minimized {
        // Out of the icon its pixels are not kept.
        flags |= SWP_NOCOPYBITS;
    }
    let opts = if child {
        Opts { child_activate: true, keep_z: true, ..Opts::default() }
    } else {
        Opts { extra, activate: focus::input(env).active != i.hwnd, fit, ..Opts::default() }
    };
    swp(env, i.hwnd, HWND_TOP, r.left, r.top, r.width(), r.height(), flags, opts);
    after_restore_activation(env, i, opts.activate);
}

/// A minimized window that a command restored or maximized and activated
/// gets WM_ACTIVATE once more after its WM_SIZE, naming no other window
/// (GetActiveWindow.yaml `show_activates`, interactive).
fn after_restore_activation<E: Env + ?Sized>(env: &E, i: &WinInfo, activated: bool) {
    if activated && i.minimized && screen(env).interactive && focus::input(env).active == i.hwnd {
        send(env, u64::from(i.hwnd), focus::WM_ACTIVATE, focus::WA_ACTIVE, 0);
    }
}

/// SW_SHOWNORMAL, SW_RESTORE, SW_SHOWDEFAULT and SW_SHOWNOACTIVATE on a
/// minimized or maximized window (ShowWindow.yaml `state_changes`,
/// `restore`, `show_after_hidden_maximized`): back to the normal rectangle,
/// or, for a window that was maximized when it was minimized, to the
/// maximized one.
fn restore<E: Env + ?Sized>(env: &E, i: &WinInfo, cmd: i32) {
    let display = screen(env);
    let interactive = display.interactive;
    let child = i.style & WS_CHILD != 0;
    let hidden = i.style & WS_VISIBLE == 0;
    let to_max = i.minimized && i.restore_max;
    let (r, fit) = if to_max {
        let mmi = ask_minmax(env, i);
        let raw = Rect::new(
            mmi.max_position.x,
            mmi.max_position.y,
            mmi.max_position.x + mmi.max_size.x,
            mmi.max_position.y + mmi.max_size.y,
        );
        let fit = metrics::maximized_rect(
            i.style,
            i.ex_style,
            &display,
            (mmi.max_size.x, mmi.max_size.y),
            (mmi.max_position.x, mmi.max_position.y),
        );
        (raw, (!child).then_some(fit))
    } else {
        (normal_rect(env, i), None)
    };
    let mut extra = shown_bit(env, i);
    // Observed in session 0: showing a window that was hidden while
    // maximized reports 0x40000000.
    if hidden && i.maximized && !i.minimized && !interactive {
        extra |= 0x4000_0000;
    }
    env.call(Request::SetState {
        hwnd: i.hwnd,
        visible: None,
        minimized: Some(false),
        maximized: Some(to_max),
        normal: None,
        size_move_pending: None,
    });
    let mut flags = SWP_STATECHANGED | SWP_FRAMECHANGED;
    if i.minimized {
        flags |= SWP_NOCOPYBITS;
    }
    if hidden {
        flags |= SWP_SHOWWINDOW;
    }
    let no_activate = cmd == SW_SHOWNOACTIVATE;
    if no_activate && interactive {
        flags |= SWP_NOACTIVATE;
    }
    let opts = if child {
        Opts { child_activate: true, keep_z: true, ..Opts::default() }
    } else {
        Opts { extra, activate: !no_activate && focus::input(env).active != i.hwnd, fit, ..Opts::default() }
    };
    swp(env, i.hwnd, HWND_TOP, r.left, r.top, r.width(), r.height(), flags, opts);
    after_restore_activation(env, i, opts.activate);
}

fn normal_rect<E: Env + ?Sized>(env: &E, i: &WinInfo) -> Rect {
    match env.call(Request::GetLong { hwnd: u64::from(i.hwnd), index: NORMAL_RECT_INDEX, size: 8 }) {
        Reply::Value { value } => {
            let x = value as u32 as i16 as i32;
            let y = (value >> 16) as u16 as i16 as i32;
            let w = (value >> 32) as u16 as i32;
            let h = (value >> 48) as u16 as i32;
            Rect::new(x, y, x + w, y + h)
        }
        _ => i.window_rel,
    }
}

/// A private index of `GetLong`: the normal rectangle of a minimized or
/// maximized window, packed (x, y, width, height as 16-bit values).
pub const NORMAL_RECT_INDEX: i32 = -0x7000;

/// `SetWindowPos` (`SetWindowPos.yaml`, `MoveWindow.yaml`).
#[allow(clippy::too_many_arguments)]
pub fn set_window_pos<E: Env + ?Sized>(
    env: &E,
    hwnd: u64,
    after: u64,
    x: i32,
    y: i32,
    cx: i32,
    cy: i32,
    flags: u32,
) -> bool {
    let Some(i) = info(env, hwnd) else {
        env.set_last_error(ERROR_INVALID_WINDOW_HANDLE);
        return false;
    };
    swp(env, i.hwnd, after, x, y, cx, cy, flags, Opts::default())
}

/// The work of `SetWindowPos` and of every `ShowWindow` command.
///
/// `WM_WINDOWPOSCHANGING` carries the caller's flags. A call without
/// `SWP_NOZORDER` asks for the window's place in the z order (the top,
/// unless `after` says otherwise). `WM_WINDOWPOSCHANGED` has the same flags,
/// less `SWP_SHOWWINDOW` for a window that was visible, plus `SWP_NOZORDER`
/// when the window was already where the call puts it, `SWP_NOMOVE` /
/// `SWP_NOSIZE` likewise, and 0x800 / 0x1000 while the client area keeps
/// its size / position; `WM_NCCALCSIZE`'s `WINDOWPOS` has those flags with
/// both of the last two (ShowWindow.yaml `z_order`, `state_changes`).
#[allow(clippy::too_many_arguments)]
fn swp<E: Env + ?Sized>(
    env: &E,
    hwnd: u32,
    after: u64,
    x: i32,
    y: i32,
    cx: i32,
    cy: i32,
    flags: u32,
    opts: Opts,
) -> bool {
    let h = u64::from(hwnd);
    let Some(i) = info(env, h) else { return false };
    let mut wp = WindowPos { hwnd: h, insert_after: after, x, y, cx: cx.max(0), cy: cy.max(0), flags };
    if flags & SWP_NOSENDCHANGING == 0 {
        send(env, h, WM_WINDOWPOSCHANGING, 0, ptr(&mut wp));
    }
    // A maximized window is fitted to the work area after the message,
    // which showed the size and position of WM_GETMINMAXINFO (observed:
    // WM_WINDOWPOSCHANGING -8,-8 1936x1096, WM_WINDOWPOSCHANGED -8,16 1936x1072).
    if let Some(r) = opts.fit
        && (wp.x, wp.y, wp.cx, wp.cy) == (x, y, cx.max(0), cy.max(0))
    {
        (wp.x, wp.y, wp.cx, wp.cy) = (r.left, r.top, r.width(), r.height());
    }
    let interactive = screen(env).interactive;
    let top = !i.desktop && i.style & WS_CHILD == 0;
    let f = wp.flags;
    // Whether the window moves in the z order: asked before an activation
    // below brings it to the top, which is a move all the same.
    let zchanged = f & SWP_NOZORDER == 0
        && !opts.keep_z
        && matches!(
            env.call(Request::ZOrder { hwnd, after: wp.insert_after, apply: false }),
            Reply::Bool { value: true }
        );
    let styled_visible = i.style & WS_VISIBLE != 0;
    let hide = f & SWP_HIDEWINDOW != 0 && styled_visible;
    let was_active = top && focus::input(env).active == hwnd;
    if hide && f & SWP_NOACTIVATE == 0 && interactive && i.owner != 0 && was_active {
        // The owner is the next window to activate: it is asked first.
        let mut o = WindowPos {
            hwnd: u64::from(i.owner),
            flags: SWP_NOACTIVATE | SWP_NOMOVE | SWP_NOSIZE,
            ..WindowPos::default()
        };
        send(env, u64::from(i.owner), WM_WINDOWPOSCHANGING, 0, ptr(&mut o));
    }
    let cur = i.window_rel;
    let (nx, ny) = if f & SWP_NOMOVE != 0 { (cur.left, cur.top) } else { (wp.x, wp.y) };
    let (mut ncx, mut ncy) =
        if f & SWP_NOSIZE != 0 { (cur.width(), cur.height()) } else { (wp.cx.max(0), wp.cy.max(0)) };
    if f & SWP_NOSIZE == 0 && top && metrics::track_limited(i.style) {
        let mmi = ask_minmax(env, &i);
        if f & SWP_STATECHANGED == 0 {
            ncx = ncx.max(mmi.min_track.x).min(mmi.max_track.x.max(mmi.min_track.x));
            ncy = ncy.max(mmi.min_track.y).min(mmi.max_track.y.max(mmi.min_track.y));
        }
    }
    let moved = (nx, ny) != (cur.left, cur.top);
    let sized = (ncx, ncy) != (cur.width(), cur.height());
    let show = f & SWP_SHOWWINDOW != 0 && !styled_visible;
    let frame = f & SWP_FRAMECHANGED != 0;
    if !(moved || sized || zchanged || show || hide || frame) {
        return true;
    }
    let mut eff = f;
    if !moved {
        eff |= SWP_NOMOVE;
    }
    if !sized {
        eff |= SWP_NOSIZE;
    }
    if !zchanged {
        eff |= SWP_NOZORDER;
    }
    // SWP_SHOWWINDOW is reported only when the call shows the window.
    if styled_visible {
        eff &= !SWP_SHOWWINDOW;
    }
    // The interactive desktop marks every hide (and destroy) of a
    // top-level window that has a taskbar button: not an owned one, a tool
    // window or a WS_EX_NOACTIVATE one (gaw_owner, GetForegroundWindow.yaml
    // `handover_same_thread`).
    let mut extra = opts.extra;
    if hide && interactive && top && metrics::has_taskbar_button(i.ex_style, i.owner != 0) {
        extra |= 0x2000_0000;
    }
    let parent_visible =
        i.desktop || i.parent == 0 || info(env, u64::from(i.parent)).is_some_and(|p| p.visible || p.desktop);
    let vis_before = i.visible;
    let vis_after = if show {
        parent_visible
    } else if hide {
        false
    } else {
        vis_before
    };
    if !vis_before && !vis_after {
        eff |= SWP_NOREDRAW;
    }
    let new_win = Rect::new(nx, ny, nx + ncx, ny + ncy);
    let old_client = i.client_rel;
    let new_client = if sized || frame {
        let mut calc = WindowPos {
            hwnd: h,
            insert_after: wp.insert_after,
            x: nx,
            y: ny,
            cx: ncx,
            cy: ncy,
            flags: eff | SWP_NOCLIENTSIZE | SWP_NOCLIENTMOVE,
        };
        let mut params = NcCalcSizeParams {
            rgrc: [CRect::from(new_win), CRect::from(cur), CRect::from(old_client)],
            pos: &mut calc,
        };
        send(env, h, WM_NCCALCSIZE, 1, ptr(&mut params));
        Rect::from(params.rgrc[0])
    } else {
        let (dx, dy) = (nx - cur.left, ny - cur.top);
        Rect::new(old_client.left + dx, old_client.top + dy, old_client.right + dx, old_client.bottom + dy)
    };
    // A minimized window's client area is empty at its corner
    // (ShowWindow.yaml: WM_MOVE 0,740, client 0x0).
    let new_client = if i.minimized { Rect::new(nx, ny, nx, ny) } else { new_client };
    let client_moved = (new_client.left, new_client.top) != (old_client.left, old_client.top);
    let client_sized = (new_client.width(), new_client.height()) != (old_client.width(), old_client.height());
    let mut changed = eff | extra;
    if !client_sized {
        changed |= SWP_NOCLIENTSIZE;
    }
    if !client_moved {
        changed |= SWP_NOCLIENTMOVE;
    }
    // The activation, inside the call.
    if opts.child_activate {
        send(env, h, WM_CHILDACTIVATE, 0, 0);
    } else if opts.activate && top {
        if !interactive {
            focus::activate_local(env, hwnd, true);
        } else {
            if i.owner != 0 {
                let mut o = WindowPos {
                    hwnd: u64::from(i.owner),
                    flags: SWP_NOACTIVATE | SWP_NOMOVE | SWP_NOSIZE,
                    ..WindowPos::default()
                };
                send(env, u64::from(i.owner), WM_WINDOWPOSCHANGING, 0, ptr(&mut o));
            }
            // The window takes the foreground from another thread only if
            // its thread may; a WS_EX_NOACTIVATE window never takes it from
            // another thread of its process (gfw_xthread). Without it the
            // window is the active one of its thread all the same.
            let stay_back = !focus::may_take_foreground(env)
                || (i.ex_style & WS_EX_NOACTIVATE != 0 && focus::in_background(env, i.pid));
            focus::activate_as(env, hwnd, true, true, stay_back);
        }
    }
    env.call(Request::SetRects { hwnd, window_rel: Some(new_win), client_rel: Some(new_client) });
    if f & SWP_NOZORDER == 0 && !opts.keep_z {
        env.call(Request::ZOrder { hwnd, after: wp.insert_after, apply: true });
    }
    if show || hide {
        env.call(Request::SetState {
            hwnd,
            visible: Some(show),
            minimized: None,
            maximized: None,
            normal: None,
            size_move_pending: None,
        });
    }
    // An interactive session paints (slice B): what a shown window, a
    // moved, resized or hidden child, or a grown window leaves to repaint,
    // before WM_WINDOWPOSCHANGED (UpdateWindow.yaml, GetUpdateRgn.yaml).
    if let Some(now) = info(env, h) {
        let child = now.style & WS_CHILD != 0;
        if child && let Some(p) = info(env, u64::from(now.parent)) {
            if hide && vis_before {
                paint::expose_parent(env, &p, &[cur]);
            } else if vis_before && vis_after && (moved || sized) {
                let left = crate::rgn::Rgn::rect(cur).subtract(&crate::rgn::Rgn::rect(new_win));
                paint::expose_parent(env, &p, left.rects());
            }
        }
        if show && vis_after {
            paint::shown_paint(env, &now);
        } else if vis_before && vis_after && f & SWP_STATECHANGED != 0 && now.minimized {
            paint::minimized_paint(env, &now);
        } else if vis_before && vis_after && f & SWP_STATECHANGED != 0 && wp.flags & SWP_NOCOPYBITS != 0 {
            paint::restored_paint(env, &now);
        } else if vis_before && vis_after && sized {
            paint::resized_paint(env, hwnd, old_client, new_client);
        } else if vis_before && vis_after && frame && !moved {
            paint::frame_changed_paint(env, &now);
        }
        if interactive && vis_after && !child && (moved || sized) {
            paint::moved_paint(env, &now, cur, f & SWP_NOCOPYBITS != 0 && f & SWP_STATECHANGED == 0);
        }
    }
    let mut done = WindowPos { hwnd: h, insert_after: wp.insert_after, x: nx, y: ny, cx: ncx, cy: ncy, flags: changed };
    send(env, h, WM_WINDOWPOSCHANGED, 0, ptr(&mut done));
    if hide && vis_before && top {
        focus::after_hide(env, &i, was_active && f & SWP_NOACTIVATE == 0);
    } else if hide && vis_before && interactive {
        focus::child_hidden(env, &i);
    }
    true
}

/// `SetParent`: the old parent (`GetParent.yaml`).
pub fn set_parent<E: Env + ?Sized>(env: &E, hwnd: u64, parent: u64) -> u64 {
    let Some(i) = info(env, hwnd) else {
        env.set_last_error(ERROR_INVALID_WINDOW_HANDLE);
        return 0;
    };
    let old = match env.call(Request::SetParent { hwnd: i.hwnd, parent }) {
        Reply::Handle { hwnd } => hwnd,
        Reply::Error { code } => {
            env.set_last_error(code);
            return 0;
        }
        _ => return 0,
    };
    let mut wp = WindowPos { hwnd, flags: SWP_NOSIZE | SWP_NOZORDER, ..WindowPos::default() };
    send(env, hwnd, WM_WINDOWPOSCHANGING, 0, ptr(&mut wp));
    send(env, hwnd, WM_CHILDACTIVATE, 0, 0);
    u64::from(old)
}

// ----- the default window procedure

/// `DefWindowProcW` (`DefWindowProcW.yaml`).
pub fn def_window_proc<E: Env + ?Sized>(env: &E, hwnd: u64, msg: u32, w: u64, l: u64) -> u64 {
    match msg {
        WM_NCCREATE => {
            if l != 0 {
                // SAFETY: lParam is a CREATESTRUCTW for WM_NCCREATE.
                let cs = unsafe { &*(l as *const CreateStructW) };
                if cs.name != 0 && cs.name >> 16 != 0 {
                    // SAFETY: a NUL-terminated string the creator passed.
                    let text = unsafe { read_wstr(cs.name) };
                    env.call(Request::SetText { hwnd: hwnd as u32, text });
                }
            }
            1
        }
        0x00A0 => {
            // WM_NCMOUSEMOVE: DefWindowProc tracks the non-client area
            // until the cursor leaves it (WM_NCMOUSELEAVE).
            input::NC_TRACK.store(hwnd as u32, core::sync::atomic::Ordering::Relaxed);
            0
        }
        WM_NCCALCSIZE => {
            let Some(i) = info(env, hwnd) else { return 0 };
            if l != 0 {
                // SAFETY: lParam starts with a RECT (NCCALCSIZE_PARAMS for TRUE).
                let r = unsafe { &mut *(l as *mut CRect) };
                *r = CRect::from(metrics::default_client(Rect::from(*r), i.style, i.ex_style, false));
            }
            0
        }
        WM_WINDOWPOSCHANGED => {
            if l == 0 {
                return 0;
            }
            // SAFETY: lParam is a WINDOWPOS.
            let flags = unsafe { (*(l as *const WindowPos)).flags };
            let Some(i) = info(env, hwnd) else { return 0 };
            if flags & SWP_NOCLIENTMOVE == 0 {
                send(env, hwnd, WM_MOVE, 0, makelparam(i.client_rel.left, i.client_rel.top));
            }
            if flags & SWP_NOCLIENTSIZE == 0 {
                send(env, hwnd, WM_SIZE, size_kind(&i), makelparam(i.client.width(), i.client.height()));
            }
            0
        }
        WM_SETTEXT => {
            // SAFETY: lParam is NULL or a NUL-terminated string.
            let text = if l == 0 { Vec::new() } else { unsafe { read_wstr(l) } };
            env.call(Request::SetText { hwnd: hwnd as u32, text });
            1
        }
        WM_GETTEXT => {
            if w == 0 || l == 0 {
                return 0;
            }
            let text = match env.call(Request::GetText { hwnd }) {
                Reply::Text { text } => text,
                _ => Vec::new(),
            };
            let n = text.len().min(w as usize - 1);
            // SAFETY: lParam is a buffer of wParam characters.
            unsafe {
                core::ptr::copy_nonoverlapping(text.as_ptr(), l as *mut u16, n);
                *(l as *mut u16).add(n) = 0;
            }
            n as u64
        }
        WM_GETTEXTLENGTH => info(env, hwnd).map_or(0, |i| u64::from(i.text_len)),
        WM_CLOSE => {
            destroy_window(env, hwnd);
            0
        }
        WM_SYSCOMMAND => {
            if w & 0xFFF0 == SC_CLOSE {
                send(env, hwnd, WM_CLOSE, 0, 0);
            }
            0
        }
        WM_QUERYENDSESSION => 1,
        WM_NOTIFYFORMAT => 2,
        // The parent has the first say (WindowFromPoint.yaml `child_activate`).
        WM_MOUSEACTIVATE => match info(env, hwnd) {
            Some(i) if i.style & WS_CHILD != 0 => {
                let r = send(env, u64::from(i.parent), WM_MOUSEACTIVATE, w, l);
                if r != 0 { r } else { 1 }
            }
            _ => 1,
        },
        input::WM_SETCURSOR => {
            let Some(i) = info(env, hwnd) else { return 0 };
            if i.style & WS_CHILD != 0 {
                let r = send(env, u64::from(i.parent), input::WM_SETCURSOR, w, l);
                if r != 0 {
                    return r;
                }
            }
            // The window the cursor is over sets its class cursor for a
            // client hit and returns FALSE; a window asked on behalf of a
            // child changes nothing (SetCursor.yaml `class_cursor`,
            // `child_without_cursor`). A class without a cursor keeps the
            // shape (`kept`).
            if w == hwnd
                && l & 0xFFFF == input::HTCLIENT
                && let Reply::Value { value } = env.call(Request::ClassLong { hwnd, index: -12 })
                && value != 0
            {
                env.set_cursor(value);
            }
            0
        }
        WM_NCHITTEST => {
            let Some(i) = info(env, hwnd) else { return 0 };
            let (px, py) = (l as u16 as i16 as i32, (l >> 16) as u16 as i16 as i32);
            if i.client.contains(px, py) {
                1
            } else if i.window.contains(px, py) {
                if has_caption(i.style) && py < i.client.top { 2 } else { 18 }
            } else {
                0
            }
        }
        // Validates what is invalid (BeginPaint/EndPaint), erasing first.
        paint::WM_PAINT => {
            let mut ps = PaintStruct::default();
            begin_paint(env, hwnd, &mut ps);
            end_paint(env, hwnd, &ps);
            0
        }
        // Poly's frame is the compositor's: nothing to draw here.
        paint::WM_NCPAINT => 0,
        WM_ERASEBKGND => {
            let Some(i) = info(env, hwnd) else { return 0 };
            if i.class_brush == 0 || w == 0 {
                return 0;
            }
            env.fill_rect(
                w,
                CRect { left: 0, top: 0, right: i.client.width(), bottom: i.client.height() },
                i.class_brush,
            );
            1
        }
        WM_RBUTTONUP => {
            let Some(i) = info(env, hwnd) else { return 0 };
            let (px, py) = (l as u16 as i16 as i32, (l >> 16) as u16 as i16 as i32);
            send(env, hwnd, WM_CONTEXTMENU, hwnd, makelparam(px + i.client.left, py + i.client.top));
            0
        }
        WM_CONTEXTMENU | WM_MOUSEWHEEL => {
            if let Some(i) = info(env, hwnd)
                && i.style & WS_CHILD != 0
            {
                return send(env, u64::from(i.parent), msg, w, l);
            }
            0
        }
        _ => 0,
    }
}

/// A NUL-terminated UTF-16 string.
///
/// # Safety
/// `p` points at a NUL-terminated string.
pub unsafe fn read_wstr(p: u64) -> Vec<u16> {
    let mut v = Vec::new();
    let mut q = p as *const u16;
    // SAFETY: the caller's promise.
    unsafe {
        while *q != 0 {
            v.push(*q);
            q = q.add(1);
        }
    }
    v
}

// ----- messages

fn to_cmsg(m: &Msg) -> CMsg {
    CMsg {
        hwnd: u64::from(m.hwnd),
        message: m.msg,
        w: m.w,
        l: m.l,
        time: m.time,
        pt: CPoint { x: m.x, y: m.y },
        private: m.private,
    }
}

/// `PeekMessageW`: messages sent by other threads run first.
pub fn peek_message<E: Env + ?Sized>(env: &E, out: &mut CMsg, filter: u64, min: u32, max: u32, flags: u32) -> bool {
    loop {
        match env.call(Request::Peek { filter, min, max, flags }) {
            Reply::Incoming { incoming } => run_incoming(env, &incoming),
            Reply::Message { msg } => {
                *out = to_cmsg(&msg);
                if (0x0200..=0x020E).contains(&msg.msg) && flags & PM_REMOVE != 0 && !input::mouse_retrieved(env, out) {
                    continue;
                }
                return true;
            }
            Reply::Error { code } => {
                env.set_last_error(code);
                return false;
            }
            _ => return false,
        }
    }
}

/// `GetMessageW`: 0 for `WM_QUIT`, -1 for a bad filter window.
pub fn get_message<E: Env + ?Sized>(env: &E, out: &mut CMsg, filter: u64, min: u32, max: u32) -> i32 {
    loop {
        match env.call(Request::Peek { filter, min, max, flags: PM_REMOVE }) {
            Reply::Incoming { incoming } => run_incoming(env, &incoming),
            Reply::Message { msg } => {
                *out = to_cmsg(&msg);
                if (0x0200..=0x020E).contains(&msg.msg) && !input::mouse_retrieved(env, out) {
                    continue;
                }
                return if msg.msg == WM_QUIT { 0 } else { 1 };
            }
            Reply::Empty { next_timer_ms } => env.wait_queue(next_timer_ms),
            Reply::Error { code } => {
                env.set_last_error(code);
                return -1;
            }
            _ => return -1,
        }
    }
}

/// `DispatchMessageW` (`DispatchMessageW.yaml`).
pub fn dispatch_message<E: Env + ?Sized>(env: &E, m: &CMsg) -> u64 {
    if m.message == WM_TIMER && m.l != 0 {
        let valid = matches!(
            env.call(Request::TimerProc { hwnd: m.hwnd, id: m.w, timer_proc: m.l }),
            Reply::Bool { value: true }
        );
        if valid {
            env.call_proc(m.l, m.hwnd as u32, WM_TIMER, m.w, u64::from(env.tick()));
            // The value the oracle returned after a timer callback.
            return 22;
        }
        return 0;
    }
    if m.hwnd == 0 {
        return 0;
    }
    let Some(i) = info(env, m.hwnd) else {
        env.set_last_error(ERROR_INVALID_WINDOW_HANDLE);
        return 0;
    };
    env.call_proc(i.wndproc, i.hwnd, m.message, m.w, m.l)
}

/// The character a key makes with no modifier down on the US layout, for
/// `TranslateMessage` (`TranslateMessage.yaml`).
pub fn key_char(vk: u32) -> Option<u16> {
    Some(match vk {
        0x08 => 8,
        0x09 => 9,
        0x0D => 13,
        0x1B => 27,
        0x20 => 32,
        0x30..=0x39 => vk as u16,
        0x41..=0x5A => (vk + 0x20) as u16,
        0x60..=0x69 => (vk - 0x30) as u16,
        0x6A => b'*' as u16,
        0x6B => b'+' as u16,
        0x6D => b'-' as u16,
        0x6E => b'.' as u16,
        0x6F => b'/' as u16,
        0xBA => b';' as u16,
        0xBB => b'=' as u16,
        0xBC => b',' as u16,
        0xBD => b'-' as u16,
        0xBE => b'.' as u16,
        0xBF => b'/' as u16,
        0xC0 => b'`' as u16,
        0xDB => b'[' as u16,
        0xDC => b'\\' as u16,
        0xDD => b']' as u16,
        0xDE => b'\'' as u16,
        _ => return None,
    })
}

/// `TranslateMessage`.
pub fn translate_message<E: Env + ?Sized>(env: &E, m: &CMsg) -> bool {
    let out = match m.message {
        WM_KEYDOWN => WM_CHAR,
        WM_SYSKEYDOWN => WM_SYSCHAR,
        WM_KEYUP | WM_SYSKEYUP => return true,
        _ => return false,
    };
    if let Some(c) = key_char(m.w as u32) {
        env.call(Request::Post { hwnd: m.hwnd, msg: out, w: u64::from(c), l: m.l });
    }
    true
}

/// `MsgWaitForMultipleObjectsEx` (`MsgWaitForMultipleObjects.yaml`): input
/// counts when it is new since the queue was last looked at (or, with
/// `MWMO_INPUTAVAILABLE`, whenever some is there); a timer counts once it
/// falls due.
pub fn msg_wait<E: Env + ?Sized>(env: &E, handles: &[u64], wait_all: bool, ms: u32, mask: u32, flags: u32) -> u32 {
    const WAIT_TIMEOUT: u32 = 0x102;
    const MWMO_INPUTAVAILABLE: u32 = 0x4;
    let n = handles.len() as u32;
    let deadline = if ms == u32::MAX { None } else { Some(u64::from(env.tick()) + u64::from(ms)) };
    loop {
        let (now, changed, next_timer) = match env.call(Request::QueueStatus { flags: 0xFFFF, clear: false }) {
            Reply::Bits { now, changed, next_timer_ms } => (now, changed, next_timer_ms),
            _ => (0, 0, None),
        };
        let input = changed & mask != 0 || (flags & MWMO_INPUTAVAILABLE != 0 && now & mask != 0);
        if input && !wait_all {
            return n;
        }
        let left = deadline.map(|d| d.saturating_sub(u64::from(env.tick())));
        if left == Some(0) {
            return WAIT_TIMEOUT;
        }
        let timer = if mask & crate::wm::QS_TIMER != 0 { next_timer.map(|t| t.max(1)) } else { None };
        let wait = match (left, timer) {
            (Some(a), Some(b)) => Some(a.min(b)),
            (a, None) => a,
            (None, b) => b,
        };
        if let Some(i) = env.wait_handles(handles, wait_all, wait, !wait_all)
            && i < n
        {
            return i;
        }
    }
}

#[cfg(test)]
#[path = "user_tests.rs"]
mod tests;

pub mod focus;
pub mod place;
pub mod input;
pub mod paint;
pub use paint::{PaintStruct, begin_paint, end_paint};

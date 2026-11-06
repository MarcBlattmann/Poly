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
    // WS_VISIBLE is the show at the end of the sequence.
    spec.style &= !WS_VISIBLE;
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
    if style & WS_VISIBLE != 0 {
        show(env, &i, ShowKind::Create);
    }
    hwnd
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
    }
    // A descendant of the hidden window that holds the capture loses it too.
    focus::release_capture_below(env, hwnd);
    if let Reply::Relatives { owned, .. } = env.call(Request::Relatives { hwnd }) {
        for o in owned {
            destroy_tree(env, o, false);
        }
    }
    send_destroy(env, hwnd);
    nc_destroy(env, hwnd);
}

fn send_destroy<E: Env + ?Sized>(env: &E, hwnd: u32) {
    send(env, u64::from(hwnd), WM_DESTROY, 0, 0);
    if let Reply::Relatives { children, .. } = env.call(Request::Relatives { hwnd }) {
        for c in children {
            send_destroy(env, c);
        }
    }
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

/// `ShowWindow` (`ShowWindow.yaml`): the previous `WS_VISIBLE` state.
pub fn show_window<E: Env + ?Sized>(env: &E, hwnd: u64, cmd: i32) -> bool {
    let Some(i) = info(env, hwnd) else {
        env.set_last_error(ERROR_INVALID_WINDOW_HANDLE);
        return false;
    };
    let was = i.style & WS_VISIBLE != 0;
    match cmd {
        SW_HIDE => {
            if was {
                hide(env, &i);
            }
        }
        SW_SHOWMINIMIZED | SW_MINIMIZE | SW_SHOWMINNOACTIVE | SW_FORCEMINIMIZE => {
            if screen(env).interactive {
                minimize_interactive(env, &i, cmd);
            } else {
                minimize(env, &i);
            }
        }
        SW_SHOWMAXIMIZED => maximize(env, &i),
        _ => {
            if (i.minimized || i.maximized) && matches!(cmd, SW_SHOWNORMAL | SW_RESTORE | SW_SHOWDEFAULT) {
                restore(env, &i, true);
            } else if i.minimized && cmd == SW_SHOWNOACTIVATE {
                // "Displays a window in its most recent size and position"
                // (Learn): a minimized window is restored (observed:
                // WM_WINDOWPOSCHANGING to the normal rectangle, 0x8120).
                restore(env, &i, false);
            } else if !was {
                show(env, &i, ShowKind::Command(cmd));
            }
        }
    }
    was
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
    let activatable = i.ex_style & WS_EX_NOACTIVATE == 0;
    let mut flags = SWP_SHOWWINDOW | SWP_NOMOVE | SWP_NOSIZE;
    let mut extra = 0;
    match kind {
        ShowKind::Create => {}
        ShowKind::Command(SW_SHOWNA) => flags |= SWP_NOACTIVATE,
        ShowKind::Command(SW_SHOWNOACTIVATE) => {
            flags |= SWP_NOACTIVATE | SWP_NOZORDER;
            if i.owner != 0 {
                extra = 0x1000_0000;
            }
        }
        ShowKind::Command(_) => {
            if !activatable {
                flags |= SWP_NOACTIVATE;
            }
        }
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
    let interactive = screen(env).interactive;
    if interactive {
        // Observed on the interactive desktop: the top bit 0x10000000 in
        // WM_WINDOWPOSCHANGED of every show but a WS_EX_NOACTIVATE window's
        // creation; creating a visible window activates it even then.
        extra = if !activatable && matches!(kind, ShowKind::Create) { 0 } else { 0x1000_0000 };
    }
    let activate = (activatable || (interactive && matches!(kind, ShowKind::Create))) && flags & SWP_NOACTIVATE == 0;
    swp(env, i.hwnd, HWND_TOP, 0, 0, 0, 0, flags, extra, activate);
    if i.size_move_pending {
        env.call(Request::SetState {
            hwnd: i.hwnd,
            visible: None,
            minimized: None,
            maximized: None,
            normal: None,
            size_move_pending: Some(false),
        });
        if let Some(i) = info(env, h) {
            send_size_move(env, &i);
        }
    }
}

fn hide<E: Env + ?Sized>(env: &E, i: &WinInfo) {
    let h = u64::from(i.hwnd);
    send(env, h, WM_SHOWWINDOW, 0, 0);
    // Hiding the active window of an owner (interactive): the system
    // activates the owner inside the call, so the call is not
    // SWP_NOACTIVATE / SWP_NOZORDER (GetActiveWindow.yaml gaw_owner: 0x83).
    let hands_over = screen(env).interactive && i.owner != 0 && focus::input(env).active == i.hwnd;
    let flags = if hands_over {
        SWP_HIDEWINDOW | SWP_NOMOVE | SWP_NOSIZE
    } else {
        SWP_HIDEWINDOW | SWP_NOACTIVATE | SWP_NOZORDER | SWP_NOMOVE | SWP_NOSIZE
    };
    set_window_pos(env, h, 0, 0, 0, 0, 0, flags);
}

fn set_state<E: Env + ?Sized>(env: &E, hwnd: u32, minimized: bool, maximized: bool, normal: Option<Rect>) {
    env.call(Request::SetState {
        hwnd,
        visible: Some(true),
        minimized: Some(minimized),
        maximized: Some(maximized),
        normal,
        size_move_pending: Some(false),
    });
}

fn minimize<E: Env + ?Sized>(env: &E, i: &WinInfo) {
    if i.minimized {
        return;
    }
    let sh = screen(env).height;
    let normal = if i.maximized { None } else { Some(i.window_rel) };
    set_state(env, i.hwnd, true, false, normal);
    let (w, h) = MINIMIZED;
    swp(env, i.hwnd, HWND_TOP, 0, sh - h, w, h, SWP_STATECHANGED | 0x100 | SWP_SHOWWINDOW | SWP_FRAMECHANGED, 0, false);
}

/// Minimizing on the interactive desktop (GetFocus.yaml `minimize`,
/// GetForegroundWindow.yaml): to -32000,-32000 (SM_ARRANGE is ARW_HIDE);
/// SW_SHOWMINIMIZED activates the minimized window, SW_MINIMIZE of the
/// active window takes the focus away first and gives the activation away
/// after.
fn minimize_interactive<E: Env + ?Sized>(env: &E, i: &WinInfo, cmd: i32) {
    if i.minimized {
        return;
    }
    let normal = if i.maximized { None } else { Some(i.window_rel) };
    let st = focus::input(env);
    let was_active = st.active == i.hwnd;
    let activate = cmd == SW_SHOWMINIMIZED;
    if was_active && !activate && st.focus != 0 {
        send(env, u64::from(st.focus), focus::WM_KILLFOCUS, 0, 0);
        env.call(Request::SetInput { active: 0, focus: 0, capture: 0, which: crate::wm::input::SET_FOCUS });
    }
    // A hidden window is shown by the swp below (its WINDOWPOS keeps
    // SWP_SHOWWINDOW and counts as a z-order change); a visible one already is.
    env.call(Request::SetState {
        hwnd: i.hwnd,
        visible: i.visible.then_some(true),
        minimized: Some(true),
        maximized: Some(false),
        normal,
        size_move_pending: Some(false),
    });
    let (w, h) = MINIMIZED;
    let flags =
        SWP_STATECHANGED | 0x100 | SWP_SHOWWINDOW | SWP_FRAMECHANGED | if activate { 0 } else { SWP_NOACTIVATE };
    let extra = if was_active && !activate { 0 } else { 0x1000_0000 };
    // A window with a minimize box goes to the bottom-left corner like
    // session 0's (ClientToScreen.yaml cts_states: 0,1052 on 1080 rows);
    // one without (a caption-only popup) is parked at -32000,-32000
    // (GetForegroundWindow.yaml gfw_show).
    let (mx, my) = if i.style & WS_MINIMIZEBOX != 0 { (0, screen(env).height - h) } else { (-32000, -32000) };
    swp(env, i.hwnd, HWND_TOP, mx, my, w, h, flags, extra, activate);
    if was_active && !activate {
        focus::deactivate_away(env, true);
    }
}

fn maximize<E: Env + ?Sized>(env: &E, i: &WinInfo) {
    if i.maximized {
        return;
    }
    // WM_GETMINMAXINFO first: the maximized size and position are the
    // procedure's answer (ShowWindow.yaml: maxsize 1040,784 at -8,-8 on a
    // 1024x768 screen, the frame overhanging it).
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
    let size = (mmi.max_size.x, mmi.max_size.y);
    let pos = (display.work.left + mmi.max_position.x, display.work.top + mmi.max_position.y);
    let normal = if i.minimized { None } else { Some(i.window_rel) };
    set_state(env, i.hwnd, false, true, normal);
    swp(env, i.hwnd, HWND_TOP, pos.0, pos.1, size.0, size.1, SWP_STATECHANGED | SWP_FRAMECHANGED, 0, false);
}

/// SW_RESTORE and friends. On the interactive desktop restoring a window
/// that was minimized or maximized activates it inside the call
/// (`activate`; GetFocus.yaml `restore`: WM_WINDOWPOSCHANGING 0x8120, the
/// activation with its own WM_WINDOWPOSCHANGING 0x3, WM_WINDOWPOSCHANGED
/// 0x8120, then WM_ACTIVATE once more).
fn restore<E: Env + ?Sized>(env: &E, i: &WinInfo, activate: bool) {
    let r = normal_rect(env, i);
    set_state(env, i.hwnd, false, false, None);
    // Out of the icon its pixels are not kept (SWP_NOCOPYBITS, as when it
    // was minimized; observed 0x8120).
    let nocopy = if i.minimized { 0x100 } else { 0 };
    let flags = SWP_STATECHANGED | nocopy | SWP_FRAMECHANGED;
    let activating = activate
        && screen(env).interactive
        && i.style & WS_CHILD == 0
        && i.ex_style & WS_EX_NOACTIVATE == 0
        && focus::input(env).active != i.hwnd;
    swp(env, i.hwnd, HWND_TOP, r.left, r.top, r.width(), r.height(), flags, 0, activating);
    if activating && focus::input(env).active == i.hwnd {
        send(env, u64::from(i.hwnd), focus::WM_ACTIVATE, focus::WA_ACTIVE, 0);
    }
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
    swp(env, i.hwnd, after, x, y, cx, cy, flags, 0, false)
}

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
    extra: u32,
    activate: bool,
) -> bool {
    let h = u64::from(hwnd);
    let Some(i) = info(env, h) else { return false };
    let mut wp = WindowPos { hwnd: h, insert_after: after, x, y, cx: cx.max(0), cy: cy.max(0), flags };
    if flags & SWP_NOSENDCHANGING == 0 {
        send(env, h, WM_WINDOWPOSCHANGING, 0, ptr(&mut wp));
    }
    let interactive = screen(env).interactive;
    // Whether the window is raised: asked before the activation below brings
    // it to the top, which is a z-order change all the same (GetActiveWindow.yaml
    // `show_again`: no SWP_NOZORDER once another process's window was raised
    // above it meanwhile).
    let raised_by_call = flags & SWP_NOZORDER == 0
        && matches!(
            env.call(Request::ZOrder { hwnd, after: wp.insert_after, apply: false }),
            Reply::Bool { value: true }
        );
    if flags & SWP_HIDEWINDOW != 0
        && flags & SWP_NOACTIVATE == 0
        && interactive
        && i.owner != 0
        && focus::input(env).active == hwnd
    {
        // The owner is the next window to activate: it is asked first.
        let mut o = WindowPos {
            hwnd: u64::from(i.owner),
            flags: SWP_NOACTIVATE | SWP_NOMOVE | SWP_NOSIZE,
            ..WindowPos::default()
        };
        send(env, u64::from(i.owner), WM_WINDOWPOSCHANGING, 0, ptr(&mut o));
    }
    if activate && interactive && !focus::in_background(env, i.pid) {
        // The interactive session activates it (focus::activate), inside
        // this SetWindowPos.
        if i.owner != 0 {
            let mut o = WindowPos {
                hwnd: u64::from(i.owner),
                flags: SWP_NOACTIVATE | SWP_NOMOVE | SWP_NOSIZE,
                ..WindowPos::default()
            };
            send(env, u64::from(i.owner), WM_WINDOWPOSCHANGING, 0, ptr(&mut o));
        }
        focus::activate(env, hwnd, true, true);
    } else if activate {
        // Session 0 activates nothing: bringing it to the top only.
        let mut a = WindowPos { hwnd: h, flags: SWP_NOACTIVATE | SWP_NOMOVE | SWP_NOSIZE, ..WindowPos::default() };
        send(env, h, WM_WINDOWPOSCHANGING, 0, ptr(&mut a));
    }
    let f = wp.flags;
    let cur = i.window_rel;
    let (nx, ny) = if f & SWP_NOMOVE != 0 { (cur.left, cur.top) } else { (wp.x, wp.y) };
    let (mut ncx, mut ncy) =
        if f & SWP_NOSIZE != 0 { (cur.width(), cur.height()) } else { (wp.cx.max(0), wp.cy.max(0)) };
    let top = !i.desktop && i.style & WS_CHILD == 0;
    if f & SWP_NOSIZE == 0 && top && (has_caption(i.style) || i.style & WS_THICKFRAME != 0) {
        let [ms, mp, mt, xt] = metrics::minmax(i.style, i.ex_style, &screen(env));
        let mut mmi = MinMaxInfo {
            reserved: CPoint::default(),
            max_size: CPoint { x: ms.0, y: ms.1 },
            max_position: CPoint { x: mp.0, y: mp.1 },
            min_track: CPoint { x: mt.0, y: mt.1 },
            max_track: CPoint { x: xt.0, y: xt.1 },
        };
        send(env, h, WM_GETMINMAXINFO, 0, ptr(&mut mmi));
        if f & SWP_STATECHANGED == 0 && metrics::track_limited(i.style) {
            ncx = ncx.max(mmi.min_track.x).min(mmi.max_track.x.max(mmi.min_track.x));
            ncy = ncy.max(mmi.min_track.y).min(mmi.max_track.y.max(mmi.min_track.y));
        }
    }
    let moved = (nx, ny) != (cur.left, cur.top);
    let sized = (ncx, ncy) != (cur.width(), cur.height());
    let styled_visible = i.style & WS_VISIBLE != 0;
    let show = f & SWP_SHOWWINDOW != 0 && !styled_visible;
    // A minimizing show of a hidden window enters the z order: it changed
    // (GetForegroundWindow.yaml gfw_show: no SWP_NOZORDER in the result).
    let zchanged = f & SWP_NOZORDER == 0
        && (raised_by_call
            || matches!(
                env.call(Request::ZOrder { hwnd, after: wp.insert_after, apply: false }),
                Reply::Bool { value: true }
            )
            || (show && interactive && !activate && f & SWP_STATECHANGED != 0));
    let hide = f & SWP_HIDEWINDOW != 0 && styled_visible;
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
        // Minimizing: the WINDOWPOS of WM_NCCALCSIZE has neither
        // SWP_SHOWWINDOW nor SWP_NOZORDER (observed 0x9920 for 0x8160).
        let calc_flags = if f & SWP_STATECHANGED != 0 && i.minimized {
            (eff | SWP_NOCLIENTSIZE | SWP_NOCLIENTMOVE) & !(SWP_SHOWWINDOW | SWP_NOZORDER)
        } else {
            eff | SWP_NOCLIENTSIZE | SWP_NOCLIENTMOVE
        };
        let mut calc =
            WindowPos { hwnd: h, insert_after: wp.insert_after, x: nx, y: ny, cx: ncx, cy: ncy, flags: calc_flags };
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
    if interactive && f & SWP_SHOWWINDOW != 0 && vis_before {
        changed &= !SWP_SHOWWINDOW;
    }
    if !client_sized {
        changed |= SWP_NOCLIENTSIZE;
    }
    if !client_moved {
        changed |= SWP_NOCLIENTMOVE;
    }
    env.call(Request::SetRects { hwnd, window_rel: Some(new_win), client_rel: Some(new_client) });
    if zchanged {
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
        } else if vis_before && vis_after && sized {
            paint::resized_paint(env, hwnd, old_client, new_client);
        }
    }
    let mut done = WindowPos { hwnd: h, insert_after: wp.insert_after, x: nx, y: ny, cx: ncx, cy: ncy, flags: changed };
    send(env, h, WM_WINDOWPOSCHANGED, 0, ptr(&mut done));
    if hide && vis_before && interactive {
        // Destroying a window ends the capture of a window below it before
        // the deactivation messages (SetCapture.yaml `destroy`).
        if focus::DESTROYING.load(core::sync::atomic::Ordering::Relaxed) == hwnd {
            focus::release_capture_below(env, hwnd);
        }
        focus::after_hide(env, &i);
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
pub mod input;
pub mod paint;
pub use paint::{PaintStruct, begin_paint, end_paint};

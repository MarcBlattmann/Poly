//! The message sequences against a `Wm` in memory, with window procedures
//! that log what they get the way the slice's visible tests print it.

extern crate std;

use super::*;
use crate::wm::paint::{
    RDW_ALLCHILDREN, RDW_ERASE, RDW_ERASENOW, RDW_INVALIDATE, RDW_NOCHILDREN, RDW_UPDATENOW, RDW_VALIDATE,
};
use crate::wm::{Ctx, Effects, Wm};
use alloc::format;
use alloc::string::{String, ToString};
use core::cell::{Cell, RefCell};
use std::thread_local;

struct TestEnv {
    wm: RefCell<Wm>,
    tid: Cell<u32>,
    err: Cell<u32>,
    /// The cursor shape DefWindowProc's WM_SETCURSOR set last.
    cursor: Cell<u64>,
    /// GDI regions the tests made: handle and rectangles.
    regions: RefCell<Vec<(u64, Vec<Rect>)>>,
}

thread_local! {
    static LOG: RefCell<Vec<String>> = const { RefCell::new(Vec::new()) };
    static ENV: Cell<*const TestEnv> = const { Cell::new(core::ptr::null()) };
}

fn env() -> &'static TestEnv {
    // SAFETY: set by `with_env` for the length of a test.
    ENV.with(|e| unsafe { &*e.get() })
}

fn log(s: String) {
    LOG.with(|l| l.borrow_mut().push(s));
}

fn take_log() -> Vec<String> {
    LOG.with(|l| core::mem::take(&mut *l.borrow_mut()))
}

impl Env for TestEnv {
    fn call(&self, req: Request) -> Reply {
        let ctx = Ctx { pid: 7, tid: self.tid.get(), now: 0 };
        self.wm.borrow_mut().handle(ctx, req, &mut Effects::default())
    }
    fn call_proc(&self, proc_: u64, hwnd: u32, msg: u32, w: u64, l: u64) -> u64 {
        if proc_ == 0 {
            return def_window_proc(self, u64::from(hwnd), msg, w, l);
        }
        // SAFETY: procedures of these tests have this signature.
        let f: extern "system" fn(u64, u32, u64, u64) -> u64 = unsafe { core::mem::transmute(proc_ as usize) };
        f(u64::from(hwnd), msg, w, l)
    }
    fn tid(&self) -> u32 {
        self.tid.get()
    }
    fn set_last_error(&self, e: u32) {
        self.err.set(e);
    }
    fn tick(&self) -> u32 {
        0
    }
    fn wait_queue(&self, _: Option<u64>) {}
    fn wait_handles(&self, _: &[u64], _: bool, _: Option<u64>, _: bool) -> Option<u32> {
        None
    }
    fn fill_rect(&self, _: u64, _: CRect, _: u64) {}
    fn delete_dc(&self, _: u64) {}
    fn paint_dc(&self, _: u32, _: Option<&[Rect]>) -> u64 {
        0x77
    }
    fn release_dc(&self, _: u32, _: u64) {}
    fn make_region(&self, _: &[Rect]) -> u64 {
        0x99
    }
    fn delete_object(&self, _: u64) {}
    fn set_cursor(&self, cursor: u64) {
        self.cursor.set(cursor);
    }
    fn region_rects(&self, rgn: u64) -> Option<Vec<Rect>> {
        self.regions.borrow().iter().find(|r| r.0 == rgn).map(|r| r.1.clone())
    }
    fn set_region_rects(&self, rgn: u64, rects: &[Rect]) -> i32 {
        let mut all = self.regions.borrow_mut();
        let Some(r) = all.iter_mut().find(|r| r.0 == rgn) else { return 0 };
        r.1 = rects.to_vec();
        match rects.len() {
            0 => 1,
            1 => 2,
            _ => 3,
        }
    }
}

fn with_env(f: impl FnOnce(&TestEnv)) {
    let e = TestEnv {
        wm: RefCell::new(Wm::new()),
        tid: Cell::new(1),
        err: Cell::new(0),
        cursor: Cell::new(0),
        regions: RefCell::new(Vec::new()),
    };
    ENV.with(|p| p.set(&e));
    take_log();
    f(&e);
    ENV.with(|p| p.set(core::ptr::null()));
}

/// The tag of a window: its GWLP_USERDATA byte.
fn tag(h: u64) -> char {
    match env().call(Request::GetLong { hwnd: h, index: -21, size: 8 }) {
        Reply::Value { value } if value != 0 => value as u8 as char,
        _ => NEXT.with(|n| n.get()),
    }
}

thread_local! {
    static NEXT: Cell<char> = const { Cell::new('?') };
}

extern "system" fn logging_proc(h: u64, msg: u32, w: u64, l: u64) -> u64 {
    if msg == WM_NCCREATE {
        let t = NEXT.with(|n| n.get());
        env().call(Request::SetLong { hwnd: h, index: -21, size: 8, value: t as u64 });
    }
    let t = tag(h);
    match msg {
        WM_NCCREATE | WM_CREATE => {
            // SAFETY: a CREATESTRUCTW.
            let cs = unsafe { &*(l as *const CreateStructW) };
            log(format!(
                "{t} {} pos={},{} size={},{} style={:#x} ex={:#x}",
                if msg == WM_CREATE { "WM_CREATE" } else { "WM_NCCREATE" },
                cs.x,
                cs.y,
                cs.cx,
                cs.cy,
                cs.style,
                cs.ex_style
            ));
        }
        WM_GETMINMAXINFO => {
            // SAFETY: a MINMAXINFO.
            let m = unsafe { &*(l as *const MinMaxInfo) };
            log(format!(
                "{t} WM_GETMINMAXINFO maxsize={},{} maxpos={},{} mintrack={},{} maxtrack={},{}",
                m.max_size.x,
                m.max_size.y,
                m.max_position.x,
                m.max_position.y,
                m.min_track.x,
                m.min_track.y,
                m.max_track.x,
                m.max_track.y
            ));
        }
        WM_NCCALCSIZE => {
            if w != 0 {
                // SAFETY: NCCALCSIZE_PARAMS.
                let p = unsafe { &*(l as *const NcCalcSizeParams) };
                let r = p.rgrc[0];
                // SAFETY: its WINDOWPOS.
                let wp = unsafe { &*p.pos };
                log(format!(
                    "{t} WM_NCCALCSIZE 1 new={},{},{},{} pos={},{},{},{} flags={:#x}",
                    r.left, r.top, r.right, r.bottom, wp.x, wp.y, wp.cx, wp.cy, wp.flags
                ));
            } else {
                // SAFETY: a RECT.
                let r = unsafe { &*(l as *const CRect) };
                log(format!("{t} WM_NCCALCSIZE 0 rect={},{},{},{}", r.left, r.top, r.right, r.bottom));
            }
        }
        WM_WINDOWPOSCHANGING | WM_WINDOWPOSCHANGED => {
            // SAFETY: a WINDOWPOS.
            let wp = unsafe { &*(l as *const WindowPos) };
            let name = if msg == WM_WINDOWPOSCHANGING { "WM_WINDOWPOSCHANGING" } else { "WM_WINDOWPOSCHANGED" };
            log(format!("{t} {name} pos={},{} size={},{} flags={:#x}", wp.x, wp.y, wp.cx, wp.cy, wp.flags));
        }
        WM_SIZE | WM_MOVE => {
            let name = if msg == WM_SIZE { "WM_SIZE" } else { "WM_MOVE" };
            log(format!("{t} {name} w={} l={},{}", w, l as u16 as i16, (l >> 16) as u16 as i16));
        }
        WM_PARENTNOTIFY => {
            let ev = if w & 0xFFFF == u64::from(WM_CREATE) { "WM_CREATE" } else { "WM_DESTROY" };
            log(format!("{t} WM_PARENTNOTIFY event={ev} id={} l={}", w >> 16, u8::from(l != 0)));
        }
        WM_SHOWWINDOW => log(format!("{t} WM_SHOWWINDOW w={w:#x} l={l:#x}")),
        WM_NCHITTEST => log(format!("{t} WM_NCHITTEST screen={},{}", l as u16 as i16, (l >> 16) as u16 as i16)),
        input::WM_SETCURSOR => {
            log(format!("{t} WM_SETCURSOR window={} hit={} msg={}", tag(w), l & 0xFFFF, (l >> 16) & 0xFFFF))
        }
        paint::WM_PAINT => log(format!("{t} WM_PAINT")),
        paint::WM_NCPAINT => log(format!("{t} WM_NCPAINT w={w:#x}")),
        WM_ERASEBKGND => log(format!("{t} WM_ERASEBKGND")),
        focus::WM_ACTIVATE => log(format!("{t} WM_ACTIVATE w={w:#x}")),
        focus::WM_NCACTIVATE => log(format!("{t} WM_NCACTIVATE w={w:#x}")),
        focus::WM_ACTIVATEAPP => log(format!("{t} WM_ACTIVATEAPP w={w:#x}")),
        focus::WM_SETFOCUS => log(format!("{t} WM_SETFOCUS")),
        focus::WM_KILLFOCUS => log(format!("{t} WM_KILLFOCUS")),
        focus::WM_CANCELMODE => log(format!("{t} WM_CANCELMODE")),
        0x0215 => log(format!("{t} WM_CAPTURECHANGED lparam={}", if l == 0 { '0' } else { tag(l) })),
        0x0200 => log(format!("{t} WM_MOUSEMOVE client={},{}", l as u16 as i16, (l >> 16) as u16 as i16)),
        0x00A0 => log(format!("{t} WM_NCMOUSEMOVE hit={w}")),
        0x02A3 => log(format!("{t} WM_MOUSELEAVE")),
        0x02A2 => log(format!("{t} WM_NCMOUSELEAVE")),
        WM_DESTROY => log(format!("{t} WM_DESTROY")),
        WM_NCDESTROY => log(format!("{t} WM_NCDESTROY")),
        _ => {}
    }
    def_window_proc(env(), h, msg, w, l)
}

fn register(e: &TestEnv, name: &str) {
    let spec = ClassSpec {
        name: name.encode_utf16().collect(),
        wndproc: logging_proc as *const () as u64,
        unicode: true,
        ..ClassSpec::default()
    };
    assert!(matches!(e.call(Request::RegisterClass { spec }), Reply::Atom { .. }));
}

#[allow(clippy::too_many_arguments)]
fn create(
    e: &TestEnv,
    tag: char,
    ex: u32,
    class: &str,
    style: u32,
    x: i32,
    y: i32,
    cx: i32,
    cy: i32,
    parent: u64,
    menu: u64,
) -> u64 {
    NEXT.with(|n| n.set(tag));
    let spec = CreateSpec {
        ex_style: ex,
        class: ClassRef::Name { name: class.encode_utf16().collect() },
        style,
        x,
        y,
        cx,
        cy,
        parent,
        menu,
        instance: 0x1_4000_0000,
    };
    let h = u64::from(create_window(e, spec, 0, 0));
    NEXT.with(|n| n.set('?'));
    h
}

#[test]
fn a_hidden_overlapped_window_is_created_in_the_documented_order() {
    with_env(|e| {
        register(e, "Over");
        let ow = WS_CAPTION | WS_SYSMENU | WS_THICKFRAME | 0x0003_0000;
        let h = create(e, 'A', 0, "Over", ow, 100, 100, 300, 200, 0, 0);
        assert_eq!(
            take_log(),
            [
                "A WM_GETMINMAXINFO maxsize=1040,784 maxpos=-8,-8 mintrack=136,39 maxtrack=1044,788",
                "A WM_NCCREATE pos=100,100 size=300,200 style=0xcf0000 ex=0x100",
                "A WM_NCCALCSIZE 0 rect=100,100,400,300",
                "A WM_CREATE pos=100,100 size=300,200 style=0xcf0000 ex=0x100",
            ]
        );
        let Reply::Window { info } = e.call(Request::Window { hwnd: h }) else { panic!() };
        assert_eq!((info.client.width(), info.client.height()), (284, 161));
        assert_eq!((info.client.left, info.client.top), (108, 131));
        assert_eq!(info.style, 0x04cf_0000);
        assert_eq!(info.ex_style, 0x100);
    });
}

#[test]
fn children_and_visible_popups_get_their_messages() {
    with_env(|e| {
        register(e, "C");
        let p = create(e, 'P', 0, "C", WS_POPUP, 40, 30, 300, 200, 0, 0);
        take_log();
        create(e, 'D', 0, "C", WS_CHILD | WS_VISIBLE, 60, 20, 50, 40, p, 6);
        assert_eq!(
            take_log(),
            [
                "D WM_NCCREATE pos=60,20 size=50,40 style=0x50000000 ex=0x0",
                "D WM_NCCALCSIZE 0 rect=100,50,150,90",
                "D WM_CREATE pos=60,20 size=50,40 style=0x50000000 ex=0x0",
                "D WM_SIZE w=0 l=50,40",
                "D WM_MOVE w=0 l=60,20",
                "P WM_PARENTNOTIFY event=WM_CREATE id=6 l=1",
                "D WM_SHOWWINDOW w=0x1 l=0x0",
            ]
        );
        create(e, 'B', 0, "C", WS_POPUP | WS_VISIBLE, 200, 10, 120, 90, 0, 0);
        assert_eq!(
            take_log()[5..],
            [
                "B WM_SHOWWINDOW w=0x1 l=0x0",
                "B WM_WINDOWPOSCHANGING pos=0,0 size=0,0 flags=0x43",
                "B WM_WINDOWPOSCHANGING pos=0,0 size=0,0 flags=0x13",
                "B WM_WINDOWPOSCHANGED pos=200,10 size=120,90 flags=0x1847",
            ]
        );
    });
}

#[test]
fn set_window_pos_on_a_hidden_child() {
    with_env(|e| {
        register(e, "S");
        let p = create(e, 'P', 0, "S", WS_POPUP, 0, 0, 300, 300, 0, 0);
        let c = create(e, 'C', 0, "S", WS_CHILD, 10, 10, 50, 50, p, 1);
        take_log();
        let nz = SWP_NOZORDER | SWP_NOACTIVATE;
        set_window_pos(e, c, 0, 20, 30, 0, 0, nz | SWP_NOSIZE);
        assert_eq!(
            take_log(),
            [
                "C WM_WINDOWPOSCHANGING pos=20,30 size=0,0 flags=0x15",
                "C WM_WINDOWPOSCHANGED pos=20,30 size=50,50 flags=0x81d",
                "C WM_MOVE w=0 l=20,30",
            ]
        );
        set_window_pos(e, c, 0, 0, 0, 70, 40, nz | SWP_NOMOVE);
        assert_eq!(
            take_log(),
            [
                "C WM_WINDOWPOSCHANGING pos=0,0 size=70,40 flags=0x16",
                "C WM_NCCALCSIZE 1 new=20,30,90,70 pos=20,30,70,40 flags=0x181e",
                "C WM_WINDOWPOSCHANGED pos=20,30 size=70,40 flags=0x101e",
                "C WM_SIZE w=0 l=70,40",
            ]
        );
        set_window_pos(e, c, 0, 0, 0, 0, 0, nz | SWP_NOMOVE | SWP_NOSIZE | SWP_SHOWWINDOW);
        assert_eq!(
            take_log(),
            [
                "C WM_WINDOWPOSCHANGING pos=0,0 size=0,0 flags=0x57",
                "C WM_WINDOWPOSCHANGED pos=20,30 size=70,40 flags=0x185f",
            ]
        );
    });
}

#[test]
fn minimize_nccalcsize_drops_show_and_nozorder_flags() {
    with_env(|e| {
        register(e, "Min");
        let style = WS_CAPTION | WS_SYSMENU | WS_THICKFRAME | 0x0003_0000;
        let h = create(e, 'M', 0, "Min", style, 50, 40, 300, 200, 0, 0);
        take_log();

        show_window(e, h, SW_MINIMIZE);

        assert!(
            take_log()
                .iter()
                .any(|entry| { entry == "M WM_NCCALCSIZE 1 new=0,740,160,768 pos=0,740,160,28 flags=0x9920" })
        );
    });
}

#[test]
fn rtl_children_inherit_layout_and_map_points_mirror_horizontally() {
    with_env(|e| {
        register(e, "Rtl");
        let rtl = create(e, 'R', WS_EX_NOACTIVATE | WS_EX_LAYOUTRTL, "Rtl", WS_POPUP, 650, 100, 200, 150, 0, 0);
        let s = create(e, 'S', 0, "Rtl", WS_CHILD | WS_VISIBLE, 10, 20, 60, 40, rtl, 1);
        let n = create(e, 'N', WS_EX_NOINHERITLAYOUT, "Rtl", WS_CHILD | WS_VISIBLE, 100, 20, 60, 40, rtl, 2);
        let Reply::Window { info: r } = e.call(Request::Window { hwnd: rtl }) else { panic!() };
        let Reply::Window { info: s } = e.call(Request::Window { hwnd: s }) else { panic!() };
        let Reply::Window { info: n } = e.call(Request::Window { hwnd: n }) else { panic!() };

        assert_eq!(s.ex_style, WS_EX_LAYOUTRTL);
        assert_eq!(n.ex_style, WS_EX_LAYOUTRTL | WS_EX_NOINHERITLAYOUT);
        assert_eq!(s.window, Rect::new(780, 120, 840, 160));
        assert_eq!(n.window, Rect::new(690, 120, 750, 160));

        let r_origin = MapOrigin { x: r.client.right, y: r.client.top, rtl: true };
        let screen = MapOrigin::default();
        let mut single = [(10, 5)];
        assert_eq!(map_points(r_origin, screen, &mut single), 0x0064_fcae_u32 as i32);
        assert_eq!(single, [(840, 105)]);
        let mut rect = [(10, 5), (50, 25)];
        assert_eq!(map_points(r_origin, screen, &mut rect), 0x0064_fcae_u32 as i32);
        assert_eq!(rect, [(800, 105), (840, 125)]);
    });
}

#[test]
fn client_mouse_move_sends_nchittest_before_setcursor() {
    with_env(|e| {
        register(e, "MouseMove");
        let h = create(e, 'A', 0, "MouseMove", WS_POPUP | WS_VISIBLE, 0, 0, 100, 100, 0, 0);
        take_log();
        let mut message =
            CMsg { hwnd: h, message: crate::wm::raw::WM_MOUSEMOVE, l: makelparam(50, 50), ..CMsg::default() };

        assert!(input::mouse_retrieved(e, &mut message));

        assert_eq!(take_log(), ["A WM_NCHITTEST screen=50,50", "A WM_SETCURSOR window=A hit=1 msg=512"]);
        assert_eq!(message.l, makelparam(50, 50));
    });
}

#[test]
fn nonclient_mouse_move_uses_geometric_hit_without_nchittest_callout() {
    with_env(|e| {
        register(e, "MouseMoveNc");
        let h = create(e, 'A', 0, "MouseMoveNc", WS_POPUP | WS_VISIBLE | WS_CAPTION, 0, 0, 100, 100, 0, 0);
        take_log();
        let mut message =
            CMsg { hwnd: h, message: crate::wm::raw::WM_MOUSEMOVE, l: makelparam(50, 5), ..CMsg::default() };

        assert!(input::mouse_retrieved(e, &mut message));

        assert_eq!(take_log(), ["A WM_SETCURSOR window=A hit=2 msg=512"]);
        assert_eq!(message.message, 0x00A0);
        assert_eq!(message.w, 2);
        assert_eq!(message.l, makelparam(50, 5));
    });
}

#[test]
fn background_focus_activation_does_not_send_windowposchanged() {
    with_env(|e| {
        register(e, "FocusBackground");
        let a = create(e, 'A', 0, "FocusBackground", WS_POPUP | WS_VISIBLE, 50, 50, 100, 100, 0, 0);
        e.call(Request::SetInput {
            active: a as u32,
            focus: a as u32,
            capture: 0,
            which: crate::wm::input::SET_ACTIVE | crate::wm::input::SET_FOCUS | crate::wm::input::SET_FOREGROUND,
        });

        e.tid.set(2);
        let t = create(e, 'T', 0, "FocusBackground", WS_POPUP | WS_VISIBLE, 200, 50, 100, 100, 0, 0);
        e.call(Request::SetInput {
            active: t as u32,
            focus: t as u32,
            capture: 0,
            which: crate::wm::input::SET_ACTIVE | crate::wm::input::SET_FOCUS | crate::wm::input::SET_FOREGROUND,
        });
        e.tid.set(1);
        take_log();

        assert_eq!(focus::set_focus(e, a), a);

        let log = take_log();
        assert!(log.iter().any(|line| line == "A WM_WINDOWPOSCHANGING pos=0,0 size=0,0 flags=0x3"));
        assert!(!log.iter().any(|line| line.starts_with("A WM_WINDOWPOSCHANGED")));
        assert!(matches!(e.call(Request::InputState {}), Reply::Input { active, focus, foreground, .. }
            if u64::from(active) == a && u64::from(focus) == a && u64::from(foreground) == t));
    });
}

#[test]
fn destroy_goes_down_then_back_up() {
    with_env(|e| {
        register(e, "D");
        let p = create(e, 'P', 0, "D", WS_POPUP, 0, 0, 200, 200, 0, 0);
        let a = create(e, 'A', 0, "D", WS_CHILD, 0, 0, 50, 50, p, 1);
        create(e, 'B', 0, "D", WS_CHILD, 60, 0, 50, 50, p, 2);
        create(e, 'G', 0, "D", WS_CHILD, 1, 1, 10, 10, a, 3);
        let c = create(e, 'C', 0, "D", WS_CHILD, 120, 0, 50, 50, p, 4);
        take_log();
        assert!(destroy_window(e, c));
        assert_eq!(take_log(), ["P WM_PARENTNOTIFY event=WM_DESTROY id=4 l=1", "C WM_DESTROY", "C WM_NCDESTROY"]);
        assert!(destroy_window(e, p));
        assert_eq!(
            take_log(),
            [
                "P WM_DESTROY",
                "A WM_DESTROY",
                "G WM_DESTROY",
                "B WM_DESTROY",
                "G WM_NCDESTROY",
                "A WM_NCDESTROY",
                "B WM_NCDESTROY",
                "P WM_NCDESTROY",
            ]
        );
        assert!(!e.wm.borrow().is_window(p));
    });
}

fn from_point(e: &TestEnv, x: i32, y: i32, below: u32) -> u32 {
    let Reply::Handle { hwnd } = e.call(Request::WindowFromPoint { x, y, below }) else { panic!() };
    hwnd
}

#[test]
fn topmost_windows_stay_above_later_created_ones() {
    with_env(|e| {
        register(e, "Z");
        let top = create(e, 'T', crate::metrics::WS_EX_TOPMOST, "Z", WS_POPUP | WS_VISIBLE, 10, 10, 100, 100, 0, 0);
        let plain = create(e, 'P', 0, "Z", WS_POPUP | WS_VISIBLE, 10, 10, 100, 100, 0, 0);
        // WindowFromPoint.yaml wfp_toplevel: a later created window does not
        // cover a topmost one; HWND_TOP leaves it below the topmost band.
        assert_eq!(u64::from(from_point(e, 50, 50, 0)), top);
        assert!(matches!(
            e.call(Request::ZOrder { hwnd: plain as u32, after: HWND_TOP, apply: true }),
            Reply::Bool { .. }
        ));
        assert_eq!(u64::from(from_point(e, 50, 50, 0)), top);
        // A window answering HTTRANSPARENT passes to the one below it.
        assert_eq!(u64::from(from_point(e, 50, 50, top as u32)), plain);
        assert_ne!(from_point(e, 50, 50, plain as u32), plain as u32);
    });
}

#[test]
fn a_new_capture_drops_the_synthesized_move_of_a_release() {
    with_env(|e| {
        register(e, "Cap");
        let a = create(e, 'A', 0, "Cap", WS_POPUP | WS_VISIBLE, 0, 0, 200, 200, 0, 0);
        let set = |capture: u32| {
            e.call(Request::SetInput { active: 0, focus: 0, capture, which: crate::wm::input::SET_CAPTURE });
        };
        e.call(Request::Cursor { set: true, x: 50, y: 50 });
        let drain = || {
            let mut n = 0;
            while let Reply::Message { .. } =
                e.call(Request::Peek { filter: 0, min: 0x200, max: 0x20E, flags: PM_REMOVE })
            {
                n += 1;
            }
            n
        };
        assert_eq!(drain(), 1);
        set(a as u32);
        set(0);
        set(a as u32);
        assert_eq!(drain(), 0);
        set(0);
        assert_eq!(drain(), 1);
    });
}

#[test]
fn a_minimized_window_with_a_minimize_box_sits_at_the_bottom_left_on_the_interactive_screen() {
    with_env(|e| {
        e.wm.borrow_mut().set_display(crate::metrics::Display::INTERACTIVE);
        register(e, "Min");
        let style = WS_CAPTION | WS_SYSMENU | WS_THICKFRAME | crate::metrics::WS_MINIMIZEBOX;
        let h = create(e, 'M', 0, "Min", style, 300, 100, 300, 200, 0, 0);
        let p = create(e, 'N', 0, "Min", WS_POPUP | WS_CAPTION, 500, 100, 150, 100, 0, 0);
        show_window(e, h, SW_SHOWMINNOACTIVE);
        show_window(e, p, SW_SHOWMINNOACTIVE);
        // ClientToScreen.yaml cts_states: 0,1052 on 1080 rows; GetForegroundWindow.yaml
        // gfw_show: a caption-only popup is parked at -32000,-32000.
        let Reply::Window { info } = e.call(Request::Window { hwnd: h }) else { panic!() };
        assert_eq!((info.window.left, info.window.top), (0, 1052));
        let Reply::Window { info } = e.call(Request::Window { hwnd: p }) else { panic!() };
        assert_eq!((info.window.left, info.window.top), (-32000, -32000));
    });
}

// ----- the held-out hunt: behaviours the sheets describe that the visible
// tests do not cover

fn interactive(e: &TestEnv) {
    e.wm.borrow_mut().set_display(crate::metrics::Display::INTERACTIVE);
}

fn input_state(e: &TestEnv) -> (u32, u32, u32) {
    let Reply::Input { active, focus, capture, .. } = e.call(Request::InputState {}) else { panic!() };
    (active, focus, capture)
}

#[test]
fn client_and_screen_points_follow_the_layout_direction() {
    // ClientToScreen.yaml popup / framed / children / rtl / minimized, ScreenToClient.yaml.
    with_env(|e| {
        register(e, "Pt");
        let ex = WS_EX_NOACTIVATE;
        let popup = create(e, 'P', ex, "Pt", WS_POPUP, 100, 100, 200, 150, 0, 0);
        for (c, s) in [((0, 0), (100, 100)), ((5, 7), (105, 107)), ((-5, -5), (95, 95)), ((1000, 1000), (1100, 1100))] {
            assert_eq!(convert_point(e, popup, c, true), Some(s));
            assert_eq!(convert_point(e, popup, s, false), Some(c));
        }
        assert_eq!(convert_point(e, popup, (1919, 1079), false), Some((1819, 979)));
        // A framed window: the client origin is 308,131.
        let ow = WS_CAPTION | WS_SYSMENU | WS_THICKFRAME | crate::metrics::WS_MINIMIZEBOX | 0x0001_0000;
        let framed = create(e, 'F', ex, "Pt", ow, 300, 100, 300, 200, 0, 0);
        assert_eq!(convert_point(e, framed, (0, 0), true), Some((308, 131)));
        assert_eq!(convert_point(e, framed, (845, 120), false), Some((537, -11)));
        let kid = create(e, 'K', 0, "Pt", WS_CHILD | WS_VISIBLE, 10, 20, 50, 50, framed, 1);
        assert_eq!(convert_point(e, kid, (0, 0), true), Some((318, 151)));
        assert_eq!(convert_point(e, kid, (350, 160), false), Some((32, 9)));
        // A right-to-left popup: x runs leftwards from the client's right edge.
        let rtl = create(e, 'R', ex | WS_EX_LAYOUTRTL, "Pt", WS_POPUP, 650, 100, 200, 150, 0, 0);
        for (c, s) in [((0, 0), (850, 100)), ((5, 7), (845, 107)), ((-5, -5), (855, 95)), ((1000, 1000), (-150, 1100))]
        {
            assert_eq!(convert_point(e, rtl, c, true), Some(s), "{c:?}");
            assert_eq!(convert_point(e, rtl, s, false), Some(c), "{s:?}");
        }
        assert_eq!(convert_point(e, rtl, (0, 0), false), Some((850, -100)));
        assert_eq!(convert_point(e, rtl, (845, 120), false), Some((5, 20)));
        assert_eq!(convert_point(e, rtl, (1919, 1079), false), Some((-1069, 979)));
        // Its child (created at 10,20, 60 wide, lying at 780,120,840,160) maps (0,0) to 840,120.
        let rk = create(e, 'S', 0, "Pt", WS_CHILD | WS_VISIBLE, 10, 20, 60, 40, rtl, 2);
        assert_eq!(convert_point(e, rk, (0, 0), true), Some((840, 120)));
        // A hidden window maps like a visible one.
        assert_eq!(convert_point(e, popup, (5, 7), true), Some((105, 107)));
        // NULL and made-up handles: nothing, ERROR_INVALID_WINDOW_HANDLE.
        for bad in [0, 0x1234_5678] {
            e.err.set(0);
            assert_eq!(convert_point(e, bad, (1, 2), true), None);
            assert_eq!(e.err.get(), ERROR_INVALID_WINDOW_HANDLE);
            e.err.set(0);
            assert_eq!(convert_point(e, bad, (1, 2), false), None);
            assert_eq!(e.err.get(), ERROR_INVALID_WINDOW_HANDLE);
        }
        // A destroyed window too.
        assert!(destroy_window(e, popup));
        assert_eq!(convert_point(e, popup, (1, 2), false), None);
    });
}

#[test]
fn a_minimized_window_maps_points_from_its_top_left() {
    // ClientToScreen.yaml `minimized`, ScreenToClient.yaml `states`.
    with_env(|e| {
        interactive(e);
        register(e, "MinPt");
        let style = WS_CAPTION | WS_SYSMENU | WS_THICKFRAME | crate::metrics::WS_MINIMIZEBOX;
        let m = create(e, 'M', WS_EX_NOACTIVATE, "MinPt", style, 300, 100, 300, 200, 0, 0);
        show_window(e, m, SW_SHOWMINNOACTIVE);
        assert_eq!(convert_point(e, m, (5, 7), true), Some((5, 1059)));
        assert_eq!(convert_point(e, m, (500, 400), false), Some((500, -652)));
    });
}

fn make_region(e: &TestEnv, handle: u64, rects: &[Rect]) {
    e.regions.borrow_mut().push((handle, rects.to_vec()));
}

fn update_rects(e: &TestEnv, h: u64) -> Vec<Rect> {
    paint::pending(e, h as u32).expect("window").rects
}

#[test]
fn invalidate_region_judges_the_window_and_the_region_object() {
    // InvalidateRgn.yaml errors and `hidden`, `empty`.
    with_env(|e| {
        interactive(e);
        register(e, "Irg");
        let w = create(e, 'W', WS_EX_NOACTIVATE, "Irg", WS_POPUP | WS_VISIBLE, 0, 0, 100, 80, 0, 0);
        paint::redraw_window(e, w, None, RDW_VALIDATE);
        assert!(update_rects(e, w).is_empty());
        // A made-up handle: 0 and ERROR_INVALID_WINDOW_HANDLE (not InvalidateRect's 1).
        e.err.set(0);
        assert!(!paint::invalidate_rgn(e, 0x1234_5678, 0, false));
        assert_eq!(e.err.get(), ERROR_INVALID_WINDOW_HANDLE);
        // A region handle that names nothing (deleted, or a brush): 1,
        // ERROR_INVALID_HANDLE, nothing invalidated.
        e.err.set(0);
        assert!(paint::invalidate_rgn(e, w, 0xDEAD, true));
        assert_eq!(e.err.get(), 6);
        assert!(update_rects(e, w).is_empty());
        // A made-up window is judged before a made-up region.
        e.err.set(0);
        assert!(!paint::invalidate_rgn(e, 0x1234_5678, 0xDEAD, true));
        assert_eq!(e.err.get(), ERROR_INVALID_WINDOW_HANDLE);
        // A real region: its rectangles (clipped), an empty one adds nothing.
        make_region(e, 0x41, &[Rect::new(-10, -10, 10, 10), Rect::new(90, 70, 150, 150)]);
        make_region(e, 0x42, &[]);
        e.err.set(0);
        assert!(paint::invalidate_rgn(e, w, 0x42, true));
        assert!(update_rects(e, w).is_empty());
        assert!(paint::invalidate_rgn(e, w, 0x41, false));
        assert_eq!(update_rects(e, w), [Rect::new(0, 0, 10, 10), Rect::new(90, 70, 100, 80)]);
        assert_eq!(e.err.get(), 0);
        // NULL: all of the client area.
        paint::redraw_window(e, w, None, RDW_VALIDATE);
        assert!(paint::invalidate_rgn(e, w, 0, false));
        assert_eq!(update_rects(e, w), [Rect::new(0, 0, 100, 80)]);
    });
}

#[test]
fn get_update_rgn_reports_kinds_and_errors_as_observed() {
    // GetUpdateRgn.yaml `replace`, `bands`, `errors`.
    with_env(|e| {
        interactive(e);
        register(e, "Gurg");
        let w = create(e, 'W', WS_EX_NOACTIVATE, "Gurg", WS_POPUP | WS_VISIBLE, 0, 0, 100, 80, 0, 0);
        paint::redraw_window(e, w, None, RDW_VALIDATE);
        make_region(e, 0x51, &[Rect::new(1, 2, 3, 4)]);
        // Nothing invalid: an empty region and NULLREGION; the old contents are replaced.
        assert_eq!(paint::get_update_rgn(e, w, 0x51, false), 1);
        assert!(e.region_rects(0x51).unwrap().is_empty());
        // Two rectangles that touch: y-x bands; stacked ones of one width merge.
        paint::invalidate_region(e, w, Some(&[Rect::new(10, 10, 30, 20), Rect::new(20, 15, 40, 25)]), false);
        assert_eq!(paint::get_update_rgn(e, w, 0x51, false), 3);
        assert_eq!(
            e.region_rects(0x51).unwrap(),
            [Rect::new(10, 10, 30, 15), Rect::new(10, 15, 40, 20), Rect::new(20, 20, 40, 25)]
        );
        // A bad window: 0, 1400, the region is not changed.
        for bad in [0, 0x1234_5678] {
            e.err.set(0);
            assert_eq!(paint::get_update_rgn(e, bad, 0x51, false), 0);
            assert_eq!(e.err.get(), ERROR_INVALID_WINDOW_HANDLE);
        }
        assert_eq!(e.region_rects(0x51).unwrap().len(), 3);
        // NULL region: 0 and ERROR_INVALID_HANDLE.
        e.err.set(0);
        assert_eq!(paint::get_update_rgn(e, w, 0, false), 0);
        assert_eq!(e.err.get(), 6);
        // A deleted region or a brush: SIMPLEREGION, last error 0.
        e.err.set(0xDEAD);
        assert_eq!(paint::get_update_rgn(e, w, 0xBEEF, false), 2);
        assert_eq!(e.err.get(), 0);
    });
}

#[test]
fn redraw_window_now_reaches_the_children_unless_told_not_to() {
    // RedrawWindow.yaml `updatenow_children`, `erasenow`; Learn: RDW_NOCHILDREN.
    with_env(|e| {
        interactive(e);
        register(e, "Rw");
        let p = create(e, 'P', WS_EX_NOACTIVATE, "Rw", WS_POPUP | WS_VISIBLE, 0, 0, 200, 200, 0, 0);
        let c = create(e, 'C', 0, "Rw", WS_CHILD | WS_VISIBLE, 10, 10, 50, 50, p, 1);
        let all = RDW_INVALIDATE | RDW_ERASE;
        let fresh = || {
            paint::redraw_window(e, p, None, RDW_VALIDATE | RDW_ALLCHILDREN);
            paint::redraw_window(e, p, None, all);
            take_log();
        };
        // RDW_UPDATENOW | RDW_NOCHILDREN paints the window only; its child stays invalid.
        fresh();
        assert!(paint::redraw_window(e, p, None, RDW_UPDATENOW | RDW_NOCHILDREN));
        assert_eq!(take_log(), ["P WM_PAINT", "P WM_ERASEBKGND"]);
        assert_eq!(update_rects(e, c), [Rect::new(0, 0, 50, 50)]);
        // Without it the window, then its invalid child.
        assert!(paint::redraw_window(e, p, None, RDW_UPDATENOW));
        let log = take_log();
        assert!(log.iter().any(|l| l == "C WM_PAINT"), "{log:?}");
        assert!(update_rects(e, c).is_empty());
        // RDW_ERASENOW: the erase now, also below the window; the regions stay.
        fresh();
        assert!(paint::redraw_window(e, p, None, RDW_ERASENOW));
        let log = take_log();
        assert_eq!(log.iter().filter(|l| l.ends_with("WM_ERASEBKGND")).count(), 2, "{log:?}");
        assert!(!update_rects(e, p).is_empty() && !update_rects(e, c).is_empty());
        fresh();
        assert!(paint::redraw_window(e, p, None, RDW_ERASENOW | RDW_NOCHILDREN));
        assert_eq!(take_log(), ["P WM_ERASEBKGND"]);
        assert!(!update_rects(e, c).is_empty());
    });
}

#[test]
fn a_disabled_popup_gets_wm_setcursor_with_a_hit_error_and_nothing_else() {
    // EnableWindow.yaml `mouse_disabled_popup`.
    with_env(|e| {
        register(e, "Dis");
        let d = create(e, 'D', 0, "Dis", WS_POPUP | WS_VISIBLE | WS_DISABLED, 0, 0, 100, 100, 0, 0);
        take_log();
        for (msg, hit_msg) in [(crate::wm::raw::WM_LBUTTONDOWN, 513), (crate::wm::raw::WM_MOUSEMOVE, 512)] {
            let mut m = CMsg { hwnd: d, message: msg, l: makelparam(50, 50), ..CMsg::default() };
            assert!(!input::mouse_retrieved(e, &mut m), "the message is dropped");
            assert_eq!(take_log(), [format!("D WM_SETCURSOR window=D hit=65534 msg={hit_msg}")]);
        }
        // Enabled again it is hit-tested and delivered as usual.
        e.call(Request::SetStyle { hwnd: d as u32, which: -16, value: WS_POPUP | WS_VISIBLE });
        let mut m = CMsg { hwnd: d, message: crate::wm::raw::WM_MOUSEMOVE, l: makelparam(50, 50), ..CMsg::default() };
        assert!(input::mouse_retrieved(e, &mut m));
        assert_eq!(take_log().first().map(String::as_str), Some("D WM_NCHITTEST screen=50,50"));
    });
}

#[test]
fn def_window_proc_sets_the_class_cursor_for_a_client_hit_only_for_itself() {
    // SetCursor.yaml `class_cursor`, `child_without_cursor`, `kept`, `parent_says_true`.
    with_env(|e| {
        let spec = |name: &str, cursor: u64| ClassSpec {
            name: name.encode_utf16().collect(),
            wndproc: logging_proc as *const () as u64,
            unicode: true,
            cursor,
            ..ClassSpec::default()
        };
        for (name, cursor) in [("WithCursor", 0x60003), ("NoCursor", 0)] {
            assert!(matches!(e.call(Request::RegisterClass { spec: spec(name, cursor) }), Reply::Atom { .. }));
        }
        let a = create(e, 'A', WS_EX_NOACTIVATE, "WithCursor", WS_POPUP | WS_VISIBLE, 0, 0, 100, 100, 0, 0);
        let k = create(e, 'K', 0, "NoCursor", WS_CHILD | WS_VISIBLE, 10, 10, 30, 30, a, 1);
        let hit = |code: u64| code | (0x200 << 16);
        e.cursor.set(0x60001);
        // The window itself, a client hit: the cross, FALSE.
        assert_eq!(def_window_proc(e, a, input::WM_SETCURSOR, a, hit(1)), 0);
        assert_eq!(e.cursor.get(), 0x60003);
        // A non-client hit leaves it.
        e.cursor.set(0x60001);
        assert_eq!(def_window_proc(e, a, input::WM_SETCURSOR, a, hit(2)), 0);
        assert_eq!(e.cursor.get(), 0x60001);
        // The child's class has none: its DefWindowProc asks the parent first
        // (on the child's behalf: the parent changes nothing), then FALSE.
        take_log();
        assert_eq!(def_window_proc(e, k, input::WM_SETCURSOR, k, hit(1)), 0);
        assert_eq!(take_log(), ["A WM_SETCURSOR window=K hit=1 msg=512"]);
        assert_eq!(e.cursor.get(), 0x60001, "the cursor stays what it was");
        // A parent that answers TRUE ends it: the child sets nothing.
        e.call(Request::RegisterClass { spec: spec("Other", 0x60005) });
        let b = create(e, 'B', WS_EX_NOACTIVATE, "NoCursor", WS_POPUP | WS_VISIBLE, 200, 0, 100, 100, 0, 0);
        let c = create(e, 'C', 0, "Other", WS_CHILD | WS_VISIBLE, 10, 10, 30, 30, b, 1);
        e.call(Request::SetLong { hwnd: b, index: -4, size: 8, value: truthy_proc as *const () as u64 });
        assert_eq!(def_window_proc(e, c, input::WM_SETCURSOR, c, hit(1)), 1);
        assert_eq!(e.cursor.get(), 0x60001);
    });
}

extern "system" fn truthy_proc(_h: u64, _msg: u32, _w: u64, _l: u64) -> u64 {
    1
}

#[test]
fn destroying_a_parent_cancels_the_capture_of_its_child_but_not_its_own() {
    // SetCapture.yaml `destroy`.
    with_env(|e| {
        interactive(e);
        register(e, "Cap2");
        let p = create(e, 'P', WS_EX_NOACTIVATE, "Cap2", WS_POPUP | WS_VISIBLE, 0, 0, 200, 200, 0, 0);
        let c = create(e, 'C', 0, "Cap2", WS_CHILD | WS_VISIBLE, 10, 10, 50, 50, p, 1);
        let capture = |h: u64| {
            e.call(Request::SetInput { active: 0, focus: 0, capture: h as u32, which: crate::wm::input::SET_CAPTURE });
        };
        capture(c);
        take_log();
        assert!(destroy_window(e, p));
        let log = take_log();
        let at = |s: &str| log.iter().position(|l| l.starts_with(s)).unwrap_or_else(|| panic!("{s} in {log:?}"));
        assert!(log.contains(&"C WM_CANCELMODE".to_string()), "{log:?}");
        assert!(log.contains(&"C WM_CAPTURECHANGED lparam=0".to_string()), "{log:?}");
        assert!(at("P WM_WINDOWPOSCHANGED") < at("C WM_CANCELMODE"), "{log:?}");
        assert!(at("C WM_CAPTURECHANGED") < at("P WM_DESTROY"), "{log:?}");
        assert_eq!(input_state(e).2, 0);
        // A window destroyed while it captures: no WM_CAPTURECHANGED.
        let w = create(e, 'W', WS_EX_NOACTIVATE, "Cap2", WS_POPUP | WS_VISIBLE, 0, 0, 100, 100, 0, 0);
        capture(w);
        take_log();
        assert!(destroy_window(e, w));
        assert!(!take_log().iter().any(|l| l.contains("CAPTURECHANGED") || l.contains("CANCELMODE")));
        assert_eq!(input_state(e).2, 0);
        // A hidden parent's child loses it when the parent is destroyed too.
        let h = create(e, 'H', WS_EX_NOACTIVATE, "Cap2", WS_POPUP, 0, 0, 100, 100, 0, 0);
        let k = create(e, 'K', 0, "Cap2", WS_CHILD | WS_VISIBLE, 0, 0, 10, 10, h, 1);
        capture(k);
        take_log();
        assert!(destroy_window(e, h));
        assert!(take_log().contains(&"K WM_CAPTURECHANGED lparam=0".to_string()));
    });
}

#[test]
fn restoring_a_minimized_window_activates_it_on_the_interactive_desktop() {
    // GetFocus.yaml `restore`: the restore's WM_WINDOWPOSCHANGING (0x8120),
    // the activation (its own 0x3, WM_ACTIVATEAPP, WM_NCACTIVATE,
    // WM_ACTIVATE, WM_SETFOCUS), WM_WINDOWPOSCHANGED (0x8120), WM_ACTIVATE again.
    with_env(|e| {
        interactive(e);
        register(e, "Rest");
        let style = WS_POPUP | WS_CAPTION | WS_VISIBLE;
        let a = create(e, 'A', crate::metrics::WS_EX_TOPMOST, "Rest", style, 50, 50, 300, 200, 0, 0);
        assert_eq!(input_state(e).0 as u64, a);
        show_window(e, a, SW_MINIMIZE);
        assert_eq!(input_state(e).0, 0, "minimizing gave the activation away");
        take_log();
        show_window(e, a, SW_RESTORE);
        let seq: Vec<String> = take_log()
            .into_iter()
            .filter(|l| {
                ["A WM_WINDOWPOS", "A WM_ACTIVATE", "A WM_NCACTIVATE", "A WM_SETFOCUS"].iter().any(|p| l.starts_with(p))
            })
            .collect();
        assert_eq!(
            seq,
            [
                "A WM_WINDOWPOSCHANGING pos=50,50 size=300,200 flags=0x8120",
                "A WM_WINDOWPOSCHANGING pos=0,0 size=0,0 flags=0x3",
                "A WM_ACTIVATEAPP w=0x1",
                "A WM_NCACTIVATE w=0x1",
                "A WM_ACTIVATE w=0x1",
                "A WM_SETFOCUS",
                "A WM_WINDOWPOSCHANGED pos=50,50 size=300,200 flags=0x8120",
                "A WM_ACTIVATE w=0x1",
            ]
        );
        assert_eq!(input_state(e).0 as u64, a);
        assert_eq!(input_state(e).1 as u64, a);
        // SW_SHOWNOACTIVATE restores without activating.
        show_window(e, a, SW_MINIMIZE);
        take_log();
        show_window(e, a, SW_SHOWNOACTIVATE);
        assert!(!take_log().iter().any(|l| l.contains("WM_ACTIVATE")));
        assert_eq!(input_state(e).0, 0);
    });
}

#[test]
fn cancelling_mouse_tracking_cancels_only_the_kind_named() {
    // TrackMouseEvent.yaml `cancel`, `hover_leave`, `query`.
    with_env(|e| {
        interactive(e);
        register(e, "Tme");
        let a = create(e, 'A', WS_EX_NOACTIVATE, "Tme", WS_POPUP | WS_VISIBLE, 0, 0, 200, 200, 0, 0);
        e.call(Request::Cursor { set: true, x: 50, y: 50 });
        let ah = a as u32;
        let query = || match e.call(Request::TrackMouse { hwnd: 0, flags: 0x4000_0000, hover: 0 }) {
            Reply::Value { value } => (value & 0xFFFF, (value >> 16) & 0xFFFF, value >> 32),
            r => panic!("{r:?}"),
        };
        assert_eq!(query(), (0, 0, 0));
        e.call(Request::TrackMouse { hwnd: ah, flags: 0x3, hover: 100 });
        assert_eq!(query(), (3, 100, a));
        // TME_CANCEL | TME_HOVER takes only the hover; the leave stays.
        e.call(Request::TrackMouse { hwnd: ah, flags: 0x8000_0001, hover: 0 });
        assert_eq!(query(), (2, 0, a));
        e.call(Request::TrackMouse { hwnd: ah, flags: 0x1, hover: u32::MAX });
        assert_eq!(query(), (3, 400, a), "HOVER_DEFAULT is reported as 400");
        // TME_CANCEL | TME_LEAVE takes only the leave.
        e.call(Request::TrackMouse { hwnd: ah, flags: 0x8000_0002, hover: 0 });
        assert_eq!(query(), (1, 400, a));
        e.call(Request::TrackMouse { hwnd: ah, flags: 0x8000_0001, hover: 0 });
        assert_eq!(query(), (0, 0, 0));
    });
}

#[test]
fn nonclient_tracking_posts_the_nonclient_leave_at_once_from_the_client_area() {
    // TrackMouseEvent.yaml `nonclient`.
    with_env(|e| {
        interactive(e);
        register(e, "TmeNc");
        let a = create(e, 'A', WS_EX_NOACTIVATE, "TmeNc", WS_POPUP | WS_CAPTION | WS_VISIBLE, 0, 0, 200, 200, 0, 0);
        let peek = || {
            matches!(
                e.call(Request::Peek { filter: 0, min: 0x2A2, max: 0x2A2, flags: PM_REMOVE }),
                Reply::Message { .. }
            )
        };
        // The cursor in the client area: WM_NCMOUSELEAVE at once.
        e.call(Request::Cursor { set: true, x: 100, y: 100 });
        while peek() {}
        input::track_nonclient(e, a as u32, 0x10 | 0x2);
        assert!(peek(), "WM_NCMOUSELEAVE");
        // In the caption: nothing until it leaves (DefWindowProc tracks it).
        e.call(Request::Cursor { set: true, x: 100, y: 5 });
        while peek() {}
        input::track_nonclient(e, a as u32, 0x10 | 0x2);
        assert!(!peek());
        assert_eq!(input::NC_TRACK.load(core::sync::atomic::Ordering::Relaxed), a as u32);
        input::track_nonclient(e, a as u32, 0x10 | 0x2 | 0x8000_0000);
        assert_eq!(input::NC_TRACK.load(core::sync::atomic::Ordering::Relaxed), 0);
    });
}

/// Send one mouse event (`flags`: MOUSEEVENTF_*) at time `now`.
fn mouse(e: &TestEnv, now: u64, flags: u32) {
    let ev = InputEvent { kind: 0, a: 0, b: 0, data: 0, flags };
    e.wm.borrow_mut().handle(
        Ctx { pid: 7, tid: 1, now },
        Request::SendInput { events: std::vec![ev] },
        &mut Effects::default(),
    );
}

/// The mouse messages (down, up, double click) queued now, with their keys.
fn mouse_messages(e: &TestEnv) -> Vec<(u32, u64)> {
    let mut out = Vec::new();
    while let Reply::Message { msg } = e.call(Request::Peek { filter: 0, min: 0x201, max: 0x209, flags: PM_REMOVE }) {
        out.push((msg.msg, msg.w & 0xFF));
    }
    out
}

#[test]
fn a_second_quick_click_is_a_double_click_for_a_class_that_wants_it() {
    // WindowFromPoint.yaml `double_click`, `mk_bits`.
    const DOWN: [u32; 3] = [0x2, 0x8, 0x20];
    with_env(|e| {
        let class = |name: &str, style: u32| {
            let spec = ClassSpec {
                name: name.encode_utf16().collect(),
                style,
                wndproc: logging_proc as *const () as u64,
                unicode: true,
                ..ClassSpec::default()
            };
            assert!(matches!(e.call(Request::RegisterClass { spec }), Reply::Atom { .. }));
        };
        class("Dbl", 0x0008);
        class("Plain", 0);
        let w = create(e, 'W', WS_EX_NOACTIVATE, "Dbl", WS_POPUP | WS_VISIBLE, 0, 0, 300, 300, 0, 0);
        let at = |x: i32, y: i32| {
            e.call(Request::Cursor { set: true, x, y });
            while !mouse_messages(e).is_empty() {}
        };
        let (ld, lu) = (0x0002, 0x0004);
        // Left button: down, up, down 1 px to the right: the second down is the double click.
        at(100, 100);
        mouse(e, 1000, ld);
        mouse(e, 1010, lu);
        e.call(Request::Cursor { set: true, x: 101, y: 100 });
        mouse(e, 1100, ld);
        mouse(e, 1110, lu);
        assert_eq!(mouse_messages(e), [(0x201, 1), (0x202, 0), (0x203, 1), (0x202, 0)]);
        // A third quick click starts afresh: a plain down.
        mouse(e, 1200, ld);
        mouse(e, 1210, lu);
        assert_eq!(mouse_messages(e), [(0x201, 1), (0x202, 0)]);
        // Too far: 3 px to the right, 2 px down; too slow: 501 ms later.
        let cases = [(3, 0, 10, false), (0, 2, 10, false), (-2, 1, 10, true), (0, 0, 501, false), (0, 0, 500, true)];
        for (n, (dx, dy, dt, double)) in cases.into_iter().enumerate() {
            let base = 5000 + 10_000 * n as u64;
            at(100, 100);
            mouse(e, base, ld);
            mouse(e, base + 1, lu);
            e.call(Request::Cursor { set: true, x: 100 + dx, y: 100 + dy });
            mouse(e, base + dt, ld);
            mouse(e, base + 1 + dt, lu);
            let got = mouse_messages(e);
            assert_eq!(got[2].0, if double { 0x203 } else { 0x201 }, "{dx},{dy},{dt}ms: {got:?}");
        }
        // Right and middle buttons likewise.
        for (i, (down, up, dbl)) in [(0x0008, 0x0010, 0x206), (0x0020, 0x0040, 0x209)].into_iter().enumerate() {
            at(100, 100);
            mouse(e, 9000, down);
            mouse(e, 9001, up);
            mouse(e, 9050, down);
            mouse(e, 9051, up);
            let got = mouse_messages(e);
            assert_eq!(got[2].0, dbl, "button {i}: {got:?}");
            assert_eq!(got[2].1, if i == 0 { 2 } else { 0x10 });
        }
        // A different button between the clicks is no double click.
        at(100, 100);
        mouse(e, 12000, ld);
        mouse(e, 12001, lu);
        mouse(e, 12010, DOWN[1]);
        mouse(e, 12011, 0x0010);
        mouse(e, 12020, ld);
        mouse(e, 12021, lu);
        assert!(mouse_messages(e).iter().all(|m| m.0 != 0x203));
        // A class without CS_DBLCLKS gets two plain downs.
        let p = create(e, 'P', WS_EX_NOACTIVATE, "Plain", WS_POPUP | WS_VISIBLE, 400, 0, 100, 100, 0, 0);
        assert_ne!(p, w);
        at(450, 50);
        mouse(e, 20000, ld);
        mouse(e, 20001, lu);
        mouse(e, 20010, ld);
        mouse(e, 20011, lu);
        assert_eq!(mouse_messages(e), [(0x201, 1), (0x202, 0), (0x201, 1), (0x202, 0)]);
    });
}

// The held-out hunt, one file per group (spec sheets re-read rule by rule).
#[path = "user_tests/focus.rs"]
mod focus_hunt;
#[path = "user_tests/input.rs"]
mod input_hunt;
#[path = "user_tests/paint.rs"]
mod paint_hunt;

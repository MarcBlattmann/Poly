//! The message sequences against a `Wm` in memory, with window procedures
//! that log what they get the way the slice's visible tests print it.

extern crate std;

use super::*;
use crate::wm::{Ctx, Effects, Wm};
use alloc::format;
use alloc::string::String;
use core::cell::{Cell, RefCell};
use std::thread_local;

struct TestEnv {
    wm: RefCell<Wm>,
    tid: Cell<u32>,
    err: Cell<u32>,
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
}

fn with_env(f: impl FnOnce(&TestEnv)) {
    let e = TestEnv { wm: RefCell::new(Wm::new()), tid: Cell::new(1), err: Cell::new(0) };
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

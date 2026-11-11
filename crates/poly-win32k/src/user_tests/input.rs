//! Held-out hunt, input group: behaviours the spec sheets describe that the visible
//! tests do not cover (each test names the sheet and rule it follows).

use super::*;

use crate::metrics::{WS_BORDER, WS_EX_CLIENTEDGE, WS_MINIMIZEBOX};

fn message_window(e: &TestEnv) -> u32 {
    let Reply::Desktop { message, .. } = e.call(Request::Desktop {}) else { panic!() };
    message
}

#[test]
fn client_origin_of_framed_edged_bordered_and_special_windows() {
    // ClientToScreen.yaml `framed`, `children`, `message_only`, `desktop`.
    with_env(|e| {
        interactive(e);
        register(e, "Cts");
        let ow = WS_CAPTION | WS_SYSMENU | WS_THICKFRAME | WS_MINIMIZEBOX | 0x0001_0000;
        let ex = WS_EX_NOACTIVATE;
        let edged = create(e, 'E', ex | WS_EX_CLIENTEDGE, "Cts", ow, 300, 350, 300, 200, 0, 0);
        assert_eq!(convert_point(e, edged, (0, 0), true), Some((310, 383)));
        let Reply::Window { info } = e.call(Request::Window { hwnd: edged }) else { panic!() };
        assert_eq!((info.client.width(), info.client.height()), (280, 157));
        let framed = create(e, 'F', ex, "Cts", ow, 300, 100, 300, 200, 0, 0);
        let kid = create(e, 'K', 0, "Cts", WS_CHILD | WS_VISIBLE, 10, 20, 100, 100, framed, 1);
        let bordered = create(e, 'B', 0, "Cts", WS_CHILD | WS_VISIBLE | WS_BORDER, 3, 4, 50, 40, kid, 2);
        assert_eq!(convert_point(e, kid, (0, 0), true), Some((318, 151)));
        let Reply::Window { info } = e.call(Request::Window { hwnd: bordered }) else { panic!() };
        assert_eq!((info.window.left, info.window.top, info.window.right, info.window.bottom), (321, 155, 371, 195));
        assert_eq!(convert_point(e, bordered, (0, 0), true), Some((322, 156)));
        // A message-only window of style 0 is overlapped (frame 8, caption 23).
        let mo = create(e, 'O', 0, "Cts", 0, 40, 50, 200, 150, u64::from(message_window(e)), 0);
        assert_eq!(convert_point(e, mo, (5, 7), true), Some((53, 88)));
        assert_eq!(convert_point(e, mo, (500, 400), false), Some((452, 319)));
        let oc = create(e, 'C', 0, "Cts", WS_CHILD, 10, 20, 50, 50, mo, 1);
        assert_eq!(convert_point(e, oc, (5, 7), true), Some((63, 108)));
        // The desktop window: unchanged.
        let d = u64::from(desktop(e));
        assert_eq!(convert_point(e, d, (5, 7), true), Some((5, 7)));
        assert_eq!(convert_point(e, d, (500, 400), false), Some((500, 400)));
    });
}

extern "system" fn ctx_proc(h: u64, msg: u32, w: u64, l: u64) -> u64 {
    if msg == 0x007B {
        log(format!("WM_CONTEXTMENU w={} l={},{}", w, l as u16 as i16, (l >> 16) as u16 as i16));
    }
    def_window_proc(env(), h, msg, w, l)
}

#[test]
fn mouse_keys_and_context_menu_follow_the_sheet() {
    // WindowFromPoint.yaml `mk_bits`, `contextmenu`.
    with_env(|e| {
        let spec = ClassSpec {
            name: "Mk".encode_utf16().collect(),
            wndproc: ctx_proc as *const () as u64,
            unicode: true,
            ..ClassSpec::default()
        };
        assert!(matches!(e.call(Request::RegisterClass { spec }), Reply::Atom { .. }));
        let w = create(e, 'W', WS_EX_NOACTIVATE, "Mk", WS_POPUP | WS_VISIBLE, 100, 100, 300, 300, 0, 0);
        e.call(Request::Cursor { set: true, x: 150, y: 160 });
        while !mouse_messages(e).is_empty() {}
        let (ld, lu, rd, ru) = (0x0002, 0x0004, 0x0008, 0x0010);
        // Right clicked while left is held: RBUTTONDOWN 3, RBUTTONUP 1.
        mouse(e, 100, ld);
        mouse(e, 101, rd);
        mouse(e, 102, ru);
        mouse(e, 103, lu);
        assert_eq!(mouse_messages(e), [(0x201, 1), (0x204, 3), (0x205, 1), (0x202, 0)]);
        // With Shift held: LBUTTONDOWN 5, LBUTTONUP 4.
        let key = |up: bool| InputEvent { kind: 1, a: 0x10, b: 0, data: 0, flags: u32::from(up) * 2 };
        e.wm.borrow_mut().handle(
            Ctx { pid: 7, tid: 1, now: 200 },
            Request::SendInput { events: std::vec![key(false)] },
            &mut Effects::default(),
        );
        mouse(e, 201, ld);
        mouse(e, 202, lu);
        e.wm.borrow_mut().handle(
            Ctx { pid: 7, tid: 1, now: 203 },
            Request::SendInput { events: std::vec![key(true)] },
            &mut Effects::default(),
        );
        assert_eq!(mouse_messages(e), [(0x201, 5), (0x202, 4)]);
        // DefWindowProc answers WM_RBUTTONUP with WM_CONTEXTMENU: the window,
        // the screen point.
        take_log();
        def_window_proc(e, w, 0x0205, 0, makelparam(50, 60));
        assert_eq!(take_log(), [format!("WM_CONTEXTMENU w={w} l=150,160")]);
    });
}

#[test]
fn a_child_is_hit_only_inside_its_parents_client_area() {
    // WindowFromPoint.yaml `children`, `framed`: the deepest child of the
    // topmost window; a child larger than its parent's client area is
    // clipped to it.
    with_env(|e| {
        interactive(e);
        register(e, "Wfp");
        let ow = WS_CAPTION | WS_SYSMENU | WS_THICKFRAME | WS_MINIMIZEBOX | 0x0001_0000;
        let f = create(e, 'F', WS_EX_NOACTIVATE, "Wfp", ow | WS_VISIBLE, 100, 250, 300, 200, 0, 0);
        // The client area is 108,281 .. 392,442; the child reaches over the caption.
        let k = create(e, 'K', 0, "Wfp", WS_CHILD | WS_VISIBLE, -5, -20, 400, 300, f, 1);
        let at = |x, y| match e.call(Request::WindowFromPoint { x, y, below: 0 }) {
            Reply::Handle { hwnd } => u64::from(hwnd),
            _ => 0,
        };
        assert_eq!(at(110, 283), k);
        assert_eq!(at(200, 265), f, "the caption");
        assert_eq!(at(391, 441), k);
        assert_eq!(at(392, 300), f, "the right frame");
        assert_eq!(at(101, 400), f, "the left frame");
    });
}

/// Dispatch every message the thread has queued (mouse messages through
/// the retrieval steps `GetMessage` does).
fn pump(e: &TestEnv) {
    let mut m = CMsg::default();
    while peek_message(e, &mut m, 0, 0, 0, PM_REMOVE) {
        dispatch_message(e, &m);
    }
}

fn move_to(e: &TestEnv, x: i32, y: i32) {
    e.call(Request::Cursor { set: true, x, y });
    pump(e);
}

#[test]
fn leaving_the_client_area_ends_the_client_tracking_also_for_the_frame_and_the_desktop() {
    // TrackMouseEvent.yaml `leave`, `nonclient`: a client TME_LEAVE ends when the cursor goes
    // into the window's own caption (WM_MOUSELEAVE after WM_NCMOUSEMOVE) or over no window;
    // moving from the caption into the client area ends DefWindowProc's tracking of the frame.
    with_env(|e| {
        interactive(e);
        register(e, "Nc");
        let ow = WS_CAPTION | WS_SYSMENU | WS_THICKFRAME | 0x0003_0000;
        let f = create(e, 'F', WS_EX_NOACTIVATE | WS_EX_TOPMOST, "Nc", ow | WS_VISIBLE, 100, 100, 300, 200, 0, 0);
        move_to(e, 500, 500);
        let track = |flags: u32| {
            e.call(Request::TrackMouse { hwnd: f as u32, flags, hover: 0 });
        };
        move_to(e, 200, 250);
        take_log();
        track(0x2);
        move_to(e, 200, 112);
        assert_eq!(take_log(), ["F WM_SETCURSOR window=F hit=2 msg=512", "F WM_NCMOUSEMOVE hit=2", "F WM_MOUSELEAVE"]);
        move_to(e, 210, 250);
        assert_eq!(
            take_log(),
            [
                "F WM_NCHITTEST screen=210,250",
                "F WM_SETCURSOR window=F hit=1 msg=512",
                "F WM_MOUSEMOVE client=102,119",
                "F WM_NCMOUSELEAVE"
            ]
        );
        // Tracked again in the client area, the cursor goes to the empty desktop.
        track(0x2);
        move_to(e, 1500, 600);
        assert_eq!(take_log(), ["F WM_MOUSELEAVE"]);
        // A hover that was over the window ends there too.
        move_to(e, 210, 250);
        take_log();
        e.call(Request::TrackMouse { hwnd: f as u32, flags: 0x1, hover: 100 });
        move_to(e, 1500, 600);
        assert_eq!(e.call(Request::TrackMouse { hwnd: 0, flags: 0x4000_0000, hover: 0 }), Reply::Value { value: 0 });
    });
}

#[test]
fn the_children_of_a_disabled_window_are_not_hit() {
    // EnableWindow.yaml `click_disabled_toplevel_inactive`: over an enabled
    // child H of the disabled popup D WindowFromPoint returns D;
    // `mouse_disabled_child`: a disabled child falls through to its parent.
    with_env(|e| {
        interactive(e);
        register(e, "Dis");
        let at = |x: i32, y: i32| match e.call(Request::WindowFromPoint { x, y, below: 0 }) {
            Reply::Handle { hwnd } => u64::from(hwnd),
            _ => 0,
        };
        let d = create(e, 'D', 0, "Dis", WS_POPUP | WS_VISIBLE, 100, 100, 200, 150, 0, 0);
        let h = create(e, 'H', 0, "Dis", WS_CHILD | WS_VISIBLE, 20, 20, 80, 50, d, 1);
        let g = create(e, 'G', 0, "Dis", WS_CHILD | WS_VISIBLE, 5, 5, 20, 20, h, 2);
        assert_eq!((at(130, 130), at(105, 105)), (g, d));
        assert_eq!(at(180, 150), h);
        crate::user::focus::enable_window(e, d, false);
        assert_eq!((at(130, 130), at(180, 150), at(105, 105)), (d, d, d));
        crate::user::focus::enable_window(e, d, true);
        crate::user::focus::enable_window(e, h, false);
        assert_eq!((at(130, 130), at(180, 150)), (d, d), "a disabled child and what is in it fall through");
    });
}

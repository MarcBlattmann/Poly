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

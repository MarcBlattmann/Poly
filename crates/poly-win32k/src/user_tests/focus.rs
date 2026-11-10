//! Held-out hunt, focus group: behaviours the spec sheets describe that the visible
//! tests do not cover (each test names the sheet and rule it follows).

use super::*;

const TOP: u32 = WS_EX_TOPMOST;

/// A log without the messages the oracle tests leave out (painting, the
/// size and move notifications, WM_NCCALCSIZE).
fn documented(log: Vec<String>) -> Vec<String> {
    log.into_iter()
        .filter(|l| !["PAINT", "ERASE", "NCCALCSIZE", "WM_MOVE", "WM_SIZE"].iter().any(|x| l.contains(x)))
        .collect()
}

/// The interactive desktop with A (a visible topmost popup, active, with the
/// focus) and B (a topmost popup shown with SW_SHOWNA, never active), as
/// GetActiveWindow.yaml / GetFocus.yaml set them up.
fn a_and_b(e: &TestEnv) -> (u64, u64) {
    interactive(e);
    register(e, "Fc");
    let a = create(e, 'A', TOP, "Fc", WS_POPUP | WS_VISIBLE, 50, 50, 300, 200, 0, 0);
    let b = create(e, 'B', TOP, "Fc", WS_POPUP, 400, 50, 200, 120, 0, 0);
    show_window(e, b, SW_SHOWNA);
    assert_eq!(input_state(e), (a as u32, a as u32, 0));
    take_log();
    (a, b)
}

#[test]
fn hiding_the_active_window_hands_the_activation_to_another_process() {
    // GetActiveWindow.yaml `hide_active`, `show_again`; GetFocus.yaml `hide_parent`.
    with_env(|e| {
        let (a, b) = a_and_b(e);
        show_window(e, a, SW_HIDE);
        assert_eq!(
            take_log(),
            [
                "A WM_SHOWWINDOW w=0x0 l=0x0",
                "A WM_WINDOWPOSCHANGING pos=0,0 size=0,0 flags=0x83",
                "A WM_WINDOWPOSCHANGED pos=50,50 size=300,200 flags=0x20001883",
                "A WM_NCACTIVATE w=0x0",
                "A WM_ACTIVATE w=0x0",
                "A WM_ACTIVATEAPP w=0x0",
                "B WM_ACTIVATEAPP w=0x0",
                "A WM_KILLFOCUS",
            ]
        );
        // Not B: the activation went to another process's window.
        assert_eq!(input_state(e).0, 0);
        assert_eq!(input_state(e).1, 0);
        show_window(e, a, SW_SHOW);
        let log = take_log();
        assert_eq!(
            log.iter().filter(|l| !l.contains("PAINT") && !l.contains("ERASE")).cloned().collect::<Vec<_>>(),
            [
                "A WM_SHOWWINDOW w=0x1 l=0x0",
                "A WM_WINDOWPOSCHANGING pos=0,0 size=0,0 flags=0x43",
                "A WM_WINDOWPOSCHANGING pos=0,0 size=0,0 flags=0x3",
                "A WM_ACTIVATEAPP w=0x1",
                "B WM_ACTIVATEAPP w=0x1",
                "A WM_NCACTIVATE w=0x1",
                "A WM_ACTIVATE w=0x1",
                "A WM_SETFOCUS",
                "A WM_WINDOWPOSCHANGED pos=50,50 size=300,200 flags=0x10001843",
            ]
        );
        assert_eq!(input_state(e), (a as u32, a as u32, 0));
        let _ = b;
    });
}

#[test]
fn minimizing_a_visible_window_on_the_interactive_desktop() {
    // GetActiveWindow.yaml `minimize_inactive`: B moves to -32000,-32000 (160x28,
    // flags 0x8170 / 0x8130), no activation; GetFocus.yaml `minimize`: the
    // active A with the focus on its child gets the same WINDOWPOS flags and
    // WM_ACTIVATE(WA_INACTIVE, minimized).
    with_env(|e| {
        let (a, b) = a_and_b(e);
        show_window(e, b, SW_MINIMIZE);
        assert_eq!(
            take_log().iter().filter(|l| l.starts_with("B WM_WINDOWPOS")).cloned().collect::<Vec<_>>(),
            [
                "B WM_WINDOWPOSCHANGING pos=-32000,-32000 size=160,28 flags=0x8170",
                "B WM_WINDOWPOSCHANGED pos=-32000,-32000 size=160,28 flags=0x8130",
            ]
        );
        assert_eq!(input_state(e), (a as u32, a as u32, 0));
        // The active A with the focus on its child C.
        let c = create(e, 'C', 0, "Fc", WS_CHILD | WS_VISIBLE, 10, 10, 100, 50, a, 1);
        take_log();
        crate::user::focus::set_focus(e, c);
        take_log();
        show_window(e, a, SW_MINIMIZE);
        assert_eq!(
            documented(take_log()),
            [
                "C WM_KILLFOCUS",
                "A WM_WINDOWPOSCHANGING pos=-32000,-32000 size=160,28 flags=0x8170",
                "A WM_WINDOWPOSCHANGED pos=-32000,-32000 size=160,28 flags=0x8130",
                "A WM_NCACTIVATE w=0x0",
                "A WM_ACTIVATE w=0x200000",
                // From the top of the z order down: the minimized A is at
                // the bottom (gaw_leave_a).
                "B WM_ACTIVATEAPP w=0x0",
                "A WM_ACTIVATEAPP w=0x0",
            ]
        );
        assert_eq!(input_state(e).0, 0);
        assert_eq!(input_state(e).1, 0);
    });
}

#[test]
fn the_focus_follows_hiding_and_destroying_windows() {
    // GetFocus.yaml `no_window`, `new_child`, `hide_focused_child`, `hide_parent`,
    // `show_again`, `destroy`.
    with_env(|e| {
        interactive(e);
        register(e, "Gf");
        assert_eq!(input_state(e), (0, 0, 0));
        // Only a hidden window: no focus.
        let a = create(e, 'A', TOP, "Gf", WS_POPUP, 50, 50, 300, 200, 0, 0);
        assert_eq!(input_state(e).1, 0);
        show_window(e, a, SW_SHOW);
        assert_eq!(input_state(e), (a as u32, a as u32, 0));
        // A visible child does not take the focus.
        let c = create(e, 'C', 0, "Gf", WS_CHILD | WS_VISIBLE, 10, 10, 100, 50, a, 1);
        assert_eq!(input_state(e).1, a as u32);
        // Hiding the focused child: the focus goes to its parent.
        crate::user::focus::set_focus(e, c);
        assert_eq!(input_state(e).1, c as u32);
        take_log();
        show_window(e, c, SW_HIDE);
        assert_eq!(input_state(e), (a as u32, a as u32, 0));
        show_window(e, c, SW_SHOW);
        crate::user::focus::set_focus(e, c);
        take_log();
        // Hiding the parent with the focus on its child.
        show_window(e, a, SW_HIDE);
        let log = documented(take_log());
        assert_eq!(
            &log[..3],
            &[
                "A WM_SHOWWINDOW w=0x0 l=0x0",
                "A WM_WINDOWPOSCHANGING pos=0,0 size=0,0 flags=0x83",
                // GetFocus.yaml `hide_parent`: 0x...887, A is alone in its
                // z-order group and on top.
                "A WM_WINDOWPOSCHANGED pos=50,50 size=300,200 flags=0x20001887",
            ]
        );
        assert_eq!(
            &log[3..],
            ["A WM_NCACTIVATE w=0x0", "A WM_ACTIVATE w=0x0", "A WM_ACTIVATEAPP w=0x0", "C WM_KILLFOCUS"]
        );
        assert_eq!(input_state(e), (0, 0, 0));
        // Shown again: A is active and has the focus itself, not C.
        show_window(e, a, SW_SHOW);
        assert_eq!(input_state(e), (a as u32, a as u32, 0));
        // Destroying the active window: no focus, no active window.
        destroy_window(e, a);
        assert_eq!(input_state(e), (0, 0, 0));
    });
}

fn fg(e: &TestEnv) -> u32 {
    crate::user::focus::input(e).foreground
}

#[test]
fn destroying_the_foreground_window_hands_it_to_the_other_thread() {
    // GetForegroundWindow.yaml `threads`, `destroy_foreground`.
    with_env(|e| {
        interactive(e);
        register(e, "Fg");
        let a = create(e, 'A', TOP, "Fg", WS_POPUP | WS_VISIBLE, 50, 50, 200, 150, 0, 0);
        assert_eq!(fg(e), a as u32);
        // Thread T's own popup, shown while A is the foreground.
        e.tid.set(2);
        let t = create(e, 'T', TOP, "Fg", WS_POPUP | WS_VISIBLE, 300, 50, 150, 100, 0, 0);
        assert_eq!(fg(e), a as u32, "same foreground window for every thread");
        assert_eq!(input_state(e), (0, 0, 0), "T has no active window and no focus");
        e.tid.set(1);
        assert_eq!(input_state(e), (a as u32, a as u32, 0));
        take_log();
        destroy_window(e, a);
        assert_eq!(fg(e), t as u32);
        e.tid.set(2);
        assert_eq!(fg(e), t as u32);
        assert_eq!(input_state(e), (t as u32, t as u32, 0), "T sees active T and focus T");
        e.tid.set(1);
        assert_eq!(input_state(e), (0, 0, 0));
    });
}

fn capture(e: &TestEnv) -> u32 {
    input_state(e).2
}

#[test]
fn set_capture_takes_odd_windows_and_refuses_other_threads() {
    // SetCapture.yaml `odd_windows`, `null`, `changed_on_set`, `bad_handle`.
    use crate::user::focus::set_capture;
    with_env(|e| {
        interactive(e);
        register(e, "Sc");
        let a = create(e, 'A', TOP, "Sc", WS_POPUP | WS_VISIBLE, 100, 100, 200, 150, 0, 0);
        let b = create(e, 'B', TOP, "Sc", WS_POPUP | WS_VISIBLE, 400, 100, 200, 150, 0, 0);
        let hidden = create(e, 'H', TOP, "Sc", WS_POPUP, 0, 0, 50, 50, 0, 0);
        let disabled = create(e, 'D', TOP, "Sc", WS_POPUP | WS_VISIBLE | WS_DISABLED, 0, 300, 50, 50, 0, 0);
        let child = create(e, 'C', 0, "Sc", WS_CHILD | WS_VISIBLE, 10, 10, 40, 40, a, 1);
        e.tid.set(2);
        let other = create(e, 'O', TOP, "Sc", WS_POPUP | WS_VISIBLE, 700, 100, 100, 100, 0, 0);
        e.tid.set(1);
        take_log();
        // SetCapture(NULL) with no capture: NULL, nothing.
        e.err.set(0);
        assert_eq!(set_capture(e, 0), 0);
        assert_eq!((capture(e), e.err.get()), (0, 0));
        // A window of another thread: NULL, error 0, no capture.
        assert_eq!(set_capture(e, other), 0);
        assert_eq!((capture(e), e.err.get()), (0, 0));
        // A made-up handle: NULL, ERROR_INVALID_WINDOW_HANDLE, unchanged.
        assert_eq!(set_capture(e, 0x0bad_0bad), 0);
        assert_eq!((capture(e), e.err.get()), (0, ERROR_INVALID_WINDOW_HANDLE));
        // Hidden, disabled and child windows can be given the capture.
        for w in [hidden, disabled, child] {
            assert_eq!(set_capture(e, w), 0);
            assert_eq!(capture(e), w as u32);
            assert_eq!(set_capture(e, 0), w);
            assert_eq!(capture(e), 0);
        }
        take_log();
        // SetCapture(A) twice: the second returns A and A hears of it with itself.
        assert_eq!(set_capture(e, a), 0);
        assert_eq!(take_log(), Vec::<String>::new());
        assert_eq!(set_capture(e, a), a);
        assert_eq!(take_log(), ["A WM_CAPTURECHANGED lparam=A"]);
        // B while A captures: A gets WM_CAPTURECHANGED (lParam B) before the call returns.
        assert_eq!(set_capture(e, b), a);
        assert_eq!(take_log(), ["A WM_CAPTURECHANGED lparam=B"]);
        // SetCapture(NULL) releases it: the previous one gets lParam NULL.
        assert_eq!(set_capture(e, 0), b);
        assert_eq!(take_log(), ["B WM_CAPTURECHANGED lparam=0"]);
        assert_eq!(capture(e), 0);
    });
}

#[test]
fn hiding_keeps_the_capture_and_disabling_releases_it() {
    // SetCapture.yaml `hide`, `disable`.
    use crate::user::focus::set_capture;
    with_env(|e| {
        let (a, b) = a_and_b(e);
        set_capture(e, b);
        take_log();
        show_window(e, b, SW_HIDE);
        assert_eq!(capture(e), b as u32, "hiding keeps the capture");
        assert!(!take_log().iter().any(|l| l.contains("CAPTURECHANGED")));
        show_window(e, b, SW_SHOWNA);
        take_log();
        crate::user::focus::enable_window(e, b, false);
        // (WM_ENABLE comes after, which the test's procedure does not log.)
        assert_eq!(take_log(), ["B WM_CANCELMODE", "B WM_CAPTURECHANGED lparam=0"]);
        assert_eq!(capture(e), 0);
        let _ = a;
    });
}

extern "system" fn mouse_proc(h: u64, msg: u32, w: u64, l: u64) -> u64 {
    let t = tag(h);
    match msg {
        0x0201 | 0x0202 => log(format!(
            "{t} WM_LBUTTON{} keys={w:#x} client={},{}",
            if msg == 0x0201 { "DOWN" } else { "UP" },
            l as u16 as i16,
            (l >> 16) as u16 as i16
        )),
        0x0200 => log(format!("{t} WM_MOUSEMOVE keys={w:#x} client={},{}", l as u16 as i16, (l >> 16) as u16 as i16)),
        0x0021 => log(format!("{t} WM_MOUSEACTIVATE top={} hit={} msg={:#x}", tag(w), l & 0xFFFF, l >> 16)),
        _ => {}
    }
    logging_proc(h, msg, w, l)
}

fn register_mouse(e: &TestEnv, name: &str) {
    let spec = ClassSpec {
        name: name.encode_utf16().collect(),
        wndproc: mouse_proc as *const () as u64,
        unicode: true,
        ..ClassSpec::default()
    };
    assert!(matches!(e.call(Request::RegisterClass { spec }), Reply::Atom { .. }));
}

/// Run every mouse message of the thread through PeekMessage and
/// DispatchMessage.
fn pump_mouse(e: &TestEnv) {
    let mut m = CMsg::default();
    while peek_message(e, &mut m, 0, 0x0200, 0x020E, PM_REMOVE) {
        dispatch_message(e, &m);
    }
}

fn move_cursor(e: &TestEnv, x: i32, y: i32) {
    e.call(Request::Cursor { set: true, x, y });
}

#[test]
fn a_click_on_a_disabled_child_goes_to_its_parent() {
    // EnableWindow.yaml `mouse_disabled_child`, `mouse_enabled_child`.
    with_env(|e| {
        interactive(e);
        register_mouse(e, "Em");
        let a = create(e, 'A', TOP, "Em", WS_POPUP | WS_VISIBLE, 50, 50, 300, 200, 0, 0);
        let c = create(e, 'C', 0, "Em", WS_CHILD | WS_VISIBLE | WS_DISABLED, 10, 10, 100, 50, a, 1);
        move_cursor(e, 100, 80);
        pump_mouse(e);
        take_log();
        // WindowFromPoint's own hit test: the parent, never the disabled child.
        assert_eq!(super::from_point(e, 100, 80, 0), a as u32);
        mouse(e, 0, 0x2);
        mouse(e, 0, 0x4);
        pump_mouse(e);
        let log = take_log();
        assert!(log.iter().all(|l| l.starts_with("A ")), "C gets nothing: {log:?}");
        assert_eq!(
            log.iter().map(String::as_str).filter(|l| !l.contains("NCHITTEST")).collect::<Vec<_>>(),
            [
                "A WM_SETCURSOR window=A hit=1 msg=513",
                "A WM_LBUTTONDOWN keys=0x1 client=50,30",
                "A WM_SETCURSOR window=A hit=1 msg=514",
                "A WM_LBUTTONUP keys=0x0 client=50,30",
            ]
        );
        // WM_NCHITTEST for the button-down and for the button-up (the third
        // of the sheet is WindowFromPoint's own call, in user32).
        assert_eq!(log.iter().filter(|l| l.contains("NCHITTEST")).count(), 2, "{log:?}");
        // Enabled: the child gets it, its parent hears WM_SETCURSOR too.
        crate::user::focus::enable_window(e, c, true);
        take_log();
        mouse(e, 0, 0x2);
        mouse(e, 0, 0x4);
        pump_mouse(e);
        let log = take_log();
        std::eprintln!("{log:#?}");
        assert!(log.iter().any(|l| l == "C WM_LBUTTONDOWN keys=0x1 client=40,20"), "{log:?}");
        assert!(!log.iter().any(|l| l.starts_with("A WM_LBUTTON")), "{log:?}");
    });
}

#[test]
fn session_zero_minimize_flags_follow_the_command_and_the_visibility() {
    // ShowWindow.yaml `minimize`: WM_WINDOWPOSCHANGING 0x8160 / 0x8160,
    // WM_WINDOWPOSCHANGED 0x8164 (SW_SHOWMINNOACTIVE of a hidden window) /
    // 0x8120 (SW_MINIMIZE of a shown one).
    with_env(|e| {
        register(e, "Mn");
        let ow = WS_CAPTION | WS_SYSMENU | WS_THICKFRAME | crate::metrics::WS_MINIMIZEBOX | 0x0001_0000;
        let w = create(e, 'W', 0, "Mn", ow, 50, 40, 300, 200, 0, 0);
        take_log();
        let flags = || -> Vec<String> { take_log().into_iter().filter(|l| l.contains("WM_WINDOWPOS")).collect() };
        show_window(e, w, SW_SHOWMINNOACTIVE);
        assert_eq!(
            flags(),
            [
                "W WM_WINDOWPOSCHANGING pos=0,740 size=160,28 flags=0x8160",
                "W WM_WINDOWPOSCHANGED pos=0,740 size=160,28 flags=0x8164"
            ]
        );
        show_window(e, w, SW_RESTORE);
        take_log();
        show_window(e, w, SW_MINIMIZE);
        assert_eq!(
            flags(),
            [
                "W WM_WINDOWPOSCHANGING pos=0,740 size=160,28 flags=0x8160",
                "W WM_WINDOWPOSCHANGED pos=0,740 size=160,28 flags=0x8120"
            ]
        );
    });
}

#[test]
fn set_foreground_window_across_threads_follows_the_sheet() {
    // SetForegroundWindow.yaml `other_thread`: the target's thread gets the
    // activation itself, whichever other windows are visible meanwhile.
    use crate::user::focus::set_foreground_window;
    with_env(|e| {
        interactive(e);
        register(e, "Sfw");
        let a = create(e, 'A', TOP, "Sfw", WS_POPUP | WS_VISIBLE, 50, 50, 200, 150, 0, 0);
        e.tid.set(2);
        let t = create(e, 'T', 0, "Sfw", WS_POPUP | WS_VISIBLE, 300, 50, 150, 100, 0, 0);
        e.tid.set(3);
        let u = create(e, 'U', 0, "Sfw", WS_POPUP | WS_VISIBLE, 500, 50, 150, 100, 0, 0);
        e.tid.set(1);
        take_log();
        // A to T: A is deactivated (and loses the focus), T is active with the focus.
        assert!(set_foreground_window(e, t));
        assert_eq!(
            take_log(),
            ["A WM_NCACTIVATE w=0x0", "A WM_ACTIVATE w=0x0", "A WM_ACTIVATEAPP w=0x0", "A WM_KILLFOCUS",]
        );
        assert_eq!(fg(e), t as u32);
        assert_eq!(input_state(e), (0, 0, 0));
        e.tid.set(3);
        assert_eq!(input_state(e), (0, 0, 0), "the window above T was not handed the foreground on the way");
        e.tid.set(2);
        assert_eq!(input_state(e), (t as u32, t as u32, 0));
        // Again: nothing.
        assert!(set_foreground_window(e, t));
        assert!(take_log().is_empty());
        // Back to A from the main thread: A gets the whole activation sequence.
        e.tid.set(1);
        assert!(set_foreground_window(e, a));
        let log = take_log();
        // A is topmost and T is not: A needs no z-order change (no WM_WINDOWPOSCHANGED).
        assert_eq!(
            log,
            [
                "A WM_WINDOWPOSCHANGING pos=0,0 size=0,0 flags=0x3",
                "A WM_ACTIVATEAPP w=0x1",
                "A WM_NCACTIVATE w=0x1",
                "A WM_ACTIVATE w=0x1",
                "A WM_SETFOCUS",
            ]
        );
        assert!(log.iter().any(|l| l == "A WM_ACTIVATEAPP w=0x1"), "{log:?}");
        assert!(log.iter().any(|l| l == "A WM_NCACTIVATE w=0x1"), "{log:?}");
        assert!(log.iter().any(|l| l == "A WM_ACTIVATE w=0x1"), "{log:?}");
        assert!(log.iter().any(|l| l == "A WM_SETFOCUS"), "{log:?}");
        assert_eq!(fg(e), a as u32);
        // To U (a third thread's window) from A's thread, and U never took a
        // detour through another thread's window.
        e.tid.set(1);
        take_log();
        assert!(set_foreground_window(e, u));
        assert_eq!(fg(e), u as u32);
        e.tid.set(3);
        assert_eq!(input_state(e), (u as u32, u as u32, 0));
        e.tid.set(2);
        assert_eq!(input_state(e), (0, 0, 0), "T was not handed the foreground on the way");
    });
}

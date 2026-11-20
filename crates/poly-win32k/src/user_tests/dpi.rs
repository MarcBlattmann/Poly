//! DPI awareness at 200 percent and a scale change (ADR 0231), against a `Wm` in memory: the
//! numbers and message orders of `DisplayConfigSetDeviceInfo.yaml`, `GetWindowRect.yaml`
//! (`at_200_units`) and `SetProcessDpiAwarenessContext.yaml` (`at_200_values`).

use super::*;
use crate::guic;
use crate::metrics::Display;

extern "system" fn dpi_proc(h: u64, msg: u32, w: u64, l: u64) -> u64 {
    let Reply::Window { info } = env().call(Request::Window { hwnd: h }) else {
        return def_window_proc(env(), h, msg, w, l);
    };
    let kind = if info.style & WS_CHILD != 0 {
        "child"
    } else if info.style & 0x8000_0000 != 0 {
        "popup"
    } else {
        "top"
    };
    let (lo, hi) = (l as u16 as i16, (l >> 16) as u16 as i16);
    match msg {
        WM_GETDPISCALEDSIZE => {
            // SAFETY: a SIZE for this message.
            let s = unsafe { &*(l as *const CSize) };
            log(format!("{kind} WM_GETDPISCALEDSIZE dpi {w} size {}x{}", s.cx, s.cy));
            return 0;
        }
        WM_DPICHANGED => {
            // SAFETY: the suggested RECT.
            let r = unsafe { *(l as *const CRect) };
            log(format!(
                "{kind} WM_DPICHANGED dpi x {} y {} suggested {},{},{},{}",
                w & 0xFFFF,
                w >> 16,
                r.left,
                r.top,
                r.right,
                r.bottom
            ));
        }
        WM_DPICHANGED_BEFOREPARENT => log(format!("{kind} WM_DPICHANGED_BEFOREPARENT")),
        WM_DPICHANGED_AFTERPARENT => log(format!("{kind} WM_DPICHANGED_AFTERPARENT")),
        WM_SIZE => log(format!("{kind} WM_SIZE type {w} client {lo}x{hi}")),
        WM_MOVE => log(format!("{kind} WM_MOVE {lo},{hi}")),
        _ => {}
    }
    def_window_proc(env(), h, msg, w, l)
}

/// The oracle's session: 2560x1600 at 200 percent, the taskbar at the bottom.
fn session(e: &TestEnv, awareness: Option<u8>) {
    let mut wm = e.wm.borrow_mut();
    wm.set_system_dpi(192);
    wm.set_display(Display::interactive_at(2560, 1600, 200, true));
    drop(wm);
    if let Some(a) = awareness {
        assert_eq!(e.call(Request::Dpi { set: true, process: true, awareness: a }), Reply::Ok {});
    }
    let spec = ClassSpec {
        name: "PolyDpiWindow".encode_utf16().collect(),
        wndproc: dpi_proc as *const () as u64,
        unicode: true,
        ..ClassSpec::default()
    };
    assert!(matches!(e.call(Request::RegisterClass { spec }), Reply::Atom { .. }));
}

fn window(e: &TestEnv, style: u32, (x, y, cx, cy): (i32, i32, i32, i32), parent: u64) -> u64 {
    let spec = CreateSpec {
        class: ClassRef::Name { name: "PolyDpiWindow".encode_utf16().collect() },
        style,
        x,
        y,
        cx,
        cy,
        parent,
        instance: 0x1_4000_0000,
        ..CreateSpec::default()
    };
    let h = u64::from(create_window(e, spec, 0, 0));
    show_window(e, h, 4);
    h
}

fn pump(e: &TestEnv) {
    let mut m = CMsg::default();
    while peek_message(e, &mut m, 0, 0, 0, PM_REMOVE) {
        dispatch_message(e, &m);
    }
}

fn rects(e: &TestEnv, wins: &[u64]) -> Vec<String> {
    wins.iter()
        .map(|&h| {
            let Reply::Window { info } = e.call(Request::Window { hwnd: h }) else { panic!() };
            let (w, c) = (info.window, info.client);
            format!(
                "{} {},{},{},{} client {}x{}",
                info.dpi_value,
                w.left,
                w.top,
                w.right,
                w.bottom,
                c.width(),
                c.height()
            )
        })
        .collect()
}

fn screen(e: &TestEnv) -> (i32, i32, u32) {
    let Reply::Desktop { width, height, dpi, .. } = e.call(Request::Desktop {}) else { panic!() };
    (width, height, dpi)
}

const OVERLAPPED: u32 = 0x00CF_0000;
const CHILD: u32 = WS_CHILD | WS_VISIBLE;
const POPUP: u32 = 0x8000_0000 | WS_VISIBLE;

/// What `scale_change` saw: the messages, the rectangles and the screen before, down and back.
type Change = (Vec<String>, [Vec<String>; 3], [(i32, i32, u32); 3]);

/// The three windows of `dpi_change_*`: their rectangles before, after one step down and back.
fn scale_change(e: &TestEnv) -> Change {
    let top = window(e, OVERLAPPED, (100, 100, 400, 300), 0);
    let child = window(e, CHILD, (10, 10, 100, 50), top);
    let pop = window(e, POPUP, (600, 150, 250, 120), 0);
    pump(e);
    take_log();
    let wins = [top, child, pop];
    let before = rects(e, &wins);
    let s0 = screen(e);
    assert_eq!(e.call(Request::SetScale { step: 3 }), Reply::Ok {});
    pump(e);
    let mut log = take_log();
    let down = rects(e, &wins);
    let s1 = screen(e);
    log.push("--".into());
    assert_eq!(e.call(Request::SetScale { step: 4 }), Reply::Ok {});
    pump(e);
    log.extend(take_log());
    let back = rects(e, &wins);
    let s2 = screen(e);
    (log, [before, down, back], [s0, s1, s2])
}

#[test]
fn a_per_monitor_v2_window_follows_the_scale_with_its_messages() {
    with_env(|e| {
        session(e, Some(guic::AWARE_PER_MONITOR_V2));
        let (log, [before, down, back], s) = scale_change(e);
        assert_eq!(
            log,
            [
                "popup WM_GETDPISCALEDSIZE dpi 168 size 250x120",
                "popup WM_DPICHANGED dpi x 168 y 168 suggested 600,150,819,255",
                "popup WM_SIZE type 0 client 219x105",
                "top WM_GETDPISCALEDSIZE dpi 168 size 400x300",
                "child WM_DPICHANGED_BEFOREPARENT",
                "top WM_DPICHANGED dpi x 168 y 168 suggested 100,100,450,363",
                "top WM_MOVE 112,152",
                "top WM_SIZE type 0 client 376x236",
                "child WM_DPICHANGED_AFTERPARENT",
                "top WM_SIZE type 0 client 326x199",
                "--",
                "popup WM_GETDPISCALEDSIZE dpi 192 size 219x105",
                "popup WM_DPICHANGED dpi x 192 y 192 suggested 600,150,850,270",
                "popup WM_SIZE type 0 client 250x120",
                "top WM_GETDPISCALEDSIZE dpi 192 size 350x263",
                "child WM_DPICHANGED_BEFOREPARENT",
                "top WM_DPICHANGED dpi x 192 y 192 suggested 100,100,500,400",
                "top WM_MOVE 113,158",
                "top WM_SIZE type 0 client 324x192",
                "child WM_DPICHANGED_AFTERPARENT",
                "top WM_SIZE type 0 client 374x229",
            ]
        );
        assert_eq!(
            before,
            [
                "192 100,100,500,400 client 374x229",
                "192 123,168,223,218 client 100x50",
                "192 600,150,850,270 client 250x120"
            ]
        );
        assert_eq!(
            down,
            [
                "168 100,100,450,363 client 326x199",
                "168 122,162,222,212 client 100x50",
                "168 600,150,819,255 client 219x105"
            ]
        );
        assert_eq!(back, before);
        // The system DPI and the screen stay (GetDpiForSystem 192, 2560x1600).
        assert_eq!(s, [(2560, 1600, 192); 3]);
    });
}

#[test]
fn a_system_aware_window_is_scaled_by_the_system() {
    with_env(|e| {
        session(e, Some(guic::AWARE_SYSTEM));
        let (log, [before, down, back], s) = scale_change(e);
        assert_eq!(
            log,
            ["popup WM_MOVE 686,171", "top WM_MOVE 129,172", "--", "popup WM_MOVE 600,150", "top WM_MOVE 113,158"]
        );
        assert_eq!(
            before,
            [
                "192 100,100,500,400 client 374x229",
                "192 123,168,223,218 client 100x50",
                "192 600,150,850,270 client 250x120"
            ]
        );
        assert_eq!(
            down,
            [
                "192 116,114,516,414 client 374x229",
                "192 139,182,239,232 client 100x50",
                "192 686,171,936,291 client 250x120"
            ]
        );
        assert_eq!(back, before);
        assert_eq!(s, [(2560, 1600, 192), (2926, 1829, 192), (2560, 1600, 192)]);
    });
}

#[test]
fn an_unaware_window_lives_in_half_pixels_and_is_scaled_by_the_system() {
    with_env(|e| {
        session(e, None);
        let (log, [before, down, back], s) = scale_change(e);
        assert_eq!(
            log,
            ["popup WM_MOVE 686,171", "top WM_MOVE 123,145", "--", "popup WM_MOVE 600,150", "top WM_MOVE 108,131"]
        );
        assert_eq!(
            before,
            [
                "96 100,100,500,400 client 384x261",
                "96 118,141,218,191 client 100x50",
                "96 600,150,850,270 client 250x120"
            ]
        );
        assert_eq!(
            down,
            [
                "96 115,114,515,414 client 384x261",
                "96 133,155,233,205 client 100x50",
                "96 686,171,936,291 client 250x120"
            ]
        );
        assert_eq!(back, before);
        assert_eq!(s, [(1280, 800, 96), (1463, 914, 96), (1280, 800, 96)]);
    });
}

#[test]
fn every_thread_reads_a_window_in_its_own_units() {
    // GetWindowRect.yaml `at_200_thread_contexts`: an aware window as an unaware thread reads it,
    // and an unaware one as an aware thread does.
    with_env(|e| {
        session(e, None);
        let unaware = window(e, OVERLAPPED, (100, 100, 400, 300), 0);
        assert_eq!(
            e.call(Request::Dpi { set: true, process: false, awareness: guic::AWARE_PER_MONITOR_V2 }),
            Reply::Value { value: 0 }
        );
        let aware = window(e, OVERLAPPED, (100, 100, 400, 300), 0);
        assert_eq!(
            rects(e, &[aware, unaware]),
            ["192 100,100,500,400 client 374x229", "96 200,200,1000,800 client 768x522"]
        );
        e.call(Request::Dpi { set: true, process: false, awareness: guic::AWARE_UNAWARE });
        assert_eq!(
            rects(e, &[aware, unaware]),
            ["192 50,50,250,200 client 187x114", "96 100,100,500,400 client 384x261"]
        );
        let Reply::Window { info } = e.call(Request::Window { hwnd: aware }) else { panic!() };
        assert_eq!((info.client.left, info.client.top), (57, 79));
        // The work area: 48 of the 96 pixels of the taskbar (GetMonitorInfoW.yaml at_200_values).
        let Reply::Desktop { work, monitor_dpi, phys_width, process_dpi, .. } = e.call(Request::Desktop {}) else {
            panic!()
        };
        assert_eq!((work.right, work.bottom, monitor_dpi, phys_width, process_dpi), (1280, 752, 96, 2560, 192));
        // The cursor clamps to the last physical pixel (GetCursorPos.yaml at_200_units).
        let Reply::Value { value } = e.call(Request::Cursor { set: true, x: 5000, y: 5000 }) else { panic!() };
        assert_eq!((value as u32, (value >> 32) as u32), (1280, 800));
        let Reply::Value { value } = e.call(Request::Cursor { set: true, x: 639, y: 359 }) else { panic!() };
        assert_eq!((value as u32, (value >> 32) as u32), (639, 359));
    });
}

#[test]
fn a_scale_outside_the_displays_steps_is_refused() {
    with_env(|e| {
        session(e, None);
        assert_eq!(e.call(Request::SetScale { step: 7 }), Reply::Error { code: 87 });
        assert_eq!(e.call(Request::SetScale { step: -1 }), Reply::Error { code: 87 });
        assert_eq!(e.call(Request::SetScale { step: 6 }), Reply::Ok {});
        let Reply::Desktop { scale, .. } = e.call(Request::Desktop {}) else { panic!() };
        assert_eq!(scale, 250);
    });
}

fn default_window(e: &TestEnv) -> (i32, i32) {
    let h = window(e, OVERLAPPED, (CW_USEDEFAULT, CW_USEDEFAULT, CW_USEDEFAULT, CW_USEDEFAULT), 0);
    let Reply::Window { info } = e.call(Request::Window { hwnd: h }) else { panic!() };
    (info.window.width(), info.window.height())
}

const CW_USEDEFAULT: i32 = i32::MIN;

#[test]
fn a_default_window_size_follows_the_callers_screen_and_work_area() {
    // GetWindowRect.yaml `at_200_units`: 960x543 for an unaware caller, 1920x1107 for an aware
    // one; CreateWindowExW.yaml `cw_usedefault`: 768x555 on the 1024x768 screen.
    with_env(|e| {
        session(e, None);
        assert_eq!(default_window(e), (960, 543));
        e.call(Request::Dpi { set: true, process: false, awareness: guic::AWARE_PER_MONITOR_V2 });
        assert_eq!(default_window(e), (1920, 1107));
        e.call(Request::Dpi { set: true, process: false, awareness: guic::AWARE_SYSTEM });
        assert_eq!(default_window(e), (1920, 1107));
    });
    with_env(|e| {
        let spec = ClassSpec {
            name: "PolyDpiWindow".encode_utf16().collect(),
            wndproc: dpi_proc as *const () as u64,
            unicode: true,
            ..ClassSpec::default()
        };
        assert!(matches!(e.call(Request::RegisterClass { spec }), Reply::Atom { .. }));
        assert_eq!(default_window(e), (768, 555));
    });
}

#[test]
fn a_search_finds_the_window_of_another_process_by_its_class() {
    with_env(|e| {
        session(e, None);
        // Process 9 registers a class of its own and shows a per-monitor-v2 popup.
        let other = Ctx { pid: 9, tid: 1, now: 0 };
        let mut wm = e.wm.borrow_mut();
        let fx = &mut Effects::default();
        wm.handle(other, Request::Dpi { set: true, process: true, awareness: guic::AWARE_PER_MONITOR_V2 }, fx);
        let name: Vec<u16> = "PolyXprocWin".encode_utf16().collect();
        let spec = ClassSpec { name: name.clone(), wndproc: 0x1000, unicode: true, ..ClassSpec::default() };
        assert!(matches!(wm.handle(other, Request::RegisterClass { spec }, fx), Reply::Atom { .. }));
        let spec = CreateSpec {
            class: ClassRef::Name { name: name.clone() },
            style: 0x8000_0000 | WS_VISIBLE,
            x: 50,
            y: 50,
            cx: 300,
            cy: 200,
            ..CreateSpec::default()
        };
        let Reply::Created { created } = wm.handle(other, Request::CreateWindow { spec }, fx) else { panic!() };
        wm.handle(
            other,
            Request::SetRects {
                hwnd: created.hwnd,
                window_rel: Some(Rect::new(50, 50, 350, 250)),
                client_rel: Some(Rect::new(50, 50, 350, 250)),
            },
            fx,
        );
        wm.handle(other, Request::FinishCreate { hwnd: created.hwnd }, fx);
        drop(wm);
        // This process (7, unaware) never registered the class.
        let found =
            e.call(Request::FindWindow { class: Some(ClassRef::Name { name }), title: None, parent: 0, after: 0 });
        assert_eq!(found, Reply::Handle { hwnd: created.hwnd });
        let none = e.call(Request::FindWindow {
            class: Some(ClassRef::Name { name: "NoSuchClass".encode_utf16().collect() }),
            title: None,
            parent: 0,
            after: 0,
        });
        assert_eq!(none, Reply::Handle { hwnd: 0 });
        // GetWindowRect.yaml `at_200_other_process`: half of it for an unaware caller.
        assert_eq!(rects(e, &[u64::from(created.hwnd)]), ["192 25,25,175,125 client 150x100"]);
    });
}

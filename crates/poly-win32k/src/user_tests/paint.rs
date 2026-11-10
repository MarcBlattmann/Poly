//! Held-out hunt, paint group: behaviours the spec sheets describe that the visible
//! tests do not cover (each test names the sheet and rule it follows).

use super::*;
use crate::wm::paint::{
    RDW_ALLCHILDREN, RDW_ERASE, RDW_ERASENOW, RDW_FRAME, RDW_INTERNALPAINT, RDW_INVALIDATE, RDW_NOCHILDREN,
    RDW_UPDATENOW, RDW_VALIDATE,
};

fn pending_rects(e: &TestEnv, h: u64) -> Vec<Rect> {
    paint::pending(e, h as u32).expect("window").rects
}

fn qs_paint(e: &TestEnv) -> u32 {
    let Reply::Bits { now, .. } = e.call(Request::QueueStatus { flags: 0x20, clear: false }) else { panic!() };
    now & 0x20
}

fn popup(e: &TestEnv, tag: char, class: &str, x: i32, y: i32) -> u64 {
    create(e, tag, WS_EX_NOACTIVATE, class, WS_POPUP | WS_VISIBLE, x, y, 100, 80, 0, 0)
}

#[test]
fn begin_paint_reports_what_is_pending_and_validates() {
    // BeginPaint.yaml `erase_sent`, `ferase`, `erase_any`, `frame`,
    // `outside_wm_paint`, `nested`, `hidden`, `bad_window`.
    with_env(|e| {
        interactive(e);
        register(e, "Bp");
        let w = popup(e, 'W', "Bp", 0, 0);
        // The window is new: the whole client area; a shown top-level window
        // has been erased already (nothing is sent now).
        let mut ps = paint::PaintStruct::default();
        take_log();
        let dc = paint::begin_paint(e, w, &mut ps);
        assert_ne!(dc, 0);
        assert_eq!(ps.hdc, dc, "returned DC equals ps->hdc");
        // The class has no brush: WM_ERASEBKGND returned 0, so fErase is 1.
        assert_eq!(take_log(), Vec::<String>::new());
        assert_eq!((ps.erase, ps.paint), (1, CRect::from(Rect::new(0, 0, 100, 80))));
        assert_eq!((ps.restore, ps.inc_update), (0, 0));
        assert!(pending_rects(e, w).is_empty(), "validated");
        assert_eq!(qs_paint(e), 0, "QS_PAINT is not in the queue");
        paint::end_paint(e, w, &ps);
        // No erase pending: fErase is 1, nothing is sent (outside WM_PAINT too).
        paint::invalidate(e, w, Some(&[Rect::new(10, 20, 30, 40)]), false);
        take_log();
        let mut ps = paint::PaintStruct::default();
        paint::begin_paint(e, w, &mut ps);
        assert_eq!(take_log(), Vec::<String>::new());
        assert_eq!((ps.erase, ps.paint), (1, CRect::from(Rect::new(10, 20, 30, 40))));
        // No WM_PAINT comes afterwards: the update region is empty.
        assert_eq!(qs_paint(e), 0);
        // Nothing invalid: an empty rectangle.
        let mut ps2 = paint::PaintStruct::default();
        paint::begin_paint(e, w, &mut ps2);
        assert_eq!(ps2.paint, CRect::from(Rect::new(0, 0, 0, 0)));
        // Nested: a second BeginPaint before EndPaint after invalidating again.
        paint::invalidate(e, w, Some(&[Rect::new(50, 40, 60, 50)]), false);
        let mut ps3 = paint::PaintStruct::default();
        paint::begin_paint(e, w, &mut ps3);
        assert_eq!(ps3.paint, CRect::from(Rect::new(50, 40, 60, 50)));
        // Several rectangles with erase TRUE among others with FALSE: one erase.
        paint::invalidate(e, w, Some(&[Rect::new(0, 0, 10, 10)]), false);
        paint::invalidate(e, w, Some(&[Rect::new(20, 20, 30, 30)]), true);
        paint::invalidate(e, w, Some(&[Rect::new(40, 40, 50, 50)]), false);
        take_log();
        let mut ps4 = paint::PaintStruct::default();
        paint::begin_paint(e, w, &mut ps4);
        assert_eq!(take_log(), ["W WM_ERASEBKGND"]);
        assert_eq!(ps4.paint, CRect::from(Rect::new(0, 0, 50, 50)));
        // A captioned window with a pending frame: WM_NCPAINT (1), then the erase.
        let f = create(e, 'F', WS_EX_NOACTIVATE, "Bp", WS_POPUP | WS_CAPTION | WS_VISIBLE, 200, 0, 150, 100, 0, 0);
        paint::redraw_window(e, f, None, RDW_VALIDATE | RDW_ALLCHILDREN);
        paint::redraw_window(e, f, None, RDW_INVALIDATE | RDW_ERASE | RDW_FRAME);
        take_log();
        paint::begin_paint(e, f, &mut paint::PaintStruct::default());
        assert_eq!(take_log(), ["F WM_NCPAINT w=0x1", "F WM_ERASEBKGND"]);
        // A hidden window: a DC, rcPaint empty, no message; fErase is the
        // window's state (BeginPaint.yaml `hidden`): 1 for a window that was
        // never erased, 0 after an erase that succeeded.
        let hid = create(e, 'H', WS_EX_NOACTIVATE, "Bp", WS_POPUP, 400, 0, 100, 80, 0, 0);
        take_log();
        let mut psh = paint::PaintStruct::default();
        let dc = paint::begin_paint(e, hid, &mut psh);
        assert_ne!(dc, 0);
        assert_eq!((psh.erase, psh.paint), (1, CRect::default()));
        assert_eq!(take_log(), Vec::<String>::new());
        // A made-up handle: NULL, ERROR_INVALID_WINDOW_HANDLE, the PAINTSTRUCT zeroed.
        for bad in [0, 0x1234_5678] {
            e.err.set(0);
            let mut psb = paint::PaintStruct { hdc: 5, erase: 1, ..Default::default() };
            assert_eq!(paint::begin_paint(e, bad, &mut psb), 0);
            assert_eq!(e.err.get(), ERROR_INVALID_WINDOW_HANDLE);
            assert_eq!((psb.hdc, psb.erase, psb.paint), (0, 0, CRect::default()));
        }
    });
}

fn gur(e: &TestEnv, h: u64, erase: bool) -> (bool, Rect) {
    paint::get_update_rect(e, h, erase).expect("window")
}

#[test]
fn get_update_rect_follows_visibility_position_and_state() {
    // GetUpdateRect.yaml `new_window`, `hidden`, `hidden_parent`, `off_screen`,
    // `minimized`, `erase_true_none_pending`, `erase_false`, `bad_window`.
    with_env(|e| {
        interactive(e);
        register(e, "Gur");
        // A new visible window: the whole client area before its first WM_PAINT.
        let w = popup(e, 'W', "Gur", 100, 100);
        assert_eq!(gur(e, w, false), (true, Rect::new(0, 0, 100, 80)));
        paint::redraw_window(e, w, None, RDW_VALIDATE);
        assert_eq!(gur(e, w, false), (false, Rect::default()));
        // erase TRUE with no erase pending sends nothing; with an erase
        // pending and bErase FALSE nothing, the erase stays for BeginPaint.
        paint::invalidate(e, w, Some(&[Rect::new(5, 6, 7, 8)]), false);
        take_log();
        assert_eq!(gur(e, w, true), (true, Rect::new(5, 6, 7, 8)));
        assert_eq!(take_log(), Vec::<String>::new());
        paint::invalidate(e, w, Some(&[Rect::new(5, 6, 7, 8)]), true);
        assert_eq!(gur(e, w, false), (true, Rect::new(5, 6, 7, 8)));
        assert_eq!(take_log(), Vec::<String>::new());
        assert!(paint::pending(e, w as u32).unwrap().erase);
        // erase TRUE sends it once; BeginPaint then sends none and its
        // fErase is 1 when the procedure returned 0.
        gur(e, w, true);
        assert_eq!(take_log(), ["W WM_ERASEBKGND"]);
        gur(e, w, true);
        assert_eq!(take_log(), Vec::<String>::new());
        let mut ps = paint::PaintStruct::default();
        paint::begin_paint(e, w, &mut ps);
        assert_eq!((take_log(), ps.erase), (Vec::<String>::new(), 1));
        // A hidden window has nothing invalid; showing it makes the client invalid.
        let hid = create(e, 'H', WS_EX_NOACTIVATE, "Gur", WS_POPUP, 300, 100, 100, 80, 0, 0);
        paint::invalidate(e, hid, None, true);
        assert_eq!(gur(e, hid, false), (false, Rect::default()));
        show_window(e, hid, SW_SHOWNA);
        assert_eq!(gur(e, hid, false), (true, Rect::new(0, 0, 100, 80)));
        // Hiding a window with an update region empties it.
        show_window(e, hid, SW_HIDE);
        assert_eq!(gur(e, hid, false), (false, Rect::default()));
        // A visible child of a hidden parent: InvalidateRect is 1, nothing
        // invalid; when the parent is shown the child is wholly invalid too.
        let p = create(e, 'P', WS_EX_NOACTIVATE, "Gur", WS_POPUP, 500, 100, 200, 150, 0, 0);
        let c = create(e, 'C', 0, "Gur", WS_CHILD | WS_VISIBLE, 10, 10, 50, 40, p, 1);
        assert!(paint::invalidate(e, c, None, true));
        assert_eq!(gur(e, c, false), (false, Rect::default()));
        show_window(e, p, SW_SHOWNA);
        assert_eq!(gur(e, c, false), (true, Rect::new(0, 0, 50, 40)));
        // GetUpdateRgn: a hidden window is NULLREGION after InvalidateRect.
        make_region(e, 0x61, &[Rect::new(1, 2, 3, 4)]);
        paint::invalidate(e, hid, None, false);
        assert_eq!(paint::get_update_rgn(e, hid, 0x61, false), 1);
        // Made-up handles: 0, 1400, the rectangle untouched (it is not written).
        for bad in [0, 0x1234_5678] {
            e.err.set(0);
            assert_eq!(paint::get_update_rect(e, bad, false), None);
            assert_eq!(e.err.get(), ERROR_INVALID_WINDOW_HANDLE);
        }
    });
}

#[test]
fn a_window_moved_off_the_screen_has_nothing_invalid_but_can_be_invalidated() {
    // GetUpdateRect.yaml `off_screen`.
    with_env(|e| {
        interactive(e);
        register(e, "Off");
        let w = popup(e, 'W', "Off", 100, 100);
        assert!(set_window_pos(e, w, 0, -500, -500, 0, 0, SWP_NOSIZE | SWP_NOZORDER | SWP_NOACTIVATE));
        assert_eq!(gur(e, w, false), (false, Rect::default()));
        assert!(paint::invalidate(e, w, None, false));
        assert_eq!(gur(e, w, false), (true, Rect::new(0, 0, 100, 80)));
        // It is painted: a WM_PAINT is pending for it.
        assert_eq!(qs_paint(e), 0x20);
        // Moving it back over the screen leaves it as it is.
        assert!(set_window_pos(e, w, 0, 50, 50, 0, 0, SWP_NOSIZE | SWP_NOZORDER | SWP_NOACTIVATE));
        assert_eq!(gur(e, w, false), (true, Rect::new(0, 0, 100, 80)));
        // Partly on the screen: not touched.
        paint::redraw_window(e, w, None, RDW_VALIDATE);
        paint::invalidate(e, w, Some(&[Rect::new(0, 0, 5, 5)]), false);
        assert!(set_window_pos(e, w, 0, -50, 10, 0, 0, SWP_NOSIZE | SWP_NOZORDER | SWP_NOACTIVATE));
        assert_eq!(gur(e, w, false), (true, Rect::new(0, 0, 5, 5)));
    });
}

#[test]
fn a_minimized_window_gets_a_wm_paint_with_an_empty_rectangle() {
    // GetUpdateRect.yaml `minimized`.
    with_env(|e| {
        interactive(e);
        register(e, "MinP");
        let style = WS_CAPTION | WS_SYSMENU | WS_THICKFRAME | crate::metrics::WS_MINIMIZEBOX;
        let m = create(e, 'M', WS_EX_NOACTIVATE, "MinP", style | WS_VISIBLE, 300, 100, 300, 200, 0, 0);
        show_window(e, m, SW_SHOWMINNOACTIVE);
        paint::redraw_window(e, m, None, RDW_VALIDATE | RDW_ALLCHILDREN);
        assert!(paint::invalidate(e, m, None, false));
        assert_eq!(gur(e, m, false), (true, Rect::default()));
        assert_eq!(qs_paint(e), 0x20);
        take_log();
        let mut ps = paint::PaintStruct::default();
        paint::begin_paint(e, m, &mut ps);
        assert_eq!(ps.paint, CRect::default());
    });
}

#[test]
fn redraw_window_returns_one_for_any_window_state_and_flag() {
    // RedrawWindow.yaml `return`, `hidden`, `internalpaint`, `invalidate_and_validate`.
    with_env(|e| {
        interactive(e);
        register(e, "Rwr");
        let w = popup(e, 'W', "Rwr", 0, 0);
        let hid = create(e, 'H', WS_EX_NOACTIVATE, "Rwr", WS_POPUP, 300, 0, 100, 80, 0, 0);
        take_log();
        for flags in [0, RDW_INVALIDATE, RDW_VALIDATE, RDW_INTERNALPAINT, RDW_UPDATENOW, RDW_ERASENOW, 0xFFF] {
            for h in [w, hid] {
                e.err.set(0);
                assert!(paint::redraw_window(e, h, None, flags), "{h:#x} {flags:#x}");
                assert_eq!(e.err.get(), 0);
            }
        }
        // Hidden: nothing invalid, nothing sent.
        paint::redraw_window(e, hid, None, RDW_VALIDATE);
        take_log();
        assert!(paint::redraw_window(e, hid, None, RDW_INVALIDATE | RDW_ERASE | RDW_FRAME | RDW_UPDATENOW));
        assert_eq!(take_log(), Vec::<String>::new());
        assert!(pending_rects(e, hid).is_empty());
        // RDW_INTERNALPAINT: a WM_PAINT without an update region.
        paint::redraw_window(e, w, None, RDW_VALIDATE);
        assert_eq!(qs_paint(e), 0);
        paint::redraw_window(e, w, None, RDW_INTERNALPAINT);
        assert_eq!(qs_paint(e), 0x20);
        assert!(pending_rects(e, w).is_empty());
        // A made-up handle: 0 and 1400.
        e.err.set(0);
        assert!(!paint::redraw_window(e, 0x1234_5678, None, RDW_INVALIDATE));
        assert_eq!(e.err.get(), ERROR_INVALID_WINDOW_HANDLE);
        // NULL is the desktop window (Learn): 1, no error.
        e.err.set(0);
        assert!(paint::redraw_window(e, 0, None, RDW_INVALIDATE | RDW_UPDATENOW));
        assert_eq!(e.err.get(), 0);
        let _ = RDW_NOCHILDREN;
    });
}

#[test]
fn invalidate_rgn_reaches_children_hidden_windows_and_other_threads() {
    // InvalidateRgn.yaml `return`, `children`, `child`, `other_thread`, `hidden`.
    with_env(|e| {
        interactive(e);
        register(e, "Ir");
        make_region(e, 0x71, &[Rect::new(0, 0, 40, 40)]);
        // A hidden window: 1, no update region.
        let hid = create(e, 'H', WS_EX_NOACTIVATE, "Ir", WS_POPUP, 300, 0, 100, 80, 0, 0);
        assert!(paint::invalidate_rgn(e, hid, 0x71, true));
        assert!(pending_rects(e, hid).is_empty());
        // The parent's region also invalidates the visible children under it,
        // in their own coordinates; a child's does not reach its parent.
        let p = create(e, 'P', WS_EX_NOACTIVATE, "Ir", WS_POPUP | WS_VISIBLE, 0, 0, 200, 150, 0, 0);
        let c = create(e, 'C', 0, "Ir", WS_CHILD | WS_VISIBLE, 20, 20, 50, 50, p, 1);
        paint::redraw_window(e, p, None, RDW_VALIDATE | RDW_ALLCHILDREN);
        assert!(paint::invalidate_rgn(e, p, 0x71, false));
        assert_eq!(pending_rects(e, p), [Rect::new(0, 0, 40, 40)]);
        assert_eq!(pending_rects(e, c), [Rect::new(0, 0, 20, 20)]);
        paint::redraw_window(e, p, None, RDW_VALIDATE | RDW_ALLCHILDREN);
        make_region(e, 0x72, &[Rect::new(30, 30, 90, 90)]);
        assert!(paint::invalidate_rgn(e, c, 0x72, false));
        assert!(pending_rects(e, p).is_empty());
        assert_eq!(pending_rects(e, c), [Rect::new(30, 30, 50, 50)], "clipped to the child's client area");
        // The region is copied: deleting or changing it afterwards changes nothing.
        e.regions.borrow_mut().retain(|r| r.0 != 0x72);
        assert_eq!(pending_rects(e, c), [Rect::new(30, 30, 50, 50)]);
        // A window of another thread: its update region is changed, 1.
        e.tid.set(2);
        let o = create(e, 'O', WS_EX_NOACTIVATE, "Ir", WS_POPUP | WS_VISIBLE, 400, 0, 100, 80, 0, 0);
        paint::redraw_window(e, o, None, RDW_VALIDATE);
        e.tid.set(1);
        assert!(paint::invalidate_rgn(e, o, 0x71, false));
        assert_eq!(pending_rects(e, o), [Rect::new(0, 0, 40, 40)]);
    });
}

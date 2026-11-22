//! Table 1 (win32k, ADR 0128): `PolyUserCall` carries window-manager
//! requests through user memory, posting wakes the target thread's
//! registered event, the win32k lockdown refuses everything, unknown
//! numbers stay unimplemented.

use super::*;
use crate::STATUS_NOT_IMPLEMENTED;
use crate::win32k::{LOCKDOWN_STATUS, service_name};
use poly_abi::Outcome;
use poly_ntobj::status::STATUS_BUFFER_TOO_SMALL;
use poly_win32k::proto::{ClassSpec, Reply, Request};
use poly_win32k::services::Win32kService;
use poly_win32k::wire;

/// Make the system call `number` as a `win32u` stub would.
pub(super) fn raw(f: &mut Fake, number: u32, args: &[u64]) -> (Outcome, u32) {
    let mut regs = SyscallRegs { rax: u64::from(number), rsp: STACK, ..Default::default() };
    let slots = [&mut regs.r10, &mut regs.rdx, &mut regs.r8, &mut regs.r9];
    for (slot, &a) in slots.into_iter().zip(args) {
        *slot = a;
    }
    for (i, &a) in args.iter().enumerate().skip(4) {
        f.mem.write_bytes(STACK + 0x28 + 8 * (i as u64 - 4), &a.to_le_bytes()).unwrap();
    }
    let outcome = crate::dispatch_with(f, &mut regs);
    (outcome, regs.rax as u32)
}

/// One `PolyUserCall` round trip.
pub(super) fn user_call(f: &mut Fake, req: &Request, cap: usize) -> (NtStatus, Option<Reply>, u64) {
    let bytes = wire::encode(req);
    let req_ptr = f.put(&bytes);
    let out = f.alloc(cap.max(1));
    let len = f.put_u64(0);
    let (outcome, status) = raw(f, Win32kService::Call.number(), &[req_ptr, bytes.len() as u64, out, cap as u64, len]);
    assert_eq!(outcome, Outcome::Handled);
    let n = f.u64_at(len);
    let reply = (status == 0).then(|| wire::decode(&f.bytes_at(out, n as usize)).expect("a reply"));
    (NtStatus(status), reply, n)
}

#[test]
fn requests_round_trip_through_user_memory() {
    let mut f = Fake::new();
    let (s, r, _) = user_call(&mut f, &Request::Desktop {}, 256);
    assert_eq!(s, STATUS_SUCCESS);
    assert!(matches!(r, Some(Reply::Desktop { width: 1024, height: 768, .. })));
    let spec = ClassSpec { name: "K".encode_utf16().collect(), wndproc: 0x1000, ..ClassSpec::default() };
    let (_, r, _) = user_call(&mut f, &Request::RegisterClass { spec }, 256);
    assert!(matches!(r, Some(Reply::Atom { atom }) if atom >= 0xC000));
    // A reply that does not fit says how long it is.
    let (s, r, n) = user_call(&mut f, &Request::Desktop {}, 2);
    assert_eq!((s, r.is_none()), (STATUS_BUFFER_TOO_SMALL, true));
    assert!(n > 2);
    // Garbage is refused.
    let bad = f.put(&[0xEE]);
    let out = f.alloc(16);
    let len = f.put_u64(0);
    assert_eq!(
        raw(&mut f, Win32kService::Call.number(), &[bad, 1, out, 16, len]).1,
        poly_ntobj::status::STATUS_INVALID_PARAMETER.0
    );
}

#[test]
fn posting_sets_the_threads_registered_event() {
    let mut f = Fake::new();
    let ev = f.put_u64(0);
    assert_eq!(f.call(Service::CreateEvent, &[ev, 0x1F0003, 0, 1, 0]), STATUS_SUCCESS);
    let handle = f.u64_at(ev);
    let (_, r, _) = user_call(&mut f, &Request::RegisterEvent { handle }, 64);
    assert_eq!(r, Some(Reply::Ok {}));
    let (_, r, _) = user_call(&mut f, &Request::RegisterEvent { handle: 0 }, 64);
    assert_eq!(r, Some(Reply::Value { value: handle }));
    let (_, r, _) = user_call(&mut f, &Request::Post { hwnd: 0, msg: 0x401, w: 1, l: 2 }, 64);
    assert_eq!(r, Some(Reply::Ok {}));
    // The event is signalled now: a wait on it does not time out.
    let zero = f.put(&0i64.to_le_bytes());
    assert_eq!(f.call(Service::WaitForSingleObject, &[handle, 0, zero]), STATUS_SUCCESS);
}

#[test]
fn the_win32k_lockdown_refuses_table_one_only() {
    let mut f = Fake::new();
    f.with_state(|_, p, _| p.win32k_disabled = true);
    assert_eq!(raw(&mut f, Win32kService::Call.number(), &[0, 0, 0, 0, 0]), (Outcome::Handled, LOCKDOWN_STATUS.0));
    assert_eq!(raw(&mut f, 0x1FFF, &[]), (Outcome::Handled, LOCKDOWN_STATUS.0));
    let status = f.call(Service::YieldExecution, &[]);
    assert!(status == STATUS_SUCCESS || status == STATUS_NO_YIELD_PERFORMED);
}

#[test]
fn unknown_numbers_are_unimplemented_and_named_when_known() {
    let mut f = Fake::new();
    assert_eq!(
        raw(&mut f, 0x1FFF, &[]),
        (Outcome::Unimplemented { table: "win32k", number: 0xFFF }, STATUS_NOT_IMPLEMENTED)
    );
    assert_eq!(service_name(0), Some("PolyUserCall"));
    assert_eq!(service_name(0xFFF), None);
    assert!(!NtProcess::new(PID).win32k_disabled);
}

#[test]
fn a_shown_window_gets_a_surface_its_pixels_are_committed_to() {
    use poly_win32k::frame::{FrameMetrics, Rect};
    use poly_win32k::link::{FromCompositor, Request as LinkRequest, Stream, ToCompositor, VERSION, encode_all};
    use poly_win32k::proto::{ClassRef, CreateSpec};
    let mut f = Fake::new();
    let call = |f: &mut Fake, req: Request| user_call(f, &req, 4096).1.expect("a reply");
    // Without a compositor a window has no surface.
    let name: alloc::vec::Vec<u16> = "Fill".encode_utf16().collect();
    let spec = ClassSpec { name: name.clone(), wndproc: 0x1000, unicode: true, ..ClassSpec::default() };
    call(&mut f, Request::RegisterClass { spec });
    let spec =
        CreateSpec { class: ClassRef::Name { name }, style: 0x00CF_0000, cx: 300, cy: 200, ..CreateSpec::default() };
    let Reply::Created { created } = call(&mut f, Request::CreateWindow { spec }) else { panic!("not created") };
    let h = created.hwnd;
    let (window_rel, client_rel) = (Some(Rect::new(100, 50, 136, 99)), Some(Rect::new(108, 81, 128, 91)));
    call(&mut f, Request::SetRects { hwnd: h, window_rel, client_rel });
    call(&mut f, Request::FinishCreate { hwnd: h });
    let show = Request::SetState {
        hwnd: h,
        visible: Some(true),
        minimized: None,
        maximized: None,
        normal: None,
        size_move_pending: None,
    };
    call(&mut f, show);
    assert_eq!(call(&mut f, Request::Surface { hwnd: h }), Reply::Value { value: 0 });

    // The compositor greets win32k through the device: the window there
    // is replayed with a client-sized buffer.
    let hello =
        FromCompositor::Hello { version: VERSION, width: 1280, height: 800, scale: 1, frame: FrameMetrics::POLY };
    f.with_state(|sys, _, _| {
        let (win32k, objects) = (&mut sys.win32k, &mut sys.objects);
        win32k.link_receive(objects, &encode_all(&[hello]));
    });
    let (w, hgt) = (20u64, 10u64);
    assert_eq!(call(&mut f, Request::Surface { hwnd: h }), Reply::Value { value: w | (hgt << 32) });
    let records = |f: &mut Fake| {
        let mut s = Stream::<ToCompositor>::new();
        s.push(&f.with_state(|sys, _, _| sys.win32k.link_output()));
        s.map(|r| r.unwrap()).collect::<alloc::vec::Vec<_>>()
    };
    let r = records(&mut f);
    assert!(matches!(r[0], ToCompositor::Welcome { .. }));
    let (surface, buffer) = r
        .iter()
        .find_map(
            |x| if let ToCompositor::Buffer { surface, buffer, .. } = x { Some((*surface, *buffer)) } else { None },
        )
        .expect("a buffer");
    assert!(r.contains(&ToCompositor::Geometry { surface, x: 108, y: 81, width: 20, height: 10 }));

    // PolyUserCommitSurface: rows `stride` apart in the app's memory.
    let stride = w * 4 + 8;
    let mut px = alloc::vec![0u8; (stride * hgt) as usize];
    px[0..4].copy_from_slice(&[0x60, 0x50, 0x40, 0]);
    let last = ((hgt - 1) * stride) as usize;
    px[last..last + 4].copy_from_slice(&[1, 2, 3, 0]);
    let p = f.put(&px);
    let commit = Win32kService::CommitSurface.number();
    assert_eq!(raw(&mut f, commit, &[u64::from(h), p, w, hgt, stride]), (Outcome::Handled, 0));
    assert_eq!(records(&mut f), alloc::vec![ToCompositor::Damage { surface, x: 0, y: 0, width: 20, height: 10 }]);
    let (first, end) = f.with_state(|sys, _, _| {
        let rows = sys.win32k.link_rows(buffer, 0, 10).unwrap();
        (rows[0..4].to_vec(), rows[(9 * 20 * 4)..(9 * 20 * 4 + 4)].to_vec())
    });
    assert_eq!((first.as_slice(), end.as_slice()), (&[0x60, 0x50, 0x40, 0][..], &[1, 2, 3, 0][..]));
    // A size that is not the surface's is refused.
    assert_eq!(
        raw(&mut f, commit, &[u64::from(h), p, w - 1, hgt, stride]).1,
        poly_ntobj::status::STATUS_INVALID_PARAMETER.0
    );

    // PolyUserCommitSurfaceArea (ADR 0219): another process's copy, all grey, of which it drew
    // only columns 2..5 of rows 3..5. Only those reach the buffer and are damaged; the corners
    // committed above stay.
    let grey = alloc::vec![0x77u8; (stride * hgt) as usize];
    let g = f.put(&grey);
    let area = f.put(&[2i32, 3, 5, 5].iter().flat_map(|v| v.to_le_bytes()).collect::<alloc::vec::Vec<u8>>());
    let commit_area = Win32kService::CommitSurfaceArea.number();
    assert_eq!(raw(&mut f, commit_area, &[u64::from(h), g, w, hgt, stride, area]), (Outcome::Handled, 0));
    assert_eq!(records(&mut f), alloc::vec![ToCompositor::Damage { surface, x: 2, y: 3, width: 3, height: 2 }]);
    let rows = f.with_state(|sys, _, _| sys.win32k.link_rows(buffer, 0, 10).unwrap());
    let px_at = |x: usize, y: usize| rows[(y * 20 + x) * 4..(y * 20 + x) * 4 + 4].to_vec();
    assert_eq!(px_at(0, 0), [0x60, 0x50, 0x40, 0], "outside the area: as committed before");
    assert_eq!(px_at(0, 9), [1, 2, 3, 0]);
    assert_eq!((px_at(2, 3), px_at(4, 4)), (alloc::vec![0x77; 4], alloc::vec![0x77; 4]));
    assert_ne!(px_at(5, 4), alloc::vec![0x77; 4], "the right edge is exclusive");
    assert_ne!(px_at(2, 5), alloc::vec![0x77; 4], "the bottom edge is exclusive");
    // An empty area commits nothing; an area beyond the surface is cut to it.
    let empty = f.put(&[4i32, 4, 4, 9].iter().flat_map(|v| v.to_le_bytes()).collect::<alloc::vec::Vec<u8>>());
    assert_eq!(raw(&mut f, commit_area, &[u64::from(h), g, w, hgt, stride, empty]), (Outcome::Handled, 0));
    assert!(records(&mut f).is_empty());
    let wide = f.put(&[-5i32, 8, 99, 99].iter().flat_map(|v| v.to_le_bytes()).collect::<alloc::vec::Vec<u8>>());
    assert_eq!(raw(&mut f, commit_area, &[u64::from(h), g, w, hgt, stride, wide]), (Outcome::Handled, 0));
    assert_eq!(records(&mut f), alloc::vec![ToCompositor::Damage { surface, x: 0, y: 8, width: 20, height: 2 }]);

    // PolyUserPresentBuffer (ADR 0224): a pagefile section holding a frame becomes a layer buffer
    // of the window's surface; its dirty rectangle is the damage; nothing is copied. win32k's
    // reference keeps the section after the app closes its handle, and the compositor finds its
    // pages by the buffer; section 0 removes the layer and lets the section go.
    let out = f.alloc(8);
    let size = f.put_u64(4096);
    let attrs = [out, u64::from(poly_ntobj::access::SECTION_ALL_ACCESS), 0, size, 0x04, 0x0800_0000, 0];
    assert_eq!(f.call(crate::services::Service::CreateSection, &attrs), STATUS_SUCCESS);
    let section = f.u64_at(out);
    let sections = f.with_state(|sys, _, _| sys.section_count());
    let dirty = f.put(&[1i32, 2, 4, 6].iter().flat_map(|v| v.to_le_bytes()).collect::<alloc::vec::Vec<u8>>());
    let present = Win32kService::PresentBuffer.number();
    assert_eq!(raw(&mut f, present, &[u64::from(h), section, w, hgt, w * 4, dirty, 1]), (Outcome::Handled, 0));
    let r = records(&mut f);
    let ToCompositor::LayerBuffer { layer, buffer: lb, .. } = r[0] else { panic!("{r:?}") };
    assert_eq!(
        r,
        alloc::vec![
            ToCompositor::LayerBuffer { surface, layer, buffer: lb, width: 20, height: 10, stride: 80 },
            ToCompositor::LayerPlace { surface, layer, x: 0, y: 0, clip: (0, 0, 20, 10) },
            ToCompositor::LayerDamage { surface, layer, buffer: lb, x: 1, y: 2, width: 3, height: 4 },
        ]
    );
    // A frame larger than the section is refused.
    let big = [u64::from(h), section, w, 100, w * 4, 0, 0];
    assert_eq!(raw(&mut f, present, &big).1, poly_ntobj::status::STATUS_INVALID_PARAMETER.0);
    assert_eq!(f.call(crate::services::Service::Close, &[section]), STATUS_SUCCESS);
    assert_eq!(f.with_state(|sys, _, _| sys.section_count()), sections, "win32k's reference keeps it");
    assert!(f.with_state(|sys, _, _| sys.win32k.link_layer_backing(lb)).is_some());
    assert_eq!(raw(&mut f, present, &[u64::from(h), 0, 0, 0, 0, 0, 0]), (Outcome::Handled, 0));
    assert_eq!(records(&mut f), alloc::vec![ToCompositor::LayerGone { surface, layer }]);
    assert_eq!(f.with_state(|sys, _, _| sys.section_count()), sections - 1, "let go of");
    assert!(f.with_state(|sys, _, _| sys.win32k.link_layer_backing(lb)).is_none());

    // A compositor move updates Win32's screen geometry; its title-bar
    // close is queued as WM_CLOSE on the owning GUI thread.
    let surface = f.with_state(|sys, _, _| sys.win32k.surfaces.surface(h)).expect("a Windows surface");
    let configure = FromCompositor::Configure { surface, x: 200, y: 300, width: 20, height: 10 };
    f.with_state(|sys, _, _| {
        let (win32k, objects) = (&mut sys.win32k, &mut sys.objects);
        win32k.link_receive(objects, &encode_all(&[configure]));
    });
    let Reply::Window { info } = call(&mut f, Request::Window { hwnd: u64::from(h) }) else { panic!() };
    assert_eq!(info.client, Rect::new(200, 300, 220, 310));
    assert_eq!(info.window, Rect::new(199, 275, 221, 311));

    let close = FromCompositor::Request { surface, request: LinkRequest::Close };
    f.with_state(|sys, _, _| {
        let (win32k, objects) = (&mut sys.win32k, &mut sys.objects);
        win32k.link_receive(objects, &encode_all(&[close]));
    });
    let Reply::Message { msg } =
        call(&mut f, Request::Peek { filter: 0, min: 0, max: 0, flags: poly_win32k::wm::PM_REMOVE })
    else {
        panic!("the desktop close request did not reach the GUI thread")
    };
    assert_eq!((msg.hwnd, msg.msg), (h, poly_win32k::user::WM_CLOSE));
}

/// ADR 0219: a compositor whose Windows apps live in output pixels greets with the output in those
/// pixels and `Units`: the system DPI is 96 times the units, and the work area is below the
/// desktop's ridge in those pixels too. Without `Units` it stays 96. ADR 0231: a DPI-unaware
/// thread sees all of it in units of 96 DPI.
#[test]
fn the_desktop_links_units_set_the_system_dpi() {
    use poly_win32k::frame::FrameMetrics;
    use poly_win32k::link::{FromCompositor, VERSION, encode_all};
    for (units, dpi, ridge) in [(1u32, 96u32, 24), (2, 192, 48)] {
        let mut f = Fake::new();
        let hello = FromCompositor::Hello {
            version: VERSION,
            width: 1280 * units,
            height: 800 * units,
            scale: 2,
            frame: FrameMetrics::POLY,
        };
        let mut records = alloc::vec![hello];
        if units > 1 {
            records.push(FromCompositor::Units { units });
        }
        f.with_state(|sys, _, _| {
            let (win32k, objects) = (&mut sys.win32k, &mut sys.objects);
            win32k.link_receive(objects, &encode_all(&records));
        });
        // A DPI-unaware thread sees the screen in units of 96 DPI (ADR 0231)...
        let Reply::Desktop { width, height, work, dpi: got, phys_width, process_dpi, .. } =
            user_call(&mut f, &Request::Desktop {}, 4096).1.expect("a reply")
        else {
            panic!("not the desktop")
        };
        assert_eq!((width, height, got, work.top), (1280, 800, 96, 24));
        assert_eq!((phys_width, process_dpi), ((1280 * units) as i32, dpi));
        // ...and a per-monitor aware one in output pixels.
        user_call(&mut f, &Request::Dpi { set: true, process: true, awareness: 3 }, 4096);
        let Reply::Desktop { width, height, work, dpi: got, .. } =
            user_call(&mut f, &Request::Desktop {}, 4096).1.expect("a reply")
        else {
            panic!("not the desktop")
        };
        assert_eq!((width, height, got), ((1280 * units) as i32, (800 * units) as i32, dpi));
        assert_eq!(work.top, ridge);
    }
}

/// ADR 0249: a `Window` (or `Desktop`) request is answered without counting as a change of the
/// window manager, a change raises the generation before it is made, and `SharedPage` is 0 where
/// the kernel has no window-state page (this fake).
#[test]
fn reads_leave_the_window_state_generation_and_changes_raise_it() {
    use core::sync::atomic::{AtomicU64, Ordering};
    use poly_win32k::proto::{ClassRef, CreateSpec};
    static BUMPS: AtomicU64 = AtomicU64::new(0);
    fn bump() {
        BUMPS.fetch_add(1, Ordering::SeqCst);
    }
    let mut f = Fake::new();
    f.with_state(|sys, _, _| sys.win32k.wm.set_hook(bump));
    let call = |f: &mut Fake, req: Request| user_call(f, &req, 4096).1.expect("a reply");
    let name: alloc::vec::Vec<u16> = "Gen".encode_utf16().collect();
    call(
        &mut f,
        Request::RegisterClass { spec: ClassSpec { name: name.clone(), wndproc: 0x1000, ..ClassSpec::default() } },
    );
    let spec =
        CreateSpec { class: ClassRef::Name { name }, style: 0x00CF_0000, cx: 300, cy: 200, ..CreateSpec::default() };
    let Reply::Created { created } = call(&mut f, Request::CreateWindow { spec }) else { panic!("not created") };
    let h = u64::from(created.hwnd);
    let before = BUMPS.load(Ordering::SeqCst);
    assert!(matches!(call(&mut f, Request::Window { hwnd: h }), Reply::Window { .. }));
    assert!(matches!(call(&mut f, Request::Desktop {}), Reply::Desktop { .. }));
    assert_eq!(call(&mut f, Request::SharedPage {}), Reply::Value { value: 0 });
    assert_eq!(BUMPS.load(Ordering::SeqCst), before, "reads");
    call(&mut f, Request::SetText { hwnd: created.hwnd, text: "x".encode_utf16().collect() });
    assert!(BUMPS.load(Ordering::SeqCst) > before, "a change");
}

#[test]
fn clipboard_bridges_bidirectionally_over_desktop_link_without_echo() {
    use poly_win32k::clip::{
        self, CF_DIB, CF_DIBV5, CF_OEMTEXT, CF_TEXT, CF_UNICODETEXT, ClipOp, ClipRes, WM_CLIPBOARDUPDATE,
    };
    use poly_win32k::dib;
    use poly_win32k::frame::FrameMetrics;
    use poly_win32k::link::{
        ClipAssembly, ClipFormat, FromCompositor, Stream, ToCompositor, VERSION, encode_all,
        encode_from_compositor_clip,
    };
    use poly_win32k::proto::{ClassRef, CreateSpec};

    let mut f = Fake::new();
    let call = |f: &mut Fake, req: Request| user_call(f, &req, 512).1.expect("a reply");
    let clip = |f: &mut Fake, op: ClipOp| match call(f, Request::Clip { op }) {
        Reply::Clip { res } => res,
        other => panic!("unexpected {other:?}"),
    };
    let take_records = |f: &mut Fake| {
        let mut s = Stream::<ToCompositor>::new();
        s.push(&f.with_state(|sys, _, _| sys.win32k.link_output()));
        s.map(Result::unwrap).collect::<alloc::vec::Vec<_>>()
    };

    // Create a window and register it as a clipboard format listener.
    let name: alloc::vec::Vec<u16> = "ClipWin".encode_utf16().collect();
    call(
        &mut f,
        Request::RegisterClass { spec: ClassSpec { name: name.clone(), wndproc: 0x1000, ..ClassSpec::default() } },
    );
    let spec =
        CreateSpec { class: ClassRef::Name { name }, style: 0x00CF_0000, cx: 100, cy: 100, ..CreateSpec::default() };
    let Reply::Created { created } = call(&mut f, Request::CreateWindow { spec }) else { panic!() };
    let hwnd = created.hwnd;
    assert_eq!(clip(&mut f, ClipOp::Listener { hwnd, add: true }), ClipRes::Ok {});

    // Connect compositor.
    let hello =
        FromCompositor::Hello { version: VERSION, width: 800, height: 600, scale: 1, frame: FrameMetrics::POLY };
    f.with_state(|sys, _, _| {
        let (win32k, objects) = (&mut sys.win32k, &mut sys.objects);
        win32k.link_receive(objects, &encode_all(&[hello]));
    });
    let _ = take_records(&mut f);

    // 1. Windows -> Wayland: set Unicode text + DIB and close.
    let dib = dib::make32(2, 1, &[0x10, 0x20, 0x30, 0xFF, 0x40, 0x50, 0x60, 0xFF]);
    assert_eq!(clip(&mut f, ClipOp::Open { hwnd }), ClipRes::Ok {});
    assert!(matches!(clip(&mut f, ClipOp::Empty {}), ClipRes::Val { .. }));
    assert_eq!(
        clip(&mut f, ClipOp::Set { format: CF_UNICODETEXT, handle: 1, data: Some(clip::from_utf8(b"Hello\nWorld")) }),
        ClipRes::Ok {}
    );
    assert_eq!(clip(&mut f, ClipOp::Set { format: CF_DIB, handle: 2, data: Some(dib.clone()) }), ClipRes::Ok {});
    assert_eq!(clip(&mut f, ClipOp::Close {}), ClipRes::Ok {});

    let recs = take_records(&mut f);
    let mut asm = ClipAssembly::new();
    let mut payload = None;
    for r in recs {
        match r {
            ToCompositor::ClipData { seq, format, offset, total, data } => asm.push(seq, format, offset, total, &data),
            ToCompositor::ClipCommit { seq } => payload = asm.commit(seq),
            ToCompositor::ClipClear { seq } => payload = Some(asm.clear(seq)),
            _ => {}
        }
    }
    let payload = payload.expect("exported clipboard payload");
    assert_eq!(payload.text.as_deref(), Some(&b"Hello\nWorld"[..]));
    assert_eq!(payload.dib.as_deref(), Some(dib.as_slice()));
    // Drain WM_CLIPBOARDUPDATE from our own write.
    let _ = call(&mut f, Request::Peek { filter: 0, min: 0, max: 0, flags: poly_win32k::wm::PM_REMOVE });

    // 2. Wayland -> Windows: send text + DIB from compositor.
    let ClipRes::Val { value: seq_before } = clip(&mut f, ClipOp::Sequence {}) else { panic!() };
    let mut wire = alloc::vec::Vec::new();
    let dib2 = dib::make32(1, 2, &[0xAA, 0xBB, 0xCC, 0xFF, 0x11, 0x22, 0x33, 0xFF]);
    encode_from_compositor_clip(&mut wire, 10, Some("Linux 🌍\nLine 2".as_bytes()), Some(&dib2));
    f.with_state(|sys, _, _| {
        let (win32k, objects) = (&mut sys.win32k, &mut sys.objects);
        win32k.link_receive(objects, &wire);
    });

    // Must NOT echo back to the compositor!
    let echo = take_records(&mut f);
    assert!(
        !echo.iter().any(|r| matches!(
            r,
            ToCompositor::ClipClear { .. } | ToCompositor::ClipData { .. } | ToCompositor::ClipCommit { .. }
        )),
        "unexpected echo back to compositor: {echo:?}"
    );

    // Sequence number bumped, listener got WM_CLIPBOARDUPDATE, formats synthesized.
    let ClipRes::Val { value: seq_after } = clip(&mut f, ClipOp::Sequence {}) else { panic!() };
    assert!(seq_after > seq_before);
    let mut got_update = false;
    while let Reply::Message { msg } =
        call(&mut f, Request::Peek { filter: 0, min: 0, max: 0, flags: poly_win32k::wm::PM_REMOVE })
    {
        if msg.hwnd == hwnd && msg.msg == WM_CLIPBOARDUPDATE {
            got_update = true;
        }
    }
    assert!(got_update, "WM_CLIPBOARDUPDATE not posted to listener");

    assert_eq!(clip(&mut f, ClipOp::Open { hwnd: 0 }), ClipRes::Ok {});
    let ClipRes::Data { bytes, .. } = clip(&mut f, ClipOp::Get { format: CF_UNICODETEXT }) else { panic!() };
    assert_eq!(clip::to_utf8(CF_UNICODETEXT, &bytes).unwrap(), "Linux 🌍\nLine 2".as_bytes());
    assert_eq!(clip(&mut f, ClipOp::Available { format: CF_TEXT }), ClipRes::Val { value: 1 });
    assert_eq!(clip(&mut f, ClipOp::Available { format: CF_OEMTEXT }), ClipRes::Val { value: 1 });
    assert_eq!(clip(&mut f, ClipOp::Available { format: CF_DIBV5 }), ClipRes::Val { value: 1 });
    let ClipRes::Data { bytes: got_dib, .. } = clip(&mut f, ClipOp::Get { format: CF_DIB }) else { panic!() };
    assert_eq!(got_dib, dib2);
    assert_eq!(clip(&mut f, ClipOp::Close {}), ClipRes::Ok {});
    assert!(take_records(&mut f).is_empty(), "reading clipboard must not trigger export");

    // 3. Windows clears clipboard -> ClipClear emitted.
    assert_eq!(clip(&mut f, ClipOp::Open { hwnd }), ClipRes::Ok {});
    assert!(matches!(clip(&mut f, ClipOp::Empty {}), ClipRes::Val { .. }));
    assert_eq!(clip(&mut f, ClipOp::Close {}), ClipRes::Ok {});
    assert!(take_records(&mut f).iter().any(|r| matches!(r, ToCompositor::ClipClear { .. })));

    // 4. Wayland clears clipboard -> no echo.
    let mut clear_wire = alloc::vec::Vec::new();
    encode_from_compositor_clip(&mut clear_wire, 11, None, None);
    f.with_state(|sys, _, _| {
        let (win32k, objects) = (&mut sys.win32k, &mut sys.objects);
        win32k.link_receive(objects, &clear_wire);
    });
    assert!(take_records(&mut f).is_empty());

    // 5. Delayed rendering: does not block, exports empty until rendered.
    assert_eq!(clip(&mut f, ClipOp::Open { hwnd }), ClipRes::Ok {});
    assert!(matches!(clip(&mut f, ClipOp::Empty {}), ClipRes::Val { .. }));
    assert_eq!(clip(&mut f, ClipOp::Set { format: CF_UNICODETEXT, handle: 0, data: None }), ClipRes::Ok {});
    assert_eq!(clip(&mut f, ClipOp::Close {}), ClipRes::Ok {});
    assert!(take_records(&mut f).iter().any(|r| matches!(r, ToCompositor::ClipClear { .. })));
    // Once rendered, exports the materialized text.
    assert_eq!(clip(&mut f, ClipOp::Open { hwnd: 0 }), ClipRes::Ok {});
    assert_eq!(
        clip(&mut f, ClipOp::Get { format: CF_UNICODETEXT }),
        ClipRes::Render { owner: hwnd, format: CF_UNICODETEXT }
    );
    assert_eq!(
        clip(&mut f, ClipOp::Set { format: CF_UNICODETEXT, handle: 9, data: Some(clip::from_utf8(b"lazy")) }),
        ClipRes::Ok {}
    );
    let rendered_recs = take_records(&mut f);
    assert!(rendered_recs.iter().any(|r| matches!(
        r,
        ToCompositor::ClipData { format: ClipFormat::Utf8Text, data, .. } if data == b"lazy"
    )));
    assert_eq!(clip(&mut f, ClipOp::RenderDone { format: CF_UNICODETEXT }), ClipRes::Ok {});
    assert_eq!(clip(&mut f, ClipOp::Close {}), ClipRes::Ok {});
}

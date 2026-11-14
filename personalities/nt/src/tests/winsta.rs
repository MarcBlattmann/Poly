//! Window stations and desktops through `PolyUserCall` (ADR 0186): the rules of the sheets
//! `CreateWindowStationW`, `SetProcessWindowStation`, `CreateDesktopW`, `GetThreadDesktop` and
//! `GetUserObjectInformationW`, one test per group of rules.

use super::win32k::user_call;
use super::*;
use crate::winsta::Start;
use alloc::format;
use poly_security::token::{SE_GROUP_INTEGRITY, Token};
use poly_win32k::proto::{Reply, Request};
use poly_win32k::winsta::*;

/// A fake kernel whose process has its token (the real kernel gives every process one when it is made).
fn fake() -> Fake {
    let mut f = Fake::new();
    crate::security::process_token(&mut f.sys, PID, TID).unwrap();
    f
}

fn ask(f: &mut Fake, op: WinstaOp) -> WinstaRes {
    let (s, r, _) = user_call(f, &Request::Winsta { op }, 256);
    assert_eq!(s, STATUS_SUCCESS);
    match r {
        Some(Reply::Winsta { res }) => res,
        other => panic!("not a window station reply: {other:?}"),
    }
}

fn handle(r: WinstaRes) -> u64 {
    match r {
        WinstaRes::Handle { handle } | WinstaRes::HandleClear { handle } => handle,
        other => panic!("no handle: {other:?}"),
    }
}

fn err(r: WinstaRes) -> u32 {
    match r {
        WinstaRes::Err { code } => code,
        other => panic!("no error: {other:?}"),
    }
}

fn create_station(f: &mut Fake, name: &str, flags: u32, access: u32) -> WinstaRes {
    ask(f, WinstaOp::CreateStation { name: wide(name), flags, access, sd: 0 })
}

fn create_desktop(f: &mut Fake, name: &str, access: u32) -> WinstaRes {
    ask(f, WinstaOp::CreateDesktop { name: wide(name), has_device: false, flags: 0, access, sd: 0 })
}

fn info(f: &mut Fake, h: u64, index: i32, capacity: u32) -> WinstaRes {
    ask(f, WinstaOp::GetInfo { handle: h, index, capacity })
}

fn open_station(f: &mut Fake, name: &str, access: u32) -> WinstaRes {
    ask(f, WinstaOp::OpenStation { name: wide(name), inherit: false, access })
}

fn open_desktop(f: &mut Fake, name: &str, access: u32) -> WinstaRes {
    ask(f, WinstaOp::OpenDesktop { name: wide(name), flags: 0, inherit: false, access })
}

/// Make the process's current station `ws` (a handle).
fn switch(f: &mut Fake, ws: u64) {
    assert_eq!(ask(f, WinstaOp::SetProcessStation { handle: ws }), WinstaRes::Done {});
}

const GENERIC_ALL: u32 = 0x1000_0000;
const MAXIMUM_ALLOWED: u32 = 0x0200_0000;

#[test]
fn a_station_is_made_found_by_name_and_goes_with_its_last_handle() {
    let mut f = fake();
    assert_eq!(err(create_station(&mut f, "", 0, GENERIC_ALL)), ERROR_ACCESS_DENIED);
    assert_eq!(err(create_station(&mut f, "pa\\b", 0, GENERIC_ALL)), ERROR_PATH_NOT_FOUND);
    let a = create_station(&mut f, "polyws_create", 0, GENERIC_ALL);
    assert!(matches!(a, WinstaRes::HandleClear { .. }), "the last error is set to 0");
    let a = handle(a);
    let WinstaRes::Info { data } = info(&mut f, a, UOI_NAME, 256) else { panic!() };
    assert_eq!(data.len(), 28);
    // The same name again (in any case) is another handle to the same station.
    let b = handle(create_station(&mut f, "POLYWS_CREATE", 0, GENERIC_ALL));
    assert_ne!(a, b);
    assert_eq!(err(create_station(&mut f, "POLYWS_CREATE", CWF_CREATE_ONLY, GENERIC_ALL)), ERROR_ALREADY_EXISTS);
    // The other flags are accepted.
    for flags in [2, 0x100, 0x8000_0000] {
        assert!(matches!(create_station(&mut f, "polyws_flags", flags, GENERIC_ALL), WinstaRes::HandleClear { .. }));
    }
    let c = handle(open_station(&mut f, "polyws_create", WINSTA_READATTRIBUTES));
    assert_eq!(err(open_station(&mut f, "nope", 1)), ERROR_FILE_NOT_FOUND);
    // Closing every handle ends the station.
    for h in [a, b, c] {
        assert_eq!(ask(&mut f, WinstaOp::CloseStation { handle: h }), WinstaRes::Done {});
    }
    assert_eq!(err(open_station(&mut f, "polyws_create", 2)), ERROR_FILE_NOT_FOUND);
    // A closed handle is no handle.
    assert_eq!(err(ask(&mut f, WinstaOp::CloseStation { handle: a })), ERROR_INVALID_HANDLE);
}

#[test]
fn a_station_handle_has_exactly_the_rights_asked_for() {
    let mut f = fake();
    let readable = |f: &mut Fake, access: u32| {
        let h = handle(create_station(f, "polyws_rights", 0, access));
        matches!(info(f, h, UOI_NAME, 256), WinstaRes::Info { .. })
    };
    assert!(readable(&mut f, WINSTA_ALL));
    assert!(readable(&mut f, MAXIMUM_ALLOWED));
    assert!(readable(&mut f, WINSTA_CREATEDESKTOP | WINSTA_READATTRIBUTES));
    assert!(!readable(&mut f, WINSTA_ENUMDESKTOPS));
    assert!(!readable(&mut f, 0));
    assert!(!readable(&mut f, 0x0002_0000 | 0x0004_0000));
}

#[test]
fn the_process_station_is_switched_and_cannot_be_closed_while_it_is_current() {
    let mut f = fake();
    let first = handle(ask(&mut f, WinstaOp::ProcessStation {}));
    assert_eq!(handle(ask(&mut f, WinstaOp::ProcessStation {})), first, "the same handle every time");
    let WinstaRes::Info { data } = info(&mut f, first, UOI_NAME, 256) else { panic!() };
    assert_eq!(data.len(), 16, "WinSta0");
    let alt = handle(create_station(&mut f, "polyws_alt", 0, GENERIC_ALL));
    switch(&mut f, alt);
    assert_eq!(handle(ask(&mut f, WinstaOp::ProcessStation {})), alt, "exactly that handle, not the first");
    // The current station's handle is busy.
    assert_eq!(err(ask(&mut f, WinstaOp::CloseStation { handle: alt })), ERROR_BUSY);
    // A desktop made now belongs to the switched-to station.
    let d = handle(create_desktop(&mut f, "polydesk_alt", GENERIC_ALL));
    assert!(matches!(open_desktop(&mut f, "polydesk_alt", 1), WinstaRes::Handle { .. }));
    switch(&mut f, first);
    assert_eq!(err(open_desktop(&mut f, "polydesk_alt", 1)), ERROR_FILE_NOT_FOUND);
    assert_eq!(ask(&mut f, WinstaOp::CloseStation { handle: alt }), WinstaRes::Done {}, "closes once switched away");
    // The refused handles: none, a desktop, a closed one.
    for bad in [0, d, alt] {
        assert_eq!(err(ask(&mut f, WinstaOp::SetProcessStation { handle: bad })), ERROR_INVALID_HANDLE);
    }
    assert_eq!(err(ask(&mut f, WinstaOp::CloseStation { handle: d })), ERROR_INVALID_HANDLE, "a desktop is no station");
    // The call does not look at the rights of the handle.
    let weak = handle(create_station(&mut f, "polyws_weak", 0, WINSTA_ENUMDESKTOPS));
    switch(&mut f, weak);
    switch(&mut f, first);
}

#[test]
fn desktops_have_their_rules() {
    let mut f = fake();
    let alt = handle(create_station(&mut f, "polyws_d", 0, GENERIC_ALL));
    let first = handle(ask(&mut f, WinstaOp::ProcessStation {}));
    switch(&mut f, alt);
    let make = |f: &mut Fake, name: &str, device: bool, flags: u32, access: u32| {
        ask(f, WinstaOp::CreateDesktop { name: wide(name), has_device: device, flags, access, sd: 0 })
    };
    assert_eq!(err(make(&mut f, "", false, 0, GENERIC_ALL)), ERROR_INVALID_HANDLE);
    assert_eq!(err(make(&mut f, "d", false, 0, 0)), ERROR_NOT_ENOUGH_MEMORY);
    assert_eq!(err(make(&mut f, "d", false, 0x100, GENERIC_ALL)), ERROR_INVALID_PARAMETER);
    assert_eq!(err(make(&mut f, "d", true, 0, GENERIC_ALL)), ERROR_INVALID_PARAMETER);
    let a = handle(make(&mut f, "polydesk_one", false, 0, GENERIC_ALL));
    assert!(matches!(info(&mut f, a, UOI_NAME, 256), WinstaRes::Info { data } if data.len() == 26));
    // The same name again, even with a narrower access, is another handle.
    let again = make(&mut f, "polydesk_one", false, DF_ALLOWOTHERACCOUNTHOOK, DESKTOP_READOBJECTS);
    assert_ne!(handle(again), a);
    // Chrome's narrow mask and the full and maximum ones give handles.
    let chrome = DESKTOP_CREATEWINDOW | DESKTOP_READOBJECTS | 0x0002_0000 | 0x0004_0000 | 0x0008_0000;
    for access in [chrome, DESKTOP_ALL, MAXIMUM_ALLOWED] {
        assert!(matches!(make(&mut f, "polydesk_masks", false, 0, access), WinstaRes::Handle { .. }));
    }
    // The call does not need rights on the station handle.
    let weak = handle(create_station(&mut f, "polyws_weak", 0, WINSTA_READATTRIBUTES));
    switch(&mut f, weak);
    assert!(matches!(make(&mut f, "polydesk_weak", false, 0, GENERIC_ALL), WinstaRes::Handle { .. }));
    switch(&mut f, first);
}

#[test]
fn a_threads_desktop_is_one_handle_that_cannot_be_closed_while_it_is_used() {
    let mut f = fake();
    let tid = TID.0 as u32;
    let d = handle(ask(&mut f, WinstaOp::ThreadDesktop { tid }));
    assert_eq!(handle(ask(&mut f, WinstaOp::ThreadDesktop { tid })), d);
    let WinstaRes::Info { data } = info(&mut f, d, UOI_TYPE, 256) else { panic!() };
    assert_eq!(data.len(), 16, "Desktop");
    // No such thread: nothing, and the last error is left alone.
    for bad in [0, 4, 99999] {
        assert_eq!(ask(&mut f, WinstaOp::ThreadDesktop { tid: bad }), WinstaRes::Silent {});
    }
    assert_eq!(err(ask(&mut f, WinstaOp::CloseDesktop { handle: d })), ERROR_BUSY);
    // Another desktop (of another station) and back.
    let ws = handle(create_station(&mut f, "polyws_t", 0, GENERIC_ALL));
    let first = handle(ask(&mut f, WinstaOp::ProcessStation {}));
    switch(&mut f, ws);
    let other = handle(create_desktop(&mut f, "polydesk_thr", GENERIC_ALL));
    switch(&mut f, first);
    assert_eq!(ask(&mut f, WinstaOp::SetThreadDesktop { handle: other }), WinstaRes::Done {});
    assert_eq!(handle(ask(&mut f, WinstaOp::ThreadDesktop { tid })), other);
    assert_eq!(err(ask(&mut f, WinstaOp::CloseDesktop { handle: other })), ERROR_BUSY);
    assert_eq!(ask(&mut f, WinstaOp::SetThreadDesktop { handle: d }), WinstaRes::Done {});
    assert_eq!(ask(&mut f, WinstaOp::CloseDesktop { handle: other }), WinstaRes::Done {});
    assert_eq!(err(ask(&mut f, WinstaOp::CloseDesktop { handle: other })), ERROR_INVALID_HANDLE);
    assert_eq!(err(ask(&mut f, WinstaOp::SetThreadDesktop { handle: 0 })), ERROR_INVALID_HANDLE);
    assert_eq!(err(ask(&mut f, WinstaOp::SetThreadDesktop { handle: ws })), ERROR_INVALID_HANDLE, "no desktop");
}

#[test]
fn a_desktop_keeps_its_station_alive() {
    let mut f = fake();
    let ws = handle(create_station(&mut f, "polyws_l", 0, GENERIC_ALL));
    let first = handle(ask(&mut f, WinstaOp::ProcessStation {}));
    switch(&mut f, ws);
    let d = handle(create_desktop(&mut f, "polydesk_l", GENERIC_ALL));
    switch(&mut f, first);
    // The station's handle goes, the desktop still holds the station.
    assert_eq!(ask(&mut f, WinstaOp::CloseStation { handle: ws }), WinstaRes::Done {});
    let again = open_station(&mut f, "polyws_l", 2);
    assert!(matches!(again, WinstaRes::Handle { .. }));
    assert_eq!(ask(&mut f, WinstaOp::CloseStation { handle: handle(again) }), WinstaRes::Done {});
    // The last handle of the desktop ends both.
    assert_eq!(ask(&mut f, WinstaOp::CloseDesktop { handle: d }), WinstaRes::Done {});
    assert_eq!(err(open_station(&mut f, "polyws_l", 2)), ERROR_FILE_NOT_FOUND);
}

#[test]
fn user_object_information_negotiates_sizes_and_checks_handles() {
    let mut f = fake();
    let ws = handle(create_station(&mut f, "polyws_create", 0, GENERIC_ALL));
    // Too small: error 122 with the size (this name has 13 characters and a NUL).
    assert_eq!(info(&mut f, ws, UOI_NAME, 4), WinstaRes::Short { needed: 28 });
    assert_eq!(info(&mut f, ws, UOI_NAME, 0), WinstaRes::Short { needed: 28 });
    assert_eq!(info(&mut f, ws, UOI_FLAGS, 11), WinstaRes::Short { needed: 12 });
    assert_eq!(info(&mut f, ws, UOI_TYPE, 2), WinstaRes::Short { needed: 28 });
    assert_eq!(info(&mut f, ws, UOI_FLAGS, 12), WinstaRes::Info { data: vec![0; 12] });
    assert_eq!(info(&mut f, ws, UOI_USER_SID, 0), WinstaRes::Info { data: vec![] });
    assert_eq!(err(info(&mut f, ws, UOI_HEAPSIZE, 4)), ERROR_INVALID_FUNCTION);
    assert_eq!(err(info(&mut f, ws, UOI_IO, 4)), ERROR_INVALID_FUNCTION);
    assert_eq!(err(info(&mut f, ws, 0, 4)), ERROR_INVALID_PARAMETER);
    assert_eq!(err(info(&mut f, ws, 99, 4)), ERROR_INVALID_PARAMETER);
    // A NULL handle fails and leaves the last error alone; other kinds of handle are error 1.
    assert_eq!(info(&mut f, 0, UOI_NAME, 256), WinstaRes::Silent {});
    assert_eq!(err(info(&mut f, u64::MAX, UOI_NAME, 256)), ERROR_INVALID_FUNCTION);
    // The desktop's own answers.
    let first = handle(ask(&mut f, WinstaOp::ProcessStation {}));
    switch(&mut f, ws);
    let d = handle(create_desktop(&mut f, "polydesk_info", DESKTOP_WRITEOBJECTS));
    switch(&mut f, first);
    assert!(matches!(info(&mut f, d, UOI_NAME, 256), WinstaRes::Info { .. }), "WRITEOBJECTS alone reads the name");
    assert_eq!(info(&mut f, d, UOI_IO, 4), WinstaRes::Info { data: vec![0; 4] });
    assert!(matches!(info(&mut f, d, UOI_HEAPSIZE, 4), WinstaRes::Info { .. }));
    // The station's flags can be set (and the name cannot).
    let mut flags = vec![0u8; 12];
    flags[0] = 1;
    assert_eq!(
        ask(&mut f, WinstaOp::SetInfo { handle: ws, index: UOI_FLAGS, data: flags.clone() }),
        WinstaRes::Done {}
    );
    assert_eq!(info(&mut f, ws, UOI_FLAGS, 12), WinstaRes::Info { data: flags.clone() });
    let set_name = ask(&mut f, WinstaOp::SetInfo { handle: ws, index: UOI_NAME, data: flags });
    assert_eq!(err(set_name), ERROR_INVALID_PARAMETER);
}

/// The process token of `f` lowered to integrity `rid`.
fn lower(f: &mut Fake, rid: u32) {
    let mut t = Token::poly_user();
    for g in t.groups.iter_mut().filter(|g| g.1 & SE_GROUP_INTEGRITY != 0) {
        g.0 = poly_security::sid::mandatory(rid);
    }
    let (id, _) = f.sys.objects.new_token(t, TID).unwrap();
    f.sys.objects.set_process_token(PID, id).unwrap();
}

fn sd(f: &mut Fake, sddl: &str) -> u64 {
    let bytes = poly_security::sdstr::parse(&wide(sddl)).expect("valid SDDL");
    f.put(&bytes)
}

/// The process as if the kernel had made it with `lpDesktop` = `spec`, connecting.
fn connect_after(f: &mut Fake, spec: &str) -> WinstaRes {
    f.sys.winsta.process_exited(&mut f.sys.objects, PID);
    f.sys.winsta.begin(PID, Start { spec: wide(spec), station: None, desktop: None });
    ask(f, WinstaOp::Connect {})
}

#[test]
fn a_process_started_on_a_named_station_and_desktop_needs_them_and_a_descriptor_for_its_token() {
    let mut f = fake();
    // Nothing named: it keeps what it inherits, whatever the token.
    assert_eq!(connect_after(&mut f, ""), WinstaRes::Done {});
    // Names that are not there.
    assert_eq!(connect_after(&mut f, "polyws_c0\\polydesk_c0"), WinstaRes::Err { code: STATUS_DLL_INIT_FAILED });
    // The descriptor of the station and desktop, and whether a Low token may start on them: the
    // creator's default security has the creator's integrity as its label, Everyone without a
    // label does not help a Low token, a label of its level or lower does.
    let cases = [
        ("", false),
        ("D:(A;;GA;;;WD)", false),
        ("D:(A;;GA;;;WD)S:(ML;;;;;LW)", true),
        ("D:(A;;GA;;;WD)S:(ML;;;;;S-1-16-0)", true),
    ];
    let first = handle(ask(&mut f, WinstaOp::ProcessStation {}));
    for (i, (sddl, _)) in cases.into_iter().enumerate() {
        let sd_at = if sddl.is_empty() { 0 } else { sd(&mut f, sddl) };
        let name = wide(&format!("polyws_c{i}"));
        let w = handle(ask(&mut f, WinstaOp::CreateStation { name, flags: 0, access: GENERIC_ALL, sd: sd_at }));
        switch(&mut f, w);
        let name = wide(&format!("polydesk_c{i}"));
        let made =
            ask(&mut f, WinstaOp::CreateDesktop { name, has_device: false, flags: 0, access: GENERIC_ALL, sd: sd_at });
        assert!(matches!(made, WinstaRes::Handle { .. }));
        switch(&mut f, first);
        // The token as it is lets every descriptor in.
        assert_eq!(connect_after(&mut f, &format!("polyws_c{i}\\polydesk_c{i}")), WinstaRes::Done {}, "case {i}");
    }
    lower(&mut f, 0x1000);
    for (i, (_, ok)) in cases.into_iter().enumerate() {
        let want = if ok { WinstaRes::Done {} } else { WinstaRes::Err { code: STATUS_DLL_INIT_FAILED } };
        assert_eq!(connect_after(&mut f, &format!("polyws_c{i}\\polydesk_c{i}")), want, "case {i}, Low");
    }
    // A desktop name alone is looked for in the process's own station.
    assert_eq!(connect_after(&mut f, "polydesk_c2"), WinstaRes::Err { code: STATUS_DLL_INIT_FAILED });
}

#[test]
fn a_desktop_the_descriptor_denies_its_maker_is_not_made() {
    let mut f = fake();
    // The sheet: this descriptor names neither the caller nor a group it is in.
    let at = sd(&mut f, "D:(A;;GA;;;SY)(A;;GWGR;;;S-1-15-2-1)S:(ML;;;;;S-1-16-0)");
    let name = wide("polyws_n");
    let ws = handle(ask(&mut f, WinstaOp::CreateStation { name, flags: 0, access: GENERIC_ALL, sd: at }));
    let first = handle(ask(&mut f, WinstaOp::ProcessStation {}));
    switch(&mut f, ws);
    // The station was made (its creator is not checked); the desktop is refused to its maker.
    let name = wide("polydesk_n");
    let r = ask(&mut f, WinstaOp::CreateDesktop { name, has_device: false, flags: 0, access: GENERIC_ALL, sd: at });
    assert_eq!(err(r), ERROR_ACCESS_DENIED);
    assert_eq!(err(open_desktop(&mut f, "polydesk_n", 1)), ERROR_FILE_NOT_FOUND);
    switch(&mut f, first);
}

#[test]
fn an_unreadable_descriptor_is_refused() {
    let mut f = fake();
    let junk = f.put(&[0xEE; 8]);
    let r = ask(&mut f, WinstaOp::CreateStation { name: wide("polyws_junk"), flags: 0, access: 1, sd: junk });
    assert_eq!(err(r), ERROR_INVALID_PARAMETER);
}

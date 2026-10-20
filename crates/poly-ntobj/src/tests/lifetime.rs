use super::*;

#[test]
fn temporary_named_object_goes_with_its_last_handle() {
    let (mut om, c) = setup();
    let before = om.object_count();
    let ev = event(&mut om, &c, "\\BaseNamedObjects\\Ev", 0).unwrap();
    let h2 = open(&mut om, &c, "\\BaseNamedObjects\\Ev", 0).unwrap();
    assert_eq!((om.handle_count(ev.object), om.pointer_count(ev.object)), (Some(2), Some(2)));
    om.nt_close(&c, ev.handle).unwrap();
    assert!(open(&mut om, &c, "\\BaseNamedObjects\\Ev", 0).is_ok_and(|h| om.nt_close(&c, h).is_ok()));
    om.nt_close(&c, h2).unwrap();
    assert!(!om.is_alive(ev.object));
    assert_eq!(open(&mut om, &c, "\\BaseNamedObjects\\Ev", 0), Err(STATUS_OBJECT_NAME_NOT_FOUND));
    assert_eq!(om.object_count(), before);
    // The name is free again.
    assert_eq!(event(&mut om, &c, "\\BaseNamedObjects\\Ev", 0).unwrap().status, STATUS_SUCCESS);
}

#[test]
fn pointer_references_keep_an_object_but_not_its_name() {
    let (mut om, c) = setup();
    let ev = event(&mut om, &c, "\\BaseNamedObjects\\Ev", 0).unwrap();
    let r = om.reference_by_handle(&c, ev.handle, EVENT_MODIFY_STATE, Some(ObjectType::Event)).unwrap();
    assert_eq!(om.pointer_count(r), Some(2));
    om.nt_close(&c, ev.handle).unwrap();
    // No handles: the name is gone, the object is not.
    assert!(om.is_alive(r));
    assert_eq!(om.object_name(r), None);
    assert_eq!(open(&mut om, &c, "\\BaseNamedObjects\\Ev", 0), Err(STATUS_OBJECT_NAME_NOT_FOUND));
    assert_eq!(om.set_event(r), Ok(false));
    om.reference(r).unwrap();
    om.dereference(r).unwrap();
    om.dereference(r).unwrap();
    assert!(!om.is_alive(r));
    assert_eq!(om.dereference(r), Err(STATUS_INVALID_PARAMETER));
    assert_eq!(om.reference(r), Err(STATUS_INVALID_PARAMETER));
}

#[test]
fn dereference_cannot_take_a_handle_s_reference() {
    let (mut om, c) = setup();
    let ev = event(&mut om, &c, "\\BaseNamedObjects\\Ev", 0).unwrap();
    assert_eq!(om.dereference(ev.object), Err(STATUS_INVALID_PARAMETER));
    assert_eq!(om.pointer_count(ev.object), Some(1));
}

#[test]
fn reference_by_handle_checks_type_and_access() {
    let (mut om, c) = setup();
    let ev = event(&mut om, &c, "\\BaseNamedObjects\\Ev", 0).unwrap();
    let q = open(&mut om, &c, "\\BaseNamedObjects\\Ev", 0).unwrap();
    om.nt_close(&c, q).unwrap();
    let name = w("\\BaseNamedObjects\\Ev");
    let q = om.nt_open_event(&c, &ObjectAttributes::named(&name, 0), EVENT_QUERY_STATE).unwrap();
    assert_eq!(om.reference_by_handle(&c, q, EVENT_MODIFY_STATE, None), Err(STATUS_ACCESS_DENIED));
    assert_eq!(om.reference_by_handle(&c, q, 0, Some(ObjectType::Mutant)), Err(STATUS_OBJECT_TYPE_MISMATCH));
    assert_eq!(om.reference_by_handle(&c, Handle(0x100), 0, None), Err(STATUS_INVALID_HANDLE));
    assert_eq!(om.pointer_count(ev.object), Some(2));
}

#[test]
fn permanent_objects_outlive_their_handles() {
    let (mut om, c) = setup();
    assert_eq!(event(&mut om, &c, "\\BaseNamedObjects\\Perm", OBJ_PERMANENT), Err(STATUS_PRIVILEGE_NOT_HELD));
    let privileged = Caller { privileges: Privileges { create_permanent: true, security: false }, ..c };
    let ev = event(&mut om, &privileged, "\\BaseNamedObjects\\Perm", OBJ_PERMANENT).unwrap();
    om.nt_close(&c, ev.handle).unwrap();
    assert!(om.is_alive(ev.object));
    assert_eq!((om.handle_count(ev.object), om.pointer_count(ev.object)), (Some(0), Some(0)));
    let h = open(&mut om, &c, "\\BaseNamedObjects\\Perm", 0).unwrap();
    assert_eq!(om.nt_query_object_basic(&c, h).unwrap().attributes, OBJ_PERMANENT);
    // Make it temporary: it goes when the last handle closes.
    om.nt_make_temporary_object(&c, h).unwrap();
    assert!(om.is_alive(ev.object));
    om.nt_close(&c, h).unwrap();
    assert!(!om.is_alive(ev.object));
}

#[test]
fn make_permanent_needs_the_privilege() {
    let (mut om, c) = setup();
    let ev = event(&mut om, &c, "\\BaseNamedObjects\\Ev", 0).unwrap();
    assert_eq!(om.nt_make_permanent_object(&c, ev.handle), Err(STATUS_PRIVILEGE_NOT_HELD));
    om.nt_make_permanent_object(&Caller::kernel(P1, T1), ev.handle).unwrap();
    om.nt_close(&c, ev.handle).unwrap();
    let id = lookup(&mut om, "\\BaseNamedObjects\\Ev").unwrap();
    assert_eq!(id, ev.object);
    // Temporary with no handles: gone at once.
    om.make_temporary(id).unwrap();
    assert!(!om.is_alive(id));
}

#[test]
fn unnamed_objects_ignore_permanent() {
    let (mut om, _) = setup();
    let k = Caller::kernel(P1, T1);
    let ev = om
        .nt_create_event(
            &k,
            &ObjectAttributes::unnamed(OBJ_PERMANENT),
            EVENT_ALL_ACCESS,
            EventType::Notification,
            false,
        )
        .unwrap();
    om.nt_close(&k, ev.handle).unwrap();
    assert!(!om.is_alive(ev.object));
}

#[test]
fn a_directory_outlives_the_names_in_it() {
    let (mut om, c) = setup();
    let before = om.object_count();
    let dir = w("\\BaseNamedObjects\\Dir");
    let d = om.nt_create_directory_object(&c, &ObjectAttributes::named(&dir, 0), DIRECTORY_ALL_ACCESS).unwrap();
    let ev = event(&mut om, &c, "\\BaseNamedObjects\\Dir\\Ev", 0).unwrap();
    assert_eq!((om.handle_count(d.object), om.pointer_count(d.object)), (Some(1), Some(2)));
    om.nt_close(&c, d.handle).unwrap();
    // The directory lost its name but the event still names it.
    assert!(om.is_alive(d.object));
    assert_eq!(open(&mut om, &c, "\\BaseNamedObjects\\Dir", 0), Err(STATUS_OBJECT_NAME_NOT_FOUND));
    assert_eq!(name_of(&om, ev.object).as_deref(), Some("\\Ev"));
    om.nt_close(&c, ev.handle).unwrap();
    assert!(!om.is_alive(ev.object));
    assert!(!om.is_alive(d.object));
    assert_eq!(om.object_count(), before);
}

#[test]
fn opaque_objects_report_their_deletion() {
    let (mut om, c) = setup();
    let k = Caller::kernel(P1, T1);
    let file = NewObject::Opaque { kind: ObjectType::File, cookie: 42, signaled: None, parse: false };
    let f = om.create_object(&k, &ObjectAttributes::unnamed(0), FILE_ALL_ACCESS, file, &mut NoParse).unwrap();
    assert_eq!(om.cookie(f.object), Some(42));
    assert_eq!(om.pop_deleted(), None);
    om.nt_close(&c, f.handle).unwrap();
    assert_eq!(om.pop_deleted(), Some(Deleted { id: f.object, kind: ObjectType::File, cookie: 42 }));
    assert_eq!(om.pop_deleted(), None);
    // Not every type is opaque.
    let bad = NewObject::Opaque { kind: ObjectType::Event, cookie: 0, signaled: None, parse: false };
    assert_eq!(om.new_object(bad, T1), Err(STATUS_INVALID_PARAMETER));
    // Events are not reported.
    let ev = unnamed_event(&mut om, &c, EventType::Notification, false);
    om.nt_close(&c, Handle(4)).unwrap();
    assert!(!om.is_alive(ev));
    assert_eq!(om.pop_deleted(), None);
}

#[test]
fn a_blocked_wait_keeps_its_objects_alive() {
    let (mut om, c) = setup();
    let ev = om
        .nt_create_event(&c, &ObjectAttributes::unnamed(0), EVENT_ALL_ACCESS, EventType::Notification, false)
        .unwrap();
    let w = blocked(om.nt_wait_for_single_object(&c, ev.handle, false, Timeout::Infinite));
    assert_eq!(om.pointer_count(ev.object), Some(2));
    om.nt_close(&c, ev.handle).unwrap();
    assert!(om.is_alive(ev.object));
    om.set_event(ev.object).unwrap();
    assert_eq!(woken(&mut om), [(T1, STATUS_WAIT_0)]);
    assert!(!om.wait_pending(w));
    assert!(!om.is_alive(ev.object));

    // Cancelling drops the references too.
    let ev = unnamed_event(&mut om, &c, EventType::Notification, false);
    let w = blocked(wait1(&mut om, T1, ev));
    om.nt_close(&c, Handle(4)).unwrap();
    assert_eq!(om.cancel_wait(w, WaitCancel::Timeout), Ok(STATUS_TIMEOUT));
    assert!(!om.is_alive(ev));
}

#[test]
fn process_exit_releases_its_handles() {
    let (mut om, c) = setup();
    let before = om.object_count();
    let a = event(&mut om, &c, "\\BaseNamedObjects\\A", 0).unwrap();
    let b = unnamed_event(&mut om, &c, EventType::Notification, false);
    let c2 = caller(P2, T2);
    let a2 = open(&mut om, &c2, "\\BaseNamedObjects\\A", 0).unwrap();
    om.destroy_process(P1).unwrap();
    assert!(om.is_alive(a.object));
    assert!(!om.is_alive(b));
    assert_eq!(om.process_handle_count(P1), None);
    om.nt_close(&c2, a2).unwrap();
    assert!(!om.is_alive(a.object));
    assert_eq!(om.object_count(), before);
}

#[test]
fn ids_of_deleted_objects_stay_dead() {
    let (mut om, c) = setup();
    let a = unnamed_event(&mut om, &c, EventType::Notification, false);
    om.nt_close(&c, Handle(4)).unwrap();
    let b = unnamed_event(&mut om, &c, EventType::Notification, false);
    assert_ne!(a, b);
    assert!(!om.is_alive(a));
    assert_eq!(om.set_event(a), Err(STATUS_INVALID_HANDLE));
    assert_eq!(om.object_type(a), None);
    assert_eq!(om.wait_objects(T1, &[a], WaitType::Any, false, Timeout::Zero), Err(STATUS_INVALID_HANDLE));
}

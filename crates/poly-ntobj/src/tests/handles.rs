use super::*;

fn unnamed(om: &mut ObjectManager, c: &Caller, access: AccessMask, attrs: u32) -> Created {
    om.nt_create_event(c, &ObjectAttributes::unnamed(attrs), access, EventType::Notification, false).unwrap()
}

#[test]
fn handle_values_are_multiples_of_four_and_reused_lowest_first() {
    let (mut om, c) = setup();
    let hs: Vec<Handle> = (0..3).map(|_| unnamed(&mut om, &c, EVENT_ALL_ACCESS, 0).handle).collect();
    assert_eq!(hs, [Handle(4), Handle(8), Handle(12)]);
    om.nt_close(&c, Handle(8)).unwrap();
    assert_eq!(unnamed(&mut om, &c, EVENT_ALL_ACCESS, 0).handle, Handle(8));
    assert_eq!(unnamed(&mut om, &c, EVENT_ALL_ACCESS, 0).handle, Handle(16));
    assert_eq!(om.process_handle_count(P1), Some(4));
}

#[test]
fn low_two_bits_are_ignored() {
    let (mut om, c) = setup();
    let ev = unnamed(&mut om, &c, EVENT_ALL_ACCESS, 0);
    for tag in 1..4 {
        let tagged = Handle(ev.handle.0 | tag);
        assert_eq!(om.object_by_handle(&c, tagged, EVENT_MODIFY_STATE, Some(ObjectType::Event)), Ok(ev.object));
        assert_eq!(om.nt_set_event(&c, tagged), Ok(tag != 1));
    }
    om.nt_close(&c, Handle(ev.handle.0 | 3)).unwrap();
    assert_eq!(om.nt_close(&c, ev.handle), Err(STATUS_INVALID_HANDLE));
}

#[test]
fn invalid_handles() {
    let (mut om, c) = setup();
    let ev = unnamed(&mut om, &c, EVENT_ALL_ACCESS, 0);
    for h in [Handle::NULL, Handle(3), Handle(8), Handle(0x1_0000_0000), Handle(u64::MAX - 7)] {
        assert_eq!(om.nt_close(&c, h), Err(STATUS_INVALID_HANDLE), "{h:?}");
        assert_eq!(om.nt_set_event(&c, h), Err(STATUS_INVALID_HANDLE), "{h:?}");
    }
    // Handles are per process.
    assert_eq!(om.nt_set_event(&caller(P2, T2), ev.handle), Err(STATUS_INVALID_HANDLE));
    // A process without a table.
    assert_eq!(om.nt_set_event(&caller(ProcessId(9), T2), ev.handle), Err(STATUS_INVALID_HANDLE));
    om.nt_close(&c, ev.handle).unwrap();
    assert_eq!(om.nt_close(&c, ev.handle), Err(STATUS_INVALID_HANDLE));
    assert_eq!(om.nt_set_event(&c, ev.handle), Err(STATUS_INVALID_HANDLE));
}

#[test]
fn access_is_checked_when_a_handle_is_used() {
    let (mut om, c) = setup();
    let ev = event(&mut om, &c, "\\BaseNamedObjects\\Ev", 0).unwrap();
    let name = w("\\BaseNamedObjects\\Ev");
    let attrs = ObjectAttributes::named(&name, 0);
    let query = om.nt_open_event(&c, &attrs, EVENT_QUERY_STATE).unwrap();
    assert_eq!(om.nt_set_event(&c, query), Err(STATUS_ACCESS_DENIED));
    assert_eq!(om.nt_reset_event(&c, query), Err(STATUS_ACCESS_DENIED));
    assert_eq!(om.nt_pulse_event(&c, query), Err(STATUS_ACCESS_DENIED));
    assert_eq!(om.nt_clear_event(&c, query), Err(STATUS_ACCESS_DENIED));
    assert!(om.nt_query_event(&c, query).is_ok());
    assert_eq!(om.nt_wait_for_single_object(&c, query, false, Timeout::Zero), Err(STATUS_ACCESS_DENIED));
    assert_eq!(om.nt_make_temporary_object(&c, query), Err(STATUS_ACCESS_DENIED));

    let modify = om.nt_open_event(&c, &attrs, EVENT_MODIFY_STATE).unwrap();
    assert_eq!(om.nt_set_event(&c, modify), Ok(false));
    assert_eq!(om.nt_query_event(&c, modify), Err(STATUS_ACCESS_DENIED));

    // Generic rights are mapped and stored mapped.
    let generic = om.nt_open_event(&c, &attrs, GENERIC_READ | GENERIC_EXECUTE).unwrap();
    assert_eq!(om.granted_access(&c, generic), Ok(READ_CONTROL | EVENT_QUERY_STATE | SYNCHRONIZE));
    assert_eq!(om.nt_wait_for_single_object(&c, generic, false, Timeout::Zero), done(STATUS_WAIT_0));
    // MAXIMUM_ALLOWED gets everything allowed.
    let max = om.nt_open_event(&c, &attrs, MAXIMUM_ALLOWED).unwrap();
    assert_eq!(om.granted_access(&c, max), Ok(EVENT_ALL_ACCESS));
    // The creator gets what it asked for.
    assert_eq!(om.granted_access(&c, ev.handle), Ok(EVENT_ALL_ACCESS));
    // A zero-access handle is a handle.
    let none = om.nt_open_event(&c, &attrs, 0).unwrap();
    assert_eq!(om.granted_access(&c, none), Ok(0));
}

#[test]
fn access_is_checked_when_a_handle_is_made() {
    let (mut om, c) = setup();
    let ev = event(&mut om, &c, "\\BaseNamedObjects\\Ev", 0).unwrap();
    om.set_allowed_access(ev.object, SYNCHRONIZE | EVENT_QUERY_STATE).unwrap();
    let name = w("\\BaseNamedObjects\\Ev");
    let attrs = ObjectAttributes::named(&name, 0);
    assert_eq!(om.nt_open_event(&c, &attrs, EVENT_MODIFY_STATE), Err(STATUS_ACCESS_DENIED));
    assert_eq!(om.nt_open_event(&c, &attrs, GENERIC_ALL), Err(STATUS_ACCESS_DENIED));
    let max = om.nt_open_event(&c, &attrs, MAXIMUM_ALLOWED).unwrap();
    assert_eq!(om.granted_access(&c, max), Ok(SYNCHRONIZE | EVENT_QUERY_STATE));
    // OPENIF goes through the same check.
    let openif = ObjectAttributes::named(&name, OBJ_OPENIF);
    assert_eq!(
        om.nt_create_event(&c, &openif, EVENT_ALL_ACCESS, EventType::Notification, false),
        Err(STATUS_ACCESS_DENIED)
    );
    // Kernel mode skips it, unless forced.
    let k = Caller::kernel(P1, T1);
    let h = om.nt_open_event(&k, &attrs, EVENT_MODIFY_STATE).unwrap();
    assert_eq!(om.granted_access(&k, h), Ok(EVENT_MODIFY_STATE));
    let forced = ObjectAttributes::named(&name, OBJ_FORCE_ACCESS_CHECK);
    assert_eq!(om.nt_open_event(&k, &forced, EVENT_MODIFY_STATE), Err(STATUS_ACCESS_DENIED));
    // ...and kernel-mode use of a handle is not checked.
    let none = om.nt_open_event(&c, &attrs, 0).unwrap();
    assert_eq!(om.nt_set_event(&k, none), Ok(false));
    assert_eq!(om.nt_set_event(&c, none), Err(STATUS_ACCESS_DENIED));
    // ACCESS_SYSTEM_SECURITY needs SeSecurityPrivilege.
    assert_eq!(om.nt_open_event(&c, &attrs, ACCESS_SYSTEM_SECURITY), Err(STATUS_PRIVILEGE_NOT_HELD));
}

#[test]
fn type_is_checked_when_a_handle_is_used() {
    let (mut om, c) = setup();
    let m = om.nt_create_mutant(&c, &ObjectAttributes::unnamed(0), MUTANT_ALL_ACCESS, false).unwrap();
    assert_eq!(om.nt_set_event(&c, m.handle), Err(STATUS_OBJECT_TYPE_MISMATCH));
    assert_eq!(om.nt_release_semaphore(&c, m.handle, 1), Err(STATUS_OBJECT_TYPE_MISMATCH));
    assert_eq!(om.nt_query_symbolic_link_object(&c, m.handle), Err(STATUS_OBJECT_TYPE_MISMATCH));
    assert_eq!(om.nt_query_directory_object(&c, m.handle), Err(STATUS_OBJECT_TYPE_MISMATCH));
    assert_eq!(om.nt_set_timer(&c, m.handle), Err(STATUS_OBJECT_TYPE_MISMATCH));
}

/// A process with a process object and a thread object registered.
fn with_process_objects(om: &mut ObjectManager) -> (ObjectId, ObjectId) {
    let process = om.new_object(
        NewObject::Opaque { kind: ObjectType::Process, cookie: 1, signaled: Some(false), parse: false },
        T1,
    );
    let thread = om
        .new_object(NewObject::Opaque { kind: ObjectType::Thread, cookie: 2, signaled: Some(false), parse: false }, T1);
    let (process, thread) = (process.unwrap(), thread.unwrap());
    om.set_process_object(P1, process).unwrap();
    om.set_thread_object(T1, Some(thread)).unwrap();
    (process, thread)
}

#[test]
fn pseudo_handles() {
    let (mut om, c) = setup();
    // Not registered yet.
    assert_eq!(om.nt_query_object_type(&c, Handle::CURRENT_PROCESS), Err(STATUS_INVALID_HANDLE));
    let (process, thread) = with_process_objects(&mut om);
    assert_eq!(Handle::CURRENT_PROCESS, Handle(-1i64 as u64));
    assert_eq!(Handle::CURRENT_THREAD, Handle(-2i64 as u64));
    assert_eq!(om.object_by_handle(&c, Handle::CURRENT_PROCESS, PROCESS_ALL_ACCESS, None), Ok(process));
    assert_eq!(om.object_by_handle(&c, Handle::CURRENT_THREAD, THREAD_ALL_ACCESS, None), Ok(thread));
    assert_eq!(om.nt_query_object_type(&c, Handle::CURRENT_PROCESS), Ok("Process"));
    assert_eq!(om.nt_query_object_type(&c, Handle::CURRENT_THREAD), Ok("Thread"));
    assert_eq!(om.granted_access(&c, Handle::CURRENT_PROCESS), Ok(PROCESS_ALL_ACCESS));
    // Closing one does nothing.
    assert_eq!(om.nt_close(&c, Handle::CURRENT_PROCESS), Ok(()));
    assert_eq!(om.nt_close(&c, Handle::CURRENT_THREAD), Ok(()));
    assert!(om.is_alive(process));
    // Duplicating one gives a real handle.
    let real = om.nt_duplicate_object(&c, P1, Handle::CURRENT_PROCESS, Some(P1), 0, 0, DUPLICATE_SAME_ACCESS).unwrap();
    let real = real.unwrap();
    assert!(!real.is_pseudo());
    assert_eq!(om.object_by_handle(&c, real, 0, None), Ok(process));
    assert_eq!(om.granted_access(&c, real), Ok(PROCESS_ALL_ACCESS));
    assert_eq!(om.handle_count(process), Some(1));
    // A pseudo handle is no use as a name or flags target.
    assert_eq!(om.set_handle_flags(&c, Handle::CURRENT_PROCESS, HandleFlags::default()), Err(STATUS_INVALID_HANDLE));
    // The thread object of another thread.
    assert_eq!(om.nt_query_object_type(&caller(P1, T2), Handle::CURRENT_THREAD), Err(STATUS_INVALID_HANDLE));
    assert_eq!(om.set_process_object(P1, thread), Err(STATUS_OBJECT_TYPE_MISMATCH));
    assert_eq!(om.set_thread_object(T1, Some(process)), Err(STATUS_OBJECT_TYPE_MISMATCH));
}

#[test]
fn duplicate_within_and_across_processes() {
    let (mut om, c) = setup();
    let ev = unnamed(&mut om, &c, EVENT_ALL_ACCESS, 0);
    let same = om.nt_duplicate_object(&c, P1, ev.handle, Some(P1), 0, 0, DUPLICATE_SAME_ACCESS).unwrap().unwrap();
    assert_ne!(same, ev.handle);
    assert_eq!(om.granted_access(&c, same), Ok(EVENT_ALL_ACCESS));
    assert_eq!(om.handle_count(ev.object), Some(2));
    // Less access.
    let less = om.nt_duplicate_object(&c, P1, ev.handle, Some(P2), SYNCHRONIZE, 0, 0).unwrap().unwrap();
    let c2 = caller(P2, T2);
    assert_eq!(om.granted_access(&c2, less), Ok(SYNCHRONIZE));
    assert_eq!(om.nt_set_event(&c2, less), Err(STATUS_ACCESS_DENIED));
    assert_eq!(om.object_by_handle(&c2, less, SYNCHRONIZE, None), Ok(ev.object));
    // More access than the source has (allowed by the object).
    let q = om.nt_duplicate_object(&c, P1, ev.handle, Some(P1), EVENT_QUERY_STATE, 0, 0).unwrap().unwrap();
    let more = om.nt_duplicate_object(&c, P1, q, Some(P1), EVENT_MODIFY_STATE, 0, 0).unwrap().unwrap();
    assert_eq!(om.granted_access(&c, more), Ok(EVENT_MODIFY_STATE));
    // ...but not more than the object allows.
    om.set_allowed_access(ev.object, EVENT_QUERY_STATE).unwrap();
    assert_eq!(om.nt_duplicate_object(&c, P1, q, Some(P1), EVENT_MODIFY_STATE, 0, 0), Err(STATUS_ACCESS_DENIED));
    assert_eq!(om.handle_count(ev.object), Some(5));
}

#[test]
fn duplicate_close_source() {
    let (mut om, c) = setup();
    let ev = unnamed(&mut om, &c, EVENT_ALL_ACCESS, 0);
    let moved = om
        .nt_duplicate_object(&c, P1, ev.handle, Some(P2), 0, 0, DUPLICATE_SAME_ACCESS | DUPLICATE_CLOSE_SOURCE)
        .unwrap()
        .unwrap();
    assert_eq!(om.nt_close(&c, ev.handle), Err(STATUS_INVALID_HANDLE));
    assert_eq!(om.handle_count(ev.object), Some(1));
    let c2 = caller(P2, T2);
    assert_eq!(om.object_by_handle(&c2, moved, 0, None), Ok(ev.object));

    // No target: only close.
    assert_eq!(om.nt_duplicate_object(&c2, P2, moved, None, 0, 0, DUPLICATE_CLOSE_SOURCE), Ok(None));
    assert!(!om.is_alive(ev.object));

    // The source is closed even when the duplication fails.
    let ev = unnamed(&mut om, &c, EVENT_ALL_ACCESS, 0);
    om.set_allowed_access(ev.object, 0).unwrap();
    assert_eq!(
        om.nt_duplicate_object(&c, P1, ev.handle, Some(P2), EVENT_MODIFY_STATE, 0, DUPLICATE_CLOSE_SOURCE),
        Err(STATUS_ACCESS_DENIED)
    );
    assert!(!om.is_alive(ev.object));
    let ev = unnamed(&mut om, &c, EVENT_ALL_ACCESS, 0);
    assert_eq!(
        om.nt_duplicate_object(&c, P1, ev.handle, Some(ProcessId(9)), 0, 0, DUPLICATE_CLOSE_SOURCE),
        Err(STATUS_INVALID_HANDLE)
    );
    assert!(!om.is_alive(ev.object));

    // A protected source stays open.
    let ev = unnamed(&mut om, &c, EVENT_ALL_ACCESS, 0);
    om.set_handle_flags(&c, ev.handle, HandleFlags { inherit: false, protect_from_close: true }).unwrap();
    let dup = om.nt_duplicate_object(&c, P1, ev.handle, Some(P1), 0, 0, DUPLICATE_SAME_ACCESS | DUPLICATE_CLOSE_SOURCE);
    assert!(dup.unwrap().is_some());
    assert_eq!(om.handle_count(ev.object), Some(2));
}

#[test]
fn duplicate_errors_and_attributes() {
    let (mut om, c) = setup();
    let ev = unnamed(&mut om, &c, EVENT_ALL_ACCESS, OBJ_INHERIT);
    assert_eq!(om.nt_duplicate_object(&c, P1, ev.handle, Some(P1), 0, 0, 0x8), Err(STATUS_INVALID_PARAMETER));
    assert_eq!(om.nt_duplicate_object(&c, P1, Handle(0x400), Some(P1), 0, 0, 0), Err(STATUS_INVALID_HANDLE));
    assert_eq!(om.nt_duplicate_object(&c, ProcessId(9), ev.handle, Some(P1), 0, 0, 0), Err(STATUS_INVALID_HANDLE));
    assert_eq!(om.nt_duplicate_object(&c, P1, ev.handle, None, 0, 0, 0), Err(STATUS_INVALID_PARAMETER));
    assert_eq!(om.handle_count(ev.object), Some(1));

    let same = om.nt_duplicate_object(&c, P1, ev.handle, Some(P1), 0, 0, DUPLICATE_SAME_ATTRIBUTES).unwrap().unwrap();
    assert_eq!(om.handle_flags(&c, same), Ok(HandleFlags { inherit: true, protect_from_close: false }));
    let given = om.nt_duplicate_object(&c, P1, ev.handle, Some(P1), 0, OBJ_PROTECT_CLOSE, 0).unwrap().unwrap();
    assert_eq!(om.handle_flags(&c, given), Ok(HandleFlags { inherit: false, protect_from_close: true }));
    assert_eq!(om.nt_close(&c, given), Err(STATUS_HANDLE_NOT_CLOSABLE));
}

#[test]
fn protect_from_close() {
    let (mut om, c) = setup();
    let ev = unnamed(&mut om, &c, EVENT_ALL_ACCESS, 0);
    om.set_handle_flags(&c, ev.handle, HandleFlags { inherit: false, protect_from_close: true }).unwrap();
    assert_eq!(om.nt_close(&c, ev.handle), Err(STATUS_HANDLE_NOT_CLOSABLE));
    assert_eq!(om.handle_count(ev.object), Some(1));
    let info = om.nt_query_object_basic(&c, ev.handle).unwrap();
    assert_eq!(info.attributes, OBJ_PROTECT_CLOSE);
    om.set_handle_flags(&c, ev.handle, HandleFlags::default()).unwrap();
    assert_eq!(om.nt_close(&c, ev.handle), Ok(()));
    assert!(!om.is_alive(ev.object));

    // Process exit closes protected handles too.
    let ev = unnamed(&mut om, &c, EVENT_ALL_ACCESS, 0);
    om.set_handle_flags(&c, ev.handle, HandleFlags { inherit: false, protect_from_close: true }).unwrap();
    om.destroy_process(P1).unwrap();
    assert!(!om.is_alive(ev.object));
    assert_eq!(om.destroy_process(P1), Err(STATUS_INVALID_PARAMETER));
}

#[test]
fn inheritance_copies_inheritable_handles_at_the_same_values() {
    let (mut om, c) = setup();
    let a = unnamed(&mut om, &c, EVENT_ALL_ACCESS, OBJ_INHERIT);
    let b = unnamed(&mut om, &c, EVENT_QUERY_STATE, 0);
    let d = unnamed(&mut om, &c, SYNCHRONIZE, 0);
    om.set_handle_flags(&c, d.handle, HandleFlags { inherit: true, protect_from_close: true }).unwrap();
    let child = ProcessId(3);
    om.create_process(child, Some(P1)).unwrap();
    let cc = caller(child, T3);
    assert_eq!(om.process_handle_count(child), Some(2));
    assert_eq!(om.object_by_handle(&cc, a.handle, EVENT_ALL_ACCESS, None), Ok(a.object));
    assert_eq!(om.object_by_handle(&cc, b.handle, 0, None), Err(STATUS_INVALID_HANDLE));
    assert_eq!(om.granted_access(&cc, d.handle), Ok(SYNCHRONIZE));
    assert_eq!(om.handle_flags(&cc, d.handle), Ok(HandleFlags { inherit: true, protect_from_close: true }));
    assert_eq!(om.handle_count(a.object), Some(2));
    // The gap is reused first.
    let e = unnamed(&mut om, &cc, EVENT_ALL_ACCESS, 0);
    assert_eq!(e.handle, b.handle);
    // Without a parent: empty. Twice: refused.
    om.create_process(ProcessId(4), None).unwrap();
    assert_eq!(om.process_handle_count(ProcessId(4)), Some(0));
    assert_eq!(om.create_process(ProcessId(4), None), Err(STATUS_INVALID_PARAMETER));
    assert_eq!(om.create_process(ProcessId(5), Some(ProcessId(77))), Err(STATUS_INVALID_PARAMETER));
}

#[test]
fn kernel_handles() {
    let (mut om, c) = setup();
    let k = Caller::kernel(P1, T1);
    let ev = om
        .nt_create_event(
            &k,
            &ObjectAttributes::unnamed(OBJ_KERNEL_HANDLE),
            EVENT_ALL_ACCESS,
            EventType::Notification,
            false,
        )
        .unwrap();
    assert!(ev.handle.is_kernel());
    assert_eq!(om.kernel_handle_count(), 1);
    assert_eq!(om.process_handle_count(P1), Some(0));
    // Valid from kernel mode in any process context; not from user mode.
    assert_eq!(om.nt_set_event(&Caller::kernel(P2, T2), ev.handle), Ok(false));
    assert_eq!(om.nt_set_event(&c, ev.handle), Err(STATUS_INVALID_HANDLE));
    assert_eq!(om.nt_close(&c, ev.handle), Err(STATUS_INVALID_HANDLE));
    // From user mode the flag is ignored.
    let user = om
        .nt_create_event(
            &c,
            &ObjectAttributes::unnamed(OBJ_KERNEL_HANDLE),
            EVENT_ALL_ACCESS,
            EventType::Notification,
            false,
        )
        .unwrap();
    assert!(!user.handle.is_kernel());
    om.nt_close(&k, ev.handle).unwrap();
    assert_eq!(om.kernel_handle_count(), 0);
    // Duplicating into the kernel table.
    let kd = om.nt_duplicate_object(&k, P1, user.handle, Some(P1), 0, OBJ_KERNEL_HANDLE, DUPLICATE_SAME_ACCESS);
    assert!(kd.unwrap().unwrap().is_kernel());
}

#[test]
fn exclusive_objects() {
    let (mut om, c) = setup();
    let c2 = caller(P2, T2);
    let k = Caller::kernel(P1, T1);
    let ev = event(&mut om, &k, "\\BaseNamedObjects\\Ex", OBJ_EXCLUSIVE | OBJ_PERMANENT).unwrap();
    assert_eq!(open(&mut om, &c2, "\\BaseNamedObjects\\Ex", 0), Err(STATUS_ACCESS_DENIED));
    let h = open(&mut om, &c, "\\BaseNamedObjects\\Ex", OBJ_EXCLUSIVE).unwrap();
    assert_eq!(om.nt_duplicate_object(&c, P1, h, Some(P2), 0, 0, DUPLICATE_SAME_ACCESS), Err(STATUS_ACCESS_DENIED));
    assert_eq!(om.nt_query_object_basic(&c, h).unwrap().attributes, OBJ_EXCLUSIVE | OBJ_PERMANENT);
    // Exclusive access to an object not created exclusive.
    event(&mut om, &c, "\\BaseNamedObjects\\Shared", 0).unwrap();
    assert_eq!(open(&mut om, &c, "\\BaseNamedObjects\\Shared", OBJ_EXCLUSIVE), Err(STATUS_INVALID_PARAMETER));
    // Once every handle is closed, another process may take it.
    om.nt_close(&c, h).unwrap();
    om.nt_close(&k, ev.handle).unwrap();
    assert!(open(&mut om, &c2, "\\BaseNamedObjects\\Ex", 0).is_ok());
    assert_eq!(open(&mut om, &c, "\\BaseNamedObjects\\Ex", 0), Err(STATUS_ACCESS_DENIED));
}

#[test]
fn query_object_basic_information() {
    let (mut om, c) = setup();
    let ev = event(&mut om, &c, "\\BaseNamedObjects\\Ev", OBJ_INHERIT).unwrap();
    let h2 = open(&mut om, &c, "\\BaseNamedObjects\\Ev", 0).unwrap();
    let info = om.nt_query_object_basic(&c, ev.handle).unwrap();
    assert_eq!(
        info,
        ObjectBasicInformation {
            attributes: OBJ_INHERIT,
            granted_access: EVENT_ALL_ACCESS,
            handle_count: 2,
            pointer_count: 2
        }
    );
    assert_eq!(om.nt_query_object_basic(&c, h2).unwrap().attributes, 0);
    let r = om.reference_by_handle(&c, h2, SYNCHRONIZE, Some(ObjectType::Event)).unwrap();
    assert_eq!(om.nt_query_object_basic(&c, h2).unwrap().pointer_count, 3);
    om.dereference(r).unwrap();
    assert_eq!(om.nt_query_object_type(&c, h2), Ok("Event"));
    assert_eq!(om.nt_query_object_name(&c, h2), Ok(w("\\BaseNamedObjects\\Ev")));
    let anon = unnamed(&mut om, &c, EVENT_ALL_ACCESS, 0);
    assert_eq!(om.nt_query_object_name(&c, anon.handle), Ok(Vec::new()));
    assert_eq!(om.nt_query_object_basic(&c, Handle(0x1000)), Err(STATUS_INVALID_HANDLE));
}

#[test]
fn table_limit() {
    // The limit is large; check the arithmetic only.
    assert_eq!(crate::handle::MAX_HANDLES, 1 << 24);
    let (mut om, c) = setup();
    let ev = unnamed(&mut om, &c, EVENT_ALL_ACCESS, 0);
    let top = Handle(crate::handle::MAX_HANDLES as u64 * 4);
    assert_eq!(om.nt_close(&c, top), Err(STATUS_INVALID_HANDLE));
    assert_eq!(om.nt_close(&c, Handle(top.0 + 4)), Err(STATUS_INVALID_HANDLE));
    om.nt_close(&c, ev.handle).unwrap();
}

#[test]
fn open_by_pointer_gives_a_handle_to_a_kernel_object() {
    let (mut om, c) = setup();
    let file = om
        .new_object(NewObject::Opaque { kind: ObjectType::File, cookie: 7, signaled: None, parse: false }, T1)
        .unwrap();
    let h = om.open_by_pointer(&c, file, GENERIC_READ, 0).unwrap();
    assert_eq!(om.object_by_handle(&c, h, 0, Some(ObjectType::File)), Ok(file));
    assert_eq!(om.granted_access(&c, h), Ok(FILE_GENERIC_READ));
    assert_eq!(om.handle_count(file), Some(1));
    // The kernel's reference and the handle's.
    assert_eq!(om.pointer_count(file), Some(2));
    om.dereference(file).unwrap();
    om.close(&c, h).unwrap();
    assert!(!om.is_alive(file));
    assert_eq!(om.pop_deleted().map(|d| d.cookie), Some(7));
    assert_eq!(om.open_by_pointer(&c, file, 0, 0), Err(STATUS_INVALID_PARAMETER));
}

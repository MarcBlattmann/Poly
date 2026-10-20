use super::*;
use crate::wait::MUTANT_LIMIT;

const N: EventType = EventType::Notification;
const S: EventType = EventType::Synchronization;

/// An object with no handle; the creation reference keeps it alive.
fn new(om: &mut ObjectManager, what: NewObject<'_>) -> ObjectId {
    om.new_object(what, T1).unwrap()
}

fn mutant(om: &mut ObjectManager, owner: Option<ThreadRef>) -> ObjectId {
    om.new_object(NewObject::Mutant { initial_owner: owner.is_some() }, owner.unwrap_or(T1)).unwrap()
}

fn semaphore(om: &mut ObjectManager, initial: i32, maximum: i32) -> ObjectId {
    new(om, NewObject::Semaphore { initial, maximum })
}

fn ev(om: &mut ObjectManager, kind: EventType, signaled: bool) -> ObjectId {
    new(om, NewObject::Event { kind, signaled })
}

fn any(om: &mut ObjectManager, t: ThreadRef, objects: &[ObjectId]) -> Result<WaitOutcome> {
    om.wait_objects(t, objects, WaitType::Any, false, Timeout::Infinite)
}

fn all(om: &mut ObjectManager, t: ThreadRef, objects: &[ObjectId]) -> Result<WaitOutcome> {
    om.wait_objects(t, objects, WaitType::All, false, Timeout::Infinite)
}

#[test]
fn notification_event_releases_every_waiter_and_stays_signaled() {
    let (mut om, _) = setup();
    let e = ev(&mut om, N, false);
    for t in [T1, T2, T3] {
        blocked(wait1(&mut om, t, e));
    }
    assert_eq!(om.pending_waits(), 3);
    assert_eq!(om.set_event(e), Ok(false));
    assert_eq!(woken(&mut om), [(T1, STATUS_WAIT_0), (T2, STATUS_WAIT_0), (T3, STATUS_WAIT_0)]);
    assert_eq!(om.event_info(e), Ok(EventInfo { kind: N, signaled: true }));
    assert_eq!(wait1(&mut om, T1, e), done(STATUS_WAIT_0));
    assert_eq!(om.set_event(e), Ok(true));
    assert_eq!(om.reset_event(e), Ok(true));
    assert_eq!(om.reset_event(e), Ok(false));
    assert_eq!(om.wait_objects(T1, &[e], WaitType::Any, false, Timeout::Zero), done(STATUS_TIMEOUT));
}

#[test]
fn synchronization_event_releases_one_waiter_at_a_time() {
    let (mut om, _) = setup();
    let e = ev(&mut om, S, false);
    for t in [T1, T2, T3] {
        blocked(wait1(&mut om, t, e));
    }
    om.set_event(e).unwrap();
    assert_eq!(woken(&mut om), [(T1, STATUS_WAIT_0)]);
    assert!(!om.event_info(e).unwrap().signaled);
    om.set_event(e).unwrap();
    assert_eq!(woken(&mut om), [(T2, STATUS_WAIT_0)]);
    om.set_event(e).unwrap();
    assert_eq!(woken(&mut om), [(T3, STATUS_WAIT_0)]);
    // Nobody waiting: it stays signaled until a wait takes it.
    om.set_event(e).unwrap();
    assert!(om.event_info(e).unwrap().signaled);
    assert_eq!(wait1(&mut om, T1, e), done(STATUS_WAIT_0));
    assert!(!om.event_info(e).unwrap().signaled);
    assert_eq!(om.wait_objects(T1, &[e], WaitType::Any, false, Timeout::Zero), done(STATUS_TIMEOUT));
}

#[test]
fn pulse_event() {
    let (mut om, _) = setup();
    let n = ev(&mut om, N, false);
    let s = ev(&mut om, S, false);
    for t in [T1, T2] {
        blocked(wait1(&mut om, t, n));
    }
    assert_eq!(om.pulse_event(n), Ok(false));
    assert_eq!(woken(&mut om), [(T1, STATUS_WAIT_0), (T2, STATUS_WAIT_0)]);
    assert!(!om.event_info(n).unwrap().signaled);
    for t in [T1, T2] {
        blocked(wait1(&mut om, t, s));
    }
    om.pulse_event(s).unwrap();
    assert_eq!(woken(&mut om), [(T1, STATUS_WAIT_0)]);
    assert!(!om.event_info(s).unwrap().signaled);
    // With no waiters it only resets.
    let lone = ev(&mut om, N, true);
    assert_eq!(om.pulse_event(lone), Ok(true));
    assert!(!om.event_info(lone).unwrap().signaled);
    assert_eq!(om.pulse_event(lone), Ok(false));
    assert!(woken(&mut om).is_empty());
    // A waiter that cannot be satisfied (wait-all) is not released.
    let other = ev(&mut om, N, false);
    let w = blocked(all(&mut om, T3, &[lone, other]));
    om.pulse_event(lone).unwrap();
    assert!(om.wait_pending(w));
    assert!(woken(&mut om).is_empty());
}

#[test]
fn mutant_ownership_and_recursion() {
    let (mut om, _) = setup();
    let m = mutant(&mut om, Some(T1));
    assert_eq!(om.mutant_info(m), Ok(MutantInfo { owner: Some(T1), recursion: 1, abandoned: false }));
    // The owner waits again without blocking.
    assert_eq!(wait1(&mut om, T1, m), done(STATUS_WAIT_0));
    assert_eq!(om.mutant_info(m).unwrap().recursion, 2);
    // Others block.
    blocked(wait1(&mut om, T2, m));
    assert_eq!(om.wait_objects(T3, &[m], WaitType::Any, false, Timeout::Zero), done(STATUS_TIMEOUT));
    blocked(wait1(&mut om, T3, m));
    assert_eq!(om.release_mutant(m, T2), Err(STATUS_MUTANT_NOT_OWNED));
    assert_eq!(om.release_mutant(m, T1), Ok(2));
    assert!(woken(&mut om).is_empty());
    assert_eq!(om.release_mutant(m, T1), Ok(1));
    assert_eq!(woken(&mut om), [(T2, STATUS_WAIT_0)]);
    assert_eq!(om.mutant_info(m), Ok(MutantInfo { owner: Some(T2), recursion: 1, abandoned: false }));
    assert_eq!(om.release_mutant(m, T1), Err(STATUS_MUTANT_NOT_OWNED));
    om.release_mutant(m, T2).unwrap();
    assert_eq!(woken(&mut om), [(T3, STATUS_WAIT_0)]);
    om.release_mutant(m, T3).unwrap();
    assert_eq!(om.mutant_info(m), Ok(MutantInfo { owner: None, recursion: 0, abandoned: false }));
    assert_eq!(om.release_mutant(m, T3), Err(STATUS_MUTANT_NOT_OWNED));
    // Free: the first wait takes it.
    assert_eq!(wait1(&mut om, T2, m), done(STATUS_WAIT_0));
    assert_eq!(om.mutant_info(m).unwrap().owner, Some(T2));
}

#[test]
fn mutant_abandonment() {
    let (mut om, _) = setup();
    let e = ev(&mut om, N, false);
    let m = mutant(&mut om, Some(T1));
    blocked(any(&mut om, T2, &[e, m]));
    om.thread_terminated(T1);
    assert_eq!(woken(&mut om), [(T2, NtStatus::abandoned(1))]);
    // The new owner holds it normally.
    assert_eq!(om.mutant_info(m), Ok(MutantInfo { owner: Some(T2), recursion: 1, abandoned: false }));

    // Abandoned with nobody waiting: the next wait gets the abandoned status
    // once.
    om.thread_terminated(T2);
    assert_eq!(om.mutant_info(m), Ok(MutantInfo { owner: None, recursion: 0, abandoned: true }));
    assert_eq!(om.release_mutant(m, T2), Err(STATUS_MUTANT_NOT_OWNED));
    assert_eq!(any(&mut om, T3, &[e, m]), done(NtStatus::abandoned(1)));
    om.release_mutant(m, T3).unwrap();
    assert_eq!(any(&mut om, T3, &[m]), done(STATUS_WAIT_0));

    // A recursively owned mutant is freed completely.
    let m2 = mutant(&mut om, Some(T1));
    assert_eq!(wait1(&mut om, T1, m2), done(STATUS_WAIT_0));
    om.thread_terminated(T1);
    assert_eq!(om.mutant_info(m2), Ok(MutantInfo { owner: None, recursion: 0, abandoned: true }));
}

#[test]
fn ending_thread_frees_its_mutants_in_id_order_and_drops_its_wait() {
    let (mut om, _) = setup();
    let m1 = mutant(&mut om, Some(T1));
    let m2 = mutant(&mut om, Some(T1));
    let e = ev(&mut om, N, false);
    blocked(any(&mut om, T2, &[m2, m1]));
    blocked(any(&mut om, T3, &[m2]));
    // T1 itself is blocked on something when it is killed.
    let w1 = blocked(wait1(&mut om, T1, e));
    om.thread_terminated(T1);
    assert!(!om.wait_pending(w1));
    // m1 is freed first and goes to T2 (index 1); then m2 goes to T3.
    assert_eq!(woken(&mut om), [(T2, NtStatus::abandoned(1)), (T3, NtStatus::abandoned(0))]);
    assert!(!om.event_info(e).unwrap().signaled);
    assert_eq!(om.pending_waits(), 0);
}

#[test]
fn mutant_recursion_limit() {
    let (mut om, _) = setup();
    let m = mutant(&mut om, Some(T1));
    om.set_mutant_count(m, MUTANT_LIMIT);
    assert_eq!(wait1(&mut om, T1, m), Err(STATUS_MUTANT_LIMIT_EXCEEDED));
    assert_eq!(om.mutant_info(m).unwrap().recursion, MUTANT_LIMIT);
    om.set_mutant_count(m, MUTANT_LIMIT - 1);
    assert_eq!(wait1(&mut om, T1, m), done(STATUS_WAIT_0));
    assert_eq!(om.release_mutant(m, T1), Ok(MUTANT_LIMIT));
}

#[test]
fn semaphore_counts() {
    let (mut om, c) = setup();
    for (initial, maximum) in [(0, 0), (-1, 5), (6, 5), (0, -3)] {
        assert_eq!(
            om.nt_create_semaphore(&c, &ObjectAttributes::unnamed(0), SEMAPHORE_ALL_ACCESS, initial, maximum),
            Err(STATUS_INVALID_PARAMETER)
        );
    }
    let s = semaphore(&mut om, 2, 3);
    assert_eq!(wait1(&mut om, T1, s), done(STATUS_WAIT_0));
    assert_eq!(wait1(&mut om, T1, s), done(STATUS_WAIT_0));
    assert_eq!(om.semaphore_info(s), Ok(SemaphoreInfo { count: 0, maximum: 3 }));
    for t in [T1, T2, T3] {
        blocked(wait1(&mut om, t, s));
    }
    assert_eq!(om.release_semaphore(s, 2), Ok(0));
    assert_eq!(woken(&mut om), [(T1, STATUS_WAIT_0), (T2, STATUS_WAIT_0)]);
    assert_eq!(om.semaphore_info(s).unwrap().count, 0);
    assert_eq!(om.release_semaphore(s, 3), Ok(0));
    assert_eq!(woken(&mut om), [(T3, STATUS_WAIT_0)]);
    assert_eq!(om.semaphore_info(s).unwrap().count, 2);
    // Past the maximum: nothing changes.
    assert_eq!(om.release_semaphore(s, 2), Err(STATUS_SEMAPHORE_LIMIT_EXCEEDED));
    assert_eq!(om.release_semaphore(s, i32::MAX), Err(STATUS_SEMAPHORE_LIMIT_EXCEEDED));
    assert_eq!(om.semaphore_info(s).unwrap().count, 2);
    assert_eq!(om.release_semaphore(s, 0), Err(STATUS_INVALID_PARAMETER));
    assert_eq!(om.release_semaphore(s, -1), Err(STATUS_INVALID_PARAMETER));
    assert_eq!(om.release_semaphore(s, 1), Ok(2));
}

#[test]
fn wait_any_reports_the_smallest_signaled_index() {
    let (mut om, _) = setup();
    let a = ev(&mut om, S, false);
    let b = ev(&mut om, S, true);
    let c = ev(&mut om, S, true);
    assert_eq!(any(&mut om, T1, &[a, c, b]), done(NtStatus::wait(1)));
    // Only the satisfying object is taken.
    assert!(!om.event_info(c).unwrap().signaled);
    assert!(om.event_info(b).unwrap().signaled);
    assert_eq!(any(&mut om, T1, &[a, c, b]), done(NtStatus::wait(2)));
    // Blocked, then woken with the index of the object that came.
    blocked(any(&mut om, T2, &[a, b, c]));
    om.set_event(c).unwrap();
    assert_eq!(woken(&mut om), [(T2, NtStatus::wait(2))]);
    // The same object twice in a wait-any is allowed; the first index wins.
    om.set_event(a).unwrap();
    assert_eq!(any(&mut om, T1, &[b, a, a]), done(NtStatus::wait(1)));
    blocked(any(&mut om, T1, &[b, a, a]));
    om.set_event(a).unwrap();
    assert_eq!(woken(&mut om), [(T1, NtStatus::wait(1))]);
}

#[test]
fn wait_all_takes_everything_at_once_or_nothing() {
    let (mut om, _) = setup();
    let a = ev(&mut om, S, false);
    let b = ev(&mut om, S, false);
    let s = semaphore(&mut om, 1, 1);
    let w = blocked(all(&mut om, T1, &[a, b, s]));
    om.set_event(a).unwrap();
    // Not all signaled: nothing is taken.
    assert!(om.wait_pending(w));
    assert!(om.event_info(a).unwrap().signaled);
    assert_eq!(om.semaphore_info(s).unwrap().count, 1);
    // A wait-any behind it can take what the wait-all does not hold.
    assert_eq!(any(&mut om, T2, &[a]), done(STATUS_WAIT_0));
    om.set_event(b).unwrap();
    assert!(om.wait_pending(w));
    om.set_event(a).unwrap();
    assert_eq!(woken(&mut om), [(T1, STATUS_WAIT_0)]);
    assert!(!om.event_info(a).unwrap().signaled);
    assert!(!om.event_info(b).unwrap().signaled);
    assert_eq!(om.semaphore_info(s).unwrap().count, 0);
    // Immediate when everything is signaled.
    om.set_event(a).unwrap();
    om.set_event(b).unwrap();
    om.release_semaphore(s, 1).unwrap();
    assert_eq!(all(&mut om, T1, &[a, b, s]), done(STATUS_WAIT_0));
}

#[test]
fn wait_all_with_mutants() {
    let (mut om, _) = setup();
    let mine = mutant(&mut om, Some(T1));
    let free = mutant(&mut om, None);
    let e = ev(&mut om, N, false);
    // A mutant the thread owns counts as signaled for it.
    let w = blocked(all(&mut om, T1, &[e, mine, free]));
    om.set_event(e).unwrap();
    assert_eq!(wakes(&mut om).first().map(|x| (x.wait, x.status)), Some((w, STATUS_WAIT_0)));
    assert_eq!(om.mutant_info(mine).unwrap().recursion, 2);
    assert_eq!(om.mutant_info(free).unwrap().owner, Some(T1));
    // An abandoned mutant in a satisfied wait-all: the first abandoned index.
    let a1 = mutant(&mut om, Some(T3));
    let a2 = mutant(&mut om, Some(T3));
    om.thread_terminated(T3);
    assert_eq!(all(&mut om, T2, &[e, a2, a1]), done(NtStatus::abandoned(1)));
    assert!(!om.mutant_info(a1).unwrap().abandoned && !om.mutant_info(a2).unwrap().abandoned);
}

#[test]
fn wait_parameter_errors() {
    let (mut om, _) = setup();
    let e = ev(&mut om, N, true);
    assert_eq!(any(&mut om, T1, &[]), Err(STATUS_INVALID_PARAMETER));
    let many: Vec<ObjectId> = (0..MAXIMUM_WAIT_OBJECTS + 1).map(|_| ev(&mut om, N, true)).collect();
    assert_eq!(any(&mut om, T1, &many), Err(STATUS_INVALID_PARAMETER));
    assert_eq!(all(&mut om, T1, &many[..MAXIMUM_WAIT_OBJECTS]), done(STATUS_WAIT_0));
    assert_eq!(any(&mut om, T1, &many[..MAXIMUM_WAIT_OBJECTS]), done(STATUS_WAIT_0));
    assert_eq!(all(&mut om, T1, &[e, e]), Err(STATUS_INVALID_PARAMETER_MIX));
    let dir = om.root();
    assert_eq!(any(&mut om, T1, &[e, dir]), Err(STATUS_OBJECT_TYPE_MISMATCH));
    let file = new(&mut om, NewObject::Opaque { kind: ObjectType::File, cookie: 0, signaled: None, parse: false });
    assert_eq!(any(&mut om, T1, &[file]), Err(STATUS_OBJECT_TYPE_MISMATCH));
    assert_eq!(om.pending_waits(), 0);
}

#[test]
fn cancelling_waits() {
    let (mut om, _) = setup();
    let e = ev(&mut om, S, false);
    let w1 = blocked(wait1(&mut om, T1, e));
    let w2 = blocked(om.wait_objects(T2, &[e], WaitType::Any, true, Timeout::Finite));
    let w3 = blocked(om.wait_objects(T3, &[e], WaitType::Any, true, Timeout::Infinite));
    assert_eq!(om.cancel_wait(w1, WaitCancel::Alerted), Err(STATUS_INVALID_PARAMETER));
    assert_eq!(om.cancel_wait(w1, WaitCancel::UserApc), Err(STATUS_INVALID_PARAMETER));
    assert_eq!(om.cancel_wait(w1, WaitCancel::Timeout), Ok(STATUS_TIMEOUT));
    assert_eq!(om.cancel_wait(w1, WaitCancel::Timeout), Err(STATUS_INVALID_PARAMETER));
    assert_eq!(om.cancel_wait(w2, WaitCancel::Alerted), Ok(STATUS_ALERTED));
    // The cancelled waits are gone from the queue: T3 gets the event.
    om.set_event(e).unwrap();
    let wk = wakes(&mut om);
    assert_eq!(wk.len(), 1);
    assert_eq!((wk[0].wait, wk[0].thread, wk[0].status), (w3, T3, STATUS_WAIT_0));
    // Too late to cancel a satisfied wait.
    assert_eq!(om.cancel_wait(w3, WaitCancel::Timeout), Err(STATUS_INVALID_PARAMETER));
    let w4 = blocked(om.wait_objects(T3, &[e], WaitType::Any, true, Timeout::Infinite));
    assert_eq!(om.cancel_wait(w4, WaitCancel::UserApc), Ok(STATUS_USER_APC));
    let w5 = blocked(wait1(&mut om, T3, e));
    assert_eq!(om.cancel_wait(w5, WaitCancel::Terminated), Ok(STATUS_SUCCESS));
    assert_eq!(om.pending_waits(), 0);
}

#[test]
fn zero_timeout_never_blocks_or_changes_state() {
    let (mut om, _) = setup();
    let m = mutant(&mut om, Some(T1));
    let s = semaphore(&mut om, 1, 1);
    let z = |om: &mut ObjectManager, t, objects: &[ObjectId], wt| om.wait_objects(t, objects, wt, false, Timeout::Zero);
    assert_eq!(z(&mut om, T2, &[m, s], WaitType::All), done(STATUS_TIMEOUT));
    assert_eq!(om.semaphore_info(s).unwrap().count, 1);
    assert_eq!(z(&mut om, T2, &[m, s], WaitType::Any), done(NtStatus::wait(1)));
    assert_eq!(z(&mut om, T2, &[m, s], WaitType::Any), done(STATUS_TIMEOUT));
    assert_eq!(om.pending_waits(), 0);
}

#[test]
fn timers() {
    let (mut om, c) = setup();
    let n = om.nt_create_timer(&c, &ObjectAttributes::unnamed(0), TIMER_ALL_ACCESS, N).unwrap();
    let s = om.nt_create_timer(&c, &ObjectAttributes::unnamed(0), TIMER_ALL_ACCESS, S).unwrap();
    assert_eq!(om.timer_info(n.object), Ok((EventInfo { kind: N, signaled: false }, false)));
    assert_eq!(om.nt_set_timer(&c, n.handle), Ok((n.object, false)));
    for t in [T1, T2] {
        blocked(wait1(&mut om, t, n.object));
        blocked(wait1(&mut om, t, s.object));
    }
    om.timer_expired(n.object, false).unwrap();
    assert_eq!(woken(&mut om), [(T1, STATUS_WAIT_0), (T2, STATUS_WAIT_0)]);
    assert_eq!(om.timer_info(n.object), Ok((EventInfo { kind: N, signaled: true }, false)));
    // Setting again resets the state; cancelling keeps it.
    assert_eq!(om.nt_set_timer(&c, n.handle), Ok((n.object, true)));
    assert!(!om.timer_info(n.object).unwrap().0.signaled);
    om.timer_expired(n.object, true).unwrap();
    assert_eq!(om.nt_cancel_timer(&c, n.handle), Ok((n.object, true)));
    assert_eq!(om.timer_info(n.object), Ok((EventInfo { kind: N, signaled: true }, false)));
    // Synchronization timers release one waiter per expiry.
    om.nt_set_timer(&c, s.handle).unwrap();
    om.timer_expired(s.object, true).unwrap();
    assert_eq!(woken(&mut om), [(T1, STATUS_WAIT_0)]);
    assert_eq!(om.timer_info(s.object), Ok((EventInfo { kind: S, signaled: false }, true)));
    om.timer_expired(s.object, true).unwrap();
    assert_eq!(woken(&mut om), [(T2, STATUS_WAIT_0)]);
    // Access.
    let q = om.nt_duplicate_object(&c, P1, n.handle, Some(P1), TIMER_QUERY_STATE, 0, 0).unwrap().unwrap();
    assert_eq!(om.nt_set_timer(&c, q), Err(STATUS_ACCESS_DENIED));
    assert_eq!(om.timer_expired(om.root(), false), Err(STATUS_OBJECT_TYPE_MISMATCH));
}

#[test]
fn opaque_waitable_objects() {
    let (mut om, _) = setup();
    let p =
        new(&mut om, NewObject::Opaque { kind: ObjectType::Process, cookie: 0, signaled: Some(false), parse: false });
    blocked(wait1(&mut om, T1, p));
    blocked(wait1(&mut om, T2, p));
    om.set_signaled(p, true).unwrap();
    assert_eq!(woken(&mut om), [(T1, STATUS_WAIT_0), (T2, STATUS_WAIT_0)]);
    assert_eq!(wait1(&mut om, T3, p), done(STATUS_WAIT_0));
    om.set_signaled(p, false).unwrap();
    assert_eq!(om.wait_objects(T3, &[p], WaitType::Any, false, Timeout::Zero), done(STATUS_TIMEOUT));
    let f = new(&mut om, NewObject::Opaque { kind: ObjectType::File, cookie: 0, signaled: None, parse: false });
    assert_eq!(om.set_signaled(f, true), Err(STATUS_OBJECT_TYPE_MISMATCH));
}

#[test]
fn handle_level_waits_and_state_services() {
    let (mut om, c) = setup();
    let e = om.nt_create_event(&c, &ObjectAttributes::unnamed(0), EVENT_ALL_ACCESS, S, false).unwrap();
    let m = om.nt_create_mutant(&c, &ObjectAttributes::unnamed(0), MUTANT_ALL_ACCESS, true).unwrap();
    let s = om.nt_create_semaphore(&c, &ObjectAttributes::unnamed(0), SEMAPHORE_ALL_ACCESS, 0, 2).unwrap();
    assert_eq!(om.nt_query_mutant(&c, m.handle), Ok(MutantInfo { owner: Some(T1), recursion: 1, abandoned: false }));
    // The owner's wait-any on [event, mutant] is satisfied by the mutant.
    assert_eq!(
        om.nt_wait_for_multiple_objects(&c, &[e.handle, m.handle], WaitType::Any, false, Timeout::Infinite),
        done(NtStatus::wait(1))
    );
    assert_eq!(om.nt_release_mutant(&c, m.handle), Ok(2));
    assert_eq!(om.nt_release_mutant(&caller(P1, T2), m.handle), Err(STATUS_MUTANT_NOT_OWNED));
    // Another thread blocks on all three.
    let c2 = caller(P1, T2);
    let all3 = [e.handle, m.handle, s.handle];
    let pending = blocked(om.nt_wait_for_multiple_objects(&c2, &all3, WaitType::All, false, Timeout::Infinite));
    assert_eq!(om.nt_set_event(&c, e.handle), Ok(false));
    assert_eq!(om.nt_release_semaphore(&c, s.handle, 1), Ok(0));
    assert!(om.wait_pending(pending));
    assert_eq!(om.nt_release_mutant(&c, m.handle), Ok(1));
    assert_eq!(woken(&mut om), [(T2, STATUS_WAIT_0)]);
    assert_eq!(om.nt_query_semaphore(&c, s.handle), Ok(SemaphoreInfo { count: 0, maximum: 2 }));
    assert_eq!(om.nt_query_event(&c, e.handle), Ok(EventInfo { kind: S, signaled: false }));
    // Errors: count, handle, access, same object twice through two handles.
    assert_eq!(
        om.nt_wait_for_multiple_objects(&c, &[], WaitType::Any, false, Timeout::Zero),
        Err(STATUS_INVALID_PARAMETER)
    );
    let too_many = [e.handle; MAXIMUM_WAIT_OBJECTS + 1];
    assert_eq!(
        om.nt_wait_for_multiple_objects(&c, &too_many, WaitType::Any, false, Timeout::Zero),
        Err(STATUS_INVALID_PARAMETER)
    );
    assert_eq!(om.nt_wait_for_single_object(&c, Handle(0x800), false, Timeout::Zero), Err(STATUS_INVALID_HANDLE));
    let e2 = om.nt_duplicate_object(&c, P1, e.handle, Some(P1), SYNCHRONIZE, 0, 0).unwrap().unwrap();
    assert_eq!(
        om.nt_wait_for_multiple_objects(&c, &[e.handle, e2], WaitType::All, false, Timeout::Zero),
        Err(STATUS_INVALID_PARAMETER_MIX)
    );
    let dir = w("\\BaseNamedObjects");
    let d = om.nt_open_directory_object(&c, &ObjectAttributes::named(&dir, 0), DIRECTORY_ALL_ACCESS | SYNCHRONIZE);
    assert_eq!(d, Err(STATUS_ACCESS_DENIED));
    let d = om.nt_open_directory_object(&c, &ObjectAttributes::named(&dir, 0), DIRECTORY_QUERY).unwrap();
    assert_eq!(
        om.nt_wait_for_single_object(&Caller::kernel(P1, T1), d, false, Timeout::Zero),
        Err(STATUS_OBJECT_TYPE_MISMATCH)
    );
}

#[test]
fn queues_are_fifo_per_object() {
    let (mut om, _) = setup();
    let a = ev(&mut om, S, false);
    let b = ev(&mut om, S, false);
    // T1 waits for both, T2 for a only, T3 for a or b.
    let w1 = blocked(all(&mut om, T1, &[a, b]));
    blocked(any(&mut om, T2, &[a]));
    blocked(any(&mut om, T3, &[b, a]));
    // a alone: T1 cannot use it, T2 is next.
    om.set_event(a).unwrap();
    assert_eq!(woken(&mut om), [(T2, STATUS_WAIT_0)]);
    // b alone: T1 still cannot (a was taken), T3 takes b.
    om.set_event(b).unwrap();
    assert_eq!(woken(&mut om), [(T3, STATUS_WAIT_0)]);
    om.set_event(a).unwrap();
    om.set_event(b).unwrap();
    assert_eq!(wakes(&mut om).iter().map(|x| x.wait).collect::<Vec<_>>(), [w1]);
}

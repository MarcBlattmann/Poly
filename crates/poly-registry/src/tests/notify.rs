use super::*;
use crate::Fired;
use poly_ntobj::status::*;

fn fired(r: &mut Registry) -> Vec<Fired> {
    core::iter::from_fn(|| r.pop_fired()).collect()
}

#[test]
fn filters_select_changes() {
    let (mut r, root) = with_hive();
    let k = create(&mut r, root, "k");
    let names = r.open(k).unwrap();
    let values = r.open(k).unwrap();
    r.notify(names, REG_NOTIFY_CHANGE_NAME, false, 1).unwrap();
    r.notify(values, REG_NOTIFY_CHANGE_LAST_SET, false, 2).unwrap();
    r.set_value(k, &wide("v"), REG_DWORD, &[1, 0, 0, 0], T0).unwrap();
    assert_eq!(fired(&mut r), [Fired { open: values, request: 2, status: STATUS_SUCCESS }]);
    create(&mut r, k, "child");
    assert_eq!(fired(&mut r), [Fired { open: names, request: 1, status: STATUS_SUCCESS }]);
    // One-time: nothing queued, nothing fires.
    create(&mut r, k, "child2");
    r.set_value(k, &wide("v"), REG_DWORD, &[2, 0, 0, 0], T0).unwrap();
    assert!(fired(&mut r).is_empty());
}

#[test]
fn same_value_does_not_notify_and_delete_value_does() {
    let (mut r, root) = with_hive();
    let k = create(&mut r, root, "k");
    r.set_value(k, &wide("v"), REG_DWORD, &[1, 0, 0, 0], T0).unwrap();
    let o = r.open(k).unwrap();
    r.notify(o, REG_NOTIFY_CHANGE_LAST_SET, false, 5).unwrap();
    r.set_value(k, &wide("V"), REG_DWORD, &[1, 0, 0, 0], T0).unwrap();
    assert!(fired(&mut r).is_empty());
    r.delete_value(k, &wide("v"), T0).unwrap();
    assert_eq!(fired(&mut r).len(), 1);
}

#[test]
fn subtree_watches_see_descendants() {
    let (mut r, root) = with_hive();
    let a = create(&mut r, root, "a");
    let b = create(&mut r, a, "b");
    let tree = r.open(a).unwrap();
    let flat = r.open(a).unwrap();
    r.notify(tree, REG_NOTIFY_CHANGE_LAST_SET, true, 1).unwrap();
    r.notify(flat, REG_NOTIFY_CHANGE_LAST_SET, false, 2).unwrap();
    r.set_value(b, &wide("x"), REG_SZ, &sz("y"), T0).unwrap();
    assert_eq!(fired(&mut r), [Fired { open: tree, request: 1, status: STATUS_SUCCESS }]);
    // A change above the watched key does not count.
    r.notify(tree, REG_NOTIFY_CHANGE_LAST_SET, true, 3).unwrap();
    r.set_value(root, &wide("x"), REG_SZ, &sz("y"), T0).unwrap();
    assert!(fired(&mut r).is_empty());
}

#[test]
fn first_call_sets_the_filter() {
    let (mut r, root) = with_hive();
    let k = create(&mut r, root, "k");
    let o = r.open(k).unwrap();
    r.notify(o, REG_NOTIFY_CHANGE_NAME, false, 1).unwrap();
    // A later call with another filter only queues a request.
    r.notify(o, REG_NOTIFY_CHANGE_LAST_SET, false, 2).unwrap();
    assert_eq!(r.pending_requests(o), 2);
    r.set_value(k, &wide("v"), REG_SZ, &sz("x"), T0).unwrap();
    assert!(fired(&mut r).is_empty());
    create(&mut r, k, "c");
    let f = fired(&mut r);
    assert_eq!(f.iter().map(|f| f.request).collect::<Vec<_>>(), [1, 2]);
}

#[test]
fn close_and_delete_complete_requests() {
    let (mut r, root) = with_hive();
    let k = create(&mut r, root, "k");
    let o = r.open(k).unwrap();
    r.notify(o, REG_NOTIFY_CHANGE_LAST_SET, false, 1).unwrap();
    r.close(o);
    assert_eq!(fired(&mut r), [Fired { open: o, request: 1, status: STATUS_NOTIFY_CLEANUP }]);

    let o = r.open(k).unwrap();
    let parent = r.open(root).unwrap();
    r.notify(o, REG_NOTIFY_CHANGE_LAST_SET, false, 2).unwrap();
    r.notify(parent, REG_NOTIFY_CHANGE_NAME, false, 3).unwrap();
    r.delete_key(k, T0).unwrap();
    assert_eq!(
        fired(&mut r),
        [
            Fired { open: o, request: 2, status: STATUS_KEY_DELETED },
            Fired { open: parent, request: 3, status: STATUS_SUCCESS }
        ]
    );
    assert_eq!(r.notify(o, REG_NOTIFY_CHANGE_NAME, false, 4), Err(STATUS_KEY_DELETED));
    r.close(o);
    r.close(parent);
}

#[test]
fn bad_filters_and_cancel() {
    let (mut r, root) = with_hive();
    let o = r.open(root).unwrap();
    assert_eq!(r.notify(o, 0, false, 1), Err(STATUS_INVALID_PARAMETER));
    assert_eq!(r.notify(o, REG_NOTIFY_THREAD_AGNOSTIC, false, 1), Err(STATUS_INVALID_PARAMETER));
    assert_eq!(r.notify(o, 0x20, false, 1), Err(STATUS_INVALID_PARAMETER));
    r.notify(o, REG_NOTIFY_CHANGE_NAME | REG_NOTIFY_THREAD_AGNOSTIC, false, 1).unwrap();
    assert!(r.cancel_request(o, 1));
    assert!(!r.cancel_request(o, 1));
    create(&mut r, root, "x");
    assert!(fired(&mut r).is_empty());
}

#[test]
fn attributes_fire_on_write_time() {
    let (mut r, root) = with_hive();
    let o = r.open(root).unwrap();
    r.notify(o, REG_NOTIFY_CHANGE_ATTRIBUTES, false, 1).unwrap();
    r.set_last_write(root, 42).unwrap();
    assert_eq!(r.key(root).unwrap().last_write(), 42);
    assert_eq!(fired(&mut r).len(), 1);
}

use super::*;
use crate::tree::{Disposition, Resolved};
use poly_ntobj::status::*;

#[test]
fn master_keys_and_paths() {
    let r = Registry::new();
    assert_eq!(path_string(&r, r.root()), "\\REGISTRY");
    assert_eq!(path_string(&r, r.machine()), "\\REGISTRY\\MACHINE");
    assert_eq!(path_string(&r, r.user()), "\\REGISTRY\\USER");
    assert!(r.key(r.machine()).unwrap().is_volatile());
}

#[test]
fn names_ignore_case_and_keep_it() {
    let (mut r, root) = with_hive();
    let k = create(&mut r, root, "Poly Test");
    assert_eq!(r.child(root, &wide("POLY TEST")), Some(k));
    assert_eq!(r.child(root, &wide("poly test")), Some(k));
    assert_eq!(String::from_utf16(r.key(k).unwrap().name()).unwrap(), "Poly Test");
    // Creating again in another case opens the same key.
    let (again, d) = r.create_key(root, &wide("POLY TEST"), CreateOptions::default(), T0).unwrap();
    assert_eq!((again, d), (k, Disposition::Opened));
    assert_eq!(path_string(&r, k), "\\REGISTRY\\MACHINE\\TEST\\Poly Test");
}

#[test]
fn create_only_the_last_component() {
    let (mut r, root) = with_hive();
    assert_eq!(r.resolve(root, &wide("\\a\\b"), false), Err(STATUS_OBJECT_NAME_NOT_FOUND));
    let a = create(&mut r, root, "a");
    assert_eq!(r.resolve(root, &wide("\\a\\b"), false), Ok(Resolved::Missing { parent: a, name: wide("b") }));
    assert_eq!(r.resolve(root, &wide("a"), false), Ok(Resolved::Found(a)));
    assert_eq!(r.resolve(root, &wide(""), false), Ok(Resolved::Found(root)));
    assert_eq!(r.resolve(root, &wide("a\\\\b"), false), Err(STATUS_OBJECT_NAME_INVALID));
    assert_eq!(r.resolve(root, &wide("a\\"), false), Err(STATUS_OBJECT_NAME_INVALID));
}

#[test]
fn name_limits() {
    let (mut r, root) = with_hive();
    let long: String = core::iter::repeat_n('x', 256).collect();
    assert_eq!(
        r.create_key(root, &wide(&long), CreateOptions::default(), T0).map(|_| ()),
        Err(STATUS_OBJECT_NAME_INVALID)
    );
    assert!(r.create_key(root, &wide(&long[..255]), CreateOptions::default(), T0).is_ok());
    let k = create(&mut r, root, "v");
    let long_value: Vec<u16> = core::iter::repeat_n(b'v' as u16, MAX_VALUE_NAME_CHARS + 1).collect();
    assert_eq!(r.set_value(k, &long_value, REG_NONE, &[], T0), Err(STATUS_INVALID_PARAMETER));
    assert_eq!(r.set_value(k, &long_value[1..], REG_NONE, &[], T0), Ok(()));
}

#[test]
fn volatile_children_must_be_volatile() {
    let (mut r, root) = with_hive();
    let m = r.machine();
    // A master key holds only volatile keys and hives.
    assert_eq!(
        r.create_key(m, &wide("X"), CreateOptions::default(), T0).map(|_| ()),
        Err(STATUS_CHILD_MUST_BE_VOLATILE)
    );
    let v = create_volatile(&mut r, root, "vol");
    assert_eq!(
        r.create_key(v, &wide("nv"), CreateOptions::default(), T0).map(|_| ()),
        Err(STATUS_CHILD_MUST_BE_VOLATILE)
    );
    let vv = create_volatile(&mut r, v, "vv");
    assert!(r.key(vv).unwrap().is_volatile());
    // Opening an existing key ignores the options.
    let nv = create(&mut r, root, "nv");
    let (same, d) =
        r.create_key(root, &wide("nv"), CreateOptions { volatile: true, ..Default::default() }, T0).unwrap();
    assert_eq!((same, d), (nv, Disposition::Opened));
    assert!(!r.key(nv).unwrap().is_volatile());
}

#[test]
fn subkeys_enumerate_sorted_ignoring_case() {
    let (mut r, root) = with_hive();
    for n in ["delta", "Alpha", "charlie", "Bravo"] {
        create(&mut r, root, n);
    }
    assert_eq!(names(&r, root), ["Alpha", "Bravo", "charlie", "delta"]);
    assert_eq!(r.subkey_at(root, 4), Err(STATUS_NO_MORE_ENTRIES));
}

#[test]
fn values_replace_and_enumerate_in_order() {
    let (mut r, root) = with_hive();
    let k = create(&mut r, root, "k");
    r.set_value(k, &wide("b"), REG_SZ, &sz("one"), T0 + 1).unwrap();
    r.set_value(k, &wide(""), REG_DWORD, &7u32.to_le_bytes(), T0 + 2).unwrap();
    r.set_value(k, &wide("a"), REG_QWORD, &9u64.to_le_bytes(), T0 + 3).unwrap();
    r.set_value(k, &wide("B"), REG_EXPAND_SZ, &sz("%x%"), T0 + 4).unwrap();
    let names: Vec<String> = (0..3).map(|i| String::from_utf16(&r.value_at(k, i).unwrap().name).unwrap()).collect();
    assert_eq!(names, ["b", "", "a"]);
    assert_eq!(r.value_at(k, 3), Err(STATUS_NO_MORE_ENTRIES));
    let b = r.value(k, &wide("b")).unwrap();
    assert_eq!((b.kind, b.data.clone()), (REG_EXPAND_SZ, sz("%x%")));
    assert_eq!(r.key(k).unwrap().last_write(), T0 + 4);
    // Any type number is kept.
    r.set_value(k, &wide("odd"), 0x1234_5678, &[1, 2, 3], T0).unwrap();
    assert_eq!(r.value(k, &wide("ODD")).unwrap().kind, 0x1234_5678);
    r.delete_value(k, &wide("A"), T0 + 5).unwrap();
    assert_eq!(r.value(k, &wide("a")), Err(STATUS_OBJECT_NAME_NOT_FOUND));
    assert_eq!(r.delete_value(k, &wide("a"), T0), Err(STATUS_OBJECT_NAME_NOT_FOUND));
    assert_eq!(r.key(k).unwrap().last_write(), T0 + 5);
}

#[test]
fn same_value_again_changes_nothing() {
    let (mut r, root) = with_hive();
    let k = create(&mut r, root, "k");
    r.set_value(k, &wide("v"), REG_DWORD, &[1, 0, 0, 0], T0 + 1).unwrap();
    r.set_value(k, &wide("v"), REG_DWORD, &[1, 0, 0, 0], T0 + 9).unwrap();
    assert_eq!(r.key(k).unwrap().last_write(), T0 + 1);
}

#[test]
fn delete_rules_and_open_instances() {
    let (mut r, root) = with_hive();
    let a = create(&mut r, root, "a");
    let b = create(&mut r, a, "b");
    assert_eq!(r.delete_key(a, T0), Err(STATUS_CANNOT_DELETE));
    assert_eq!(r.delete_key(root, T0), Err(STATUS_CANNOT_DELETE));
    assert_eq!(r.delete_key(r.machine(), T0), Err(STATUS_CANNOT_DELETE));
    assert_eq!(r.delete_key(r.root(), T0), Err(STATUS_CANNOT_DELETE));
    let open = r.open(b).unwrap();
    let keys = r.key_count();
    r.delete_key(b, T0 + 1).unwrap();
    // Gone from the namespace at once; the open instance sees it deleted.
    assert_eq!(r.child(a, &wide("b")), None);
    assert_eq!(r.key_of(open), Err(STATUS_KEY_DELETED));
    assert_eq!(r.set_value(b, &wide("x"), REG_NONE, &[], T0), Err(STATUS_KEY_DELETED));
    assert_eq!(r.delete_key(b, T0), Err(STATUS_KEY_DELETED));
    assert_eq!(r.key_count(), keys, "kept while open");
    // A new key may take the name.
    let b2 = create(&mut r, a, "b");
    assert_ne!(b2, b);
    r.close(open);
    assert_eq!(r.key_count(), keys, "the deleted one freed, the new one there");
    assert_eq!(r.key(b).err(), Some(STATUS_KEY_DELETED));
    assert_eq!(r.key_of(open), Err(STATUS_INVALID_HANDLE));
    r.delete_key(b2, T0).unwrap();
    r.delete_key(a, T0 + 2).unwrap();
    assert_eq!(r.key(root).unwrap().last_write(), T0 + 2);
}

#[test]
fn links_reparse_and_open_link() {
    let (mut r, root) = with_hive();
    let target = create(&mut r, root, "Target");
    let inner = create(&mut r, target, "Inner");
    let (link, _) = r.create_key(root, &wide("Link"), CreateOptions { link: true, ..Default::default() }, T0).unwrap();
    assert!(r.key(link).unwrap().is_link());
    // No target yet.
    assert_eq!(r.resolve(root, &wide("Link"), false), Err(STATUS_OBJECT_TYPE_MISMATCH));
    assert_eq!(r.resolve(root, &wide("Link"), true), Ok(Resolved::Found(link)));
    r.set_value(link, &wide("SymbolicLinkValue"), REG_LINK, &link_data("\\REGISTRY\\MACHINE\\TEST\\Target"), T0)
        .unwrap();
    assert_eq!(
        r.resolve(root, &wide("Link\\Inner\\x"), false),
        Ok(Resolved::Reparse(wide("\\REGISTRY\\MACHINE\\TEST\\Target\\Inner\\x")))
    );
    assert_eq!(r.resolve(root, &wide("Link"), false), Ok(Resolved::Reparse(wide("\\REGISTRY\\MACHINE\\TEST\\Target"))));
    // The reparsed path leads to the target's keys.
    let rest = wide("\\MACHINE\\TEST\\Target\\Inner");
    assert_eq!(r.resolve(r.root(), &rest, false), Ok(Resolved::Found(inner)));
    // A path below a link opened as itself goes below the target.
    assert_eq!(
        r.resolve(link, &wide("Inner"), false),
        Ok(Resolved::Reparse(wide("\\REGISTRY\\MACHINE\\TEST\\Target\\Inner")))
    );
    // A link cannot be made over an existing key; a trailing NUL is fine.
    assert_eq!(
        r.create_key(root, &wide("target"), CreateOptions { link: true, ..Default::default() }, T0).map(|_| ()),
        Err(STATUS_OBJECT_NAME_COLLISION)
    );
    let mut nul = link_data("\\REGISTRY\\MACHINE\\TEST\\Target");
    nul.extend_from_slice(&[0, 0]);
    r.set_value(link, &wide("SymbolicLinkValue"), REG_LINK, &nul, T0).unwrap();
    assert_eq!(r.link_target(link).unwrap(), wide("\\REGISTRY\\MACHINE\\TEST\\Target"));
    // Not REG_LINK, or relative: unusable.
    r.set_value(link, &wide("SymbolicLinkValue"), REG_SZ, &nul, T0).unwrap();
    assert_eq!(r.link_target(link), Err(STATUS_OBJECT_TYPE_MISMATCH));
    r.set_value(link, &wide("SymbolicLinkValue"), REG_LINK, &link_data("Target"), T0).unwrap();
    assert_eq!(r.link_target(link), Err(STATUS_OBJECT_TYPE_MISMATCH));
}

#[test]
fn depth_limit() {
    let (mut r, root) = with_hive();
    let mut cur = root;
    // \REGISTRY is depth 0, MACHINE 1, TEST 2.
    for _ in 3..=MAX_DEPTH {
        cur = create(&mut r, cur, "d");
    }
    assert_eq!(r.create_key(cur, &wide("d"), CreateOptions::default(), T0).map(|_| ()), Err(STATUS_INVALID_PARAMETER));
}

#[test]
fn info_counts_and_maxima() {
    let (mut r, root) = with_hive();
    let k = create(&mut r, root, "k");
    r.create_key(k, &wide("abc"), CreateOptions { class: &wide("Klass"), ..Default::default() }, T0).unwrap();
    create(&mut r, k, "a");
    r.set_value(k, &wide("long name"), REG_BINARY, &[0; 10], T0).unwrap();
    r.set_value(k, &wide("x"), REG_BINARY, &[0; 30], T0).unwrap();
    let i = r.info(k).unwrap();
    assert_eq!((i.subkeys, i.max_name_len, i.max_class_len), (2, 6, 10));
    assert_eq!((i.values, i.max_value_name_len, i.max_value_data_len), (2, 18, 30));
}

use super::*;
use crate::info::*;
use crate::tree::Value;
use poly_ntobj::status::*;

fn u32_at(b: &[u8], o: usize) -> u32 {
    u32::from_le_bytes(b[o..o + 4].try_into().unwrap())
}

fn u64_at(b: &[u8], o: usize) -> u64 {
    u64::from_le_bytes(b[o..o + 8].try_into().unwrap())
}

fn utf16(b: &[u8]) -> String {
    String::from_utf16(&b.chunks(2).map(|c| u16::from_le_bytes([c[0], c[1]])).collect::<Vec<_>>()).unwrap()
}

fn sample() -> (Registry, KeyId) {
    let (mut r, root) = with_hive();
    let k = r
        .create_key(root, &wide("Sample"), CreateOptions { class: &wide("Cls"), ..Default::default() }, T0 + 5)
        .unwrap()
        .0;
    create(&mut r, k, "Sub");
    r.set_value(k, &wide("Name"), REG_SZ, &sz("Poly"), T0 + 6).unwrap();
    (r, k)
}

#[test]
fn key_basic_node_full_name_cached() {
    let (r, k) = sample();
    let b = key_information(&r, k, KEY_BASIC_INFORMATION, false).unwrap();
    assert_eq!(b.fixed, 16);
    assert_eq!(b.bytes.len(), 16 + 12);
    assert_eq!(u64_at(&b.bytes, 0), T0 + 6);
    assert_eq!(u32_at(&b.bytes, 12), 12);
    assert_eq!(utf16(&b.bytes[16..]), "Sample");

    let n = key_information(&r, k, KEY_NODE_INFORMATION, false).unwrap();
    assert_eq!(n.fixed, 24);
    let (class_offset, class_len, name_len) = (u32_at(&n.bytes, 12), u32_at(&n.bytes, 16), u32_at(&n.bytes, 20));
    assert_eq!((class_offset, class_len, name_len), (24 + 12, 6, 12));
    assert_eq!(utf16(&n.bytes[24..36]), "Sample");
    assert_eq!(utf16(&n.bytes[36..42]), "Cls");
    assert_eq!(n.bytes.len(), 42);

    let f = key_information(&r, k, KEY_FULL_INFORMATION, false).unwrap();
    assert_eq!(f.fixed, 44);
    let fields: Vec<u32> = (8..44).step_by(4).map(|o| u32_at(&f.bytes, o)).collect();
    // TitleIndex, ClassOffset, ClassLength, SubKeys, MaxNameLen, MaxClassLen,
    // Values, MaxValueNameLen, MaxValueDataLen.
    assert_eq!(fields, [0, 44, 6, 1, 6, 0, 1, 8, 10]);
    assert_eq!(utf16(&f.bytes[44..]), "Cls");

    let nm = key_information(&r, k, KEY_NAME_INFORMATION, false).unwrap();
    assert_eq!(nm.fixed, 4);
    assert_eq!(utf16(&nm.bytes[4..]), "\\REGISTRY\\MACHINE\\TEST\\Sample");
    assert_eq!(u32_at(&nm.bytes, 0) as usize, nm.bytes.len() - 4);

    let c = key_information(&r, k, KEY_CACHED_INFORMATION, false).unwrap();
    assert_eq!((c.fixed, c.bytes.len()), (40, 40));
    let fields: Vec<u32> = (8..36).step_by(4).map(|o| u32_at(&c.bytes, o)).collect();
    // TitleIndex, SubKeys, MaxNameLen, Values, MaxValueNameLen, MaxValueDataLen, NameLength.
    assert_eq!(fields, [0, 1, 6, 1, 8, 10, 12]);

    let v = key_information(&r, k, KEY_VIRTUALIZATION_INFORMATION, false).unwrap();
    assert_eq!(v.bytes, [0, 0, 0, 0]);
}

#[test]
fn class_rules() {
    let (r, k) = sample();
    for class in [KEY_NAME_INFORMATION, KEY_CACHED_INFORMATION, KEY_VIRTUALIZATION_INFORMATION, 5, 7, 8, 9, 10, 99] {
        assert_eq!(key_information(&r, k, class, true).map(|_| ()), Err(STATUS_INVALID_PARAMETER), "{class}");
    }
    for class in [5, 7, 8, 9, 10] {
        assert_eq!(key_information(&r, k, class, false).map(|_| ()), Err(STATUS_INVALID_PARAMETER));
    }
    assert_eq!(query_key_access(KEY_NAME_INFORMATION), 0);
    assert_eq!(query_key_access(KEY_BASIC_INFORMATION), KEY_QUERY_VALUE);
}

fn value() -> Value {
    Value { name: wide("abc"), kind: REG_BINARY, data: alloc::vec![1, 2, 3, 4, 5] }
}

#[test]
fn value_classes() {
    let v = value();
    let b = value_information(&v, KEY_VALUE_BASIC_INFORMATION).unwrap();
    assert_eq!((b.fixed, b.bytes.len()), (12, 18));
    assert_eq!([u32_at(&b.bytes, 4), u32_at(&b.bytes, 8)], [REG_BINARY, 6]);
    assert_eq!(utf16(&b.bytes[12..]), "abc");

    let f = value_information(&v, KEY_VALUE_FULL_INFORMATION).unwrap();
    assert_eq!(f.fixed, 20);
    let (kind, offset, len, name_len) =
        (u32_at(&f.bytes, 4), u32_at(&f.bytes, 8), u32_at(&f.bytes, 12), u32_at(&f.bytes, 16));
    assert_eq!((kind, offset, len, name_len), (REG_BINARY, 26, 5, 6));
    assert_eq!(&f.bytes[26..], &[1, 2, 3, 4, 5]);

    let fa = value_information(&v, KEY_VALUE_FULL_INFORMATION_ALIGN64).unwrap();
    assert_eq!(u32_at(&fa.bytes, 8), 32);
    assert_eq!(&fa.bytes[32..], &[1, 2, 3, 4, 5]);

    let p = value_information(&v, KEY_VALUE_PARTIAL_INFORMATION).unwrap();
    assert_eq!((p.fixed, p.bytes.len()), (12, 17));
    assert_eq!([u32_at(&p.bytes, 4), u32_at(&p.bytes, 8)], [REG_BINARY, 5]);
    assert_eq!(&p.bytes[12..], &[1, 2, 3, 4, 5]);

    let pa = value_information(&v, KEY_VALUE_PARTIAL_INFORMATION_ALIGN64).unwrap();
    assert_eq!((pa.fixed, pa.bytes.len()), (8, 13));
    assert_eq!([u32_at(&pa.bytes, 0), u32_at(&pa.bytes, 4)], [REG_BINARY, 5]);

    let l = value_information(&v, KEY_VALUE_LAYER_INFORMATION).unwrap();
    assert_eq!(l.bytes, [0, 0, 0, 0]);

    assert_eq!(value_information(&v, 6).map(|_| ()), Err(STATUS_INVALID_PARAMETER));
    assert!(value_class_needs_align64(KEY_VALUE_PARTIAL_INFORMATION_ALIGN64));
    assert!(!value_class_needs_align64(KEY_VALUE_PARTIAL_INFORMATION));
}

#[test]
fn buffer_size_statuses() {
    let p = value_information(&value(), KEY_VALUE_PARTIAL_INFORMATION).unwrap();
    let all = fit(&p, 17);
    assert_eq!((all.status, all.write.len(), all.result_length), (STATUS_SUCCESS, 17, 17));
    let big = fit(&p, 100);
    assert_eq!((big.status, big.write.len()), (STATUS_SUCCESS, 17));
    // The fixed part fits but not the data: the fixed part only.
    for length in [12, 16] {
        let part = fit(&p, length);
        assert_eq!((part.status, part.write, part.result_length), (STATUS_BUFFER_OVERFLOW, &p.bytes[..12], 17));
    }
    for length in [0, 11] {
        let none = fit(&p, length);
        assert_eq!((none.status, none.write.len(), none.result_length), (STATUS_BUFFER_TOO_SMALL, 0, 17));
    }
}

#[test]
fn deleted_key_information_fails() {
    let (mut r, k) = sample();
    let sub = r.subkey_at(k, 0).unwrap();
    let open = r.open(sub).unwrap();
    r.delete_key(sub, T0).unwrap();
    assert_eq!(key_information(&r, sub, KEY_BASIC_INFORMATION, false), Err(STATUS_KEY_DELETED));
    r.close(open);
}

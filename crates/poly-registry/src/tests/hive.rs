use super::*;
use crate::hive::*;
use crate::tree::Value;

fn sample_file() -> HiveFile {
    HiveFile {
        sequence: 7,
        root: KeyImage {
            name: wide("ROOT"),
            class: Vec::new(),
            last_write: T0,
            link: false,
            values: alloc::vec![Value { name: wide(""), kind: REG_SZ, data: sz("default") }],
            subkeys: alloc::vec![
                KeyImage {
                    name: wide("A"),
                    class: wide("Class"),
                    last_write: T0 + 1,
                    link: false,
                    values: alloc::vec![
                        Value { name: wide("d"), kind: REG_DWORD, data: alloc::vec![1, 0, 0, 0] },
                        Value { name: wide("b"), kind: REG_BINARY, data: alloc::vec![] },
                    ],
                    subkeys: Vec::new(),
                },
                KeyImage {
                    name: wide("L"),
                    link: true,
                    values: alloc::vec![Value {
                        name: wide("SymbolicLinkValue"),
                        kind: REG_LINK,
                        data: link_data("\\REGISTRY\\MACHINE\\TEST\\A")
                    }],
                    ..KeyImage::default()
                },
            ],
        },
    }
}

#[test]
fn crc32_check_value() {
    // The standard check value of CRC-32/ISO-HDLC.
    assert_eq!(crc32(b"123456789"), 0xCBF4_3926);
    assert_eq!(crc32(b""), 0);
}

#[test]
fn encode_decode_round_trip() {
    let f = sample_file();
    let bytes = encode(&f);
    assert_eq!(&bytes[..8], b"POLYHIVE");
    assert_eq!(u16::from_le_bytes([bytes[8], bytes[9]]), 1);
    assert_eq!(decode(&bytes), Ok(f));
}

#[test]
fn corrupt_files_are_refused() {
    let bytes = encode(&sample_file());
    // Truncations anywhere.
    for n in 0..bytes.len() {
        assert_eq!(decode(&bytes[..n]), Err(STATUS_REGISTRY_CORRUPT), "truncated to {n}");
    }
    // Any flipped payload bit breaks the checksum.
    for i in HEADER_SIZE..bytes.len() {
        let mut b = bytes.clone();
        b[i] ^= 0x10;
        assert_eq!(decode(&b), Err(STATUS_REGISTRY_CORRUPT), "byte {i}");
    }
    let mut b = bytes.clone();
    b[0] = b'X';
    assert_eq!(decode(&b), Err(STATUS_REGISTRY_CORRUPT));
    let mut b = bytes.clone();
    b[8] = 2;
    assert_eq!(decode(&b), Err(STATUS_REGISTRY_CORRUPT), "another major version");
    // A later minor version with trailing payload is read.
    let mut f = sample_file();
    f.sequence = 1;
    let mut payload_extra = encode(&f);
    payload_extra.extend_from_slice(&[9, 9, 9, 9]);
    let payload_len = (payload_extra.len() - HEADER_SIZE) as u32;
    payload_extra[10] = 3;
    payload_extra[24..28].copy_from_slice(&payload_len.to_le_bytes());
    let crc = crc32(&payload_extra[HEADER_SIZE..]);
    payload_extra[28..32].copy_from_slice(&crc.to_le_bytes());
    assert_eq!(decode(&payload_extra), Ok(f));
}

fn rewrap(mut f: HiveFile, edit: impl FnOnce(&mut KeyImage)) -> Vec<u8> {
    edit(&mut f.root);
    encode(&f)
}

#[test]
fn invalid_trees_are_refused() {
    let dup = rewrap(sample_file(), |r| r.subkeys[1].name = wide("a"));
    assert_eq!(decode(&dup), Err(STATUS_REGISTRY_CORRUPT), "duplicate subkey names");
    let dupv = rewrap(sample_file(), |r| r.subkeys[0].values[1].name = wide("D"));
    assert_eq!(decode(&dupv), Err(STATUS_REGISTRY_CORRUPT), "duplicate value names");
    let sep = rewrap(sample_file(), |r| r.subkeys[0].name = wide("a\\b"));
    assert_eq!(decode(&sep), Err(STATUS_REGISTRY_CORRUPT));
    let empty = rewrap(sample_file(), |r| r.subkeys[0].name = Vec::new());
    assert_eq!(decode(&empty), Err(STATUS_REGISTRY_CORRUPT));
    let link_children = rewrap(sample_file(), |r| {
        let a = r.subkeys[0].clone();
        r.subkeys[1].subkeys.push(a);
    });
    assert_eq!(decode(&link_children), Err(STATUS_REGISTRY_CORRUPT));
}

#[test]
fn registry_save_and_reload_skips_volatile_keys() {
    let (mut r, root) = with_hive();
    let a = create(&mut r, root, "Persist");
    let v = create_volatile(&mut r, a, "Gone");
    create_volatile(&mut r, v, "AlsoGone");
    r.set_value(a, &wide("Q"), REG_QWORD, &5u64.to_le_bytes(), T0 + 3).unwrap();
    r.set_value(a, &wide("M"), REG_MULTI_SZ, &[b'a', 0, 0, 0, 0, 0], T0 + 3).unwrap();
    let (link, _) = r.create_key(root, &wide("Lnk"), CreateOptions { link: true, ..Default::default() }, T0).unwrap();
    r.set_value(link, &wide("SymbolicLinkValue"), REG_LINK, &link_data("\\REGISTRY\\MACHINE\\TEST\\Persist"), T0)
        .unwrap();
    let hive = r.hive_by_file("TEST").unwrap();
    assert!(r.hive(hive).dirty);
    let bytes = r.save(hive);
    assert!(!r.hive(hive).dirty);
    assert_eq!(r.hive(hive).sequence, 1);
    assert!(r.dirty_hives().is_empty());

    // "Reboot": a new registry from the file.
    let file = decode(&bytes).unwrap();
    assert_eq!(file.sequence, 1);
    let mut r2 = Registry::new();
    let m = r2.machine();
    let h2 = r2.mount_hive(m, &wide("TEST"), "TEST", Some(&file), T0 + 100).unwrap();
    assert!(!r2.hive(h2).dirty);
    assert_eq!(r2.hive(h2).sequence, 1);
    let root2 = r2.hive(h2).root;
    let a2 = r2.child(root2, &wide("persist")).unwrap();
    assert_eq!(r2.value(a2, &wide("q")).unwrap().data, 5u64.to_le_bytes());
    assert_eq!(r2.value(a2, &wide("m")).unwrap().kind, REG_MULTI_SZ);
    assert_eq!(r2.key(a2).unwrap().last_write(), T0 + 3);
    assert_eq!(r2.child(a2, &wide("Gone")), None, "volatile keys are not saved");
    let link2 = r2.child(root2, &wide("lnk")).unwrap();
    assert!(r2.key(link2).unwrap().is_link());
    assert_eq!(r2.link_target(link2).unwrap(), wide("\\REGISTRY\\MACHINE\\TEST\\Persist"));
    // A change makes it dirty again; the next save counts on.
    r2.set_value(a2, &wide("q"), REG_QWORD, &6u64.to_le_bytes(), T0).unwrap();
    assert_eq!(r2.dirty_hives(), [h2]);
    assert_eq!(decode(&r2.save(h2)).unwrap().sequence, 2);
    // Volatile changes leave the hive clean.
    let v2 = create_volatile(&mut r2, root2, "v");
    r2.set_value(v2, &wide("x"), REG_SZ, &sz("y"), T0).unwrap();
    r2.delete_key(v2, T0).unwrap();
    assert!(r2.dirty_hives().is_empty());
}

#[test]
fn a_hive_mount_point_is_unique() {
    let (mut r, _) = with_hive();
    let m = r.machine();
    assert_eq!(
        r.mount_hive(m, &wide("test"), "OTHER", None, T0).map(|_| ()),
        Err(poly_ntobj::status::STATUS_OBJECT_NAME_COLLISION)
    );
}

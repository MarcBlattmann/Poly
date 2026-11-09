//! Tests: attributes, times, information classes and volume information.

use super::*;

#[test]
fn readonly_hidden_and_times() {
    let mut t = setup();
    let p = "\\??\\C:\\ro.txt";
    let (s, h, _) = t.create(p, RW, 0, FILE_CREATE, SYNC, FILE_ATTRIBUTE_READONLY | FILE_ATTRIBUTE_HIDDEN);
    assert_eq!(s, STATUS_SUCCESS);
    // The creator may still write.
    assert_eq!(t.write(h, b"x", None).0, STATUS_SUCCESS);
    let (s, b) = t.query(h, FILE_BASIC_INFORMATION, 40);
    assert_eq!(s, STATUS_SUCCESS);
    let attrs = u32::from_le_bytes(b[32..36].try_into().unwrap());
    assert_eq!(attrs, FILE_ATTRIBUTE_READONLY | FILE_ATTRIBUTE_HIDDEN | FILE_ATTRIBUTE_ARCHIVE);
    // 2001-09-09 01:46:40 UTC, from the fixed clock, in 100 ns since 1601.
    let write_time = i64::from_le_bytes(b[16..24].try_into().unwrap());
    assert_eq!(write_time, 1_000_000_000 * 10_000_000 + crate::io::info::EPOCH_DIFF_100NS);
    t.close(h);
    assert_eq!(t.create(p, FILE_GENERIC_WRITE, 0, FILE_OPEN, SYNC, 0).0, STATUS_ACCESS_DENIED);
    assert_eq!(t.create(p, RW | DELETE, 0, FILE_OPEN, FILE_DELETE_ON_CLOSE, 0).0, STATUS_ACCESS_DENIED);
    assert_eq!(t.create(p, FILE_GENERIC_READ | DELETE, 0, FILE_OPEN, FILE_DELETE_ON_CLOSE, 0).0, STATUS_CANNOT_DELETE);
    // A hidden file is not overwritten by a create that does not say hidden.
    assert_eq!(t.create(p, FILE_GENERIC_READ, 0, FILE_OVERWRITE_IF, SYNC, 0).0, STATUS_ACCESS_DENIED);
    // Clear READONLY and HIDDEN, set a write time.
    let (s, h, _) = t.create(p, FILE_READ_ATTRIBUTES | FILE_WRITE_ATTRIBUTES | SYNCHRONIZE, 0, FILE_OPEN, SYNC, 0);
    assert_eq!(s, STATUS_SUCCESS);
    let mut basic = [0u8; 40];
    let when: i64 = crate::io::info::EPOCH_DIFF_100NS + 5 * 10_000_000;
    basic[16..24].copy_from_slice(&when.to_le_bytes());
    basic[0..8].copy_from_slice(&(when - 1).to_le_bytes());
    basic[32..36].copy_from_slice(&FILE_ATTRIBUTE_NORMAL.to_le_bytes());
    assert_eq!(t.set(h, FILE_BASIC_INFORMATION, &basic), STATUS_SUCCESS);
    let (_, b) = t.query(h, FILE_BASIC_INFORMATION, 40);
    assert_eq!(u32::from_le_bytes(b[32..36].try_into().unwrap()), FILE_ATTRIBUTE_NORMAL);
    assert_eq!(i64::from_le_bytes(b[16..24].try_into().unwrap()), when);
    assert_eq!(i64::from_le_bytes(b[0..8].try_into().unwrap()), when - 1);
    // Setting needs FILE_WRITE_ATTRIBUTES.
    t.close(h);
    let (_, h, _) = t.create(p, FILE_GENERIC_READ, 0, FILE_OPEN, SYNC, 0);
    assert_eq!(t.set(h, FILE_BASIC_INFORMATION, &basic), STATUS_ACCESS_DENIED);
    t.close(h);
    let (s, h, _) = t.create(p, FILE_GENERIC_WRITE, 0, FILE_OPEN, SYNC, 0);
    assert_eq!(s, STATUS_SUCCESS);
    t.close(h);
}

#[test]
fn information_classes() {
    let mut t = setup();
    let h = t.file("\\??\\C:\\info.bin");
    t.write(h, &[7u8; 5000], None);
    let (s, b) = t.query(h, FILE_STANDARD_INFORMATION, 24);
    assert_eq!(s, STATUS_SUCCESS);
    assert_eq!(u64::from_le_bytes(b[8..16].try_into().unwrap()), 5000);
    assert_eq!(u64::from_le_bytes(b[0..8].try_into().unwrap()), 8192);
    assert_eq!((b[20], b[21]), (0, 0));
    assert_eq!(t.query(h, FILE_STANDARD_INFORMATION, 23).0, STATUS_INFO_LENGTH_MISMATCH);
    let (_, b) = t.query(h, FILE_POSITION_INFORMATION, 8);
    assert_eq!(u64::from_le_bytes(b[..8].try_into().unwrap()), 5000);
    assert_eq!(t.set(h, FILE_POSITION_INFORMATION, &10u64.to_le_bytes()), STATUS_SUCCESS);
    assert_eq!(t.read(h, 2, None).1, [7, 7]);
    assert_eq!(t.set(h, FILE_POSITION_INFORMATION, &(-1i64).to_le_bytes()), STATUS_INVALID_PARAMETER);
    assert_eq!(t.set(h, FILE_END_OF_FILE_INFORMATION, &100u64.to_le_bytes()), STATUS_SUCCESS);
    let (_, b) = t.query(h, FILE_NETWORK_OPEN_INFORMATION, 56);
    assert_eq!(u64::from_le_bytes(b[40..48].try_into().unwrap()), 100);
    // All information: the name at 100, with overflow when it does not fit.
    let (s, b) = t.query(h, FILE_ALL_INFORMATION, 200);
    assert_eq!(s, STATUS_SUCCESS);
    assert_eq!(u64::from_le_bytes(b[48..56].try_into().unwrap()), 100);
    assert_eq!(u32::from_le_bytes(b[76..80].try_into().unwrap()) & FILE_READ_DATA, FILE_READ_DATA);
    assert_eq!(u32::from_le_bytes(b[88..92].try_into().unwrap()), SYNC);
    assert_eq!(wide_str(&b[100..]), "\\info.bin");
    let (s, b) = t.query(h, FILE_ALL_INFORMATION, 104);
    assert_eq!((s, b.len()), (STATUS_BUFFER_OVERFLOW, 104));
    assert_eq!(u32::from_le_bytes(b[96..100].try_into().unwrap()), 18);
    assert_eq!(t.query(h, FILE_ALL_INFORMATION, 99).0, STATUS_INFO_LENGTH_MISMATCH);
    assert_eq!(t.query(h, 99, 100).0, STATUS_INVALID_INFO_CLASS);
    let (s, b) = t.query(h, FILE_ATTRIBUTE_TAG_INFORMATION, 8);
    assert_eq!(s, STATUS_SUCCESS);
    assert_eq!(u32::from_le_bytes(b[0..4].try_into().unwrap()), FILE_ATTRIBUTE_ARCHIVE);
    t.close(h);
    // Query by name.
    let oa = t.f.attrs("\\??\\C:\\INFO.BIN", CI);
    let out = t.f.alloc(56);
    assert_eq!(t.f.call(Service::QueryFullAttributesFile, &[oa, out]), STATUS_SUCCESS);
    assert_eq!(t.f.u64_at(out + 40), 100);
    let out = t.f.alloc(40);
    assert_eq!(t.f.call(Service::QueryAttributesFile, &[oa, out]), STATUS_SUCCESS);
    assert_eq!(t.f.u32_at(out + 32), FILE_ATTRIBUTE_ARCHIVE);
    let oa = t.f.attrs("\\??\\C:\\missing", CI);
    assert_eq!(t.f.call(Service::QueryAttributesFile, &[oa, out]), STATUS_OBJECT_NAME_NOT_FOUND);
    assert_eq!(t.f.sys.io.file_count(), 0);
}

#[test]
fn volume_information() {
    let mut t = setup();
    let h = t.file("\\??\\C:\\v");
    let buf = t.f.alloc(64);
    let iosb = t.f.alloc(16);
    let s =
        t.f.call(Service::QueryVolumeInformationFile, &[h, iosb, buf, 64, u64::from(FILE_FS_ATTRIBUTE_INFORMATION)]);
    assert_eq!(s, STATUS_SUCCESS);
    assert_eq!(t.f.u32_at(buf) & FILE_CASE_PRESERVED_NAMES, FILE_CASE_PRESERVED_NAMES);
    assert_eq!(t.f.u32_at(buf) & FILE_CASE_SENSITIVE_SEARCH, 0);
    assert_eq!(t.f.u32_at(buf + 4), 255);
    let len = t.f.u32_at(buf + 8) as usize;
    assert_eq!(wide_str(&t.f.bytes_at(buf + 12, len)), "PolyFS");
    let s = t.f.call(Service::QueryVolumeInformationFile, &[h, iosb, buf, 64, u64::from(FILE_FS_DEVICE_INFORMATION)]);
    assert_eq!(s, STATUS_SUCCESS);
    assert_eq!(t.f.u32_at(buf), FILE_DEVICE_DISK);
    let s = t.f.call(Service::QueryVolumeInformationFile, &[h, iosb, buf, 64, u64::from(FILE_FS_VOLUME_INFORMATION)]);
    assert_eq!(s, STATUS_SUCCESS);
    assert_eq!(wide_str(&t.f.bytes_at(buf + 18, t.f.u32_at(buf + 12) as usize)), "System");
    let s = t.f.call(Service::QueryVolumeInformationFile, &[h, iosb, buf, 64, u64::from(FILE_FS_SIZE_INFORMATION)]);
    assert_eq!(s, STATUS_SUCCESS);
    assert_eq!(t.f.u32_at(buf + 20), 512);
    let s = t.f.call(Service::QueryVolumeInformationFile, &[h, iosb, buf, 4, u64::from(FILE_FS_SIZE_INFORMATION)]);
    assert_eq!(s, STATUS_INFO_LENGTH_MISMATCH);
    assert_eq!(t.f.call(Service::FlushBuffersFile, &[h, iosb]), STATUS_SUCCESS);
    t.close(h);
}

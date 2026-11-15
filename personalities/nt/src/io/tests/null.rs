//! Tests: the null device (`NUL`, `\Device\Null`).

use super::*;

#[test]
fn nul_opens_drops_writes_and_reads_nothing() {
    let mut t = setup();
    // `\\.\nul` is `\??\nul`; the name is matched without case, and every
    // disposition opens the device (git opens `/dev/null` as `nul` for reading and writing).
    for (path, disp) in [
        ("\\??\\nul", FILE_OPEN),
        ("\\??\\NUL", FILE_OPEN_IF),
        ("\\??\\Nul", FILE_CREATE),
        ("\\Device\\Null", FILE_OVERWRITE_IF),
    ] {
        let (s, h, info) = t.create(path, RW, SHARE_ALL, disp, SYNC | FILE_NON_DIRECTORY_FILE, 0);
        assert_eq!(s, STATUS_SUCCESS, "{path}");
        assert_eq!(info, FILE_OPENED, "{path}");
        assert_eq!(t.write(h, b"dropped", None), (STATUS_SUCCESS, 7));
        assert_eq!(t.read(h, 16, None), (STATUS_END_OF_FILE, Vec::new()));
        assert_eq!(t.read(h, 0, None).0, STATUS_SUCCESS);
        // GetFileType asks FileFsDeviceInformation: FILE_DEVICE_NULL is a character device.
        let iosb = t.f.alloc(16);
        let buf = t.f.alloc(64);
        let s =
            t.f.call(Service::QueryVolumeInformationFile, &[h, iosb, buf, 64, u64::from(FILE_FS_DEVICE_INFORMATION)]);
        assert_eq!(s, STATUS_SUCCESS);
        assert_eq!(t.f.bytes_at(buf, 4), FILE_DEVICE_NULL.to_le_bytes());
        let (s, _) = t.query(h, FILE_STANDARD_INFORMATION, 24);
        assert_eq!(s, STATUS_SUCCESS);
        t.close(h);
    }
    // Not a directory, and nothing below it.
    let (s, _, _) = t.create("\\??\\NUL", RW, SHARE_ALL, FILE_OPEN, SYNC | FILE_DIRECTORY_FILE, 0);
    assert_eq!(s, STATUS_NOT_A_DIRECTORY);
    let (s, _, _) = t.create("\\Device\\Null\\x", RW, SHARE_ALL, FILE_OPEN, SYNC, 0);
    assert_eq!(s, STATUS_OBJECT_NAME_NOT_FOUND);
}

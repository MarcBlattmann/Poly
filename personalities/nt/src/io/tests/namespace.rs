//! Tests: delete, rename, hard links, symbolic links and streams.

use super::*;

#[test]
fn delete_by_disposition_delete_on_close_and_nt_delete_file() {
    let mut t = setup();
    // Disposition: gone when the last handle closes.
    let h = t.file("\\??\\C:\\gone.txt");
    let h2 = t.file("\\??\\C:\\gone.txt");
    assert_eq!(t.set(h, FILE_DISPOSITION_INFORMATION, &[1]), STATUS_SUCCESS);
    let (_, b) = t.query(h, FILE_STANDARD_INFORMATION, 24);
    assert_eq!(b[20], 1, "DeletePending");
    assert_eq!(t.create("\\??\\C:\\gone.txt", RW, SHARE_ALL, FILE_OPEN, 0, 0).0, STATUS_DELETE_PENDING);
    t.close(h);
    assert!(t.exists(b"/nt/c/gone.txt"));
    t.close(h2);
    assert!(!t.exists(b"/nt/c/gone.txt"));
    // Undoing the mark keeps the file.
    let h = t.file("\\??\\C:\\kept.txt");
    assert_eq!(t.set(h, FILE_DISPOSITION_INFORMATION, &[1]), STATUS_SUCCESS);
    assert_eq!(t.set(h, FILE_DISPOSITION_INFORMATION, &[0]), STATUS_SUCCESS);
    t.close(h);
    assert!(t.exists(b"/nt/c/kept.txt"));
    // Needs DELETE access.
    let (_, h, _) = t.create("\\??\\C:\\kept.txt", RW, SHARE_ALL, FILE_OPEN, SYNC, 0);
    assert_eq!(t.set(h, FILE_DISPOSITION_INFORMATION, &[1]), STATUS_ACCESS_DENIED);
    t.close(h);
    // FILE_DELETE_ON_CLOSE.
    let (s, h, _) = t.create("\\??\\C:\\temp.txt", RW | DELETE, 0, FILE_CREATE, SYNC | FILE_DELETE_ON_CLOSE, 0);
    assert_eq!(s, STATUS_SUCCESS);
    t.write(h, b"scratch", None);
    assert!(t.exists(b"/nt/c/temp.txt"));
    t.close(h);
    assert!(!t.exists(b"/nt/c/temp.txt"));
    // Directories: only empty ones.
    let (_, d, _) = t.create(
        "\\??\\C:\\full",
        FILE_LIST_DIRECTORY | DELETE | SYNCHRONIZE,
        SHARE_ALL,
        FILE_CREATE,
        FILE_DIRECTORY_FILE | SYNC,
        0,
    );
    let f = t.file("\\??\\C:\\full\\x");
    t.close(f);
    assert_eq!(t.set(d, FILE_DISPOSITION_INFORMATION, &[1]), STATUS_DIRECTORY_NOT_EMPTY);
    // NtDeleteFile.
    let oa = t.f.attrs("\\??\\C:\\FULL\\X", CI);
    assert_eq!(t.f.call(Service::DeleteFile, &[oa]), STATUS_SUCCESS);
    assert!(!t.exists(b"/nt/c/full/x"));
    assert_eq!(t.set(d, FILE_DISPOSITION_INFORMATION, &[1]), STATUS_SUCCESS);
    t.close(d);
    assert!(!t.exists(b"/nt/c/full"));
    // A sharing violation for NtDeleteFile while another handle denies delete.
    let (_, h, _) = t.create("\\??\\C:\\busy", RW, FILE_SHARE_READ, FILE_CREATE, SYNC, 0);
    let oa = t.f.attrs("\\??\\C:\\busy", CI);
    assert_eq!(t.f.call(Service::DeleteFile, &[oa]), STATUS_SHARING_VIOLATION);
    t.close(h);
    // The volume root cannot be deleted.
    let (_, root, _) = t.create("\\??\\C:\\", DELETE | SYNCHRONIZE, SHARE_ALL, FILE_OPEN, SYNC, 0);
    assert_eq!(t.set(root, FILE_DISPOSITION_INFORMATION, &[1]), STATUS_CANNOT_DELETE);
    t.close(root);
}

#[test]
fn rename_in_place_with_replace_and_by_full_path() {
    let mut t = setup();
    let h = t.file("\\??\\C:\\old.txt");
    t.write(h, b"moving", None);
    assert_eq!(t.set(h, FILE_RENAME_INFORMATION, &rename_info(false, 0, "new.txt")), STATUS_SUCCESS);
    assert!(!t.exists(b"/nt/c/old.txt"));
    assert!(t.exists(b"/nt/c/new.txt"));
    let (_, b) = t.query(h, FILE_NAME_INFORMATION, 100);
    assert_eq!(wide_str(&b[4..]), "\\new.txt");
    // Onto an existing file.
    let other = t.file("\\??\\C:\\other.txt");
    t.close(other);
    assert_eq!(t.set(h, FILE_RENAME_INFORMATION, &rename_info(false, 0, "OTHER.txt")), STATUS_OBJECT_NAME_COLLISION);
    assert_eq!(t.set(h, FILE_RENAME_INFORMATION, &rename_info(true, 0, "Other.txt")), STATUS_SUCCESS);
    assert!(!t.exists(b"/nt/c/other.txt"));
    assert!(t.exists(b"/nt/c/Other.txt"));
    // Only the case.
    assert_eq!(t.set(h, FILE_RENAME_INFORMATION, &rename_info(false, 0, "OTHER.TXT")), STATUS_SUCCESS);
    assert!(t.exists(b"/nt/c/OTHER.TXT"));
    assert!(!t.exists(b"/nt/c/Other.txt"));
    // A full path into a directory, and onto an open file.
    let (_, d, _) = t.create(
        "\\??\\C:\\sub",
        FILE_LIST_DIRECTORY | SYNCHRONIZE,
        SHARE_ALL,
        FILE_CREATE,
        FILE_DIRECTORY_FILE | SYNC,
        0,
    );
    assert_eq!(t.set(h, FILE_RENAME_INFORMATION, &rename_info(false, 0, "\\??\\C:\\SUB\\moved.txt")), STATUS_SUCCESS);
    assert!(t.exists(b"/nt/c/sub/moved.txt"));
    let busy = t.file("\\??\\C:\\busy.txt");
    assert_eq!(t.set(h, FILE_RENAME_INFORMATION, &rename_info(true, 0, "\\??\\C:\\busy.txt")), STATUS_ACCESS_DENIED);
    // Relative to a directory handle.
    assert_eq!(t.set(h, FILE_RENAME_INFORMATION, &rename_info(false, d, "again.txt")), STATUS_SUCCESS);
    assert!(t.exists(b"/nt/c/sub/again.txt"));
    // Another volume is another device.
    assert_eq!(t.set(h, FILE_RENAME_INFORMATION, &rename_info(false, 0, "\\??\\Z:\\x.txt")), STATUS_NOT_SAME_DEVICE);
    // Needs DELETE access.
    let (_, ro, _) = t.create("\\??\\C:\\sub\\again.txt", FILE_GENERIC_READ, SHARE_ALL, FILE_OPEN, SYNC, 0);
    assert_eq!(t.set(ro, FILE_RENAME_INFORMATION, &rename_info(false, 0, "nope")), STATUS_ACCESS_DENIED);
    t.close(ro);
    assert_eq!(t.read(h, 6, Some(0)).1, b"moving");
    for x in [h, busy, d] {
        t.close(x);
    }
}

#[test]
fn hard_links_by_file_link_information() {
    let mut t = setup();
    let h = t.file("\\??\\C:\\one.txt");
    t.write(h, b"shared", None);
    // A second name in the same directory: the same file, two links.
    assert_eq!(t.set(h, FILE_LINK_INFORMATION, &rename_info(false, 0, "two.txt")), STATUS_SUCCESS);
    assert!(t.exists(b"/nt/c/one.txt") && t.exists(b"/nt/c/two.txt"));
    let (_, std) = t.query(h, FILE_STANDARD_INFORMATION, 24);
    assert_eq!(u32::from_le_bytes(std[16..20].try_into().unwrap()), 2, "NumberOfLinks");
    let two = t.file("\\??\\C:\\two.txt");
    assert_eq!(t.read(two, 6, Some(0)).1, b"shared");
    t.close(two);
    // An existing name: a collision unless it may be replaced, and an open
    // file is not replaced.
    let other = t.file("\\??\\C:\\other.txt");
    assert_eq!(t.set(h, FILE_LINK_INFORMATION, &rename_info(false, 0, "OTHER.TXT")), STATUS_OBJECT_NAME_COLLISION);
    assert_eq!(t.set(h, FILE_LINK_INFORMATION, &rename_info(true, 0, "other.txt")), STATUS_ACCESS_DENIED);
    t.close(other);
    assert_eq!(t.set(h, FILE_LINK_INFORMATION, &rename_info(true, 0, "other.txt")), STATUS_SUCCESS);
    let o = t.file("\\??\\C:\\other.txt");
    assert_eq!(t.read(o, 6, Some(0)).1, b"shared");
    t.close(o);
    // By full path into a directory; not onto another volume.
    let (_, d, _) = t.create(
        "\\??\\C:\\sub",
        FILE_LIST_DIRECTORY | SYNCHRONIZE,
        SHARE_ALL,
        FILE_CREATE,
        FILE_DIRECTORY_FILE | SYNC,
        0,
    );
    assert_eq!(t.set(h, FILE_LINK_INFORMATION, &rename_info(false, 0, "\\??\\C:\\sub\\three.txt")), STATUS_SUCCESS);
    assert!(t.exists(b"/nt/c/sub/three.txt"));
    assert_eq!(t.set(h, FILE_LINK_INFORMATION, &rename_info(false, 0, "\\??\\Z:\\x.txt")), STATUS_NOT_SAME_DEVICE);
    // A directory gets no second name.
    assert_eq!(t.set(d, FILE_LINK_INFORMATION, &rename_info(false, 0, "\\??\\C:\\dir2")), STATUS_FILE_IS_A_DIRECTORY);
    // The file object keeps its own name.
    let (_, b) = t.query(h, FILE_NAME_INFORMATION, 100);
    assert_eq!(wide_str(&b[4..]), "\\one.txt");
    t.close(d);
    t.close(h);
}

#[test]
fn a_symbolic_link_reads_as_a_reparse_point() {
    let mut t = setup();
    let ctx = t.vfs.root_context().unwrap();
    t.vfs_create(b"/nt/c/target.txt", b"x");
    t.vfs.symlink(&ctx, &Cred::root(), b"../c/target.txt", At::Cwd, b"/nt/c/rel").unwrap();
    t.vfs.symlink(&ctx, &Cred::root(), b"/etc/hosts", At::Cwd, b"/nt/c/abs").unwrap();
    let control = |t: &mut T, name: &str, outlen: usize| {
        let (s, h, _) =
            t.create(name, FILE_READ_ATTRIBUTES | SYNCHRONIZE, SHARE_ALL, FILE_OPEN, SYNC | FILE_OPEN_REPARSE_POINT, 0);
        assert_eq!(s, STATUS_SUCCESS, "{name}");
        let out = t.f.alloc(outlen.max(1));
        let iosb = t.f.alloc(16);
        let s = t.f.call(
            Service::DeviceIoControlFile,
            &[h, 0, 0, 0, iosb, u64::from(FSCTL_GET_REPARSE_POINT), 0, 0, out, outlen as u64],
        );
        let n = t.f.u64_at(iosb + 8) as usize;
        let b = t.f.bytes_at(out, n.min(outlen));
        t.close(h);
        (s, b)
    };
    let u16_at = |b: &[u8], o: usize| usize::from(u16::from_le_bytes([b[o], b[o + 1]]));
    let name_at = |b: &[u8], off: usize, len: usize| wide_str(&b[20 + off..20 + off + len]);
    let (s, b) = control(&mut t, "\\??\\C:\\rel", 512);
    assert_eq!(s, STATUS_SUCCESS);
    assert_eq!(u32::from_le_bytes(b[0..4].try_into().unwrap()), IO_REPARSE_TAG_SYMLINK);
    assert_eq!(u16_at(&b, 4), b.len() - 8, "ReparseDataLength");
    assert_eq!(name_at(&b, u16_at(&b, 8), u16_at(&b, 10)), "..\\c\\target.txt");
    assert_eq!(name_at(&b, u16_at(&b, 12), u16_at(&b, 14)), "..\\c\\target.txt");
    assert_eq!(u32::from_le_bytes(b[16..20].try_into().unwrap()), SYMLINK_FLAG_RELATIVE);
    let (s, b) = control(&mut t, "\\??\\C:\\abs", 512);
    assert_eq!(s, STATUS_SUCCESS);
    assert_eq!(name_at(&b, u16_at(&b, 8), u16_at(&b, 10)), "\\??\\Z:\\etc\\hosts");
    assert_eq!(name_at(&b, u16_at(&b, 12), u16_at(&b, 14)), "Z:\\etc\\hosts");
    assert_eq!(u32::from_le_bytes(b[16..20].try_into().unwrap()), 0);
    // Short buffers; a plain file; another control code.
    assert_eq!(control(&mut t, "\\??\\C:\\rel", 4).0, STATUS_BUFFER_TOO_SMALL);
    let (s, b) = control(&mut t, "\\??\\C:\\rel", 24);
    assert_eq!((s, b.len()), (STATUS_BUFFER_OVERFLOW, 24));
    assert_eq!(control(&mut t, "\\??\\C:\\target.txt", 512).0, STATUS_NOT_A_REPARSE_POINT);
    // Setting one: an empty file or directory becomes a symbolic link.
    let (_, rel) = control(&mut t, "\\??\\C:\\rel", 512);
    let (_, abs) = control(&mut t, "\\??\\C:\\abs", 512);
    let set = |t: &mut T, name: &str, options: u32, data: &[u8]| {
        let (s, h, _) = t.create(name, FILE_WRITE_ATTRIBUTES | SYNCHRONIZE, SHARE_ALL, FILE_CREATE, SYNC | options, 0);
        assert_eq!(s, STATUS_SUCCESS, "{name}");
        let inp = t.f.put(data);
        let iosb = t.f.alloc(16);
        let s = t.f.call(
            Service::DeviceIoControlFile,
            &[h, 0, 0, 0, iosb, u64::from(FSCTL_SET_REPARSE_POINT), inp, data.len() as u64, 0, 0],
        );
        t.close(h);
        s
    };
    assert_eq!(set(&mut t, "\\??\\C:\\newlink", FILE_NON_DIRECTORY_FILE, &rel), STATUS_SUCCESS);
    assert_eq!(t.vfs.readlink(&ctx, &Cred::root(), At::Cwd, b"/nt/c/newlink").unwrap(), b"../c/target.txt");
    assert_eq!(set(&mut t, "\\??\\C:\\newdir", FILE_DIRECTORY_FILE, &abs), STATUS_SUCCESS);
    assert_eq!(t.vfs.readlink(&ctx, &Cred::root(), At::Cwd, b"/nt/c/newdir").unwrap(), b"/etc/hosts");
    // A target on C: is below C:'s directory in the VFS.
    let mut on_c = abs.clone();
    let units: Vec<u8> = "\\??\\C:\\target.txt".encode_utf16().flat_map(u16::to_le_bytes).collect();
    on_c.truncate(20);
    on_c[8..12].copy_from_slice(&[0, 0, units.len() as u8, 0]);
    on_c[12..16].copy_from_slice(&[0, 0, 0, 0]);
    on_c.extend_from_slice(&units);
    let len = (on_c.len() - 8) as u16;
    on_c[4..6].copy_from_slice(&len.to_le_bytes());
    assert_eq!(set(&mut t, "\\??\\C:\\onc", FILE_NON_DIRECTORY_FILE, &on_c), STATUS_SUCCESS);
    assert_eq!(t.vfs.readlink(&ctx, &Cred::root(), At::Cwd, b"/nt/c/onc").unwrap(), b"/nt/c/target.txt");
    // Garbage and other tags.
    assert_eq!(set(&mut t, "\\??\\C:\\bad1", FILE_NON_DIRECTORY_FILE, &rel[..10]), STATUS_INVALID_PARAMETER);
    let mut other = rel.clone();
    other[0..4].copy_from_slice(&0xA000_0003u32.to_le_bytes());
    assert_eq!(set(&mut t, "\\??\\C:\\bad2", FILE_NON_DIRECTORY_FILE, &other), STATUS_NOT_SUPPORTED);
}

#[test]
fn a_file_has_its_default_data_stream_only() {
    let mut t = setup();
    let h = t.file("\\??\\C:\\data.txt");
    t.write(h, b"twelve bytes", None);
    let (s, b) = t.query(h, FILE_STREAM_INFORMATION, 100);
    assert_eq!(s, STATUS_SUCCESS);
    assert_eq!(b.len(), STREAM_INFO_FIXED + 14);
    assert_eq!(u32::from_le_bytes(b[0..4].try_into().unwrap()), 0, "the last entry");
    assert_eq!(u32::from_le_bytes(b[4..8].try_into().unwrap()), 14);
    assert_eq!(u64::from_le_bytes(b[8..16].try_into().unwrap()), 12);
    assert_eq!(wide_str(&b[24..]), "::$DATA");
    // A buffer for the fixed part only: the name does not fit.
    assert_eq!(t.query(h, FILE_STREAM_INFORMATION, 30).0, STATUS_BUFFER_OVERFLOW);
    assert_eq!(t.query(h, FILE_STREAM_INFORMATION, 8).0, STATUS_INFO_LENGTH_MISMATCH);
    t.close(h);
    // The default stream's full name opens the file; another stream is no
    // name on Poly's volumes.
    let (s, h, _) = t.create("\\??\\C:\\data.txt::$data", FILE_GENERIC_READ, SHARE_ALL, FILE_OPEN, SYNC, 0);
    assert_eq!(s, STATUS_SUCCESS);
    assert_eq!(t.read(h, 12, None).1, b"twelve bytes");
    t.close(h);
    let (s, _, _) = t.create("\\??\\C:\\data.txt:alt", FILE_GENERIC_READ, SHARE_ALL, FILE_OPEN, SYNC, 0);
    assert_eq!(s, STATUS_OBJECT_NAME_INVALID);
    // A directory has no data stream.
    let (_, d, _) = t.create(
        "\\??\\C:\\dir",
        FILE_LIST_DIRECTORY | SYNCHRONIZE,
        SHARE_ALL,
        FILE_CREATE,
        FILE_DIRECTORY_FILE | SYNC,
        0,
    );
    let (s, b) = t.query(d, FILE_STREAM_INFORMATION, 100);
    assert_eq!((s, b.len()), (STATUS_SUCCESS, 0));
    t.close(d);
}

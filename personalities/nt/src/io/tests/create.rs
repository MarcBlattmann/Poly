//! Tests: the namespace, creating and opening, names and case, sharing.

use super::*;

#[test]
fn drive_letters_and_devices_are_in_the_namespace() {
    let mut t = setup();
    assert!(t.exists(b"/nt/c"));
    let om = &mut t.f.sys.objects;
    let names = |om: &mut poly_ntobj::ObjectManager, dir: &str| -> Vec<(String, &'static str)> {
        let id = om.lookup(&poly_ntobj::name::wide(dir), true).unwrap();
        om.directory_entries(id).unwrap().into_iter().map(|(n, k)| (String::from_utf16(&n).unwrap(), k)).collect()
    };
    let devices = names(om, "\\Device");
    for d in ["HarddiskVolume1", "HarddiskVolume2", "PolyPipe"] {
        assert!(devices.contains(&(String::from(d), "Device")), "{d}");
    }
    let dos = names(om, "\\??");
    assert!(dos.contains(&(String::from("C:"), "SymbolicLink")));
    assert!(dos.contains(&(String::from("Z:"), "SymbolicLink")));
    assert_eq!(t.f.sys.io.volumes[1].path, b"/nt/c");
    assert_eq!(t.f.sys.io.volumes[0].letter, Some('Z'));
}

/// ADR 0185: the volume devices are named like Windows' disk volumes, the
/// system volume (`C:`) has the lowest number, and nothing is called
/// `PolyFs` any more.
#[test]
fn volume_devices_are_named_like_windows_disk_volumes() {
    let mut t = setup();
    let ctx = t.vfs.root_context().unwrap();
    let configs = default_volumes(t.vfs, &ctx);
    let named = |letter: char| configs.iter().find(|c| c.letter == Some(letter)).unwrap().device.clone();
    assert_eq!((named('C').as_str(), named('Z').as_str()), ("HarddiskVolume1", "HarddiskVolume2"));
    assert!(configs.iter().all(|c| c.device.starts_with("HarddiskVolume")));
    let numbers: Vec<usize> = configs.iter().map(|c| c.device["HarddiskVolume".len()..].parse().unwrap()).collect();
    let mut sorted = numbers.clone();
    sorted.sort_unstable();
    sorted.dedup();
    assert_eq!(sorted.len(), numbers.len(), "every volume has its own number");
    assert_eq!(sorted[0], 1);

    // The drive letters are links to those devices (`NtQuerySymbolicLinkObject`).
    let target = |t: &mut T, link: &str| {
        let out = t.f.alloc(8);
        let oa = t.f.attrs(link, CI);
        assert_eq!(t.f.call(Service::OpenSymbolicLinkObject, &[out, 1, oa]), STATUS_SUCCESS, "{link}");
        let h = t.f.u64_at(out);
        let buf = t.f.alloc(256);
        let mut header = Vec::new();
        header.extend_from_slice(&0u16.to_le_bytes());
        header.extend_from_slice(&256u16.to_le_bytes());
        header.extend_from_slice(&[0; 4]);
        header.extend_from_slice(&buf.to_le_bytes());
        let ustr = t.f.put(&header);
        assert_eq!(t.f.call(Service::QuerySymbolicLinkObject, &[h, ustr, 0]), STATUS_SUCCESS);
        let len = usize::from(u16::from_le_bytes(t.f.bytes_at(ustr, 2).try_into().unwrap()));
        wide_str(&t.f.bytes_at(buf, len))
    };
    assert_eq!(target(&mut t, "\\??\\C:"), "\\Device\\HarddiskVolume1");
    assert_eq!(target(&mut t, "\\??\\Z:"), "\\Device\\HarddiskVolume2");
    let devices = {
        let om = &mut t.f.sys.objects;
        let id = om.lookup(&poly_ntobj::name::wide("\\Device"), true).unwrap();
        om.directory_entries(id).unwrap().into_iter().map(|(n, _)| String::from_utf16(&n).unwrap()).collect::<Vec<_>>()
    };
    assert!(devices.iter().all(|d| !d.starts_with("PolyFs")), "{devices:?}");

    // A file's object name (`NtQueryObject`, ObjectNameInformation): its
    // device and the path below it, the same file on both letters.
    let object_name = |t: &mut T, h: u64| {
        let buf = t.f.alloc(528);
        let ret = t.f.alloc(8);
        let s = t.f.call(Service::QueryObject, &[h, 1, buf, 528, ret]);
        let b = t.f.bytes_at(buf, 528);
        let len = usize::from(u16::from_le_bytes([b[0], b[1]]));
        (s, wide_str(&b[16..16 + len]))
    };
    let on_c = t.file("\\??\\C:\\a.txt");
    assert_eq!(object_name(&mut t, on_c), (STATUS_SUCCESS, "\\Device\\HarddiskVolume1\\a.txt".into()));
    let on_z = t.file("\\??\\Z:\\nt\\c\\a.txt");
    assert_eq!(object_name(&mut t, on_z), (STATUS_SUCCESS, "\\Device\\HarddiskVolume2\\nt\\c\\a.txt".into()));
    // Opening by the device name works as it does on Windows.
    let by_device = t.file("\\Device\\HarddiskVolume1\\a.txt");
    assert_eq!(object_name(&mut t, by_device).1, "\\Device\\HarddiskVolume1\\a.txt");
    // Poly's old device names are gone.
    assert_ne!(t.create("\\Device\\PolyFs1\\a.txt", RW, SHARE_ALL, FILE_OPEN, SYNC, 0).0, STATUS_SUCCESS);
}

#[test]
fn create_write_read_close_under_c() {
    let mut t = setup();
    let (s, h, info) = t.create("\\??\\C:\\hello.txt", RW, 0, FILE_CREATE, SYNC, FILE_ATTRIBUTE_NORMAL);
    assert_eq!((s, info), (STATUS_SUCCESS, FILE_CREATED));
    assert_eq!(t.write(h, b"hello, world", None), (STATUS_SUCCESS, 12));
    // The position moved; a read at the position hits the end.
    assert_eq!(t.read(h, 4, None).0, STATUS_END_OF_FILE);
    assert_eq!(t.read(h, 5, Some(0)), (STATUS_SUCCESS, b"hello".to_vec()));
    // The explicit offset moved the position too (synchronous handle).
    assert_eq!(t.read(h, 100, None), (STATUS_SUCCESS, b", world".to_vec()));
    t.close(h);
    assert_eq!(t.f.sys.io.file_count(), 0);
    assert!(t.exists(b"/nt/c/hello.txt"));
    // And it is on Z: too, the whole VFS.
    let (s, h, info) = t.create("\\??\\Z:\\nt\\c\\hello.txt", FILE_GENERIC_READ, 0, FILE_OPEN, SYNC, 0);
    assert_eq!((s, info), (STATUS_SUCCESS, FILE_OPENED));
    assert_eq!(t.read(h, 100, None), (STATUS_SUCCESS, b"hello, world".to_vec()));
    t.close(h);
}

#[test]
fn create_dispositions() {
    let mut t = setup();
    let p = "\\??\\C:\\d.txt";
    assert_eq!(t.create(p, RW, 0, FILE_OPEN, SYNC, 0).0, STATUS_OBJECT_NAME_NOT_FOUND);
    assert_eq!(t.create(p, RW, 0, FILE_OVERWRITE, SYNC, 0).0, STATUS_OBJECT_NAME_NOT_FOUND);
    let (s, h, info) = t.create(p, RW, 0, FILE_OPEN_IF, SYNC, 0);
    assert_eq!((s, info), (STATUS_SUCCESS, FILE_CREATED));
    t.write(h, b"0123456789", None);
    t.close(h);
    assert_eq!(t.create(p, RW, 0, FILE_CREATE, SYNC, 0).0, STATUS_OBJECT_NAME_COLLISION);
    let (s, h, info) = t.create(p, RW, 0, FILE_OPEN_IF, SYNC, 0);
    assert_eq!((s, info), (STATUS_SUCCESS, FILE_OPENED));
    assert_eq!(t.read(h, 3, None).1, b"012");
    t.close(h);
    let (s, h, info) = t.create(p, RW, 0, FILE_OVERWRITE, SYNC, 0);
    assert_eq!((s, info), (STATUS_SUCCESS, FILE_OVERWRITTEN));
    assert_eq!(t.read(h, 3, None).0, STATUS_END_OF_FILE);
    t.write(h, b"abc", None);
    t.close(h);
    let (s, h, info) = t.create(p, RW, 0, FILE_SUPERSEDE, SYNC, 0);
    assert_eq!((s, info), (STATUS_SUCCESS, FILE_SUPERSEDED));
    t.close(h);
    let (s, h, info) = t.create("\\??\\C:\\new.txt", RW, 0, FILE_OVERWRITE_IF, SYNC, 0);
    assert_eq!((s, info), (STATUS_SUCCESS, FILE_CREATED));
    t.close(h);
    let (s, h, info) = t.create("\\??\\C:\\new2.txt", RW, 0, FILE_SUPERSEDE, SYNC, 0);
    assert_eq!((s, info), (STATUS_SUCCESS, FILE_CREATED));
    t.close(h);
}

#[test]
fn create_parameter_and_path_errors() {
    let mut t = setup();
    let p = "\\??\\C:\\x";
    let inval = STATUS_INVALID_PARAMETER;
    assert_eq!(t.create(p, RW, 0, 6, SYNC, 0).0, inval);
    assert_eq!(t.create(p, RW, 0, FILE_CREATE, FILE_DIRECTORY_FILE | FILE_NON_DIRECTORY_FILE, 0).0, inval);
    assert_eq!(t.create(p, RW, 0, FILE_OVERWRITE_IF, FILE_DIRECTORY_FILE, 0).0, inval);
    assert_eq!(t.create(p, RW, 0, FILE_CREATE, SYNC | FILE_SYNCHRONOUS_IO_ALERT, 0).0, inval);
    assert_eq!(t.create(p, FILE_READ_DATA, 0, FILE_CREATE, SYNC, 0).0, inval);
    assert_eq!(t.create(p, RW, 0, FILE_CREATE, FILE_DELETE_ON_CLOSE, 0).0, inval);
    assert_eq!(t.create(p, RW, 8, FILE_CREATE, 0, 0).0, inval);
    assert_eq!(t.create("\\??\\C:\\no\\such\\file", RW, 0, FILE_CREATE, 0, 0).0, STATUS_OBJECT_PATH_NOT_FOUND);
    assert_eq!(t.create("\\??\\C:\\a*b", RW, 0, FILE_CREATE, 0, 0).0, STATUS_OBJECT_NAME_INVALID);
    assert_eq!(t.create("\\??\\C:\\a\\..\\b", RW, 0, FILE_CREATE, 0, 0).0, STATUS_OBJECT_NAME_INVALID);
    assert_eq!(t.create("\\??\\C:\\f:", RW, 0, FILE_CREATE, 0, 0).0, STATUS_OBJECT_NAME_INVALID);
    assert_eq!(t.create("\\??\\C:\\f:s:$INDEX", RW, 0, FILE_CREATE, 0, 0).0, STATUS_OBJECT_NAME_INVALID);
    assert_eq!(t.create("\\??\\C:\\d:s\\f", RW, 0, FILE_CREATE, 0, 0).0, STATUS_OBJECT_NAME_INVALID);
    assert_eq!(t.create("\\??\\Q:\\x", RW, 0, FILE_CREATE, 0, 0).0, STATUS_OBJECT_PATH_NOT_FOUND);
    let h = t.file("\\??\\C:\\plain");
    t.close(h);
    assert_eq!(t.create("\\??\\C:\\plain", RW, 0, FILE_OPEN, FILE_DIRECTORY_FILE, 0).0, STATUS_NOT_A_DIRECTORY);
    assert_eq!(t.create("\\??\\C:\\plain\\", RW, 0, FILE_OPEN, 0, 0).0, STATUS_OBJECT_NAME_INVALID);
    assert_eq!(t.create("\\??\\C:\\plain\\x", RW, 0, FILE_OPEN, 0, 0).0, STATUS_OBJECT_PATH_NOT_FOUND);
    let (s, h, info) =
        t.create("\\??\\C:\\Dir", FILE_LIST_DIRECTORY | SYNCHRONIZE, 0, FILE_CREATE, FILE_DIRECTORY_FILE, 0);
    assert_eq!((s, info), (STATUS_SUCCESS, FILE_CREATED));
    t.close(h);
    assert!(t.exists(b"/nt/c/Dir"));
    assert_eq!(t.create("\\??\\C:\\dir", RW, 0, FILE_OPEN, FILE_NON_DIRECTORY_FILE, 0).0, STATUS_FILE_IS_A_DIRECTORY);
    assert_eq!(t.create("\\??\\C:\\dir", RW, 0, FILE_OVERWRITE, 0, 0).0, STATUS_FILE_IS_A_DIRECTORY);
    // Reading a directory is not a thing.
    let (s, h, _) = t.create("\\??\\C:\\DIR\\", FILE_GENERIC_READ, SHARE_ALL, FILE_OPEN, SYNC, 0);
    assert_eq!(s, STATUS_SUCCESS);
    assert_eq!(t.read(h, 1, None).0, STATUS_INVALID_DEVICE_REQUEST);
    t.close(h);
    // No extended attributes.
    let out = t.f.alloc(8);
    let iosb = t.f.alloc(16);
    let oa = t.f.attrs("\\??\\C:\\ea", CI);
    let ea = t.f.alloc(16);
    let s = t.f.call(Service::CreateFile, &[out, u64::from(RW), oa, iosb, 0, 0, 0, u64::from(FILE_CREATE), 0, ea, 16]);
    assert_eq!(s, STATUS_EAS_NOT_SUPPORTED);
}

#[test]
fn case_insensitive_case_preserving_lookup() {
    let mut t = setup();
    let (s, h, _) = t.create("\\??\\C:\\MixedCase.TXT", RW, SHARE_ALL, FILE_CREATE, SYNC, 0);
    assert_eq!(s, STATUS_SUCCESS);
    t.write(h, b"data", None);
    t.close(h);
    assert!(t.exists(b"/nt/c/MixedCase.TXT"));
    let (s, h, info) = t.create("\\??\\c:\\MIXEDcase.txt", FILE_GENERIC_READ, SHARE_ALL, FILE_OPEN, SYNC, 0);
    assert_eq!((s, info), (STATUS_SUCCESS, FILE_OPENED));
    assert_eq!(t.read(h, 10, None).1, b"data");
    // The name it reports is the one on disk.
    let (s, b) = t.query(h, FILE_NAME_INFORMATION, 100);
    assert_eq!(s, STATUS_SUCCESS);
    assert_eq!(wide_str(&b[4..]), "\\MixedCase.TXT");
    t.close(h);
    // A create in another case finds the file.
    assert_eq!(t.create("\\??\\C:\\mixedcase.txt", RW, 0, FILE_CREATE, 0, 0).0, STATUS_OBJECT_NAME_COLLISION);
    // Directories on the way too.
    let (s, h, _) =
        t.create("\\??\\C:\\SubDir", FILE_LIST_DIRECTORY | SYNCHRONIZE, 0, FILE_CREATE, FILE_DIRECTORY_FILE, 0);
    assert_eq!(s, STATUS_SUCCESS);
    t.close(h);
    let h = t.file("\\??\\C:\\subdir\\Inner.txt");
    t.close(h);
    assert!(t.exists(b"/nt/c/SubDir/Inner.txt"));
}

#[test]
fn names_that_differ_only_in_case() {
    let mut t = setup();
    t.vfs_create(b"/nt/c/readme", b"lower");
    t.vfs_create(b"/nt/c/README", b"upper");
    t.vfs_create(b"/nt/c/ReadMe", b"mixed");
    fn read(t: &mut T, name: &str) -> Vec<u8> {
        let (s, h, _) = t.create(name, FILE_GENERIC_READ, SHARE_ALL, FILE_OPEN, SYNC, 0);
        assert_eq!(s, STATUS_SUCCESS);
        let r = t.read(h, 10, None).1;
        t.close(h);
        r
    }
    // Exact matches win; otherwise the smallest name as bytes.
    assert_eq!(read(&mut t, "\\??\\C:\\readme"), b"lower");
    assert_eq!(read(&mut t, "\\??\\C:\\ReadMe"), b"mixed");
    assert_eq!(read(&mut t, "\\??\\C:\\README"), b"upper");
    assert_eq!(read(&mut t, "\\??\\C:\\rEADME"), b"upper");
}

#[test]
fn case_insensitive_lookup_stays_right_while_a_directory_changes() {
    // The lookups that find nothing are answered from an index of the
    // directory (ADR 0091); every change must show in what they find.
    let mut t = setup();
    let open = |t: &mut T, name: &str| t.create(name, FILE_GENERIC_READ, SHARE_ALL, FILE_OPEN, SYNC, 0);
    let (s, d, _) =
        t.create("\\??\\C:\\Many", FILE_LIST_DIRECTORY | SYNCHRONIZE, SHARE_ALL, FILE_CREATE, FILE_DIRECTORY_FILE, 0);
    assert_eq!(s, STATUS_SUCCESS);
    // Files made one after another, each found in another case at once.
    for i in 0..40 {
        let (s, h, info) =
            t.create(&alloc::format!("\\??\\C:\\Many\\File{i}.Txt"), RW, SHARE_ALL, FILE_CREATE, SYNC, 0);
        assert_eq!((s, info), (STATUS_SUCCESS, FILE_CREATED), "{i}");
        t.close(h);
        let (s, h, _) = open(&mut t, &alloc::format!("\\??\\C:\\MANY\\FILE{i}.TXT"));
        assert_eq!(s, STATUS_SUCCESS, "{i}");
        t.close(h);
        assert_eq!(
            t.create(&alloc::format!("\\??\\c:\\many\\file{i}.txt"), RW, 0, FILE_CREATE, 0, 0).0,
            STATUS_OBJECT_NAME_COLLISION
        );
    }
    assert!(t.exists(b"/nt/c/Many/File39.Txt"));
    // A file another program made (straight through the VFS) is found in
    // another case; one it removed is not.
    t.vfs_create(b"/nt/c/Many/Outside.txt", b"x");
    assert_eq!(open(&mut t, "\\??\\C:\\Many\\OUTSIDE.TXT").0, STATUS_SUCCESS);
    let ctx = t.vfs.root_context().unwrap();
    t.vfs.unlink(&ctx, &Cred::root(), At::Cwd, b"/nt/c/Many/File3.Txt").unwrap();
    assert_eq!(open(&mut t, "\\??\\C:\\Many\\file3.txt").0, STATUS_OBJECT_NAME_NOT_FOUND);
    // A rename: the new name is found in any case, the old one no longer.
    let (_, h, _) = t.create("\\??\\C:\\Many\\file5.txt", RW | DELETE, SHARE_ALL, FILE_OPEN, SYNC, 0);
    assert_eq!(t.set(h, FILE_RENAME_INFORMATION, &rename_info(false, 0, "Renamed.txt")), STATUS_SUCCESS);
    t.close(h);
    assert_eq!(open(&mut t, "\\??\\C:\\Many\\RENAMED.TXT").0, STATUS_SUCCESS);
    assert_eq!(open(&mut t, "\\??\\C:\\Many\\FILE5.TXT").0, STATUS_OBJECT_NAME_NOT_FOUND);
    // Two spellings made outside: the smallest as bytes is the one found.
    t.vfs_create(b"/nt/c/Many/twin", b"lower");
    t.vfs_create(b"/nt/c/Many/TWIN", b"upper");
    let (s, h, _) = open(&mut t, "\\??\\C:\\Many\\Twin");
    assert_eq!(s, STATUS_SUCCESS);
    assert_eq!(t.read(h, 8, None).1, b"upper");
    t.close(h);
    t.close(d);
}

#[test]
fn share_modes_across_handles() {
    let mut t = setup();
    let p = "\\??\\C:\\shared.txt";
    let (s, h1, _) = t.create(p, FILE_GENERIC_READ, FILE_SHARE_READ, FILE_OPEN_IF, SYNC, 0);
    assert_eq!(s, STATUS_SUCCESS);
    assert_eq!(t.create(p, FILE_GENERIC_WRITE, FILE_SHARE_READ, FILE_OPEN, SYNC, 0).0, STATUS_SHARING_VIOLATION);
    assert_eq!(t.create(p, FILE_GENERIC_READ, FILE_SHARE_WRITE, FILE_OPEN, SYNC, 0).0, STATUS_SHARING_VIOLATION);
    let (s, h2, _) = t.create(p, FILE_GENERIC_READ, FILE_SHARE_READ, FILE_OPEN, SYNC, 0);
    assert_eq!(s, STATUS_SUCCESS);
    // Attribute-only opens never conflict.
    let (s, h3, _) = t.create(p, FILE_READ_ATTRIBUTES | SYNCHRONIZE, 0, FILE_OPEN, SYNC, 0);
    assert_eq!(s, STATUS_SUCCESS);
    // An overwrite refused for sharing leaves the file alone.
    t.close(h1);
    t.close(h2);
    let (_, w, _) = t.create(p, RW, FILE_SHARE_READ, FILE_OPEN, SYNC, 0);
    t.write(w, b"keep", None);
    assert_eq!(t.create(p, RW, SHARE_ALL, FILE_OVERWRITE, SYNC, 0).0, STATUS_SHARING_VIOLATION);
    assert_eq!(t.read(w, 4, Some(0)).1, b"keep");
    t.close(w);
    t.close(h3);
    let (s, h, _) = t.create(p, FILE_GENERIC_WRITE, 0, FILE_OPEN, SYNC, 0);
    assert_eq!(s, STATUS_SUCCESS);
    t.close(h);
}

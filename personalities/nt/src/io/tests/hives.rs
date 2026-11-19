//! Tests: hive files of programs (`NtSaveKey`, `NtLoadKey`, `NtUnloadKey`,
//! ADR 0224) on the tmpfs VFS, as `RegSaveKeyW.yaml`, `RegLoadKeyW.yaml` and
//! `RegUnLoadKeyW.yaml` describe them.

use super::*;
use poly_registry::consts::{KEY_ALL_ACCESS, REG_DWORD, STATUS_KEY_DELETED, STATUS_REGISTRY_IO_FAILED};

fn key(t: &mut T, root: u64, name: &str, create: bool) -> (NtStatus, u64) {
    let out = t.f.alloc(8);
    let oa = t.f.attrs_in(root, name, CI);
    let s = if create {
        t.f.call(Service::CreateKey, &[out, u64::from(KEY_ALL_ACCESS), oa, 0, 0, 0, 0])
    } else {
        t.f.call(Service::OpenKey, &[out, u64::from(KEY_ALL_ACCESS), oa])
    };
    (s, t.f.u64_at(out))
}

fn set_dword(t: &mut T, h: u64, name: &str, v: u32) {
    let n = t.f.unicode(name);
    let d = t.f.put(&v.to_le_bytes());
    assert_eq!(t.f.call(Service::SetValueKey, &[h, n, 0, u64::from(REG_DWORD), d, 4]), STATUS_SUCCESS);
}

/// The DWORD value `name` of key `h`.
fn dword(t: &mut T, h: u64, name: &str) -> Option<u32> {
    let n = t.f.unicode(name);
    let buf = t.f.alloc(64);
    let ret = t.f.alloc(4);
    // KEY_VALUE_PARTIAL_INFORMATION: the data at 12.
    let s = t.f.call(Service::QueryValueKey, &[h, n, 2, buf, 64, ret]);
    (s == STATUS_SUCCESS).then(|| t.f.u32_at(buf + 12))
}

fn load(t: &mut T, target: &str, file: &str) -> NtStatus {
    let a = t.f.attrs(target, CI);
    let b = t.f.attrs(file, CI);
    t.f.call(Service::LoadKey, &[a, b])
}

fn unload(t: &mut T, target: &str) -> NtStatus {
    let a = t.f.attrs(target, CI);
    t.f.call(Service::UnloadKey, &[a])
}

fn vfs_bytes(t: &T, path: &[u8]) -> Vec<u8> {
    let ctx = t.vfs.root_context().unwrap();
    let f = t.vfs.open(&ctx, &Cred::root(), At::Cwd, path, OpenFlags::RDONLY, 0).unwrap();
    let mut out = alloc::vec![0u8; 1 << 16];
    let n = f.read_at(0, &mut out).unwrap();
    out.truncate(n);
    out
}

#[test]
fn save_load_write_unload_reload() {
    let mut t = setup();
    // A key with a value and a subkey, saved to a new file.
    let (s, src) = key(&mut t, 0, "\\Registry\\Machine\\SOFTWARE\\Src", true);
    assert_eq!(s, STATUS_SUCCESS);
    set_dword(&mut t, src, "D", 42);
    let (_, sub) = key(&mut t, src, "Sub", true);
    set_dword(&mut t, sub, "X", 5);
    t.close(sub);
    let (s, fh, _) = t.create("\\??\\C:\\src.hiv", RW, 0, FILE_CREATE, SYNC | FILE_NON_DIRECTORY_FILE, 0);
    assert_eq!(s, STATUS_SUCCESS);
    assert_eq!(t.f.call(Service::SaveKey, &[src, fh]), STATUS_SUCCESS);
    t.close(fh);
    t.close(src);
    let bytes = vfs_bytes(&t, b"/nt/c/src.hiv");
    assert_eq!(&bytes[..4], b"regf");
    assert!(bytes.len() >= 8192 && bytes.len().is_multiple_of(4096));

    // Loaded: the saved key's content, not its name.
    assert_eq!(load(&mut t, "\\Registry\\Machine\\PolyLoaded", "\\??\\C:\\src.hiv"), STATUS_SUCCESS);
    assert!(t.exists(b"/nt/c/src.hiv.LOG1") && t.exists(b"/nt/c/src.hiv.LOG2"));
    let (s, hl) = key(&mut t, 0, "\\Registry\\Machine\\PolyLoaded", false);
    assert_eq!(s, STATUS_SUCCESS);
    assert_eq!(dword(&mut t, hl, "D"), Some(42));
    let (s, hs) = key(&mut t, hl, "Sub", false);
    assert_eq!(s, STATUS_SUCCESS);
    assert_eq!(dword(&mut t, hs, "X"), Some(5));
    t.close(hs);
    assert_eq!(key(&mut t, hl, "Src", false).0, STATUS_OBJECT_NAME_NOT_FOUND);
    set_dword(&mut t, hl, "N", 7);
    let (s, hn) = key(&mut t, hl, "NewSub", true);
    assert_eq!(s, STATUS_SUCCESS);
    t.close(hn);

    // Again under the same name: nothing changes; under another: the file is in use.
    assert_eq!(load(&mut t, "\\Registry\\Machine\\PolyLoaded", "\\??\\C:\\src.hiv"), STATUS_SUCCESS);
    assert_eq!(load(&mut t, "\\Registry\\Machine\\PolyLoaded2", "\\??\\C:\\src.hiv"), STATUS_SHARING_VIOLATION);
    assert_eq!(key(&mut t, 0, "\\Registry\\Machine\\PolyLoaded2", false).0, STATUS_OBJECT_NAME_NOT_FOUND);

    // An open key keeps it loaded.
    assert_eq!(unload(&mut t, "\\Registry\\Machine\\PolyLoaded"), STATUS_ACCESS_DENIED);
    t.close(hl);
    assert_eq!(unload(&mut t, "\\Registry\\User\\PolyLoaded"), STATUS_INVALID_PARAMETER);
    assert_eq!(unload(&mut t, "\\Registry\\Machine\\polyloaded\\"), STATUS_SUCCESS);
    assert_eq!(key(&mut t, 0, "\\Registry\\Machine\\PolyLoaded", false).0, STATUS_OBJECT_NAME_NOT_FOUND);
    assert_eq!(unload(&mut t, "\\Registry\\Machine\\PolyLoaded"), STATUS_INVALID_PARAMETER);
    assert_eq!(unload(&mut t, "\\Registry\\Machine\\SOFTWARE"), STATUS_INVALID_PARAMETER, "a system hive");

    // The changes are in the file: a reload below HKEY_USERS shows them.
    assert_eq!(load(&mut t, "\\Registry\\User\\PolyLoaded", "\\??\\C:\\src.hiv"), STATUS_SUCCESS);
    let (s, hu) = key(&mut t, 0, "\\Registry\\User\\PolyLoaded", false);
    assert_eq!(s, STATUS_SUCCESS);
    assert_eq!(dword(&mut t, hu, "N"), Some(7));
    let (s, hn) = key(&mut t, hu, "NewSub", false);
    assert_eq!(s, STATUS_SUCCESS);
    t.close(hn);
    t.close(hu);
    assert_eq!(unload(&mut t, "\\Registry\\Machine\\PolyLoaded"), STATUS_INVALID_PARAMETER);
    assert_eq!(unload(&mut t, "\\Registry\\User\\PolyLoaded"), STATUS_SUCCESS);
    // The file is closed again: it opens without sharing.
    let (s, h, _) = t.create("\\??\\C:\\src.hiv", RW, 0, FILE_OPEN, SYNC | FILE_NON_DIRECTORY_FILE, 0);
    assert_eq!(s, STATUS_SUCCESS);
    t.close(h);
}

#[test]
fn bad_targets_and_files() {
    let mut t = setup();
    // Only directly below \REGISTRY\MACHINE or \REGISTRY\USER.
    assert_eq!(load(&mut t, "\\Registry\\Machine\\SOFTWARE\\PolyX", "\\??\\C:\\a.hiv"), STATUS_ACCESS_DENIED);
    assert_eq!(load(&mut t, "\\Registry\\Machine\\SOFTWARE", "\\??\\C:\\a.hiv"), STATUS_OBJECT_NAME_COLLISION);
    // A file that is not a hive loads nothing.
    t.vfs_create(b"/nt/c/junk.hiv", b"this is not a registry hive file, only text");
    assert_eq!(load(&mut t, "\\Registry\\Machine\\PolyJunk", "\\??\\C:\\junk.hiv"), STATUS_REGISTRY_IO_FAILED);
    assert_eq!(key(&mut t, 0, "\\Registry\\Machine\\PolyJunk", false).0, STATUS_OBJECT_NAME_NOT_FOUND);
    // A directory that does not exist.
    assert_eq!(load(&mut t, "\\Registry\\Machine\\PolyNo", "\\??\\C:\\nodir\\x.hiv"), STATUS_OBJECT_PATH_NOT_FOUND);
    // A file another handle holds without sharing.
    let (s, h, _) = t.create("\\??\\C:\\held.hiv", RW, 0, FILE_CREATE, SYNC | FILE_NON_DIRECTORY_FILE, 0);
    assert_eq!(s, STATUS_SUCCESS);
    assert_eq!(load(&mut t, "\\Registry\\Machine\\PolyHeld", "\\??\\C:\\held.hiv"), STATUS_SHARING_VIOLATION);
    t.close(h);
    // A missing file becomes an empty hive.
    assert!(!t.exists(b"/nt/c/new.hiv"));
    assert_eq!(load(&mut t, "\\Registry\\Machine\\PolyNew", "\\??\\C:\\new.hiv"), STATUS_SUCCESS);
    assert_eq!(&vfs_bytes(&t, b"/nt/c/new.hiv")[..4], b"regf");
    let (s, h) = key(&mut t, 0, "\\Registry\\Machine\\PolyNew", false);
    assert_eq!(s, STATUS_SUCCESS);
    assert_eq!(dword(&mut t, h, "S"), None);
    t.close(h);
    assert_eq!(unload(&mut t, "\\Registry\\Machine\\PolyNew"), STATUS_SUCCESS);
    // A key handle of a deleted key does not save.
    let (_, k) = key(&mut t, 0, "\\Registry\\Machine\\SOFTWARE\\Gone", true);
    assert_eq!(t.f.call(Service::DeleteKey, &[k]), STATUS_SUCCESS);
    let fh = t.file("\\??\\C:\\gone.hiv");
    assert_eq!(t.f.call(Service::SaveKey, &[k, fh]), STATUS_KEY_DELETED);
}

//! Tests: directory enumeration.

use super::*;

/// Directory entries in a buffer: (name, attributes) of each.
fn entries(b: &[u8], class: u32) -> Vec<(String, u32)> {
    let mut out = Vec::new();
    let mut at = 0usize;
    loop {
        let next = u32::from_le_bytes(b[at..at + 4].try_into().unwrap()) as usize;
        let (len_at, name_at) = match class {
            FILE_NAMES_INFORMATION => (8, 12),
            _ => (60, crate::io::info::dir_fixed(class).unwrap()),
        };
        let len = u32::from_le_bytes(b[at + len_at..at + len_at + 4].try_into().unwrap()) as usize;
        let attrs = if class == FILE_NAMES_INFORMATION {
            0
        } else {
            u32::from_le_bytes(b[at + 56..at + 60].try_into().unwrap())
        };
        out.push((wide_str(&b[at + name_at..at + name_at + len]), attrs));
        if next == 0 {
            break;
        }
        assert_eq!(next % 8, 0);
        at += next;
    }
    out
}

impl T {
    fn list(
        &mut self,
        h: u64,
        class: u32,
        pattern: Option<&str>,
        single: bool,
        restart: bool,
        len: usize,
    ) -> (NtStatus, Vec<u8>) {
        let buf = self.f.alloc(len);
        let iosb = self.f.alloc(16);
        let name = pattern.map_or(0, |p| self.f.unicode(p));
        let s = self.f.call(
            Service::QueryDirectoryFile,
            &[h, 0, 0, 0, iosb, buf, len as u64, u64::from(class), u64::from(single), name, u64::from(restart)],
        );
        let n = self.f.u64_at(iosb + 8) as usize;
        (s, self.f.bytes_at(buf, n.min(len)))
    }
}

#[test]
fn directory_enumeration_with_wildcards() {
    let mut t = setup();
    let (s, d, _) = t.create(
        "\\??\\C:\\Docs",
        FILE_LIST_DIRECTORY | SYNCHRONIZE,
        SHARE_ALL,
        FILE_CREATE,
        FILE_DIRECTORY_FILE | SYNC,
        0,
    );
    assert_eq!(s, STATUS_SUCCESS);
    for name in ["a.txt", "B.TXT", "c.log", "d.txt.bak"] {
        let h = t.file(&alloc::format!("\\??\\C:\\Docs\\{name}"));
        t.close(h);
    }
    let (s, b) = t.list(d, FILE_DIRECTORY_INFORMATION, Some("*.txt"), false, false, 4096);
    assert_eq!(s, STATUS_SUCCESS);
    let mut names: Vec<String> = entries(&b, FILE_DIRECTORY_INFORMATION).into_iter().map(|e| e.0).collect();
    names.sort();
    assert_eq!(names, ["B.TXT", "a.txt"]);
    // The pattern stays; the scan is done.
    assert_eq!(t.list(d, FILE_DIRECTORY_INFORMATION, None, false, false, 4096).0, STATUS_NO_MORE_FILES);
    // Restart, one at a time, with a `?` pattern.
    let (s, b) = t.list(d, FILE_BOTH_DIRECTORY_INFORMATION, Some("?.log"), true, true, 4096);
    assert_eq!(s, STATUS_SUCCESS);
    assert_eq!(entries(&b, FILE_BOTH_DIRECTORY_INFORMATION), [(String::from("c.log"), FILE_ATTRIBUTE_ARCHIVE)]);
    assert_eq!(t.list(d, FILE_BOTH_DIRECTORY_INFORMATION, None, true, false, 4096).0, STATUS_NO_MORE_FILES);
    // Everything, `.` and `..` included below the root.
    let (s, b) = t.list(d, FILE_ID_BOTH_DIRECTORY_INFORMATION, Some("*"), false, true, 4096);
    assert_eq!(s, STATUS_SUCCESS);
    let all = entries(&b, FILE_ID_BOTH_DIRECTORY_INFORMATION);
    assert_eq!(all.len(), 6);
    assert!(all.iter().any(|e| e.0 == "." && e.1 == FILE_ATTRIBUTE_DIRECTORY));
    assert!(all.iter().any(|e| e.0 == ".."));
    // Nothing matches on a fresh scan.
    let (s, _) = t.list(d, FILE_NAMES_INFORMATION, Some("*.zip"), false, true, 4096);
    assert_eq!(s, STATUS_NO_SUCH_FILE);
    // A buffer for exactly one entry, then overflow for a too-small one.
    let (s, b) = t.list(d, FILE_NAMES_INFORMATION, Some("a.TXT"), false, true, 12 + 10);
    assert_eq!(s, STATUS_SUCCESS);
    assert_eq!(entries(&b, FILE_NAMES_INFORMATION)[0].0, "a.txt");
    let (s, b) = t.list(d, FILE_NAMES_INFORMATION, Some("a.txt"), false, true, 14);
    assert_eq!((s, b.len()), (STATUS_BUFFER_OVERFLOW, 14));
    // A name without wildcards lists every spelling of it, and only those;
    // `.` is still the directory's own entry.
    t.vfs_create(b"/nt/c/Docs/A.TXT", b"");
    let (s, b) = t.list(d, FILE_NAMES_INFORMATION, Some("a.txt"), false, true, 4096);
    assert_eq!(s, STATUS_SUCCESS);
    let names: Vec<String> = entries(&b, FILE_NAMES_INFORMATION).into_iter().map(|e| e.0).collect();
    assert_eq!(names, ["A.TXT", "a.txt"]);
    let ctx = t.vfs.root_context().unwrap();
    t.vfs.unlink(&ctx, &Cred::root(), At::Cwd, b"/nt/c/Docs/A.TXT").unwrap();
    let (s, b) = t.list(d, FILE_NAMES_INFORMATION, Some("."), false, true, 4096);
    assert_eq!((s, entries(&b, FILE_NAMES_INFORMATION)[0].0.as_str()), (STATUS_SUCCESS, "."));
    assert_eq!(t.list(d, FILE_NAMES_INFORMATION, Some("none.txt"), false, true, 4096).0, STATUS_NO_SUCH_FILE);
    assert_eq!(t.list(d, FILE_NAMES_INFORMATION, None, false, true, 4).0, STATUS_INFO_LENGTH_MISMATCH);
    assert_eq!(t.list(d, FILE_BASIC_INFORMATION, None, false, true, 4096).0, STATUS_INVALID_INFO_CLASS);
    t.close(d);
    // The root lists no `.` or `..`; a file handle cannot be listed.
    let (_, root, _) = t.create("\\??\\C:\\", FILE_LIST_DIRECTORY | SYNCHRONIZE, SHARE_ALL, FILE_OPEN, SYNC, 0);
    let (_, b) = t.list(root, FILE_NAMES_INFORMATION, None, false, false, 4096);
    let names: Vec<String> = entries(&b, FILE_NAMES_INFORMATION).into_iter().map(|e| e.0).collect();
    assert_eq!(names, ["Docs"]);
    // An open relative to a directory handle.
    let out = t.f.alloc(8);
    let iosb = t.f.alloc(16);
    let oa = t.f.attrs_in(root, "docs\\A.txt", CI);
    let s = t
        .f
        .call(Service::OpenFile, &[out, u64::from(FILE_GENERIC_READ), oa, iosb, u64::from(SHARE_ALL), u64::from(SYNC)]);
    assert_eq!(s, STATUS_SUCCESS);
    let rel = t.f.u64_at(out);
    let (_, b) = t.query(rel, FILE_NAME_INFORMATION, 100);
    assert_eq!(wide_str(&b[4..]), "\\Docs\\a.txt");
    assert_eq!(t.list(rel, FILE_NAMES_INFORMATION, None, false, false, 4096).0, STATUS_INVALID_PARAMETER);
    t.close(rel);
    t.close(root);
}

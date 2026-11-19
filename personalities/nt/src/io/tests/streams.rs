//! Tests: alternate data streams (ADR 0223).

use super::*;

/// The streams `FILE_STREAM_INFORMATION` lists for handle `h`: (name, size).
fn streams(t: &mut T, h: u64) -> Vec<(String, u64)> {
    let (s, b) = t.query(h, FILE_STREAM_INFORMATION, 1000);
    assert_eq!(s, STATUS_SUCCESS);
    let mut out = Vec::new();
    let mut at = 0;
    while at < b.len() {
        let next = u32::from_le_bytes(b[at..at + 4].try_into().unwrap()) as usize;
        let len = u32::from_le_bytes(b[at + 4..at + 8].try_into().unwrap()) as usize;
        let size = u64::from_le_bytes(b[at + 8..at + 16].try_into().unwrap());
        out.push((wide_str(&b[at + 24..at + 24 + len]), size));
        if next == 0 {
            break;
        }
        assert_eq!(next % 8, 0, "entries are 8-byte aligned");
        at += next;
    }
    out
}

/// The streams of the file `path`, through a handle opened for it alone.
fn streams_of(t: &mut T, path: &str) -> Vec<(String, u64)> {
    let (s, h, _) = t.create(path, FILE_READ_ATTRIBUTES | SYNCHRONIZE, SHARE_ALL, FILE_OPEN, SYNC, 0);
    assert_eq!(s, STATUS_SUCCESS, "{path}");
    let r = streams(t, h);
    t.close(h);
    r
}

fn put(t: &mut T, path: &str, data: &[u8]) {
    let (s, h, _) = t.create(path, RW, SHARE_ALL, FILE_OVERWRITE_IF, SYNC | FILE_NON_DIRECTORY_FILE, 0);
    assert_eq!(s, STATUS_SUCCESS, "{path}");
    t.write(h, data, None);
    t.close(h);
}

fn get(t: &mut T, path: &str) -> core::result::Result<Vec<u8>, NtStatus> {
    let (s, h, _) = t.create(path, FILE_GENERIC_READ, SHARE_ALL, FILE_OPEN, SYNC | FILE_NON_DIRECTORY_FILE, 0);
    if s != STATUS_SUCCESS {
        return Err(s);
    }
    let (_, b) = t.read(h, 200, Some(0));
    t.close(h);
    Ok(b)
}

fn entry(name: &str, size: u64) -> (String, u64) {
    (name.into(), size)
}

#[test]
fn a_named_stream_is_written_read_listed_and_kept_apart() {
    let mut t = setup();
    put(&mut t, "\\??\\C:\\main.bin", b"main data");
    put(&mut t, "\\??\\C:\\main.bin:Zone.Identifier", b"[ZoneTransfer]\r\nZoneId=3\r\n");
    assert_eq!(get(&mut t, "\\??\\C:\\main.bin").unwrap(), b"main data");
    assert_eq!(get(&mut t, "\\??\\C:\\main.bin:Zone.Identifier").unwrap(), b"[ZoneTransfer]\r\nZoneId=3\r\n");
    // Without case, with its type, and the default stream by its full name.
    assert_eq!(get(&mut t, "\\??\\C:\\MAIN.BIN:zone.identifier:$data").unwrap().len(), 26);
    assert_eq!(get(&mut t, "\\??\\C:\\main.bin::$DATA").unwrap(), b"main data");
    assert_eq!(get(&mut t, "\\??\\C:\\main.bin:nope"), Err(STATUS_OBJECT_NAME_NOT_FOUND));
    // A stream sorts by name without case; `<` is a character of a name.
    put(&mut t, "\\??\\C:\\main.bin:second", b"");
    put(&mut t, "\\??\\C:\\main.bin:a<b", b"x");
    assert_eq!(
        streams_of(&mut t, "\\??\\C:\\main.bin"),
        [entry("::$DATA", 9), entry(":a<b:$DATA", 1), entry(":second:$DATA", 0), entry(":Zone.Identifier:$DATA", 26)]
    );
    // A stream's own handle lists that stream.
    let (s, h, _) =
        t.create("\\??\\C:\\main.bin:second", FILE_READ_ATTRIBUTES | SYNCHRONIZE, SHARE_ALL, FILE_OPEN, SYNC, 0);
    assert_eq!(s, STATUS_SUCCESS);
    assert_eq!(streams(&mut t, h), [entry(":second:$DATA", 0)]);
    t.close(h);
    // A directory listing shows the file only.
    let (_, d, _) =
        t.create("\\??\\C:\\", FILE_LIST_DIRECTORY | SYNCHRONIZE, SHARE_ALL, FILE_OPEN, FILE_DIRECTORY_FILE | SYNC, 0);
    t.close(d);
    let ctx = t.vfs.root_context().unwrap();
    let names: Vec<Vec<u8>> = {
        let dir = t.vfs.lookup(&ctx, &Cred::root(), At::Cwd, b"/nt/c", true).unwrap();
        let f =
            t.vfs.open(&ctx, &Cred::root(), At::Node(&dir), b".", OpenFlags::RDONLY | OpenFlags::DIRECTORY, 0).unwrap();
        let mut v = Vec::new();
        f.read_dir(&mut |e| {
            v.push(e.name.to_vec());
            true
        })
        .unwrap();
        v
    };
    assert!(names.iter().all(|n| !n.starts_with(b".poly-streams")), "the store is not in the volume");
    // Deleting the stream leaves the file.
    let (s, h, _) = t.create("\\??\\C:\\main.bin:second", DELETE | SYNCHRONIZE, SHARE_ALL, FILE_OPEN, SYNC, 0);
    assert_eq!(s, STATUS_SUCCESS);
    assert_eq!(t.set(h, FILE_DISPOSITION_INFORMATION, &[1]), STATUS_SUCCESS);
    t.close(h);
    assert_eq!(
        streams_of(&mut t, "\\??\\C:\\main.bin"),
        [entry("::$DATA", 9), entry(":a<b:$DATA", 1), entry(":Zone.Identifier:$DATA", 26)]
    );
    assert_eq!(get(&mut t, "\\??\\C:\\main.bin").unwrap(), b"main data");
}

#[test]
fn a_stream_of_a_missing_file_makes_the_file_and_overwriting_drops_streams() {
    let mut t = setup();
    put(&mut t, "\\??\\C:\\new.bin:Zone.Identifier", b"");
    assert_eq!(get(&mut t, "\\??\\C:\\new.bin").unwrap(), b"");
    assert_eq!(streams_of(&mut t, "\\??\\C:\\new.bin"), [entry("::$DATA", 0), entry(":Zone.Identifier:$DATA", 0)]);
    // Opening a stream of a missing file to read: not found, nothing made.
    assert_eq!(get(&mut t, "\\??\\C:\\none.bin:s"), Err(STATUS_OBJECT_NAME_NOT_FOUND));
    assert!(!t.exists(b"/nt/c/none.bin"));
    // FILE_OVERWRITE_IF of the file itself: its streams go.
    put(&mut t, "\\??\\C:\\new.bin:s", b"abc");
    put(&mut t, "\\??\\C:\\new.bin", b"x");
    assert_eq!(streams_of(&mut t, "\\??\\C:\\new.bin"), [entry("::$DATA", 1)]);
    assert_eq!(get(&mut t, "\\??\\C:\\new.bin:s"), Err(STATUS_OBJECT_NAME_NOT_FOUND));
    // An empty stream name is no name.
    let (s, _, _) = t.create("\\??\\C:\\new.bin:", RW, SHARE_ALL, FILE_OPEN_IF, SYNC, 0);
    assert_eq!(s, STATUS_OBJECT_NAME_INVALID);
}

#[test]
fn rename_keeps_streams_replace_takes_the_sources_and_delete_drops_them() {
    let mut t = setup();
    put(&mut t, "\\??\\C:\\src.bin", b"data");
    put(&mut t, "\\??\\C:\\src.bin:Zone.Identifier", b"zone");
    let h = t.file("\\??\\C:\\src.bin");
    assert_eq!(t.set(h, FILE_RENAME_INFORMATION, &rename_info(false, 0, "\\??\\C:\\dst.bin")), STATUS_SUCCESS);
    t.close(h);
    assert_eq!(get(&mut t, "\\??\\C:\\dst.bin:Zone.Identifier").unwrap(), b"zone");
    assert_eq!(get(&mut t, "\\??\\C:\\src.bin:Zone.Identifier"), Err(STATUS_OBJECT_NAME_NOT_FOUND));
    // A file without streams replaces it: the result has none.
    put(&mut t, "\\??\\C:\\plain.bin", b"plain");
    let h = t.file("\\??\\C:\\plain.bin");
    assert_eq!(t.set(h, FILE_RENAME_INFORMATION, &rename_info(true, 0, "\\??\\C:\\dst.bin")), STATUS_SUCCESS);
    t.close(h);
    assert_eq!(streams_of(&mut t, "\\??\\C:\\dst.bin"), [entry("::$DATA", 5)]);
    // Deleting a file with a stream: a new file of the same name has none.
    put(&mut t, "\\??\\C:\\dst.bin:s", b"s");
    let h = t.file("\\??\\C:\\dst.bin");
    assert_eq!(t.set(h, FILE_DISPOSITION_INFORMATION, &[1]), STATUS_SUCCESS);
    t.close(h);
    put(&mut t, "\\??\\C:\\dst.bin", b"again");
    assert_eq!(streams_of(&mut t, "\\??\\C:\\dst.bin"), [entry("::$DATA", 5)]);
    // A stream itself cannot be renamed.
    let (_, h, _) = t.create("\\??\\C:\\dst.bin:s", RW | DELETE, SHARE_ALL, FILE_OPEN_IF, SYNC, 0);
    assert_eq!(t.set(h, FILE_RENAME_INFORMATION, &rename_info(false, 0, ":t")), STATUS_INVALID_PARAMETER);
    t.close(h);
}

#[test]
fn a_directory_has_named_streams_only() {
    let mut t = setup();
    let (s, d, _) = t.create(
        "\\??\\C:\\folder",
        FILE_LIST_DIRECTORY | SYNCHRONIZE,
        SHARE_ALL,
        FILE_CREATE,
        FILE_DIRECTORY_FILE | SYNC,
        0,
    );
    assert_eq!(s, STATUS_SUCCESS);
    t.close(d);
    put(&mut t, "\\??\\C:\\folder:info", b"");
    let (s, d, _) = t.create(
        "\\??\\C:\\folder",
        FILE_LIST_DIRECTORY | SYNCHRONIZE,
        SHARE_ALL,
        FILE_OPEN,
        FILE_DIRECTORY_FILE | SYNC,
        0,
    );
    assert_eq!(s, STATUS_SUCCESS);
    assert_eq!(streams(&mut t, d), [entry(":info:$DATA", 0)]);
    t.close(d);
    // A stream is no directory.
    let (s, _, _) = t.create("\\??\\C:\\folder:info", RW, SHARE_ALL, FILE_OPEN, FILE_DIRECTORY_FILE | SYNC, 0);
    assert_eq!(s, STATUS_NOT_A_DIRECTORY);
}

#[test]
fn the_volume_reports_named_streams() {
    let mut t = setup();
    let h = t.file("\\??\\C:\\v");
    let buf = t.f.alloc(64);
    let iosb = t.f.alloc(16);
    let s =
        t.f.call(Service::QueryVolumeInformationFile, &[h, iosb, buf, 64, u64::from(FILE_FS_ATTRIBUTE_INFORMATION)]);
    assert_eq!(s, STATUS_SUCCESS);
    assert_eq!(t.f.u32_at(buf) & FILE_NAMED_STREAMS, FILE_NAMED_STREAMS);
    t.close(h);
}

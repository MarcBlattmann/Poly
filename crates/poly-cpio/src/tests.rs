extern crate std;

use super::*;
use alloc::sync::Arc;
use alloc::vec;
use alloc::vec::Vec;
use poly_vfs::{At, Clock, Cred, DeviceId, Errno, OpenFlags, Stat, Timespec, Tmpfs, Vfs};

fn all(data: &[u8]) -> Result<Vec<Entry<'_>>, Error> {
    entries(data).collect()
}

fn names(data: &[u8]) -> Vec<Vec<u8>> {
    all(data).unwrap().iter().map(|e| e.name.to_vec()).collect()
}

fn sample(format: Format) -> Vec<u8> {
    let mut w = Writer::new(format);
    w.mtime = 1_700_000_000;
    w.dir(b".", 0o755);
    w.dir(b"etc", 0o755);
    w.file(b"etc/motd", 0o644, b"hello, world\n");
    w.file(b"etc/empty", 0o600, b"");
    w.symlink(b"etc/link", b"motd");
    w.node(b"null", FileType::CharDevice, 0o666, DeviceId::new(1, 3));
    w.hard_link(&[b"a", b"b", b"c"], 0o640, b"shared data");
    w.finish()
}

#[test]
fn round_trip_both_formats() {
    for format in [Format::Newc, Format::Crc] {
        let data = sample(format);
        assert_eq!(data.len() % 4, 0);
        let es = all(&data).unwrap();
        let got: Vec<&[u8]> = es.iter().map(|e| e.name).collect();
        assert_eq!(got, [&b"."[..], b"etc", b"etc/motd", b"etc/empty", b"etc/link", b"null", b"a", b"b", b"c"]);
        let motd = &es[2];
        assert_eq!(motd.format, format);
        assert_eq!(motd.data, b"hello, world\n");
        assert_eq!(motd.file_type(), Some(FileType::Regular));
        assert_eq!(motd.perm(), 0o644);
        assert_eq!(motd.header.mtime, 1_700_000_000);
        assert_eq!(motd.header.check, if format == Format::Crc { checksum(b"hello, world\n") } else { 0 });
        assert_eq!(es[4].file_type(), Some(FileType::Symlink));
        assert_eq!(es[4].data, b"motd");
        assert_eq!(es[5].rdev(), DeviceId::new(1, 3));
        assert_eq!((es[6].header.ino, es[6].header.nlink, es[6].data.len()), (es[8].header.ino, 3, 0));
        assert_eq!(es[8].data, b"shared data");
        // Every header starts 4-aligned.
        assert!(es.iter().all(|e| e.offset % 4 == 0));
    }
}

#[test]
fn padding_for_every_name_and_data_length() {
    for name_len in 1..=9 {
        for data_len in 0..=9 {
            let name = vec![b'n'; name_len];
            let body: Vec<u8> = (0..data_len as u8).collect();
            let mut w = Writer::new(Format::Crc);
            w.file(&name, 0o644, &body);
            w.file(b"next", 0o644, b"x");
            let data = w.finish();
            let es = all(&data).unwrap();
            assert_eq!(es.len(), 2, "name {name_len} data {data_len}");
            assert_eq!(es[0].name, &name[..]);
            assert_eq!(es[0].data, &body[..]);
            assert_eq!(es[1].data, b"x");
        }
    }
}

#[test]
fn concatenated_archives_and_padding() {
    let mut first = Writer::new(Format::Newc);
    first.file(b"one", 0o644, b"1");
    let mut data = first.finish();
    data.extend_from_slice(&[0u8; 512]);
    let mut second = Writer::new(Format::Crc);
    second.file(b"two", 0o644, b"2");
    data.extend_from_slice(&second.finish());
    data.extend_from_slice(&[0u8; 3]);
    assert_eq!(names(&data), [b"one".to_vec(), b"two".to_vec()]);
    // Empty input and pure padding hold nothing.
    assert_eq!(all(&[]).unwrap().len(), 0);
    assert_eq!(all(&[0u8; 64]).unwrap().len(), 0);
    // An archive of just a trailer is empty too.
    assert_eq!(all(&Writer::new(Format::Newc).finish()).unwrap().len(), 0);
}

#[test]
fn final_padding_may_be_missing() {
    let mut w = Writer::new(Format::Newc);
    w.file(b"f", 0o644, b"abc");
    let mut data = w.finish();
    // Cut the padding after the trailer's name (and the NUL, put back below).
    while data.last() == Some(&0) {
        data.pop();
    }
    data.push(0); // the trailer name's NUL
    assert_eq!(names(&data), [b"f".to_vec()]);
}

/// Replaces the 8 hex digits of header field `index` of the entry at `at`.
fn set_field(data: &mut [u8], at: usize, index: usize, value: &[u8; 8]) {
    let start = at + 6 + 8 * index;
    data[start..start + 8].copy_from_slice(value);
}

#[test]
fn malformed_headers() {
    let good = sample(Format::Newc);
    let second = all(&good).unwrap()[1].offset;

    let mut d = good.clone();
    d[second..second + 6].copy_from_slice(b"070707");
    let r = all(&d);
    assert_eq!(r, Err(Error::BadMagic { offset: second }));
    // Entries before the error are still yielded.
    let mut it = entries(&d);
    assert!(it.next().unwrap().is_ok());
    assert_eq!(it.next(), Some(Err(Error::BadMagic { offset: second })));
    assert_eq!(it.next(), None);

    let mut d = good.clone();
    set_field(&mut d, second, 6, b"0000000g");
    assert_eq!(all(&d), Err(Error::BadField { offset: second, field: "filesize" }));
    let mut d = good.clone();
    set_field(&mut d, second, 0, b"-0000001");
    assert_eq!(all(&d), Err(Error::BadField { offset: second, field: "ino" }));

    let mut d = good.clone();
    set_field(&mut d, second, 11, b"00000000");
    assert_eq!(all(&d), Err(Error::BadName { offset: second }));
    let mut d = good.clone();
    set_field(&mut d, second, 11, b"00000001");
    assert_eq!(all(&d), Err(Error::BadName { offset: second }));
    // A name whose last byte is not NUL ("etc" read as 3 bytes: "et" + 'c').
    let mut d = good.clone();
    set_field(&mut d, second, 11, b"00000003");
    assert_eq!(all(&d), Err(Error::BadName { offset: second }));

    // Sizes that run past the end.
    for size in [b"ffffffff", b"7fffffff", b"00010000"] {
        let mut d = good.clone();
        set_field(&mut d, second, 6, size);
        assert_eq!(all(&d), Err(Error::Truncated { offset: second }));
        let mut d = good.clone();
        set_field(&mut d, second, 11, size);
        assert_eq!(all(&d), Err(Error::Truncated { offset: second }));
    }

    // Garbage where an archive should start.
    assert_eq!(all(b"hello"), Err(Error::BadMagic { offset: 0 }));
    assert_eq!(all(b"0707"), Err(Error::Truncated { offset: 0 }));
    // Uppercase hex is fine.
    let mut d = good.clone();
    let motd = all(&good).unwrap()[2].offset;
    set_field(&mut d, motd, 0, b"0000ABCD");
    assert_eq!(all(&d).unwrap()[2].header.ino, 0xabcd);
}

#[test]
fn crc_checksums_are_verified() {
    let good = sample(Format::Crc);
    let motd = all(&good).unwrap()[2];
    let data_at = motd.data.as_ptr() as usize - good.as_ptr() as usize;
    let mut d = good.clone();
    d[data_at] ^= 1;
    let expected = motd.header.check;
    let actual = expected.wrapping_sub(good[data_at] as u32).wrapping_add(d[data_at] as u32);
    assert_eq!(all(&d), Err(Error::Checksum { offset: motd.offset, expected, actual }));
    // Only regular files are summed: GNU cpio leaves a symlink's check at 0.
    let link = all(&good).unwrap()[4];
    assert_eq!((link.file_type(), link.header.check), (Some(FileType::Symlink), 0));
    // In newc the field is ignored.
    let mut newc = sample(Format::Newc);
    let off = all(&newc).unwrap()[2].offset;
    set_field(&mut newc, off, 12, b"12345678");
    assert!(all(&newc).is_ok());
}

#[test]
fn missing_trailer() {
    let mut w = Writer::new(Format::Newc);
    w.file(b"f", 0o644, b"data");
    let full = w.finish();
    let end_of_f = all(&full).unwrap()[0].offset + 112 + 4; // header + "f\0" padded + data
    let cut = &full[..end_of_f];
    let mut it = entries(cut);
    assert!(it.next().unwrap().is_ok());
    assert_eq!(it.next(), Some(Err(Error::MissingTrailer { offset: end_of_f })));
    assert_eq!(it.next(), None);
}

/// Every prefix of an archive: no panic, and the entries that do come out
/// are exactly the first ones of the full archive.
#[test]
fn truncation_never_panics() {
    for format in [Format::Newc, Format::Crc] {
        let full = sample(format);
        let complete = all(&full).unwrap();
        let trailer_at = full.len() - 112 - 12; // header + "TRAILER!!!\0" padded to 4
        for len in 0..full.len() {
            let prefix = &full[..len];
            let mut got = Vec::new();
            let mut error = None;
            for r in entries(prefix) {
                match r {
                    Ok(e) => got.push(e),
                    Err(e) => error = Some(e),
                }
            }
            assert_eq!(&complete[..got.len()], &got[..], "prefix of {len}");
            if len == 0 {
                assert!(error.is_none());
            } else if len < trailer_at + 110 + 11 {
                assert!(error.is_some(), "a cut archive ({len} of {}) must be an error", full.len());
            }
        }
    }
}

/// Small deterministic generator for fuzz-style tests.
struct XorShift(u64);

impl XorShift {
    fn next(&mut self) -> u64 {
        self.0 ^= self.0 << 13;
        self.0 ^= self.0 >> 7;
        self.0 ^= self.0 << 17;
        self.0
    }
}

#[test]
fn mutations_never_panic() {
    let base = sample(Format::Crc);
    // Every byte, a few replacement values each.
    for i in 0..base.len() {
        for v in [0u8, b'0', b'f', b'g', 0xff, base[i] ^ 0x01] {
            let mut d = base.clone();
            d[i] = v;
            for r in entries(&d) {
                let _ = r;
            }
        }
    }
    // Random garbage, with and without a valid magic in front.
    let mut rng = XorShift(0x9e37_79b9_7f4a_7c15);
    for round in 0..2000 {
        let len = (rng.next() % 400) as usize;
        let mut d: Vec<u8> = (0..len).map(|_| rng.next() as u8).collect();
        if round % 2 == 0 && d.len() >= 6 {
            d[..6].copy_from_slice(NEWC_MAGIC);
            // Mostly hex digits so parsing gets further.
            for b in d.iter_mut().skip(6).take(104) {
                *b = b"0123456789abcdef"[(*b % 16) as usize];
            }
        }
        for r in entries(&d) {
            let _ = r;
        }
    }
}

#[derive(Debug)]
struct ZeroClock;

impl Clock for ZeroClock {
    fn now(&self) -> Timespec {
        Timespec::default()
    }
}

struct Root {
    vfs: Vfs,
    ctx: poly_vfs::FsContext,
    cred: Cred,
}

impl Root {
    fn new() -> Root {
        let vfs = Vfs::new(Arc::new(Tmpfs::new(Arc::new(ZeroClock))));
        let ctx = vfs.root_context().unwrap();
        Root { vfs, ctx, cred: Cred::root() }
    }

    fn unpack(&self, data: &[u8]) -> Result<UnpackStats, UnpackError> {
        unpack(data, &self.vfs, &self.ctx, At::Cwd)
    }

    fn lstat(&self, path: &str) -> Result<Stat, Errno> {
        self.vfs.stat(&self.ctx, &self.cred, At::Cwd, path.as_bytes(), false)
    }

    fn read(&self, path: &str) -> Vec<u8> {
        let f = self.vfs.open(&self.ctx, &self.cred, At::Cwd, path.as_bytes(), OpenFlags::RDONLY, 0).unwrap();
        let mut buf = vec![0u8; 1 << 16];
        let n = f.read(&mut buf).unwrap();
        buf.truncate(n);
        buf
    }
}

#[test]
fn unpack_makes_everything() {
    let root = Root::new();
    let stats = root.unpack(&sample(Format::Newc)).unwrap();
    assert_eq!(stats, UnpackStats { dirs: 1, files: 3, symlinks: 1, nodes: 1, hard_links: 2, bytes: 13 + 11 });
    assert_eq!(root.read("/etc/motd"), b"hello, world\n");
    assert_eq!(root.read("/etc/link"), b"hello, world\n");
    let motd = root.lstat("/etc/motd").unwrap();
    assert_eq!((motd.perm(), motd.mtime), (0o644, Timespec::new(1_700_000_000, 0)));
    assert_eq!(root.lstat("/etc/empty").unwrap().perm(), 0o600);
    assert_eq!(root.lstat("/etc/link").unwrap().kind(), Some(FileType::Symlink));
    assert_eq!(root.lstat("/etc").unwrap().mtime, Timespec::new(1_700_000_000, 0), "directory time set last");
    assert_eq!(root.lstat("/").unwrap().mtime, Timespec::new(1_700_000_000, 0));
    let null = root.lstat("/null").unwrap();
    assert_eq!((null.kind(), null.rdev, null.perm()), (Some(FileType::CharDevice), DeviceId::new(1, 3), 0o666));
    // The three hard links are one inode with the data.
    let (a, c) = (root.lstat("/a").unwrap(), root.lstat("/c").unwrap());
    assert_eq!((a.ino, a.nlink, a.size), (c.ino, 3, 11));
    assert_eq!(root.read("/b"), b"shared data");
}

#[test]
fn unpack_details() {
    let root = Root::new();
    let mut w = Writer::new(Format::Newc);
    // Missing parents, leading "/" and "./", set-user-ID, owners.
    let header = Header {
        ino: 7,
        mode: FileType::Regular.mode_bits() | 0o4755,
        uid: 1000,
        gid: 100,
        nlink: 1,
        mtime: 5,
        ..Header::default()
    };
    w.push(header, b"/deep/er/tool", b"#!");
    w.file(b"./deep/plain", 0o640, b"p");
    w.symlink(b"lib", b"deep/er");
    w.file(b"lib/viaLink", 0o644, b"through the link");
    let stats = root.unpack(&w.finish()).unwrap();
    assert_eq!(stats.files, 3);
    let tool = root.lstat("/deep/er/tool").unwrap();
    assert_eq!((tool.uid, tool.gid, tool.perm(), tool.mtime.sec), (1000, 100, 0o4755, 5));
    assert_eq!(root.lstat("/deep").unwrap().perm(), 0o755);
    assert_eq!(root.read("/deep/er/viaLink"), b"through the link");

    // A second archive replaces names, keeps directories, and fails on "..".
    let mut w = Writer::new(Format::Newc);
    w.dir(b"deep", 0o700);
    w.file(b"deep/plain", 0o600, b"replaced");
    w.dir(b"deep/er/tool", 0o755); // a file replaced by a directory
    let stats = root.unpack(&w.finish()).unwrap();
    assert_eq!(stats.dirs, 2);
    assert_eq!(root.read("/deep/plain"), b"replaced");
    assert_eq!(root.lstat("/deep").unwrap().perm(), 0o700);
    assert_eq!(root.lstat("/deep/er/tool").unwrap().kind(), Some(FileType::Directory));

    let mut w = Writer::new(Format::Newc);
    w.file(b"ok", 0o644, b"");
    w.file(b"a/../../escape", 0o644, b"");
    assert_eq!(
        root.unpack(&w.finish()),
        Err(UnpackError::Entry { name: b"a/../../escape".to_vec(), errno: Errno::EINVAL })
    );
    assert!(root.lstat("/ok").is_ok());
    assert_eq!(root.lstat("/escape"), Err(Errno::ENOENT));

    // Unknown file type.
    let mut w = Writer::new(Format::Newc);
    w.push(Header { mode: 0o170644, nlink: 1, ..Header::default() }, b"weird", b"");
    assert_eq!(root.unpack(&w.finish()), Err(UnpackError::Entry { name: b"weird".to_vec(), errno: Errno::EINVAL }));
    // A broken archive.
    assert_eq!(root.unpack(b"garbage!"), Err(UnpackError::Archive(Error::BadMagic { offset: 0 })));
    // A file where a parent directory should be.
    let mut w = Writer::new(Format::Newc);
    w.file(b"deep/plain/x", 0o644, b"");
    assert_eq!(
        root.unpack(&w.finish()),
        Err(UnpackError::Entry { name: b"deep/plain/x".to_vec(), errno: Errno::ENOTDIR })
    );
}

#[test]
fn unpack_into_a_subdirectory_and_into_a_bare_file_system() {
    let root = Root::new();
    root.vfs.mkdir(&root.ctx, &root.cred, At::Cwd, b"/initrd", 0o755).unwrap();
    let dir = root.vfs.lookup(&root.ctx, &root.cred, At::Cwd, b"/initrd", true).unwrap();
    unpack(&sample(Format::Crc), &root.vfs, &root.ctx, At::Node(&dir)).unwrap();
    assert_eq!(root.read("/initrd/etc/motd"), b"hello, world\n");

    let fs = Arc::new(Tmpfs::new(Arc::new(ZeroClock)));
    let stats = unpack_fs(&sample(Format::Newc), fs.clone()).unwrap();
    assert_eq!(stats.files, 3);
    // root + etc + motd + empty + link + null + one inode for a/b/c
    assert_eq!(fs.inode_count(), 7);
}

#[test]
fn unpacking_damaged_archives_never_panics() {
    let base = sample(Format::Newc);
    for len in (0..base.len()).step_by(3) {
        let root = Root::new();
        let _ = root.unpack(&base[..len]);
    }
    let mut rng = XorShift(12345);
    for _ in 0..300 {
        let mut d = base.clone();
        for _ in 0..3 {
            let i = (rng.next() as usize) % d.len();
            d[i] = rng.next() as u8;
        }
        let root = Root::new();
        let _ = root.unpack(&d);
    }
}

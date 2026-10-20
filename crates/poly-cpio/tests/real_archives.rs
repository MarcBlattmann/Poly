//! Archives made by a real cpio implementation.
//!
//! `data/gnu-newc.cpio` and `data/gnu-crc.cpio` were made with GNU cpio
//! 2.15 from this tree (all owned by root except `etc/motd`, 1000:100;
//! every time 1700000000):
//!
//! ```text
//! .  bin/  bin/hello (0755, 21 bytes)  dev/  dev/console (c 5 1, 0600)
//! dev/null (c 1 3, 0666)  etc/  etc/empty (0600, empty)  etc/motd (0644)
//! lib -> usr/lib  tmp/ (1777)  tmp/fifo (0644)  usr/  usr/lib/
//! usr/lib/libx.so and usr/lib/libx.so.1 (hard links, 5000 bytes of i % 251)
//! ```
//!
//! with `find . | LC_ALL=C sort | cpio -o -H newc --reproducible` (and
//! `-H crc`). GNU cpio pads the archive to 512-byte blocks and puts a hard
//! link's data on its last name.
//!
//! [`host_cpio_agrees`] also runs the host's `cpio`, if there is one, on a
//! fresh tree and checks the parse against the files on disk.

use poly_cpio::{Entry, Error, Format, UnpackStats, entries, unpack_fs};
use poly_vfs::{At, Clock, Cred, DeviceId, FileType, OpenFlags, Timespec, Tmpfs, Vfs};
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::Arc;

const NEWC: &[u8] = include_bytes!("data/gnu-newc.cpio");
const CRC: &[u8] = include_bytes!("data/gnu-crc.cpio");

fn libx() -> Vec<u8> {
    (0..5000u32).map(|i| (i % 251) as u8).collect()
}

fn listing(data: &[u8]) -> Vec<Entry<'_>> {
    entries(data).collect::<Result<Vec<_>, Error>>().unwrap()
}

#[test]
fn gnu_archives_parse() {
    for (data, format) in [(NEWC, Format::Newc), (CRC, Format::Crc)] {
        assert_eq!(data.len() % 512, 0);
        let es = listing(data);
        let names: Vec<&str> = es.iter().map(|e| std::str::from_utf8(e.name).unwrap()).collect();
        assert_eq!(
            names,
            [
                ".",
                "bin",
                "bin/hello",
                "dev",
                "dev/console",
                "dev/null",
                "etc",
                "etc/empty",
                "etc/motd",
                "lib",
                "tmp",
                "tmp/fifo",
                "usr",
                "usr/lib",
                "usr/lib/libx.so",
                "usr/lib/libx.so.1",
            ]
        );
        assert!(es.iter().all(|e| e.format == format));
        assert!(es.iter().all(|e| e.header.mtime == 1_700_000_000));
        let get = |n: &str| *es.iter().find(|e| e.name == n.as_bytes()).unwrap();
        assert_eq!(get("bin/hello").data, b"#!/bin/sh\necho hello\n");
        assert_eq!(get("bin/hello").perm(), 0o755);
        assert_eq!(get("etc/motd").data, b"Welcome to Poly\n");
        assert_eq!((get("etc/motd").header.uid, get("etc/motd").header.gid), (1000, 100));
        assert_eq!(get("etc/empty").perm(), 0o600);
        assert_eq!(get("lib").file_type(), Some(FileType::Symlink));
        assert_eq!(get("lib").data, b"usr/lib");
        assert_eq!(get("tmp").perm(), 0o1777);
        assert_eq!(get("tmp/fifo").file_type(), Some(FileType::Fifo));
        assert_eq!(get("dev/null").file_type(), Some(FileType::CharDevice));
        assert_eq!(get("dev/null").rdev(), DeviceId::new(1, 3));
        assert_eq!(get("dev/console").rdev(), DeviceId::new(5, 1));
        let (a, b) = (get("usr/lib/libx.so"), get("usr/lib/libx.so.1"));
        assert_eq!((a.header.ino, a.header.nlink), (b.header.ino, 2));
        assert!(a.data.is_empty());
        assert_eq!(b.data, &libx()[..]);
    }
}

#[derive(Debug)]
struct ZeroClock;

impl Clock for ZeroClock {
    fn now(&self) -> Timespec {
        Timespec::default()
    }
}

#[test]
fn gnu_archive_unpacks_into_tmpfs() {
    let fs = Arc::new(Tmpfs::new(Arc::new(ZeroClock)));
    let stats = unpack_fs(NEWC, fs.clone()).unwrap();
    assert_eq!(stats, UnpackStats { dirs: 6, files: 4, symlinks: 1, nodes: 3, hard_links: 1, bytes: 21 + 16 + 5000 });
    let vfs = Vfs::new(fs);
    let ctx = vfs.root_context().unwrap();
    let cred = Cred::root();
    let read = |p: &str| {
        let f = vfs.open(&ctx, &cred, At::Cwd, p.as_bytes(), OpenFlags::RDONLY, 0).unwrap();
        let mut out = vec![0u8; 10000];
        let n = f.read(&mut out).unwrap();
        out.truncate(n);
        out
    };
    assert_eq!(read("/lib/libx.so"), libx(), "through the symlink and the hard link");
    assert_eq!(read("/etc/motd"), b"Welcome to Poly\n");
    let st = |p: &str| vfs.stat(&ctx, &cred, At::Cwd, p.as_bytes(), false).unwrap();
    assert_eq!(st("/tmp").perm(), 0o1777);
    assert_eq!(st("/tmp").mtime, Timespec::new(1_700_000_000, 0));
    assert_eq!(st("/usr/lib/libx.so").nlink, 2);
    assert_eq!(st("/dev/console").rdev, DeviceId::new(5, 1));
    assert_eq!((st("/etc/motd").uid, st("/etc/motd").gid), (1000, 100));
    assert_eq!(st("/").perm(), 0o755);
    // The crc archive unpacks to the same thing.
    let fs2 = Arc::new(Tmpfs::new(Arc::new(ZeroClock)));
    assert_eq!(unpack_fs(CRC, fs2).unwrap(), stats);
}

#[test]
fn every_truncation_of_a_gnu_archive_is_handled() {
    for data in [NEWC, CRC] {
        let full = listing(data).len();
        for len in 0..data.len() {
            let n = entries(&data[..len]).filter(|r| r.is_ok()).count();
            assert!(n <= full);
            let fs = Arc::new(Tmpfs::new(Arc::new(ZeroClock)));
            let _ = unpack_fs(&data[..len], fs);
        }
    }
}

/// A scratch directory removed on drop.
struct TempDir(PathBuf);

impl Drop for TempDir {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

fn host_cpio() -> Option<&'static str> {
    ["cpio", "bsdcpio"]
        .into_iter()
        .find(|tool| Command::new(tool).arg("--version").stdout(Stdio::null()).stderr(Stdio::null()).status().is_ok())
}

fn walk(root: &Path, dir: &Path, out: &mut Vec<PathBuf>) {
    let mut children: Vec<_> = std::fs::read_dir(dir).unwrap().map(|e| e.unwrap().path()).collect();
    children.sort();
    for c in children {
        out.push(c.strip_prefix(root).unwrap().to_path_buf());
        if std::fs::symlink_metadata(&c).unwrap().is_dir() {
            walk(root, &c, out);
        }
    }
}

#[test]
fn host_cpio_agrees() {
    use std::os::unix::fs::{MetadataExt, PermissionsExt};
    let Some(tool) = host_cpio() else {
        eprintln!("no cpio or bsdcpio on this host; checked the stored archives only");
        return;
    };
    let dir = TempDir(std::env::temp_dir().join(format!("poly-cpio-test-{}", std::process::id())));
    let root = dir.0.join("tree");
    std::fs::create_dir_all(root.join("a/b/c")).unwrap();
    for (i, size) in [0usize, 1, 3, 4, 5, 511, 512, 4096, 70001].into_iter().enumerate() {
        let data: Vec<u8> = (0..size).map(|j| (j * 7 + i) as u8).collect();
        std::fs::write(root.join(format!("a/f{i}")), data).unwrap();
    }
    std::fs::write(root.join("a/b/c/deep"), b"deep").unwrap();
    std::fs::set_permissions(root.join("a/f1"), std::fs::Permissions::from_mode(0o751)).unwrap();
    std::os::unix::fs::symlink("../f2", root.join("a/b/up")).unwrap();
    std::fs::hard_link(root.join("a/f3"), root.join("a/b/f3-again")).unwrap();
    let mut paths = Vec::new();
    walk(&root, &root, &mut paths);

    for format in ["newc", "crc"] {
        let mut child = Command::new(tool)
            .args(["--quiet", "-o", "-H", format])
            .current_dir(&root)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .spawn()
            .unwrap();
        let list: String = paths.iter().map(|p| format!("{}\n", p.display())).collect();
        let mut stdin = child.stdin.take().unwrap();
        let writer = std::thread::spawn(move || {
            use std::io::Write;
            stdin.write_all(list.as_bytes()).unwrap();
        });
        let out = child.wait_with_output().unwrap();
        writer.join().unwrap();
        assert!(out.status.success(), "{tool} failed");
        let es = listing(&out.stdout);
        // GNU cpio moves multiply-linked files to the end, so compare by name.
        let mut got: Vec<&[u8]> = es.iter().map(|e| e.name).collect();
        got.sort();
        let mut want: Vec<&[u8]> = paths.iter().map(|p| p.to_str().unwrap().as_bytes()).collect();
        want.sort();
        assert_eq!(got, want);
        for e in &es {
            let p = Path::new(std::str::from_utf8(e.name).unwrap());
            let meta = std::fs::symlink_metadata(root.join(p)).unwrap();
            assert_eq!(e.header.mode, meta.mode(), "{}", p.display());
            assert_eq!(e.header.mtime as i64, meta.mtime());
            if meta.file_type().is_symlink() {
                assert_eq!(e.data, std::fs::read_link(root.join(p)).unwrap().to_str().unwrap().as_bytes());
            } else if meta.is_file() && !e.data.is_empty() {
                assert_eq!(e.data, &std::fs::read(root.join(p)).unwrap()[..]);
            }
        }
        // And it unpacks with every file's contents intact.
        let fs = Arc::new(Tmpfs::new(Arc::new(ZeroClock)));
        unpack_fs(&out.stdout, fs.clone()).unwrap();
        let vfs = Vfs::new(fs);
        let ctx = vfs.root_context().unwrap();
        let cred = Cred::root();
        for p in &paths {
            let meta = std::fs::symlink_metadata(root.join(p)).unwrap();
            if !meta.is_file() {
                continue;
            }
            let f = vfs.open(&ctx, &cred, At::Cwd, p.to_str().unwrap().as_bytes(), OpenFlags::RDONLY, 0).unwrap();
            let mut buf = vec![0u8; meta.len() as usize + 1];
            let mut n = 0;
            loop {
                let k = f.read(&mut buf[n..]).unwrap();
                if k == 0 {
                    break;
                }
                n += k;
            }
            assert_eq!(&buf[..n], &std::fs::read(root.join(p)).unwrap()[..], "{}", p.display());
        }
    }
}

//! Extended attributes on ext2 and ext4 against the host's e2fsprogs:
//! attributes written by `debugfs ea_set` read back; attributes the driver
//! writes (in the inode and in a block, set, replaced and removed) pass
//! `e2fsck -fn` and read back with `debugfs ea_get`; `format`'s volumes
//! keep attributes across a remount.

mod common;

use common::*;
use poly_ext2::{Ext2, FormatOptions, MountOptions};
use poly_vfs::xattr::SetMode;
use poly_vfs::{Errno, FileSystem, FileType, NewNode};

type Fs<'a> = Ext2<&'a MemDisk>;

fn mount(disk: &MemDisk) -> Fs<'_> {
    Ext2::mount(disk, Ticker::new(), MountOptions::default()).unwrap()
}

fn file(fs: &Fs<'_>, name: &[u8]) -> u64 {
    let node = NewNode { kind: FileType::Regular, perm: 0o644, uid: 0, gid: 0, rdev: Default::default() };
    fs.create(fs.root(), name, &node).unwrap()
}

const ANY: SetMode = SetMode { create: false, replace: false };

/// Sets, replaces and removes attributes of every size on a few files; the
/// expected end state is returned as (file, name, value).
fn workload(fs: &Fs<'_>) -> Vec<(&'static str, &'static str, Vec<u8>)> {
    let small = file(fs, b"small");
    let big = file(fs, b"big");
    let mixed = file(fs, b"mixed");
    let gone = file(fs, b"gone");
    let bs = fs.stats().block_size as usize;
    // Fits in a large inode.
    fs.setxattr(small, b"user.a", b"1", ANY).unwrap();
    fs.setxattr(small, b"security.selinux", b"label\0", ANY).unwrap();
    // Too big for any inode: a block.
    let big_value = pattern(1, bs / 2);
    fs.setxattr(big, b"user.big", &big_value, ANY).unwrap();
    fs.setxattr(big, b"trusted.t", b"", ANY).unwrap();
    // Several, one replaced, one removed.
    for i in 0..10u8 {
        let name = format!("user.attr-{i}");
        fs.setxattr(mixed, name.as_bytes(), &vec![i; 20 + 10 * i as usize], ANY).unwrap();
    }
    fs.setxattr(mixed, b"user.attr-3", b"replaced", SetMode { create: false, replace: true }).unwrap();
    assert_eq!(fs.setxattr(mixed, b"user.attr-3", b"x", SetMode { create: true, replace: false }), Err(Errno::EEXIST));
    assert_eq!(fs.setxattr(mixed, b"user.nope", b"x", SetMode { create: false, replace: true }), Err(Errno::ENODATA));
    fs.removexattr(mixed, b"user.attr-5").unwrap();
    assert_eq!(fs.removexattr(mixed, b"user.attr-5"), Err(Errno::ENODATA));
    // All removed again: the block goes.
    fs.setxattr(gone, b"user.big", &big_value, ANY).unwrap();
    fs.removexattr(gone, b"user.big").unwrap();
    // Too big for a block.
    assert_eq!(fs.setxattr(small, b"user.huge", &vec![0; bs], ANY), Err(Errno::ENOSPC));
    let mut want = vec![
        ("small", "user.a", b"1".to_vec()),
        ("small", "security.selinux", b"label\0".to_vec()),
        ("big", "user.big", big_value),
        ("big", "trusted.t", Vec::new()),
    ];
    for i in 0..10u8 {
        let value = match i {
            3 => b"replaced".to_vec(),
            5 => continue,
            _ => vec![i; 20 + 10 * i as usize],
        };
        want.push(("mixed", Box::leak(format!("user.attr-{i}").into_boxed_str()), value));
    }
    want
}

/// What the driver reads back for `want`.
fn check_back(fs: &Fs<'_>, want: &[(&str, &str, Vec<u8>)]) {
    for (f, name, value) in want {
        let ino = fs.lookup(fs.root(), f.as_bytes()).unwrap();
        assert_eq!(fs.getxattr(ino, name.as_bytes()).as_ref(), Ok(value), "{f} {name}");
        let list = fs.listxattr(ino).unwrap();
        assert!(list.split(|&b| b == 0).any(|n| n == name.as_bytes()), "{f}: {name} not listed");
    }
    let gone = fs.lookup(fs.root(), b"gone").unwrap();
    assert_eq!(fs.listxattr(gone), Ok(Vec::new()));
    assert_eq!(fs.getxattr(gone, b"user.big"), Err(Errno::ENODATA));
    assert_eq!(fs.getattr(gone).unwrap().blocks, 0, "the attribute block was released");
}

#[test]
fn written_attributes_pass_fsck_and_read_back_with_debugfs() {
    let Some(t) = tools() else {
        eprintln!("e2fsprogs not installed: skipping extended attributes against e2fsprogs");
        return;
    };
    let configs: &[(&str, &str, &[&str])] = &[
        ("ext4-256", "ext4", &["-b", "4096", "-I", "256"]),
        ("ext4-1k-nocsum", "ext4", &["-b", "1024", "-I", "256", "-O", "^metadata_csum"]),
        ("ext2-128", "ext2", &["-b", "1024", "-I", "128", "-O", "ext_attr,filetype,sparse_super"]),
        ("ext2-noea", "ext2", &["-b", "1024", "-I", "256", "-O", "^ext_attr"]),
    ];
    for (name, fs_type, args) in configs {
        let dir = TempDir::new(&format!("xattr-{name}"));
        let image = t.mkfs_type(&dir, 16 << 20, fs_type, args);
        let disk = MemDisk::new(image, 512);
        let fs = mount(&disk);
        let want = workload(&fs);
        check_back(&fs, &want);
        fs.unmount().unwrap();
        drop(fs);
        let mut image = disk.image();
        t.assert_clean(&dir, &image, &format!("{name}: writing attributes"));
        for (f, attr, value) in &want {
            let out = dir.path("value");
            let _ = std::fs::remove_file(&out);
            let log = t.debugfs(&dir, &mut image, &format!("ea_get -f {} /{f} {attr}\n", out.display()));
            let got = std::fs::read(&out).unwrap_or_else(|_| panic!("{name}: debugfs ea_get /{f} {attr}: {log}"));
            assert_eq!(&got, value, "{name}: debugfs reads /{f} {attr}");
        }
        // And mounted again.
        let disk = MemDisk::new(image, 512);
        let fs = mount(&disk);
        check_back(&fs, &want);
    }
}

#[test]
fn reads_attributes_debugfs_wrote() {
    let Some(t) = tools() else {
        eprintln!("e2fsprogs not installed: skipping attributes written by debugfs");
        return;
    };
    let dir = TempDir::new("xattr-debugfs");
    let mut image = t.mkfs_type(&dir, 16 << 20, "ext4", &["-b", "4096", "-I", "256"]);
    let src = dir.path("content");
    std::fs::write(&src, b"data").unwrap();
    let big = dir.path("big");
    std::fs::write(&big, pattern(7, 1500)).unwrap();
    let cmds = format!(
        "write {s} f\nea_set /f user.mime text/plain\nea_set /f security.label x\nea_set -f {b} /f user.big\n",
        s = src.display(),
        b = big.display()
    );
    let log = t.debugfs(&dir, &mut image, &cmds);
    assert!(!log.contains("rror"), "{log}");
    let disk = MemDisk::new(image, 512);
    let fs = mount(&disk);
    let f = fs.lookup(fs.root(), b"f").unwrap();
    assert_eq!(fs.getxattr(f, b"user.mime"), Ok(b"text/plain".to_vec()));
    assert_eq!(fs.getxattr(f, b"security.label"), Ok(b"x".to_vec()));
    assert_eq!(fs.getxattr(f, b"user.big"), Ok(pattern(7, 1500)));
    let mut names: Vec<Vec<u8>> =
        fs.listxattr(f).unwrap().split(|&b| b == 0).filter(|n| !n.is_empty()).map(<[u8]>::to_vec).collect();
    names.sort();
    assert_eq!(names, [b"security.label".to_vec(), b"user.big".to_vec(), b"user.mime".to_vec()]);
    // Changing one rewrites them all; e2fsck still agrees.
    fs.removexattr(f, b"user.mime").unwrap();
    fs.setxattr(f, b"user.new", b"v", ANY).unwrap();
    fs.unmount().unwrap();
    drop(fs);
    t.assert_clean(&dir, &disk.image(), "changing attributes debugfs wrote");
}

#[test]
fn format_keeps_attributes_across_a_remount() {
    for opts in [FormatOptions::ext4(1024, 0), FormatOptions { inode_size: 128, ..FormatOptions::ext4(4096, 1024) }] {
        let disk = MemDisk::new(vec![0; 16 << 20], 512);
        poly_ext2::format(&disk, &opts).unwrap();
        let fs = mount(&disk);
        let want = workload(&fs);
        check_back(&fs, &want);
        fs.unmount().unwrap();
        drop(fs);
        let fs = mount(&disk);
        check_back(&fs, &want);
        if let Some(t) = tools() {
            fs.unmount().unwrap();
            drop(fs);
            let dir = TempDir::new("xattr-format");
            t.assert_clean(&dir, &disk.image(), "attributes on a formatted volume");
        }
    }
}

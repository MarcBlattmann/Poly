//! ext4 against the host's e2fsprogs: images made by `mke2fs -t ext4` with
//! the host's default features (and variations) read back exactly;
//! everything the driver writes passes `e2fsck -fn` (which verifies every
//! checksum we write) and reads back with `debugfs`; we verify theirs;
//! extent trees of depth 1 and 2; hash-indexed directories; journals
//! written by debugfs replay as e2fsck replays them; and `format`'s ext4.
//!
//! Without e2fsprogs on the host, each test says so and checks what it can
//! with volumes from `poly_ext2::format` instead.

mod common;

use common::*;
use poly_ext2::{Ext2, FormatOptions, MountError, MountOptions};
use poly_vfs::{DeviceId, Errno, FileSystem, FileType, NewNode, RenameFlags, SetAttr, Tmpfs};
use std::os::unix::fs::FileExt;

type Fs<'a> = Ext2<&'a MemDisk>;

fn mount(disk: &MemDisk) -> Fs<'_> {
    Ext2::mount(disk, Ticker::new(), MountOptions::default()).unwrap()
}

const REG: NewNode = NewNode { kind: FileType::Regular, perm: 0o644, uid: 0, gid: 0, rdev: DeviceId::new(0, 0) };

/// mke2fs -t ext4 variations: the defaults, then with features off or on.
const CONFIGS: &[(&str, &[&str])] = &[
    ("default", &[]),
    ("1k", &["-b", "1024"]),
    ("no-journal", &["-O", "^has_journal"]),
    ("uninit_bg", &["-O", "^metadata_csum,uninit_bg"]),
    ("32bit-2k", &["-b", "2048", "-O", "^64bit"]),
    ("small-inodes", &["-I", "128"]),
    ("csum-seed-sparse2", &["-O", "metadata_csum_seed,sparse_super2,^resize_inode"]),
    ("no-flex", &["-O", "^flex_bg", "-g", "8192"]),
];

/// A host tree: files of many sizes, a directory with many entries, a
/// fragmented file (an extent tree of depth 1), sparse files, links.
fn host_tree(dir: &TempDir, bs: u64) -> std::path::PathBuf {
    let root = dir.path("tree");
    std::fs::create_dir_all(root.join("a/b/c")).unwrap();
    std::fs::create_dir_all(root.join("many")).unwrap();
    for (i, size) in [0u64, 1, 59, 60, 4095, 4096, 4097, 12 * bs + 1, 70_000, 1 << 20].iter().enumerate() {
        std::fs::write(root.join(format!("f{i}")), pattern(i as u64, *size as usize)).unwrap();
    }
    // Every other block: one extent each, more than fit in the inode.
    let frag = std::fs::File::create(root.join("a/fragmented")).unwrap();
    for i in 0..600u64 {
        frag.write_all_at(&pattern(i, 100), i * 2 * bs).unwrap();
    }
    let sparse = std::fs::File::create(root.join("a/b/sparse")).unwrap();
    sparse.write_all_at(b"start", 0).unwrap();
    sparse.write_all_at(&pattern(5, 3000), 300_000).unwrap();
    sparse.write_all_at(b"far", 70 << 20).unwrap();
    for i in 0..400 {
        std::fs::write(root.join(format!("many/entry-with-a-longish-name-{i:04}")), format!("{i}")).unwrap();
    }
    std::os::unix::fs::symlink("f1", root.join("short")).unwrap();
    std::os::unix::fs::symlink("y".repeat(200), root.join("slow")).unwrap();
    std::fs::hard_link(root.join("f3"), root.join("a/f3-again")).unwrap();
    root
}

fn features(t: &Tools, dir: &TempDir, image: &[u8]) -> String {
    let mut img = image.to_vec();
    t.debugfs(dir, &mut img, "features\n")
}

#[test]
fn reads_ext4_images_made_by_mke2fs() {
    let Some(t) = tools() else {
        eprintln!("e2fsprogs not installed: skipping images made by mke2fs");
        return;
    };
    for (name, args) in CONFIGS {
        let dir = TempDir::new(&format!("ext4-read-{name}"));
        let bs: u64 = args.windows(2).find(|w| w[0] == "-b").map_or(4096, |w| w[1].parse().unwrap());
        let root = host_tree(&dir, bs);
        let rootarg = root.display().to_string();
        let mut all = args.to_vec();
        all.extend(["-d", &rootarg]);
        let mut image = t.mkfs_type(&dir, 32 << 20, "ext4", &all);
        // Index the big directories, as e2fsck -D does.
        let (code, out) = t.fsck(&dir, &mut image, "-fyD");
        assert!(code <= 1, "{name}: e2fsck -fyD: {out}");
        t.assert_clean(&dir, &image, &format!("mke2fs {name}"));
        let mut img = image.clone();
        let stat = t.debugfs(&dir, &mut img, "stat /many\nex /a/fragmented\n");
        assert!(stat.contains("Flags: 0x81000"), "{name}: an indexed extent directory: {stat}");
        assert!(stat.contains(" 1/ 1 ") || stat.contains(" 2/ 2 "), "{name}: a deep extent tree: {stat}");
        let disk = MemDisk::new(image, 512);
        {
            let fs =
                Ext2::mount(&disk, Ticker::new(), MountOptions { read_only: true, ..MountOptions::default() }).unwrap();
            assert!(fs.stats().was_clean, "{name}");
            assert_eq!(fs.info().name, "ext4");
            let mut got = tree(&fs);
            got.remove("/lost+found");
            let want = common::host_tree(&root);
            assert_eq!(got.len(), want.len(), "{name}: node count");
            for (path, node) in &want {
                assert_eq!(got.get(path), Some(node), "{name}: {path}");
            }
            let sparse = fs.getattr(resolve(&fs, "a/b/sparse").unwrap()).unwrap();
            assert_eq!(sparse.size, (70 << 20) + 3);
            assert!(sparse.blocks < 64, "{name}: holes take no blocks ({} sectors)", sparse.blocks);
            assert_eq!(fs.getattr(resolve(&fs, "a/f3-again").unwrap()).unwrap().nlink, 2);
            assert!(!fs.stats().errors, "{name}");
        }
        // Mounting read-write and unmounting changes nothing e2fsck minds.
        let fs = mount(&disk);
        assert!(!fs.read_only(), "{name}");
        fs.unmount().unwrap();
        drop(fs);
        t.assert_clean(&dir, &disk.image(), &format!("mount/unmount {name}"));
        let _ = features(&t, &dir, &disk.image());
    }
}

/// Every operation of the trait, done on ext4 and on tmpfs alike.
fn workload(fs: &dyn FileSystem, bs: u64) {
    let root = fs.root();
    let node = |kind, perm, rdev| NewNode { kind, perm, uid: 1000, gid: 100, rdev };
    let d1 = fs.mkdir(root, b"dir", 0o755, 0, 0).unwrap();
    let d2 = fs.mkdir(d1, b"sub", 0o700, 1000, 1000).unwrap();
    fs.mkdir(d2, b"deeper", 0o750, 0, 0).unwrap();
    for (i, size) in [0usize, 1, 100, 4095, 4096, 4097, 50_000, 300_000].into_iter().enumerate() {
        let f = fs.create(d1, format!("file{i}").as_bytes(), &REG).unwrap();
        let data = pattern(1000 + i as u64, size);
        let mut at = 0;
        while at < size {
            let n = (size - at).min(7777);
            assert_eq!(fs.write(f, at as u64, &data[at..at + n]).unwrap(), n);
            at += n;
        }
    }
    // Fragmented on purpose: every other block, backwards, so extents
    // cannot merge and the tree splits (depth 2 with 1 KiB blocks).
    let frag = fs.create(root, b"fragmented", &REG).unwrap();
    for i in (0..500u64).rev() {
        fs.write(frag, i * 2 * bs + 7, &pattern(i, 50)).unwrap();
    }
    // Then fill some holes: extents merge again.
    for i in 0..100u64 {
        fs.write(frag, (2 * i + 1) * bs, &pattern(i + 9000, bs as usize)).unwrap();
    }
    let big = fs.create(root, b"big", &REG).unwrap();
    fs.write(big, 0, &pattern(7, 3 << 20)).unwrap();
    let sparse = fs.create(root, b"sparse", &REG).unwrap();
    fs.write(sparse, 5, b"hello").unwrap();
    fs.write(sparse, 60 << 20, b"far away").unwrap();
    fs.write(sparse, 3 << 20, &pattern(8, 10_000)).unwrap();
    // Truncation: into the middle of the fragmented file, of the big one.
    fs.truncate(frag, 300 * bs + 3).unwrap();
    fs.truncate(big, 1_000_001).unwrap();
    fs.truncate(big, 2_000_000).unwrap();
    fs.write(big, 2_100_000, b"tail").unwrap();
    let t2 = fs.create(root, b"trunc-to-zero", &REG).unwrap();
    fs.write(t2, 0, &pattern(10, 90_000)).unwrap();
    fs.truncate(t2, 0).unwrap();
    fs.symlink(root, b"fast", b"dir/file1", 0, 0).unwrap();
    fs.symlink(root, b"slow", "../".repeat(40).as_bytes(), 0, 0).unwrap();
    fs.create(root, b"null", &node(FileType::CharDevice, 0o666, DeviceId::new(1, 3))).unwrap();
    fs.create(root, b"fifo", &node(FileType::Fifo, 0o600, DeviceId::default())).unwrap();
    let f0 = fs.lookup(d1, b"file2").unwrap();
    fs.link(d2, b"file2-link", f0).unwrap();
    fs.rename(d1, b"file3", root, b"moved", RenameFlags::default()).unwrap();
    fs.rename(d1, b"file4", d1, b"file5", RenameFlags::default()).unwrap();
    fs.rename(root, b"dir", root, b"renamed-dir", RenameFlags::default()).unwrap();
    fs.rename(d1, b"sub", root, b"sub-moved", RenameFlags::default()).unwrap();
    let many = fs.mkdir(root, b"many", 0o755, 0, 0).unwrap();
    for i in 0..300 {
        fs.create(many, format!("a-rather-long-file-name-{i:05}").as_bytes(), &REG).unwrap();
    }
    for i in (0..300).step_by(3) {
        fs.unlink(many, format!("a-rather-long-file-name-{i:05}").as_bytes()).unwrap();
    }
    let gone = fs.mkdir(root, b"gone", 0o755, 0, 0).unwrap();
    fs.create(gone, b"x", &REG).unwrap();
    fs.unlink(gone, b"x").unwrap();
    fs.rmdir(root, b"gone").unwrap();
    fs.unlink(d1, b"file0").unwrap();
    // Open, unlinked, written, released.
    let open = fs.create(root, b"open", &REG).unwrap();
    fs.write(open, 0, &pattern(11, 30_000)).unwrap();
    fs.open(open).unwrap();
    fs.unlink(root, b"open").unwrap();
    assert_eq!(read_all(fs, open).unwrap(), pattern(11, 30_000));
    fs.write(open, 30_000, b"more").unwrap();
    fs.release(open);
    assert_eq!(fs.getattr(open).map(|_| ()), Err(Errno::ENOENT));
    let f1 = fs.lookup(d1, b"file1").unwrap();
    fs.setattr(f1, &SetAttr { perm: Some(0o4711), uid: Some(123_456), gid: Some(7), ..SetAttr::default() }).unwrap();
}

/// Volumes to write to: mke2fs configurations and `format`'s ext4.
fn write_volumes(t: &Option<Tools>, dir: &TempDir) -> Vec<(String, Vec<u8>, u64)> {
    let mut out = Vec::new();
    for (name, bs, journal) in
        [("format-4k", 4096u32, 1024u32), ("format-1k", 1024, 1024), ("format-nojournal", 4096, 0)]
    {
        let disk = MemDisk::new(vec![0; 64 << 20], 512);
        poly_ext2::format(&disk, &FormatOptions::ext4(bs, journal)).unwrap();
        out.push((name.to_string(), disk.image(), bs as u64));
    }
    if let Some(t) = t {
        for (name, args) in CONFIGS {
            let bs: u64 = args.windows(2).find(|w| w[0] == "-b").map_or(4096, |w| w[1].parse().unwrap());
            out.push((format!("mke2fs-{name}"), t.mkfs_type(dir, 32 << 20, "ext4", args), bs));
        }
    }
    out
}

#[test]
fn writes_pass_fsck_and_read_back() {
    let t = tools();
    if t.is_none() {
        eprintln!("e2fsprogs not installed: checking format() volumes against tmpfs only");
    }
    let dir = TempDir::new("ext4-write");
    for (name, image, bs) in write_volumes(&t, &dir) {
        if let Some(t) = &t {
            t.assert_clean(&dir, &image, &format!("{name} as made"));
        }
        let disk = MemDisk::new(image, 512);
        let model = Tmpfs::new(Ticker::new());
        workload(&model, bs);
        let want = tree(&model);
        {
            let fs = mount(&disk);
            workload(&fs, bs);
            let mut got = tree(&fs);
            got.remove("/lost+found");
            assert_eq!(got, want, "{name}: driver view before unmount");
            assert!(!fs.stats().errors, "{name}");
            fs.unmount().unwrap();
        }
        {
            let fs = mount(&disk);
            let mut got = tree(&fs);
            got.remove("/lost+found");
            assert_eq!(got, want, "{name}: after remount");
            assert_eq!(fs.getattr(fs.root()).unwrap().nlink, 6, "{name}: root links");
            fs.unmount().unwrap();
        }
        let Some(t) = &t else { continue };
        t.assert_clean(&dir, &disk.image(), &format!("the workload on {name}"));
        let mut img = disk.image();
        let out = t.debugfs(&dir, &mut img, "cat /big\n");
        assert!(out.len() > 2_000_000, "{name}: debugfs reads /big");
        let ex = t.debugfs(&dir, &mut img, "ex /fragmented\nstat /many\n");
        assert!(ex.contains(" 1/ 1 ") || ex.contains(" 2/ 2 "), "{name}: a deep tree: {ex}");
        let out_dir = dir.path("rdump");
        t.rdump(&dir, &disk.image(), &out_dir);
        let dumped = common::host_tree(&out_dir);
        for (path, node) in &want {
            if let Node::File { data, .. } = node {
                match dumped.get(path) {
                    Some(Node::File { data: d, .. }) => assert!(d == data, "{name}: rdump {path}"),
                    other => panic!("{name}: rdump {path}: {other:?}"),
                }
            }
        }
    }
}

#[test]
fn format_ext4_passes_fsck() {
    let Some(t) = tools() else {
        eprintln!("e2fsprogs not installed: skipping the e2fsck check of format()");
        return;
    };
    let dir = TempDir::new("ext4-format");
    for (size, opts) in [
        (8 << 20, FormatOptions::ext4(1024, 1024)),
        (16 << 20, FormatOptions::ext4(4096, 1024)),
        (40 << 20, FormatOptions { blocks_per_group: Some(2048), ..FormatOptions::ext4(1024, 0) }),
        (140 << 20, FormatOptions { inode_size: 128, ..FormatOptions::ext4(4096, 4096) }),
    ] {
        let disk = MemDisk::new(vec![0; size], 512);
        poly_ext2::format(&disk, &opts).unwrap();
        t.assert_clean(&dir, &disk.image(), &format!("format {opts:?}"));
        let fs = mount(&disk);
        assert_eq!(fs.info().name, "ext4");
        fs.mkdir(fs.root(), b"x", 0o755, 0, 0).unwrap();
        let f = fs.create(fs.root(), b"f", &REG).unwrap();
        fs.write(f, 0, &pattern(3, 100_000)).unwrap();
        fs.unmount().unwrap();
        t.assert_clean(&dir, &disk.image(), &format!("writing on format {opts:?}"));
    }
}

#[test]
fn checksums_are_verified() {
    let disk = MemDisk::new(vec![0; 16 << 20], 512);
    poly_ext2::format(&disk, &FormatOptions::ext4(4096, 1024)).unwrap();
    let (f, d) = {
        let fs = mount(&disk);
        let d = fs.mkdir(fs.root(), b"d", 0o755, 0, 0).unwrap();
        let f = fs.create(d, b"the-checked-file", &REG).unwrap();
        fs.write(f, 0, b"checked").unwrap();
        fs.unmount().unwrap();
        (f, d)
    };
    let clean = disk.image();
    // The superblock.
    let mut bad = clean.clone();
    bad[1024 + 200] ^= 1;
    assert_eq!(
        Ext2::mount(&MemDisk::new(bad, 512), Ticker::new(), MountOptions::default()).err(),
        Some(MountError::Corrupt("superblock checksum"))
    );
    // A group descriptor.
    let mut bad = clean.clone();
    bad[4096 + 0x10] ^= 1;
    assert_eq!(
        Ext2::mount(&MemDisk::new(bad, 512), Ticker::new(), MountOptions::default()).err(),
        Some(MountError::Corrupt("group descriptor checksum"))
    );
    // An inode (its mtime): EBADMSG, the volume marked as having errors.
    let mut bad = clean.clone();
    let inode_at = find_inode(&clean, f as u32);
    bad[inode_at + 16] ^= 1;
    let bad_disk = MemDisk::new(bad, 512);
    let fs = Ext2::mount(&bad_disk, Ticker::new(), MountOptions::default()).unwrap();
    assert_eq!(fs.getattr(f).map(|_| ()), Err(Errno::EBADMSG));
    assert!(fs.stats().errors);
    // A directory block.
    let mut bad = clean.clone();
    let dir_block = find_dir_block(&clean, b"the-checked-file");
    bad[dir_block + 20] ^= 1;
    let bad_disk = MemDisk::new(bad, 512);
    let fs = Ext2::mount(&bad_disk, Ticker::new(), MountOptions::default()).unwrap();
    assert_eq!(fs.lookup(d, b"the-checked-file"), Err(Errno::EBADMSG));
}

/// Byte offset of inode `ino` in a `format` ext4 image (4 KiB blocks, one
/// group: the table at the group descriptor's address).
fn find_inode(image: &[u8], ino: u32) -> usize {
    let gd = 4096;
    let table = u32::from_le_bytes(image[gd + 8..gd + 12].try_into().unwrap()) as usize;
    table * 4096 + (ino as usize - 1) * 256
}

/// Byte offset of the directory block holding `name`.
fn find_dir_block(image: &[u8], name: &[u8]) -> usize {
    let mut needle = vec![name.len() as u8, 1];
    needle.extend_from_slice(name);
    // The last copy: the journal's log (before it) has older ones.
    let at = image.windows(needle.len()).rposition(|w| w == needle.as_slice()).unwrap();
    at / 4096 * 4096
}

#[test]
fn hash_indexed_directories_lose_their_index_cleanly() {
    let Some(t) = tools() else {
        eprintln!("e2fsprogs not installed: skipping hash-indexed directories");
        return;
    };
    for bs in ["1024", "4096"] {
        let dir = TempDir::new(&format!("ext4-htree-{bs}"));
        let root = dir.path("tree");
        std::fs::create_dir_all(root.join("big")).unwrap();
        let n = if bs == "1024" { 8000 } else { 3000 };
        for i in 0..n {
            std::fs::write(root.join(format!("big/name-{i:05}")), format!("{i}")).unwrap();
        }
        let rootarg = root.display().to_string();
        let mut image = t.mkfs_type(&dir, 32 << 20, "ext4", &["-b", bs, "-d", &rootarg]);
        let (code, out) = t.fsck(&dir, &mut image, "-fyD");
        assert!(code <= 1, "e2fsck -fyD: {out}");
        let mut img = image.clone();
        let stat = t.debugfs(&dir, &mut img, "htree /big\n");
        if bs == "1024" {
            assert!(stat.contains("Indirect levels: 1"), "two levels of index: {stat}");
        }
        let disk = MemDisk::new(image, 512);
        {
            let fs = mount(&disk);
            let big = resolve(&fs, "big").unwrap();
            assert_eq!(list(&fs, big).unwrap().len(), n);
            for i in (0..n).step_by(97) {
                let f = fs.lookup(big, format!("name-{i:05}").as_bytes()).unwrap();
                assert_eq!(read_all(&fs, f).unwrap(), format!("{i}").as_bytes());
            }
            fs.create(big, b"added", &REG).unwrap();
            fs.unlink(big, b"name-00100").unwrap();
            assert_eq!(list(&fs, big).unwrap().len(), n);
            fs.unmount().unwrap();
        }
        let dir2 = TempDir::new(&format!("ext4-htree-check-{bs}"));
        t.assert_clean(&dir2, &disk.image(), "changing a hash-indexed directory");
        let mut img = disk.image();
        let stat = t.debugfs(&dir2, &mut img, "stat /big\n");
        assert!(stat.contains("Flags: 0x80000"), "the index flag is gone: {stat}");
    }
}

#[test]
fn inline_data_reads() {
    let Some(t) = tools() else {
        eprintln!("e2fsprogs not installed: skipping inline data");
        return;
    };
    let dir = TempDir::new("ext4-inline");
    let root = dir.path("tree");
    std::fs::create_dir_all(root.join("small-dir")).unwrap();
    std::fs::write(root.join("tiny"), b"tiny file").unwrap();
    std::fs::write(root.join("in-xattr"), pattern(1, 100)).unwrap();
    std::fs::write(root.join("small-dir/a"), b"a").unwrap();
    std::fs::write(root.join("small-dir/b"), b"bb").unwrap();
    std::fs::write(root.join("big"), pattern(2, 10_000)).unwrap();
    let rootarg = root.display().to_string();
    let image = t.mkfs_type(&dir, 16 << 20, "ext4", &["-O", "inline_data", "-d", &rootarg]);
    t.assert_clean(&dir, &image, "inline data image");
    let mut img = image.clone();
    let stat = t.debugfs(&dir, &mut img, "stat /tiny\nstat /small-dir\n");
    assert!(stat.contains("Flags: 0x10000000"), "inline inodes: {stat}");
    let disk = MemDisk::new(image, 512);
    let fs = mount(&disk);
    assert!(fs.read_only(), "inline_data is read-only");
    let mut got = tree(&fs);
    got.remove("/lost+found");
    assert_eq!(got, common::host_tree(&root));
    assert_eq!(fs.lookup(resolve(&fs, "small-dir").unwrap(), b"..").unwrap(), fs.root());
}

#[test]
fn unsupported_features_refuse_or_mount_read_only() {
    let Some(t) = tools() else {
        eprintln!("e2fsprogs not installed: skipping feature refusal");
        return;
    };
    let dir = TempDir::new("ext4-features");
    for (args, want) in [
        (&["-O", "bigalloc", "-C", "16384"][..], Err(MountError::Unsupported("bigalloc"))),
        (&["-O", "meta_bg,^resize_inode"][..], Err(MountError::IncompatFeatures(0x10))),
        (&["-O", "encrypt"][..], Err(MountError::IncompatFeatures(0x10000))),
        (&["-O", "quota"][..], Ok(true)),
        (&["-O", "project,quota"][..], Ok(true)),
    ] {
        let image = t.mkfs_type(&dir, 32 << 20, "ext4", args);
        let disk = MemDisk::new(image, 512);
        let got = Ext2::mount(&disk, Ticker::new(), MountOptions::default()).map(|fs| fs.read_only());
        assert_eq!(got, want, "{args:?}");
    }
}

#[test]
fn large_and_huge_files() {
    let disk = MemDisk::new(vec![0; 16 << 20], 512);
    poly_ext2::format(&disk, &FormatOptions::ext4(1024, 1024)).unwrap();
    let fs = mount(&disk);
    let f = fs.create(fs.root(), b"huge", &REG).unwrap();
    // 1 KiB blocks: logical blocks are 32 bits, so 4 TiB minus a block.
    let max = ((1u64 << 32) - 1) * 1024;
    assert_eq!(fs.write(f, max - 3, b"end"), Ok(3));
    assert_eq!(fs.write(f, max - 1, b"xy"), Err(Errno::EFBIG));
    assert_eq!(fs.write(f, max - 1, b"x"), Ok(1));
    assert_eq!(fs.write(f, max, b"x"), Err(Errno::EFBIG));
    fs.write(f, 0, b"start").unwrap();
    let mut buf = [0u8; 3];
    fs.read(f, max - 3, &mut buf).unwrap();
    assert_eq!(&buf, b"enx");
    assert_eq!(fs.getattr(f).unwrap().size, max);
    fs.truncate(f, 1 << 40).unwrap();
    fs.unmount().unwrap();
    if let Some(t) = tools() {
        let dir = TempDir::new("ext4-huge");
        t.assert_clean(&dir, &disk.image(), "a file reaching the last logical block");
    }
}

#[test]
fn deep_extent_trees_split_merge_and_shrink() {
    let t = tools();
    let dir = TempDir::new("ext4-deep");
    let disk = MemDisk::new(vec![0; 32 << 20], 512);
    poly_ext2::format(&disk, &FormatOptions::ext4(1024, 1024)).unwrap();
    let check = |t: &Tools, what: &str| -> String {
        let mut img = disk.image();
        let out = t.debugfs(&dir, &mut img, "ex /f\n");
        t.assert_clean(&dir, &img, what);
        out
    };
    let fs = mount(&disk);
    let f = fs.create(fs.root(), b"f", &REG).unwrap();
    let mut model = vec![0u8; 0];
    let put = |fs: &Fs<'_>, model: &mut Vec<u8>, at: usize, data: &[u8]| {
        fs.write(f, at as u64, data).unwrap();
        if model.len() < at + data.len() {
            model.resize(at + data.len(), 0);
        }
        model[at..at + data.len()].copy_from_slice(data);
    };
    // 2000 one-block extents, in an order that splits leaves in the middle.
    let mut order: Vec<u64> = (0..2000).collect();
    let mut rng = Rng::new(5);
    for i in (1..order.len()).rev() {
        order.swap(i, rng.below(i as u64 + 1) as usize);
    }
    for &i in &order {
        put(&fs, &mut model, (i * 2048) as usize, &pattern(i, 1024));
    }
    fs.sync().unwrap();
    assert_eq!(read_all(&fs, f).unwrap(), model);
    if let Some(t) = &t {
        let out = check(t, "2000 extents");
        assert!(out.contains(" 2/ 2 "), "depth 2: {}", &out[..out.len().min(400)]);
    }
    // Fill every hole (new blocks merge with their neighbours where they
    // happen to be adjacent).
    for i in 0..2000u64 {
        put(&fs, &mut model, (i * 2048 + 1024) as usize, &pattern(i + 5000, 1024));
    }
    fs.sync().unwrap();
    assert_eq!(read_all(&fs, f).unwrap(), model);
    // Cut down in steps; the tree shrinks back into the inode.
    for size in [3_000_000usize, 1_000_000, 100_000, 3000, 0] {
        fs.truncate(f, size as u64).unwrap();
        model.truncate(size);
        fs.sync().unwrap();
        assert_eq!(read_all(&fs, f).unwrap(), model, "truncated to {size}");
        if let Some(t) = &t {
            let out = check(t, &format!("truncating to {size}"));
            if size == 3000 {
                assert!(out.contains(" 0/ 0 "), "back in the inode at {size}: {out}");
            }
        }
    }
    let a = fs.getattr(f).unwrap();
    assert_eq!((a.size, a.blocks), (0, 0));
    fs.unmount().unwrap();
}

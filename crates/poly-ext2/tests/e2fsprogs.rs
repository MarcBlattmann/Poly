//! The driver against the host's e2fsprogs: images made by `mke2fs` (from a
//! host directory with `-d`, plus special files from `debugfs -w`) read back
//! exactly, and everything the driver writes passes `e2fsck -fn` and reads
//! back with `debugfs rdump`.
//!
//! Without e2fsprogs on the host, each test says so and checks what it can
//! with volumes from `poly_ext2::format` instead.

mod common;

use common::*;
use poly_ext2::{Ext2, FormatOptions, MountOptions};
use poly_vfs::{DeviceId, Errno, FileSystem, FileType, NewNode, RenameFlags, SetAttr, Tmpfs};
use std::collections::BTreeMap;
use std::os::unix::fs::{FileExt, PermissionsExt};
use std::sync::Arc;

type Fs<'a> = Ext2<&'a MemDisk>;

fn mount(disk: &MemDisk) -> Fs<'_> {
    Ext2::mount(disk, Ticker::new(), MountOptions::default()).unwrap()
}

/// Volume configurations: mke2fs arguments.
const CONFIGS: &[(&str, &[&str])] = &[
    ("1k", &["-b", "1024", "-I", "256", "-O", "ext_attr,resize_inode,dir_index,filetype,sparse_super,large_file"]),
    ("2k", &["-b", "2048", "-I", "128", "-O", "filetype,sparse_super,large_file"]),
    ("4k", &["-b", "4096", "-I", "256", "-O", "filetype,sparse_super,large_file,dir_index"]),
    ("plain-1k", &["-b", "1024", "-I", "128", "-O", "none"]),
    ("rev0-1k", &["-b", "1024", "-r", "0"]),
];

/// A host tree with every kind of content `mke2fs -d` copies.
fn host_tree_for(dir: &TempDir, bs: u64) -> std::path::PathBuf {
    let root = dir.path("tree");
    std::fs::create_dir_all(root.join("a/b/c/d")).unwrap();
    std::fs::create_dir_all(root.join("many")).unwrap();
    std::fs::create_dir_all(root.join("empty")).unwrap();
    let sizes = [0u64, 1, 59, 60, 1023, 1024, 1025, 4095, 4096, 4097, 12 * bs, 12 * bs + 1, 70_000];
    for (i, size) in sizes.iter().enumerate() {
        std::fs::write(root.join(format!("f{i}")), pattern(i as u64, *size as usize)).unwrap();
    }
    // Past the single indirect block: into double indirect.
    std::fs::write(root.join("a/double"), pattern(99, ((12 + bs / 4 + 7) * bs) as usize)).unwrap();
    // Sparse: data, a hole, data in the double (and for 1 KiB blocks
    // triple) indirect range, a hole at the end.
    let sparse = std::fs::File::create(root.join("a/b/sparse")).unwrap();
    sparse.write_all_at(b"start", 0).unwrap();
    sparse.write_all_at(&pattern(5, 3000), 300_000).unwrap();
    sparse.write_all_at(b"far", 70 << 20).unwrap();
    sparse.set_len((70 << 20) + 10_000).unwrap();
    for i in 0..300 {
        std::fs::write(root.join(format!("many/entry-with-a-longish-name-{i:04}")), format!("{i}")).unwrap();
    }
    std::os::unix::fs::symlink("f1", root.join("short")).unwrap();
    std::os::unix::fs::symlink("x".repeat(59), root.join("fast59")).unwrap();
    std::os::unix::fs::symlink("y".repeat(60), root.join("slow60")).unwrap();
    std::os::unix::fs::symlink("a/b/".repeat(60), root.join("slow240")).unwrap();
    std::os::unix::fs::symlink("../../f2", root.join("a/b/up")).unwrap();
    std::fs::hard_link(root.join("f3"), root.join("a/f3-again")).unwrap();
    std::fs::set_permissions(root.join("f4"), std::fs::Permissions::from_mode(0o4751)).unwrap();
    std::fs::set_permissions(root.join("a/b"), std::fs::Permissions::from_mode(0o1777)).unwrap();
    root
}

const SPECIALS: &str = "mknod fifo p\n\
    mknod tty c 4 64\n\
    mknod nvme b 8 2\n\
    sif nvme block[0] 0\n\
    sif nvme block[1] 0x1E9301B0\n\
    mknod big b 259 300\n\
    mknod old b 8 1\n\
    sif /f5 uid 100000\n\
    sif /f5 gid 70001\n\
    sif /f6 mtime 20300101000000\n";

#[test]
fn reads_images_made_by_mke2fs() {
    let Some(t) = tools() else {
        eprintln!("e2fsprogs not installed: skipping images made by mke2fs");
        return;
    };
    for (name, args) in CONFIGS {
        let dir = TempDir::new(&format!("read-{name}"));
        let bs: u64 = args.windows(2).find(|w| w[0] == "-b").map_or(1024, |w| w[1].parse().unwrap());
        let root = host_tree_for(&dir, bs);
        let mut all = args.to_vec();
        let rootarg = root.display().to_string();
        all.extend(["-d", &rootarg]);
        let mut image = t.mkfs(&dir, 20 << 20, &all);
        let out = t.debugfs(&dir, &mut image, SPECIALS);
        assert!(!out.contains("Couldn't") && !out.contains("rror"), "{name}: debugfs: {out}");
        t.assert_clean(&dir, &image, &format!("mke2fs {name}"));

        let disk = MemDisk::new(image.clone(), 512);
        {
            let fs =
                Ext2::mount(&disk, Ticker::new(), MountOptions { read_only: true, ..MountOptions::default() }).unwrap();
            assert!(fs.stats().was_clean);
            let mut got = tree(&fs);
            // Specials and owners come from debugfs, not the host tree.
            let mut special = |path: &str, kind, rdev: (u32, u32)| {
                let n = got.remove(path).unwrap_or_else(|| panic!("{name}: {path} missing"));
                match n {
                    Node::Special { kind: k, rdev: r, .. } => assert_eq!((k, r), (kind, rdev), "{name}: {path}"),
                    n => panic!("{name}: {path} is {n:?}"),
                }
            };
            special("/fifo", FileType::Fifo, (0, 0));
            special("/tty", FileType::CharDevice, (4, 64));
            special("/nvme", FileType::BlockDevice, (769, 125360));
            special("/big", FileType::BlockDevice, (259, 300));
            special("/old", FileType::BlockDevice, (8, 1));
            got.remove("/lost+found");
            let want = host_tree(&root);
            assert_eq!(got.len(), want.len(), "{name}: node count");
            for (path, node) in &want {
                assert_eq!(got.get(path), Some(node), "{name}: {path}");
            }
            // mke2fs records Linux as the creator: ids have high halves.
            let f5 = fs.getattr(resolve(&fs, "f5").unwrap()).unwrap();
            assert_eq!((f5.uid, f5.gid), (100000, 70001), "{name}");
            let f6 = fs.getattr(resolve(&fs, "f6").unwrap()).unwrap();
            assert_eq!(f6.mtime.sec, 1_893_456_000, "{name}: a time past 2038 in a large inode or clamped");
            let sparse = fs.getattr(resolve(&fs, "a/b/sparse").unwrap()).unwrap();
            assert_eq!(sparse.size, (70 << 20) + 10_000);
            assert!(sparse.blocks < 64, "{name}: holes take no blocks ({} sectors)", sparse.blocks);
            let link = fs.getattr(resolve(&fs, "a/f3-again").unwrap()).unwrap();
            assert_eq!((link.nlink, link.ino), (2, resolve(&fs, "f3").unwrap()));
            assert_eq!(fs.getattr(resolve(&fs, "a").unwrap()).unwrap().nlink, 3);
            assert_eq!(fs.write(resolve(&fs, "f1").unwrap(), 0, b"x"), Err(Errno::EROFS));
        }
        // Mounting read-write and unmounting changes nothing e2fsck minds.
        let fs = mount(&disk);
        assert!(!fs.read_only());
        fs.unmount().unwrap();
        drop(fs);
        t.assert_clean(&dir, &disk.image(), &format!("mount/unmount {name}"));
    }
}

/// Every operation of the trait, done on ext2 and on tmpfs alike.
fn workload(fs: &dyn FileSystem, bs: u64) {
    let root = fs.root();
    let node = |kind, perm, rdev| NewNode { kind, perm, uid: 1000, gid: 100, rdev };
    let reg = node(FileType::Regular, 0o644, DeviceId::default());
    let d1 = fs.mkdir(root, b"dir", 0o755, 0, 0).unwrap();
    let d2 = fs.mkdir(d1, b"sub", 0o700, 1000, 1000).unwrap();
    let _d3 = fs.mkdir(d2, b"deeper", 0o750, 0, 0).unwrap();
    for (i, size) in [0usize, 1, 100, 4095, 4096, 4097, 50_000, 300_000].into_iter().enumerate() {
        let f = fs.create(d1, format!("file{i}").as_bytes(), &reg).unwrap();
        let data = pattern(1000 + i as u64, size);
        // Write in uneven pieces.
        let mut at = 0;
        while at < size {
            let n = (size - at).min(7777);
            assert_eq!(fs.write(f, at as u64, &data[at..at + n]).unwrap(), n);
            at += n;
        }
    }
    // Past the double indirect start.
    let big = fs.create(root, b"big", &reg).unwrap();
    let len = ((12 + bs / 4 + 3) * bs + 17) as usize;
    fs.write(big, 0, &pattern(7, len)).unwrap();
    // Sparse, into triple indirect for 1 KiB blocks.
    let sparse = fs.create(root, b"sparse", &reg).unwrap();
    fs.write(sparse, 5, b"hello").unwrap();
    fs.write(sparse, 80 << 20, b"far away").unwrap();
    fs.write(sparse, 3 << 20, &pattern(8, 10_000)).unwrap();
    // Truncation: down into the middle of a block, then up again.
    let t = fs.create(root, b"trunc", &reg).unwrap();
    fs.write(t, 0, &pattern(9, 200_000)).unwrap();
    fs.truncate(t, 100_001).unwrap();
    fs.truncate(t, 150_000).unwrap();
    fs.write(t, 160_000, b"tail").unwrap();
    let t2 = fs.create(root, b"trunc-to-zero", &reg).unwrap();
    fs.write(t2, 0, &pattern(10, 90_000)).unwrap();
    fs.truncate(t2, 0).unwrap();
    // Symbolic links, fast and slow; device nodes; FIFOs and sockets.
    fs.symlink(root, b"fast", b"dir/file1", 0, 0).unwrap();
    fs.symlink(root, b"slow", "../".repeat(40).as_bytes(), 0, 0).unwrap();
    fs.create(root, b"null", &node(FileType::CharDevice, 0o666, DeviceId::new(1, 3))).unwrap();
    fs.create(root, b"bigdev", &node(FileType::BlockDevice, 0o660, DeviceId::new(259, 70000))).unwrap();
    fs.create(root, b"fifo", &node(FileType::Fifo, 0o600, DeviceId::default())).unwrap();
    fs.create(root, b"sock", &node(FileType::Socket, 0o755, DeviceId::default())).unwrap();
    // Links, renames, removals.
    let f0 = fs.lookup(d1, b"file2").unwrap();
    fs.link(d2, b"file2-link", f0).unwrap();
    fs.link(root, b"file2-link2", f0).unwrap();
    fs.unlink(root, b"file2-link2").unwrap();
    fs.rename(d1, b"file3", root, b"moved", RenameFlags::default()).unwrap();
    fs.rename(d1, b"file4", d1, b"file5", RenameFlags::default()).unwrap();
    fs.rename(root, b"dir", root, b"renamed-dir", RenameFlags::default()).unwrap();
    fs.rename(d1, b"sub", root, b"sub-moved", RenameFlags::default()).unwrap();
    fs.rename(root, b"fifo", d1, b"sock-swap", RenameFlags::default()).unwrap();
    fs.rename(root, b"sock", d1, b"sock-swap", RenameFlags { exchange: true, noreplace: false }).unwrap();
    let gone = fs.mkdir(root, b"gone", 0o755, 0, 0).unwrap();
    fs.create(gone, b"x", &reg).unwrap();
    assert_eq!(fs.rmdir(root, b"gone"), Err(Errno::ENOTEMPTY));
    fs.unlink(gone, b"x").unwrap();
    fs.rmdir(root, b"gone").unwrap();
    fs.unlink(d1, b"file0").unwrap();
    // An unlinked file that is still open, then closed.
    let open = fs.create(root, b"open", &reg).unwrap();
    fs.write(open, 0, &pattern(11, 30_000)).unwrap();
    fs.open(open).unwrap();
    fs.unlink(root, b"open").unwrap();
    assert_eq!(read_all(fs, open).unwrap(), pattern(11, 30_000));
    fs.write(open, 30_000, b"more").unwrap();
    fs.release(open);
    assert_eq!(fs.getattr(open).map(|_| ()), Err(Errno::ENOENT));
    // Attributes.
    let f1 = fs.lookup(d1, b"file1").unwrap();
    fs.setattr(f1, &SetAttr { perm: Some(0o4711), uid: Some(123_456), gid: Some(7), ..SetAttr::default() }).unwrap();
}

#[test]
fn writes_pass_fsck_and_read_back() {
    let t = tools();
    if t.is_none() {
        eprintln!("e2fsprogs not installed: checking against format() volumes and tmpfs only");
    }
    let configs: Vec<(&str, Vec<&str>, Option<FormatOptions>)> = vec![
        ("1k", CONFIGS[0].1.to_vec(), Some(FormatOptions { block_size: 1024, ..FormatOptions::default() })),
        (
            "2k",
            CONFIGS[1].1.to_vec(),
            Some(FormatOptions { block_size: 2048, inode_size: 128, ..FormatOptions::default() }),
        ),
        ("4k", CONFIGS[2].1.to_vec(), Some(FormatOptions::default())),
        (
            "plain-1k",
            CONFIGS[3].1.to_vec(),
            Some(FormatOptions {
                block_size: 1024,
                filetype: false,
                sparse_super: false,
                large_file: false,
                ..FormatOptions::default()
            }),
        ),
        (
            "rev0-1k",
            CONFIGS[4].1.to_vec(),
            Some(FormatOptions { block_size: 1024, dynamic: false, ..FormatOptions::default() }),
        ),
    ];
    for (name, args, fmt) in configs {
        let dir = TempDir::new(&format!("write-{name}"));
        let bs = fmt.unwrap().block_size as u64;
        let image = match &t {
            Some(t) => t.mkfs(&dir, 24 << 20, &args),
            None => {
                let disk = MemDisk::new(vec![0; 24 << 20], 512);
                poly_ext2::format(&disk, &fmt.unwrap()).unwrap();
                disk.image()
            }
        };
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
            let f1 = resolve(&fs, "renamed-dir/file1").unwrap();
            let a = fs.getattr(f1).unwrap();
            assert_eq!((a.uid, a.gid, a.perm), (123_456, 7, 0o4711));
            fs.unmount().unwrap();
        }
        // Mounted again, it reads the same.
        {
            let fs = mount(&disk);
            let mut got = tree(&fs);
            got.remove("/lost+found");
            assert_eq!(got, want, "{name}: after remount");
            assert_eq!(fs.getattr(resolve(&fs, "renamed-dir").unwrap()).unwrap().nlink, 2, "{name}");
            assert_eq!(fs.getattr(fs.root()).unwrap().nlink, 5, "{name}: root links");
            fs.unmount().unwrap();
        }
        let Some(t) = &t else { continue };
        t.assert_clean(&dir, &disk.image(), &format!("the workload on {name}"));
        let out = dir.path("rdump");
        t.rdump(&dir, &disk.image(), &out);
        let dumped = host_tree(&out);
        let mut want_files: BTreeMap<String, Node> = want
            .iter()
            .filter(|(_, n)| !matches!(n, Node::Special { .. }))
            .map(|(p, n)| (p.clone(), n.clone()))
            .collect();
        want_files.insert("/lost+found".into(), Node::Dir { perm: 0o700 });
        // rdump does not restore set-user-ID bits as root may not; compare
        // contents and kinds.
        let strip = |m: &BTreeMap<String, Node>| -> BTreeMap<String, Node> {
            m.iter()
                .map(|(p, n)| {
                    let n = match n {
                        Node::File { data, .. } => Node::File { perm: 0, data: data.clone() },
                        Node::Dir { .. } => Node::Dir { perm: 0 },
                        n => n.clone(),
                    };
                    (p.clone(), n)
                })
                .collect()
        };
        assert_eq!(strip(&dumped), strip(&want_files), "{name}: debugfs rdump");
    }
}

#[test]
fn format_passes_fsck() {
    let Some(t) = tools() else {
        eprintln!("e2fsprogs not installed: skipping the e2fsck check of format()");
        return;
    };
    let dir = TempDir::new("format");
    for (size, opts) in [
        (8 << 20, FormatOptions { block_size: 1024, ..FormatOptions::default() }),
        (40 << 20, FormatOptions { block_size: 1024, blocks_per_group: Some(2048), ..FormatOptions::default() }),
        (
            40 << 20,
            FormatOptions { block_size: 2048, inode_size: 128, sparse_super: false, ..FormatOptions::default() },
        ),
        (140 << 20, FormatOptions::default()),
        (5 << 20, FormatOptions { block_size: 1024, dynamic: false, ..FormatOptions::default() }),
        ((3 << 20) + 5000, FormatOptions { block_size: 4096, blocks_per_group: Some(256), ..FormatOptions::default() }),
    ] {
        let disk = MemDisk::new(vec![0; size], 512);
        poly_ext2::format(&disk, &opts).unwrap();
        t.assert_clean(&dir, &disk.image(), &format!("format {opts:?}"));
        let fs = mount(&disk);
        assert_eq!(list(&fs, fs.root()).unwrap().len(), 1);
        fs.mkdir(fs.root(), b"x", 0o755, 0, 0).unwrap();
        fs.unmount().unwrap();
        t.assert_clean(&dir, &disk.image(), &format!("mkdir on format {opts:?}"));
    }
}

#[test]
fn many_files_in_one_directory() {
    let t = tools();
    let dir = TempDir::new("many");
    let disk = MemDisk::new(vec![0; 32 << 20], 512);
    poly_ext2::format(&disk, &FormatOptions { block_size: 1024, ..FormatOptions::default() }).unwrap();
    let reg = NewNode { kind: FileType::Regular, perm: 0o644, uid: 0, gid: 0, rdev: DeviceId::default() };
    let n = 3000;
    {
        let fs = mount(&disk);
        let d = fs.mkdir(fs.root(), b"d", 0o755, 0, 0).unwrap();
        for i in 0..n {
            fs.create(d, format!("file-number-{i:05}").as_bytes(), &reg).unwrap();
        }
        let names = list(&fs, d).unwrap();
        assert_eq!(names.len(), n);
        // Remove every other one, then add new ones into the gaps.
        for i in (0..n).step_by(2) {
            fs.unlink(d, format!("file-number-{i:05}").as_bytes()).unwrap();
        }
        let size_before = fs.getattr(d).unwrap().size;
        for i in 0..n / 2 {
            fs.create(d, format!("new-{i:05}").as_bytes(), &reg).unwrap();
        }
        assert_eq!(fs.getattr(d).unwrap().size, size_before, "new entries reuse the freed space");
        for i in (1..n).step_by(2) {
            fs.lookup(d, format!("file-number-{i:05}").as_bytes()).unwrap();
        }
        assert_eq!(fs.lookup(d, b"file-number-00000"), Err(Errno::ENOENT));
        // A listing in small steps sees every entry exactly once.
        let mut seen = Vec::new();
        let mut pos = 0;
        loop {
            let mut got = 0;
            fs.readdir(d, pos, &mut |e| {
                if got == 7 {
                    return false;
                }
                got += 1;
                seen.push(e.name.to_vec());
                pos = e.next;
                true
            })
            .unwrap();
            if got == 0 {
                break;
            }
        }
        assert_eq!(seen.len(), n + 2);
        seen.sort();
        seen.dedup();
        assert_eq!(seen.len(), n + 2);
        fs.unmount().unwrap();
    }
    if let Some(t) = t {
        t.assert_clean(&dir, &disk.image(), "3000 files in one directory");
        let mut img = disk.image();
        let out = t.debugfs(&dir, &mut img, "ls -l /d\n");
        assert_eq!(out.matches("new-").count(), n / 2);
    }
}

#[test]
fn hash_indexed_directories() {
    let Some(t) = tools() else {
        eprintln!("e2fsprogs not installed: skipping hash-indexed directories");
        return;
    };
    let dir = TempDir::new("htree");
    let root = dir.path("tree");
    std::fs::create_dir_all(root.join("big")).unwrap();
    for i in 0..600 {
        std::fs::write(root.join(format!("big/name-{i:04}")), format!("{i}")).unwrap();
    }
    let rootarg = root.display().to_string();
    let mut image = t.mkfs(&dir, 16 << 20, &["-b", "1024", "-O", "dir_index,filetype", "-d", &rootarg]);
    let (code, out) = t.fsck(&dir, &mut image, "-fyD");
    assert!(code <= 1, "e2fsck -fyD: {out}");
    let mut img = image.clone();
    let stat = t.debugfs(&dir, &mut img, "stat /big\n");
    assert!(stat.contains("Flags: 0x1000"), "the directory got an index: {stat}");
    let disk = MemDisk::new(image, 512);
    let reg = NewNode { kind: FileType::Regular, perm: 0o644, uid: 0, gid: 0, rdev: DeviceId::default() };
    {
        let fs = mount(&disk);
        let big = resolve(&fs, "big").unwrap();
        assert_eq!(list(&fs, big).unwrap().len(), 600);
        for i in (0..600).step_by(37) {
            let f = fs.lookup(big, format!("name-{i:04}").as_bytes()).unwrap();
            assert_eq!(read_all(&fs, f).unwrap(), format!("{i}").as_bytes());
        }
        fs.create(big, b"added", &reg).unwrap();
        fs.unlink(big, b"name-0100").unwrap();
        fs.unmount().unwrap();
    }
    t.assert_clean(&dir, &disk.image(), "changing a hash-indexed directory");
    let mut img = disk.image();
    let stat = t.debugfs(&dir, &mut img, "stat /big\n");
    assert!(stat.contains("Flags: 0x0"), "the index flag is gone: {stat}");
}

#[test]
fn large_files_and_the_large_file_feature() {
    let t = tools();
    let dir = TempDir::new("large");
    let image = match &t {
        Some(t) => t.mkfs(&dir, 16 << 20, &["-b", "2048", "-O", "filetype,sparse_super,^large_file"]),
        None => {
            let disk = MemDisk::new(vec![0; 16 << 20], 512);
            let opts = FormatOptions { block_size: 2048, large_file: false, ..FormatOptions::default() };
            poly_ext2::format(&disk, &opts).unwrap();
            disk.image()
        }
    };
    let disk = MemDisk::new(image, 512);
    let reg = NewNode { kind: FileType::Regular, perm: 0o644, uid: 0, gid: 0, rdev: DeviceId::default() };
    {
        let fs = mount(&disk);
        let f = fs.create(fs.root(), b"huge", &reg).unwrap();
        fs.write(f, 3 << 30, b"past 2 GiB").unwrap();
        fs.write(f, 0, b"start").unwrap();
        assert_eq!(fs.getattr(f).unwrap().size, (3 << 30) + 10);
        let mut buf = [0u8; 10];
        assert_eq!(fs.read(f, 3 << 30, &mut buf).unwrap(), 10);
        assert_eq!(&buf, b"past 2 GiB");
        assert_eq!(fs.read(f, 1 << 30, &mut buf).unwrap(), 10);
        assert_eq!(buf, [0; 10]);
        // The largest offset a 2 KiB block map reaches, and one past it.
        let p = 512u64;
        let max = (12 + p + p * p + p * p * p) * 2048;
        assert_eq!(fs.write(f, max - 1, b"x"), Ok(1));
        assert_eq!(fs.write(f, max, b"x"), Err(Errno::EFBIG));
        fs.unmount().unwrap();
    }
    if let Some(t) = &t {
        t.assert_clean(&dir, &disk.image(), "a 2 GiB+ file on a volume without large_file");
        let mut img = disk.image();
        let out = t.debugfs(&dir, &mut img, "features\n");
        assert!(out.contains("large_file"), "{out}");
    }
    // Freeing it gives every block back.
    let fs = mount(&disk);
    let before = fs.stats();
    fs.unlink(fs.root(), b"huge").unwrap();
    fs.sync().unwrap();
    let after = fs.stats();
    assert!(after.free_blocks > before.free_blocks);
    fs.unmount().unwrap();
    if let Some(t) = &t {
        t.assert_clean(&dir, &disk.image(), "deleting the large file");
    }
}

#[test]
fn rename_edge_cases() {
    let t = tools();
    let dir = TempDir::new("rename");
    let disk = MemDisk::new(vec![0; 8 << 20], 512);
    poly_ext2::format(&disk, &FormatOptions { block_size: 1024, ..FormatOptions::default() }).unwrap();
    let fs = mount(&disk);
    let model = Tmpfs::new(Ticker::new());
    let reg = NewNode { kind: FileType::Regular, perm: 0o644, uid: 0, gid: 0, rdev: DeviceId::default() };
    for fs in [&fs as &dyn FileSystem, &model] {
        let r = fs.root();
        let a = fs.mkdir(r, b"a", 0o755, 0, 0).unwrap();
        let b = fs.mkdir(a, b"b", 0o755, 0, 0).unwrap();
        let c = fs.mkdir(r, b"c", 0o755, 0, 0).unwrap();
        let empty = fs.mkdir(r, b"empty", 0o755, 0, 0).unwrap();
        let f = fs.create(r, b"f", &reg).unwrap();
        fs.write(f, 0, b"file f").unwrap();
        let g = fs.create(a, b"g", &reg).unwrap();
        fs.link(r, b"f-link", f).unwrap();
        let none = RenameFlags::default();
        let noreplace = RenameFlags { noreplace: true, exchange: false };
        let exchange = RenameFlags { noreplace: false, exchange: true };
        // Into itself or below: EINVAL.
        assert_eq!(fs.rename(r, b"a", a, b"x", none), Err(Errno::EINVAL));
        assert_eq!(fs.rename(r, b"a", b, b"x", none), Err(Errno::EINVAL));
        assert_eq!(fs.rename(r, b"f", r, b"nothing", exchange), Err(Errno::ENOENT));
        // Kinds must match when replacing.
        assert_eq!(fs.rename(r, b"f", r, b"empty", none), Err(Errno::EISDIR));
        assert_eq!(fs.rename(r, b"empty", r, b"f", none), Err(Errno::ENOTDIR));
        assert_eq!(fs.rename(r, b"empty", r, b"a", none), Err(Errno::ENOTEMPTY));
        assert_eq!(fs.rename(r, b"f", a, b"g", noreplace), Err(Errno::EEXIST));
        // Hard links to the same inode: nothing happens.
        fs.rename(r, b"f", r, b"f-link", none).unwrap();
        assert_eq!(fs.lookup(r, b"f").unwrap(), f);
        // Replace a file across directories; the old one goes.
        fs.rename(r, b"f", a, b"g", none).unwrap();
        assert_eq!(fs.lookup(a, b"g").unwrap(), f);
        assert_eq!(fs.getattr(g).map(|_| ()), Err(Errno::ENOENT));
        assert_eq!(fs.getattr(f).unwrap().nlink, 2);
        // Replace an empty directory with a directory from elsewhere.
        fs.rename(a, b"b", r, b"empty", none).unwrap();
        assert_eq!(fs.lookup(b, b"..").unwrap(), r);
        assert_eq!(fs.getattr(empty).map(|_| ()), Err(Errno::ENOENT));
        assert_eq!(fs.getattr(a).unwrap().nlink, 2);
        // Exchange a directory and a file across directories.
        fs.rename(r, b"empty", a, b"g", exchange).unwrap();
        assert_eq!(fs.lookup(a, b"g").unwrap(), b);
        assert_eq!(fs.lookup(b, b"..").unwrap(), a);
        assert_eq!(fs.lookup(r, b"empty").unwrap(), f);
        assert_eq!(fs.getattr(a).unwrap().nlink, 3);
        // Same directory, many times, long names (records split and merge).
        let mut name = b"z".to_vec();
        for i in 0..50 {
            let next = format!("{}-{i}", "long".repeat(i % 20)).into_bytes();
            if i == 0 {
                fs.rename(r, b"empty", c, &next, none).unwrap();
            } else {
                fs.rename(c, &name, c, &next, none).unwrap();
            }
            name = next;
        }
        assert_eq!(list(fs, c).unwrap().len(), 1);
    }
    assert_eq!(tree(&fs), {
        let mut m = tree(&model);
        m.insert("/lost+found".into(), Node::Dir { perm: 0o700 });
        m
    });
    fs.unmount().unwrap();
    if let Some(t) = t {
        t.assert_clean(&dir, &disk.image(), "rename edge cases");
    }
}

#[test]
fn shares_a_volume_through_the_vfs() {
    use poly_vfs::{At, Cred, OpenFlags, Vfs};
    let disk: &'static MemDisk = Box::leak(Box::new(MemDisk::new(vec![0; 4 << 20], 4096)));
    poly_ext2::format(disk, &FormatOptions { block_size: 1024, ..FormatOptions::default() }).unwrap();
    let fs = Arc::new(Ext2::mount(disk, Ticker::new(), MountOptions::default()).unwrap());
    let vfs = Vfs::new(fs.clone());
    let ctx = vfs.root_context().unwrap();
    let root = Cred::root();
    vfs.mkdir(&ctx, &root, At::Cwd, b"/etc", 0o755).unwrap();
    let f = vfs.open(&ctx, &root, At::Cwd, b"/etc/motd", OpenFlags::from_bits(0o1102), 0o644).unwrap();
    f.write(b"hello from ext2\n").unwrap();
    vfs.symlink(&ctx, &root, b"etc/motd", At::Cwd, b"/motd").unwrap();
    let g = vfs.open(&ctx, &root, At::Cwd, b"/motd", OpenFlags::from_bits(0), 0).unwrap();
    let mut buf = [0u8; 64];
    let n = g.read(&mut buf).unwrap();
    assert_eq!(&buf[..n], b"hello from ext2\n");
    let st = vfs.stat(&ctx, &root, At::Cwd, b"/etc/motd", true).unwrap();
    assert_eq!((st.size, st.nlink, st.blksize), (16, 1, 1024));
    drop((f, g));
    fs.sync().unwrap();
    fs.unmount().unwrap();
    assert_eq!(vfs.mkdir(&ctx, &root, At::Cwd, b"/x", 0o755), Err(Errno::EROFS));
}

#[test]
fn extended_attribute_blocks_are_released_by_reference_count() {
    let Some(t) = tools() else {
        eprintln!("e2fsprogs not installed: skipping extended attribute blocks");
        return;
    };
    let dir = TempDir::new("xattr");
    let mut image = t.mkfs(&dir, 4 << 20, &["-b", "1024", "-I", "128", "-O", "ext_attr,filetype,sparse_super"]);
    let src = dir.path("content");
    std::fs::write(&src, b"some content").unwrap();
    let val = dir.path("value");
    std::fs::write(&val, [b'v'; 400]).unwrap();
    // Two files with an attribute block each, then the second made to share
    // the first's block (reference count 2) and its own freed.
    let cmds = format!(
        "write {s} f\nwrite {s} g\nea_set -f {v} f user.big\nea_set -f {v} g user.big\nstat f\nstat g\n",
        s = src.display(),
        v = val.display()
    );
    let out = t.debugfs(&dir, &mut image, &cmds);
    let acls: Vec<u32> =
        out.lines().filter_map(|l| l.strip_prefix("File ACL: ")).map(|n| n.trim().parse().unwrap()).collect();
    assert_eq!(acls.len(), 2, "{out}");
    let share = format!("sif g file_acl {}\nfreeb {}\nzap_block -o 4 -l 1 -p 2 {}\n", acls[0], acls[1], acls[0]);
    t.debugfs(&dir, &mut image, &share);
    let (code, out) = t.fsck(&dir, &mut image, "-fy");
    assert!(code <= 1, "{out}");
    t.assert_clean(&dir, &image, "sharing an attribute block");
    let disk = MemDisk::new(image, 512);
    let free = |fs: &Fs<'_>| {
        fs.sync().unwrap();
        fs.stats().free_blocks
    };
    let fs = mount(&disk);
    let start = free(&fs);
    // The first unlink drops a reference; the block stays.
    fs.unlink(fs.root(), b"f").unwrap();
    assert_eq!(free(&fs), start + 1, "only the data block is freed");
    fs.unmount().unwrap();
    drop(fs);
    t.assert_clean(&dir, &disk.image(), "unlinking one of two attribute block users");
    let fs = mount(&disk);
    fs.unlink(fs.root(), b"g").unwrap();
    assert_eq!(free(&fs), start + 3, "the attribute block goes with its last user");
    fs.unmount().unwrap();
    drop(fs);
    t.assert_clean(&dir, &disk.image(), "unlinking the last attribute block user");
}

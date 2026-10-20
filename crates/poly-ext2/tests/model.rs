//! Randomised tests: the driver against tmpfs as a model, corrupted
//! volumes, and crashes at every kind of point.
//!
//! - **Model**: thousands of random operations on ext2 and on tmpfs; every
//!   result (value or error number) must match, and so must the trees,
//!   also after remounting. The volume then passes `e2fsck -fn`.
//! - **Corruption**: random byte changes in a populated volume's metadata
//!   and anywhere in its used part; mounting, walking, reading and writing
//!   must never panic or hang.
//! - **Crashes**: a workload on a disk that records its writes and flushes;
//!   a crash keeps everything before some flush and any subset of the
//!   writes after it. e2fsck must repair every such volume without
//!   multiply-claimed blocks, and names that were renamed over must exist.

mod common;

use common::*;
use poly_ext2::{Ext2, FormatOptions, MountOptions};
use poly_vfs::{DeviceId, Errno, FileSystem, FileType, Ino, NewNode, RenameFlags, SetAttr, Tmpfs};
use std::collections::BTreeMap;
use std::panic::{AssertUnwindSafe, catch_unwind};

const NAMES: &[&str] = &["a", "b", "c", "long-name-number-one", "x"];

/// Directories that exist in the model, by path ("" is the root).
fn dirs(fs: &dyn FileSystem) -> Vec<String> {
    let mut out = vec![String::new()];
    let mut i = 0;
    while i < out.len() {
        let d = out[i].clone();
        if let Ok(ino) = resolve(fs, &d) {
            for (name, _, kind) in list(fs, ino).unwrap_or_default() {
                if kind == FileType::Directory && name != b"lost+found" {
                    out.push(format!("{d}/{}", String::from_utf8_lossy(&name)));
                }
            }
        }
        i += 1;
    }
    out
}

/// One random operation, as a function of a file system to its outcome.
type Op = Box<dyn Fn(&dyn FileSystem) -> Result<String, Errno>>;

fn random_op(rng: &mut Rng, dirs: &[String]) -> (String, Op) {
    let dir = dirs[rng.below(dirs.len() as u64) as usize].clone();
    let name = NAMES[rng.below(NAMES.len() as u64) as usize];
    let path = format!("{dir}/{name}");
    let dir2 = dirs[rng.below(dirs.len() as u64) as usize].clone();
    let name2 = NAMES[rng.below(NAMES.len() as u64) as usize];
    let path2 = format!("{dir2}/{name2}");
    let seed = rng.next();
    let k = rng.below(100);
    let small = rng.below(20_000) as usize;
    let offset = match rng.below(10) {
        0 => rng.below(20 << 20),
        1..=3 => rng.below(300_000),
        _ => rng.below(20_000),
    };
    let perm = (rng.below(0o10000)) as u16;
    match k {
        0..=11 => {
            let kind = [FileType::Regular, FileType::Regular, FileType::Fifo, FileType::CharDevice, FileType::Socket]
                [rng.below(5) as usize];
            let rdev = DeviceId::new(rng.below(300) as u32, rng.below(70_000) as u32);
            let desc = format!("create {path} {kind:?}");
            (
                desc,
                Box::new(move |fs| {
                    let (d, n) = parent(fs, &path)?;
                    let rdev = if kind == FileType::CharDevice { rdev } else { DeviceId::default() };
                    fs.create(d, n.as_bytes(), &NewNode { kind, perm: perm & 0o777, uid: 7, gid: 8, rdev })
                        .map(|_| String::new())
                }),
            )
        }
        12..=19 => (
            format!("mkdir {path}"),
            Box::new(move |fs| {
                let (d, n) = parent(fs, &path)?;
                fs.mkdir(d, n.as_bytes(), perm, 1, 2).map(|_| String::new())
            }),
        ),
        20..=24 => {
            let target = pattern(seed, 1 + small % 300).iter().map(|b| b'a' + b % 26).collect::<Vec<u8>>();
            (
                format!("symlink {path}"),
                Box::new(move |fs| {
                    let (d, n) = parent(fs, &path)?;
                    fs.symlink(d, n.as_bytes(), &target, 0, 0).map(|_| String::new())
                }),
            )
        }
        25..=28 => (
            format!("link {path2} -> {path}"),
            Box::new(move |fs| {
                let src = resolve(fs, &path)?;
                let (d, n) = parent(fs, &path2)?;
                fs.link(d, n.as_bytes(), src).map(|_| String::new())
            }),
        ),
        29..=36 => (
            format!("unlink {path}"),
            Box::new(move |fs| {
                let (d, n) = parent(fs, &path)?;
                fs.unlink(d, n.as_bytes()).map(|_| String::new())
            }),
        ),
        37..=41 => (
            format!("rmdir {path}"),
            Box::new(move |fs| {
                let (d, n) = parent(fs, &path)?;
                fs.rmdir(d, n.as_bytes()).map(|_| String::new())
            }),
        ),
        42..=51 => {
            let flags = match seed % 6 {
                0 => RenameFlags { noreplace: true, exchange: false },
                1 => RenameFlags { noreplace: false, exchange: true },
                _ => RenameFlags::default(),
            };
            (
                format!("rename {path} {path2} {flags:?}"),
                Box::new(move |fs| {
                    let (d1, n1) = parent(fs, &path)?;
                    let (d2, n2) = parent(fs, &path2)?;
                    fs.rename(d1, n1.as_bytes(), d2, n2.as_bytes(), flags).map(|_| String::new())
                }),
            )
        }
        52..=69 => (
            format!("write {path} @{offset} +{small}"),
            Box::new(move |fs| {
                let ino = resolve(fs, &path)?;
                fs.write(ino, offset, &pattern(seed, small)).map(|n| n.to_string())
            }),
        ),
        70..=75 => (
            format!("truncate {path} {offset}"),
            Box::new(move |fs| {
                let ino = resolve(fs, &path)?;
                fs.truncate(ino, offset).map(|_| String::new())
            }),
        ),
        76..=83 => (
            format!("read {path} @{offset} +{small}"),
            Box::new(move |fs| {
                let ino = resolve(fs, &path)?;
                let mut buf = vec![0u8; small];
                let n = fs.read(ino, offset / 4, &mut buf)?;
                Ok(format!("{n} {:?}", buf[..n].iter().map(|&b| b as u64).sum::<u64>()))
            }),
        ),
        84..=88 => (
            format!("readdir {dir}"),
            Box::new(move |fs| {
                let ino = resolve(fs, &dir)?;
                let mut l = list(fs, ino)?;
                l.retain(|e| e.0 != b"lost+found");
                Ok(format!("{:?}", l.iter().map(|e| (&e.0, e.2)).collect::<Vec<_>>()))
            }),
        ),
        89..=93 => (
            format!("getattr {path}"),
            Box::new(move |fs| {
                let a = fs.getattr(resolve(fs, &path)?)?;
                let size = if a.kind == FileType::Directory { 0 } else { a.size };
                Ok(format!("{:?} {:o} {} {} {} {size} {:?}", a.kind, a.perm, a.nlink, a.uid, a.gid, a.rdev))
            }),
        ),
        94..=96 => (
            format!("setattr {path}"),
            Box::new(move |fs| {
                let ino = resolve(fs, &path)?;
                let set = SetAttr { perm: Some(perm), uid: Some(seed as u32 >> 8), ..SetAttr::default() };
                fs.setattr(ino, &set).map(|a| format!("{:o} {}", a.perm, a.uid))
            }),
        ),
        _ => (
            format!("open-unlink-write-read-release {path}"),
            Box::new(move |fs| {
                let ino = resolve(fs, &path)?;
                if fs.getattr(ino)?.kind != FileType::Regular {
                    return Ok("skip".into());
                }
                let (d, n) = parent(fs, &path)?;
                fs.open(ino)?;
                let r = (|| {
                    fs.unlink(d, n.as_bytes())?;
                    fs.write(ino, offset / 8, &pattern(seed, small))?;
                    let data = read_all(fs, ino)?;
                    Ok(format!("{} {}", data.len(), data.iter().map(|&b| b as u64).sum::<u64>()))
                })();
                fs.release(ino);
                r.map(|s| format!("{s} {:?}", fs.getattr(ino).map(|a| a.nlink)))
            }),
        ),
    }
}

fn compare_trees(model: &dyn FileSystem, ext: &dyn FileSystem, what: &str) {
    let mut got = tree(ext);
    got.remove("/lost+found");
    let want = tree(model);
    if got != want {
        for (p, n) in &want {
            assert_eq!(got.get(p), Some(n), "{what}: {p}");
        }
        assert_eq!(got.keys().collect::<Vec<_>>(), want.keys().collect::<Vec<_>>(), "{what}");
    }
}

fn model_run(seed: u64, fmt: FormatOptions, opts: MountOptions, ops: usize) {
    let tools = tools();
    let disk = MemDisk::new(vec![0; 48 << 20], 512);
    poly_ext2::format(&disk, &fmt).unwrap();
    let model = Tmpfs::new(Ticker::new());
    let mut ext = Some(Ext2::mount(&disk, Ticker::new(), opts).unwrap());
    let mut rng = Rng::new(seed);
    let mut known = dirs(&model);
    for n in 0..ops {
        let (desc, op) = random_op(&mut rng, &known);
        let want = op(&model);
        let fs = ext.as_ref().unwrap();
        let got = op(fs);
        assert_eq!(got, want, "seed {seed} op {n}: {desc}");
        if desc.starts_with("mkdir") || desc.starts_with("rmdir") || desc.starts_with("rename") {
            known = dirs(&model);
        }
        if n % 250 == 249 {
            compare_trees(&model, fs, &format!("seed {seed} after op {n}"));
        }
        if n % 600 == 599 {
            ext.take().unwrap().unmount().unwrap();
            ext = Some(Ext2::mount(&disk, Ticker::new(), opts).unwrap());
        }
    }
    let fs = ext.take().unwrap();
    compare_trees(&model, &fs, &format!("seed {seed} at the end"));
    assert!(!fs.stats().errors, "seed {seed}: the driver found corruption in its own volume");
    fs.unmount().unwrap();
    drop(fs);
    if let Some(t) = tools {
        let dir = TempDir::new(&format!("model-{seed}"));
        t.assert_clean(&dir, &disk.image(), &format!("model run seed {seed} {fmt:?}"));
    }
}

#[test]
fn model_1k_ordered() {
    model_run(1, FormatOptions { block_size: 1024, ..FormatOptions::default() }, MountOptions::default(), 2500);
}

#[test]
fn model_4k_unordered() {
    let opts = MountOptions { ordered: false, ..MountOptions::default() };
    model_run(2, FormatOptions::default(), opts, 2500);
}

#[test]
fn model_2k_without_file_types() {
    let fmt = FormatOptions { block_size: 2048, filetype: false, inode_size: 128, ..FormatOptions::default() };
    model_run(3, fmt, MountOptions::default(), 2500);
}

#[test]
fn model_revision_0_many_groups() {
    let fmt =
        FormatOptions { block_size: 1024, dynamic: false, blocks_per_group: Some(1024), ..FormatOptions::default() };
    model_run(4, fmt, MountOptions::default(), 2500);
}

// ----- corruption -----

/// A volume with a bit of everything, made by the driver itself.
fn populated(bs: u32) -> Vec<u8> {
    let disk = MemDisk::new(vec![0; 3 << 20], 512);
    let fmt = FormatOptions {
        block_size: bs,
        blocks_per_group: Some(1024),
        bytes_per_inode: 4096,
        ..FormatOptions::default()
    };
    poly_ext2::format(&disk, &fmt).unwrap();
    let fs = Ext2::mount(&disk, Ticker::new(), MountOptions::default()).unwrap();
    let reg = NewNode { kind: FileType::Regular, perm: 0o644, uid: 0, gid: 0, rdev: DeviceId::default() };
    let r = fs.root();
    let d = fs.mkdir(r, b"dir", 0o755, 0, 0).unwrap();
    let dd = fs.mkdir(d, b"sub", 0o755, 0, 0).unwrap();
    for i in 0..40 {
        let f = fs.create(if i % 2 == 0 { d } else { dd }, format!("file-{i}").as_bytes(), &reg).unwrap();
        fs.write(f, 0, &pattern(i, (i as usize * 3001) % 20_000)).unwrap();
    }
    let big = fs.create(r, b"big", &reg).unwrap();
    fs.write(big, 0, &pattern(77, 400_000)).unwrap();
    let sparse = fs.create(r, b"sparse", &reg).unwrap();
    fs.write(sparse, 30 << 20, b"far").unwrap();
    fs.symlink(r, b"fast", b"dir/file-0", 0, 0).unwrap();
    fs.symlink(r, b"slow", &[b'q'; 100], 0, 0).unwrap();
    fs.create(r, b"dev", &NewNode { kind: FileType::CharDevice, rdev: DeviceId::new(4, 1), ..reg }).unwrap();
    fs.link(dd, b"hard", big).unwrap();
    fs.unmount().unwrap();
    drop(fs);
    disk.image()
}

/// Everything the tests do with a volume, ignoring errors.
fn exercise(image: Vec<u8>) {
    let disk = MemDisk::new(image, 512);
    let Ok(fs) = Ext2::mount(&disk, Ticker::new(), MountOptions::default()) else { return };
    let mut budget = 400;
    let mut nodes: Vec<Ino> = Vec::new();
    let mut stack = vec![(fs.root(), 0)];
    while let Some((dir, depth)) = stack.pop() {
        let Ok(entries) = list(&fs, dir) else { continue };
        for (name, ino, kind) in entries {
            if budget == 0 {
                break;
            }
            budget -= 1;
            let _ = fs.lookup(dir, &name);
            let _ = fs.getattr(ino);
            nodes.push(ino);
            match kind {
                FileType::Directory if depth < 6 => stack.push((ino, depth + 1)),
                FileType::Symlink => {
                    let _ = fs.readlink(ino);
                }
                FileType::Regular => {
                    let mut buf = vec![0u8; 70_000];
                    let _ = fs.read(ino, 0, &mut buf);
                    let _ = fs.read(ino, 1 << 20, &mut buf[..10]);
                }
                _ => {}
            }
        }
    }
    let r = fs.root();
    let reg = NewNode { kind: FileType::Regular, perm: 0o644, uid: 0, gid: 0, rdev: DeviceId::default() };
    if let Ok(f) = fs.create(r, b"new-file", &reg) {
        let _ = fs.write(f, 0, &pattern(1, 100_000));
        let _ = fs.write(f, 5 << 20, b"x");
        let _ = fs.truncate(f, 3000);
    }
    let _ = fs.mkdir(r, b"new-dir", 0o755, 0, 0);
    let _ = fs.rename(r, b"big", r, b"dir", RenameFlags::default());
    let _ = fs.rename(r, b"dir", r, b"renamed", RenameFlags::default());
    for &ino in nodes.iter().take(30) {
        let _ = fs.write(ino, 10, b"overwrite");
        let _ = fs.truncate(ino, 1);
        let _ = fs.open(ino);
        fs.release(ino);
    }
    for name in [&b"big"[..], b"sparse", b"fast", b"slow", b"dev", b"new-file"] {
        let _ = fs.unlink(r, name);
    }
    let _ = fs.rmdir(r, b"new-dir");
    let _ = fs.rmdir(r, b"renamed");
    let _ = fs.sync();
    let _ = fs.unmount();
}

#[test]
fn corrupted_volumes_never_panic() {
    for bs in [1024u32, 4096] {
        let clean = populated(bs);
        // The fixed metadata of group 0 and the used part of the volume.
        let meta_end = 40 * bs as u64;
        let used_end = clean.iter().rposition(|&b| b != 0).unwrap_or(0) as u64 + 1;
        let iterations = if bs == 1024 { 300 } else { 150 };
        for iter in 0..iterations {
            let seed = bs as u64 * 1000 + iter;
            let mut rng = Rng::new(seed);
            let mut img = clean.clone();
            for _ in 0..1 + rng.below(8) {
                let at = match rng.below(10) {
                    0..=2 => 1024 + rng.below(1024 + bs as u64),
                    3..=6 => rng.below(meta_end),
                    _ => rng.below(used_end),
                } as usize;
                if rng.chance(50) {
                    img[at] ^= 1 << rng.below(8);
                } else {
                    img[at] = rng.next() as u8;
                }
            }
            if let Err(e) = catch_unwind(AssertUnwindSafe(|| exercise(img))) {
                panic!("corrupted volume (seed {seed}) made the driver panic: {e:?}");
            }
        }
    }
}

#[test]
fn truncated_and_garbage_devices() {
    let clean = populated(1024);
    for len in [0usize, 512, 1536, 2048, 4096, 100 << 10, 1 << 20] {
        let disk = MemDisk::new(clean[..len].to_vec(), 512);
        assert!(Ext2::mount(&disk, Ticker::new(), MountOptions::default()).is_err(), "{len} bytes");
    }
    let mut rng = Rng::new(9);
    for _ in 0..50 {
        let mut img: Vec<u8> = (0..1 << 20).map(|_| rng.next() as u8).collect();
        img[1024 + 56] = 0x53;
        img[1024 + 57] = 0xef;
        exercise(img);
    }
}

// ----- crashes -----

/// Applies the recorded writes to `base`: everything before flush number
/// `flush`, then each later write before the next flush with probability
/// `keep` percent.
fn crash_image(base: &[u8], events: &[Event], flush: usize, rng: &mut Rng, keep: u64) -> Vec<u8> {
    let mut img = base.to_vec();
    let mut flushes = 0;
    for e in events {
        match e {
            Event::Flush => {
                flushes += 1;
                if flushes > flush {
                    break;
                }
            }
            Event::Write(at, data) => {
                if flushes < flush || rng.chance(keep) {
                    img[*at as usize..*at as usize + data.len()].copy_from_slice(data);
                }
            }
        }
    }
    img
}

#[test]
fn crashes_leave_repairable_volumes_1k() {
    crashes(1024, 6 << 20, 1);
}

#[test]
fn crashes_leave_repairable_volumes_4k() {
    crashes(4096, 12 << 20, 2);
}

fn crashes(bs: u32, size: usize, seed: u64) {
    let Some(t) = tools() else {
        eprintln!("e2fsprogs not installed: skipping crash consistency (needs e2fsck)");
        return;
    };
    let dir = TempDir::new(&format!("crash-{bs}"));
    let reg = NewNode { kind: FileType::Regular, perm: 0o644, uid: 0, gid: 0, rdev: DeviceId::default() };
    // The starting volume: three files that are always renamed over.
    let base = {
        let disk = MemDisk::new(vec![0; size], 512);
        poly_ext2::format(&disk, &FormatOptions { block_size: bs, ..FormatOptions::default() }).unwrap();
        let fs = Ext2::mount(&disk, Ticker::new(), MountOptions::default()).unwrap();
        let d = fs.mkdir(fs.root(), b"keep", 0o755, 0, 0).unwrap();
        for i in 0..3 {
            let f = fs.create(d, format!("k{i}").as_bytes(), &reg).unwrap();
            fs.write(f, 0, &pattern(i, 20_000)).unwrap();
        }
        fs.unmount().unwrap();
        drop(fs);
        disk.image()
    };
    let disk = MemDisk::recording(base.clone(), 512);
    {
        let fs = Ext2::mount(&disk, Ticker::new(), MountOptions::default()).unwrap();
        let r = fs.root();
        let keep = fs.lookup(r, b"keep").unwrap();
        let mut rng = Rng::new(42 + seed);
        for round in 0..40u64 {
            // Replace a kept file by renaming a new one over it.
            let tmp = fs.create(keep, b"tmp", &reg).unwrap();
            fs.write(tmp, 0, &pattern(round, 1000 + rng.below(300_000) as usize)).unwrap();
            fs.rename(keep, b"tmp", keep, format!("k{}", round % 3).as_bytes(), RenameFlags::default()).unwrap();
            // Move one between directories and back.
            let sub = fs
                .mkdir(r, format!("d{round}").as_bytes(), 0o755, 0, 0)
                .unwrap_or_else(|_| fs.lookup(r, b"d0").unwrap());
            fs.rename(keep, b"k0", sub, b"k0", RenameFlags::default()).unwrap();
            fs.rename(sub, b"k0", keep, b"k0", RenameFlags::default()).unwrap();
            // Grow and shrink files so blocks are freed and reused.
            let f =
                fs.create(sub, format!("f{round}").as_bytes(), &reg).unwrap_or_else(|_| fs.lookup(sub, b"f0").unwrap());
            fs.write(f, 0, &pattern(round + 100, rng.below(400_000) as usize)).unwrap();
            fs.truncate(f, rng.below(5000)).unwrap();
            if round % 4 == 3 {
                let old = format!("d{}", round - 2);
                if let Ok(od) = fs.lookup(r, old.as_bytes()) {
                    for (name, _, _) in list(&fs, od).unwrap() {
                        fs.unlink(od, &name).unwrap();
                    }
                    fs.rmdir(r, old.as_bytes()).unwrap();
                }
            }
            if round % 10 == 9 {
                fs.sync().unwrap();
            }
        }
        // Crash: no unmount.
        let _ = fs.into_device_unsynced();
    }
    let events = disk.events();
    let flushes = events.iter().filter(|e| matches!(e, Event::Flush)).count();
    assert!(flushes > 40, "the workload took barriers ({flushes} flushes)");
    let mut rng = Rng::new(7 + seed);
    let points: Vec<usize> = (0..36).map(|i| if i < 4 { i } else { rng.below(flushes as u64 + 1) as usize }).collect();
    for (n, &flush) in points.iter().enumerate() {
        let keep = [0, 50, 100][n % 3];
        let mut img = crash_image(&base, &events, flush, &mut rng, keep);
        let (code, out) = t.fsck(&dir, &mut img, "-fy");
        assert!(code <= 1, "crash at flush {flush} (keeping {keep}% after it): e2fsck -fy exit {code}:\n{out}");
        assert!(!out.contains("ultiply-claimed"), "crash at flush {flush}: blocks shared:\n{out}");
        assert!(!out.contains("llegal block"), "crash at flush {flush}: garbage pointers:\n{out}");
        t.assert_clean(&dir, &img, &format!("repairing a crash at flush {flush}"));
        let disk = MemDisk::new(img, 512);
        let fs = Ext2::mount(&disk, Ticker::new(), MountOptions::default()).unwrap();
        let keep_dir = resolve(&fs, "keep").unwrap();
        for i in 1..3 {
            let name = format!("k{i}");
            assert!(fs.lookup(keep_dir, name.as_bytes()).is_ok(), "crash at flush {flush}: keep/{name} lost");
        }
        // k0 travels to a d* directory and back.
        let k0 = fs.lookup(keep_dir, b"k0").is_ok()
            || list(&fs, fs.root())
                .unwrap()
                .iter()
                .any(|(_, ino, kind)| *kind == FileType::Directory && fs.lookup(*ino, b"k0").is_ok());
        assert!(k0, "crash at flush {flush}: k0 lost:\n{out}\n{:?}", tree(&fs).keys().collect::<Vec<_>>());
        assert!(!fs.stats().errors);
        let mut budget = usize::MAX;
        walk(&fs, fs.root(), "", &mut BTreeMap::new(), &mut budget).unwrap();
    }
}

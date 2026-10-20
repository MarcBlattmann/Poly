//! The jbd2 journal: logs written by debugfs replay as e2fsck replays them
//! (descriptors, revoke records, checksum v3, 64-bit tags, more than one
//! descriptor per transaction), and crash consistency of our own
//! journaling: a workload on a disk that records its writes and flushes,
//! cut at random points (everything before some flush, any subset of the
//! writes after it), must give volumes that e2fsck finds clean after
//! replaying the journal, whether e2fsck or this driver replays it, with
//! the orphan list processed and no renamed-over name lost. Also a model
//! test against tmpfs on journaled volumes, and the orphan list across a
//! crash.

mod common;

use common::*;
use poly_ext2::{Ext2, FormatOptions, MountOptions};
use poly_vfs::{DeviceId, Errno, FileSystem, FileType, NewNode, RenameFlags, Tmpfs};
use std::collections::BTreeMap;

const REG: NewNode = NewNode { kind: FileType::Regular, perm: 0o644, uid: 0, gid: 0, rdev: DeviceId::new(0, 0) };

fn mount(disk: &MemDisk) -> Ext2<&MemDisk> {
    Ext2::mount(disk, Ticker::new(), MountOptions::default()).unwrap()
}

/// The physical blocks of a file, from `debugfs blocks`.
fn file_blocks(t: &Tools, dir: &TempDir, image: &[u8], path: &str) -> Vec<u64> {
    let mut img = image.to_vec();
    let out = t.debugfs(dir, &mut img, &format!("blocks {path}\n"));
    let numbers = |l: &str| l.split_whitespace().filter_map(|w| w.parse().ok()).collect::<Vec<u64>>();
    out.lines().map(numbers).max_by_key(|n| n.len()).unwrap_or_default()
}

#[test]
fn replays_journals_written_by_debugfs() {
    let Some(t) = tools() else {
        eprintln!("e2fsprogs not installed: skipping replay of debugfs journals");
        return;
    };
    for (name, mkfs, jo) in [
        ("csum-v3", &[][..], "jo -c"),
        ("no-csum", &["-O", "^metadata_csum"][..], "jo"),
        ("1k-32bit", &["-b", "1024", "-O", "^64bit"][..], "jo -c"),
    ] {
        let dir = TempDir::new(&format!("replay-{name}"));
        let root = dir.path("tree");
        std::fs::create_dir_all(&root).unwrap();
        std::fs::write(root.join("file"), pattern(1, 400 * 4096)).unwrap();
        let rootarg = root.display().to_string();
        let mut args = mkfs.to_vec();
        args.extend(["-d", &rootarg]);
        let mut image = t.mkfs_type(&dir, 32 << 20, "ext4", &args);
        let blocks = file_blocks(&t, &dir, &image, "/file");
        let bs = if mkfs.contains(&"1024") { 1024 } else { 4096 };
        assert!(blocks.len() >= 300, "{name}: {blocks:?}");
        // Transaction 1: 300 blocks (two descriptor blocks with 4 KiB
        // blocks, more with 1 KiB). Transaction 2: revokes one of them.
        // Transaction 3: logs another, which replays (after the revoke).
        let new = dir.path("new");
        std::fs::write(&new, pattern(2, 300 * bs)).unwrap();
        let list = |b: &[u64]| b.iter().map(|b| b.to_string()).collect::<Vec<_>>().join(",");
        let cmds = format!(
            "{jo}\njw -b {} {}\njw -r {}\njw -b {} {}\njc\n",
            list(&blocks[..300]),
            new.display(),
            blocks[7],
            blocks[7],
            dir.path("third").display(),
        );
        std::fs::write(dir.path("third"), pattern(3, bs)).unwrap();
        let out = t.debugfs(&dir, &mut image, &cmds);
        assert!(!out.contains("rror"), "{name}: {out}");
        let mut logdump = image.clone();
        let dump = t.debugfs(&dir, &mut logdump, "logdump\n");
        assert!(dump.contains("revoke"), "{name}: {dump}");
        // e2fsck's replay.
        let mut theirs = image.clone();
        let (code, out) = t.fsck(&dir, &mut theirs, "-fy");
        assert!(code <= 1, "{name}: e2fsck -fy: {out}");
        // Ours.
        let disk = MemDisk::new(image.clone(), 512);
        let fs = mount(&disk);
        assert_eq!(fs.stats().replayed, 3, "{name}");
        let f = resolve(&fs, "file").unwrap();
        let data = read_all(&fs, f).unwrap();
        let mut want = pattern(1, 400 * 4096);
        let new_data = pattern(2, 300 * bs);
        for i in 0..300 {
            want[i * bs..(i + 1) * bs].copy_from_slice(&new_data[i * bs..(i + 1) * bs]);
        }
        want[7 * bs..8 * bs].copy_from_slice(&pattern(3, bs));
        assert!(data == want, "{name}: the replayed file");
        fs.unmount().unwrap();
        drop(fs);
        t.assert_clean(&dir, &disk.image(), &format!("our replay of {name}"));
        // The same blocks as e2fsck's replay wrote.
        let mine = disk.image();
        for &b in &blocks {
            let r = (b as usize * bs)..(b as usize + 1) * bs;
            assert!(mine[r.clone()] == theirs[r], "{name}: block {b} differs from e2fsck's replay");
        }
    }
}

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

fn base_volume(t: &Option<Tools>, dir: &TempDir, bs: u32, size: usize, mke2fs: bool) -> Vec<u8> {
    let image = match t {
        Some(t) if mke2fs => t.mkfs_type(dir, size as u64, "ext4", &["-b", &bs.to_string()]),
        _ => {
            let disk = MemDisk::new(vec![0; size], 512);
            poly_ext2::format(&disk, &FormatOptions::ext4(bs, 1024)).unwrap();
            disk.image()
        }
    };
    let disk = MemDisk::new(image, 512);
    let fs = mount(&disk);
    let d = fs.mkdir(fs.root(), b"keep", 0o755, 0, 0).unwrap();
    for i in 0..3 {
        let f = fs.create(d, format!("k{i}").as_bytes(), &REG).unwrap();
        fs.write(f, 0, &pattern(i, 20_000)).unwrap();
    }
    fs.unmount().unwrap();
    drop(fs);
    disk.image()
}

/// What a crashed volume must still have: the kept names (k0 may be in a
/// d* directory), and a tree that walks without errors.
fn check_survivors(fs: &Ext2<&MemDisk>, what: &str) {
    let keep_dir = resolve(fs, "keep").unwrap_or_else(|e| panic!("{what}: keep: {e:?}"));
    for i in 1..3 {
        assert!(fs.lookup(keep_dir, format!("k{i}").as_bytes()).is_ok(), "{what}: keep/k{i} lost");
    }
    let k0 = fs.lookup(keep_dir, b"k0").is_ok()
        || list(fs, fs.root())
            .unwrap()
            .iter()
            .any(|(_, ino, kind)| *kind == FileType::Directory && fs.lookup(*ino, b"k0").is_ok());
    assert!(k0, "{what}: k0 lost");
    let mut budget = usize::MAX;
    walk(fs, fs.root(), "", &mut BTreeMap::new(), &mut budget).unwrap();
    assert!(!fs.stats().errors, "{what}: the driver found corruption");
}

fn crashes(bs: u32, size: usize, seed: u64, mke2fs: bool) {
    let t = tools();
    if t.is_none() {
        eprintln!("e2fsprogs not installed: crash images checked by remounting only");
    }
    let dir = TempDir::new(&format!("jcrash-{bs}-{seed}"));
    let base = base_volume(&t, &dir, bs, size, mke2fs);
    let disk = MemDisk::recording(base.clone(), 512);
    {
        let fs = mount(&disk);
        let r = fs.root();
        let keep = fs.lookup(r, b"keep").unwrap();
        let mut rng = Rng::new(42 + seed);
        let mut open = Vec::new();
        for round in 0..40u64 {
            // Replace a kept file by renaming a new one over it.
            let tmp = fs.create(keep, b"tmp", &REG).unwrap();
            fs.write(tmp, 0, &pattern(round, 1000 + rng.below(300_000) as usize)).unwrap();
            fs.rename(keep, b"tmp", keep, format!("k{}", round % 3).as_bytes(), RenameFlags::default()).unwrap();
            let sub = fs
                .mkdir(r, format!("d{round}").as_bytes(), 0o755, 0, 0)
                .unwrap_or_else(|_| fs.lookup(r, b"d0").unwrap());
            fs.rename(keep, b"k0", sub, b"k0", RenameFlags::default()).unwrap();
            fs.rename(sub, b"k0", keep, b"k0", RenameFlags::default()).unwrap();
            // Fragmented files grown and cut back.
            let f =
                fs.create(sub, format!("f{round}").as_bytes(), &REG).unwrap_or_else(|_| fs.lookup(sub, b"f0").unwrap());
            for i in 0..rng.below(200) {
                fs.write(f, i * 2 * bs as u64, &pattern(i, 100)).unwrap();
            }
            fs.write(f, 1 << 20, &pattern(round + 100, rng.below(400_000) as usize)).unwrap();
            fs.truncate(f, rng.below(50_000)).unwrap();
            // Files deleted while open: on the orphan list until released.
            let o = fs.create(sub, format!("o{round}").as_bytes(), &REG).unwrap();
            fs.write(o, 0, &pattern(round, 50_000)).unwrap();
            fs.open(o).unwrap();
            fs.unlink(sub, format!("o{round}").as_bytes()).unwrap();
            open.push(o);
            if open.len() > 3 {
                fs.release(open.remove(0));
            }
            if round % 4 == 3 {
                let old = format!("d{}", round - 2);
                if let Ok(od) = fs.lookup(r, old.as_bytes()) {
                    for (name, _, _) in list(&fs, od).unwrap() {
                        fs.unlink(od, &name).unwrap();
                    }
                    fs.rmdir(r, old.as_bytes()).unwrap();
                }
            }
            if rng.chance(30) {
                fs.sync().unwrap();
            }
        }
        assert!(fs.stats().commits > 5, "{} commits", fs.stats().commits);
        // Crash: no unmount.
        let _ = fs.into_device_unsynced();
    }
    let events = disk.events();
    let flushes = events.iter().filter(|e| matches!(e, Event::Flush)).count();
    assert!(flushes > 20, "the workload committed ({flushes} flushes)");
    let mut rng = Rng::new(7 + seed);
    let points: Vec<usize> = (0..30).map(|i| if i < 3 { i } else { rng.below(flushes as u64 + 1) as usize }).collect();
    let mut replayed_some = false;
    for (n, &flush) in points.iter().enumerate() {
        let keep = [0, 50, 100][n % 3];
        let img = crash_image(&base, &events, flush, &mut rng, keep);
        let what = format!("crash at flush {flush} of {flushes} (keeping {keep}% after it)");
        // e2fsck replays the journal, then finds nothing more to fix.
        if let Some(t) = &t {
            let mut theirs = img.clone();
            let (code, out) = t.fsck(&dir, &mut theirs, "-fy");
            assert!(code <= 1, "{what}: e2fsck -fy exit {code}:\n{out}");
            for bad in ["ultiply-claimed", "llegal block", "has deleted/unused inode", "nlinked"] {
                assert!(!out.contains(bad), "{what}: e2fsck -fy found {bad}:\n{out}");
            }
            t.assert_clean(&dir, &theirs, &format!("e2fsck's replay after the {what}"));
        }
        // So does ours (and our orphan processing).
        let disk = MemDisk::new(img, 512);
        let fs = mount(&disk);
        replayed_some |= fs.stats().replayed > 0;
        check_survivors(&fs, &what);
        fs.unmount().unwrap();
        drop(fs);
        if let Some(t) = &t {
            t.assert_clean(&dir, &disk.image(), &format!("our replay after the {what}"));
        }
    }
    assert!(replayed_some, "no crash point needed a replay");
}

#[test]
fn crashes_replay_clean_4k() {
    crashes(4096, 32 << 20, 1, false);
}

#[test]
fn crashes_replay_clean_1k() {
    crashes(1024, 16 << 20, 2, false);
}

#[test]
fn crashes_replay_clean_on_mke2fs_volumes() {
    crashes(4096, 32 << 20, 3, true);
}

#[test]
fn orphans_survive_a_crash_and_are_freed_at_mount() {
    let t = tools();
    let dir = TempDir::new("orphans");
    let disk = MemDisk::new(vec![0; 16 << 20], 512);
    poly_ext2::format(&disk, &FormatOptions::ext4(4096, 1024)).unwrap();
    let before = {
        let fs = mount(&disk);
        let s = fs.stats();
        let r = fs.root();
        for i in 0..3 {
            let f = fs.create(r, format!("o{i}").as_bytes(), &REG).unwrap();
            fs.write(f, 0, &pattern(i, 300_000)).unwrap();
            fs.open(f).unwrap();
            fs.unlink(r, format!("o{i}").as_bytes()).unwrap();
        }
        // Still readable while open.
        fs.sync().unwrap();
        s
    };
    // The crash left three inodes with no names on the orphan list.
    let crashed = disk.image();
    if let Some(t) = &t {
        let mut img = crashed.clone();
        let (code, out) = t.fsck(&dir, &mut img, "-fy");
        assert!(code <= 1, "{out}");
        t.assert_clean(&dir, &img, "e2fsck's orphan processing");
    }
    let disk = MemDisk::new(crashed, 512);
    let fs = mount(&disk);
    assert_eq!(fs.stats().orphans, 3);
    fs.sync().unwrap();
    let after = fs.stats();
    assert_eq!((after.free_blocks, after.free_inodes), (before.free_blocks, before.free_inodes));
    fs.unmount().unwrap();
    drop(fs);
    if let Some(t) = &t {
        t.assert_clean(&dir, &disk.image(), "our orphan processing");
    }
}

#[test]
fn truncations_bigger_than_a_transaction() {
    // Transactions of at most 16 blocks and a file with thousands of
    // extents over dozens of groups: truncating it takes several
    // transactions, the file on the orphan list in between.
    let t = tools();
    let dir = TempDir::new("big-truncate");
    let disk = MemDisk::recording(vec![0; 64 << 20], 512);
    // Small groups: the file's blocks spread over many bitmaps.
    let fmt = FormatOptions { blocks_per_group: Some(2048), bytes_per_inode: 65536, ..FormatOptions::ext4(1024, 1024) };
    poly_ext2::format(&disk, &fmt).unwrap();
    let base = disk.image();
    let fs = mount(&disk);
    fs.limit_transactions(16);
    let f = fs.create(fs.root(), b"f", &REG).unwrap();
    for i in 0..20_000u64 {
        fs.write(f, i * 2048, b"x").unwrap();
    }
    fs.sync().unwrap();
    let commits = fs.stats().commits;
    let flushes_before = disk.events().iter().filter(|e| matches!(e, Event::Flush)).count();
    fs.truncate(f, 10).unwrap();
    let took = fs.stats().commits - commits;
    assert!(took >= 1, "the truncation committed on the way ({took})");
    fs.sync().unwrap();
    drop(fs);
    let events = disk.events();
    let flushes = events.iter().filter(|e| matches!(e, Event::Flush)).count();
    let mut rng = Rng::new(3);
    // Crashes at every flush from the last commit before the truncation on.
    let mut finished_at_mount = 0;
    for (n, flush) in (flushes_before..=flushes).enumerate() {
        let img = crash_image(&base, &events, flush, &mut rng, 50);
        let disk = MemDisk::new(img, 512);
        let fs = mount(&disk);
        let f = resolve(&fs, "f").unwrap();
        let a = fs.getattr(f).unwrap();
        assert!(a.size == 10 || a.size == 19_999 * 2048 + 1, "crash {n}: size {}", a.size);
        if fs.stats().orphans > 0 {
            finished_at_mount += 1;
            assert_eq!(a.size, 10);
        }
        fs.unmount().unwrap();
        drop(fs);
        if let Some(t) = &t {
            t.assert_clean(&dir, &disk.image(), &format!("a crash during a long truncation ({n})"));
        }
    }
    assert!(finished_at_mount > 0, "no crash fell between the truncation's transactions");
}

#[test]
fn model_against_tmpfs_on_journaled_volumes() {
    let t = tools();
    for (bs, seed) in [(1024u32, 11u64), (4096, 12)] {
        let disk = MemDisk::new(vec![0; 48 << 20], 512);
        poly_ext2::format(&disk, &FormatOptions::ext4(bs, 1024)).unwrap();
        let model = Tmpfs::new(Ticker::new());
        let mut fs = Some(mount(&disk));
        let mut rng = Rng::new(seed);
        let names = ["a", "b", "c", "long-name-number-one"];
        for n in 0..3000 {
            let name = names[rng.below(4) as usize];
            let op = rng.below(10);
            let size = rng.below(200_000) as usize;
            let at = rng.below(1 << 20);
            let run = |f: &dyn FileSystem| -> Result<String, Errno> {
                let r = f.root();
                match op {
                    0..=2 => f.create(r, name.as_bytes(), &REG).map(|_| String::new()),
                    3 => f.unlink(r, name.as_bytes()).map(|_| String::new()),
                    4..=6 => {
                        let ino = f.lookup(r, name.as_bytes())?;
                        f.write(ino, at, &pattern(n, size)).map(|n| n.to_string())
                    }
                    7 => {
                        let ino = f.lookup(r, name.as_bytes())?;
                        f.truncate(ino, at / 2).map(|_| String::new())
                    }
                    8 => {
                        let ino = f.lookup(r, name.as_bytes())?;
                        read_all(f, ino).map(|d| format!("{} {}", d.len(), d.iter().map(|&b| b as u64).sum::<u64>()))
                    }
                    _ => f
                        .rename(
                            r,
                            name.as_bytes(),
                            r,
                            names[(op as usize + n as usize) % 4].as_bytes(),
                            RenameFlags::default(),
                        )
                        .map(|_| String::new()),
                }
            };
            let want = run(&model);
            let got = run(fs.as_ref().unwrap());
            assert_eq!(got, want, "bs {bs} op {n}");
            if n % 500 == 499 {
                fs.take().unwrap().unmount().unwrap();
                fs = Some(mount(&disk));
            } else if n % 97 == 0 {
                fs.as_ref().unwrap().sync().unwrap();
            }
        }
        let f = fs.take().unwrap();
        let mut got = tree(&f);
        got.remove("/lost+found");
        assert_eq!(got, tree(&model), "bs {bs}");
        f.unmount().unwrap();
        drop(f);
        if let Some(t) = &t {
            let dir = TempDir::new(&format!("jmodel-{bs}"));
            t.assert_clean(&dir, &disk.image(), &format!("the model run with {bs}-byte blocks"));
        }
    }
}

extern crate std;

use super::*;
use poly_vfs::{Clock, Timespec, Tmpfs};
use std::vec::Vec;

struct Tick;
impl Clock for Tick {
    fn now(&self) -> Timespec {
        Timespec::new(100, 0)
    }
}

fn layer() -> (Arc<Tmpfs>, Layer) {
    let fs = Arc::new(Tmpfs::new(Arc::new(Tick)));
    let l = Layer { fs: fs.clone(), root: fs.root() };
    (fs, l)
}

fn file(fs: &Tmpfs, dir: Ino, name: &str, data: &[u8]) -> Ino {
    let i = fs
        .create(
            dir,
            name.as_bytes(),
            &NewNode { kind: FileType::Regular, perm: 0o644, uid: 5, gid: 6, rdev: DeviceId::default() },
        )
        .unwrap();
    fs.write(i, 0, data).unwrap();
    i
}

fn dir(fs: &Tmpfs, parent: Ino, name: &str) -> Ino {
    fs.mkdir(parent, name.as_bytes(), 0o750, 7, 8).unwrap()
}

struct World {
    upper: Arc<Tmpfs>,
    lower1: Arc<Tmpfs>,
    lower2: Arc<Tmpfs>,
    ov: Overlay,
}

fn world() -> World {
    let (u, ul) = layer();
    let (l1, l1l) = layer();
    let (l2, l2l) = layer();
    file(&l1, l1.root(), "a", b"lower one a");
    file(&l1, l1.root(), "both", b"from one");
    let d1 = dir(&l1, l1.root(), "d");
    file(&l1, d1, "x", b"x one");
    file(&l2, l2.root(), "b", b"lower two b");
    file(&l2, l2.root(), "both", b"from two");
    let d2 = dir(&l2, l2.root(), "d");
    file(&l2, d2, "y", b"y two");
    dir(&l2, l2.root(), "only2");
    let ov =
        Overlay::new(Config { upper: Some(ul), lowers: vec![l1l, l2l], userxattr: false, redirect: Redirect::Off })
            .unwrap();
    World { upper: u, lower1: l1, lower2: l2, ov }
}

fn path(ov: &Overlay, p: &str) -> Result<Ino> {
    let mut i = ov.root();
    for part in p.split('/').filter(|s| !s.is_empty()) {
        i = ov.lookup(i, part.as_bytes())?;
    }
    Ok(i)
}

fn read_all(ov: &Overlay, i: Ino) -> Vec<u8> {
    let mut v = vec![0u8; 4096];
    let n = ov.read(i, 0, &mut v).unwrap();
    v.truncate(n);
    v
}

fn names(ov: &Overlay, d: Ino) -> Vec<std::string::String> {
    let mut v = Vec::new();
    ov.readdir(d, 0, &mut |e| {
        if e.name != b"." && e.name != b".." {
            v.push(std::string::String::from_utf8_lossy(e.name).into_owned());
        }
        true
    })
    .unwrap();
    v.sort();
    v
}

#[test]
fn merged_view() {
    let w = world();
    assert_eq!(names(&w.ov, w.ov.root()), ["a", "b", "both", "d", "only2"]);
    assert_eq!(read_all(&w.ov, path(&w.ov, "both").unwrap()), b"from one");
    assert_eq!(names(&w.ov, path(&w.ov, "d").unwrap()), ["x", "y"]);
    assert_eq!(read_all(&w.ov, path(&w.ov, "d/y").unwrap()), b"y two");
    assert_eq!(w.ov.lookup(w.ov.root(), b"nope"), Err(Errno::ENOENT));
    assert_eq!(w.ov.statfs().unwrap().magic, 0x794c_7630);
    assert_eq!(w.ov.getattr(path(&w.ov, "d").unwrap()).unwrap().perm, 0o750);
}

#[test]
fn write_copies_up_and_leaves_the_lower_alone() {
    let w = world();
    let a = path(&w.ov, "d/x").unwrap();
    w.ov.prepare_write(a).unwrap();
    // The copy exists at once, with the old content, mode and owner.
    let ud = w.upper.lookup(w.upper.root(), b"d").unwrap();
    let ux = w.upper.lookup(ud, b"x").unwrap();
    let at = w.upper.getattr(ux).unwrap();
    assert_eq!((at.perm, at.uid, at.gid, at.size), (0o644, 5, 6, 5));
    assert_eq!(w.upper.getattr(ud).unwrap().perm, 0o750);
    assert_eq!(w.ov.write(a, 0, b"NEW"), Ok(3));
    assert_eq!(read_all(&w.ov, a), b"NEWne");
    let l1d = w.lower1.lookup(w.lower1.root(), b"d").unwrap();
    let l1x = w.lower1.lookup(l1d, b"x").unwrap();
    let mut b = [0u8; 16];
    assert_eq!(w.lower1.read(l1x, 0, &mut b), Ok(5));
    assert_eq!(&b[..5], b"x one");
    // Siblings were not copied.
    assert_eq!(w.upper.lookup(ud, b"y"), Err(Errno::ENOENT));
}

#[test]
fn unlink_leaves_a_whiteout_and_create_replaces_it() {
    let w = world();
    let root = w.ov.root();
    w.ov.unlink(root, b"a").unwrap();
    assert_eq!(w.ov.lookup(root, b"a"), Err(Errno::ENOENT));
    let wh = w.upper.lookup(w.upper.root(), b"a").unwrap();
    let at = w.upper.getattr(wh).unwrap();
    assert_eq!((at.kind, at.rdev, at.perm), (FileType::CharDevice, DeviceId::new(0, 0), 0));
    assert!(!names(&w.ov, root).contains(&"a".into()));
    assert_eq!(w.ov.unlink(root, b"a"), Err(Errno::ENOENT));
    // A name in two lowers is hidden in both.
    w.ov.unlink(root, b"both").unwrap();
    assert_eq!(w.ov.lookup(root, b"both"), Err(Errno::ENOENT));
    // Creating the name again.
    let n =
        w.ov.create(
            root,
            b"a",
            &NewNode { kind: FileType::Regular, perm: 0o600, uid: 1, gid: 2, rdev: DeviceId::default() },
        )
        .unwrap();
    w.ov.write(n, 0, b"fresh").unwrap();
    assert_eq!(read_all(&w.ov, path(&w.ov, "a").unwrap()), b"fresh");
    // A name only the upper layer has leaves nothing.
    w.ov.create(
        root,
        b"u",
        &NewNode { kind: FileType::Regular, perm: 0o600, uid: 1, gid: 2, rdev: DeviceId::default() },
    )
    .unwrap();
    w.ov.unlink(root, b"u").unwrap();
    assert_eq!(w.upper.lookup(w.upper.root(), b"u"), Err(Errno::ENOENT));
}

#[test]
fn rmdir_and_opaque_directories() {
    let w = world();
    let root = w.ov.root();
    assert_eq!(w.ov.rmdir(root, b"d"), Err(Errno::ENOTEMPTY));
    assert_eq!(w.ov.rmdir(root, b"a"), Err(Errno::ENOTDIR));
    assert_eq!(w.ov.unlink(root, b"d"), Err(Errno::EISDIR));
    let d = path(&w.ov, "d").unwrap();
    w.ov.unlink(d, b"x").unwrap();
    w.ov.unlink(d, b"y").unwrap();
    w.ov.rmdir(root, b"d").unwrap();
    assert_eq!(w.ov.lookup(root, b"d"), Err(Errno::ENOENT));
    let wh = w.upper.lookup(w.upper.root(), b"d").unwrap();
    assert_eq!(w.upper.getattr(wh).unwrap().kind, FileType::CharDevice);
    // A directory made anew over the whiteout shows nothing of the lowers.
    let nd = w.ov.mkdir(root, b"d", 0o700, 0, 0).unwrap();
    assert_eq!(names(&w.ov, nd), Vec::<std::string::String>::new());
    let ud = w.upper.lookup(w.upper.root(), b"d").unwrap();
    assert_eq!(w.upper.getxattr(ud, b"trusted.overlay.opaque").unwrap(), b"y");
    assert_eq!(w.ov.getxattr(nd, b"trusted.overlay.opaque"), Err(Errno::ENODATA));
    assert_eq!(w.ov.listxattr(nd).unwrap(), b"");
}

#[test]
fn premade_whiteouts_and_opaque_lowers() {
    let w = world();
    // A whiteout in the upper layer hides both lowers.
    w.upper.create(w.upper.root(), b"b", &super::whiteout_node()).unwrap();
    assert_eq!(w.ov.lookup(w.ov.root(), b"b"), Err(Errno::ENOENT));
    // An opaque directory in the first lower hides the second lower's entries.
    let l1d = w.lower1.lookup(w.lower1.root(), b"d").unwrap();
    w.lower1.setxattr(l1d, b"trusted.overlay.opaque", b"y", SetMode::default()).unwrap();
    let ov = Overlay::<RawSpinLock>::new(Config {
        upper: Some(Layer { fs: w.upper.clone(), root: w.upper.root() }),
        lowers: vec![
            Layer { fs: w.lower1.clone(), root: w.lower1.root() },
            Layer { fs: w.lower2.clone(), root: w.lower2.root() },
        ],
        userxattr: false,
        redirect: Redirect::Off,
    })
    .unwrap();
    assert_eq!(names(&ov, path(&ov, "d").unwrap()), ["x"]);
}

#[test]
fn rename_files_and_directories() {
    let w = world();
    let root = w.ov.root();
    w.ov.rename(root, b"a", root, b"moved", RenameFlags::default()).unwrap();
    assert_eq!(w.ov.lookup(root, b"a"), Err(Errno::ENOENT));
    assert_eq!(read_all(&w.ov, path(&w.ov, "moved").unwrap()), b"lower one a");
    let wh = w.upper.lookup(w.upper.root(), b"a").unwrap();
    assert_eq!(w.upper.getattr(wh).unwrap().kind, FileType::CharDevice);
    let l1a = w.lower1.lookup(w.lower1.root(), b"a").unwrap();
    assert_eq!(w.lower1.getattr(l1a).unwrap().size, 11);
    // NOREPLACE onto a lower name.
    assert_eq!(
        w.ov.rename(root, b"moved", root, b"b", RenameFlags { noreplace: true, exchange: false }),
        Err(Errno::EEXIST)
    );
    // A directory of a lower layer cannot move.
    assert_eq!(w.ov.rename(root, b"d", root, b"d2", RenameFlags::default()), Err(Errno::EXDEV));
    // An upper-only directory can.
    let nd = w.ov.mkdir(root, b"fresh", 0o755, 0, 0).unwrap();
    w.ov.create(nd, b"f", &NewNode { kind: FileType::Regular, perm: 0o644, uid: 0, gid: 0, rdev: DeviceId::default() })
        .unwrap();
    w.ov.rename(root, b"fresh", root, b"fresh2", RenameFlags::default()).unwrap();
    assert_eq!(names(&w.ov, path(&w.ov, "fresh2").unwrap()), ["f"]);
    assert_eq!(w.upper.lookup(w.upper.root(), b"fresh"), Err(Errno::ENOENT));
}

#[test]
fn readdir_positions_stay_valid_when_names_go() {
    let w = world();
    let root = w.ov.root();
    let mut seen = Vec::new();
    let mut pos = 0;
    loop {
        let mut one = None;
        w.ov.readdir(root, pos, &mut |e| {
            if e.name == b"." || e.name == b".." {
                pos = e.next;
                return true;
            }
            one = Some((e.name.to_vec(), e.next));
            false
        })
        .unwrap();
        let Some((name, next)) = one else { break };
        pos = next;
        seen.push(std::string::String::from_utf8_lossy(&name).into_owned());
        if name != b"d" && name != b"only2" {
            let _ = w.ov.unlink(root, &name);
        }
    }
    seen.sort();
    assert_eq!(seen, ["a", "b", "both", "d", "only2"]);
}

#[test]
fn read_only_overlay() {
    let (_, l1) = layer();
    let (_, l2) = layer();
    let ov = Overlay::<RawSpinLock>::new(Config {
        upper: None,
        lowers: vec![l1, l2],
        userxattr: false,
        redirect: Redirect::Off,
    })
    .unwrap();
    assert!(ov.info().read_only);
    assert_eq!(ov.mkdir(ov.root(), b"x", 0o755, 0, 0), Err(Errno::EROFS));
    assert_eq!(ov.unlink(ov.root(), b"x"), Err(Errno::EROFS));
}

#[test]
fn inside() {
    let (fs, l) = layer();
    let a = dir(&fs, l.root, "a");
    let b = dir(&fs, a, "b");
    assert!(is_inside(&*fs, b, a));
    assert!(is_inside(&*fs, a, a));
    assert!(!is_inside(&*fs, a, b));
}

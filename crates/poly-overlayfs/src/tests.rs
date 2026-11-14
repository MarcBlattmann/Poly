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

/// A mapping of a file of the overlay maps the file of the layer that
/// holds its data; a writable shared one copies the file up first.
#[test]
fn the_backing_file_is_the_layers_and_a_writable_mapping_copies_up() {
    let w = world();
    let same = |a: &Arc<dyn FileSystem>, b: &Arc<Tmpfs>| Arc::as_ptr(a) as *const u8 == Arc::as_ptr(b) as *const u8;
    let a = w.ov.lookup(w.ov.root(), b"a").unwrap();
    // Reading: the lower file, nothing copied.
    let (fs, ino) = w.ov.backing_file(a, false).unwrap().unwrap();
    assert!(same(&fs, &w.lower1));
    assert_eq!(ino, w.lower1.lookup(w.lower1.root(), b"a").unwrap());
    assert_eq!(w.upper.lookup(w.upper.root(), b"a"), Err(Errno::ENOENT));
    // Writing: the upper copy, made now, with the old content.
    let (fs, ino) = w.ov.backing_file(a, true).unwrap().unwrap();
    assert!(same(&fs, &w.upper));
    assert_eq!(ino, w.upper.lookup(w.upper.root(), b"a").unwrap());
    let mut buf = [0u8; 16];
    let n = w.upper.read(ino, 0, &mut buf).unwrap();
    assert_eq!(&buf[..n], b"lower one a");
    // What is written to that file is what the overlay reads; the lower
    // layer keeps its own.
    w.upper.write(ino, 6, b"ONE").unwrap();
    let n = w.ov.read(a, 0, &mut buf).unwrap();
    assert_eq!(&buf[..n], b"lower ONE a");
    let l = w.lower1.lookup(w.lower1.root(), b"a").unwrap();
    let n = w.lower1.read(l, 0, &mut buf).unwrap();
    assert_eq!(&buf[..n], b"lower one a");
    // From now on a reading mapping shows the upper copy too.
    let (fs, again) = w.ov.backing_file(a, false).unwrap().unwrap();
    assert!(same(&fs, &w.upper) && again == ino);
    // Not for directories; and no upper layer, no writable mapping.
    let d = w.ov.lookup(w.ov.root(), b"d").unwrap();
    assert!(w.ov.backing_file(d, false).unwrap().is_none());
    let (l1, l1l) = layer();
    let (_l2, l2l) = layer();
    file(&l1, l1.root(), "f", b"f");
    let ro: Overlay =
        Overlay::new(Config { upper: None, lowers: vec![l1l, l2l], userxattr: false, redirect: Redirect::Off })
            .unwrap();
    let f = ro.lookup(ro.root(), b"f").unwrap();
    assert!(ro.backing_file(f, false).unwrap().is_some());
    assert_eq!(ro.backing_file(f, true).err(), Some(Errno::EROFS));
}

// ----- redirect_dir -----

const REDIR: &[u8] = b"trusted.overlay.redirect";

struct Redir {
    upper: Arc<Tmpfs>,
    lower1: Arc<Tmpfs>,
    lower2: Arc<Tmpfs>,
    ov: Overlay,
}

fn redir_world(mode: Redirect) -> Redir {
    let (u, ul) = layer();
    let (l1, l1l) = layer();
    let (l2, l2l) = layer();
    let a = dir(&l1, l1.root(), "a");
    let b = dir(&l1, a, "b");
    let c = dir(&l1, b, "c");
    file(&l1, c, "g", b"gg");
    dir(&l1, a, "x");
    dir(&l1, l1.root(), "top");
    // The second lower layer has a directory of the first one's name too.
    let t2 = dir(&l2, l2.root(), "top");
    file(&l2, t2, "t2", b"t2");
    let ov =
        Overlay::new(Config { upper: Some(ul), lowers: vec![l1l, l2l], userxattr: false, redirect: mode }).unwrap();
    Redir { upper: u, lower1: l1, lower2: l2, ov }
}

fn fs_path(fs: &Tmpfs, p: &str) -> Ino {
    let mut i = fs.root();
    for part in p.split('/').filter(|s| !s.is_empty()) {
        i = fs.lookup(i, part.as_bytes()).unwrap();
    }
    i
}

fn redirect_of(fs: &Tmpfs, p: &str) -> Option<Vec<u8>> {
    fs.getxattr(fs_path(fs, p), REDIR).ok()
}

fn rn(w: &Redir, od: &str, on: &str, nd: &str, nn: &str) -> Result<()> {
    let (od, nd) = (path(&w.ov, od).unwrap(), path(&w.ov, nd).unwrap());
    w.ov.rename(od, on.as_bytes(), nd, nn.as_bytes(), RenameFlags::default())
}

#[test]
fn only_redirect_dir_on_renames_a_lower_directory() {
    for mode in [Redirect::Off, Redirect::Follow, Redirect::NoFollow] {
        let w = redir_world(mode);
        assert_eq!(rn(&w, "a/b", "c", "a/b", "c2"), Err(Errno::EXDEV));
        assert_eq!(w.upper.lookup(w.upper.root(), b"a"), Err(Errno::ENOENT));
        let root = w.ov.root();
        let free = RenameFlags { noreplace: true, exchange: false };
        assert_eq!(w.ov.rename(root, b"top", root, b"top2", free), Err(Errno::EXDEV));
        assert_eq!(w.ov.rename(root, b"top", root, b"a", free), Err(Errno::EEXIST));
        let ex = RenameFlags { noreplace: false, exchange: true };
        assert_eq!(w.ov.rename(root, b"top", root, b"a", ex), Err(Errno::EXDEV));
    }
    assert_eq!(Redirect::from_value(b"On"), Err(Errno::EINVAL));
    assert_eq!(Redirect::from_value(b""), Err(Errno::EINVAL));
    assert_eq!(Redirect::from_value(b"nofollow"), Ok(Redirect::NoFollow));
}

#[test]
fn a_redirect_rename_names_the_lower_directory() {
    let w = redir_world(Redirect::On);
    // The same parent: the old name alone.
    rn(&w, "a/b", "c", "a/b", "c2").unwrap();
    assert_eq!(redirect_of(&w.upper, "a/b/c2").unwrap(), b"c");
    assert_eq!(w.ov.lookup(path(&w.ov, "a/b").unwrap(), b"c"), Err(Errno::ENOENT));
    assert_eq!(names(&w.ov, path(&w.ov, "a/b").unwrap()), ["c2"]);
    assert_eq!(read_all(&w.ov, path(&w.ov, "a/b/c2/g").unwrap()), b"gg");
    let wh = fs_path(&w.upper, "a/b/c");
    assert_eq!(w.upper.getattr(wh).unwrap().kind, FileType::CharDevice);
    // The redirect is not shown.
    assert_eq!(w.ov.getxattr(path(&w.ov, "a/b/c2").unwrap(), REDIR), Err(Errno::ENODATA));
    // Another parent: the old path from the root, and it is kept.
    rn(&w, "a/b", "c2", "", "top1").unwrap();
    assert_eq!(redirect_of(&w.upper, "top1").unwrap(), b"/a/b/c");
    rn(&w, "", "top1", "a/x", "c3").unwrap();
    assert_eq!(redirect_of(&w.upper, "a/x/c3").unwrap(), b"/a/b/c");
    assert_eq!(read_all(&w.ov, path(&w.ov, "a/x/c3/g").unwrap()), b"gg");
    // Back to the old name: the redirect names the origin, no whiteout.
    rn(&w, "a/x", "c3", "a/b", "c").unwrap();
    assert_eq!(redirect_of(&w.upper, "a/b/c").unwrap(), b"/a/b/c");
    assert_eq!(w.upper.getattr(fs_path(&w.upper, "a/b/c")).unwrap().kind, FileType::Directory);
    assert_eq!(read_all(&w.ov, path(&w.ov, "a/b/c/g").unwrap()), b"gg");
}

#[test]
fn a_redirect_rename_moves_every_lower_directory_of_the_name() {
    let w = redir_world(Redirect::On);
    let root = w.ov.root();
    assert_eq!(names(&w.ov, path(&w.ov, "top").unwrap()), ["t2"]);
    w.ov.rename(root, b"top", root, b"top_m", RenameFlags::default()).unwrap();
    assert_eq!(names(&w.ov, path(&w.ov, "top_m").unwrap()), ["t2"]);
    assert_eq!(redirect_of(&w.upper, "top_m").unwrap(), b"top");
    // A directory made at the old name is empty and opaque.
    w.ov.mkdir(root, b"top", 0o755, 0, 0).unwrap();
    assert!(names(&w.ov, path(&w.ov, "top").unwrap()).is_empty());
    assert_eq!(w.upper.getxattr(fs_path(&w.upper, "top"), b"trusted.overlay.opaque").unwrap(), b"y");
    // Everything made in a directory that exists below is opaque.
    w.ov.mkdir(root, b"fresh", 0o755, 0, 0).unwrap();
    assert_eq!(w.upper.getxattr(fs_path(&w.upper, "fresh"), b"trusted.overlay.opaque").unwrap(), b"y");
    // The lower layers are untouched.
    assert!(w.lower2.lookup(w.lower2.root(), b"top").is_ok());
    assert!(w.lower1.lookup(w.lower1.root(), b"top").is_ok());
}

#[test]
fn a_redirect_that_is_too_long_is_exdev() {
    let w = redir_world(Redirect::On);
    let long = std::string::String::from_utf8(vec![b'n'; 251]).unwrap();
    let longer = std::string::String::from_utf8(vec![b'm'; 252]).unwrap();
    let lim = dir(&w.lower1, w.lower1.root(), "lim");
    dir(&w.lower1, lim, &long);
    dir(&w.lower1, lim, &longer);
    rn(&w, "lim", &long, "", "m256").unwrap();
    assert_eq!(redirect_of(&w.upper, "m256").unwrap().len(), 256);
    assert_eq!(rn(&w, "lim", &longer, "", "m257"), Err(Errno::EXDEV));
    assert_eq!(w.upper.lookup(w.upper.root(), b"m257"), Err(Errno::ENOENT));
    rn(&w, "lim", &longer, "lim", "sib").unwrap();
    assert_eq!(redirect_of(&w.upper, "lim/sib").unwrap(), longer.as_bytes());
}

#[test]
fn exchange_with_redirects() {
    let w = redir_world(Redirect::On);
    let root = w.ov.root();
    let ex = RenameFlags { noreplace: false, exchange: true };
    w.ov.rename(root, b"top", root, b"a", ex).unwrap();
    assert_eq!(redirect_of(&w.upper, "top").unwrap(), b"a");
    assert_eq!(redirect_of(&w.upper, "a").unwrap(), b"top");
    assert_eq!(names(&w.ov, path(&w.ov, "top").unwrap()), ["b", "x"]);
    assert_eq!(names(&w.ov, path(&w.ov, "a").unwrap()), ["t2"]);
}

#[test]
fn redirects_in_the_layers_are_followed_or_refused() {
    for (mode, follows) in
        [(Redirect::Follow, true), (Redirect::Off, true), (Redirect::On, true), (Redirect::NoFollow, false)]
    {
        let (_u, ul) = layer();
        let (l0, l0l) = layer();
        let (l1, l1l) = layer();
        let (l2, l2l) = layer();
        let real = dir(&l2, l2.root(), "dir");
        file(&l2, real, "f", b"from l2");
        let own = dir(&l1, l1.root(), "dir");
        file(&l1, own, "own", b"o");
        for n in ["rel", "abs", "none", "bad"] {
            dir(&l1, l1.root(), n);
        }
        dir(&l0, l0.root(), "moved");
        l0.setxattr(fs_path(&l0, "moved"), REDIR, b"abs", SetMode::default()).unwrap();
        let set = |p: &str, v: &[u8]| l1.setxattr(fs_path(&l1, p), REDIR, v, SetMode::default()).unwrap();
        set("rel", b"dir");
        set("abs", b"/dir");
        set("none", b"nothere");
        set("bad", b"a/b");
        let ov: Overlay =
            Overlay::new(Config { upper: Some(ul), lowers: vec![l0l, l1l, l2l], userxattr: false, redirect: mode })
                .unwrap();
        let root = ov.root();
        let all = ["abs", "bad", "dir", "moved", "none", "rel"];
        assert!(ov.lookup(root, b"dir").is_ok());
        assert_eq!(names(&ov, root), all);
        if !follows {
            assert_eq!(ov.lookup(root, b"rel"), Err(Errno::EPERM));
            assert_eq!(ov.lookup(root, b"none"), Err(Errno::EPERM));
            continue;
        }
        assert_eq!(names(&ov, path(&ov, "rel").unwrap()), ["f"]);
        assert_eq!(names(&ov, path(&ov, "abs").unwrap()), ["f"]);
        assert_eq!(read_all(&ov, path(&ov, "rel/f").unwrap()), b"from l2");
        assert!(names(&ov, path(&ov, "none").unwrap()).is_empty());
        assert_eq!(ov.lookup(root, b"bad"), Err(Errno::EINVAL));
        // A redirect of a redirect is followed in turn.
        assert_eq!(names(&ov, path(&ov, "moved").unwrap()), ["f"]);
    }
}

#[test]
fn a_planted_redirect_survives_a_rename_and_a_copy_up() {
    let (u, ul) = layer();
    let (l1, l1l) = layer();
    let (l2, l2l) = layer();
    let real = dir(&l2, l2.root(), "dir");
    file(&l2, real, "f", b"from l2");
    dir(&l1, l1.root(), "abs");
    l1.setxattr(fs_path(&l1, "abs"), REDIR, b"/dir", SetMode::default()).unwrap();
    let ov: Overlay =
        Overlay::new(Config { upper: Some(ul), lowers: vec![l1l, l2l], userxattr: false, redirect: Redirect::On })
            .unwrap();
    let root = ov.root();
    ov.rename(root, b"abs", root, b"x", RenameFlags::default()).unwrap();
    assert_eq!(u.getxattr(fs_path(&u, "x"), REDIR).unwrap(), b"abs");
    assert_eq!(names(&ov, path(&ov, "x").unwrap()), ["f"]);
    assert_eq!(ov.lookup(root, b"abs"), Err(Errno::ENOENT));
    // Writing into a redirected lower directory keeps what it shows.
    let (u2, ul2) = layer();
    let (l3, l3l) = layer();
    let (l4, l4l) = layer();
    let real = dir(&l4, l4.root(), "dir");
    file(&l4, real, "f", b"from l4");
    dir(&l3, l3.root(), "rel");
    l3.setxattr(fs_path(&l3, "rel"), REDIR, b"dir", SetMode::default()).unwrap();
    let ov: Overlay =
        Overlay::new(Config { upper: Some(ul2), lowers: vec![l3l, l4l], userxattr: false, redirect: Redirect::Follow })
            .unwrap();
    let rel = path(&ov, "rel").unwrap();
    let node = NewNode { kind: FileType::Regular, perm: 0o644, uid: 0, gid: 0, rdev: DeviceId::default() };
    ov.create(rel, b"new", &node).unwrap();
    assert_eq!(u2.getxattr(fs_path(&u2, "rel"), REDIR).unwrap(), b"rel");
    assert_eq!(names(&ov, rel), ["f", "new"]);
}

#[test]
fn mountinfo_shows_on_and_nofollow_only() {
    assert_eq!(shown_options(b"lowerdir=/l,redirect_dir=off,index=off"), b"lowerdir=/l,index=off");
    assert_eq!(shown_options(b"redirect_dir=follow,upperdir=/u"), b"upperdir=/u");
    assert_eq!(shown_options(b"lowerdir=/l,redirect_dir=on"), b"lowerdir=/l,redirect_dir=on");
    assert_eq!(shown_options(b"redirect_dir=nofollow"), b"redirect_dir=nofollow");
    assert_eq!(shown_options(b""), b"");
}

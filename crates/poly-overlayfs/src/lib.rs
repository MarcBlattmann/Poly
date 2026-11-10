//! overlayfs (ADR 0168) as a [`poly_vfs::FileSystem`] over directories of
//! other file systems.
//!
//! An [`Overlay`] shows an optional writable *upper* layer and one or more
//! read-only *lower* layers as one tree:
//!
//! - **Lookup** takes a name from the highest layer that has it. A
//!   non-directory hides everything below it, a directory merges with the
//!   directories of the same name below it, unless it is *opaque* (it carries
//!   the extended attribute `overlay.opaque` = `y`), and a *whiteout* (a
//!   character device 0:0) hides the name in every layer below.
//! - **Reading** is done on the highest layer's object. Anything that
//!   changes an object first *copies it up* to the upper layer (missing
//!   parent directories as empty copies with the lower directory's mode and
//!   owner, files with their data, holes kept, times and extended
//!   attributes); the lower layers are never changed.
//! - **Deleting** a name that a lower layer has leaves a whiteout; a
//!   directory created over a whiteout is opaque.
//! - **Renaming** a file moves its upper copy and leaves a whiteout behind
//!   if a lower layer has the old name; a directory that exists in a lower
//!   layer cannot be renamed (`EXDEV`, as with `redirect_dir=off`).
//!
//! Every overlay inode is one name in the merged tree (a lower file with two
//! names is two inodes, as with `index=off`), found again by its place in
//! the tree; an inode whose name was removed keeps what it showed, so an
//! unlinked open file stays readable. The attributes `trusted.overlay.*`
//! (`user.overlay.*` with `userxattr`) are the overlay's own and hidden.

#![no_std]
extern crate alloc;

use alloc::collections::BTreeMap;
use alloc::sync::Arc;
use alloc::vec;
use alloc::vec::Vec;
use poly_vfs::lock::{Lock, RawLock, RawSpinLock};
use poly_vfs::xattr::SetMode;
use poly_vfs::{
    Attr, DeviceId, DirEntry, Errno, FileSystem, FileType, FsInfo, FsStats, Ino, NewNode, RenameFlags, Result, SetAttr,
    SetTime,
};

/// The `f_type` of overlayfs (`linux/magic.h`).
pub const OVERLAYFS_SUPER_MAGIC: u64 = 0x794c_7630;
const ROOT: Ino = 1;
const COPY_CHUNK: usize = 128 * 1024;

/// One layer: a directory of a file system.
#[derive(Clone)]
pub struct Layer {
    pub fs: Arc<dyn FileSystem>,
    pub root: Ino,
}

impl core::fmt::Debug for Layer {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.debug_struct("Layer").field("root", &self.root).finish_non_exhaustive()
    }
}

/// `redirect_dir=`: whether renaming a lower directory is supported.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Redirect {
    Off,
    Follow,
    On,
}

impl Redirect {
    /// The option's value (`on`, `off`, `follow`, `nofollow`).
    pub fn from_value(v: &[u8]) -> Result<Redirect> {
        match v {
            b"on" => Ok(Redirect::On),
            b"off" | b"nofollow" => Ok(Redirect::Off),
            b"follow" => Ok(Redirect::Follow),
            _ => Err(Errno::EINVAL),
        }
    }
}

/// How an overlay is made.
#[derive(Clone, Debug)]
pub struct Config {
    /// The writable layer, if any.
    pub upper: Option<Layer>,
    /// The lower layers, highest first.
    pub lowers: Vec<Layer>,
    /// `userxattr`: keep the markers in `user.overlay.*`.
    pub userxattr: bool,
    pub redirect: Redirect,
}

#[derive(Clone, Copy, Debug)]
struct Real {
    layer: usize,
    ino: Ino,
}

#[derive(Debug)]
struct Node {
    parent: Ino,
    name: Vec<u8>,
    kind: FileType,
    /// The objects it shows, highest first (a merged directory has several,
    /// anything else one); `None` until looked up.
    real: Option<Vec<Real>>,
    /// Directories: a stable position for each name listed so far.
    cookies: BTreeMap<Vec<u8>, u64>,
    next_cookie: u64,
}

#[derive(Debug)]
struct State {
    nodes: BTreeMap<Ino, Node>,
    by_name: BTreeMap<(Ino, Vec<u8>), Ino>,
    next: Ino,
}

/// A mounted overlay.
pub struct Overlay<L: RawLock + Default + 'static = RawSpinLock> {
    layers: Vec<Layer>,
    has_upper: bool,
    opaque: &'static [u8],
    internal: &'static [u8],
    state: Lock<L, State>,
}

impl<L: RawLock + Default + 'static> core::fmt::Debug for Overlay<L> {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.debug_struct("Overlay").field("layers", &self.layers.len()).finish_non_exhaustive()
    }
}

fn whiteout_node() -> NewNode {
    NewNode { kind: FileType::CharDevice, perm: 0, uid: 0, gid: 0, rdev: DeviceId::new(0, 0) }
}

fn is_whiteout(a: &Attr) -> bool {
    a.kind == FileType::CharDevice && a.rdev == DeviceId::new(0, 0)
}

struct Ops<'a, L: RawLock + Default + 'static> {
    o: &'a Overlay<L>,
    s: &'a mut State,
}

impl<L: RawLock + Default + 'static> Overlay<L> {
    /// Makes the overlay of `config`'s layers.
    pub fn new(config: Config) -> Result<Overlay<L>> {
        if config.lowers.is_empty() {
            return Err(Errno::EINVAL);
        }
        let has_upper = config.upper.is_some();
        let mut layers = Vec::new();
        layers.extend(config.upper);
        layers.extend(config.lowers);
        let (opaque, internal): (&'static [u8], &'static [u8]) = if config.userxattr {
            (b"user.overlay.opaque", b"user.overlay.")
        } else {
            (b"trusted.overlay.opaque", b"trusted.overlay.")
        };
        let mut root_real = Vec::new();
        for (i, l) in layers.iter().enumerate() {
            root_real.push(Real { layer: i, ino: l.root });
            // An opaque root hides the layers below it.
            if i + 1 < layers.len() && l.fs.getxattr(l.root, opaque).is_ok_and(|v| v == b"y") {
                break;
            }
        }
        let mut nodes = BTreeMap::new();
        nodes.insert(
            ROOT,
            Node {
                parent: ROOT,
                name: Vec::new(),
                kind: FileType::Directory,
                real: Some(root_real),
                cookies: BTreeMap::new(),
                next_cookie: 2,
            },
        );
        Ok(Overlay {
            layers,
            has_upper,
            opaque,
            internal,
            state: Lock::new(State { nodes, by_name: BTreeMap::new(), next: ROOT + 1 }),
        })
    }

    fn with<R>(&self, f: impl FnOnce(&mut Ops<'_, L>) -> Result<R>) -> Result<R> {
        let mut s = self.state.lock();
        let mut ops = Ops { o: self, s: &mut s };
        f(&mut ops)
    }
}

impl<L: RawLock + Default + 'static> Ops<'_, L> {
    fn layer(&self, i: usize) -> &Layer {
        &self.o.layers[i]
    }

    fn up(&self) -> Result<&Layer> {
        if self.o.has_upper { Ok(&self.o.layers[0]) } else { Err(Errno::EROFS) }
    }

    fn is_upper(&self, r: &Real) -> bool {
        self.o.has_upper && r.layer == 0
    }

    fn node(&self, ino: Ino) -> Result<&Node> {
        self.s.nodes.get(&ino).ok_or(Errno::ENOENT)
    }

    /// What the node shows (looked up now if it was only listed).
    fn real(&mut self, ino: Ino) -> Result<Vec<Real>> {
        let n = self.node(ino)?;
        if let Some(r) = &n.real {
            return Ok(r.clone());
        }
        let (parent, name) = (n.parent, n.name.clone());
        let p = self.real(parent)?;
        let (real, _) = self.find_child(&p, &name)?.ok_or(Errno::ENOENT)?;
        if let Some(n) = self.s.nodes.get_mut(&ino) {
            n.real = Some(real.clone());
        }
        Ok(real)
    }

    fn is_opaque(&self, r: &Real) -> bool {
        let l = self.layer(r.layer);
        l.fs.getxattr(r.ino, self.o.opaque).is_ok_and(|v| v == b"y")
    }

    /// What `name` is in the merged directory whose parts are `parent`.
    fn find_child(&self, parent: &[Real], name: &[u8]) -> Result<Option<(Vec<Real>, FileType)>> {
        let mut out: Vec<Real> = Vec::new();
        let mut kind: Option<FileType> = None;
        for p in parent {
            let l = self.layer(p.layer);
            let child = match l.fs.lookup(p.ino, name) {
                Ok(c) => c,
                Err(Errno::ENOENT) => continue,
                Err(e) => return Err(e),
            };
            let a = l.fs.getattr(child)?;
            if is_whiteout(&a) {
                break;
            }
            let r = Real { layer: p.layer, ino: child };
            match kind {
                None => {
                    kind = Some(a.kind);
                    out.push(r);
                    if a.kind != FileType::Directory {
                        break;
                    }
                }
                Some(_) => {
                    if a.kind != FileType::Directory {
                        break;
                    }
                    out.push(r);
                }
            }
            if a.kind == FileType::Directory && self.is_opaque(&r) {
                break;
            }
        }
        Ok(kind.map(|k| (out, k)))
    }

    /// Whether a lower layer of directory `parent` has `name` (not hidden by
    /// a whiteout above it).
    fn lower_has(&self, parent: &[Real], name: &[u8]) -> Result<bool> {
        for p in parent {
            if self.is_upper(p) {
                continue;
            }
            let l = self.layer(p.layer);
            match l.fs.lookup(p.ino, name) {
                Ok(c) => {
                    let a = l.fs.getattr(c)?;
                    return Ok(!is_whiteout(&a));
                }
                Err(Errno::ENOENT) => {}
                Err(e) => return Err(e),
            }
        }
        Ok(false)
    }

    fn intern(&mut self, parent: Ino, name: &[u8], kind: FileType, real: Option<Vec<Real>>) -> Ino {
        if let Some(&i) = self.s.by_name.get(&(parent, name.to_vec())) {
            if let (Some(r), Some(n)) = (real, self.s.nodes.get_mut(&i)) {
                n.real = Some(r);
            }
            return i;
        }
        let ino = self.s.next;
        self.s.next += 1;
        self.s
            .nodes
            .insert(ino, Node { parent, name: name.to_vec(), kind, real, cookies: BTreeMap::new(), next_cookie: 2 });
        self.s.by_name.insert((parent, name.to_vec()), ino);
        ino
    }

    /// The node for `name` in `dir`, if the name exists in the merged tree.
    fn lookup_child(&mut self, dir: Ino, name: &[u8]) -> Result<Option<Ino>> {
        if let Some(&i) = self.s.by_name.get(&(dir, name.to_vec())) {
            return Ok(Some(i));
        }
        let p = self.real(dir)?;
        match self.find_child(&p, name)? {
            Some((real, kind)) => Ok(Some(self.intern(dir, name, kind, Some(real)))),
            None => Ok(None),
        }
    }

    fn check_dir(&mut self, dir: Ino) -> Result<()> {
        if self.node(dir)?.kind == FileType::Directory { Ok(()) } else { Err(Errno::ENOTDIR) }
    }

    // ----- copy-up -----

    fn copy_data(&self, src: &Layer, sino: Ino, dst: &Layer, dino: Ino, size: u64) -> Result<()> {
        let mut buf = vec![0u8; COPY_CHUNK];
        let mut at = 0u64;
        while at < size {
            let n = src.fs.read(sino, at, &mut buf)?;
            if n == 0 {
                break;
            }
            // Zero blocks stay holes.
            if buf[..n].iter().any(|&b| b != 0) {
                let mut done = 0;
                while done < n {
                    let w = dst.fs.write(dino, at + done as u64, &buf[done..n])?;
                    if w == 0 {
                        return Err(Errno::EIO);
                    }
                    done += w;
                }
            }
            at += n as u64;
        }
        dst.fs.truncate(dino, size)
    }

    fn copy_xattrs(&self, src: &Layer, sino: Ino, dst: &Layer, dino: Ino) {
        let Ok(list) = src.fs.listxattr(sino) else { return };
        for name in list.split(|&b| b == 0).filter(|n| !n.is_empty()) {
            if name.starts_with(self.o.internal) {
                continue;
            }
            if let Ok(v) = src.fs.getxattr(sino, name) {
                let _ = dst.fs.setxattr(dino, name, &v, SetMode::default());
            }
        }
    }

    /// Makes sure `ino` is in the upper layer and returns its upper inode.
    fn ensure_upper(&mut self, ino: Ino) -> Result<Ino> {
        let up = self.up()?.clone();
        let real = self.real(ino)?;
        if self.is_upper(&real[0]) {
            return Ok(real[0].ino);
        }
        let (parent, name) = {
            let n = self.node(ino)?;
            (n.parent, n.name.clone())
        };
        let parent_up = self.ensure_upper(parent)?;
        // The parent may have gained the name meanwhile (a rename).
        let src = real[0];
        let sl = self.layer(src.layer).clone();
        let a = sl.fs.getattr(src.ino)?;
        let new = match a.kind {
            FileType::Directory => up.fs.mkdir(parent_up, &name, a.perm, a.uid, a.gid)?,
            FileType::Symlink => {
                let target = sl.fs.readlink(src.ino)?;
                up.fs.symlink(parent_up, &name, &target, a.uid, a.gid)?
            }
            FileType::Regular => {
                let n = up.fs.create(
                    parent_up,
                    &name,
                    &NewNode {
                        kind: FileType::Regular,
                        perm: a.perm,
                        uid: a.uid,
                        gid: a.gid,
                        rdev: DeviceId::default(),
                    },
                )?;
                self.copy_data(&sl, src.ino, &up, n, a.size)?;
                n
            }
            kind => {
                up.fs.create(parent_up, &name, &NewNode { kind, perm: a.perm, uid: a.uid, gid: a.gid, rdev: a.rdev })?
            }
        };
        let set = SetAttr {
            perm: Some(a.perm),
            uid: Some(a.uid),
            gid: Some(a.gid),
            atime: Some(SetTime::At(a.atime)),
            mtime: Some(SetTime::At(a.mtime)),
            ..SetAttr::default()
        };
        if a.kind == FileType::Symlink {
            let _ = up.fs.setattr(new, &SetAttr { uid: Some(a.uid), gid: Some(a.gid), ..SetAttr::default() });
        } else {
            up.fs.setattr(new, &set)?;
        }
        self.copy_xattrs(&sl, src.ino, &up, new);
        let mut r = vec![Real { layer: 0, ino: new }];
        if a.kind == FileType::Directory {
            r.extend_from_slice(&real);
        }
        if let Some(n) = self.s.nodes.get_mut(&ino) {
            n.real = Some(r);
        }
        Ok(new)
    }

    /// Takes the whiteout at `name` in upper directory `dir` away, if there is one.
    fn remove_whiteout(&self, up: &Layer, dir: Ino, name: &[u8]) -> Result<bool> {
        match up.fs.lookup(dir, name) {
            Ok(c) => {
                let a = up.fs.getattr(c)?;
                if is_whiteout(&a) {
                    up.fs.unlink(dir, name)?;
                    return Ok(true);
                }
                Ok(false)
            }
            Err(Errno::ENOENT) => Ok(false),
            Err(e) => Err(e),
        }
    }

    fn forget(&mut self, parent: Ino, name: &[u8]) {
        self.s.by_name.remove(&(parent, name.to_vec()));
    }

    // ----- merged listing -----

    /// Every name of merged directory `dir` with its kind, in the order the
    /// layers give them.
    fn merged_names(&mut self, dir: Ino) -> Result<Vec<(Vec<u8>, FileType)>> {
        let parts = self.real(dir)?;
        let mut seen: BTreeMap<Vec<u8>, ()> = BTreeMap::new();
        let mut out: Vec<(Vec<u8>, FileType)> = Vec::new();
        for p in &parts {
            let l = self.layer(p.layer).clone();
            let mut entries: Vec<(Vec<u8>, FileType, Ino)> = Vec::new();
            let mut pos = 0;
            loop {
                let mut batch: Vec<(Vec<u8>, FileType, Ino, u64)> = Vec::new();
                l.fs.readdir(p.ino, pos, &mut |e| {
                    batch.push((e.name.to_vec(), e.kind, e.ino, e.next));
                    batch.len() < 256
                })?;
                if batch.is_empty() {
                    break;
                }
                pos = batch.last().map_or(pos, |b| b.3);
                let full = batch.len() >= 256;
                entries.extend(batch.into_iter().map(|(n, k, i, _)| (n, k, i)));
                if !full {
                    break;
                }
            }
            for (name, kind, ino) in entries {
                if name == b"." || name == b".." || seen.contains_key(&name) {
                    continue;
                }
                seen.insert(name.clone(), ());
                if kind == FileType::CharDevice && is_whiteout(&l.fs.getattr(ino)?) {
                    continue;
                }
                out.push((name, kind));
            }
        }
        Ok(out)
    }

    fn make_whiteout(&self, up: &Layer, dir: Ino, name: &[u8]) -> Result<()> {
        up.fs.create(dir, name, &whiteout_node()).map(|_| ())
    }

    fn attr_of(&mut self, ino: Ino) -> Result<Attr> {
        let real = self.real(ino)?;
        let top = real[0];
        let mut a = self.layer(top.layer).fs.getattr(top.ino)?;
        a.ino = ino;
        Ok(a)
    }

    fn lookup(&mut self, dir: Ino, name: &[u8]) -> Result<Ino> {
        self.check_dir(dir)?;
        if name == b".." {
            return Ok(self.node(dir)?.parent);
        }
        self.lookup_child(dir, name)?.ok_or(Errno::ENOENT)
    }

    fn add_child(&mut self, dir: Ino, name: &[u8], kind: FileType, upper: Ino) -> Ino {
        self.intern(dir, name, kind, Some(vec![Real { layer: 0, ino: upper }]))
    }

    fn create(&mut self, dir: Ino, name: &[u8], node: &NewNode) -> Result<Ino> {
        self.check_dir(dir)?;
        let up = self.up()?.clone();
        if self.lookup_child(dir, name)?.is_some() {
            return Err(Errno::EEXIST);
        }
        let up_dir = self.ensure_upper(dir)?;
        self.remove_whiteout(&up, up_dir, name)?;
        let n = up.fs.create(up_dir, name, node)?;
        Ok(self.add_child(dir, name, node.kind, n))
    }

    fn mkdir(&mut self, dir: Ino, name: &[u8], perm: u16, uid: u32, gid: u32) -> Result<Ino> {
        self.check_dir(dir)?;
        let up = self.up()?.clone();
        if self.lookup_child(dir, name)?.is_some() {
            return Err(Errno::EEXIST);
        }
        let up_dir = self.ensure_upper(dir)?;
        let hid = self.remove_whiteout(&up, up_dir, name)?;
        let n = up.fs.mkdir(up_dir, name, perm, uid, gid)?;
        if hid {
            up.fs.setxattr(n, self.o.opaque, b"y", SetMode::default()).map_err(|_| Errno::EIO)?;
        }
        Ok(self.add_child(dir, name, FileType::Directory, n))
    }

    fn symlink(&mut self, dir: Ino, name: &[u8], target: &[u8], uid: u32, gid: u32) -> Result<Ino> {
        self.check_dir(dir)?;
        let up = self.up()?.clone();
        if self.lookup_child(dir, name)?.is_some() {
            return Err(Errno::EEXIST);
        }
        let up_dir = self.ensure_upper(dir)?;
        self.remove_whiteout(&up, up_dir, name)?;
        let n = up.fs.symlink(up_dir, name, target, uid, gid)?;
        Ok(self.add_child(dir, name, FileType::Symlink, n))
    }

    fn link(&mut self, dir: Ino, name: &[u8], ino: Ino) -> Result<()> {
        self.check_dir(dir)?;
        let up = self.up()?.clone();
        if self.node(ino)?.kind == FileType::Directory {
            return Err(Errno::EPERM);
        }
        if self.lookup_child(dir, name)?.is_some() {
            return Err(Errno::EEXIST);
        }
        let target = self.ensure_upper(ino)?;
        let up_dir = self.ensure_upper(dir)?;
        self.remove_whiteout(&up, up_dir, name)?;
        up.fs.link(up_dir, name, target)?;
        let kind = self.node(ino)?.kind;
        self.add_child(dir, name, kind, target);
        Ok(())
    }

    fn remove(&mut self, dir: Ino, name: &[u8], is_dir: bool) -> Result<()> {
        self.check_dir(dir)?;
        let up = self.up()?.clone();
        let child = self.lookup_child(dir, name)?.ok_or(Errno::ENOENT)?;
        let kind = self.node(child)?.kind;
        if is_dir && kind != FileType::Directory {
            return Err(Errno::ENOTDIR);
        }
        if !is_dir && kind == FileType::Directory {
            return Err(Errno::EISDIR);
        }
        if is_dir && !self.merged_names(child)?.is_empty() {
            return Err(Errno::ENOTEMPTY);
        }
        let up_dir = self.ensure_upper(dir)?;
        let real = self.real(child)?;
        if self.is_upper(&real[0]) {
            if is_dir {
                // What is left of the directory in the upper layer is whiteouts.
                let upper_dir = real[0].ino;
                let mut names = Vec::new();
                up.fs.readdir(upper_dir, 0, &mut |e| {
                    if e.name != b"." && e.name != b".." {
                        names.push(e.name.to_vec());
                    }
                    true
                })?;
                for n in names {
                    up.fs.unlink(upper_dir, &n)?;
                }
                up.fs.rmdir(up_dir, name)?;
            } else {
                up.fs.unlink(up_dir, name)?;
            }
        }
        let parent_real = self.real(dir)?;
        if self.lower_has(&parent_real, name)? {
            self.make_whiteout(&up, up_dir, name)?;
        }
        self.forget(dir, name);
        Ok(())
    }

    #[allow(clippy::too_many_lines)]
    fn rename(&mut self, od: Ino, on: &[u8], nd: Ino, nn: &[u8], flags: RenameFlags) -> Result<()> {
        self.check_dir(od)?;
        self.check_dir(nd)?;
        let up = self.up()?.clone();
        let src = self.lookup_child(od, on)?.ok_or(Errno::ENOENT)?;
        let dst = self.lookup_child(nd, nn)?;
        if dst.is_some() && flags.noreplace {
            return Err(Errno::EEXIST);
        }
        if flags.exchange && dst.is_none() {
            return Err(Errno::ENOENT);
        }
        if dst == Some(src) {
            return Ok(());
        }
        let src_kind = self.node(src)?.kind;
        let src_real = self.real(src)?;
        let from_lower = src_real.iter().any(|r| !self.is_upper(r));
        if src_kind == FileType::Directory && from_lower {
            return Err(Errno::EXDEV);
        }
        if let Some(d) = dst {
            let dk = self.node(d)?.kind;
            if flags.exchange {
                if dk == FileType::Directory && self.real(d)?.iter().any(|r| !self.is_upper(r)) {
                    return Err(Errno::EXDEV);
                }
            } else if src_kind == FileType::Directory {
                if dk != FileType::Directory {
                    return Err(Errno::ENOTDIR);
                }
                if !self.merged_names(d)?.is_empty() {
                    return Err(Errno::ENOTEMPTY);
                }
            } else if dk == FileType::Directory {
                return Err(Errno::EISDIR);
            }
        }
        let src_up = self.ensure_upper(src)?;
        let up_od = self.ensure_upper(od)?;
        let up_nd = self.ensure_upper(nd)?;
        if flags.exchange
            && let Some(d) = dst
        {
            self.ensure_upper(d)?;
        }
        let new_parent_real = self.real(nd)?;
        let old_parent_real = self.real(od)?;
        let mut over_lower = false;
        if let Some(d) = dst
            && !flags.exchange
        {
            // A directory replacing a directory must not show what is below.
            over_lower = self.node(d)?.kind == FileType::Directory && self.lower_has(&new_parent_real, nn)?;
            if self.real(d).is_ok_and(|r| self.is_upper(&r[0])) && self.node(d)?.kind == FileType::Directory {
                // Its upper directory only holds whiteouts.
                let ud = self.real(d)?[0].ino;
                let mut names = Vec::new();
                up.fs.readdir(ud, 0, &mut |e| {
                    if e.name != b"." && e.name != b".." {
                        names.push(e.name.to_vec());
                    }
                    true
                })?;
                for n in names {
                    up.fs.unlink(ud, &n)?;
                }
            }
        } else if dst.is_none() {
            self.remove_whiteout(&up, up_nd, nn)?;
            over_lower = src_kind == FileType::Directory && self.lower_has(&new_parent_real, nn)?;
        }
        up.fs.rename(up_od, on, up_nd, nn, RenameFlags { noreplace: false, exchange: flags.exchange })?;
        if over_lower {
            up.fs.setxattr(src_up, self.o.opaque, b"y", SetMode::default()).map_err(|_| Errno::EIO)?;
        }
        if !flags.exchange && self.lower_has(&old_parent_real, on)? {
            self.make_whiteout(&up, up_od, on)?;
        }
        // The nodes follow their objects.
        self.s.by_name.remove(&(od, on.to_vec()));
        self.s.by_name.remove(&(nd, nn.to_vec()));
        if let Some(n) = self.s.nodes.get_mut(&src) {
            n.parent = nd;
            n.name = nn.to_vec();
        }
        self.s.by_name.insert((nd, nn.to_vec()), src);
        if flags.exchange
            && let Some(d) = dst
        {
            if let Some(n) = self.s.nodes.get_mut(&d) {
                n.parent = od;
                n.name = on.to_vec();
            }
            self.s.by_name.insert((od, on.to_vec()), d);
        }
        Ok(())
    }

    fn readdir(&mut self, dir: Ino, pos: u64, emit: &mut dyn FnMut(&DirEntry<'_>) -> bool) -> Result<()> {
        self.check_dir(dir)?;
        let parent = self.node(dir)?.parent;
        if pos == 0 && !emit(&DirEntry { ino: dir, kind: FileType::Directory, name: b".", next: 1 }) {
            return Ok(());
        }
        if pos <= 1 && !emit(&DirEntry { ino: parent, kind: FileType::Directory, name: b"..", next: 2 }) {
            return Ok(());
        }
        let names = self.merged_names(dir)?;
        // Give new names the next position; the listing is in position order.
        let mut listing: Vec<(u64, Vec<u8>, FileType)> = Vec::new();
        {
            let n = self.s.nodes.get_mut(&dir).ok_or(Errno::ENOENT)?;
            for (name, kind) in names {
                let c = match n.cookies.get(&name) {
                    Some(&c) => c,
                    None => {
                        let c = n.next_cookie;
                        n.next_cookie += 1;
                        n.cookies.insert(name.clone(), c);
                        c
                    }
                };
                listing.push((c, name, kind));
            }
            listing.sort_by_key(|e| e.0);
        }
        for (cookie, name, kind) in listing {
            if cookie < pos {
                continue;
            }
            let ino = self.intern(dir, &name, kind, None);
            if !emit(&DirEntry { ino, kind, name: &name, next: cookie + 1 }) {
                return Ok(());
            }
        }
        Ok(())
    }
}

impl<L: RawLock + Default + 'static> FileSystem for Overlay<L> {
    fn info(&self) -> FsInfo {
        FsInfo { name: "overlay", read_only: !self.has_upper, cache_lookups: false }
    }

    fn root(&self) -> Ino {
        ROOT
    }

    fn lookup(&self, dir: Ino, name: &[u8]) -> Result<Ino> {
        self.with(|o| o.lookup(dir, name))
    }

    fn getattr(&self, ino: Ino) -> Result<Attr> {
        self.with(|o| o.attr_of(ino))
    }

    fn setattr(&self, ino: Ino, set: &SetAttr) -> Result<Attr> {
        self.with(|o| {
            let up = o.up()?.clone();
            let u = o.ensure_upper(ino)?;
            let mut a = up.fs.setattr(u, set)?;
            a.ino = ino;
            Ok(a)
        })
    }

    fn create(&self, dir: Ino, name: &[u8], node: &NewNode) -> Result<Ino> {
        self.with(|o| o.create(dir, name, node))
    }

    fn mkdir(&self, dir: Ino, name: &[u8], perm: u16, uid: u32, gid: u32) -> Result<Ino> {
        self.with(|o| o.mkdir(dir, name, perm, uid, gid))
    }

    fn symlink(&self, dir: Ino, name: &[u8], target: &[u8], uid: u32, gid: u32) -> Result<Ino> {
        self.with(|o| o.symlink(dir, name, target, uid, gid))
    }

    fn link(&self, dir: Ino, name: &[u8], ino: Ino) -> Result<()> {
        self.with(|o| o.link(dir, name, ino))
    }

    fn unlink(&self, dir: Ino, name: &[u8]) -> Result<()> {
        self.with(|o| o.remove(dir, name, false))
    }

    fn rmdir(&self, dir: Ino, name: &[u8]) -> Result<()> {
        self.with(|o| o.remove(dir, name, true))
    }

    fn rename(&self, od: Ino, on: &[u8], nd: Ino, nn: &[u8], flags: RenameFlags) -> Result<()> {
        self.with(|o| o.rename(od, on, nd, nn, flags))
    }

    fn readlink(&self, ino: Ino) -> Result<Vec<u8>> {
        self.with(|o| {
            let real = o.real(ino)?;
            o.layer(real[0].layer).fs.readlink(real[0].ino)
        })
    }

    fn read(&self, ino: Ino, offset: u64, buf: &mut [u8]) -> Result<usize> {
        self.with(|o| {
            let real = o.real(ino)?;
            o.layer(real[0].layer).fs.read(real[0].ino, offset, buf)
        })
    }

    fn write(&self, ino: Ino, offset: u64, buf: &[u8]) -> Result<usize> {
        self.with(|o| {
            let up = o.up()?.clone();
            let u = o.ensure_upper(ino)?;
            up.fs.write(u, offset, buf)
        })
    }

    fn append(&self, ino: Ino, buf: &[u8]) -> Result<(u64, usize)> {
        self.with(|o| {
            let up = o.up()?.clone();
            let u = o.ensure_upper(ino)?;
            up.fs.append(u, buf)
        })
    }

    fn fallocate(&self, ino: Ino, mode: u32, offset: u64, len: u64) -> Result<()> {
        self.with(|o| {
            let up = o.up()?.clone();
            let u = o.ensure_upper(ino)?;
            up.fs.fallocate(u, mode, offset, len)
        })
    }

    fn prepare_write(&self, ino: Ino) -> Result<()> {
        self.with(|o| {
            if o.node(ino)?.kind == FileType::Regular {
                o.ensure_upper(ino)?;
            }
            Ok(())
        })
    }

    fn readdir(&self, dir: Ino, pos: u64, emit: &mut dyn FnMut(&DirEntry<'_>) -> bool) -> Result<()> {
        // Collect under the lock, hand out without it.
        let mut entries: Vec<(Ino, FileType, Vec<u8>, u64)> = Vec::new();
        self.with(|o| {
            o.readdir(dir, pos, &mut |e| {
                entries.push((e.ino, e.kind, e.name.to_vec(), e.next));
                true
            })
        })?;
        for (ino, kind, name, next) in entries {
            if !emit(&DirEntry { ino, kind, name: &name, next }) {
                break;
            }
        }
        Ok(())
    }

    fn statfs(&self) -> Result<FsStats> {
        let l = &self.layers[0];
        let mut st = l.fs.statfs()?;
        st.magic = OVERLAYFS_SUPER_MAGIC;
        Ok(st)
    }

    fn getxattr(&self, ino: Ino, name: &[u8]) -> Result<Vec<u8>> {
        if name.starts_with(self.internal) {
            return Err(Errno::ENODATA);
        }
        self.with(|o| {
            let real = o.real(ino)?;
            o.layer(real[0].layer).fs.getxattr(real[0].ino, name)
        })
    }

    fn setxattr(&self, ino: Ino, name: &[u8], value: &[u8], how: SetMode) -> Result<()> {
        if name.starts_with(self.internal) {
            return Err(Errno::EOPNOTSUPP);
        }
        self.with(|o| {
            let up = o.up()?.clone();
            let u = o.ensure_upper(ino)?;
            up.fs.setxattr(u, name, value, how)
        })
    }

    fn listxattr(&self, ino: Ino) -> Result<Vec<u8>> {
        self.with(|o| {
            let real = o.real(ino)?;
            let raw = o.layer(real[0].layer).fs.listxattr(real[0].ino)?;
            let mut out = Vec::new();
            for n in raw.split(|&b| b == 0).filter(|n| !n.is_empty()) {
                if !n.starts_with(o.o.internal) {
                    out.extend_from_slice(n);
                    out.push(0);
                }
            }
            Ok(out)
        })
    }

    fn removexattr(&self, ino: Ino, name: &[u8]) -> Result<()> {
        if name.starts_with(self.internal) {
            return Err(Errno::ENODATA);
        }
        self.with(|o| {
            let up = o.up()?.clone();
            // A name no layer's object has is ENODATA without a copy-up.
            let real = o.real(ino)?;
            o.layer(real[0].layer).fs.getxattr(real[0].ino, name)?;
            let u = o.ensure_upper(ino)?;
            up.fs.removexattr(u, name)
        })
    }
}

/// Whether directory `ino` of `fs` is `ancestor` or below it (following
/// `..` to the root).
pub fn is_inside(fs: &dyn FileSystem, mut ino: Ino, ancestor: Ino) -> bool {
    for _ in 0..4096 {
        if ino == ancestor {
            return true;
        }
        match fs.lookup(ino, b"..") {
            Ok(p) if p != ino => ino = p,
            _ => return false,
        }
    }
    false
}

#[cfg(test)]
mod tests;

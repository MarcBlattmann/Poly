//! Path resolution, following `path_resolution(7)`.
//!
//! A walk starts at the process root (absolute paths) or at a directory
//! (the working directory, or the directory a `dirfd` names). Each component
//! except the last must name a directory the caller may search (`ENOTDIR`,
//! `EACCES`), and is resolved like this:
//!
//! - empty components (`a//b`) are skipped; `.` stays put;
//! - `..` goes to the parent, except at the process root, where it stays;
//!   at the root of a mount it first climbs to the mount point, so `..`
//!   leaves the mounted file system;
//! - a name is looked up, and if a file system is mounted on the result,
//!   the walk continues at the root of the topmost such mount;
//! - a symbolic link is replaced by its target (an absolute target restarts
//!   at the process root, a relative one continues in the link's
//!   directory). At most [`MAX_SYMLINKS`] links are followed per walk,
//!   then `ELOOP`. An empty target is `ENOENT`.
//!
//! The last component is handed to the caller unresolved ([`Parent`]),
//! because what happens to it depends on the operation: followed or not,
//! created, removed. A trailing slash means the last component must be a
//! directory, and a symbolic link there is followed even when the
//! operation would not otherwise follow one.
//!
//! Walks are iterative (a symbolic link is spliced into the remaining path
//! instead of recursing), so kernel stack use does not grow with link depth.
//!
//! # The order of path errors
//!
//! When a path has more than one problem, the first in this order decides
//! (the spec sheets' `path_order`, the same for every call that takes a
//! path; symlinkat's `precedence`):
//!
//! 1. the string as a whole: unreadable (`EFAULT`), `PATH_MAX` bytes or more
//!    with its NUL (`ENAMETOOLONG`), empty (`ENOENT`, where that is an
//!    error), whatever its components are;
//! 2. for a relative path, the directory descriptor: not open (`EBADF`),
//!    open on a non-directory (`ENOTDIR`, from the walk's first step);
//! 3. the components from left to right as the walk reaches them, symbolic
//!    links followed on the way. For each name the walk first needs the
//!    directory it is in: a non-directory there is `ENOTDIR`, no search
//!    permission `EACCES`; only then is the name itself looked at (over
//!    [`NAME_MAX`] bytes `ENAMETOOLONG`, missing `ENOENT`). So the first
//!    failure decides: `D/nope/<256 bytes>` is `ENOENT`, `D/file/<256
//!    bytes>` is `ENOTDIR`, `D/<256 bytes>/nope` is `ENAMETOOLONG`, and a
//!    `..` after a missing name or a dangling link changes nothing;
//! 4. what the operation finds about the last name (`EEXIST`, `EISDIR`, ...).
//!
//! The kernel and the personality take care of 1 and 2 for one path (the
//! string is copied first, then the descriptor looked up); a call with two
//! paths hands the second one's string and descriptor over as a
//! [`PathArg`], whose errors the VFS reports in their place
//! ([`Vfs::link_paths`], [`Vfs::rename_paths`]). [`Walk::dirs`] stops before
//! the last name's length, so that rename can check both paths' directories
//! before either last name ([`Parent::check_name`]).

use crate::cred::{Access, Cred};
use crate::fdtable::At;
use crate::node::Location;
use crate::{Errno, FileType, Result, Vfs};
use alloc::vec::Vec;

/// The longest path, including its terminating NUL (`PATH_MAX`, Linux UAPI
/// `linux/limits.h`); longer is `ENAMETOOLONG`.
pub const PATH_MAX: usize = 4096;
/// The longest path component (`NAME_MAX`, `linux/limits.h`).
pub const NAME_MAX: usize = 255;
/// Symbolic links followed in one resolution before `ELOOP`
/// (`path_resolution(7)` and `symlink(7)` give Linux's limit as 40).
pub const MAX_SYMLINKS: u32 = 40;

/// The last component of a path.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum Last {
    /// There is none: the path was `/` (or only slashes); `dir` is meant.
    Root,
    Dot,
    DotDot,
    Name(Vec<u8>),
}

/// A path with everything but its last component resolved.
pub(crate) struct Parent {
    /// The directory the last component is in. The caller may search it.
    pub(crate) dir: Location,
    pub(crate) last: Last,
    pub(crate) trailing_slash: bool,
}

impl Parent {
    /// The last name's own check, the step of the walk that comes after its
    /// directory: over [`NAME_MAX`] bytes is `ENAMETOOLONG`.
    pub(crate) fn check_name(&self) -> Result<()> {
        match &self.last {
            Last::Name(name) if name.len() > NAME_MAX => Err(Errno::ENAMETOOLONG),
            _ => Ok(()),
        }
    }
}

/// A path argument of a system call as the caller has it, its errors not
/// reported yet: the string, or why it could not be read (`EFAULT`,
/// `ENAMETOOLONG`, or `ENOENT` for an empty one), and where a relative path
/// starts, or why the directory descriptor names nothing (`EBADF`). The VFS
/// reports these errors where they come in the order of path errors (see
/// the module documentation): the string's first, the start's next (only
/// for a relative path), then the walk's. For a call with two paths that is
/// after some or all of the first path's errors.
#[derive(Clone, Copy, Debug)]
pub struct PathArg<'a> {
    pub path: Result<&'a [u8]>,
    /// Not looked at for an absolute path.
    pub at: Result<At<'a>>,
}

impl<'a> PathArg<'a> {
    /// A path whose string and start are known to be good.
    pub fn new(at: At<'a>, path: &'a [u8]) -> PathArg<'a> {
        PathArg { path: Ok(path), at: Ok(at) }
    }

    /// Steps 1 and 2 of the order: the string (also [`check_path`]'s
    /// empty and too-long checks), then, for a relative path, the start.
    pub(crate) fn begin(&self) -> Result<(&'a [u8], At<'a>)> {
        let path = self.path?;
        check_path(path)?;
        if path.first() == Some(&b'/') { Ok((path, At::Cwd)) } else { Ok((path, self.at?)) }
    }
}

/// A resolved location and, when it was found by name, the directory and
/// the name.
pub(crate) type Found = (Location, Option<(Location, Vec<u8>)>);

/// One resolution: who asks, where `/` is, how many links may still be
/// followed.
pub(crate) struct Walk<'a> {
    pub(crate) vfs: &'a Vfs,
    pub(crate) cred: &'a Cred,
    /// The process root as the context holds it.
    root: Location,
    /// The same with mounts on it followed: what `/` shows.
    root_top: Location,
    links_left: u32,
}

/// Checks a path given to a system call: non-empty (`ENOENT`) and shorter
/// than [`PATH_MAX`] (`ENAMETOOLONG`).
pub(crate) fn check_path(path: &[u8]) -> Result<()> {
    if path.is_empty() {
        Err(Errno::ENOENT)
    } else if path.len() >= PATH_MAX {
        Err(Errno::ENAMETOOLONG)
    } else {
        Ok(())
    }
}

impl<'a> Walk<'a> {
    pub(crate) fn new(vfs: &'a Vfs, cred: &'a Cred, root: Location) -> Walk<'a> {
        let root_top = vfs.follow_mounts(root.clone());
        Walk { vfs, cred, root, root_top, links_left: MAX_SYMLINKS }
    }

    /// What `/` shows for this walk.
    pub(crate) fn top_root(&self) -> &Location {
        &self.root_top
    }

    /// Resolves all but the last component of `path`, starting at `start`
    /// for a relative path, and checks the last name's length.
    pub(crate) fn parent(&mut self, start: Location, path: &[u8]) -> Result<Parent> {
        let p = self.dirs(start, path)?;
        p.check_name()?;
        Ok(p)
    }

    /// [`parent`](Self::parent) up to the last name: every directory on the
    /// way, the last name's directory included (a directory the caller may
    /// search), but not the last name's own length
    /// ([`Parent::check_name`]).
    pub(crate) fn dirs(&mut self, start: Location, path: &[u8]) -> Result<Parent> {
        let mut buf: Vec<u8> = path.to_vec();
        let mut pos = 0;
        let mut cur = if path.first() == Some(&b'/') { self.root_top.clone() } else { self.vfs.follow_mounts(start) };
        loop {
            while buf.get(pos) == Some(&b'/') {
                pos += 1;
            }
            if pos == buf.len() {
                return Ok(Parent { dir: cur, last: Last::Root, trailing_slash: true });
            }
            let end = buf[pos..].iter().position(|&c| c == b'/').map_or(buf.len(), |i| pos + i);
            let mut next = end;
            while buf.get(next) == Some(&b'/') {
                next += 1;
            }
            // Whatever comes next is looked up in `cur`: it must be a
            // directory we may search. That comes before anything about
            // the name itself, its length included: `file/<256 bytes>` is
            // ENOTDIR (the order of path errors, step 3).
            let attr = cur.attr()?;
            if attr.kind != FileType::Directory {
                return Err(Errno::ENOTDIR);
            }
            self.cred.check(&attr, Access::EXEC)?;

            let comp = &buf[pos..end];
            if next == buf.len() {
                let last = match comp {
                    b"." => Last::Dot,
                    b".." => Last::DotDot,
                    name => Last::Name(name.to_vec()),
                };
                return Ok(Parent { dir: cur, last, trailing_slash: end < buf.len() });
            }
            if comp.len() > NAME_MAX {
                return Err(Errno::ENAMETOOLONG);
            }
            match comp {
                b"." => {}
                b".." => cur = self.dotdot(cur)?,
                name => {
                    let child = self.vfs.child(&cur, name)?;
                    if child.attr()?.kind == FileType::Symlink {
                        let target = self.link_target(&child)?;
                        let mut spliced = Vec::with_capacity(target.len() + 1 + buf.len() - next);
                        spliced.extend_from_slice(&target);
                        spliced.push(b'/');
                        spliced.extend_from_slice(&buf[next..]);
                        if target[0] == b'/' {
                            cur = self.root_top.clone();
                        }
                        buf = spliced;
                        pos = 0;
                        continue;
                    }
                    cur = child;
                }
            }
            pos = next;
        }
    }

    /// Resolves `path` completely. The last component is followed if it is
    /// a symbolic link and `follow` is set or the path ends in a slash.
    pub(crate) fn resolve(&mut self, start: Location, path: &[u8], follow: bool) -> Result<Location> {
        let p = self.parent(start, path)?;
        self.finish(p, follow)
    }

    /// Resolves the last component of a [`Parent`].
    pub(crate) fn finish(&mut self, p: Parent, follow: bool) -> Result<Location> {
        self.finish_named(p, follow).map(|(loc, _)| loc)
    }

    /// Resolves `path` completely, like [`resolve`](Self::resolve), and
    /// also returns the directory and name the result was found by.
    pub(crate) fn resolve_named(&mut self, start: Location, path: &[u8], follow: bool) -> Result<Found> {
        let p = self.parent(start, path)?;
        self.finish_named(p, follow)
    }

    /// [`finish`](Self::finish), also returning the directory and name the
    /// result was looked up by (after following symbolic links); `None`
    /// when the path ended in `/`, `.` or `..`.
    pub(crate) fn finish_named(&mut self, mut p: Parent, follow: bool) -> Result<Found> {
        loop {
            let name = match p.last {
                Last::Root | Last::Dot => return Ok((p.dir, None)),
                Last::DotDot => return Ok((self.dotdot(p.dir)?, None)),
                Last::Name(name) => name,
            };
            let loc = self.vfs.child(&p.dir, &name)?;
            let kind = loc.attr()?.kind;
            if kind == FileType::Symlink && (follow || p.trailing_slash) {
                let mut target = self.link_target(&loc)?;
                if p.trailing_slash {
                    target.push(b'/');
                }
                p = self.parent(p.dir, &target)?;
                continue;
            }
            if p.trailing_slash && kind != FileType::Directory {
                return Err(Errno::ENOTDIR);
            }
            return Ok((loc, Some((p.dir, name))));
        }
    }

    /// Counts a link against the limit and reads its target.
    pub(crate) fn link_target(&mut self, link: &Location) -> Result<Vec<u8>> {
        if self.links_left == 0 {
            return Err(Errno::ELOOP);
        }
        self.links_left -= 1;
        let target = link.mount.sb.fs.readlink(link.ino)?;
        if target.is_empty() {
            return Err(Errno::ENOENT);
        }
        Ok(target)
    }

    /// The parent of `cur`, staying at the process root and climbing out of
    /// mounts.
    pub(crate) fn dotdot(&self, mut cur: Location) -> Result<Location> {
        loop {
            if cur.same(&self.root_top) || cur.same(&self.root) {
                return Ok(self.root_top.clone());
            }
            if !cur.is_mount_root() {
                break;
            }
            match &cur.mount.parent {
                Some((m, ino)) => cur = Location { mount: m.clone(), ino: *ino },
                // The topmost root, outside the process root (a chroot
                // escaped through a descriptor): stay.
                None => return Ok(cur),
            }
        }
        let parent = cur.mount.sb.lookup(cur.ino, b"..")?;
        Ok(self.vfs.follow_mounts(Location { mount: cur.mount, ino: parent }))
    }
}

//! The chain of objects from a file up to the root: what Landlock's
//! path-beneath rules are checked against (the kernel's documentation,
//! `userspace-api/landlock.rst`: a rule on a directory covers the
//! directory and everything beneath it).
//!
//! An object is named by its [`WatchKey`] (file system and inode), so a
//! rule follows its inode through renames and bind mounts. Climbing out of
//! a mount passes the mount point, which is on the chain too.

use crate::inotify::WatchKey;
use crate::node::Location;
use crate::path::{Walk, check_path};
use crate::{At, Cred, FileType, FsContext, NodeRef, Result, Vfs};
use alloc::vec::Vec;

/// What a path names, for an access check: its chain and its type.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Ancestry {
    /// The object first, then each directory above it, the root last.
    pub keys: Vec<WatchKey>,
    pub kind: FileType,
}

/// The most steps a chain takes (deeper trees are cut off at the top).
const MAX_DEPTH: usize = 4096;

fn start_of(ctx: &FsContext, at: At<'_>) -> Location {
    match at {
        At::Cwd => ctx.cwd.loc().clone(),
        At::Node(n) => n.loc().clone(),
    }
}

impl Vfs {
    /// The chain from directory `start` up to `ctx`'s root.
    fn chain_from(&self, ctx: &FsContext, start: Location, out: &mut Vec<WatchKey>) {
        let top = self.follow_mounts(ctx.root.loc().clone());
        let mut cur = start;
        for _ in 0..MAX_DEPTH {
            out.push(WatchKey::of(&cur));
            if cur.same(&top) || cur.same(ctx.root.loc()) {
                return;
            }
            if cur.is_mount_root() {
                match cur.mount.parent() {
                    Some((m, ino)) => {
                        cur = Location { mount: m, ino };
                        continue;
                    }
                    None => return,
                }
            }
            match cur.mount.sb.lookup(cur.ino, b"..") {
                Ok(p) if p != cur.ino => cur = Location { mount: cur.mount.clone(), ino: p },
                _ => return,
            }
        }
    }

    /// The chain of the object `path` names: the object, then the
    /// directory it was found in and everything above. Errors are the
    /// lookup's (`ENOENT` for a missing object).
    pub fn path_ancestry(
        &self,
        ctx: &FsContext,
        cred: &Cred,
        at: At<'_>,
        path: &[u8],
        follow: bool,
    ) -> Result<Ancestry> {
        check_path(path)?;
        let mut walk = Walk::new(self, cred, ctx.root.loc().clone());
        let (loc, named) = walk.resolve_named(start_of(ctx, at), path, follow)?;
        let kind = loc.attr()?.kind;
        let mut keys = Vec::new();
        match named {
            Some((dir, _)) if kind != FileType::Directory => {
                keys.push(WatchKey::of(&loc));
                self.chain_from(ctx, dir, &mut keys);
            }
            _ => self.chain_from(ctx, loc, &mut keys),
        }
        Ok(Ancestry { keys, kind })
    }

    /// The chain of the directory the last component of `path` is in (for
    /// a check before something is made there).
    pub fn parent_ancestry(&self, ctx: &FsContext, cred: &Cred, at: At<'_>, path: &[u8]) -> Result<Vec<WatchKey>> {
        check_path(path)?;
        let mut walk = Walk::new(self, cred, ctx.root.loc().clone());
        let p = walk.parent(start_of(ctx, at), path)?;
        let mut keys = Vec::new();
        self.chain_from(ctx, p.dir, &mut keys);
        Ok(keys)
    }

    /// The chain of `node` taken as it is: for a directory its own key and
    /// everything above; for any other file just its key (it has no
    /// parent of its own).
    pub fn node_ancestry(&self, ctx: &FsContext, node: &NodeRef) -> Ancestry {
        let kind = node.kind().unwrap_or(FileType::Regular);
        let mut keys = Vec::new();
        if kind == FileType::Directory {
            self.chain_from(ctx, node.loc().clone(), &mut keys);
        } else {
            keys.push(WatchKey::of_node(node));
        }
        Ancestry { keys, kind }
    }
}

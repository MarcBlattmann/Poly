//! Mount propagation (mount_namespaces(7), "shared subtrees"): peer groups
//! and masters, and carrying a mount or unmount below one mount to the
//! mounts that follow it.
//!
//! A shared mount belongs to a *peer group* (a number); mounts of the same
//! group are peers: a mount made or removed on a directory below one of
//! them appears below the others, at the same place. A *slave* names the
//! peer group it follows (its master): it receives that group's events and
//! sends none. A mount can be both (shared, and a slave of another group).
//! Peer groups reach across mount namespaces: a copy of a namespace keeps
//! its shared mounts in the original's groups.

use crate::mount::{Mount, MountTable, PROPAGATION_PRIVATE, PROPAGATION_SHARED};
use crate::vfs::Vfs;
use crate::{Errno, Ino, Result};
use alloc::collections::BTreeMap;
use alloc::sync::Arc;
use alloc::vec::Vec;

/// `MS_UNBINDABLE`, `MS_PRIVATE`, `MS_SLAVE`, `MS_SHARED` (`linux/mount.h`).
pub const MS_UNBINDABLE: u64 = 1 << 17;
pub const MS_PRIVATE: u64 = 1 << 18;
pub const MS_SLAVE: u64 = 1 << 19;
pub const MS_SHARED: u64 = 1 << 20;

impl MountTable {
    /// A peer group number not used yet.
    pub(crate) fn new_group(&mut self) -> u32 {
        let g = self.next_group;
        self.next_group += 1;
        g
    }

    /// The mounts that receive what happens below `pm`: its peers and the
    /// slaves of its group, the slaves of those, and so on (not `pm`).
    pub(crate) fn receivers(&self, pm: &Arc<Mount>) -> Vec<Arc<Mount>> {
        let start = pm.peer_group();
        if start == 0 {
            return Vec::new();
        }
        let mut groups = alloc::vec![start];
        let mut out: Vec<Arc<Mount>> = Vec::new();
        let mut i = 0;
        // Groups grow while mounts are found; one pass per group.
        while i < groups.len() {
            let g = groups[i];
            i += 1;
            for m in &self.mounts {
                if Arc::ptr_eq(m, pm) || out.iter().any(|o| Arc::ptr_eq(o, m)) {
                    continue;
                }
                if m.peer_group() == g || m.master_group() == g {
                    out.push(m.clone());
                    let pg = m.peer_group();
                    if pg != 0 && !groups.contains(&pg) {
                        groups.push(pg);
                    }
                }
            }
        }
        out
    }

    /// Whether a mount of `recv` can show directory `ino` (it is at or below
    /// the mount's root).
    fn covers(&self, recv: &Arc<Mount>, pm: &Arc<Mount>, ino: Ino) -> bool {
        recv.root == pm.root || is_inside(&*recv.sb.fs, ino, recv.root)
    }
}

/// Whether directory `ino` is `ancestor` or below it, following `..`.
pub(crate) fn is_inside(fs: &dyn crate::FileSystem, mut ino: Ino, ancestor: Ino) -> bool {
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

impl Mount {
    /// The peer group the mount is in (0: not shared).
    pub fn peer_group(&self) -> u32 {
        self.peer.load(core::sync::atomic::Ordering::Relaxed)
    }

    /// The peer group the mount is a slave of (0: not a slave).
    pub fn master_group(&self) -> u32 {
        self.master.load(core::sync::atomic::Ordering::Relaxed)
    }

    /// Whether a bind mount of it is refused (`MS_UNBINDABLE`).
    pub fn is_unbindable(&self) -> bool {
        self.propagation_mark.load(core::sync::atomic::Ordering::Relaxed)
    }

    pub(crate) fn set_unbindable(&self, on: bool) {
        self.propagation_mark.store(on, core::sync::atomic::Ordering::Relaxed);
    }

    pub(crate) fn set_groups(&self, peer: u32, master: u32) {
        self.peer.store(peer, core::sync::atomic::Ordering::Relaxed);
        self.master.store(master, core::sync::atomic::Ordering::Relaxed);
    }

    /// The propagation type as an `MS_*` value.
    pub(crate) fn type_bits(&self) -> u32 {
        if self.is_unbindable() {
            1 << 17
        } else if self.peer_group() != 0 {
            PROPAGATION_SHARED
        } else if self.master_group() != 0 {
            1 << 19
        } else {
            PROPAGATION_PRIVATE
        }
    }
}

impl Vfs {
    /// `mount --make-shared` and friends: gives each of `mounts` the
    /// propagation type `kind` (an `MS_SHARED`, `MS_PRIVATE`, `MS_SLAVE`
    /// or `MS_UNBINDABLE` bit; 0 changes nothing).
    pub fn set_propagation(&self, mounts: &[Arc<Mount>], kind: u64) {
        if kind == 0 {
            return;
        }
        let mut table = self.mounts.lock();
        for m in mounts {
            match kind {
                MS_SHARED => {
                    if m.peer_group() == 0 {
                        let g = table.new_group();
                        m.set_groups(g, m.master_group());
                    }
                    m.set_unbindable(false);
                }
                MS_PRIVATE => {
                    m.set_groups(0, 0);
                    m.set_unbindable(false);
                }
                MS_SLAVE => {
                    m.set_unbindable(false);
                    let peer = m.peer_group();
                    if peer != 0 {
                        // The peers it leaves become its master, or if it
                        // was alone in the group, its old master stays.
                        let others = table.mounts.iter().any(|x| !Arc::ptr_eq(x, m) && x.peer_group() == peer);
                        let master = if others { peer } else { m.master_group() };
                        m.set_groups(0, master);
                    }
                }
                MS_UNBINDABLE => {
                    m.set_groups(0, 0);
                    m.set_unbindable(true);
                }
                _ => {}
            }
        }
    }

    /// A new mount `nm` was put on directory `ino` of mount `pm`: mounts
    /// that follow `pm` get a copy of it at the same place. If `pm` is
    /// shared, so is `nm`, in a group of its own (or the one it brought).
    pub(crate) fn propagate_mount(&self, table: &mut MountTable, pm: &Arc<Mount>, ino: Ino, nm: &Arc<Mount>) {
        if pm.peer_group() == 0 {
            return;
        }
        let g_nm = if nm.peer_group() != 0 { nm.peer_group() } else { table.new_group() };
        nm.set_groups(g_nm, nm.master_group());
        let mut group_of: BTreeMap<u32, u32> = BTreeMap::new();
        group_of.insert(pm.peer_group(), g_nm);
        let recvs = table.receivers(pm);
        // The receivers come in order of discovery: a group's followers
        // after the group.
        for r in recvs {
            if !table.covers(&r, pm, ino) {
                continue;
            }
            let peer = r.peer_group();
            let own = if peer == 0 {
                0
            } else {
                match group_of.get(&peer) {
                    Some(g) => *g,
                    None => {
                        let g = table.new_group();
                        group_of.insert(peer, g);
                        g
                    }
                }
            };
            let (copy_peer, copy_master) = if peer == pm.peer_group() {
                (g_nm, nm.master_group())
            } else {
                let followed = group_of.get(&r.master_group()).copied().unwrap_or(g_nm);
                (own, followed)
            };
            let copy = table.add(nm.sb.clone(), nm.root, Some((r.clone(), ino)), nm.flags());
            copy.set_attrs(nm.attrs());
            copy.set_groups(copy_peer, copy_master);
        }
    }

    /// Mount `m`, on directory `ino` of `pm`, goes: the copies of it that
    /// propagation made below the mounts following `pm` go too (not if
    /// something is mounted on them).
    pub(crate) fn propagate_umount(&self, table: &mut MountTable, pm: &Arc<Mount>, ino: Ino, m: &Arc<Mount>) {
        if pm.peer_group() == 0 {
            return;
        }
        let mut doomed = Vec::new();
        for r in table.receivers(pm) {
            if let Some(c) = table.covering(r.id, ino)
                && Arc::ptr_eq(&c.sb, &m.sb)
                && c.root == m.root
                && !table.has_children(c.id)
            {
                doomed.push(c.id);
            }
        }
        for id in doomed {
            let _ = table.remove_tree(id);
        }
    }

    /// The group and unbindable state a bind mount of `src` starts with:
    /// a peer of a shared source, a slave of a slave's master.
    pub(crate) fn bind_inherit(&self, src: &Arc<Mount>) -> Result<(u32, u32)> {
        if src.is_unbindable() {
            return Err(Errno::EINVAL);
        }
        Ok((src.peer_group(), if src.peer_group() == 0 { src.master_group() } else { 0 }))
    }
}

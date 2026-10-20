//! User namespaces (user_namespaces(7)): id maps, the owner rules of
//! capabilities, `/proc/[pid]/uid_map`, `gid_map` and `setgroups`.
//!
//! # Ids are kernel ids
//!
//! Every id in [`Creds`] and every id the VFS stores is a *kernel* id: the
//! id as the initial namespace sees it. A namespace's maps translate
//! between its own ids and kernel ids ([`down`]: namespace id to kernel
//! id, [`up`]: back), and the system calls translate at the boundary:
//! `getuid` and friends report [`up`] (an unmapped id as [`OVERFLOW_ID`],
//! proc_sys_kernel(5)'s `overflowuid`), `setuid` and friends take [`down`]
//! (an unmapped id is `EINVAL`: "not valid in this user namespace"). The
//! initial namespace (`None`) maps every id to itself. So a process that
//! made a namespace keeps its kernel uid; writing `0 1000 1` to its
//! `uid_map` makes that kernel id 1000 read as 0 inside.
//!
//! # Capabilities belong to a namespace
//!
//! user_namespaces(7): a process "has a capability inside a user namespace
//! if it is a member of that namespace and it has the capability in its
//! effective capability set", "if a process has a capability in a user
//! namespace, then it has that capability in all child (and further
//! removed descendant) namespaces", and a process "in the parent user
//! namespace whose effective user ID matches the owner of the namespace
//! has all capabilities in the namespace" ([`ns_capable`]). A thread's
//! capability sets are those it has in its own namespace (`Creds::caps`).

use super::lock::Mutex;
use crate::cred::{Creds, cap};
use crate::errno::Errno;
use alloc::sync::Arc;
use alloc::vec::Vec;

/// The id an id without a mapping reads as (proc_sys_kernel(5):
/// `overflowuid`, `overflowgid`, 65534 by default).
pub const OVERFLOW_ID: u32 = 65534;
/// user_namespaces(7): "user namespaces can be nested ... to a maximum
/// depth of 32" (`EUSERS` beyond).
pub const MAX_DEPTH: u32 = 32;
/// The most lines a map may have (user_namespaces(7): 340 since Linux
/// 4.15).
pub const MAX_EXTENTS: usize = 340;
/// The longest write to a map file: a page.
pub const MAX_WRITE: usize = 4096;
/// `(uid_t) -1`: never a valid id.
const NO_ID: u32 = u32::MAX;

/// Which ids a map is about.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum IdKind {
    Uid,
    Gid,
}

/// One line of a map: `count` ids from `first` (inside) are the kernel ids
/// from `lower`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Extent {
    pub first: u32,
    pub lower: u32,
    pub count: u32,
}

/// A uid or gid map.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct IdMap {
    pub extents: Vec<Extent>,
}

impl IdMap {
    /// The kernel id of namespace id `id`.
    pub fn down(&self, id: u32) -> Option<u32> {
        self.extents
            .iter()
            .find(|e| id >= e.first && u64::from(id) < u64::from(e.first) + u64::from(e.count))
            .map(|e| e.lower + (id - e.first))
    }

    /// The namespace id of kernel id `kid`.
    pub fn up(&self, kid: u32) -> Option<u32> {
        self.extents
            .iter()
            .find(|e| kid >= e.lower && u64::from(kid) < u64::from(e.lower) + u64::from(e.count))
            .map(|e| e.first + (kid - e.lower))
    }
}

/// The parts of a user namespace that change after it is made.
#[derive(Debug)]
struct Maps {
    uid: Option<IdMap>,
    gid: Option<IdMap>,
    /// `/proc/[pid]/setgroups`: "allow" (true) or "deny".
    setgroups: bool,
}

/// One user namespace other than the initial one.
#[derive(Debug)]
pub struct UserNs {
    inum: u32,
    parent: UserNsRef,
    /// 1 for a child of the initial namespace.
    level: u32,
    /// The effective user and group ids (kernel ids) of the process that
    /// made it (user_namespaces(7): "the owner of the namespace").
    owner: u32,
    group: u32,
    maps: Mutex<Maps>,
}

impl PartialEq for UserNs {
    /// Namespaces are equal only to themselves.
    fn eq(&self, other: &UserNs) -> bool {
        core::ptr::eq(self, other)
    }
}

impl Eq for UserNs {}

/// A user namespace; `None` is the initial one.
pub type UserNsRef = Option<Arc<UserNs>>;

impl UserNs {
    pub fn inum(&self) -> u32 {
        self.inum
    }

    pub fn parent(&self) -> &UserNsRef {
        &self.parent
    }

    pub fn level(&self) -> u32 {
        self.level
    }

    pub fn owner(&self) -> u32 {
        self.owner
    }

    pub fn group(&self) -> u32 {
        self.group
    }

    /// A copy of a map, if it was written.
    pub fn map(&self, kind: IdKind) -> Option<IdMap> {
        let m = self.maps.lock();
        match kind {
            IdKind::Uid => m.uid.clone(),
            IdKind::Gid => m.gid.clone(),
        }
    }

    fn down(&self, kind: IdKind, id: u32) -> Option<u32> {
        let m = self.maps.lock();
        let map = match kind {
            IdKind::Uid => m.uid.as_ref(),
            IdKind::Gid => m.gid.as_ref(),
        };
        map?.down(id)
    }

    fn up(&self, kind: IdKind, kid: u32) -> Option<u32> {
        let m = self.maps.lock();
        let map = match kind {
            IdKind::Uid => m.uid.as_ref(),
            IdKind::Gid => m.gid.as_ref(),
        };
        map?.up(kid)
    }
}

/// Whether `a` and `b` are the same namespace.
pub fn same(a: &UserNsRef, b: &UserNsRef) -> bool {
    match (a, b) {
        (None, None) => true,
        (Some(a), Some(b)) => Arc::ptr_eq(a, b),
        _ => false,
    }
}

/// Whether `ancestor` is `ns` or one of its ancestors.
pub fn is_ancestor(ancestor: &UserNsRef, ns: &UserNsRef) -> bool {
    let mut n = ns.clone();
    loop {
        if same(ancestor, &n) {
            return true;
        }
        match n {
            None => return false,
            Some(x) => n = x.parent.clone(),
        }
    }
}

/// The kernel id of `ns`'s id `id`; `None` if it has no mapping.
pub fn down(ns: &UserNsRef, kind: IdKind, id: u32) -> Option<u32> {
    match ns {
        _ if id == NO_ID => None,
        None => Some(id),
        Some(n) => n.down(kind, id),
    }
}

/// `ns`'s id for kernel id `kid`; `None` if it has no mapping.
pub fn up(ns: &UserNsRef, kind: IdKind, kid: u32) -> Option<u32> {
    match ns {
        _ if kid == NO_ID => None,
        None => Some(kid),
        Some(n) => n.up(kind, kid),
    }
}

/// [`up`], with [`OVERFLOW_ID`] for an id without a mapping.
pub fn up_or_overflow(ns: &UserNsRef, kind: IdKind, kid: u32) -> u32 {
    up(ns, kind, kid).unwrap_or(OVERFLOW_ID)
}

/// The kernel ids of `count` ids of `ns` from `first`, as extents (the
/// range may cross several of `ns`'s lines); `None` if any has no
/// mapping.
fn down_range(ns: &UserNsRef, kind: IdKind, first: u32, count: u32) -> Option<Vec<(u32, u32, u32)>> {
    let Some(n) = ns else { return Some(alloc::vec![(first, first, count)]) };
    let map = n.map(kind)?;
    let mut out = Vec::new();
    let (mut id, end) = (u64::from(first), u64::from(first) + u64::from(count));
    while id < end {
        let e =
            map.extents.iter().find(|e| id >= u64::from(e.first) && id < u64::from(e.first) + u64::from(e.count))?;
        let take = (u64::from(e.first) + u64::from(e.count)).min(end) - id;
        let lower = u64::from(e.lower) + (id - u64::from(e.first));
        out.push((id as u32, lower as u32, take as u32));
        id += take;
    }
    Some(out)
}

/// user_namespaces(7)'s capability rules: whether `creds` has `cap` in
/// namespace `target`.
pub fn ns_capable(creds: &Creds, target: &UserNsRef, cap: u32) -> bool {
    let mut ns = target.clone();
    loop {
        if same(&ns, &creds.user_ns) {
            return creds.caps.effective.contains(cap);
        }
        let Some(n) = ns else { return false };
        if same(&n.parent, &creds.user_ns) && n.owner == creds.euid {
            return true;
        }
        ns = n.parent.clone();
    }
}

/// A new user namespace for a thread with `creds` (clone(2) or unshare(2)
/// with `CLONE_NEWUSER`): a child of its namespace, owned by its effective
/// ids, with no maps yet and the parent's `setgroups` setting.
/// `EUSERS` past [`MAX_DEPTH`].
pub fn create(creds: &Creds) -> Result<Arc<UserNs>, Errno> {
    let level = creds.user_ns.as_ref().map_or(0, |n| n.level) + 1;
    if level > MAX_DEPTH {
        return Err(Errno::EUSERS);
    }
    let setgroups = creds.user_ns.as_ref().is_none_or(|n| n.maps.lock().setgroups);
    Ok(Arc::new(UserNs {
        inum: super::new_inum(),
        parent: creds.user_ns.clone(),
        level,
        owner: creds.euid,
        group: creds.egid,
        maps: Mutex::new(Maps { uid: None, gid: None, setgroups }),
    }))
}

/// Whether setgroups(2) may be used in `ns`: user_namespaces(7), "the
/// setgroups(2) system call ... is not permitted" until `gid_map` was
/// written, and never after "deny" was written to `setgroups`.
pub fn setgroups_allowed(ns: &UserNsRef) -> bool {
    match ns {
        None => true,
        Some(n) => {
            let m = n.maps.lock();
            m.setgroups && m.gid.is_some()
        }
    }
}

/// The text of `target`'s `uid_map` or `gid_map` for a reader in
/// `reader`: one line per extent, the inside id, the outside id and the
/// length, each right-aligned in 10 columns. The outside id is in the
/// parent namespace if the reader is in `target`, else in the reader's
/// own namespace; extents the reader's view cannot name are left out.
/// The initial namespace's maps are `0 0 4294967295`.
pub fn format_map(target: &UserNsRef, kind: IdKind, reader: &UserNsRef) -> Vec<u8> {
    let line = |a: u32, b: u32, c: u32| alloc::format!("{a:>10} {b:>10} {c:>10}\n").into_bytes();
    let Some(t) = target else { return line(0, 0, u32::MAX) };
    let Some(map) = t.map(kind) else { return Vec::new() };
    let view = if same(target, reader) { t.parent.clone() } else { reader.clone() };
    let mut out = Vec::new();
    for e in &map.extents {
        if let Some(outside) = up(&view, kind, e.lower) {
            out.extend(line(e.first, outside, e.count));
        }
    }
    out
}

/// Parse a map write: lines of three decimal numbers. `EINVAL` for
/// anything else, an empty map, more than [`MAX_EXTENTS`] lines, a length
/// of 0, ranges that run past `u32`, or overlapping ranges inside or
/// outside (user_namespaces(7), "the rules about the format of the file").
pub fn parse_map(text: &[u8]) -> Result<Vec<Extent>, Errno> {
    let text = text.strip_suffix(b"\n").unwrap_or(text);
    let mut extents = Vec::new();
    for line in text.split(|&b| b == b'\n') {
        let mut fields = line.split(|&b| b == b' ' || b == b'\t').filter(|f| !f.is_empty());
        let mut num = || -> Result<u32, Errno> {
            let f = fields.next().ok_or(Errno::EINVAL)?;
            if !f.iter().all(u8::is_ascii_digit) {
                return Err(Errno::EINVAL);
            }
            core::str::from_utf8(f).ok().and_then(|s| s.parse().ok()).ok_or(Errno::EINVAL)
        };
        let (first, lower, count) = (num()?, num()?, num()?);
        if fields.next().is_some() || count == 0 {
            return Err(Errno::EINVAL);
        }
        let past = |start: u32| u64::from(start) + u64::from(count) > 1 << 32;
        if past(first) || past(lower) {
            return Err(Errno::EINVAL);
        }
        extents.push(Extent { first, lower, count });
        if extents.len() > MAX_EXTENTS {
            return Err(Errno::EINVAL);
        }
    }
    let overlaps = |a: u32, b: u32, n: u32, m: u32| {
        u64::from(a) < u64::from(b) + u64::from(m) && u64::from(b) < u64::from(a) + u64::from(n)
    };
    for (i, x) in extents.iter().enumerate() {
        for y in &extents[i + 1..] {
            if overlaps(x.first, y.first, x.count, y.count) || overlaps(x.lower, y.lower, x.count, y.count) {
                return Err(Errno::EINVAL);
            }
        }
    }
    Ok(extents)
}

/// A write of `data` at `offset` to `target`'s `uid_map` or `gid_map` by a
/// thread with `writer`'s credentials. user_namespaces(7), "Defining user
/// and group ID mappings: writing to uid_map and gid_map":
///
/// - the write is at offset 0, one write, at most a page (`EINVAL`), of
///   a well-formed map ([`parse_map`], `EINVAL`);
/// - "the file can be written only once" (`EPERM`);
/// - "the writing process must have the CAP_SETUID (CAP_SETGID)
///   capability in the user namespace of the process pid" and "must be
///   either in the user namespace of the process pid or be in the parent
///   user namespace" (`EPERM`);
/// - the outside ids are the parent namespace's when the writer is in
///   `target`, else the writer's own namespace's, and "must in turn have a
///   mapping in the parent user namespace" (`EPERM`);
/// - a writer with `CAP_SETUID` (`CAP_SETGID`) in the parent namespace may
///   write any such map; any other may only map its own effective id with
///   one line of length 1, must have "the same effective user ID as the
///   process that created the user namespace", and for `gid_map`
///   `setgroups` must say "deny" (`EPERM`).
///
/// The initial namespace's maps cannot be written (`EPERM`). The order of
/// the checks is Poly's (to be confirmed by the diff runner).
pub fn write_map(target: &UserNsRef, kind: IdKind, data: &[u8], offset: u64, writer: &Creds) -> Result<usize, Errno> {
    if offset != 0 || data.len() > MAX_WRITE {
        return Err(Errno::EINVAL);
    }
    let lines = parse_map(data)?;
    let Some(t) = target else { return Err(Errno::EPERM) };
    let set_already = match kind {
        IdKind::Uid => t.maps.lock().uid.is_some(),
        IdKind::Gid => t.maps.lock().gid.is_some(),
    };
    if set_already {
        return Err(Errno::EPERM);
    }
    let capability = match kind {
        IdKind::Uid => cap::SETUID,
        IdKind::Gid => cap::SETGID,
    };
    let in_target = same(target, &writer.user_ns);
    if !in_target && !same(&t.parent, &writer.user_ns) {
        return Err(Errno::EPERM);
    }
    if !ns_capable(writer, target, capability) {
        return Err(Errno::EPERM);
    }
    let outside_ns = if in_target { t.parent.clone() } else { writer.user_ns.clone() };
    let mut extents = Vec::new();
    for l in &lines {
        for (first_out, lower, count) in down_range(&outside_ns, kind, l.lower, l.count).ok_or(Errno::EPERM)? {
            extents.push(Extent { first: l.first + (first_out - l.lower), lower, count });
        }
    }
    if !ns_capable(writer, &t.parent, capability) {
        let own = match kind {
            IdKind::Uid => writer.euid,
            IdKind::Gid => writer.egid,
        };
        let one = matches!(extents.as_slice(), [e] if e.count == 1 && e.lower == own);
        if !one || writer.euid != t.owner {
            return Err(Errno::EPERM);
        }
        if kind == IdKind::Gid && t.maps.lock().setgroups {
            return Err(Errno::EPERM);
        }
    }
    let mut m = t.maps.lock();
    let slot = match kind {
        IdKind::Uid => &mut m.uid,
        IdKind::Gid => &mut m.gid,
    };
    if slot.is_some() {
        // Another writer won meanwhile.
        return Err(Errno::EPERM);
    }
    *slot = Some(IdMap { extents });
    Ok(data.len())
}

/// The text of `/proc/[pid]/setgroups`: "allow" or "deny".
pub fn format_setgroups(target: &UserNsRef) -> Vec<u8> {
    let allowed = target.as_ref().is_none_or(|t| t.maps.lock().setgroups);
    if allowed { b"allow\n".to_vec() } else { b"deny\n".to_vec() }
}

/// A write to `/proc/[pid]/setgroups`: "allow" or "deny" (a newline may
/// follow; anything else `EINVAL`), at offset 0. Only before `gid_map`
/// was written (user_namespaces(7): "writing 'deny' ... is permitted only
/// ... before ... gid_map"; afterwards, and in the initial namespace,
/// `EPERM`), by a thread with `CAP_SYS_ADMIN` in the namespace (Poly's
/// reading, to be confirmed by the diff runner).
pub fn write_setgroups(target: &UserNsRef, data: &[u8], offset: u64, writer: &Creds) -> Result<usize, Errno> {
    if offset != 0 {
        return Err(Errno::EINVAL);
    }
    let word = data.strip_suffix(b"\n").unwrap_or(data);
    let allow = match word {
        b"allow" => true,
        b"deny" => false,
        _ => return Err(Errno::EINVAL),
    };
    let Some(t) = target else { return Err(Errno::EPERM) };
    if !ns_capable(writer, target, cap::SYS_ADMIN) {
        return Err(Errno::EPERM);
    }
    let mut m = t.maps.lock();
    if m.gid.is_some() {
        return Err(Errno::EPERM);
    }
    m.setgroups = allow;
    Ok(data.len())
}

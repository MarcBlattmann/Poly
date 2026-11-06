//! NUMA memory policies: set_mempolicy(2), get_mempolicy(2), mbind(2),
//! set_mempolicy_home_node(2), migrate_pages(2) and move_pages(2).
//!
//! Poly is a machine with one NUMA node, node 0, like the machines the
//! spec sheets were observed on. Nothing ever moves, so what is left of
//! these calls is their bookkeeping and their argument rules, all here:
//!
//! - the **task policy** ([`MemPolicy`]) is per thread
//!   ([`LinuxThread::mempolicy`](crate::signal::LinuxThread)): copied to
//!   the threads and processes a thread creates, kept by execve(2);
//! - the **mapping policies** ([`RangePolicies`]) are per address space
//!   ([`LinuxProcess::mempolicies`](crate::LinuxProcess)), by page range:
//!   copied by fork(2), dropped by execve(2), and cleared for every range
//!   that is unmapped or mapped anew ([`unmapped`]), as a range's policy
//!   lives and dies with its mapping;
//! - which pages are mapped and present comes from the kernel
//!   ([`LinuxKernel::memory_regions`], [`LinuxKernel::page_query`]).
//!
//! Masks hold `maxnode - 1` bits; Poly accepts node ids up to
//! [`MAX_NUMNODES`] (the sheets' reference has 1024) and masks of up to
//! one page of bits.

use crate::abi::{PAGE_SIZE, page_aligned, page_round_up};
use crate::args::Args;
use crate::cred::{self, cap};
use crate::errno::{Errno, SysResult};
use crate::kernel::{LinuxKernel, PageQuery, UserPtr};
use crate::signal::deliver::with_own;
use crate::sys::procaccess::{self, Found};
use crate::table::{Handler, nr};
use alloc::collections::BTreeMap;
use alloc::vec::Vec;

/// The calls of this module, merged into the table (`table::GROUPS`).
pub const CALLS: &[(u64, Handler)] = &[
    (nr::MBIND, mbind),
    (nr::SET_MEMPOLICY, set_mempolicy),
    (nr::GET_MEMPOLICY, get_mempolicy),
    (nr::MIGRATE_PAGES, migrate_pages),
    (nr::MOVE_PAGES, move_pages),
    (nr::SET_MEMPOLICY_HOME_NODE, set_mempolicy_home_node),
];

// Policy modes (`linux/mempolicy.h`).
pub const MPOL_DEFAULT: u16 = 0;
pub const MPOL_PREFERRED: u16 = 1;
pub const MPOL_BIND: u16 = 2;
pub const MPOL_INTERLEAVE: u16 = 3;
pub const MPOL_LOCAL: u16 = 4;
pub const MPOL_PREFERRED_MANY: u16 = 5;
pub const MPOL_WEIGHTED_INTERLEAVE: u16 = 6;
/// One more than the highest mode (`linux/mempolicy.h`: `MPOL_MAX`).
pub const MPOL_MAX: u16 = 7;

// Mode flags, ORed into the mode (`linux/mempolicy.h`).
pub const MPOL_F_STATIC_NODES: u16 = 1 << 15;
pub const MPOL_F_RELATIVE_NODES: u16 = 1 << 14;
pub const MPOL_F_NUMA_BALANCING: u16 = 1 << 13;
/// Every mode flag (`linux/mempolicy.h`: `MPOL_MODE_FLAGS`).
pub const MPOL_MODE_FLAGS: u16 = MPOL_F_STATIC_NODES | MPOL_F_RELATIVE_NODES | MPOL_F_NUMA_BALANCING;

// get_mempolicy(2) flags (`linux/mempolicy.h`).
pub const MPOL_F_NODE: u64 = 1 << 0;
pub const MPOL_F_ADDR: u64 = 1 << 1;
pub const MPOL_F_MEMS_ALLOWED: u64 = 1 << 2;

// mbind(2) / move_pages(2) flags (`linux/mempolicy.h`).
pub const MPOL_MF_STRICT: u64 = 1 << 0;
pub const MPOL_MF_MOVE: u64 = 1 << 1;
pub const MPOL_MF_MOVE_ALL: u64 = 1 << 2;

/// Node ids Poly accepts in a mask (bits at or above are `EINVAL`): the
/// sheets' reference value (Poly's choice of the configurable limit).
pub const MAX_NUMNODES: usize = 1024;
/// The longs of a full node mask.
pub const MASK_LONGS: usize = MAX_NUMNODES / 64;
/// The largest `maxnode`: one page of bits, plus one.
pub const MAX_MAXNODE: u64 = PAGE_SIZE * 8 + 1;

/// A set of node ids, bit `n` for node `n`.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct NodeMask(pub [u64; MASK_LONGS]);

impl NodeMask {
    /// The nodes that exist: node 0 only.
    pub const ONLINE: NodeMask = {
        let mut m = [0; MASK_LONGS];
        m[0] = 1;
        NodeMask(m)
    };

    pub fn is_empty(&self) -> bool {
        self.0.iter().all(|&w| w == 0)
    }

    /// The nodes of the mask that exist.
    pub fn existing(&self) -> NodeMask {
        let mut m = *self;
        for (w, online) in m.0.iter_mut().zip(NodeMask::ONLINE.0) {
            *w &= online;
        }
        m
    }

    /// Whether every node of the mask is in `other`.
    pub fn within(&self, other: &NodeMask) -> bool {
        self.0.iter().zip(other.0).all(|(&a, b)| a & !b == 0)
    }
}

/// A memory policy: of a thread (set_mempolicy(2)) or a page range
/// (mbind(2)). The default value is `MPOL_DEFAULT`.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct MemPolicy {
    /// `MPOL_*`, below [`MPOL_MAX`].
    pub mode: u16,
    /// The mode flags kept with it ([`MPOL_MODE_FLAGS`]).
    pub flags: u16,
    /// The mask as the caller gave it.
    pub given: NodeMask,
    /// set_mempolicy_home_node(2)'s node, not visible to get_mempolicy.
    pub home: Option<u32>,
}

impl MemPolicy {
    /// The mode get_mempolicy(2) reports: the mode ORed with its flags.
    pub fn reported_mode(&self) -> i32 {
        i32::from(self.mode | self.flags)
    }

    /// The mask get_mempolicy(2) reports: the caller's with
    /// `MPOL_F_STATIC_NODES` or `MPOL_F_RELATIVE_NODES`, else the
    /// existing nodes of it (for `MPOL_PREFERRED` the first one, which
    /// with one node is the same), none for `MPOL_DEFAULT` and
    /// `MPOL_LOCAL`.
    pub fn reported_mask(&self) -> NodeMask {
        if self.flags & (MPOL_F_STATIC_NODES | MPOL_F_RELATIVE_NODES) != 0 {
            return self.given;
        }
        match self.mode {
            MPOL_DEFAULT | MPOL_LOCAL => NodeMask::default(),
            _ => self.given.existing(),
        }
    }
}

/// The mapping policies of an address space: page-aligned, disjoint
/// ranges with a policy each (ranges without one are `MPOL_DEFAULT`).
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct RangePolicies {
    /// By start address: the end and the policy.
    ranges: BTreeMap<u64, (u64, MemPolicy)>,
}

impl RangePolicies {
    pub fn is_empty(&self) -> bool {
        self.ranges.is_empty()
    }

    /// The policy of the page holding `addr`, if it has one.
    pub fn get(&self, addr: u64) -> Option<&MemPolicy> {
        let (_, (end, policy)) = self.ranges.range(..=addr).next_back()?;
        (addr < *end).then_some(policy)
    }

    /// The ranges with a policy that overlap `[start, end)`, clipped to
    /// it, in address order.
    pub fn in_range(&self, start: u64, end: u64) -> Vec<(u64, u64, MemPolicy)> {
        let first = self.ranges.range(..start).next_back().filter(|(_, (e, _))| *e > start).map(|(&s, _)| s);
        let from = first.unwrap_or(start);
        self.ranges.range(from..end).map(|(&s, &(e, p))| (s.max(start), e.min(end), p)).filter(|r| r.0 < r.1).collect()
    }

    /// Give `[start, end)` the policy `policy` (`None`: no policy of its
    /// own), splitting the ranges it cuts.
    pub fn set(&mut self, start: u64, end: u64, policy: Option<MemPolicy>) {
        if start >= end {
            return;
        }
        let cut: Vec<u64> = {
            let first = self.ranges.range(..start).next_back().filter(|(_, (e, _))| *e > start).map(|(&s, _)| s);
            self.ranges.range(first.unwrap_or(start)..end).map(|(&s, _)| s).collect()
        };
        for s in cut {
            let (e, p) = self.ranges.remove(&s).expect("listed above");
            if s < start {
                self.ranges.insert(s, (start, p));
            }
            if e > end {
                self.ranges.insert(end, (e, p));
            }
        }
        if let Some(p) = policy {
            self.ranges.insert(start, (end, p));
        }
    }

    /// Forget every policy (execve(2): a new address space).
    pub fn clear(&mut self) {
        self.ranges.clear();
    }
}

/// `[start, start + len)` of the caller's address space was unmapped or
/// mapped anew: its ranges lose their policies.
pub fn unmapped(k: &mut dyn LinuxKernel, start: u64, len: u64) {
    let end = start.saturating_add(len);
    with_own(k, |p| {
        if !p.mempolicies.is_empty() {
            p.mempolicies.set(start, end, None);
        }
    });
}

/// The calling thread's policy.
fn task_policy(k: &mut dyn LinuxKernel) -> MemPolicy {
    let tid = k.tid();
    with_own(k, |p| p.thread(tid).mempolicy)
}

/// A mode argument split into its mode and flags. `EINVAL` for an
/// unknown mode, unknown flag bits, or both `MPOL_F_STATIC_NODES` and
/// `MPOL_F_RELATIVE_NODES`.
pub fn split_mode(raw: i32) -> Result<(u16, u16), Errno> {
    let flags = (raw as u32 & u32::from(MPOL_MODE_FLAGS)) as u16;
    let mode = raw & !i32::from(MPOL_MODE_FLAGS);
    if !(0..i32::from(MPOL_MAX)).contains(&mode) {
        return Err(Errno::EINVAL);
    }
    if flags & MPOL_F_STATIC_NODES != 0 && flags & MPOL_F_RELATIVE_NODES != 0 {
        return Err(Errno::EINVAL);
    }
    Ok((mode as u16, flags))
}

/// Check a `maxnode` argument: at most [`MAX_MAXNODE`].
fn check_maxnode(maxnode: u64) -> Result<(), Errno> {
    if maxnode > MAX_MAXNODE { Err(Errno::EINVAL) } else { Ok(()) }
}

/// Read a node mask of `maxnode - 1` bits from `at` (null, or no bits:
/// empty). `EINVAL` for a `maxnode` above [`MAX_MAXNODE`] or a node at or
/// above [`MAX_NUMNODES`]; `EFAULT`.
pub fn read_mask(k: &mut dyn LinuxKernel, at: UserPtr, maxnode: u64) -> Result<NodeMask, Errno> {
    check_maxnode(maxnode)?;
    let bits = maxnode.saturating_sub(1);
    let mut mask = NodeMask::default();
    if at.is_null() || bits == 0 {
        return Ok(mask);
    }
    let longs = bits.div_ceil(64) as usize;
    let mut raw = alloc::vec![0u8; longs * 8];
    k.copy_from_user(at, &mut raw)?;
    for (i, chunk) in raw.chunks_exact(8).enumerate() {
        let mut word = u64::from_le_bytes(chunk.try_into().expect("8 bytes"));
        let used = bits - i as u64 * 64;
        if used < 64 {
            word &= (1 << used) - 1;
        }
        match mask.0.get_mut(i) {
            Some(w) => *w = word,
            None if word != 0 => return Err(Errno::EINVAL),
            None => {}
        }
    }
    Ok(mask)
}

/// The policy `mode` (with `flags`) and `mask` make, by the rules of
/// set_mempolicy(2) and mbind(2): `MPOL_DEFAULT` takes no mask and drops
/// the flags; `MPOL_LOCAL` takes neither; `MPOL_PREFERRED` without a mask
/// is `MPOL_LOCAL`; the other modes need a mask. A mask must name a node
/// that exists, but with `MPOL_F_RELATIVE_NODES` (relative node numbers
/// fold onto the existing nodes). The mode flags need a mask;
/// `MPOL_F_NUMA_BALANCING` goes with `MPOL_BIND` and
/// `MPOL_PREFERRED_MANY` only. `EINVAL` otherwise.
pub fn make_policy(mode: u16, flags: u16, mask: NodeMask) -> Result<MemPolicy, Errno> {
    let nodes = |flags: u16| {
        if mask.is_empty() || (flags & MPOL_F_RELATIVE_NODES == 0 && mask.existing().is_empty()) {
            Err(Errno::EINVAL)
        } else {
            Ok(MemPolicy { mode, flags, given: mask, home: None })
        }
    };
    let balancing = flags & MPOL_F_NUMA_BALANCING != 0;
    match mode {
        MPOL_DEFAULT if mask.is_empty() => Ok(MemPolicy::default()),
        MPOL_LOCAL if mask.is_empty() && flags == 0 => Ok(MemPolicy { mode, ..MemPolicy::default() }),
        MPOL_DEFAULT | MPOL_LOCAL => Err(Errno::EINVAL),
        MPOL_PREFERRED if balancing => Err(Errno::EINVAL),
        MPOL_PREFERRED if mask.is_empty() && flags == 0 => Ok(MemPolicy { mode: MPOL_LOCAL, ..MemPolicy::default() }),
        MPOL_INTERLEAVE | MPOL_WEIGHTED_INTERLEAVE if balancing => Err(Errno::EINVAL),
        _ => nodes(flags),
    }
}

/// `CAP_SYS_NICE` in the calling thread's effective set.
fn may_nice(k: &mut dyn LinuxKernel) -> bool {
    cred::current(k).capable(cap::SYS_NICE)
}

/// set_mempolicy(mode, nodemask, maxnode): the calling thread's policy.
/// Checks: the mode and its flags, `maxnode`, the mask (`EFAULT`), the
/// mask rules ([`make_policy`]); a failed call keeps the old policy.
pub fn set_mempolicy(k: &mut dyn LinuxKernel, a: &Args) -> SysResult {
    let (mode, flags) = split_mode(a.int(0))?;
    let mask = read_mask(k, a.ptr(1), a.ulong(2))?;
    let policy = make_policy(mode, flags, mask)?;
    let tid = k.tid();
    with_own(k, |p| p.thread_mut(tid).mempolicy = policy);
    Ok(0)
}

/// Write `mask` as `maxnode - 1` bits to `at` (whole longs; nothing for
/// `maxnode` 1).
fn write_mask(k: &mut dyn LinuxKernel, at: UserPtr, maxnode: u64, mask: &NodeMask) -> Result<(), Errno> {
    let bits = maxnode.saturating_sub(1);
    let longs = bits.div_ceil(64) as usize;
    if at.is_null() || longs == 0 {
        return Ok(());
    }
    let mut raw = alloc::vec![0u8; longs * 8];
    for (i, chunk) in raw.chunks_exact_mut(8).enumerate() {
        let mut word = mask.0.get(i).copied().unwrap_or(0);
        let used = bits - i as u64 * 64;
        if used < 64 {
            word &= (1 << used) - 1;
        }
        chunk.copy_from_slice(&word.to_le_bytes());
    }
    k.copy_to_user(at, &raw)
}

/// Whether the page holding `addr` is mapped in the caller's memory.
fn mapped(k: &mut dyn LinuxKernel, addr: u64) -> Result<bool, Errno> {
    let pid = k.pid();
    let page = addr & !(PAGE_SIZE - 1);
    Ok(!k.memory_regions(pid, page, page.saturating_add(PAGE_SIZE))?.is_empty())
}

/// get_mempolicy(mode, nodemask, maxnode, addr, flags): the calling
/// thread's policy, or with `MPOL_F_ADDR` that of the page at `addr`
/// (mappings without one: `MPOL_DEFAULT`, whatever the thread's).
/// `MPOL_F_MEMS_ALLOWED`: the nodes the process may use, `mode` left
/// alone. `MPOL_F_NODE`: with `MPOL_F_ADDR` the node of the page at
/// `addr`, else (under `MPOL_INTERLEAVE` only) the next interleave node:
/// node 0 either way. `mode` and `nodemask` may be null; `nodemask` gets
/// `maxnode - 1` bits in whole longs.
///
/// Checks, in this order (the sheet's errors; the order is Poly's): the
/// flags (unknown bits; `MPOL_F_MEMS_ALLOWED` with `MPOL_F_ADDR` or
/// `MPOL_F_NODE`; an address without `MPOL_F_ADDR`; `MPOL_F_NODE` alone
/// without an interleave policy), `maxnode` with a mask (0, or above
/// [`MAX_MAXNODE`]), an unmapped `addr` (`EFAULT`); then the writes
/// (`EFAULT`).
pub fn get_mempolicy(k: &mut dyn LinuxKernel, a: &Args) -> SysResult {
    let (mode_at, mask_at, maxnode, addr, flags) = (a.ptr(0), a.ptr(1), a.ulong(2), a.ulong(3), a.ulong(4));
    if flags & !(MPOL_F_NODE | MPOL_F_ADDR | MPOL_F_MEMS_ALLOWED) != 0 {
        return Err(Errno::EINVAL);
    }
    let maxnode_ok = || {
        if !mask_at.is_null() && (maxnode == 0 || maxnode > MAX_MAXNODE) { Err(Errno::EINVAL) } else { Ok(()) }
    };
    if flags & MPOL_F_MEMS_ALLOWED != 0 {
        if flags & (MPOL_F_NODE | MPOL_F_ADDR) != 0 {
            return Err(Errno::EINVAL);
        }
        maxnode_ok()?;
        write_mask(k, mask_at, maxnode, &NodeMask::ONLINE)?;
        return Ok(0);
    }
    if flags & MPOL_F_ADDR == 0 && addr != 0 {
        return Err(Errno::EINVAL);
    }
    let task = task_policy(k);
    if flags & (MPOL_F_ADDR | MPOL_F_NODE) == MPOL_F_NODE && task.mode != MPOL_INTERLEAVE {
        return Err(Errno::EINVAL);
    }
    maxnode_ok()?;
    let policy = if flags & MPOL_F_ADDR != 0 {
        if !mapped(k, addr)? {
            return Err(Errno::EFAULT);
        }
        with_own(k, |p| p.mempolicies.get(addr).copied()).unwrap_or_default()
    } else {
        task
    };
    // MPOL_F_NODE: the page's node, or the next interleave node; with
    // one node both are node 0.
    let mode = if flags & MPOL_F_NODE != 0 { 0 } else { policy.reported_mode() };
    if !mode_at.is_null() {
        k.copy_to_user(mode_at, &mode.to_le_bytes())?;
    }
    write_mask(k, mask_at, maxnode, &policy.reported_mask())?;
    Ok(0)
}

/// Check that every page of `[start, end)` of the caller's memory is
/// mapped (`EFAULT` otherwise).
fn check_mapped(k: &mut dyn LinuxKernel, start: u64, end: u64) -> Result<(), Errno> {
    let pid = k.pid();
    let mut at = start;
    for r in k.memory_regions(pid, start, end)? {
        if r.start > at {
            return Err(Errno::EFAULT);
        }
        at = at.max(r.end);
    }
    if at < end { Err(Errno::EFAULT) } else { Ok(()) }
}

/// mbind(addr, len, mode, nodemask, maxnode, flags): the policy of the
/// pages `[addr, addr + len)` (`len` rounded up to pages; 0 does
/// nothing), read back with get_mempolicy(`MPOL_F_ADDR`).
/// `MPOL_DEFAULT` removes the range's policy. `MPOL_MF_STRICT`,
/// `MPOL_MF_MOVE` and `MPOL_MF_MOVE_ALL` (which needs `CAP_SYS_NICE`)
/// move nothing: every page already is on node 0.
///
/// Checks, in this order (Poly's): the mode, the mask (`EINVAL` for
/// `maxnode`, `EFAULT`), the flags, `MPOL_MF_MOVE_ALL` (`EPERM`), the
/// alignment of `addr` and a range that wraps (`EINVAL`), the mask rules,
/// then every page of the range mapped (`EFAULT`). A failed call changes
/// nothing.
pub fn mbind(k: &mut dyn LinuxKernel, a: &Args) -> SysResult {
    let (addr, length, maxnode, flags) = (a.ulong(0), a.ulong(1), a.ulong(4), a.ulong(5));
    let (mode, mode_flags) = split_mode(a.int(2))?;
    let mask = read_mask(k, a.ptr(3), maxnode)?;
    if flags & !(MPOL_MF_STRICT | MPOL_MF_MOVE | MPOL_MF_MOVE_ALL) != 0 {
        return Err(Errno::EINVAL);
    }
    if flags & MPOL_MF_MOVE_ALL != 0 && !may_nice(k) {
        return Err(Errno::EPERM);
    }
    if !page_aligned(addr) {
        return Err(Errno::EINVAL);
    }
    let len = page_round_up(length).ok_or(Errno::EINVAL)?;
    let end = addr.checked_add(len).ok_or(Errno::EINVAL)?;
    if len == 0 {
        return Ok(0);
    }
    let policy = make_policy(mode, mode_flags, mask)?;
    check_mapped(k, addr, end)?;
    let policy = (policy.mode != MPOL_DEFAULT).then_some(policy);
    with_own(k, |p| p.mempolicies.set(addr, end, policy));
    Ok(0)
}

/// set_mempolicy_home_node(start, len, home_node, flags): the home node
/// of the mapping policies in `[start, start + len)`. Only
/// `MPOL_BIND` and `MPOL_PREFERRED_MANY` take one; the ranges are walked
/// in order, ranges without a policy skipped, and the first with another
/// policy ends the call with `EOPNOTSUPP` (those before it keep the
/// home node). `ENOENT` if no range took it; `len` 0 returns 0. The
/// thread's own policy plays no part.
///
/// Checks first: the flags (0), the node (it must exist), the alignment
/// of `start` and a range that wraps (`EINVAL`).
pub fn set_mempolicy_home_node(k: &mut dyn LinuxKernel, a: &Args) -> SysResult {
    let (start, length, home, flags) = (a.ulong(0), a.ulong(1), a.ulong(2), a.ulong(3));
    if flags != 0 || home != 0 {
        return Err(Errno::EINVAL);
    }
    if !page_aligned(start) {
        return Err(Errno::EINVAL);
    }
    let len = page_round_up(length).ok_or(Errno::EINVAL)?;
    let end = start.checked_add(len).ok_or(Errno::EINVAL)?;
    if len == 0 {
        return Ok(0);
    }
    with_own(k, |p| {
        let mut took = 0;
        for (s, e, mut policy) in p.mempolicies.in_range(start, end) {
            if !matches!(policy.mode, MPOL_BIND | MPOL_PREFERRED_MANY) {
                return Err(Errno::EOPNOTSUPP);
            }
            policy.home = Some(home as u32);
            p.mempolicies.set(s, e, Some(policy));
            took += 1;
        }
        if took == 0 { Err(Errno::ENOENT) } else { Ok(0) }
    })
}

/// The process a migrate_pages / move_pages `pid` names (0: the caller),
/// as a global id, after the caller's right to inspect it (ptrace(2)
/// access with real credentials, [`procaccess::may_access`]: `EPERM`).
/// `ESRCH` if there is no such live process.
fn target(k: &mut dyn LinuxKernel, pid: i32) -> Result<Option<Found>, Errno> {
    if pid == 0 {
        return Ok(None);
    }
    let found = procaccess::find(k, pid, false)?;
    if found.pid == k.pid() {
        return Ok(None);
    }
    let c = cred::current(k);
    if !procaccess::may_access(k.pid(), c.uid, c.gid, c.capable(cap::SYS_PTRACE), &found) {
        return Err(Errno::EPERM);
    }
    Ok(Some(found))
}

/// migrate_pages(pid, maxnode, old_nodes, new_nodes): move the pages of
/// process `pid` (0: the caller) on `old_nodes` to `new_nodes`; returns
/// the number of pages that could not be moved, always 0 with one node.
///
/// Checks, in this order (the sheet's, where it gives one): `maxnode`
/// (0, or above [`MAX_MAXNODE`]: `EINVAL`), both masks (`EFAULT`), the
/// process (`ESRCH`), access to it (`EPERM`; another user's needs
/// `CAP_SYS_NICE` too: migrate_pages(2) wants the caller's real or
/// effective uid to be the target's real or saved one otherwise),
/// `new_nodes` outside the caller's allowed nodes without
/// `CAP_SYS_NICE` (`EPERM`), `new_nodes` without an existing node
/// (`EINVAL`).
pub fn migrate_pages(k: &mut dyn LinuxKernel, a: &Args) -> SysResult {
    let maxnode = a.ulong(1);
    if maxnode == 0 {
        return Err(Errno::EINVAL);
    }
    let _old = read_mask(k, a.ptr(2), maxnode)?;
    let new = read_mask(k, a.ptr(3), maxnode)?;
    let nice = may_nice(k);
    if let Some(found) = target(k, a.int(0))? {
        let c = cred::current(k);
        let t = &found.info;
        let same_user = [c.uid, c.euid].iter().any(|&id| id == t.uid || id == t.suid);
        if !same_user && !nice {
            return Err(Errno::EPERM);
        }
    }
    if !nice && !new.within(&NodeMask::ONLINE) {
        return Err(Errno::EPERM);
    }
    if new.existing().is_empty() {
        return Err(Errno::EINVAL);
    }
    Ok(0)
}

/// Pages move_pages(2) handles at a time.
const MOVE_CHUNK: u64 = 256;

/// move_pages(2)'s status for what is at a page: the node (0), or
/// `-ENOENT` for a page not brought in yet, `-EFAULT` for no mapping or a
/// page of private memory only read so far.
fn page_status(q: PageQuery) -> i32 {
    match q {
        PageQuery::Present => 0,
        PageQuery::Absent => -i32::from(Errno::ENOENT.get()),
        PageQuery::Unmapped | PageQuery::Zero => -i32::from(Errno::EFAULT.get()),
    }
}

/// move_pages(pid, count, pages, nodes, status, flags): with `nodes`
/// null, where the pages at the `count` addresses in `pages` are; else
/// move `pages[i]` to `nodes[i]` (nothing moves with one node). Either
/// way `status[i]` gets the page's node or a negative errno
/// ([`page_status`]) and the call returns 0.
///
/// Checks, in this order (Poly's): the flags (`MPOL_MF_MOVE` and
/// `MPOL_MF_MOVE_ALL` only), `MPOL_MF_MOVE_ALL` without `CAP_SYS_NICE`
/// (`EPERM`), the process (`ESRCH`) and access to it (`EPERM`); `count` 0
/// then returns 0. Every entry of `nodes` must exist (`ENODEV`, before
/// any status is written); the arrays are read and written a chunk at a
/// time (`EFAULT`).
pub fn move_pages(k: &mut dyn LinuxKernel, a: &Args) -> SysResult {
    let (count, pages, nodes, status, flags) = (a.ulong(1), a.ptr(2), a.ptr(3), a.ptr(4), a.int(5));
    if flags & !((MPOL_MF_MOVE | MPOL_MF_MOVE_ALL) as i32) != 0 {
        return Err(Errno::EINVAL);
    }
    if flags & MPOL_MF_MOVE_ALL as i32 != 0 && !may_nice(k) {
        return Err(Errno::EPERM);
    }
    let pid = target(k, a.int(0))?.map_or(k.pid(), |f| f.pid);
    if count == 0 {
        return Ok(0);
    }
    let at = |base: UserPtr, i: u64, size: u64| i.checked_mul(size).and_then(|o| base.offset(o)).ok_or(Errno::EFAULT);
    if !nodes.is_null() {
        let mut done = 0;
        while done < count {
            let n = (count - done).min(MOVE_CHUNK);
            let mut raw = alloc::vec![0u8; n as usize * 4];
            k.copy_from_user(at(nodes, done, 4)?, &mut raw)?;
            if raw.chunks_exact(4).any(|c| i32::from_le_bytes(c.try_into().expect("4 bytes")) != 0) {
                return Err(Errno::ENODEV);
            }
            done += n;
        }
    }
    let mut done = 0;
    while done < count {
        let n = (count - done).min(MOVE_CHUNK);
        let mut raw = alloc::vec![0u8; n as usize * 8];
        k.copy_from_user(at(pages, done, 8)?, &mut raw)?;
        let mut out = Vec::with_capacity(n as usize * 4);
        for c in raw.chunks_exact(8) {
            let addr = u64::from_le_bytes(c.try_into().expect("8 bytes"));
            let q = k.page_query(pid, addr)?;
            out.extend_from_slice(&page_status(q).to_le_bytes());
        }
        k.copy_to_user(at(status, done, 4)?, &out)?;
        done += n;
    }
    Ok(0)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn mask(bits: &[usize]) -> NodeMask {
        let mut m = NodeMask::default();
        for &b in bits {
            m.0[b / 64] |= 1 << (b % 64);
        }
        m
    }

    #[test]
    fn modes_and_flags_split() {
        assert_eq!(split_mode(2), Ok((MPOL_BIND, 0)));
        assert_eq!(split_mode(0x8002), Ok((MPOL_BIND, MPOL_F_STATIC_NODES)));
        assert_eq!(split_mode(0x2005), Ok((MPOL_PREFERRED_MANY, MPOL_F_NUMA_BALANCING)));
        for bad in [7, 99, -1, 0x1002, 0x10002, 0xc002] {
            assert_eq!(split_mode(bad), Err(Errno::EINVAL), "{bad:#x}");
        }
    }

    #[test]
    fn policies_follow_the_mask_rules() {
        let none = NodeMask::default();
        assert_eq!(make_policy(MPOL_DEFAULT, MPOL_F_STATIC_NODES, none), Ok(MemPolicy::default()));
        assert_eq!(make_policy(MPOL_DEFAULT, 0, mask(&[0])), Err(Errno::EINVAL));
        assert_eq!(make_policy(MPOL_LOCAL, 0, mask(&[0])), Err(Errno::EINVAL));
        assert_eq!(make_policy(MPOL_PREFERRED, 0, none).unwrap().mode, MPOL_LOCAL);
        assert_eq!(make_policy(MPOL_PREFERRED, MPOL_F_STATIC_NODES, none), Err(Errno::EINVAL));
        assert_eq!(make_policy(MPOL_PREFERRED, MPOL_F_NUMA_BALANCING, mask(&[0])), Err(Errno::EINVAL));
        for mode in [MPOL_BIND, MPOL_INTERLEAVE, MPOL_PREFERRED_MANY, MPOL_WEIGHTED_INTERLEAVE] {
            assert_eq!(make_policy(mode, 0, none), Err(Errno::EINVAL));
            assert_eq!(make_policy(mode, 0, mask(&[1])), Err(Errno::EINVAL), "only a missing node");
            assert_eq!(make_policy(mode, MPOL_F_STATIC_NODES, mask(&[1])), Err(Errno::EINVAL));
            assert!(make_policy(mode, MPOL_F_RELATIVE_NODES, mask(&[1])).is_ok());
            assert!(make_policy(mode, 0, mask(&[0, 1])).is_ok());
        }
        assert!(make_policy(MPOL_BIND, MPOL_F_NUMA_BALANCING | MPOL_F_STATIC_NODES, mask(&[0])).is_ok());
        assert_eq!(make_policy(MPOL_INTERLEAVE, MPOL_F_NUMA_BALANCING, mask(&[0])), Err(Errno::EINVAL));
    }

    #[test]
    fn reported_masks() {
        let p = make_policy(MPOL_BIND, 0, mask(&[0, 1])).unwrap();
        assert_eq!((p.reported_mode(), p.reported_mask()), (2, mask(&[0])));
        let p = make_policy(MPOL_BIND, MPOL_F_STATIC_NODES, mask(&[0, 1])).unwrap();
        assert_eq!((p.reported_mode(), p.reported_mask()), (0x8002, mask(&[0, 1])));
        let p = make_policy(MPOL_INTERLEAVE, MPOL_F_RELATIVE_NODES, mask(&[3])).unwrap();
        assert_eq!((p.reported_mode(), p.reported_mask()), (0x4003, mask(&[3])));
        let p = make_policy(MPOL_PREFERRED, 0, mask(&[0, 1])).unwrap();
        assert_eq!(p.reported_mask(), mask(&[0]));
    }

    #[test]
    fn range_policies_split_and_clear() {
        let bind = MemPolicy { mode: MPOL_BIND, ..MemPolicy::default() };
        let inter = MemPolicy { mode: MPOL_INTERLEAVE, ..MemPolicy::default() };
        let mut r = RangePolicies::default();
        r.set(0x1000, 0x6000, Some(bind));
        r.set(0x3000, 0x4000, Some(inter));
        assert_eq!(r.get(0x2fff).map(|p| p.mode), Some(MPOL_BIND));
        assert_eq!(r.get(0x3000).map(|p| p.mode), Some(MPOL_INTERLEAVE));
        assert_eq!(r.get(0x4000).map(|p| p.mode), Some(MPOL_BIND));
        assert_eq!(r.get(0x6000), None);
        assert_eq!(r.get(0x0fff), None);
        let modes: Vec<(u64, u64, u16)> = r.in_range(0x2000, 0x5000).iter().map(|&(s, e, p)| (s, e, p.mode)).collect();
        assert_eq!(
            modes,
            [(0x2000, 0x3000, MPOL_BIND), (0x3000, 0x4000, MPOL_INTERLEAVE), (0x4000, 0x5000, MPOL_BIND)]
        );
        r.set(0x2000, 0x5000, None);
        assert_eq!(r.get(0x3000), None);
        assert_eq!(r.get(0x1000).map(|p| p.mode), Some(MPOL_BIND));
        assert_eq!(r.get(0x5000).map(|p| p.mode), Some(MPOL_BIND));
        r.clear();
        assert!(r.is_empty());
    }

    #[test]
    fn node_masks() {
        assert!(mask(&[0]).within(&NodeMask::ONLINE));
        assert!(!mask(&[0, 1]).within(&NodeMask::ONLINE));
        assert!(NodeMask::default().within(&NodeMask::ONLINE));
        assert_eq!(mask(&[0, 5, 700]).existing(), mask(&[0]));
    }
}

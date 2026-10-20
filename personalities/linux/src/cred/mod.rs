//! Credentials: user and group ids, supplementary groups and capabilities
//! of one thread, and every rule that changes them (ADR 0045).
//!
//! Sources: credentials(7), capabilities(7), setuid(2), setgid(2),
//! setreuid(2), setresuid(2), setfsuid(2), setfsgid(2), getgroups(2),
//! capget(2), prctl(2), execve(2) and path_resolution(7); constants from
//! the UAPI headers `linux/capability.h`, `linux/prctl.h` and
//! `linux/securebits.h`. No Linux or C library source was read.
//!
//! # Per thread, as the kernel keeps them
//!
//! credentials(7): "At the kernel level, user IDs and group IDs are a
//! per-thread attribute. However, POSIX requires that the credentials are
//! shared by all of the threads in a process"; the C library makes them so
//! by running the change on every thread (nptl(7): "the NPTL
//! implementation ... uses a signal-based technique"). Poly is the kernel
//! side: each [`LinuxThread`](crate::signal::LinuxThread) has its own
//! [`Creds`], a new thread (clone(2), with or without `CLONE_THREAD`) and
//! a forked child start with a copy of their creator's, and the
//! `set*id` calls change only the calling thread's.
//!
//! # Privilege is capabilities
//!
//! Every check that the man pages describe as "privileged (Linux: has the
//! CAP_... capability)" tests that capability in the effective set
//! ([`Creds::capable`]). Root is user 0 with a full permitted and
//! effective set; the rules of capabilities(7) "Effect of user ID changes
//! on capabilities" and "Capabilities and execution of programs by root"
//! keep the two in step. There are no file capabilities and no user
//! namespaces: every id is valid except `(uid_t) -1`.

mod exec;
#[cfg(test)]
mod tests;

pub use exec::{ExecFile, S_ISGID, S_ISUID, S_IXGRP};

use crate::errno::Errno;
use crate::kernel::{Credentials, LinuxKernel};
use crate::ns::user::{self, IdKind, UserNsRef};
use crate::signal::deliver::with_own;
use alloc::sync::Arc;

/// Capability numbers (UAPI `linux/capability.h`).
pub mod cap {
    pub const CHOWN: u32 = 0;
    pub const DAC_OVERRIDE: u32 = 1;
    pub const DAC_READ_SEARCH: u32 = 2;
    pub const FOWNER: u32 = 3;
    pub const FSETID: u32 = 4;
    pub const KILL: u32 = 5;
    pub const SETGID: u32 = 6;
    pub const SETUID: u32 = 7;
    pub const SETPCAP: u32 = 8;
    pub const LINUX_IMMUTABLE: u32 = 9;
    pub const NET_BIND_SERVICE: u32 = 10;
    pub const NET_BROADCAST: u32 = 11;
    pub const NET_ADMIN: u32 = 12;
    pub const NET_RAW: u32 = 13;
    pub const IPC_LOCK: u32 = 14;
    pub const IPC_OWNER: u32 = 15;
    pub const SYS_MODULE: u32 = 16;
    pub const SYS_RAWIO: u32 = 17;
    pub const SYS_CHROOT: u32 = 18;
    pub const SYS_PTRACE: u32 = 19;
    pub const SYS_PACCT: u32 = 20;
    pub const SYS_ADMIN: u32 = 21;
    pub const SYS_BOOT: u32 = 22;
    pub const SYS_NICE: u32 = 23;
    pub const SYS_RESOURCE: u32 = 24;
    pub const SYS_TIME: u32 = 25;
    pub const SYS_TTY_CONFIG: u32 = 26;
    pub const MKNOD: u32 = 27;
    pub const LEASE: u32 = 28;
    pub const AUDIT_WRITE: u32 = 29;
    pub const AUDIT_CONTROL: u32 = 30;
    pub const SETFCAP: u32 = 31;
    pub const MAC_OVERRIDE: u32 = 32;
    pub const MAC_ADMIN: u32 = 33;
    pub const SYSLOG: u32 = 34;
    pub const WAKE_ALARM: u32 = 35;
    pub const BLOCK_SUSPEND: u32 = 36;
    pub const AUDIT_READ: u32 = 37;
    pub const PERFMON: u32 = 38;
    pub const BPF: u32 = 39;
    pub const CHECKPOINT_RESTORE: u32 = 40;
    /// `CAP_LAST_CAP`.
    pub const LAST: u32 = CHECKPOINT_RESTORE;

    /// Whether `cap` names a capability (`cap_valid`).
    pub const fn valid(cap: u32) -> bool {
        cap <= LAST
    }
}

/// A set of capabilities: bit `n` is capability `n`.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub struct CapSet(pub u64);

impl CapSet {
    pub const EMPTY: CapSet = CapSet(0);
    /// Every capability up to [`cap::LAST`].
    pub const ALL: CapSet = CapSet((1 << (cap::LAST + 1)) - 1);
    /// The capabilities a change of the file system user id from 0 drops
    /// from the effective set, and a change to 0 raises from the permitted
    /// set (capabilities(7), "Effect of user ID changes on capabilities").
    pub const FS: CapSet = CapSet(
        1 << cap::CHOWN
            | 1 << cap::DAC_OVERRIDE
            | 1 << cap::DAC_READ_SEARCH
            | 1 << cap::FOWNER
            | 1 << cap::FSETID
            | 1 << cap::LINUX_IMMUTABLE
            | 1 << cap::MAC_OVERRIDE
            | 1 << cap::MKNOD,
    );

    pub const fn only(cap: u32) -> CapSet {
        CapSet(1 << cap)
    }

    /// The set from capget(2)'s two 32-bit words (low, high), with bits of
    /// capabilities that do not exist dropped.
    pub const fn from_words(low: u32, high: u32) -> CapSet {
        CapSet((low as u64 | (high as u64) << 32) & CapSet::ALL.0)
    }

    /// The low and high 32-bit words capget(2) reports.
    pub const fn words(self) -> (u32, u32) {
        (self.0 as u32, (self.0 >> 32) as u32)
    }

    pub const fn contains(self, cap: u32) -> bool {
        cap::valid(cap) && self.0 & (1 << cap) != 0
    }

    pub const fn with(self, cap: u32) -> CapSet {
        CapSet(self.0 | 1 << cap)
    }

    pub const fn without(self, cap: u32) -> CapSet {
        CapSet(self.0 & !(1 << cap))
    }

    pub const fn union(self, other: CapSet) -> CapSet {
        CapSet(self.0 | other.0)
    }

    pub const fn intersection(self, other: CapSet) -> CapSet {
        CapSet(self.0 & other.0)
    }

    /// The capabilities of `self` that are not in `other`.
    pub const fn minus(self, other: CapSet) -> CapSet {
        CapSet(self.0 & !other.0)
    }

    pub const fn is_subset(self, other: CapSet) -> bool {
        self.0 & !other.0 == 0
    }

    pub const fn is_empty(self) -> bool {
        self.0 == 0
    }
}

/// The capability sets of a thread (capabilities(7), "Thread capability
/// sets").
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Caps {
    /// What the kernel checks.
    pub effective: CapSet,
    /// The limit of `effective` and of what may be added to `inheritable`.
    pub permitted: CapSet,
    /// Preserved across execve(2); with file capabilities (which Poly does
    /// not have) or as root it feeds the new permitted set.
    pub inheritable: CapSet,
    /// What execve(2) may give at most; can only shrink
    /// (`PR_CAPBSET_DROP`).
    pub bounding: CapSet,
    /// Kept across execve(2) of a program that is not privileged; always a
    /// subset of `permitted` and `inheritable`.
    pub ambient: CapSet,
}

impl Caps {
    /// Root's: every capability permitted and effective.
    pub const ROOT: Caps = Caps {
        effective: CapSet::ALL,
        permitted: CapSet::ALL,
        inheritable: CapSet::EMPTY,
        bounding: CapSet::ALL,
        ambient: CapSet::EMPTY,
    };

    /// None, with the full bounding set.
    pub const NONE: Caps = Caps { effective: CapSet::EMPTY, permitted: CapSet::EMPTY, ..Caps::ROOT };

    /// capabilities(7): "The ambient capability set obeys the invariant
    /// that no capability can ever be ambient if it is not both permitted
    /// and inheritable."
    fn fix_ambient(&mut self) {
        self.ambient = self.ambient.intersection(self.permitted).intersection(self.inheritable);
    }
}

/// The ids and capabilities a file permission check uses (the VFS's
/// credentials): normally the file system ids and the effective
/// capabilities ([`Creds::fs_ids`]); access(2) uses the real ids
/// ([`Creds::access_ids`]).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct FsIds {
    pub uid: u32,
    pub gid: u32,
    pub groups: Arc<[u32]>,
    pub caps: CapSet,
}

/// `NGROUPS_MAX` (UAPI `linux/limits.h`): the most supplementary groups.
pub const NGROUPS_MAX: usize = 65536;

/// `(uid_t) -1` / `(gid_t) -1`: "unchanged" for the calls that take it,
/// an invalid id for the others.
pub const NO_ID: u32 = u32::MAX;

/// An id argument where `-1` means "leave unchanged".
pub const fn optional_id(raw: u32) -> Option<u32> {
    if raw == NO_ID { None } else { Some(raw) }
}

/// One thread's credentials (credentials(7)).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Creds {
    /// Real, effective, saved set- and file system user ids.
    pub uid: u32,
    pub euid: u32,
    pub suid: u32,
    pub fsuid: u32,
    /// The same for groups.
    pub gid: u32,
    pub egid: u32,
    pub sgid: u32,
    pub fsgid: u32,
    /// Supplementary group ids, as setgroups(2) set them.
    pub groups: Arc<[u32]>,
    pub caps: Caps,
    /// `SECBIT_KEEP_CAPS` (`PR_SET_KEEPCAPS`): keep the permitted set when
    /// every user id leaves 0. Cleared by execve(2).
    pub keep_caps: bool,
    /// `PR_SET_NO_NEW_PRIVS`: execve(2) grants nothing (set-user-ID and
    /// set-group-ID bits are ignored). Inherited, never cleared.
    pub no_new_privs: bool,
    /// The thread's user namespace (`None`: the initial one; ADR 0054).
    /// The ids above are kernel ids; the capability sets are what the
    /// thread has in this namespace ([`crate::ns::user`]).
    pub user_ns: UserNsRef,
}

impl Default for Creds {
    /// Root's: what the kernel's first processes start with.
    fn default() -> Creds {
        Creds::root()
    }
}

impl Creds {
    /// Every id 0, every capability.
    pub fn root() -> Creds {
        Creds::with_ids(0, 0, Caps::ROOT)
    }

    /// Every user id `uid`, every group id `gid`, no supplementary groups
    /// and no capabilities: what root becomes after setting all its ids.
    pub fn user(uid: u32, gid: u32) -> Creds {
        Creds::with_ids(uid, gid, Caps::NONE)
    }

    fn with_ids(uid: u32, gid: u32, caps: Caps) -> Creds {
        Creds {
            uid,
            euid: uid,
            suid: uid,
            fsuid: uid,
            gid,
            egid: gid,
            sgid: gid,
            fsgid: gid,
            groups: Arc::from([]),
            caps,
            keep_caps: false,
            no_new_privs: false,
            user_ns: None,
        }
    }

    /// Whether the thread has `cap` in its effective set: what "privileged
    /// (Linux: has the CAP_... capability)" means in the man pages, in the
    /// thread's own user namespace. For a resource of another namespace
    /// use [`crate::ns::user::ns_capable`]; for one only the initial
    /// namespace may change, [`Creds::capable_initial`].
    pub fn capable(&self, cap: u32) -> bool {
        self.caps.effective.contains(cap)
    }

    /// `cap` in the initial user namespace: for what belongs to the whole
    /// system (the clocks, rebooting, resource limits above the hard
    /// ones, disks).
    pub fn capable_initial(&self, cap: u32) -> bool {
        crate::ns::user::ns_capable(self, &None, cap)
    }

    /// The kernel id of the namespace's user 0, if it has one: the "root"
    /// of the rules of capabilities(7) inside the thread's namespace.
    pub fn root_uid(&self) -> Option<u32> {
        user::down(&self.user_ns, IdKind::Uid, 0)
    }

    fn is_root(&self, uid: u32) -> bool {
        self.root_uid() == Some(uid)
    }

    /// Kernel user id `kid` as the thread's namespace sees it (65534
    /// without a mapping).
    pub fn uid_view(&self, kid: u32) -> u32 {
        user::up_or_overflow(&self.user_ns, IdKind::Uid, kid)
    }

    /// Kernel group id `kid` as the thread's namespace sees it.
    pub fn gid_view(&self, kid: u32) -> u32 {
        user::up_or_overflow(&self.user_ns, IdKind::Gid, kid)
    }

    /// The kernel id of the namespace's user id `uid` (`EINVAL` without a
    /// mapping: setuid(2) "The user ID specified in uid is not valid in
    /// this user namespace").
    pub fn kuid(&self, uid: u32) -> Result<u32, Errno> {
        user::down(&self.user_ns, IdKind::Uid, uid).ok_or(Errno::EINVAL)
    }

    /// The same for a group id.
    pub fn kgid(&self, gid: u32) -> Result<u32, Errno> {
        user::down(&self.user_ns, IdKind::Gid, gid).ok_or(Errno::EINVAL)
    }

    /// Real and effective ids, as getuid(2) and friends report them.
    pub fn summary(&self) -> Credentials {
        Credentials { uid: self.uid, euid: self.euid, gid: self.gid, egid: self.egid }
    }

    /// Who file permission checks see: file system ids, supplementary
    /// groups, effective capabilities. In a user namespace other than the
    /// initial one only the capabilities the personality has checked
    /// itself in the right namespace reach the VFS (`CAP_SYS_ADMIN` for
    /// mounts, `CAP_SYS_CHROOT`): Poly does not yet apply capabilities(7)'s
    /// rule that the file capabilities of a namespace act only on files
    /// whose owner and group are mapped in it, so it applies none.
    pub fn fs_ids(&self) -> FsIds {
        FsIds {
            uid: self.fsuid,
            gid: self.fsgid,
            groups: Arc::clone(&self.groups),
            caps: self.vfs_caps(self.caps.effective),
        }
    }

    fn vfs_caps(&self, caps: CapSet) -> CapSet {
        if self.user_ns.is_none() {
            caps
        } else {
            caps.intersection(CapSet::only(cap::SYS_ADMIN).with(cap::SYS_CHROOT))
        }
    }

    /// Who access(2) checks as: "The check is done using the calling
    /// process's real UID and GID, rather than the effective IDs ...
    /// Similarly, for the root user, the check uses the set of permitted
    /// capabilities rather than the set of effective capabilities; and for
    /// non-root users, the check uses an empty set of capabilities."
    pub fn access_ids(&self) -> FsIds {
        let caps = if self.is_root(self.uid) { self.vfs_caps(self.caps.permitted) } else { CapSet::EMPTY };
        FsIds { uid: self.uid, gid: self.gid, groups: Arc::clone(&self.groups), caps }
    }

    // ---- user ids -------------------------------------------------------

    /// setuid(2): with `CAP_SETUID` "the real UID and saved set-user-ID
    /// are also set"; otherwise only the effective id changes, to the real
    /// or saved id (`EPERM` for anything else). `EINVAL` for `-1`.
    pub fn set_uid(&mut self, uid: u32) -> Result<(), Errno> {
        if uid == NO_ID {
            return Err(Errno::EINVAL);
        }
        if self.capable(cap::SETUID) {
            self.change_uids(uid, uid, uid)
        } else if uid == self.uid || uid == self.suid {
            self.change_uids(self.uid, uid, self.suid)
        } else {
            Err(Errno::EPERM)
        }
    }

    /// setreuid(2), `None` for `-1`: "Unprivileged processes may only set
    /// the effective user ID to the real user ID, the effective user ID,
    /// or the saved set-user-ID. Unprivileged users may only set the real
    /// user ID to the real user ID or the effective user ID. If the real
    /// user ID is set (i.e., ruid is not -1) or the effective user ID is
    /// set to a value not equal to the previous real user ID, the saved
    /// set-user-ID will be set to the new effective user ID."
    pub fn set_reuid(&mut self, ruid: Option<u32>, euid: Option<u32>) -> Result<(), Errno> {
        let (r, e, s) = re_ids(self.capable(cap::SETUID), (self.uid, self.euid, self.suid), ruid, euid)?;
        self.change_uids(r, e, s)
    }

    /// setresuid(2), `None` for `-1`: unprivileged, each id may become one
    /// of the current real, effective or saved ids; with `CAP_SETUID`
    /// anything.
    pub fn set_resuid(&mut self, ruid: Option<u32>, euid: Option<u32>, suid: Option<u32>) -> Result<(), Errno> {
        let (r, e, s) = res_ids(self.capable(cap::SETUID), (self.uid, self.euid, self.suid), ruid, euid, suid)?;
        self.change_uids(r, e, s)
    }

    /// setfsuid(2): "setfsuid() will succeed only if the caller is the
    /// superuser or if fsuid matches either the caller's real user ID,
    /// effective user ID, saved set-user-ID, or current filesystem user
    /// ID." Returns the previous file system user id either way; `-1`
    /// changes nothing (the usual way to read it).
    pub fn set_fsuid(&mut self, fsuid: u32) -> u32 {
        let old = self.fsuid;
        let allowed = [self.uid, self.euid, self.suid, self.fsuid].contains(&fsuid) || self.capable(cap::SETUID);
        if fsuid != NO_ID && allowed {
            self.fsuid = fsuid;
            self.fix_fs_caps(old);
        }
        old
    }

    /// Install new real, effective and saved user ids (the file system id
    /// follows the effective one) and apply capabilities(7), "Effect of
    /// user ID changes on capabilities".
    fn change_uids(&mut self, uid: u32, euid: u32, suid: u32) -> Result<(), Errno> {
        let old = (self.uid, self.euid, self.suid, self.fsuid);
        self.uid = uid;
        self.euid = euid;
        self.suid = suid;
        self.fsuid = euid;
        let root_uid = self.root_uid();
        let root = |id: u32| root_uid == Some(id);
        let c = &mut self.caps;
        // 1. "If one or more of the real, effective, or saved set user IDs
        //    was previously 0, and as a result of the UID changes all of
        //    these IDs have a nonzero value, then all capabilities are
        //    cleared from the permitted, effective, and ambient capability
        //    sets" (SECBIT_KEEP_CAPS keeps the permitted set, and the
        //    effective one unless rule 2 applies; the ambient set goes
        //    either way: to be confirmed by the diff runner).
        let was_root = root(old.0) || root(old.1) || root(old.2);
        if was_root && !root(uid) && !root(euid) && !root(suid) {
            if !self.keep_caps {
                c.permitted = CapSet::EMPTY;
                c.effective = CapSet::EMPTY;
            }
            c.ambient = CapSet::EMPTY;
        }
        // 2. "If the effective user ID is changed from 0 to nonzero, then
        //    all capabilities are cleared from the effective set."
        if root(old.1) && !root(euid) {
            c.effective = CapSet::EMPTY;
        }
        // 3. "If the effective user ID is changed from nonzero to 0, then
        //    the permitted set is copied to the effective set."
        if !root(old.1) && root(euid) {
            c.effective = c.permitted;
        }
        // 4. The file system user id, which followed the effective one.
        self.fix_fs_caps(old.3);
        self.caps.fix_ambient();
        Ok(())
    }

    /// capabilities(7), rule 4: "If the filesystem user ID is changed from
    /// 0 to nonzero (see setfsuid(2)), then the following capabilities are
    /// cleared from the effective set: CAP_CHOWN, CAP_DAC_OVERRIDE,
    /// CAP_DAC_READ_SEARCH, CAP_FOWNER, CAP_FSETID, CAP_LINUX_IMMUTABLE
    /// (since Linux 2.6.30), CAP_MAC_OVERRIDE, and CAP_MKNOD (since Linux
    /// 2.6.30). If the filesystem UID is changed from nonzero to 0, then
    /// any of these capabilities that are enabled in the permitted set are
    /// enabled in the effective set."
    fn fix_fs_caps(&mut self, old_fsuid: u32) {
        let (was, is) = (self.is_root(old_fsuid), self.is_root(self.fsuid));
        let c = &mut self.caps;
        if was && !is {
            c.effective = c.effective.minus(CapSet::FS);
        } else if !was && is {
            c.effective = c.effective.union(c.permitted.intersection(CapSet::FS));
        }
    }

    // ---- group ids ------------------------------------------------------

    /// setgid(2): like [`Creds::set_uid`] with `CAP_SETGID`.
    pub fn set_gid(&mut self, gid: u32) -> Result<(), Errno> {
        if gid == NO_ID {
            return Err(Errno::EINVAL);
        }
        if self.capable(cap::SETGID) {
            self.change_gids(gid, gid, gid);
            Ok(())
        } else if gid == self.gid || gid == self.sgid {
            self.change_gids(self.gid, gid, self.sgid);
            Ok(())
        } else {
            Err(Errno::EPERM)
        }
    }

    /// setregid(2): "Completely analogously" to [`Creds::set_reuid`].
    pub fn set_regid(&mut self, rgid: Option<u32>, egid: Option<u32>) -> Result<(), Errno> {
        let (r, e, s) = re_ids(self.capable(cap::SETGID), (self.gid, self.egid, self.sgid), rgid, egid)?;
        self.change_gids(r, e, s);
        Ok(())
    }

    /// setresgid(2): like [`Creds::set_resuid`] with `CAP_SETGID`.
    pub fn set_resgid(&mut self, rgid: Option<u32>, egid: Option<u32>, sgid: Option<u32>) -> Result<(), Errno> {
        let (r, e, s) = res_ids(self.capable(cap::SETGID), (self.gid, self.egid, self.sgid), rgid, egid, sgid)?;
        self.change_gids(r, e, s);
        Ok(())
    }

    /// setfsgid(2): like [`Creds::set_fsuid`] with the group ids and
    /// `CAP_SETGID`; no capability changes.
    pub fn set_fsgid(&mut self, fsgid: u32) -> u32 {
        let old = self.fsgid;
        let allowed = [self.gid, self.egid, self.sgid, self.fsgid].contains(&fsgid) || self.capable(cap::SETGID);
        if fsgid != NO_ID && allowed {
            self.fsgid = fsgid;
        }
        old
    }

    fn change_gids(&mut self, gid: u32, egid: u32, sgid: u32) {
        self.gid = gid;
        self.egid = egid;
        self.sgid = sgid;
        self.fsgid = egid;
    }

    /// setgroups(2): `CAP_SETGID` (`EPERM`), at most [`NGROUPS_MAX`]
    /// (`EINVAL`), no `-1` among them (`EINVAL`: not a valid id). The
    /// order is kept as given (getgroups(2) reports it back).
    pub fn set_groups(&mut self, groups: &[u32]) -> Result<(), Errno> {
        if groups.len() > NGROUPS_MAX || groups.contains(&NO_ID) {
            return Err(Errno::EINVAL);
        }
        if !self.capable(cap::SETGID) {
            return Err(Errno::EPERM);
        }
        self.groups = Arc::from(groups);
        Ok(())
    }

    // ---- capabilities ---------------------------------------------------

    /// capset(2) on the calling thread. "EPERM The caller attempted to add
    /// a capability to the permitted set, or to set a capability in the
    /// effective set that is not in the permitted set"; adding to the
    /// inheritable set needs the capability permitted (or `CAP_SETPCAP`)
    /// and, either way, in the bounding set (capabilities(7)). The
    /// ambient set loses what is no longer permitted and inheritable.
    pub fn set_caps(&mut self, effective: CapSet, permitted: CapSet, inheritable: CapSet) -> Result<(), Errno> {
        let c = &self.caps;
        let inheritable_limit = if self.capable(cap::SETPCAP) { CapSet::ALL } else { c.inheritable.union(c.permitted) };
        if !inheritable.is_subset(inheritable_limit)
            || !inheritable.is_subset(c.inheritable.union(c.bounding))
            || !permitted.is_subset(c.permitted)
            || !effective.is_subset(permitted)
        {
            return Err(Errno::EPERM);
        }
        self.caps.effective = effective;
        self.caps.permitted = permitted;
        self.caps.inheritable = inheritable;
        self.caps.fix_ambient();
        Ok(())
    }

    /// `PR_CAPBSET_DROP`: `EINVAL` for an unknown capability, `EPERM`
    /// without `CAP_SETPCAP`.
    pub fn drop_bounding(&mut self, cap: u32) -> Result<(), Errno> {
        if !cap::valid(cap) {
            return Err(Errno::EINVAL);
        }
        if !self.capable(cap::SETPCAP) {
            return Err(Errno::EPERM);
        }
        self.caps.bounding = self.caps.bounding.without(cap);
        Ok(())
    }

    /// `PR_CAP_AMBIENT_RAISE`: `EINVAL` for an unknown capability, `EPERM`
    /// if it is not both permitted and inheritable.
    pub fn raise_ambient(&mut self, cap: u32) -> Result<(), Errno> {
        if !cap::valid(cap) {
            return Err(Errno::EINVAL);
        }
        let c = &mut self.caps;
        if !c.permitted.contains(cap) || !c.inheritable.contains(cap) {
            return Err(Errno::EPERM);
        }
        c.ambient = c.ambient.with(cap);
        Ok(())
    }

    /// `PR_CAP_AMBIENT_LOWER`: `EINVAL` for an unknown capability.
    pub fn lower_ambient(&mut self, cap: u32) -> Result<(), Errno> {
        if !cap::valid(cap) {
            return Err(Errno::EINVAL);
        }
        self.caps.ambient = self.caps.ambient.without(cap);
        Ok(())
    }
}

/// The calling thread's credentials.
pub fn current(k: &mut dyn LinuxKernel) -> Creds {
    let tid = k.tid();
    with_own(k, |p| p.thread(tid).creds.clone())
}

/// Change the calling thread's credentials with `f`, which leaves them
/// alone when it fails.
pub fn change<R>(k: &mut dyn LinuxKernel, f: impl FnOnce(&mut Creds) -> R) -> R {
    let tid = k.tid();
    with_own(k, |p| f(&mut p.thread_mut(tid).creds))
}

/// setreuid(2) / setregid(2) on `(real, effective, saved)`.
fn re_ids(
    privileged: bool,
    (r, e, s): (u32, u32, u32),
    new_r: Option<u32>,
    new_e: Option<u32>,
) -> Result<(u32, u32, u32), Errno> {
    if !privileged && (new_r.is_some_and(|x| x != r && x != e) || new_e.is_some_and(|x| x != r && x != e && x != s)) {
        return Err(Errno::EPERM);
    }
    let nr = new_r.unwrap_or(r);
    let ne = new_e.unwrap_or(e);
    let ns = if new_r.is_some() || new_e.is_some_and(|x| x != r) { ne } else { s };
    Ok((nr, ne, ns))
}

/// setresuid(2) / setresgid(2) on `(real, effective, saved)`.
fn res_ids(
    privileged: bool,
    (r, e, s): (u32, u32, u32),
    new_r: Option<u32>,
    new_e: Option<u32>,
    new_s: Option<u32>,
) -> Result<(u32, u32, u32), Errno> {
    let ok = |x: Option<u32>| x.is_none_or(|x| x == r || x == e || x == s);
    if !privileged && !(ok(new_r) && ok(new_e) && ok(new_s)) {
        return Err(Errno::EPERM);
    }
    Ok((new_r.unwrap_or(r), new_e.unwrap_or(e), new_s.unwrap_or(s)))
}

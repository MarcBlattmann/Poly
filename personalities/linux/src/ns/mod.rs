//! Linux namespaces (namespaces(7)): which user, pid, mount, network, UTS
//! and IPC namespace a process is in, and the rules of clone(2),
//! unshare(2) and setns(2) that change that (ADR 0054).
//!
//! - **Where they live.** A thread's user namespace is part of its
//!   credentials ([`Creds::user_ns`](crate::cred::Creds::user_ns): it
//!   decides what its capabilities mean, [`user`]); the other namespaces
//!   of a process are in its [`LinuxProcess`]'s [`Namespaces`], shared by
//!   its threads (Poly keeps them per process, where Linux has them per
//!   thread; unshare(2) of them in a process with several threads changes
//!   them for all, and setns(2) of a mount namespace needs a single
//!   thread). `None` is the initial namespace of each kind, so a system
//!   that never makes one pays nothing.
//! - **What each one holds.** [`user::UserNs`]: id maps, owner. [`pid::PidNs`]:
//!   the numbers of its members. [`UtsNs`]: host and domain names. The
//!   kernel holds what the others stand for: a mount namespace is a copy
//!   of the mount table ([`MntNs::id`] names it), a network namespace has
//!   only a loopback interface that is down, so its sockets reach nothing
//!   ([`NetNs`]), an IPC namespace is empty (Poly has no System V IPC
//!   yet). Cgroup and time namespaces are not supported: their flags are
//!   `EINVAL`, as clone(2) and unshare(2) document for a kernel built
//!   without them.
//! - **Identity.** Each namespace has an inode number ([`inum`]):
//!   `/proc/[pid]/ns/<kind>` reads as `<kind>:[<inum>]` and a descriptor
//!   on it names the namespace for setns(2) ([`lookup`]). The numbers are
//!   Poly's: the initial namespaces' fixed, later ones counted up from
//!   [`FIRST_INUM`].
//!
//! [`LinuxProcess`]: crate::signal::LinuxProcess

pub mod lock;
pub mod pid;
pub mod user;

use crate::cred::{Caps, Creds, cap};
use crate::errno::Errno;
use crate::task::clone::*;
use alloc::collections::BTreeMap;
use alloc::sync::{Arc, Weak};
use alloc::vec::Vec;
use core::sync::atomic::{AtomicU32, Ordering};
use lock::Mutex;
use pid::{PidNs, PidNsRef};
use user::{UserNs, UserNsRef};

/// The kinds of namespace, as `/proc/[pid]/ns` names them.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub enum Kind {
    Ipc,
    Mnt,
    Net,
    Pid,
    /// The pid namespace the process's children go to (`pid_for_children`).
    PidForChildren,
    User,
    Uts,
}

impl Kind {
    /// Every kind, in the order `/proc/[pid]/ns` lists them.
    pub const ALL: [Kind; 7] = [Kind::Ipc, Kind::Mnt, Kind::Net, Kind::Pid, Kind::PidForChildren, Kind::User, Kind::Uts];

    /// The entry's name in `/proc/[pid]/ns`.
    pub const fn entry(self) -> &'static str {
        match self {
            Kind::Ipc => "ipc",
            Kind::Mnt => "mnt",
            Kind::Net => "net",
            Kind::Pid => "pid",
            Kind::PidForChildren => "pid_for_children",
            Kind::User => "user",
            Kind::Uts => "uts",
        }
    }

    /// The type in the link's text (`net:[...]`): `pid` for both pid
    /// entries.
    pub const fn type_name(self) -> &'static str {
        match self {
            Kind::PidForChildren => "pid",
            k => k.entry(),
        }
    }

    /// The `CLONE_NEW*` flag of the kind (setns(2)'s `nstype`).
    pub const fn flag(self) -> u64 {
        match self {
            Kind::Ipc => CLONE_NEWIPC,
            Kind::Mnt => CLONE_NEWNS,
            Kind::Net => CLONE_NEWNET,
            Kind::Pid | Kind::PidForChildren => CLONE_NEWPID,
            Kind::User => CLONE_NEWUSER,
            Kind::Uts => CLONE_NEWUTS,
        }
    }

    /// The kind a link text's type names (`pid` is [`Kind::Pid`]).
    pub fn from_type_name(name: &[u8]) -> Option<Kind> {
        Kind::ALL.into_iter().filter(|k| *k != Kind::PidForChildren).find(|k| k.type_name().as_bytes() == name)
    }

    /// A small number for each kind (inode numbers of the initial
    /// namespaces, procfs's encodings).
    pub const fn index(self) -> u32 {
        self as u32
    }

    pub fn from_index(i: u32) -> Option<Kind> {
        Kind::ALL.into_iter().find(|k| k.index() == i)
    }
}

/// The inode numbers of the initial namespaces: Poly's, below the dynamic
/// ones.
pub const fn initial_inum(kind: Kind) -> u32 {
    let k = match kind {
        Kind::PidForChildren => Kind::Pid,
        k => k,
    };
    0xefff_fff0 + k.index()
}

/// Where the numbers of new namespaces start.
pub const FIRST_INUM: u32 = 0xf000_0000;

static NEXT_INUM: AtomicU32 = AtomicU32::new(FIRST_INUM);

/// A fresh inode number for a new namespace.
pub fn new_inum() -> u32 {
    NEXT_INUM.fetch_add(1, Ordering::Relaxed)
}

/// What every namespace other than a user namespace has: its number and
/// the user namespace that owns it (the creator's, namespaces(7)).
#[derive(Debug)]
pub struct Common {
    inum: u32,
    owner: UserNsRef,
}

impl Common {
    fn new(owner: &UserNsRef) -> Common {
        Common { inum: new_inum(), owner: owner.clone() }
    }
}

/// A UTS namespace (uts_namespaces(7)): its host and NIS domain names.
#[derive(Debug)]
pub struct UtsNs {
    common: Common,
    names: Mutex<UtsNames>,
}

/// The two names a UTS namespace keeps.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct UtsNames {
    pub hostname: Vec<u8>,
    pub domainname: Vec<u8>,
}

impl UtsNs {
    pub fn names(&self) -> UtsNames {
        self.names.lock().clone()
    }

    pub fn set_hostname(&self, name: &[u8]) {
        self.names.lock().hostname = name.to_vec();
    }

    pub fn set_domainname(&self, name: &[u8]) {
        self.names.lock().domainname = name.to_vec();
    }
}

/// An IPC namespace (ipc_namespaces(7)). Poly has no System V IPC or POSIX
/// message queues yet, so it holds nothing.
#[derive(Debug)]
pub struct IpcNs {
    common: Common,
}

/// A network namespace (network_namespaces(7)): a new one has only a
/// loopback interface, which is down, and no addresses; the kernel's
/// sockets made in it reach nothing outside it.
#[derive(Debug)]
pub struct NetNs {
    common: Common,
}

/// A mount namespace (mount_namespaces(7)): the kernel's copy of the mount
/// table it names.
#[derive(Debug)]
pub struct MntNs {
    common: Common,
    id: u32,
}

impl MntNs {
    /// A mount namespace the kernel made (its table is `id`), owned by
    /// `owner`.
    pub fn new(owner: &UserNsRef, id: u32) -> Arc<MntNs> {
        let ns = Arc::new(MntNs { common: Common::new(owner), id });
        register(ns.common.inum, Handle::Mnt(Some(ns.clone())));
        ns
    }

    /// The kernel's name for the mount table.
    pub fn id(&self) -> u32 {
        self.id
    }
}

macro_rules! namespace {
    ($($t:ty),*) => {$(
        impl PartialEq for $t {
            fn eq(&self, other: &$t) -> bool {
                core::ptr::eq(self, other)
            }
        }
        impl Eq for $t {}
        impl $t {
            pub fn inum(&self) -> u32 {
                self.common.inum
            }
            pub fn owner(&self) -> &UserNsRef {
                &self.common.owner
            }
        }
    )*};
}

namespace!(UtsNs, IpcNs, NetNs, MntNs);

/// The namespaces of a process other than its user namespace; `None` is
/// the initial namespace of the kind.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Namespaces {
    pub uts: Option<Arc<UtsNs>>,
    pub ipc: Option<Arc<IpcNs>>,
    pub net: Option<Arc<NetNs>>,
    pub mnt: Option<Arc<MntNs>>,
    /// The process's own pid namespace, fixed for its life.
    pub pid: PidNsRef,
    /// Where its children go (unshare(2) and setns(2) change it).
    pub pid_for_children: PidNsRef,
}

impl Namespaces {
    /// What a child process of fork(2) starts with: the same namespaces,
    /// its own pid namespace being the parent's `pid_for_children`.
    pub fn for_child(&self) -> Namespaces {
        Namespaces { pid: self.pid_for_children.clone(), ..self.clone() }
    }

    /// The inode number of the kind's namespace (`user` from `user_ns`).
    pub fn inum(&self, kind: Kind, user_ns: &UserNsRef) -> u32 {
        let dynamic = match kind {
            Kind::Ipc => self.ipc.as_ref().map(|n| n.inum()),
            Kind::Mnt => self.mnt.as_ref().map(|n| n.inum()),
            Kind::Net => self.net.as_ref().map(|n| n.inum()),
            Kind::Pid => self.pid.as_ref().map(|n| n.inum()),
            Kind::PidForChildren => self.pid_for_children.as_ref().map(|n| n.inum()),
            Kind::User => user_ns.as_ref().map(|n| n.inum()),
            Kind::Uts => self.uts.as_ref().map(|n| n.inum()),
        };
        dynamic.unwrap_or(initial_inum(kind))
    }

    /// The user namespace that owns the mount namespace (mount(2)'s
    /// privilege is `CAP_SYS_ADMIN` there).
    pub fn mnt_owner(&self) -> UserNsRef {
        self.mnt.as_ref().and_then(|m| m.owner().clone())
    }

    pub fn uts_owner(&self) -> UserNsRef {
        self.uts.as_ref().and_then(|m| m.owner().clone())
    }

    pub fn net_owner(&self) -> UserNsRef {
        self.net.as_ref().and_then(|m| m.owner().clone())
    }
}

/// A namespace as setns(2) finds it.
#[derive(Clone, Debug)]
pub enum Handle {
    User(UserNsRef),
    Pid(PidNsRef),
    Uts(Option<Arc<UtsNs>>),
    Ipc(Option<Arc<IpcNs>>),
    Net(Option<Arc<NetNs>>),
    Mnt(Option<Arc<MntNs>>),
}

impl Handle {
    pub fn kind(&self) -> Kind {
        match self {
            Handle::User(_) => Kind::User,
            Handle::Pid(_) => Kind::Pid,
            Handle::Uts(_) => Kind::Uts,
            Handle::Ipc(_) => Kind::Ipc,
            Handle::Net(_) => Kind::Net,
            Handle::Mnt(_) => Kind::Mnt,
        }
    }

    /// The user namespace that owns it (a user namespace: itself).
    pub fn owner(&self) -> UserNsRef {
        match self {
            Handle::User(n) => n.clone(),
            Handle::Pid(n) => n.as_ref().and_then(|n| n.owner().clone()),
            Handle::Uts(n) => n.as_ref().and_then(|n| n.owner().clone()),
            Handle::Ipc(n) => n.as_ref().and_then(|n| n.owner().clone()),
            Handle::Net(n) => n.as_ref().and_then(|n| n.owner().clone()),
            Handle::Mnt(n) => n.as_ref().and_then(|n| n.owner().clone()),
        }
    }
}

/// What the registry keeps: weak references, so a namespace goes when its
/// last member does (a descriptor on its `/proc` file does not keep it).
#[derive(Debug)]
enum Weakly {
    User(Weak<UserNs>),
    Pid(Weak<PidNs>),
    Uts(Weak<UtsNs>),
    Ipc(Weak<IpcNs>),
    Net(Weak<NetNs>),
    Mnt(Weak<MntNs>),
}

impl Weakly {
    fn of(h: &Handle) -> Option<Weakly> {
        Some(match h {
            Handle::User(n) => Weakly::User(Arc::downgrade(n.as_ref()?)),
            Handle::Pid(n) => Weakly::Pid(Arc::downgrade(n.as_ref()?)),
            Handle::Uts(n) => Weakly::Uts(Arc::downgrade(n.as_ref()?)),
            Handle::Ipc(n) => Weakly::Ipc(Arc::downgrade(n.as_ref()?)),
            Handle::Net(n) => Weakly::Net(Arc::downgrade(n.as_ref()?)),
            Handle::Mnt(n) => Weakly::Mnt(Arc::downgrade(n.as_ref()?)),
        })
    }

    fn upgrade(&self) -> Option<Handle> {
        Some(match self {
            Weakly::User(w) => Handle::User(Some(w.upgrade()?)),
            Weakly::Pid(w) => Handle::Pid(Some(w.upgrade()?)),
            Weakly::Uts(w) => Handle::Uts(Some(w.upgrade()?)),
            Weakly::Ipc(w) => Handle::Ipc(Some(w.upgrade()?)),
            Weakly::Net(w) => Handle::Net(Some(w.upgrade()?)),
            Weakly::Mnt(w) => Handle::Mnt(Some(w.upgrade()?)),
        })
    }

    fn alive(&self) -> bool {
        match self {
            Weakly::User(w) => w.strong_count() > 0,
            Weakly::Pid(w) => w.strong_count() > 0,
            Weakly::Uts(w) => w.strong_count() > 0,
            Weakly::Ipc(w) => w.strong_count() > 0,
            Weakly::Net(w) => w.strong_count() > 0,
            Weakly::Mnt(w) => w.strong_count() > 0,
        }
    }
}

/// Every namespace made so far, by inode number.
static REGISTRY: Mutex<BTreeMap<u32, Weakly>> = Mutex::new(BTreeMap::new());

fn register(inum: u32, h: Handle) {
    if let Some(w) = Weakly::of(&h) {
        let mut r = REGISTRY.lock();
        r.retain(|_, w| w.alive());
        r.insert(inum, w);
    }
}

/// The namespace with inode number `inum` of type `kind` (the initial
/// ones included); `None` if there is none (any more).
pub fn lookup(inum: u32, kind: Kind) -> Option<Handle> {
    let kind = if kind == Kind::PidForChildren { Kind::Pid } else { kind };
    if inum == initial_inum(kind) {
        return Some(match kind {
            Kind::User => Handle::User(None),
            Kind::Pid | Kind::PidForChildren => Handle::Pid(None),
            Kind::Uts => Handle::Uts(None),
            Kind::Ipc => Handle::Ipc(None),
            Kind::Net => Handle::Net(None),
            Kind::Mnt => Handle::Mnt(None),
        });
    }
    let h = REGISTRY.lock().get(&inum)?.upgrade()?;
    (h.kind() == kind).then_some(h)
}

/// Whether a namespace with inode number `inum` exists.
pub fn exists(inum: u32) -> bool {
    Kind::ALL.iter().any(|&k| inum == initial_inum(k)) || REGISTRY.lock().get(&inum).is_some_and(Weakly::alive)
}

fn new_user(creds: &Creds) -> Result<Arc<UserNs>, Errno> {
    let ns = user::create(creds)?;
    register(ns.inum(), Handle::User(Some(ns.clone())));
    Ok(ns)
}

fn new_pid(parent: &PidNsRef, owner: &UserNsRef) -> Result<Arc<PidNs>, Errno> {
    let ns = PidNs::new(parent, owner.clone())?;
    register(ns.inum(), Handle::Pid(Some(ns.clone())));
    Ok(ns)
}

/// Enter `ns` as its newest member's credentials: user_namespaces(7), the
/// first process of a new user namespace (clone(2), unshare(2)) and one
/// that joins it (setns(2)) "has a complete set of capabilities in the
/// new user namespace"; the inheritable and ambient sets start empty and
/// `keep_caps` off (Poly's choice for what the pages leave open).
fn enter_user(creds: &mut Creds, ns: Arc<UserNs>) {
    creds.user_ns = Some(ns);
    creds.caps = Caps::ROOT;
    creds.keep_caps = false;
}

/// The flags of clone(2) and unshare(2) that make namespaces.
pub const NEW_NAMESPACES: u64 = CLONE_NEWNS | CLONE_NEWUTS | CLONE_NEWIPC | CLONE_NEWUSER | CLONE_NEWPID | CLONE_NEWNET;

/// Make the namespaces `flags` asks for (already checked) for a process
/// with namespaces `ns` and its thread's credentials `creds` (both
/// changed in place): clone(2) for the child (`for_child`: a new pid
/// namespace is its own) or unshare(2) for the caller (a new pid namespace
/// is only its children's). A new UTS namespace starts with a copy of the
/// current one's names (`initial_names` are the initial namespace's); a
/// new mount namespace is the kernel's copy of the mount table
/// (`copy_mounts`, given its owner), made last because it cannot be taken
/// back.
///
/// The user namespace comes first, so the others are owned by it and the
/// privilege check of each, `CAP_SYS_ADMIN` in the caller's (new) user
/// namespace (namespaces(7); clone(2) `EPERM`), passes in it. Nothing is
/// changed on an error.
pub fn make(
    ns: &mut Namespaces,
    creds: &mut Creds,
    flags: u64,
    for_child: bool,
    initial_names: UtsNames,
    copy_mounts: &mut dyn FnMut(&UserNsRef) -> Result<Arc<MntNs>, Errno>,
) -> Result<(), Errno> {
    let mut c = creds.clone();
    if flags & CLONE_NEWUSER != 0 {
        let user = new_user(&c)?;
        enter_user(&mut c, user);
    }
    let others = flags & NEW_NAMESPACES & !CLONE_NEWUSER;
    if others != 0 && !c.capable(cap::SYS_ADMIN) {
        return Err(Errno::EPERM);
    }
    let owner = c.user_ns.clone();
    let mut n = ns.clone();
    if flags & CLONE_NEWPID != 0 {
        let pid = new_pid(&ns.pid_for_children, &owner)?;
        if for_child {
            n.pid = Some(pid.clone());
        }
        n.pid_for_children = Some(pid);
    }
    if flags & CLONE_NEWUTS != 0 {
        let current = ns.uts.as_ref().map_or(initial_names, |u| u.names());
        let uts = Arc::new(UtsNs { common: Common::new(&owner), names: Mutex::new(current) });
        register(uts.inum(), Handle::Uts(Some(uts.clone())));
        n.uts = Some(uts);
    }
    if flags & CLONE_NEWIPC != 0 {
        let ipc = Arc::new(IpcNs { common: Common::new(&owner) });
        register(ipc.inum(), Handle::Ipc(Some(ipc.clone())));
        n.ipc = Some(ipc);
    }
    if flags & CLONE_NEWNET != 0 {
        let net = Arc::new(NetNs { common: Common::new(&owner) });
        register(net.inum(), Handle::Net(Some(net.clone())));
        n.net = Some(net);
    }
    // Last: the kernel's copy cannot be taken back.
    if flags & CLONE_NEWNS != 0 {
        n.mnt = Some(copy_mounts(&owner)?);
    }
    *ns = n;
    *creds = c;
    Ok(())
}

/// clone(2)'s rules about namespace flags beyond [`crate::task::clone`]'s
/// (on flags without the `CSIGNAL` byte): `CLONE_NEWUSER` or
/// `CLONE_NEWPID` with `CLONE_THREAD`, `CLONE_NEWUSER` with `CLONE_FS`,
/// `CLONE_NEWIPC` with `CLONE_SYSVSEM`: `EINVAL`.
pub fn check_clone_flags(flags: u64) -> Result<(), Errno> {
    let has = |f: u64| flags & f != 0;
    if has(CLONE_THREAD) && has(CLONE_NEWUSER | CLONE_NEWPID) {
        return Err(Errno::EINVAL);
    }
    if has(CLONE_NEWUSER) && has(CLONE_FS) {
        return Err(Errno::EINVAL);
    }
    if has(CLONE_NEWIPC) && has(CLONE_SYSVSEM) {
        return Err(Errno::EINVAL);
    }
    Ok(())
}

/// The flags unshare(2) accepts.
pub const UNSHARE_FLAGS: u64 = CLONE_FILES
    | CLONE_FS
    | CLONE_NEWCGROUP
    | CLONE_NEWIPC
    | CLONE_NEWNET
    | CLONE_NEWNS
    | CLONE_NEWPID
    | CLONE_NEWTIME
    | CLONE_NEWUSER
    | CLONE_NEWUTS
    | CLONE_SIGHAND
    | CLONE_SYSVSEM
    | CLONE_THREAD
    | CLONE_VM;

/// What unshare(2) will do with `flags`, for a process with `threads`
/// threads whose root `chrooted` is not its mount namespace's root.
/// unshare(2):
///
/// - unknown flags, and `CLONE_NEWCGROUP` / `CLONE_NEWTIME` (not
///   supported): `EINVAL`;
/// - "CLONE_NEWUSER ... implies CLONE_THREAD", "CLONE_NEWNS implies
///   CLONE_FS", "CLONE_VM implies CLONE_SIGHAND", "CLONE_SIGHAND implies
///   CLONE_THREAD"; `CLONE_THREAD`, `CLONE_SIGHAND` and `CLONE_VM` "can be
///   specified ... if the caller is single threaded" (else `EINVAL`);
/// - `CLONE_FILES` and `CLONE_FS` unshare what a process already owns
///   alone in Poly (its descriptor table and directories): nothing to do
///   for a single thread; with other threads Poly cannot split them
///   (`EINVAL`, Poly's choice); `CLONE_SYSVSEM` has nothing to undo;
/// - "EPERM CLONE_NEWUSER was specified in flags and the caller is in a
///   chroot environment".
///
/// Returns the namespace flags to make.
pub fn check_unshare(flags: u64, threads: usize, chrooted: bool) -> Result<u64, Errno> {
    if flags & !UNSHARE_FLAGS != 0 || flags & (CLONE_NEWCGROUP | CLONE_NEWTIME) != 0 {
        return Err(Errno::EINVAL);
    }
    let mut implied = flags;
    if implied & CLONE_NEWUSER != 0 {
        implied |= CLONE_THREAD | CLONE_FS;
    }
    if implied & CLONE_NEWNS != 0 {
        implied |= CLONE_FS;
    }
    if implied & CLONE_VM != 0 {
        implied |= CLONE_SIGHAND;
    }
    if implied & CLONE_SIGHAND != 0 {
        implied |= CLONE_THREAD;
    }
    if threads > 1 && implied & (CLONE_THREAD | CLONE_SIGHAND | CLONE_VM | CLONE_FILES | CLONE_FS) != 0 {
        return Err(Errno::EINVAL);
    }
    if flags & CLONE_NEWUSER != 0 && chrooted {
        return Err(Errno::EPERM);
    }
    Ok(flags & NEW_NAMESPACES)
}

/// What setns(2) changes.
#[derive(Clone, Debug)]
pub enum Join {
    /// The calling thread's credentials (a user namespace, with full
    /// capabilities in it).
    User(Creds),
    /// The process's namespaces (UTS, IPC, network, `pid_for_children`).
    Namespaces(Namespaces),
    /// A mount namespace: the kernel moves the root and working directory
    /// to its root.
    Mounts(Option<Arc<MntNs>>),
}

/// setns(2) of a thread with `creds` in a process with `ns` and `threads`
/// threads (`chrooted` as for [`check_unshare`]) into `target`, with
/// `nstype` (0: any) as given. setns(2):
///
/// - `nstype` not 0 and not the namespace's type: `EINVAL`;
/// - "the caller must have the CAP_SYS_ADMIN capability in its own user
///   namespace and in the user namespace that owns the target namespace"
///   (`EPERM`); for a user namespace `CAP_SYS_ADMIN` in it;
/// - a user namespace: "A multithreaded process may not change user
///   namespace" (`EINVAL`), "EINVAL ... the caller attempted to join the
///   user namespace in which it is already a member", not when chrooted
///   (`EPERM`);
/// - a mount namespace: also `CAP_SYS_CHROOT` in the caller's namespace;
///   with other threads sharing the directories, `EINVAL`;
/// - a pid namespace: only the caller's own or a descendant of it
///   (`EINVAL`); it becomes `pid_for_children`.
pub fn join(
    ns: &Namespaces,
    creds: &Creds,
    threads: usize,
    chrooted: bool,
    target: &Handle,
    nstype: u64,
) -> Result<Join, Errno> {
    if nstype != 0 && nstype != target.kind().flag() {
        return Err(Errno::EINVAL);
    }
    if let Handle::User(u) = target {
        if threads > 1 || user::same(u, &creds.user_ns) {
            return Err(Errno::EINVAL);
        }
        if chrooted {
            return Err(Errno::EPERM);
        }
        if !user::ns_capable(creds, u, cap::SYS_ADMIN) {
            return Err(Errno::EPERM);
        }
        let Some(u) = u else { return Err(Errno::EPERM) };
        let mut c = creds.clone();
        enter_user(&mut c, u.clone());
        return Ok(Join::User(c));
    }
    if !creds.capable(cap::SYS_ADMIN) || !user::ns_capable(creds, &target.owner(), cap::SYS_ADMIN) {
        return Err(Errno::EPERM);
    }
    let mut n = ns.clone();
    match target {
        Handle::Mnt(m) => {
            if !creds.capable(cap::SYS_CHROOT) {
                return Err(Errno::EPERM);
            }
            if threads > 1 {
                return Err(Errno::EINVAL);
            }
            return Ok(Join::Mounts(m.clone()));
        }
        Handle::Pid(p) => {
            if !pid::is_ancestor(&ns.pid, p) {
                return Err(Errno::EINVAL);
            }
            n.pid_for_children = p.clone();
        }
        Handle::Uts(u) => n.uts = u.clone(),
        Handle::Ipc(i) => n.ipc = i.clone(),
        Handle::Net(x) => n.net = x.clone(),
        Handle::User(_) => unreachable!("handled above"),
    }
    Ok(Join::Namespaces(n))
}

#[cfg(test)]
pub(crate) fn test_uts(owner: &UserNsRef, names: UtsNames) -> Arc<UtsNs> {
    Arc::new(UtsNs { common: Common::new(owner), names: Mutex::new(names) })
}

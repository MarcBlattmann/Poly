//! Namespaces (ADR 0054): unshare(2), setns(2), user namespaces and their
//! id maps, UTS names, pid numbers as a pid namespace sees them.

use super::fake::{DATA, FakeKernel, sys};
use crate::cred::{Creds, cap};
use crate::errno::Errno;
use crate::nr;
use crate::ns::pid::{self, PidNs};
use crate::ns::user::{self, IdKind, OVERFLOW_ID};
use crate::ns::{self, Kind, initial_inum};
use crate::task::clone::*;

fn unshare(k: &mut FakeKernel, flags: u64) -> Result<u64, Errno> {
    sys(k, nr::UNSHARE, &[flags])
}

fn ids(k: &mut FakeKernel) -> [u64; 4] {
    [nr::GETUID, nr::GETEUID, nr::GETGID, nr::GETEGID].map(|n| sys(k, n, &[]).unwrap())
}

#[test]
fn unshare_checks_its_flags() {
    let mut k = FakeKernel::new();
    assert_eq!(unshare(&mut k, 0), Ok(0));
    assert_eq!(unshare(&mut k, CLONE_PTRACE), Err(Errno::EINVAL), "not an unshare flag");
    assert_eq!(unshare(&mut k, 1 << 32), Err(Errno::EINVAL), "all 64 bits count");
    assert_eq!(unshare(&mut k, CLONE_NEWUTS | 1), Err(Errno::EINVAL), "bad flags before privilege");
    assert_eq!(unshare(&mut k, CLONE_NEWCGROUP), Err(Errno::EPERM));
    assert_eq!(unshare(&mut k, CLONE_NEWTIME), Err(Errno::EPERM));
    assert_eq!(unshare(&mut k, CLONE_FS | CLONE_FILES), Ok(0), "a single thread owns them already");
    // Without CAP_SYS_ADMIN only a user namespace can be made.
    assert_eq!(unshare(&mut k, CLONE_NEWUTS), Err(Errno::EPERM));
    assert_eq!(unshare(&mut k, CLONE_NEWNET), Err(Errno::EPERM));
    assert!(k.me().ns.uts.is_none() && k.me().ns.net.is_none(), "nothing changed");
    // A chrooted caller may not make a user namespace.
    k.chroot = true;
    assert_eq!(unshare(&mut k, CLONE_NEWUSER), Err(Errno::EPERM));
    k.chroot = false;
    // Nor may a thread of a process with others (CLONE_NEWUSER implies
    // CLONE_THREAD).
    k.add_thread(101);
    assert_eq!(unshare(&mut k, CLONE_NEWUSER), Err(Errno::EINVAL));
    assert_eq!(unshare(&mut k, CLONE_VM), Err(Errno::EINVAL));
    assert_eq!(unshare(&mut k, CLONE_SIGHAND), Err(Errno::EINVAL));
    assert_eq!(unshare(&mut k, CLONE_FS | CLONE_FILES | CLONE_SYSVSEM), Ok(0), "the caller's own");
    assert!(k.creds().user_ns.is_none());
}

#[test]
fn a_new_user_namespace_maps_its_ids() {
    let mut k = FakeKernel::new();
    assert_eq!(unshare(&mut k, CLONE_NEWUSER), Ok(0));
    let c = k.creds().clone();
    let u = c.user_ns.clone().expect("a new user namespace");
    assert_eq!((u.level(), u.owner(), u.group()), (1, 1001, 2001), "owned by the effective ids");
    // Every capability in the namespace, none outside it.
    assert!(c.capable(cap::SYS_ADMIN) && c.capable(cap::SETUID));
    assert!(!c.capable_initial(cap::SYS_ADMIN));
    // Nothing is mapped yet: every id reads as the overflow id.
    let o = u64::from(OVERFLOW_ID);
    assert_eq!(ids(&mut k), [o, o, o, o]);
    assert_eq!(sys(&mut k, nr::SETUID, &[0]), Err(Errno::EINVAL), "0 has no mapping");
    // The owner maps its own effective uid; once.
    let ns = c.user_ns.clone();
    assert_eq!(user::write_map(&ns, IdKind::Uid, b"0 0 1\n", 0, &c), Err(Errno::EPERM), "not its own id");
    assert_eq!(user::write_map(&ns, IdKind::Uid, b"0 1001 1\n", 1, &c), Err(Errno::EINVAL), "offset");
    assert_eq!(user::write_map(&ns, IdKind::Uid, b"0 1001 1\n", 0, &c), Ok(9));
    assert_eq!(user::write_map(&ns, IdKind::Uid, b"0 1001 1\n", 0, &c), Err(Errno::EPERM), "written once");
    assert_eq!(ids(&mut k), [o, 0, o, o], "the effective uid 1001 is root inside, the real 1000 unmapped");
    // gid_map needs "deny" in setgroups first (the writer lacks CAP_SETGID
    // in the parent namespace).
    assert_eq!(user::write_map(&ns, IdKind::Gid, b"0 2001 1", 0, &c), Err(Errno::EPERM));
    assert_eq!(user::write_setgroups(&ns, b"maybe\n", 0, &c), Err(Errno::EINVAL));
    assert_eq!(user::write_setgroups(&ns, b"deny\n", 0, &c), Ok(5));
    assert_eq!(user::format_setgroups(&ns), b"deny\n");
    assert_eq!(user::write_map(&ns, IdKind::Gid, b"0 2001 1", 0, &c), Ok(8));
    assert_eq!(user::write_setgroups(&ns, b"allow", 0, &c), Err(Errno::EPERM), "after gid_map");
    assert_eq!(ids(&mut k), [o, 0, o, 0]);
    // setgroups(2) stays refused.
    assert_eq!(sys(&mut k, nr::SETGROUPS, &[0, 0]), Err(Errno::EPERM));
    // The maps as the namespace reads them, and as the parent does.
    let inside = c.user_ns.clone();
    assert_eq!(user::format_map(&ns, IdKind::Uid, &inside), b"         0       1001          1\n");
    assert_eq!(user::format_map(&ns, IdKind::Uid, &None), b"         0       1001          1\n");
    assert_eq!(user::format_map(&None, IdKind::Uid, &inside), b"         0          0 4294967295\n");
    // Ids it sets are its own: 0 is kernel id 1001.
    assert_eq!(sys(&mut k, nr::SETRESUID, &[0, 0, 0]), Ok(0));
    assert_eq!((k.creds().uid, k.creds().euid), (1001, 1001));
    assert_eq!(ids(&mut k), [0, 0, o, 0]);
    // Leaving root inside drops the capabilities, as for root outside.
    assert_eq!(sys(&mut k, nr::SETRESUID, &[5, 5, 5]), Err(Errno::EINVAL), "5 has no mapping");
}

#[test]
fn capabilities_follow_the_owner_rules() {
    let parent = Creds::user(1000, 1000);
    let child = user::create(&parent).unwrap();
    let ns = Some(child.clone());
    // The owner, in the parent namespace, has every capability in it; an
    // unprivileged stranger none.
    assert!(user::ns_capable(&parent, &ns, cap::SYS_ADMIN));
    assert!(!user::ns_capable(&Creds::user(1002, 1002), &ns, cap::SYS_ADMIN));
    assert!(user::ns_capable(&Creds::root(), &ns, cap::SYS_ADMIN), "root of the parent");
    // A member has what its own sets say, and nothing in the parent.
    let mut member = parent.clone();
    member.user_ns = ns.clone();
    member.caps = crate::cred::Caps::ROOT;
    assert!(user::ns_capable(&member, &ns, cap::NET_ADMIN));
    assert!(!user::ns_capable(&member, &None, cap::NET_ADMIN));
    // Nested too deep: EUSERS.
    let mut c = member;
    for _ in 1..user::MAX_DEPTH {
        c.user_ns = Some(user::create(&c).unwrap());
    }
    assert_eq!(c.user_ns.as_ref().unwrap().level(), user::MAX_DEPTH);
    assert_eq!(user::create(&c).map(|_| ()), Err(Errno::EUSERS));
}

#[test]
fn id_maps_parse_and_translate() {
    let e = |text: &[u8]| user::parse_map(text).err();
    assert_eq!(e(b""), Some(Errno::EINVAL));
    assert_eq!(e(b"0 1000"), Some(Errno::EINVAL));
    assert_eq!(e(b"0 1000 0"), Some(Errno::EINVAL), "length 0");
    assert_eq!(e(b"0 1000 1 2"), Some(Errno::EINVAL));
    assert_eq!(e(b"0 -1 1"), Some(Errno::EINVAL));
    assert_eq!(e(b"4294967295 0 2"), Some(Errno::EINVAL), "past the last id");
    assert_eq!(e(b"0 1000 10\n5 2000 1\n"), Some(Errno::EINVAL), "overlap inside");
    assert_eq!(e(b"0 1000 10\n20 1005 1\n"), Some(Errno::EINVAL), "overlap outside");
    let lines = user::parse_map(b"0 1000 10\n  100\t2000 5\n").unwrap();
    let map = user::IdMap { extents: lines };
    assert_eq!((map.down(0), map.down(9), map.down(10)), (Some(1000), Some(1009), None));
    assert_eq!((map.down(104), map.up(2004), map.up(999)), (Some(2004), Some(104), None));
    // The parent's CAP_SETUID allows any map of its ids.
    let root = Creds::root();
    let ns = Some(user::create(&root).unwrap());
    assert_eq!(user::write_map(&ns, IdKind::Uid, b"0 100000 65536\n", 0, &root), Ok(15));
    assert_eq!(user::down(&ns, IdKind::Uid, 65535), Some(165_535));
    assert_eq!(user::up(&ns, IdKind::Uid, 0), None);
    assert_eq!(user::write_map(&None, IdKind::Uid, b"0 0 1", 0, &root), Err(Errno::EPERM), "the initial maps");
}

#[test]
fn a_uts_namespace_has_its_own_names() {
    let mut k = FakeKernel::new();
    assert_eq!(unshare(&mut k, CLONE_NEWUSER | CLONE_NEWUTS), Ok(0));
    let uts = k.me().ns.uts.clone().expect("a UTS namespace");
    assert_eq!(uts.names().hostname, b"testhost", "a copy of the current names");
    assert!(crate::ns::user::same(uts.owner(), &k.creds().user_ns), "owned by the new user namespace");
    k.poke(DATA, b"box");
    assert_eq!(sys(&mut k, nr::SETHOSTNAME, &[DATA, 3]), Ok(0), "privileged in its own namespace");
    assert_eq!(uts.names().hostname, b"box");
    assert_eq!(sys(&mut k, nr::UNAME, &[DATA + 0x100]), Ok(0));
    assert_eq!(k.peek(DATA + 0x100 + 65, 4), b"box\0");
    // Without CAP_SYS_ADMIN over its owner: EPERM.
    k.creds_mut().caps = crate::cred::Caps::NONE;
    assert_eq!(sys(&mut k, nr::SETHOSTNAME, &[DATA, 3]), Err(Errno::EPERM));
}

#[test]
fn unshare_of_a_pid_namespace_is_for_the_children() {
    let mut k = FakeKernel::new();
    assert_eq!(unshare(&mut k, CLONE_NEWUSER | CLONE_NEWPID | CLONE_NEWNET | CLONE_NEWIPC | CLONE_NEWNS), Ok(0));
    let n = k.me().ns.clone();
    assert!(n.pid.is_none(), "the caller stays where it is");
    let child = n.pid_for_children.clone().expect("its children's namespace");
    assert_eq!(child.level(), 1);
    assert!(n.net.is_some() && n.ipc.is_some());
    assert_eq!(k.mount_copies.len(), 1, "the kernel copied the mount table");
    assert_eq!(n.mnt.as_ref().map(|m| m.id()), Some(1000));
    assert_eq!(sys(&mut k, nr::GETPID, &[]), Ok(100));
    // A fork's child is in it.
    assert!(crate::ns::pid::same(&n.for_child().pid, &Some(child)));
    // Each namespace has its own inode number.
    let user = k.creds().user_ns.clone();
    let numbers: alloc::collections::BTreeSet<u32> = Kind::ALL.iter().map(|&kind| n.inum(kind, &user)).collect();
    assert_eq!(numbers.len(), 9, "four initial (pid, uts, cgroup, time), five new: all different");
    assert_eq!(n.inum(Kind::Pid, &user), initial_inum(Kind::Pid));
    assert!(matches!(ns::lookup(n.inum(Kind::Net, &user), Kind::Net), Some(ns::Handle::Net(Some(_)))));
    assert!(ns::lookup(n.inum(Kind::Net, &user), Kind::Ipc).is_none(), "the wrong kind");
}

#[test]
fn pid_namespaces_number_their_members() {
    let ns = Some(PidNs::new(&None, None).unwrap());
    pid::attach(&ns, 200, true).unwrap();
    pid::attach(&ns, 201, false).unwrap();
    pid::attach(&ns, 202, true).unwrap();
    assert_eq!((pid::local(&ns, 200), pid::local(&ns, 201), pid::local(&ns, 202)), (Some(1), Some(2), Some(3)));
    assert_eq!(pid::global(&ns, 3), Some(202));
    assert_eq!((pid::local_or_zero(&ns, 100), pid::local(&None, 100)), (0, Some(100)));
    assert_eq!(ns.as_ref().unwrap().init(), Some(200), "the first process is init");
    assert_eq!((pid::reaper(&ns), pid::reaper_of_new(&ns, 200), pid::reaper_of_new(&ns, 202)), (200, 1, 200));
    // Nested: a member of the inner namespace has a number in both.
    let inner = Some(PidNs::new(&ns, None).unwrap());
    pid::attach(&inner, 300, true).unwrap();
    assert_eq!((pid::local(&inner, 300), pid::local(&ns, 300)), (Some(1), Some(4)));
    assert!(pid::is_ancestor(&ns, &inner) && !pid::is_ancestor(&inner, &ns));
    // Numbers come back.
    pid::detach(&ns, 201);
    assert_eq!(pid::global(&ns, 2), None);
    // The init ends: the others are killed and nobody joins any more.
    let mut doomed = pid::init_ended(&ns, 200);
    doomed.sort_unstable();
    assert_eq!(doomed, [202, 300]);
    assert!(ns.as_ref().unwrap().is_dead());
    assert_eq!(pid::attach(&ns, 203, true), Err(Errno::ENOMEM));
    assert_eq!(pid::attach(&inner, 301, true), Err(Errno::ENOMEM), "an ancestor is dead");
    assert_eq!(pid::local(&inner, 301), None, "nothing half done");
    assert_eq!(pid::reaper(&ns), 1);
}

#[test]
fn ids_are_seen_from_the_callers_pid_namespace() {
    let mut k = FakeKernel::new();
    let ns = Some(PidNs::new(&None, None).unwrap());
    pid::attach(&ns, 100, true).unwrap();
    k.me().ns.pid = ns.clone();
    assert_eq!(sys(&mut k, nr::GETPID, &[]), Ok(1), "the init of its namespace");
    assert_eq!(sys(&mut k, nr::GETTID, &[]), Ok(1));
    assert_eq!(sys(&mut k, nr::GETPPID, &[]), Ok(0), "the parent is outside");
    assert_eq!(sys(&mut k, nr::KILL, &[1, 0]), Ok(0), "itself");
    assert_eq!(sys(&mut k, nr::KILL, &[100, 0]), Err(Errno::ESRCH), "no such number here");
}

#[test]
fn setns_joins_a_namespace() {
    let mut k = FakeKernel::new();
    *k.creds_mut() = Creds::root();
    assert_eq!(unshare(&mut k, CLONE_NEWUTS | CLONE_NEWNET), Ok(0));
    let uts = k.me().ns.uts.clone().unwrap();
    // Back to the initial UTS namespace through a descriptor on its file.
    k.ns_fds.insert(5, (initial_inum(Kind::Uts), Kind::Uts));
    assert_eq!(sys(&mut k, nr::SETNS, &[5, CLONE_NEWNET]), Err(Errno::EINVAL), "not that type");
    assert_eq!(sys(&mut k, nr::SETNS, &[5, CLONE_NEWUTS | CLONE_NEWNET]), Err(Errno::EINVAL));
    assert_eq!(sys(&mut k, nr::SETNS, &[5, CLONE_NEWUTS]), Ok(0));
    assert!(k.me().ns.uts.is_none() && k.me().ns.net.is_some());
    // And into the new one again, with nstype 0.
    k.ns_fds.insert(6, (uts.inum(), Kind::Uts));
    assert_eq!(sys(&mut k, nr::SETNS, &[6, 0]), Ok(0));
    assert!(k.me().ns.uts.as_ref().is_some_and(|u| alloc::sync::Arc::ptr_eq(u, &uts)));
    // Not a namespace file, no descriptor, a namespace that is gone.
    assert_eq!(sys(&mut k, nr::SETNS, &[0, 0]), Err(Errno::EINVAL));
    assert_eq!(sys(&mut k, nr::SETNS, &[77, 0]), Err(Errno::EBADF));
    k.ns_fds.insert(7, (0xffff_fff0, Kind::Net));
    assert_eq!(sys(&mut k, nr::SETNS, &[7, 0]), Err(Errno::EINVAL));
    // A mount namespace: the kernel moves the directories.
    k.ns_fds.insert(8, (initial_inum(Kind::Mnt), Kind::Mnt));
    assert_eq!(sys(&mut k, nr::SETNS, &[8, CLONE_NEWNS]), Ok(0));
    assert_eq!(k.entered_mounts, [None]);
    // A pid namespace becomes the children's.
    let child = Some(PidNs::new(&None, None).unwrap());
    k.ns_fds.insert(9, (child.as_ref().unwrap().inum(), Kind::Pid));
    // Not registered: only namespaces made through unshare/clone are found.
    assert_eq!(sys(&mut k, nr::SETNS, &[9, CLONE_NEWPID]), Err(Errno::EINVAL));
    // Its own user namespace: EINVAL; without privilege: EPERM.
    k.ns_fds.insert(10, (initial_inum(Kind::User), Kind::User));
    assert_eq!(sys(&mut k, nr::SETNS, &[10, CLONE_NEWUSER]), Err(Errno::EINVAL));
    k.creds_mut().caps = crate::cred::Caps::NONE;
    assert_eq!(sys(&mut k, nr::SETNS, &[5, 0]), Err(Errno::EPERM));
}

#[test]
fn a_user_namespace_can_be_joined_by_its_owner() {
    let mut k = FakeKernel::new();
    assert_eq!(unshare(&mut k, CLONE_NEWUSER), Ok(0));
    let u = k.creds().user_ns.clone().unwrap();
    // Back in the initial namespace without capabilities, the owner can
    // still join it (it has every capability there).
    *k.creds_mut() = {
        let mut c = Creds::user(1000, 2000);
        c.euid = 1001;
        c
    };
    k.ns_fds.insert(5, (u.inum(), Kind::User));
    assert_eq!(sys(&mut k, nr::SETNS, &[5, CLONE_NEWUSER]), Ok(0));
    let c = k.creds();
    assert!(user::same(&c.user_ns, &Some(u)) && c.capable(cap::SYS_ADMIN));
    // A stranger cannot; nor can a process with other threads.
    let other = Some(user::create(&Creds::user(1000, 1000)).unwrap());
    let handle = ns::Handle::User(other);
    let none = ns::Namespaces::default();
    assert_eq!(ns::join(&none, &Creds::user(1002, 1002), 1, false, &handle, 0).err(), Some(Errno::EPERM));
    assert_eq!(ns::join(&none, &Creds::user(1000, 1000), 2, false, &handle, 0).err(), Some(Errno::EINVAL));
    assert_eq!(ns::join(&none, &Creds::user(1000, 1000), 1, true, &handle, 0).err(), Some(Errno::EPERM), "chroot");
    assert!(ns::join(&none, &Creds::user(1000, 1000), 1, false, &handle, 0).is_ok());
}

#[test]
fn cgroup_and_time_namespaces() {
    let mut k = FakeKernel::new();
    *k.creds_mut() = Creds::root();
    assert_eq!(unshare(&mut k, CLONE_NEWCGROUP | CLONE_NEWTIME), Ok(0));
    let n = k.me().ns.clone();
    let user = k.creds().user_ns.clone();
    assert!(n.cgroup.is_some(), "the caller moves into a new cgroup namespace");
    assert!(n.time.is_none(), "but not into the time namespace");
    let t = n.time_for_children.clone().expect("its children's");
    assert_eq!(n.inum(Kind::TimeForChildren, &user), t.inum());
    assert_eq!(n.inum(Kind::Time, &user), initial_inum(Kind::Time));
    assert!(n.for_child().time.as_ref().is_some_and(|c| alloc::sync::Arc::ptr_eq(c, &t)));
    // A second time namespace is fine (unlike a pid namespace).
    assert_eq!(unshare(&mut k, CLONE_NEWTIME), Ok(0));
    assert_eq!(Kind::from_type_name(b"time"), Some(Kind::Time));
    assert_eq!(Kind::TimeForChildren.type_name(), "time");
    assert_eq!(Kind::TimeForChildren.flag(), CLONE_NEWTIME);
    // setns of the initial time namespace sets the children's; with
    // threads it is EUSERS, before the privilege.
    k.ns_fds.insert(5, (initial_inum(Kind::Time), Kind::Time));
    assert_eq!(sys(&mut k, nr::SETNS, &[5, CLONE_NEWTIME]), Ok(0));
    assert!(k.me().ns.time_for_children.is_none());
    k.add_thread(101);
    k.creds_mut().caps = crate::cred::Caps::NONE;
    assert_eq!(sys(&mut k, nr::SETNS, &[5, 0]), Err(Errno::EUSERS));
    k.ns_fds.insert(6, (initial_inum(Kind::Cgroup), Kind::Cgroup));
    assert_eq!(sys(&mut k, nr::SETNS, &[6, 0]), Err(Errno::EPERM));
}

#[test]
fn a_second_pid_namespace_is_refused_and_its_file_waits_for_a_child() {
    let mut k = FakeKernel::new();
    *k.creds_mut() = Creds::root();
    assert_eq!(unshare(&mut k, CLONE_NEWPID), Ok(0));
    assert_eq!(unshare(&mut k, CLONE_NEWPID), Err(Errno::EINVAL));
    let n = k.me().ns.clone();
    let user = k.creds().user_ns.clone();
    assert_eq!(n.entry_inum(Kind::PidForChildren, &user), None, "no first process yet");
    assert_eq!(n.entry_inum(Kind::Pid, &user), Some(initial_inum(Kind::Pid)));
    let p = n.pid_for_children.clone().unwrap();
    pid::attach(&Some(p.clone()), 4242, true).unwrap();
    assert_eq!(n.entry_inum(Kind::PidForChildren, &user), Some(p.inum()));
}

#[test]
fn an_open_namespace_file_keeps_its_namespace() {
    let mut k = FakeKernel::new();
    *k.creds_mut() = Creds::root();
    assert_eq!(unshare(&mut k, CLONE_NEWUTS), Ok(0));
    let inum = k.me().ns.uts.as_ref().unwrap().inum();
    ns::pin(inum, Kind::Uts, true);
    ns::pin(inum, Kind::Uts, true);
    // The process leaves it: the pins hold it.
    k.me().ns.uts = None;
    assert!(ns::lookup(inum, Kind::Uts).is_some());
    ns::pin(inum, Kind::Uts, false);
    assert!(ns::lookup(inum, Kind::Uts).is_some());
    ns::pin(inum, Kind::Uts, false);
    assert!(ns::lookup(inum, Kind::Uts).is_none(), "gone with the last pin");
}

#[test]
fn setns_through_a_pidfd_joins_several_at_once() {
    let mut k = FakeKernel::new();
    *k.creds_mut() = Creds::root();
    assert_eq!(unshare(&mut k, CLONE_NEWUTS | CLONE_NEWIPC), Ok(0));
    let theirs = k.me().ns.clone();
    let user = k.creds().user_ns.clone();
    let targets = crate::sys::ns::pidfd_targets(&theirs, &user, CLONE_NEWUTS | CLONE_NEWIPC);
    assert_eq!(targets.len(), 2);
    let none = ns::Namespaces::default();
    let (_, joined, mounts) = crate::sys::ns::join_all(&none, &Creds::root(), 1, false, &targets).unwrap();
    assert_eq!(joined.uts, theirs.uts);
    assert_eq!(joined.ipc, theirs.ipc);
    assert!(mounts.is_none());
    // Its own user namespace: EINVAL; no privilege: EPERM, all or nothing.
    let user_target = crate::sys::ns::pidfd_targets(&theirs, &user, CLONE_NEWUSER);
    assert_eq!(crate::sys::ns::join_all(&none, &Creds::root(), 1, false, &user_target).err(), Some(Errno::EINVAL));
    let plain = Creds::user(1000, 1000);
    assert_eq!(crate::sys::ns::join_all(&none, &plain, 1, false, &targets).err(), Some(Errno::EPERM));
    assert_eq!(sys(&mut k, nr::SETNS, &[77, CLONE_NEWUTS]), Err(Errno::EBADF));
}

#[test]
fn host_name_rules() {
    use crate::sys::admin::sysctl_name;
    assert_eq!(sysctl_name(b"from-proc\n"), b"from-proc");
    assert_eq!(sysctl_name(b"a\0b"), b"a");
    assert_eq!(sysctl_name(&[b'x'; 80]).len(), 64);
    let mut k = FakeKernel::new();
    k.poke(DATA, b"hijklm");
    // The privilege comes first; only the low 32 bits of the length count.
    k.creds_mut().caps = crate::cred::Caps::NONE;
    assert_eq!(sys(&mut k, nr::SETHOSTNAME, &[DATA, 65]), Err(Errno::EPERM));
    *k.creds_mut() = Creds::root();
    assert_eq!(sys(&mut k, nr::SETHOSTNAME, &[DATA, 0xffff_ffff]), Err(Errno::EINVAL));
}

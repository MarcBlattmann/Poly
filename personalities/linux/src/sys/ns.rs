//! unshare(2) and setns(2): leaving and joining namespaces (ADR 0054).
//! The rules are in [`crate::ns`]; clone(2)'s namespace flags are made by
//! the kernel's `clone_task` with the same [`ns::make`].

use crate::admin::SystemName;
use crate::args::Args;
use crate::errno::{Errno, SysResult};
use crate::kernel::LinuxKernel;
use crate::ns::{self, Join, UtsNames};
use crate::signal::deliver::with_own;
use crate::table::{Handler, nr};
use crate::task::clone::{CLONE_NEWCGROUP, CLONE_NEWTIME, CLONE_NEWUTS};

/// The calls of this module, registered with the table's groups.
pub const CALLS: &[(u64, Handler)] = &[(nr::UNSHARE, unshare), (nr::SETNS, setns)];

/// The host and domain names of the initial UTS namespace, which a new
/// UTS namespace made from it copies.
pub fn initial_names(k: &mut dyn LinuxKernel) -> UtsNames {
    match k.admin() {
        Some(a) => {
            UtsNames { hostname: a.system_name(SystemName::Host), domainname: a.system_name(SystemName::Domain) }
        }
        None => UtsNames { hostname: k.hostname().to_vec(), domainname: k.domainname().to_vec() },
    }
}

/// unshare(flags): see [`ns::check_unshare`] for the flags and
/// [`ns::make`] for the namespaces. The calling thread's credentials
/// change with a new user namespace, the process's namespaces with the
/// others (`CLONE_NEWPID`: only `pid_for_children`, unshare(2): "the
/// calling process ... is not moved into the new namespace").
pub fn unshare(k: &mut dyn LinuxKernel, a: &Args) -> SysResult {
    let flags = u64::from(a.uint(0));
    let threads = with_own(k, |p| p.thread_ids().count());
    let make = ns::check_unshare(flags, threads, k.chrooted())?;
    if make == 0 {
        return Ok(0);
    }
    let tid = k.tid();
    let (mut namespaces, mut creds) = with_own(k, |p| (p.ns.clone(), p.thread(tid).creds.clone()));
    let names = if make & CLONE_NEWUTS != 0 { initial_names(k) } else { UtsNames::default() };
    ns::make(&mut namespaces, &mut creds, make, false, names, &mut |owner| k.copy_mounts(owner))?;
    with_own(k, |p| {
        p.ns = namespaces;
        p.thread_mut(tid).creds = creds;
    });
    Ok(0)
}

/// setns(fd, nstype): join the namespace `fd` refers to (a descriptor of a
/// `/proc/[pid]/ns` file; anything else is `EINVAL`, a closed one `EBADF`),
/// by [`ns::join`]'s rules. Cgroup and time namespaces (not supported) are
/// `EINVAL`; so is a pidfd, which Poly does not have.
pub fn setns(k: &mut dyn LinuxKernel, a: &Args) -> SysResult {
    let fd = a.fd(0);
    let nstype = u64::from(a.uint(1));
    if nstype != 0 && (nstype & !ns::NEW_NAMESPACES != 0 || nstype.count_ones() != 1)
        || nstype & (CLONE_NEWCGROUP | CLONE_NEWTIME) != 0
    {
        return Err(Errno::EINVAL);
    }
    let (inum, kind) = k.namespace_of_fd(fd)?;
    let target = ns::lookup(inum, kind).ok_or(Errno::EINVAL)?;
    let tid = k.tid();
    let (namespaces, creds, threads) =
        with_own(k, |p| (p.ns.clone(), p.thread(tid).creds.clone(), p.thread_ids().count()));
    match ns::join(&namespaces, &creds, threads, k.chrooted(), &target, nstype)? {
        Join::User(c) => with_own(k, |p| p.thread_mut(tid).creds = c),
        Join::Namespaces(n) => with_own(k, |p| p.ns = n),
        Join::Mounts(m) => {
            k.enter_mounts(&m)?;
            with_own(k, |p| p.ns.mnt = m);
        }
    }
    Ok(0)
}

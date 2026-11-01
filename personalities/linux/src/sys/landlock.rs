//! landlock_create_ruleset(2), landlock_add_rule(2),
//! landlock_restrict_self(2) (UAPI `linux/landlock.h`, the kernel's
//! `userspace-api/landlock.rst`), and the checks the other calls make for
//! a thread in a Landlock domain: file opens ([`check_open`]) and TCP
//! binds and connects ([`check_socket`]). Signal scoping is in
//! [`crate::signal::deliver::may_signal`]. The rules are in
//! [`crate::landlock`].

use super::pipe::file_io;
use crate::args::Args;
use crate::cred;
use crate::errno::{Errno, SysResult};
use crate::kernel::LinuxKernel;
use crate::landlock::{self, Domain, Handled, Rule, Ruleset, abi::*};
use crate::net::abi::{SOCK_STREAM, SockAddr};
use crate::net::{OptName, OptValue};
use crate::sys::path::fs;
use crate::table::{Handler, nr};
use alloc::sync::Arc;
use alloc::vec;

/// The calls of this module, for [`crate::table::GROUPS`].
pub const CALLS: &[(u64, Handler)] = &[
    (nr::LANDLOCK_CREATE_RULESET, landlock_create_ruleset),
    (nr::LANDLOCK_ADD_RULE, landlock_add_rule),
    (nr::LANDLOCK_RESTRICT_SELF, landlock_restrict_self),
];

/// landlock_create_ruleset(attr, size, flags): with
/// `LANDLOCK_CREATE_RULESET_VERSION` or `_ERRATA` (attr NULL, size 0,
/// else `EINVAL`) the ABI version or the errata; with flags 0 a new
/// ruleset from `struct landlock_ruleset_attr`: size below 8 `EINVAL`,
/// above a page `E2BIG`, unreadable `EFAULT`, non-zero bytes past the
/// 24 known `E2BIG`, then the fields ([`Handled::parse`]).
pub fn landlock_create_ruleset(k: &mut dyn LinuxKernel, a: &Args) -> SysResult {
    let (attr, size, flags) = (a.ptr(0), a.ulong(1), a.uint(2));
    match flags {
        0 => {}
        LANDLOCK_CREATE_RULESET_VERSION | LANDLOCK_CREATE_RULESET_ERRATA => {
            if !attr.is_null() || size != 0 {
                return Err(Errno::EINVAL);
            }
            return Ok(if flags == LANDLOCK_CREATE_RULESET_VERSION { landlock::ABI_VERSION } else { landlock::ERRATA });
        }
        _ => return Err(Errno::EINVAL),
    }
    if size < RULESET_ATTR_MIN as u64 {
        return Err(Errno::EINVAL);
    }
    if size > landlock::MAX_ATTR_SIZE as u64 {
        return Err(Errno::E2BIG);
    }
    let mut bytes = vec![0u8; size as usize];
    k.copy_from_user(attr, &mut bytes)?;
    if bytes.len() > RULESET_ATTR_SIZE && bytes[RULESET_ATTR_SIZE..].iter().any(|&b| b != 0) {
        return Err(Errno::E2BIG);
    }
    let handled = Handled::parse(&bytes[..bytes.len().min(RULESET_ATTR_SIZE)])?;
    file_io(k)?.landlock_ruleset_fd(Ruleset::new(handled)).map(|fd| fd as u64)
}

/// The ruleset behind descriptor `fd`: `EBADF` if it is not open (or
/// negative), `EBADFD` if it is something else.
fn ruleset(k: &mut dyn LinuxKernel, fd: i32) -> Result<Arc<Ruleset>, Errno> {
    if fd < 0 {
        return Err(Errno::EBADF);
    }
    file_io(k)?.landlock_ruleset(fd)?.ok_or(Errno::EBADFD)
}

/// landlock_add_rule(ruleset_fd, rule_type, rule_attr, flags): flags must
/// be 0 (`EINVAL`), then the ruleset (`EBADF`, `EBADFD`), the type
/// (`EINVAL`), the attribute (`EFAULT`), its access bits
/// ([`landlock::check_allowed`]), then for a path rule `parent_fd`
/// (`EBADF`; `EBADFD` for a descriptor that is no file of a file system)
/// and for a port rule the port (`EINVAL` above 65535).
pub fn landlock_add_rule(k: &mut dyn LinuxKernel, a: &Args) -> SysResult {
    if a.uint(3) != 0 {
        return Err(Errno::EINVAL);
    }
    let rs = ruleset(k, a.fd(0))?;
    match a.uint(1) {
        LANDLOCK_RULE_PATH_BENEATH => {
            let mut b = [0u8; PATH_BENEATH_ATTR_SIZE];
            k.copy_from_user(a.ptr(2), &mut b)?;
            let allowed = u64::from_le_bytes(b[..8].try_into().unwrap_or([0; 8]));
            let parent = i32::from_le_bytes(b[8..12].try_into().unwrap_or([0; 4]));
            landlock::check_allowed(allowed, rs.handled().fs)?;
            if parent < 0 {
                return Err(Errno::EBADF);
            }
            let (object, is_dir) = fs(k)?.fd_object(parent)?.ok_or(Errno::EBADFD)?;
            rs.add(Rule::PathBeneath { allowed, object, is_dir })?;
        }
        LANDLOCK_RULE_NET_PORT => {
            let mut b = [0u8; NET_PORT_ATTR_SIZE];
            k.copy_from_user(a.ptr(2), &mut b)?;
            let allowed = u64::from_le_bytes(b[..8].try_into().unwrap_or([0; 8]));
            let port = u64::from_le_bytes(b[8..16].try_into().unwrap_or([0; 8]));
            rs.add(Rule::NetPort { allowed, port })?;
        }
        _ => return Err(Errno::EINVAL),
    }
    Ok(0)
}

/// landlock_restrict_self(ruleset_fd, flags): a caller without
/// `CAP_SYS_ADMIN` must have set no_new_privs (`EPERM`, before anything
/// else); unknown flags `EINVAL`; `-1` with `LOG_SUBDOMAINS_OFF` alone
/// only records that; otherwise the ruleset (`EBADF`, `EBADFD`) is
/// stacked on the calling thread's domain (`E2BIG` past 16 layers).
pub fn landlock_restrict_self(k: &mut dyn LinuxKernel, a: &Args) -> SysResult {
    let creds = cred::current(k);
    if !creds.no_new_privs && !creds.capable(crate::cred::cap::SYS_ADMIN) {
        return Err(Errno::EPERM);
    }
    let (fd, flags) = (a.fd(0), a.uint(1));
    if flags & !RESTRICT_SELF_ALL != 0 {
        return Err(Errno::EINVAL);
    }
    let subdomains_off = flags & LANDLOCK_RESTRICT_SELF_LOG_SUBDOMAINS_OFF != 0;
    if fd == -1 && flags == LANDLOCK_RESTRICT_SELF_LOG_SUBDOMAINS_OFF {
        cred::change(k, |c| {
            if let Some(d) = &c.landlock {
                let mut d = Domain::clone(d);
                d.log_subdomains_off = true;
                c.landlock = Some(Arc::new(d));
            }
        });
        return Ok(0);
    }
    let rules = ruleset(k, fd)?.snapshot();
    let mut domain = Domain::stack(creds.landlock.as_deref(), rules)?;
    domain.log_subdomains_off |= subdomains_off;
    let domain = Arc::new(domain);
    cred::change(k, |c| c.landlock = Some(domain));
    Ok(0)
}

/// The calling thread's Landlock domain, if it has one.
fn domain(k: &mut dyn LinuxKernel) -> Option<Arc<Domain>> {
    cred::current(k).landlock
}

/// `O_NOFOLLOW`, `O_EXCL` and the `O_TMPFILE` bit (UAPI
/// `asm-generic/fcntl.h`).
const O_NOFOLLOW: u32 = 0o400000;
const O_EXCL: u32 = 0o200;
const O_TMPFILE_BIT: u32 = 0o20000000;

/// Before open(2) / openat(2): in a domain that handles file accesses,
/// the accesses the flags ask for ([`landlock::open_access`]) on the
/// object, or `MAKE_REG` on its directory for a file `O_CREAT` would make
/// (and for `O_TMPFILE`), must be allowed: `EACCES` otherwise. Lookup
/// errors are left to the open itself.
pub fn check_open(k: &mut dyn LinuxKernel, dirfd: i32, path: &[u8], flags: u32) -> Result<(), Errno> {
    use landlock::oflags::{O_CREAT, O_PATH};
    let Some(d) = domain(k) else { return Ok(()) };
    if !d.handles_fs(ACCESS_FS_ALL) || flags & O_PATH != 0 {
        return Ok(());
    }
    let fs = fs(k)?;
    if flags & O_TMPFILE_BIT != 0 {
        let Ok((chain, _)) = fs.path_ancestry(dirfd, path, true) else { return Ok(()) };
        return allow(d.allows_fs(&chain, LANDLOCK_ACCESS_FS_MAKE_REG));
    }
    match fs.path_ancestry(dirfd, path, flags & O_NOFOLLOW == 0) {
        Ok(_) if flags & (O_CREAT | O_EXCL) == O_CREAT | O_EXCL => Ok(()),
        Ok((chain, is_dir)) => allow(d.allows_fs(&chain, landlock::open_access(flags, is_dir))),
        Err(Errno::ENOENT) if flags & O_CREAT != 0 => match fs.parent_ancestry(dirfd, path) {
            Ok(chain) => allow(d.allows_fs(&chain, LANDLOCK_ACCESS_FS_MAKE_REG)),
            Err(_) => Ok(()),
        },
        Err(_) => Ok(()),
    }
}

fn allow(ok: bool) -> Result<(), Errno> {
    if ok { Ok(()) } else { Err(Errno::EACCES) }
}

/// Before bind(2) (`connect` false) or connect(2) on `fd` with `addr`: in
/// a domain that handles the access, a TCP socket's port must be allowed
/// (`EACCES`). Other sockets are not restricted.
pub fn check_socket(k: &mut dyn LinuxKernel, fd: i32, addr: &SockAddr, connect: bool) -> Result<(), Errno> {
    let access = if connect { LANDLOCK_ACCESS_NET_CONNECT_TCP } else { LANDLOCK_ACCESS_NET_BIND_TCP };
    let Some(d) = domain(k) else { return Ok(()) };
    if !d.handles_net(access) {
        return Ok(());
    }
    let port = match addr {
        SockAddr::Inet(a) => a.port(),
        SockAddr::Inet6(a) => a.port(),
        _ => return Ok(()),
    };
    let Some(net) = k.net() else { return Ok(()) };
    if net.getsockopt(fd, OptName::Type) != Ok(OptValue::Int(SOCK_STREAM as i32)) {
        return Ok(());
    }
    allow(d.allows_net(port, access))
}

/// Before a call that makes (`MAKE_*`) or removes (`REMOVE_*`) an entry of
/// the directory the last component of `path` is in: in a domain that
/// handles `access`, it must be allowed on that directory (`EACCES`).
pub fn check_parent(k: &mut dyn LinuxKernel, dirfd: i32, path: &[u8], access: u64) -> Result<(), Errno> {
    let Some(d) = domain(k) else { return Ok(()) };
    if !d.handles_fs(access) {
        return Ok(());
    }
    match fs(k)?.parent_ancestry(dirfd, path) {
        Ok(chain) => allow(d.allows_fs(&chain, access)),
        Err(_) => Ok(()),
    }
}

/// Before a call that acts on the object `path` names (truncate(2)
/// `TRUNCATE`, execve(2) `EXECUTE`): in a domain that handles `access`,
/// it must be allowed on the object (`EACCES`).
pub fn check_object(k: &mut dyn LinuxKernel, dirfd: i32, path: &[u8], follow: bool, access: u64) -> Result<(), Errno> {
    let Some(d) = domain(k) else { return Ok(()) };
    if !d.handles_fs(access) {
        return Ok(());
    }
    match fs(k)?.path_ancestry(dirfd, path, follow) {
        Ok((chain, _)) => allow(d.allows_fs(&chain, access)),
        Err(_) => Ok(()),
    }
}

/// unlink(2) / rmdir(2): `REMOVE_FILE` or `REMOVE_DIR` on the directory.
pub fn check_remove(k: &mut dyn LinuxKernel, dirfd: i32, path: &[u8], dir: bool) -> Result<(), Errno> {
    check_parent(k, dirfd, path, if dir { LANDLOCK_ACCESS_FS_REMOVE_DIR } else { LANDLOCK_ACCESS_FS_REMOVE_FILE })
}

/// rename(2) (`rename` true) and link(2) from `old` to `new`: making the
/// entry in the new directory and (rename) removing it from the old one
/// as for [`check_parent`] (`EACCES`); between different directories each
/// layer that handles file accesses must also allow `REFER` on both
/// (landlock.rst: the one right denied by every ruleset unless a rule
/// grants it), else `EXDEV`.
pub fn check_move(
    k: &mut dyn LinuxKernel,
    olddirfd: i32,
    old: &[u8],
    newdirfd: i32,
    new: &[u8],
    rename: bool,
) -> Result<(), Errno> {
    let Some(d) = domain(k) else { return Ok(()) };
    if !d.handles_fs(ACCESS_FS_ALL) {
        return Ok(());
    }
    let fs = fs(k)?;
    let Ok((_, is_dir)) = fs.path_ancestry(olddirfd, old, false) else { return Ok(()) };
    let (Ok(from), Ok(to)) = (fs.parent_ancestry(olddirfd, old), fs.parent_ancestry(newdirfd, new)) else {
        return Ok(());
    };
    let make = if is_dir { LANDLOCK_ACCESS_FS_MAKE_DIR } else { LANDLOCK_ACCESS_FS_MAKE_REG };
    let remove = if is_dir { LANDLOCK_ACCESS_FS_REMOVE_DIR } else { LANDLOCK_ACCESS_FS_REMOVE_FILE };
    allow(d.allows_fs(&to, make) && (!rename || d.allows_fs(&from, remove)))?;
    if from.first() != to.first() && !(d.allows_refer(&from) && d.allows_refer(&to)) {
        return Err(Errno::EXDEV);
    }
    Ok(())
}

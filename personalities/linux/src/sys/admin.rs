//! System administration calls (ADR 0051): mount(2), umount2(2),
//! mknod(2) / mknodat(2), chroot(2), sethostname(2), setdomainname(2) and
//! reboot(2), checked here and carried out by the kernel's
//! [`LinuxAdmin`].
//!
//! Order of the checks (the man pages give no order; ours, to be
//! confirmed by the diff runner): unknown flags or commands (`EINVAL`),
//! then the privilege (`EPERM`), then the arguments in order (`EFAULT`,
//! `ENAMETOOLONG`, ...), then the kernel.

use crate::admin::{LinuxAdmin, MountFlags, MountRequest, Reboot, SystemName, cmd, mnt, ms};
use crate::args::Args;
use crate::cred::{self, cap};
use crate::errno::{Errno, SysResult};
use crate::fs::at::AT_FDCWD;
use crate::kernel::{LinuxKernel, UserPtr};
use crate::sys::io::bounce;
use crate::sys::path::{nonempty_path, path_arg};
use crate::table::{Handler, nr};
use alloc::vec::Vec;

/// The calls of this module, for [`crate::table::GROUPS`].
pub const CALLS: &[(u64, Handler)] = &[
    (nr::MKNOD, mknod),
    (nr::CHROOT, chroot),
    (nr::MOUNT, mount),
    (nr::UMOUNT2, umount2),
    (nr::REBOOT, reboot),
    (nr::SETHOSTNAME, sethostname),
    (nr::SETDOMAINNAME, setdomainname),
    (nr::MKNODAT, mknodat),
];

/// `__NEW_UTS_LEN` (`linux/utsname.h`; `HOST_NAME_MAX` in POSIX terms):
/// the longest host or domain name.
pub const NAME_MAX_LEN: usize = 64;

/// The page size: mount(2) copies at most one page of `data`, and file
/// system type names and sources longer than that are refused.
const PAGE: usize = 4096;

/// The kernel's administration services, `ENOSYS` without them.
fn admin(k: &mut dyn LinuxKernel) -> Result<&mut dyn LinuxAdmin, Errno> {
    k.admin().ok_or(Errno::ENOSYS)
}

fn require(k: &mut dyn LinuxKernel, capability: u32) -> Result<(), Errno> {
    if cred::current(k).capable(capability) { Ok(()) } else { Err(Errno::EPERM) }
}

// ---- mount -------------------------------------------------------------

/// A string argument of mount(2) (source, type): at most a page.
fn string_arg(k: &dyn LinuxKernel, p: UserPtr) -> Result<Vec<u8>, Errno> {
    if p.is_null() {
        return Ok(Vec::new());
    }
    let mut buf = bounce(PAGE)?;
    let n = k.strncpy_from_user(p, &mut buf)?;
    buf.truncate(n);
    Ok(buf)
}

/// mount(2)'s `data`: the file system's options, at most a page, up to
/// the first NUL. A string that runs to the end of its page without a NUL
/// is taken as it is.
fn data_arg(k: &dyn LinuxKernel, p: UserPtr) -> Result<Vec<u8>, Errno> {
    if p.is_null() {
        return Ok(Vec::new());
    }
    let mut buf = bounce(PAGE)?;
    match k.strncpy_from_user(p, &mut buf) {
        Ok(n) => buf.truncate(n),
        Err(Errno::ENAMETOOLONG) => {}
        Err(e) => return Err(e),
    }
    Ok(buf)
}

/// Decode mount(2)'s arguments into a request (the flags decide the
/// operation, in the order mount(2) lists them).
pub fn mount_request(
    k: &dyn LinuxKernel,
    source: UserPtr,
    target: UserPtr,
    fstype: UserPtr,
    flags: u64,
    data: UserPtr,
) -> Result<MountRequest, Errno> {
    // mount(2): "If the mountflags argument has the magic number 0xC0ED
    // (MS_MGC_VAL) in the top 16 bits ... then the magic number is ignored".
    let flags = if flags & ms::MGC_MSK == ms::MGC_VAL { flags & !ms::MGC_MSK } else { flags };
    let per_mount = MountFlags::from_bits(flags);
    if flags & ms::REMOUNT != 0 {
        let target = nonempty_path(k, target)?;
        let data = if flags & ms::BIND != 0 { Vec::new() } else { data_arg(k, data)? };
        return Ok(MountRequest::Remount { target, flags: per_mount, data });
    }
    if flags & ms::BIND != 0 {
        let source = nonempty_path(k, source)?;
        let target = nonempty_path(k, target)?;
        return Ok(MountRequest::Bind { source, target, flags: per_mount, recursive: flags & ms::REC != 0 });
    }
    if flags & ms::PROPAGATION != 0 {
        // mount(2): exactly one propagation type (plus MS_REC and MS_SILENT).
        if (flags & ms::PROPAGATION).count_ones() != 1 {
            return Err(Errno::EINVAL);
        }
        return Ok(MountRequest::Propagation { target: nonempty_path(k, target)? });
    }
    if flags & ms::MOVE != 0 {
        let source = nonempty_path(k, source)?;
        let target = nonempty_path(k, target)?;
        return Ok(MountRequest::Move { source, target });
    }
    let source = string_arg(k, source)?;
    let target = nonempty_path(k, target)?;
    let fstype = string_arg(k, fstype)?;
    if fstype.is_empty() {
        // An unknown (or missing) file system type: "ENODEV filesystemtype
        // not configured in the kernel".
        return Err(Errno::ENODEV);
    }
    let data = data_arg(k, data)?;
    Ok(MountRequest::New { source, target, fstype, flags: per_mount, data })
}

/// mount(source, target, filesystemtype, mountflags, data):
/// `CAP_SYS_ADMIN` (`EPERM`).
pub fn mount(k: &mut dyn LinuxKernel, a: &Args) -> SysResult {
    require(k, cap::SYS_ADMIN)?;
    let req = mount_request(k, a.ptr(0), a.ptr(1), a.ptr(2), a.ulong(3), a.ptr(4))?;
    admin(k)?.mount(&req).map(|()| 0)
}

/// umount2(target, flags): `MNT_DETACH` and `UMOUNT_NOFOLLOW` are
/// honoured; `MNT_FORCE` asks to abort pending requests, which Poly's file
/// systems never leave, so it unmounts like a plain unmount;
/// `MNT_EXPIRE` is not supported (`EAGAIN` would need two calls, and with
/// `MNT_FORCE` or `MNT_DETACH` it is `EINVAL`).
pub fn umount2(k: &mut dyn LinuxKernel, a: &Args) -> SysResult {
    let flags = a.uint(1);
    if flags & !(mnt::FORCE | mnt::DETACH | mnt::EXPIRE | mnt::UMOUNT_NOFOLLOW) != 0 {
        return Err(Errno::EINVAL);
    }
    if flags & mnt::EXPIRE != 0 && flags & (mnt::FORCE | mnt::DETACH) != 0 {
        return Err(Errno::EINVAL);
    }
    require(k, cap::SYS_ADMIN)?;
    let target = nonempty_path(k, a.ptr(0))?;
    if flags & mnt::EXPIRE != 0 {
        return Err(Errno::EINVAL);
    }
    admin(k)?.umount(&target, flags & mnt::DETACH != 0, flags & mnt::UMOUNT_NOFOLLOW == 0).map(|()| 0)
}

// ---- device nodes, chroot ---------------------------------------------

/// The file type bits of a mode (`S_IFMT`, `linux/stat.h`).
const S_IFMT: u32 = 0o170000;
const S_IFSOCK: u32 = 0o140000;
const S_IFREG: u32 = 0o100000;
const S_IFBLK: u32 = 0o060000;
const S_IFCHR: u32 = 0o020000;
const S_IFIFO: u32 = 0o010000;

fn mknod_common(k: &mut dyn LinuxKernel, dirfd: i32, p: UserPtr, mode: u32, dev: u64) -> SysResult {
    // mknod(2): "zero file type is equivalent to type S_IFREG".
    let mode = if mode & S_IFMT == 0 { mode | S_IFREG } else { mode };
    match mode & S_IFMT {
        S_IFREG | S_IFIFO | S_IFSOCK => {}
        // "EPERM mode requested creation of something other than a regular
        // file, FIFO (named pipe), or UNIX domain socket, and the caller is
        // not privileged (Linux: does not have the CAP_MKNOD capability)".
        S_IFCHR | S_IFBLK => require(k, cap::MKNOD)?,
        // "EINVAL mode requested creation of something other than a
        // regular file, device special file, FIFO or socket."
        _ => return Err(Errno::EINVAL),
    }
    let path = nonempty_path(k, p)?;
    // The system call's `dev` is an `unsigned int`.
    let dev = dev & 0xffff_ffff;
    admin(k)?.mknodat(dirfd, &path, mode & (S_IFMT | 0o7777), dev).map(|()| 0)
}

/// mknodat(dirfd, pathname, mode, dev)
pub fn mknodat(k: &mut dyn LinuxKernel, a: &Args) -> SysResult {
    mknod_common(k, a.fd(0), a.ptr(1), a.uint(2), a.ulong(3))
}

/// mknod(pathname, mode, dev)
pub fn mknod(k: &mut dyn LinuxKernel, a: &Args) -> SysResult {
    mknod_common(k, AT_FDCWD, a.ptr(0), a.uint(1), a.ulong(2))
}

/// chroot(path): `CAP_SYS_CHROOT` (`EPERM`).
pub fn chroot(k: &mut dyn LinuxKernel, a: &Args) -> SysResult {
    require(k, cap::SYS_CHROOT)?;
    let path = path_arg(k, a.ptr(0))?;
    if path.is_empty() {
        return Err(Errno::ENOENT);
    }
    admin(k)?.chroot(&path).map(|()| 0)
}

// ---- names ------------------------------------------------------------

fn set_name(k: &mut dyn LinuxKernel, a: &Args, which: SystemName) -> SysResult {
    // sethostname(2): "EINVAL len is negative or ... len is larger than the
    // maximum allowed size"; "EPERM the caller did not have the
    // CAP_SYS_ADMIN capability"; "EFAULT name is an invalid address".
    let len = a.long(1);
    if len < 0 || len as usize > NAME_MAX_LEN {
        return Err(Errno::EINVAL);
    }
    require(k, cap::SYS_ADMIN)?;
    let mut name = alloc::vec![0u8; len as usize];
    k.copy_from_user(a.ptr(0), &mut name)?;
    admin(k)?.set_system_name(which, &name).map(|()| 0)
}

/// sethostname(name, len)
pub fn sethostname(k: &mut dyn LinuxKernel, a: &Args) -> SysResult {
    set_name(k, a, SystemName::Host)
}

/// setdomainname(name, len)
pub fn setdomainname(k: &mut dyn LinuxKernel, a: &Args) -> SysResult {
    set_name(k, a, SystemName::Domain)
}

// ---- reboot -----------------------------------------------------------

/// `LINUX_REBOOT_MAGIC1` and the values `LINUX_REBOOT_MAGIC2` may take
/// (`linux/reboot.h`).
pub const MAGIC1: u32 = 0xfee1_dead;
pub const MAGIC2: [u32; 4] = [672_274_793, 85_072_278, 369_367_448, 537_993_216];

/// reboot(magic, magic2, cmd, arg): `EINVAL` for wrong magic numbers or an
/// unknown command, `EPERM` without `CAP_SYS_BOOT`. `CAD_ON` / `CAD_OFF`
/// set what Ctrl-Alt-Del does; `RESTART`, `RESTART2` (its command string
/// is ignored), `HALT` and `POWER_OFF` do not return when they work.
/// Suspending to disk and kexec are not supported (`EINVAL`, as on a
/// kernel without them). reboot(2): the call does not sync the disks
/// itself; Poly's kernel writes back and unmounts its disk file systems
/// before the machine goes down anyway.
pub fn reboot(k: &mut dyn LinuxKernel, a: &Args) -> SysResult {
    if a.uint(0) != MAGIC1 || !MAGIC2.contains(&a.uint(1)) {
        return Err(Errno::EINVAL);
    }
    let how = match a.uint(2) {
        cmd::CAD_ON | cmd::CAD_OFF => None,
        cmd::RESTART | cmd::RESTART2 => Some(Reboot::Restart),
        cmd::HALT => Some(Reboot::Halt),
        cmd::POWER_OFF => Some(Reboot::PowerOff),
        _ => return Err(Errno::EINVAL),
    };
    require(k, cap::SYS_BOOT)?;
    if cmd::RESTART2 == a.uint(2) {
        // The command string must at least be readable.
        let mut buf = bounce(256)?;
        match k.strncpy_from_user(a.ptr(3), &mut buf) {
            Ok(_) | Err(Errno::ENAMETOOLONG) => {}
            Err(e) => return Err(e),
        }
    }
    let admin = admin(k)?;
    match how {
        None => {
            admin.set_ctrl_alt_del(a.uint(2) == cmd::CAD_ON);
            Ok(0)
        }
        Some(how) => Err(admin.reboot(how)),
    }
}

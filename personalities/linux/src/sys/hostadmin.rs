//! The administration calls for facilities Poly has little or none of
//! (ADR 0135): swap areas (swapon(2), swapoff(2)), disk quotas
//! (quotactl(2), quotactl_fd(2)), Linux kernel modules (init_module(2),
//! finit_module(2), delete_module(2)), hanging up the controlling
//! terminal (vhangup(2)) and process accounting (acct(2)). Their
//! arguments are checked in the order the spec sheets record, and what
//! is left fails the way the reference does on a machine without the
//! facility: no target can be a swap area or a quota-enabled file system,
//! and no image is a module Poly can load.

use crate::abi::mode::{S_IFBLK, S_IFDIR, S_IFMT, S_IFREG};
use crate::args::Args;
use crate::cred::{self, cap};
use crate::errno::{Errno, SysResult};
use crate::fs::at::AT_FDCWD;
use crate::kernel::{LinuxKernel, UserPtr};
use crate::sys::io::bounce;
use crate::sys::path::{fs, path_arg};
use crate::table::{Handler, nr};
use alloc::vec::Vec;

/// The calls, registered in [`crate::table::GROUPS`].
pub const CALLS: &[(u64, Handler)] = &[
    (nr::VHANGUP, vhangup),
    (nr::ACCT, acct),
    (nr::SWAPON, swapon),
    (nr::SWAPOFF, swapoff),
    (nr::QUOTACTL, quotactl),
    (nr::QUOTACTL_FD, quotactl_fd),
    (nr::INIT_MODULE, init_module),
    (nr::FINIT_MODULE, finit_module),
    (nr::DELETE_MODULE, delete_module),
];

fn capable(k: &mut dyn LinuxKernel, capability: u32) -> bool {
    cred::current(k).capable_initial(capability)
}

/// The path at `p`, which must not be empty (`ENOENT`).
fn nonempty(k: &mut dyn LinuxKernel, p: UserPtr) -> Result<Vec<u8>, Errno> {
    let path = path_arg(k, p)?;
    if path.is_empty() { Err(Errno::ENOENT) } else { Ok(path) }
}

/// The file type bits of the file `path` names (symbolic links followed).
fn file_type(k: &mut dyn LinuxKernel, path: &[u8]) -> Result<u32, Errno> {
    Ok(fs(k)?.statat(AT_FDCWD, path, true)?.stat.mode & S_IFMT)
}

// ---- terminals ------------------------------------------------------------

/// vhangup(): `CAP_SYS_TTY_CONFIG` (`EPERM` without it, with or without a
/// controlling terminal); then the caller's controlling terminal, if it
/// has one, is hung up: its session leader gets `SIGHUP`, the descriptors
/// open on it die, the session loses it. Without one nothing happens.
pub fn vhangup(k: &mut dyn LinuxKernel, _a: &Args) -> SysResult {
    if !capable(k, cap::SYS_TTY_CONFIG) {
        return Err(Errno::EPERM);
    }
    let pid = k.pid();
    if let Ok(Some(sid)) = crate::task::with_tree(k, |t| t.sid(pid)) {
        k.hangup_terminal(sid);
    }
    Ok(0)
}

// ---- process accounting -------------------------------------------------------

/// acct(filename): `CAP_SYS_PACCT` first, for every argument. NULL turns
/// the pid namespace's accounting off, a file turns it on or switches it;
/// the file being left gets one record for the caller, as if it ended
/// (nothing when accounting was off). The file must exist and be a
/// regular file (`EISDIR` for a directory, `EACCES` for anything else);
/// a call that fails leaves accounting as it was. Records of ending
/// processes are written by [`crate::acct::process_exited`].
pub fn acct(k: &mut dyn LinuxKernel, a: &Args) -> SysResult {
    if !capable(k, cap::SYS_PACCT) {
        return Err(Errno::EPERM);
    }
    let ns = k.pid_ns();
    if a.ptr(0).is_null() {
        if let Some(old) = crate::acct::clear(&ns) {
            old.append(&crate::acct::caller_record(k, &ns).to_bytes());
        }
        return Ok(0);
    }
    let path = nonempty(k, a.ptr(0))?;
    match file_type(k, &path)? {
        S_IFREG => {}
        S_IFDIR => return Err(Errno::EISDIR),
        _ => return Err(Errno::EACCES),
    }
    let sink = k.acct_open(&path)?;
    if let Some(old) = crate::acct::set(&ns, sink) {
        old.append(&crate::acct::caller_record(k, &ns).to_bytes());
    }
    Ok(0)
}

// ---- swap ---------------------------------------------------------------

/// `SWAP_FLAG_PREFER`, `SWAP_FLAG_PRIO_MASK`, `SWAP_FLAG_DISCARD`,
/// `SWAP_FLAG_DISCARD_ONCE`, `SWAP_FLAG_DISCARD_PAGES` (`linux/swap.h`).
pub const SWAP_FLAGS: u32 = 0x8000 | 0x7fff | 0x1_0000 | 0x2_0000 | 0x4_0000;

/// swapon(path, flags): unknown flags are `EINVAL` before anything else;
/// then `CAP_SYS_ADMIN`; then the path. Poly has no swap: a directory is
/// `EISDIR`, every other file `EINVAL` (no swap header, or a file system
/// that cannot hold swap).
pub fn swapon(k: &mut dyn LinuxKernel, a: &Args) -> SysResult {
    if a.uint(1) & !SWAP_FLAGS != 0 {
        return Err(Errno::EINVAL);
    }
    if !capable(k, cap::SYS_ADMIN) {
        return Err(Errno::EPERM);
    }
    let path = nonempty(k, a.ptr(0))?;
    swap_target(k, &path)
}

/// swapoff(path): `CAP_SYS_ADMIN` first (the path is not read); no area
/// is active, so a directory is `EISDIR` and every other file `EINVAL`.
pub fn swapoff(k: &mut dyn LinuxKernel, a: &Args) -> SysResult {
    if !capable(k, cap::SYS_ADMIN) {
        return Err(Errno::EPERM);
    }
    let path = nonempty(k, a.ptr(0))?;
    swap_target(k, &path)
}

fn swap_target(k: &mut dyn LinuxKernel, path: &[u8]) -> SysResult {
    match file_type(k, path)? {
        S_IFDIR => Err(Errno::EISDIR),
        _ => Err(Errno::EINVAL),
    }
}

// ---- quotas ---------------------------------------------------------------

/// `MAXQUOTAS` (`linux/quota.h`): user, group and project quotas.
const MAXQUOTAS: u32 = 3;
/// `Q_SYNC` (`linux/quota.h`), the sub-command of `QCMD(Q_SYNC, type)`: the
/// command word is it shifted left by 8, the quota type in the low byte.
const Q_SYNC_SUBCMD: u32 = 0x80_0001;

/// quotactl(cmd, special, id, addr): the type (low byte of `cmd`) must be
/// below `MAXQUOTAS`; `Q_SYNC` without a device syncs everything; any other
/// command without one is `ENODEV`; a device is looked up as a path and
/// must be a block device (`ENOTBLK`), and Poly has none with quotas
/// (`ENODEV`). No permission is checked before that.
pub fn quotactl(k: &mut dyn LinuxKernel, a: &Args) -> SysResult {
    let cmd = a.uint(0);
    if cmd & 0xff >= MAXQUOTAS {
        return Err(Errno::EINVAL);
    }
    if a.ptr(1).is_null() {
        if cmd >> 8 == Q_SYNC_SUBCMD {
            fs(k)?.sync();
            return Ok(0);
        }
        return Err(Errno::ENODEV);
    }
    let path = nonempty(k, a.ptr(1))?;
    if file_type(k, &path)? != S_IFBLK {
        return Err(Errno::ENOTBLK);
    }
    Err(Errno::ENODEV)
}

/// quotactl_fd(fd, cmd, id, addr): `fd` must be open (`EBADF`, whatever
/// else is wrong), the type below `MAXQUOTAS` (`EINVAL`); no file system
/// of Poly has quota operations (`ENOSYS`, for every command).
pub fn quotactl_fd(k: &mut dyn LinuxKernel, a: &Args) -> SysResult {
    fs(k)?.status_flags(a.fd(0))?;
    if a.uint(1) & 0xff >= MAXQUOTAS {
        return Err(Errno::EINVAL);
    }
    Err(Errno::ENOSYS)
}

// ---- modules ----------------------------------------------------------------

/// The largest module image init_module(2) copies. Past it the allocation
/// fails with `ENOMEM`, before anything is read.
const MODULE_MAX: u64 = 1 << 30;
/// `MODULE_NAME_LEN` (`linux/module.h`) is 56 with the NUL: delete_module
/// reads at most 55 bytes of the name.
const MODULE_NAME_LEN: usize = 56;
/// `MODULE_INIT_IGNORE_MODVERSIONS`, `MODULE_INIT_IGNORE_VERMAGIC` and
/// `MODULE_INIT_COMPRESSED_FILE` (`linux/module.h`).
const MODULE_INIT_FLAGS: u32 = 1 | 2 | 4;
const MODULE_INIT_COMPRESSED_FILE: u32 = 4;

/// `O_ACCMODE`, `O_WRONLY` and `O_PATH` (`asm-generic/fcntl.h`).
const O_ACCMODE: u32 = 3;
const O_WRONLY: u32 = 1;
const O_PATH: u32 = 0o10000000;

/// init_module(image, len, params): `CAP_SYS_MODULE` first; a length no
/// image has is `ENOMEM`; the image must be readable in full (`EFAULT`,
/// except for length 0, which reads nothing); and Poly cannot load any
/// Linux module, so what is readable is `ENOEXEC`, before `params` is
/// read.
pub fn init_module(k: &mut dyn LinuxKernel, a: &Args) -> SysResult {
    if !capable(k, cap::SYS_MODULE) {
        return Err(Errno::EPERM);
    }
    let (image, len) = (a.ptr(0), a.ulong(1));
    if len > MODULE_MAX {
        return Err(Errno::ENOMEM);
    }
    // Every byte of the image is read (and dropped), a page at a time.
    let mut chunk = bounce(4096)?;
    let mut done = 0u64;
    while done < len {
        let n = (len - done).min(chunk.len() as u64) as usize;
        k.copy_from_user(image.offset(done).ok_or(Errno::EFAULT)?, &mut chunk[..n])?;
        done += n as u64;
    }
    Err(Errno::ENOEXEC)
}

/// finit_module(fd, params, flags): `CAP_SYS_MODULE` first; the flags
/// (`EINVAL`) before the descriptor, which must be open for reading
/// (`EBADF`, also `O_PATH`); a regular file with data (`EINVAL`) that no
/// one has open for writing (`ETXTBSY`); in-kernel decompression is not
/// there (`EOPNOTSUPP`); and Poly cannot load any Linux module
/// (`ENOEXEC`).
pub fn finit_module(k: &mut dyn LinuxKernel, a: &Args) -> SysResult {
    if !capable(k, cap::SYS_MODULE) {
        return Err(Errno::EPERM);
    }
    let (fd, flags) = (a.fd(0), a.uint(2));
    if flags & !MODULE_INIT_FLAGS != 0 {
        return Err(Errno::EINVAL);
    }
    let open = fs(k)?.status_flags(fd)?;
    if open & O_PATH != 0 || open & O_ACCMODE == O_WRONLY {
        return Err(Errno::EBADF);
    }
    let st = fs(k)?.statat(fd, b"", true)?.stat;
    if st.mode & S_IFMT != S_IFREG || st.size <= 0 {
        return Err(Errno::EINVAL);
    }
    if fs(k)?.open_for_writing(fd)? {
        return Err(Errno::ETXTBSY);
    }
    if flags & MODULE_INIT_COMPRESSED_FILE != 0 {
        return Err(Errno::EOPNOTSUPP);
    }
    Err(Errno::ENOEXEC)
}

/// delete_module(name, flags): `CAP_SYS_MODULE` first (the name is not
/// read); the name is at most 55 bytes (`EFAULT` if unreadable, a longer
/// one is cut); no module is loaded, so every name is `ENOENT`.
pub fn delete_module(k: &mut dyn LinuxKernel, a: &Args) -> SysResult {
    if !capable(k, cap::SYS_MODULE) {
        return Err(Errno::EPERM);
    }
    let mut name = [0u8; MODULE_NAME_LEN - 1];
    match k.strncpy_from_user(a.ptr(0), &mut name) {
        Ok(_) | Err(Errno::ENAMETOOLONG) => Err(Errno::ENOENT),
        Err(e) => Err(e),
    }
}

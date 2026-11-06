//! The Linux security module calls: lsm_list_modules(2),
//! lsm_get_self_attr(2), lsm_set_self_attr(2).
//!
//! Poly's active modules are capability (always first) and Landlock
//! ([`crate::landlock`]); neither has task attributes, so no module
//! answers or sets one: once their arguments pass the checks (in the
//! order the spec sheets give), getting an attribute fails with
//! `EOPNOTSUPP` and `*size` 0, setting one with `EOPNOTSUPP`.

use crate::args::Args;
use crate::errno::{Errno, SysResult};
use crate::kernel::{LinuxKernel, UserPtr};
use crate::table::{Handler, nr};
use alloc::vec;
use alloc::vec::Vec;

/// The calls, registered in [`crate::table::GROUPS`].
pub const CALLS: &[(u64, Handler)] = &[
    (nr::LSM_GET_SELF_ATTR, lsm_get_self_attr),
    (nr::LSM_SET_SELF_ATTR, lsm_set_self_attr),
    (nr::LSM_LIST_MODULES, lsm_list_modules),
];

/// `LSM_ID_CAPABILITY` (`linux/lsm.h`).
pub const LSM_ID_CAPABILITY: u64 = 100;
/// `LSM_ID_LANDLOCK` (`linux/lsm.h`).
pub const LSM_ID_LANDLOCK: u64 = 110;
/// `LSM_ATTR_UNDEF` (`linux/lsm.h`).
pub const LSM_ATTR_UNDEF: u32 = 0;
/// `LSM_FLAG_SINGLE` (`linux/lsm.h`): ask the one module `ctx->id` names.
pub const LSM_FLAG_SINGLE: u32 = 1;
/// The size of `struct lsm_ctx`'s header (`linux/lsm.h`: `id`, `flags`,
/// `len`, `ctx_len`, each a `__u64`).
pub const LSM_CTX_HEADER: u64 = 32;
/// The largest context lsm_set_self_attr(2) takes (a page).
pub const SET_ATTR_MAX: u32 = 4096;

/// The active modules, capability first.
pub const MODULES: [u64; 2] = [LSM_ID_CAPABILITY, LSM_ID_LANDLOCK];

fn read_u32(k: &mut dyn LinuxKernel, ptr: UserPtr) -> Result<u32, Errno> {
    let mut b = [0u8; 4];
    k.copy_from_user(ptr, &mut b)?;
    Ok(u32::from_le_bytes(b))
}

/// lsm_list_modules(ids, size, flags): flags must be 0; `*size` becomes
/// the bytes the list takes (`E2BIG` without writing `ids` if the buffer
/// is smaller); returns the number of modules.
pub fn lsm_list_modules(k: &mut dyn LinuxKernel, a: &Args) -> SysResult {
    if a.uint(2) != 0 {
        return Err(Errno::EINVAL);
    }
    let size_ptr = a.ptr(1);
    let size = read_u32(k, size_ptr)?;
    let list: Vec<u8> = MODULES.iter().flat_map(|id| id.to_le_bytes()).collect();
    let need = list.len() as u32;
    k.copy_to_user(size_ptr, &need.to_le_bytes())?;
    if size < need {
        return Err(Errno::E2BIG);
    }
    k.copy_to_user(a.ptr(0), &list)?;
    Ok(MODULES.len() as u64)
}

/// lsm_get_self_attr(attr, ctx, size, flags): checks flags, size and
/// attr, and with `LSM_FLAG_SINGLE` the module id in `ctx`; no module
/// provides an attribute, so then `*size` = 0 and `EOPNOTSUPP`.
pub fn lsm_get_self_attr(k: &mut dyn LinuxKernel, a: &Args) -> SysResult {
    let (attr, ctx, size_ptr, flags) = (a.uint(0), a.ptr(1), a.ptr(2), a.uint(3));
    if flags & !LSM_FLAG_SINGLE != 0 || size_ptr.is_null() {
        return Err(Errno::EINVAL);
    }
    read_u32(k, size_ptr)?;
    if attr == LSM_ATTR_UNDEF {
        return Err(Errno::EINVAL);
    }
    if flags & LSM_FLAG_SINGLE != 0 {
        if ctx.is_null() {
            return Err(Errno::EINVAL);
        }
        let mut id = [0u8; 8];
        k.copy_from_user(ctx, &mut id)?;
        if u64::from_le_bytes(id) == 0 {
            return Err(Errno::EINVAL);
        }
    }
    k.copy_to_user(size_ptr, &0u32.to_le_bytes())?;
    Err(Errno::EOPNOTSUPP)
}

/// lsm_set_self_attr(attr, ctx, size, flags): checks flags, size and the
/// `struct lsm_ctx` header; no module sets attributes, so a well-formed
/// request is `EOPNOTSUPP`.
pub fn lsm_set_self_attr(k: &mut dyn LinuxKernel, a: &Args) -> SysResult {
    let (ctx, size, flags) = (a.ptr(1), a.uint(2), a.uint(3));
    if flags != 0 || u64::from(size) <= LSM_CTX_HEADER {
        return Err(Errno::EINVAL);
    }
    if size > SET_ATTR_MAX {
        return Err(Errno::E2BIG);
    }
    let mut buf = vec![0u8; size as usize];
    k.copy_from_user(ctx, &mut buf)?;
    let field = |at: usize| u64::from_le_bytes(buf[at..at + 8].try_into().unwrap_or([0; 8]));
    let (len, ctx_len) = (field(16), field(24));
    if len > u64::from(size) || ctx_len.checked_add(LSM_CTX_HEADER).is_none_or(|least| len < least) {
        return Err(Errno::EINVAL);
    }
    Err(Errno::EOPNOTSUPP)
}

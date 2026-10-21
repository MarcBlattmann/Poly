//! prctl(2) `PR_SET_CHILD_SUBREAPER` / `PR_GET_CHILD_SUBREAPER`: the
//! "child subreaper" mark of the calling process, which makes it adopt the
//! orphans among its descendants instead of init (the rule is the process
//! tree's: [`crate::task::tree::ProcessTree::set_subreaper`]).

use crate::errno::SysResult;
use crate::kernel::{LinuxKernel, UserPtr};
use crate::task::with_tree;

/// Options (UAPI `linux/prctl.h`).
pub const PR_SET_CHILD_SUBREAPER: u32 = 36;
pub const PR_GET_CHILD_SUBREAPER: u32 = 37;

/// `PR_SET_CHILD_SUBREAPER`: "If arg2 is nonzero, set the "child
/// subreaper" attribute of the calling process; if arg2 is zero, unset the
/// attribute."
pub fn set(k: &mut dyn LinuxKernel, arg2: u64) -> SysResult {
    let me = k.pid();
    with_tree(k, |t| t.set_subreaper(me, arg2 != 0))?;
    Ok(0)
}

/// `PR_GET_CHILD_SUBREAPER`: "Return the "child subreaper" setting of the
/// caller, in the location pointed to by (int *) arg2" (`EFAULT`).
pub fn get(k: &mut dyn LinuxKernel, at: UserPtr) -> SysResult {
    let me = k.pid();
    let on = with_tree(k, |t| t.is_subreaper(me))?;
    k.copy_to_user(at, &i32::from(on).to_le_bytes())?;
    Ok(0)
}

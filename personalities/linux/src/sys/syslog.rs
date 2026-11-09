//! syslog(2) (klogctl): the kernel log buffer ([`crate::klog`], ADR 0135).
//!
//! With `kernel.dmesg_restrict` 0 (Poly's setting), reading the whole
//! buffer (`READ_ALL`) and asking its size (`SIZE_BUFFER`) are open to
//! everyone and every other action, unknown ones included, needs
//! `CAP_SYSLOG` (or `CAP_SYS_ADMIN`), checked before any argument.
//! `READ` consumes what is unread; Poly cannot wait for new text, so with
//! none it fails with `EAGAIN`.

use crate::args::Args;
use crate::cred::{self, cap};
use crate::errno::{Errno, SysResult};
use crate::kernel::LinuxKernel;
use crate::klog::{self, BUFFER_SIZE};
use crate::table::{Handler, nr};

/// The calls, registered in [`crate::table::GROUPS`].
pub const CALLS: &[(u64, Handler)] = &[(nr::SYSLOG, syslog)];

// Actions (`linux/syslog.h`).
const CLOSE: i32 = 0;
const OPEN: i32 = 1;
const READ: i32 = 2;
const READ_ALL: i32 = 3;
const READ_CLEAR: i32 = 4;
const CLEAR: i32 = 5;
const CONSOLE_OFF: i32 = 6;
const CONSOLE_ON: i32 = 7;
const CONSOLE_LEVEL: i32 = 8;
const SIZE_UNREAD: i32 = 9;
const SIZE_BUFFER: i32 = 10;

/// syslog(type, buf, len)
pub fn syslog(k: &mut dyn LinuxKernel, a: &Args) -> SysResult {
    let action = a.int(0);
    let creds = cred::current(k);
    let privileged = creds.capable_initial(cap::SYSLOG) || creds.capable_initial(cap::SYS_ADMIN);
    if !privileged && !matches!(action, READ_ALL | SIZE_BUFFER) {
        return Err(Errno::EPERM);
    }
    let len = a.int(2);
    match action {
        CLOSE | OPEN => Ok(0),
        READ | READ_ALL | READ_CLEAR => {
            if a.ptr(1).is_null() || len < 0 {
                return Err(Errno::EINVAL);
            }
            if len == 0 {
                return Ok(0);
            }
            let len = len as usize;
            let data = match action {
                READ => klog::with(|l| l.read(len)),
                _ => klog::with(|l| l.read_all(len)),
            };
            if data.is_empty() && action == READ {
                return Err(Errno::EAGAIN);
            }
            if !data.is_empty() {
                k.copy_to_user(a.ptr(1), &data)?;
            }
            if action == READ_CLEAR {
                klog::with(|l| l.clear());
            }
            Ok(data.len() as u64)
        }
        CLEAR => {
            klog::with(|l| l.clear());
            Ok(0)
        }
        CONSOLE_OFF => {
            klog::with(|l| l.console_off());
            Ok(0)
        }
        CONSOLE_ON => {
            klog::with(|l| l.console_on());
            Ok(0)
        }
        CONSOLE_LEVEL => {
            if !(1..=8).contains(&len) {
                return Err(Errno::EINVAL);
            }
            klog::with(|l| l.set_console_level(len as u8));
            Ok(0)
        }
        SIZE_UNREAD => Ok(klog::with(|l| l.unread()) as u64),
        SIZE_BUFFER => Ok(BUFFER_SIZE as u64),
        _ => Err(Errno::EINVAL),
    }
}

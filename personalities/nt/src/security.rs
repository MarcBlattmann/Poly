//! Access tokens (ADR 0083): the services our `kernelbase` needs for
//! `OpenProcessToken`, `OpenThreadToken`, `GetTokenInformation` and
//! (ADR 0114) `AdjustTokenPrivileges`.
//!
//! Poly has one user, so every process's primary token starts with the
//! same contents, [`poly_security::token::Token::poly_user`]. Each open
//! makes a new `Token` object (its cookie is the process id) that lives as
//! long as its handles; what `NtAdjustPrivilegesToken` changes (which
//! privileges are enabled, which removed) is kept per process
//! ([`NtSystem::token_privileges`]), so every handle to the process's
//! token sees it, until the process ends. Threads never impersonate, so a
//! thread has no token of its own.
//!
//! Arguments in the order of the documented `Zw*` routines (Learn, WDK):
//!
//! - `NtOpenProcessToken(ProcessHandle, DesiredAccess, TokenHandle*)`
//! - `NtOpenProcessTokenEx(ProcessHandle, DesiredAccess, HandleAttributes,
//!   TokenHandle*)`
//! - `NtOpenThreadToken(ThreadHandle, DesiredAccess, OpenAsSelf,
//!   TokenHandle*)`
//! - `NtOpenThreadTokenEx(ThreadHandle, DesiredAccess, OpenAsSelf,
//!   HandleAttributes, TokenHandle*)`: `STATUS_NO_TOKEN` once the thread
//!   handle checks out (Learn: "ZwOpenThreadTokenEx": the thread is not
//!   impersonating).
//! - `NtQueryInformationToken(TokenHandle, TokenInformationClass,
//!   TokenInformation, TokenInformationLength, ReturnLength*)`: needs
//!   `TOKEN_QUERY` (`TOKEN_QUERY_SOURCE` for `TokenSource`). The
//!   pseudo-handles of `processthreadsapi.h` work without a handle:
//!   `GetCurrentProcessToken()` (-4) and `GetCurrentThreadEffectiveToken()`
//!   (-6) are the process token; `GetCurrentThreadToken()` (-5) is
//!   `STATUS_NO_TOKEN`.
//!
//! The process handle needs `PROCESS_QUERY_LIMITED_INFORMATION` (Learn:
//! "OpenProcessToken"), the thread handle `THREAD_QUERY_LIMITED_INFORMATION`
//! (to be confirmed by the spec agent).

use crate::abi;
use crate::kernel::NtKernel;
use crate::objects::return_handle;
use crate::system::NtSystem;
use crate::threads::{process_of, thread_of};
use alloc::vec::Vec;
use poly_abi::SyscallRegs;
use poly_ntobj::access::{
    PROCESS_QUERY_LIMITED_INFORMATION, THREAD_QUERY_LIMITED_INFORMATION, TOKEN_QUERY, TOKEN_QUERY_SOURCE,
};
use poly_ntobj::status::{STATUS_ACCESS_DENIED, STATUS_BUFFER_TOO_SMALL, STATUS_INVALID_PARAMETER, STATUS_SUCCESS};
use poly_ntobj::{Handle, NewObject, NtStatus, ObjectType, ProcessId};
use poly_security::token::{self, STATUS_NO_TOKEN, TOKEN_ADJUST_PRIVILEGES, Token, class};

type Result<T> = core::result::Result<T, NtStatus>;

fn open_process_token<K: NtKernel>(
    k: &mut K,
    process: u64,
    access: u64,
    attributes: u64,
    out: u64,
) -> Result<NtStatus> {
    let pid = process_of(k, Handle(process), PROCESS_QUERY_LIMITED_INFORMATION)?;
    let caller = k.caller();
    let h = k.with_state(|sys, _, b| {
        let new = NewObject::Opaque { kind: ObjectType::Token, cookie: u64::from(pid.0), signaled: None, parse: false };
        let id = sys.objects.new_object(new, caller.thread)?;
        let h = sys.objects.open_by_pointer(&caller, id, access as u32, attributes as u32);
        // The handle (if any) holds the object now.
        let _ = sys.objects.dereference(id);
        sys.reap(b);
        h
    })?;
    return_handle(k, out, h)?;
    Ok(STATUS_SUCCESS)
}

/// `NtOpenProcessToken`.
pub fn open_process_token_service<K: NtKernel>(k: &mut K, regs: &SyscallRegs) -> Result<NtStatus> {
    let [process, access, out] = abi::args::<K, 3>(k, regs)?;
    open_process_token(k, process, access, 0, out)
}

/// `NtOpenProcessTokenEx`.
pub fn open_process_token_ex<K: NtKernel>(k: &mut K, regs: &SyscallRegs) -> Result<NtStatus> {
    let [process, access, attributes, out] = abi::args::<K, 4>(k, regs)?;
    open_process_token(k, process, access, attributes, out)
}

/// `NtOpenThreadToken`, `NtOpenThreadTokenEx`: no thread impersonates.
pub fn open_thread_token<K: NtKernel>(k: &mut K, regs: &SyscallRegs) -> Result<NtStatus> {
    let thread = abi::arg(k, regs, 0)?;
    thread_of(k, Handle(thread), THREAD_QUERY_LIMITED_INFORMATION)?;
    Err(STATUS_NO_TOKEN)
}

/// The process whose primary token `h` is, if the handle grants `access`.
/// The pseudo-handles `GetCurrentProcessToken()` and
/// `GetCurrentThreadEffectiveToken()` are the caller's, with
/// `TOKEN_QUERY` and `TOKEN_QUERY_SOURCE` only (Learn,
/// "GetCurrentProcessToken").
fn token_process<K: NtKernel>(k: &mut K, h: i64, access: u32) -> Result<ProcessId> {
    let caller = k.caller();
    if h == token::CURRENT_THREAD_TOKEN {
        return Err(STATUS_NO_TOKEN);
    }
    if h == token::CURRENT_PROCESS_TOKEN || h == token::CURRENT_THREAD_EFFECTIVE_TOKEN {
        return if access & !(TOKEN_QUERY | TOKEN_QUERY_SOURCE) == 0 {
            Ok(caller.process)
        } else {
            Err(STATUS_ACCESS_DENIED)
        };
    }
    k.with_state(|sys, _, _| {
        let id = sys.objects.object_by_handle(&caller, Handle(h as u64), access, Some(ObjectType::Token))?;
        Ok(ProcessId(sys.objects.cookie(id).unwrap_or(0) as u32))
    })
}

/// The privileges of `pid`'s primary token.
fn privileges_of(sys: &NtSystem, pid: ProcessId) -> Vec<(u32, u32)> {
    sys.token_privileges.get(&pid).cloned().unwrap_or_else(|| Token::poly_user().privileges)
}

/// `NtQueryInformationToken`.
pub fn query_information_token<K: NtKernel>(k: &mut K, regs: &SyscallRegs) -> Result<NtStatus> {
    let [h, info_class, buf, length, ret] = abi::args::<K, 5>(k, regs)?;
    let info_class = info_class as u32;
    let need = if info_class == class::TOKEN_SOURCE { TOKEN_QUERY_SOURCE } else { TOKEN_QUERY };
    let pid = token_process(k, h as i64, need)?;
    let mut t = Token::poly_user();
    t.privileges = k.with_state(|sys, _, _| privileges_of(sys, pid));
    let bytes = t.query(info_class, buf)?;
    abi::write_opt_u32(k, ret, bytes.len() as u32)?;
    let (status, write) = token::fit(&bytes, length as u32 as usize);
    if let Some(w) = write {
        abi::write(k, buf, w)?;
    }
    Ok(status)
}

/// The most privileges a `NewState` may name: each of Poly's privileges
/// a few times over (a bound on what is read from the caller).
const MAX_NEW_STATE: usize = 1024;

/// `NtAdjustPrivilegesToken` (`AdjustTokenPrivileges`, Learn): needs
/// `TOKEN_ADJUST_PRIVILEGES`, and `TOKEN_QUERY` for a `PreviousState`.
/// A `PreviousState` too small for what would change is
/// `STATUS_BUFFER_TOO_SMALL` with the size needed in `ReturnLength` and
/// nothing adjusted ("the function fails and does not adjust any
/// privileges"); a privilege the token does not hold makes the result
/// `STATUS_NOT_ALL_ASSIGNED` (a success). The state is the process's, so
/// every handle to its token sees it (ADR 0114).
pub fn adjust_privileges_token<K: NtKernel>(k: &mut K, regs: &SyscallRegs) -> Result<NtStatus> {
    let [h, disable_all, new_state, buffer_length, previous, ret] = abi::args::<K, 6>(k, regs)?;
    let disable_all = disable_all as u32 != 0;
    let access = if previous != 0 { TOKEN_ADJUST_PRIVILEGES | TOKEN_QUERY } else { TOKEN_ADJUST_PRIVILEGES };
    let pid = token_process(k, h as i64, access)?;
    let new = if disable_all {
        Vec::new()
    } else {
        if new_state == 0 {
            return Err(STATUS_INVALID_PARAMETER);
        }
        let n = abi::read_u32(k, new_state)? as usize;
        if n > MAX_NEW_STATE {
            return Err(STATUS_INVALID_PARAMETER);
        }
        let mut b = alloc::vec![0u8; 4 + 12 * n];
        abi::read(k, new_state, &mut b)?;
        token::parse_token_privileges(&b).ok_or(STATUS_INVALID_PARAMETER)?
    };
    let current = k.with_state(|sys, _, _| privileges_of(sys, pid));
    let a = token::adjust_privileges(&current, disable_all, &new);
    let before = token::token_privileges(&a.previous);
    abi::write_opt_u32(k, ret, before.len() as u32)?;
    if previous != 0 {
        if (buffer_length as u32 as usize) < before.len() {
            return Err(STATUS_BUFFER_TOO_SMALL);
        }
        abi::write(k, previous, &before)?;
    }
    k.with_state(|sys, _, _| sys.token_privileges.insert(pid, a.privileges));
    Ok(if a.all_assigned { STATUS_SUCCESS } else { token::STATUS_NOT_ALL_ASSIGNED })
}

#[cfg(test)]
mod tests {
    use crate::services::Service;
    use crate::tests::{Fake, SELF};
    use poly_ntobj::NtStatus;
    use poly_ntobj::status::*;
    use poly_security::Sid;
    use poly_security::sddl;
    use poly_security::token::STATUS_NO_TOKEN;

    const TOKEN_QUERY: u64 = 8;

    fn open(f: &mut Fake, access: u64) -> u64 {
        let out = f.alloc(8);
        assert_eq!(f.call(Service::OpenProcessToken, &[SELF, access, out]), STATUS_SUCCESS);
        f.u64_at(out)
    }

    #[test]
    fn open_query_close() {
        let mut f = Fake::new();
        let h = open(&mut f, TOKEN_QUERY);
        let (buf, ret) = (f.alloc(256), f.alloc(4));
        assert_eq!(f.call(Service::QueryInformationToken, &[h, 1, buf, 256, ret]), STATUS_SUCCESS);
        assert_eq!(f.u32_at(ret), 44);
        // The SID pointer points into the caller's buffer.
        assert_eq!(f.u64_at(buf), buf + 16);
        let sid = Sid::from_bytes(&f.bytes_at(buf + 16, 28)).unwrap();
        assert_eq!(sddl::to_string(&sid), "S-1-5-21-0-0-0-1000");
        // Too small: nothing written, the size reported.
        let small = f.alloc(8);
        assert_eq!(f.call(Service::QueryInformationToken, &[h, 1, small, 8, ret]), STATUS_BUFFER_TOO_SMALL);
        assert_eq!(f.u32_at(ret), 44);
        assert_eq!(f.u64_at(small), 0);
        // TokenElevation through the pseudo-handle.
        let e = f.alloc(4);
        assert_eq!(f.call(Service::QueryInformationToken, &[(-4i64) as u64, 20, e, 4, ret]), STATUS_SUCCESS);
        assert_eq!(f.u32_at(e), 1);
        assert_eq!(f.call(Service::QueryInformationToken, &[(-5i64) as u64, 20, e, 4, ret]), STATUS_NO_TOKEN);
        assert_eq!(f.call(Service::QueryInformationToken, &[h, 999, e, 4, ret]), STATUS_INVALID_INFO_CLASS);
        assert_eq!(f.call(Service::Close, &[h]), STATUS_SUCCESS);
        assert_eq!(f.call(Service::QueryInformationToken, &[h, 20, e, 4, ret]), STATUS_INVALID_HANDLE);
    }

    #[test]
    fn access_is_checked() {
        let mut f = Fake::new();
        // TOKEN_ADJUST_DEFAULT only: no query.
        let h = open(&mut f, 0x80);
        let (buf, ret) = (f.alloc(64), f.alloc(4));
        assert_eq!(f.call(Service::QueryInformationToken, &[h, 20, buf, 4, ret]), STATUS_ACCESS_DENIED);
        // TokenSource needs TOKEN_QUERY_SOURCE, not TOKEN_QUERY.
        let q = open(&mut f, TOKEN_QUERY);
        assert_eq!(f.call(Service::QueryInformationToken, &[q, 7, buf, 64, ret]), STATUS_ACCESS_DENIED);
        let s = open(&mut f, 0x10);
        assert_eq!(f.call(Service::QueryInformationToken, &[s, 7, buf, 64, ret]), STATUS_SUCCESS);
        // A handle of another type.
        let ev = f.alloc(8);
        assert_eq!(f.call(Service::CreateEvent, &[ev, 0x1F_0003, 0, 0, 0]), STATUS_SUCCESS);
        let ev = f.u64_at(ev);
        assert_eq!(f.call(Service::QueryInformationToken, &[ev, 20, buf, 4, ret]), STATUS_OBJECT_TYPE_MISMATCH);
        let out = f.alloc(8);
        assert_eq!(f.call(Service::OpenProcessToken, &[ev, TOKEN_QUERY, out]), STATUS_OBJECT_TYPE_MISMATCH);
        // The Ex form takes handle attributes; a bad one is refused.
        assert_eq!(f.call(Service::OpenProcessTokenEx, &[SELF, TOKEN_QUERY, 2, out]), STATUS_SUCCESS);
        assert_eq!(
            f.call(Service::OpenProcessTokenEx, &[SELF, TOKEN_QUERY, 0x8000_0000, out]),
            STATUS_INVALID_PARAMETER
        );
        // GENERIC_READ maps to TOKEN_READ, which holds TOKEN_QUERY.
        let g = open(&mut f, 0x8000_0000);
        assert_eq!(f.call(Service::QueryInformationToken, &[g, 20, buf, 4, ret]), STATUS_SUCCESS);
    }

    #[test]
    fn threads_have_no_token() {
        let mut f = Fake::new();
        let out = f.alloc(8);
        let current_thread = (-2i64) as u64;
        assert_eq!(f.call(Service::OpenThreadToken, &[current_thread, TOKEN_QUERY, 0, out]), STATUS_NO_TOKEN);
        assert_eq!(f.call(Service::OpenThreadTokenEx, &[current_thread, TOKEN_QUERY, 1, 0, out]), STATUS_NO_TOKEN);
        assert_eq!(f.call(Service::OpenThreadToken, &[0x1234, TOKEN_QUERY, 0, out]), STATUS_INVALID_HANDLE);
    }

    /// The `TOKEN_PRIVILEGES` attributes of `luid` through `h`.
    fn privilege(f: &mut Fake, h: u64, luid: u32) -> Option<u32> {
        let (buf, ret) = (f.alloc(1024), f.alloc(4));
        assert_eq!(f.call(Service::QueryInformationToken, &[h, 3, buf, 1024, ret]), STATUS_SUCCESS);
        let n = f.u32_at(buf) as u64;
        (0..n).map(|i| buf + 4 + 12 * i).find(|&at| f.u32_at(at) == luid).map(|at| f.u32_at(at + 8))
    }

    #[test]
    fn adjusting_privileges() {
        const ADJUST: u64 = 0x20;
        const RESTORE: u32 = 18;
        const LOCK_MEMORY: u32 = 4;
        let mut f = Fake::new();
        let h = open(&mut f, ADJUST);
        let q = open(&mut f, TOKEN_QUERY);
        assert_eq!(privilege(&mut f, q, RESTORE), Some(0));
        // Enable SeRestorePrivilege: no PreviousState asked for.
        let new = f.put(&[1, 0, 0, 0, RESTORE as u8, 0, 0, 0, 0, 0, 0, 0, 2, 0, 0, 0]);
        assert_eq!(f.call(Service::AdjustPrivilegesToken, &[h, 0, new, 0, 0, 0]), STATUS_SUCCESS);
        // Every handle (and the pseudo-handle) sees it.
        assert_eq!(privilege(&mut f, q, RESTORE), Some(2));
        assert_eq!(privilege(&mut f, (-4i64) as u64, RESTORE), Some(2));
        // PreviousState needs TOKEN_QUERY too; the pseudo-handle cannot adjust.
        let (prev, ret) = (f.alloc(64), f.alloc(4));
        assert_eq!(f.call(Service::AdjustPrivilegesToken, &[h, 1, 0, 64, prev, ret]), STATUS_ACCESS_DENIED);
        assert_eq!(f.call(Service::AdjustPrivilegesToken, &[(-4i64) as u64, 0, new, 0, 0, 0]), STATUS_ACCESS_DENIED);
        // Disable all: the previous state lists what was enabled; too small
        // a buffer changes nothing and reports the size.
        let hq = open(&mut f, ADJUST | TOKEN_QUERY);
        assert_eq!(f.call(Service::AdjustPrivilegesToken, &[hq, 1, 0, 4, prev, ret]), STATUS_BUFFER_TOO_SMALL);
        let need = f.u32_at(ret);
        assert_eq!(need, 4 + 12 * 4, "SeRestore and the three enabled by default");
        assert_eq!(privilege(&mut f, q, RESTORE), Some(2));
        assert_eq!(f.call(Service::AdjustPrivilegesToken, &[hq, 1, 0, 64, prev, ret]), STATUS_SUCCESS);
        assert_eq!(f.u32_at(prev), 4);
        assert_eq!(privilege(&mut f, q, RESTORE), Some(0));
        assert_eq!(privilege(&mut f, q, 23), Some(1), "SeChangeNotify: enabled by default, now disabled");
        // A privilege the token does not hold: success, not all assigned.
        let lock = f.put(&[1, 0, 0, 0, LOCK_MEMORY as u8, 0, 0, 0, 0, 0, 0, 0, 2, 0, 0, 0]);
        assert_eq!(f.call(Service::AdjustPrivilegesToken, &[h, 0, lock, 0, 0, 0]), NtStatus(0x106));
        assert_eq!(privilege(&mut f, q, LOCK_MEMORY), None);
        assert_eq!(f.call(Service::AdjustPrivilegesToken, &[h, 0, 0, 0, 0, 0]), STATUS_INVALID_PARAMETER);
    }
}

//! Access tokens (ADR 0083): the services our `kernelbase` needs for
//! `OpenProcessToken`, `OpenThreadToken` and `GetTokenInformation`.
//!
//! Poly has one user, so every process's primary token has the same
//! contents, [`poly_security::token::Token::poly_user`]. Each open makes a
//! new `Token` object (its cookie is the process id) that lives as long as
//! its handles; nothing is kept in it yet, since no service changes a
//! token (`NtAdjustPrivilegesToken` is a later item). Threads never
//! impersonate, so a thread has no token of its own.
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
use crate::threads::{process_of, thread_of};
use poly_abi::SyscallRegs;
use poly_ntobj::access::{
    PROCESS_QUERY_LIMITED_INFORMATION, THREAD_QUERY_LIMITED_INFORMATION, TOKEN_QUERY, TOKEN_QUERY_SOURCE,
};
use poly_ntobj::status::STATUS_SUCCESS;
use poly_ntobj::{Handle, NewObject, NtStatus, ObjectType};
use poly_security::token::{self, STATUS_NO_TOKEN, Token, class};

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

/// `NtQueryInformationToken`.
pub fn query_information_token<K: NtKernel>(k: &mut K, regs: &SyscallRegs) -> Result<NtStatus> {
    let [h, info_class, buf, length, ret] = abi::args::<K, 5>(k, regs)?;
    let info_class = info_class as u32;
    let h = h as i64;
    if h == token::CURRENT_THREAD_TOKEN {
        return Err(STATUS_NO_TOKEN);
    }
    if h != token::CURRENT_PROCESS_TOKEN && h != token::CURRENT_THREAD_EFFECTIVE_TOKEN {
        let need = if info_class == class::TOKEN_SOURCE { TOKEN_QUERY_SOURCE } else { TOKEN_QUERY };
        let caller = k.caller();
        k.with_state(|sys, _, _| {
            sys.objects.object_by_handle(&caller, Handle(h as u64), need, Some(ObjectType::Token))
        })?;
    }
    let bytes = Token::poly_user().query(info_class, buf)?;
    abi::write_opt_u32(k, ret, bytes.len() as u32)?;
    let (status, write) = token::fit(&bytes, length as u32 as usize);
    if let Some(w) = write {
        abi::write(k, buf, w)?;
    }
    Ok(status)
}

#[cfg(test)]
mod tests {
    use crate::services::Service;
    use crate::tests::{Fake, SELF};
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
}

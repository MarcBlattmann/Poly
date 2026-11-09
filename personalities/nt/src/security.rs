//! Access tokens and object security (ADR 0083, 0114, 0171): the services
//! behind `OpenProcessToken`, `OpenThreadToken`, `GetTokenInformation`,
//! `AdjustTokenPrivileges`/`Groups`, `DuplicateTokenEx`,
//! `CreateRestrictedToken`, `SetTokenInformation`, `AccessCheck`,
//! impersonation on threads and the security descriptors of objects.
//!
//! The model is `poly-ntobj`'s [`poly_ntobj::ObjectSecurity`]: every
//! process has a primary token (a copy of its creator's), a thread may have
//! an impersonation token, and a request is checked with the thread's token,
//! else the process's. A token is an object of type `Token`; its contents
//! are [`poly_security::token::Token`]. The first process has
//! [`Token::poly_user`], an elevated administrator's.
//!
//! Arguments in the order of the documented `Zw*` routines (Learn, WDK) or,
//! where Learn has no page, of the Win32 function that calls them.

use crate::abi;
use crate::kernel::NtKernel;
use crate::objects::return_handle;
use crate::system::NtSystem;
use crate::threads::{process_of, thread_of};
use alloc::vec::Vec;
use poly_abi::SyscallRegs;
use poly_ntobj::access::*;
use poly_ntobj::status::*;
use poly_ntobj::{Caller, Handle, NtStatus, OBJ_INHERIT, OBJ_KERNEL_HANDLE, ObjectType, ProcessId, ThreadRef};
use poly_security::access::{self as sacc, CheckToken};
use poly_security::err::SecErr;
use poly_security::sd::{self, Parts};
use poly_security::sdops;
use poly_security::sid::Sid;
use poly_security::token::{self, SetData, Token, class};

type Result<T> = core::result::Result<T, NtStatus>;

/// Every right a token handle may hold.
const TOKEN_VALID: u32 = token::TOKEN_ALL_ACCESS | ACCESS_SYSTEM_SECURITY;
const STATUS_BAD_TOKEN_TYPE: NtStatus = NtStatus(0xC000_00A8);
const STATUS_CANT_OPEN_ANONYMOUS: NtStatus = NtStatus(0xC000_00A6);
const STATUS_NO_IMPERSONATION_TOKEN: NtStatus = NtStatus(0xC000_005C);
const STATUS_NO_SECURITY_ON_OBJECT: NtStatus = NtStatus(0xC000_00D7);
const STATUS_GENERIC_NOT_MAPPED: NtStatus = NtStatus(0xC000_00E6);
const STATUS_INVALID_SECURITY_DESCR: NtStatus = NtStatus(0xC000_0079);
const STATUS_INVALID_OWNER: NtStatus = NtStatus(0xC000_005A);

// ----- the token store ---------------------------------------------------------------------

/// Make `token` the primary token of `pid` (a new token object under a new
/// id; its descriptor is the default of `creator`'s token, else of itself).
pub fn install_process_token(
    sys: &mut NtSystem,
    pid: ProcessId,
    token: Token,
    thread: ThreadRef,
    creator: Option<u64>,
) -> Result<u64> {
    let (id, obj) = sys.objects.new_token(token, thread)?;
    sys.objects.assign_security(obj, ObjectType::Token, Some(creator.unwrap_or(id)), None);
    let r = sys.objects.set_process_token(pid, id);
    // The process's reference replaces the creation one.
    let _ = sys.objects.dereference(obj);
    r.map(|()| id)
}

/// `pid`'s primary token; a process that has none yet (the first one) gets
/// the single user's.
pub fn process_token(sys: &mut NtSystem, pid: ProcessId, thread: ThreadRef) -> Result<u64> {
    if let Some(id) = sys.objects.security.process_token(pid) {
        return Ok(id);
    }
    install_process_token(sys, pid, Token::poly_user(), thread, None)
}

/// The token of a process being created: a copy of `creator`'s (or of
/// `given`, a primary token), so that what either does later does not touch
/// the other.
pub fn inherit_token(
    sys: &mut NtSystem,
    child: ProcessId,
    creator: Option<ProcessId>,
    given: Option<u64>,
    thread: ThreadRef,
) -> Result<()> {
    let base = match (given, creator) {
        (Some(id), _) => id,
        (None, Some(c)) => process_token(sys, c, thread)?,
        (None, None) => {
            install_process_token(sys, child, Token::poly_user(), thread, None)?;
            return Ok(());
        }
    };
    let id = sys.objects.security.new_luid();
    let source = sys.objects.security.token(base).ok_or(STATUS_INVALID_PARAMETER)?;
    let level = source.level;
    let mut copy = source.duplicate(token::TOKEN_PRIMARY, token::SECURITY_IMPERSONATION, id);
    // A process's primary token keeps the source's (primary) level field.
    copy.level = level;
    if given.is_some() {
        copy.clamp_privileges_to_integrity();
    }
    let creator_token = creator.and_then(|c| sys.objects.security.process_token(c));
    install_process_token(sys, child, copy, thread, creator_token).map(|_| ())
}

/// Give the objects of a new process their descriptors (the default of its
/// creator's token): the process object and its first thread.
pub fn assign_process_security(
    sys: &mut NtSystem,
    child: ProcessId,
    creator: Option<&Caller>,
    process: Option<poly_ntobj::ObjectId>,
    thread: Option<poly_ntobj::ObjectId>,
) {
    // The first process (no creator) is its own creator.
    let token = creator
        .and_then(|c| sys.objects.security.effective(c.process, c.thread))
        .or_else(|| sys.objects.security.process_token(child));
    if let Some(p) = process {
        sys.objects.assign_security(p, ObjectType::Process, token, None);
    }
    if let Some(t) = thread {
        sys.objects.assign_security(t, ObjectType::Thread, token, None);
    }
}

/// The token a request by `caller` is checked with, as a copy.
fn effective(sys: &mut NtSystem, caller: &Caller) -> Result<(u64, Token)> {
    let id = match sys.objects.security.effective(caller.process, caller.thread) {
        Some(id) => id,
        None => process_token(sys, caller.process, caller.thread)?,
    };
    Ok((id, sys.objects.security.token(id).ok_or(STATUS_INVALID_HANDLE)?.clone()))
}

/// The id of the token named by `h` (a handle or one of the
/// pseudo-handles), checked for `access`.
fn token_of_in(sys: &mut NtSystem, caller: &Caller, h: u64, access: u32) -> Result<u64> {
    let hi = h as i64;
    let quick = access & !(token::TOKEN_QUERY | token::TOKEN_QUERY_SOURCE) == 0;
    match hi {
        token::CURRENT_THREAD_TOKEN => sys.objects.security.thread_token(caller.thread).ok_or(token::STATUS_NO_TOKEN),
        token::CURRENT_PROCESS_TOKEN | token::CURRENT_THREAD_EFFECTIVE_TOKEN => {
            if !quick {
                return Err(STATUS_ACCESS_DENIED);
            }
            if hi == token::CURRENT_PROCESS_TOKEN {
                process_token(sys, caller.process, caller.thread)
            } else {
                effective(sys, caller).map(|(id, _)| id)
            }
        }
        _ => {
            let obj = sys.objects.object_by_handle(caller, Handle(h), access, Some(ObjectType::Token))?;
            Ok(sys.objects.cookie(obj).unwrap_or(0))
        }
    }
}

pub(crate) fn token_of<K: NtKernel>(k: &mut K, h: u64, access: u32) -> Result<u64> {
    let caller = k.caller();
    k.with_state(|sys, _, _| token_of_in(sys, &caller, h, access))
}

/// A copy of token `id`.
fn copy_of(k: &mut impl NtKernel, id: u64) -> Result<Token> {
    k.with_state(|sys, _, _| sys.objects.security.token(id).cloned().ok_or(STATUS_INVALID_HANDLE))
}

/// A new handle (with exactly `granted` access) to token object `obj`.
fn open_exact(
    sys: &mut NtSystem,
    caller: &Caller,
    obj: poly_ntobj::ObjectId,
    granted: u32,
    attributes: u32,
) -> Result<Handle> {
    let kernel = Caller::kernel(caller.process, caller.thread);
    sys.objects.open_by_pointer(&kernel, obj, granted, attributes & !OBJ_KERNEL_HANDLE & OBJ_INHERIT)
}

// ----- opening -----------------------------------------------------------------------------

/// The handle attributes of `NtOpenProcessToken(Ex)` / `NtOpenThreadToken(Ex)`:
/// `OBJ_EXCLUSIVE` is refused, every other bit but `OBJ_INHERIT` is ignored
/// (the sheets: 0x1, 0x4, 0x800, 0x10000 and 0x80000000 all give an ordinary
/// handle).
fn token_open_attributes(attributes: u64) -> Result<u32> {
    const OBJ_EXCLUSIVE: u32 = 0x20;
    let a = attributes as u32;
    if a & OBJ_EXCLUSIVE != 0 {
        return Err(STATUS_INVALID_PARAMETER);
    }
    Ok(a & OBJ_INHERIT)
}

fn open_process_token(k: &mut impl NtKernel, process: u64, access: u64, attributes: u64, out: u64) -> Result<NtStatus> {
    let attributes = u64::from(token_open_attributes(attributes)?);
    // `PROCESS_QUERY_LIMITED_INFORMATION`, or the full query right that
    // implies it (the sheets: a handle opened with `PROCESS_QUERY_INFORMATION`
    // opens the token).
    let pid = process_of(k, Handle(process), PROCESS_QUERY_LIMITED_INFORMATION).or_else(|e| {
        if e == STATUS_ACCESS_DENIED { process_of(k, Handle(process), PROCESS_QUERY_INFORMATION) } else { Err(e) }
    })?;
    let caller = k.caller();
    let h = k.with_state(|sys, _, b| {
        let id = process_token(sys, pid, caller.thread)?;
        let obj = sys.objects.security.token_object(id).ok_or(STATUS_INVALID_HANDLE)?;
        let h = sys.objects.open_by_pointer(&caller, obj, access as u32, attributes as u32);
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

/// `NtOpenThreadToken(Ex)(ThreadHandle, DesiredAccess, OpenAsSelf,
/// [HandleAttributes,] TokenHandle*)`: the thread's impersonation token.
/// A token of level Anonymous cannot be opened; with `OpenAsSelf` FALSE the
/// access check uses the calling thread's own impersonated identity, which
/// must be at least Impersonation level (Learn: "OpenThreadToken").
pub fn open_thread_token<K: NtKernel>(k: &mut K, regs: &SyscallRegs, ex: bool) -> Result<NtStatus> {
    let thread = abi::arg(k, regs, 0)?;
    let access = abi::arg(k, regs, 1)? as u32;
    let as_self = abi::arg(k, regs, 2)? as u8 != 0;
    let (attributes, out) =
        if ex { (abi::arg(k, regs, 3)?, abi::arg(k, regs, 4)?) } else { (0, abi::arg(k, regs, 3)?) };
    let attributes = u64::from(token_open_attributes(attributes)?);
    let caller = k.caller();
    // The thread handle needs a query right.
    let tid = {
        let need = |k: &mut K, a: u32| thread_of(k, Handle(thread), a);
        need(k, THREAD_QUERY_LIMITED_INFORMATION)
            .or_else(|e| if e == STATUS_ACCESS_DENIED { need(k, THREAD_QUERY_INFORMATION) } else { Err(e) })?
    };
    let h = k.with_state(|sys, _, b| {
        let id = sys.objects.security.thread_token(tid).ok_or(token::STATUS_NO_TOKEN)?;
        let tok = sys.objects.security.token(id).ok_or(token::STATUS_NO_TOKEN)?.clone();
        if tok.kind == token::TOKEN_IMPERSONATION && tok.level == token::SECURITY_ANONYMOUS {
            return Err(STATUS_CANT_OPEN_ANONYMOUS);
        }
        // The identity the access check runs as.
        let mut who = caller;
        if as_self {
            who.thread = ThreadRef(u64::MAX);
        } else if let Some(mine) =
            sys.objects.security.thread_token(caller.thread).and_then(|m| sys.objects.security.token(m))
        {
            if mine.level == token::SECURITY_ANONYMOUS {
                return Err(STATUS_CANT_OPEN_ANONYMOUS);
            }
            if mine.level < token::SECURITY_IMPERSONATION {
                return Err(STATUS_BAD_IMPERSONATION_LEVEL);
            }
        }
        let obj = sys.objects.security.token_object(id).ok_or(token::STATUS_NO_TOKEN)?;
        let h = sys.objects.open_by_pointer(&who, obj, access, attributes as u32);
        sys.reap(b);
        // The handle goes in the caller's table whichever identity checked it.
        h
    })?;
    return_handle(k, out, h)?;
    Ok(STATUS_SUCCESS)
}

const STATUS_BAD_IMPERSONATION_LEVEL: NtStatus = NtStatus(0xC000_00A5);

// ----- querying ----------------------------------------------------------------------------

/// `NtQueryInformationToken`.
pub fn query_information_token<K: NtKernel>(k: &mut K, regs: &SyscallRegs) -> Result<NtStatus> {
    let [h, info_class, buf, length, ret] = abi::args::<K, 5>(k, regs)?;
    let info_class = info_class as u32;
    let need = if info_class == class::TOKEN_SOURCE { token::TOKEN_QUERY_SOURCE } else { token::TOKEN_QUERY };
    let id = token_of(k, h, need)?;
    let length = length as u32 as usize;
    // TokenElevation and TokenLinkedToken insist on their exact size.
    let exact = match info_class {
        class::TOKEN_ELEVATION => Some(4),
        class::TOKEN_LINKED_TOKEN => Some(8),
        _ => None,
    };
    if let Some(n) = exact {
        if ret == 0 {
            return Err(STATUS_ACCESS_VIOLATION);
        }
        abi::write_u32(k, ret, n as u32)?;
        if length != n {
            return Err(token::STATUS_INFO_LENGTH_MISMATCH);
        }
        if info_class == class::TOKEN_LINKED_TOKEN {
            return Err(token::STATUS_NO_SUCH_LOGON_SESSION);
        }
    }
    let t = copy_of(k, id)?;
    let bytes = t.query(info_class, buf)?;
    if ret == 0 {
        return Err(STATUS_ACCESS_VIOLATION);
    }
    if bytes.len() > length {
        abi::write_u32(k, ret, bytes.len() as u32)?;
        return Err(STATUS_BUFFER_TOO_SMALL);
    }
    if buf == 0 {
        return Err(STATUS_ACCESS_VIOLATION);
    }
    abi::write(k, buf, &bytes)?;
    abi::write_u32(k, ret, bytes.len() as u32)?;
    Ok(STATUS_SUCCESS)
}

// ----- adjusting ---------------------------------------------------------------------------

/// The most entries a `NewState` may name: a bound on what is read from the
/// caller.
const MAX_NEW_STATE: usize = 1024;

/// `NtAdjustPrivilegesToken(TokenHandle, DisableAllPrivileges, NewState*,
/// BufferLength, PreviousState*, ReturnLength*)` (`AdjustTokenPrivileges`,
/// Learn): needs `TOKEN_ADJUST_PRIVILEGES`, and `TOKEN_QUERY` for a
/// `PreviousState`. A buffer too small for the privileges that would
/// change is `STATUS_BUFFER_TOO_SMALL` with the size in `ReturnLength` and
/// nothing adjusted; one the token does not hold makes the result
/// `STATUS_NOT_ALL_ASSIGNED` (a success).
pub fn adjust_privileges_token<K: NtKernel>(k: &mut K, regs: &SyscallRegs) -> Result<NtStatus> {
    let [h, disable_all, new_state, buffer_length, previous, ret] = abi::args::<K, 6>(k, regs)?;
    let disable_all = disable_all as u8 != 0;
    let access = if previous != 0 {
        token::TOKEN_ADJUST_PRIVILEGES | token::TOKEN_QUERY
    } else {
        token::TOKEN_ADJUST_PRIVILEGES
    };
    let id = token_of(k, h, access)?;
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
    let current = copy_of(k, id)?.privileges;
    let a = token::adjust_privileges(&current, disable_all, &new);
    let before = token::token_privileges(&a.previous);
    // One entry's room is always asked for (spec sheet).
    let needed = 4 + 12 * a.previous.len().max(1);
    if previous != 0 {
        abi::write_opt_u32(k, ret, needed as u32)?;
        if (buffer_length as u32 as usize) < needed {
            return Err(STATUS_BUFFER_TOO_SMALL);
        }
        abi::write(k, previous, &before)?;
    }
    k.with_state(|sys, _, _| {
        if let Some(t) = sys.objects.security.token_mut(id) {
            t.privileges = a.privileges;
        }
    });
    Ok(if a.all_assigned { STATUS_SUCCESS } else { token::STATUS_NOT_ALL_ASSIGNED })
}

/// Read a `TOKEN_GROUPS` from the caller's memory: each SID's bytes and
/// attributes (the SIDs are anywhere in the caller's memory).
fn read_groups<K: NtKernel>(k: &mut K, addr: u64) -> Result<Vec<(Vec<u8>, u32)>> {
    let n = abi::read_u32(k, addr)? as usize;
    if n > MAX_NEW_STATE {
        return Err(STATUS_INVALID_PARAMETER);
    }
    let mut out = Vec::with_capacity(n);
    for i in 0..n {
        let mut e = [0u8; 16];
        abi::read(k, addr + 8 + 16 * i as u64, &mut e)?;
        let ptr = u64::from_le_bytes(e[0..8].try_into().expect("8 bytes"));
        let attrs = u32::from_le_bytes(e[8..12].try_into().expect("4 bytes"));
        let mut head = [0u8; 8];
        abi::read(k, ptr, &mut head)?;
        let len = poly_security::sid::length_required(head[1]) as usize;
        let mut sid = alloc::vec![0u8; len];
        abi::read(k, ptr, &mut sid)?;
        out.push((sid, attrs));
    }
    Ok(out)
}

/// `NtAdjustGroupsToken(TokenHandle, ResetToDefault, NewState*,
/// BufferLength, PreviousState*, ReturnLength*)`.
pub fn adjust_groups_token<K: NtKernel>(k: &mut K, regs: &SyscallRegs) -> Result<NtStatus> {
    let [h, reset, new_state, buffer_length, previous, ret] = abi::args::<K, 6>(k, regs)?;
    let reset = reset as u8 != 0;
    let id = token_of(k, h, token::TOKEN_ADJUST_GROUPS)?;
    let new = if reset {
        Vec::new()
    } else {
        if new_state == 0 {
            return Err(STATUS_INVALID_PARAMETER);
        }
        read_groups(k, new_state)?
    };
    let mut t = copy_of(k, id)?;
    let (changed, all) = match t.adjust_groups(reset, &new) {
        Ok(r) => r,
        Err(e) => {
            // A refusal reports no length (the sheet: `*ReturnLength` 0).
            abi::write_opt_u32(k, ret, 0)?;
            return Err(e);
        }
    };
    // The previous state is a TOKEN_GROUPS laid out at the caller's buffer.
    let mut prev = Vec::new();
    prev.extend_from_slice(&(changed.len() as u32).to_le_bytes());
    prev.extend_from_slice(&[0; 4]);
    let mut sid_at = 8 + 16 * changed.len();
    for (s, a) in &changed {
        prev.extend_from_slice(&(previous + sid_at as u64).to_le_bytes());
        prev.extend_from_slice(&a.to_le_bytes());
        prev.extend_from_slice(&[0; 4]);
        sid_at += s.len();
    }
    for (s, _) in &changed {
        s.write(&mut prev);
    }
    if previous != 0 {
        abi::write_opt_u32(k, ret, prev.len() as u32)?;
        if (buffer_length as u32 as usize) < prev.len() {
            return Err(STATUS_BUFFER_TOO_SMALL);
        }
        // An empty previous state is the count alone (the length reported is 8).
        let n = if changed.is_empty() { 4 } else { prev.len() };
        abi::write(k, previous, &prev[..n])?;
    }
    k.with_state(|sys, _, _| {
        if let Some(tok) = sys.objects.security.token_mut(id) {
            tok.groups = t.groups;
        }
    });
    Ok(if all { STATUS_SUCCESS } else { token::STATUS_NOT_ALL_ASSIGNED })
}

/// `NtSetInformationToken(TokenHandle, TokenInformationClass,
/// TokenInformation, TokenInformationLength)`.
pub fn set_information_token<K: NtKernel>(k: &mut K, regs: &SyscallRegs) -> Result<NtStatus> {
    let [h, info_class, info, length] = abi::args::<K, 4>(k, regs)?;
    let info_class = info_class as u32;
    let length = length as u32 as usize;
    // Classes that cannot be set at all.
    let settable = [
        class::TOKEN_OWNER,
        class::TOKEN_PRIMARY_GROUP,
        class::TOKEN_DEFAULT_DACL,
        class::TOKEN_INTEGRITY_LEVEL,
        class::TOKEN_VIRTUALIZATION_ALLOWED,
        class::TOKEN_VIRTUALIZATION_ENABLED,
        class::TOKEN_SESSION_ID,
        class::TOKEN_UI_ACCESS,
        class::TOKEN_MANDATORY_POLICY,
        class::TOKEN_ORIGIN,
    ];
    let access =
        if info_class == class::TOKEN_SESSION_ID { token::TOKEN_ADJUST_SESSIONID } else { token::TOKEN_ADJUST_DEFAULT };
    let id = token_of(k, h, access)?;
    if !settable.contains(&info_class) {
        return Err(STATUS_INVALID_INFO_CLASS);
    }
    let data = match info_class {
        class::TOKEN_OWNER | class::TOKEN_PRIMARY_GROUP => {
            if length < 8 {
                return Err(token::STATUS_INFO_LENGTH_MISMATCH);
            }
            let p = abi::read_u64(k, info)?;
            if p == 0 {
                SetData::Sid(None)
            } else {
                let sid = read_sid(k, p)?;
                SetData::Sid(Some(Sid::from_bytes(&sid).unwrap_or_else(|| Sid::new(0, &[0]))))
            }
        }
        class::TOKEN_INTEGRITY_LEVEL => {
            if length < 16 {
                return Err(token::STATUS_INFO_LENGTH_MISMATCH);
            }
            let p = abi::read_u64(k, info)?;
            if p == 0 {
                SetData::Sid(None)
            } else {
                let sid = read_sid(k, p)?;
                SetData::Sid(Some(Sid::from_bytes(&sid).unwrap_or_else(|| Sid::new(5, &[18, 1]))))
            }
        }
        class::TOKEN_DEFAULT_DACL => {
            if length < 8 {
                return Err(token::STATUS_INFO_LENGTH_MISMATCH);
            }
            let p = abi::read_u64(k, info)?;
            if p == 0 {
                SetData::Acl(None)
            } else {
                let mut head = [0u8; 8];
                abi::read(k, p, &mut head)?;
                let size = usize::from(u16::from_le_bytes([head[2], head[3]]));
                if size < 8 || !(2..=4).contains(&head[0]) {
                    return Err(SecErr::InvalidAcl.nt());
                }
                let mut b = alloc::vec![0u8; size];
                abi::read(k, p, &mut b)?;
                SetData::Acl(Some(b))
            }
        }
        class::TOKEN_ORIGIN => {
            if length != 8 {
                return Err(token::STATUS_INFO_LENGTH_MISMATCH);
            }
            SetData::Value(0)
        }
        _ => {
            if length != 4 {
                return Err(token::STATUS_INFO_LENGTH_MISMATCH);
            }
            SetData::Value(abi::read_u32(k, info)?)
        }
    };
    k.with_state(|sys, _, _| {
        let t = sys.objects.security.token_mut(id).ok_or(STATUS_INVALID_HANDLE)?;
        t.set_information(info_class, &data)
    })?;
    Ok(STATUS_SUCCESS)
}

/// A SID from the caller's memory.
fn read_sid<K: NtKernel>(k: &mut K, p: u64) -> Result<Vec<u8>> {
    let mut head = [0u8; 8];
    abi::read(k, p, &mut head)?;
    let len = poly_security::sid::length_required(head[1]) as usize;
    let mut sid = alloc::vec![0u8; len];
    abi::read(k, p, &mut sid)?;
    Ok(sid)
}

// ----- duplicating and filtering -----------------------------------------------------------

/// `NtDuplicateToken(ExistingTokenHandle, DesiredAccess, ObjectAttributes*,
/// EffectiveOnly, TokenType, NewTokenHandle*)`. The impersonation level is
/// the `SECURITY_QUALITY_OF_SERVICE` of the attributes (12 bytes); a copy
/// cannot have a higher level than its source; a token has no name.
pub fn duplicate_token<K: NtKernel>(k: &mut K, regs: &SyscallRegs) -> Result<NtStatus> {
    let [h, desired, oa, effective_only, ty, out] = abi::args::<K, 6>(k, regs)?;
    let ty = ty as u32;
    let caller = k.caller();
    let src_id = token_of(k, h, token::TOKEN_DUPLICATE)?;
    if ty != token::TOKEN_PRIMARY && ty != token::TOKEN_IMPERSONATION {
        return Err(STATUS_INVALID_PARAMETER);
    }
    let attrs = abi::read_attributes(k, oa)?;
    let mut qos_level = None;
    if oa != 0 {
        let q = abi::read_u64(k, oa + 40)?;
        if q != 0 {
            let mut b = [0u8; 12];
            abi::read(k, q, &mut b)?;
            let len = u32::from_le_bytes(b[0..4].try_into().expect("4 bytes"));
            let level = u32::from_le_bytes(b[4..8].try_into().expect("4 bytes"));
            if len != 12 || b[8] > 1 {
                return Err(STATUS_INVALID_PARAMETER);
            }
            qos_level = Some(level);
        }
    }
    if attrs.name.is_some() || attrs.root.is_some() {
        return Err(STATUS_OBJECT_NAME_INVALID);
    }
    if attrs.attributes & !poly_ntobj::OBJ_VALID_ATTRIBUTES != 0 {
        return Err(STATUS_INVALID_PARAMETER);
    }
    if attrs.attributes & poly_ntobj::OBJ_PERMANENT != 0 {
        return Err(STATUS_PRIVILEGE_NOT_HELD);
    }
    let src = copy_of(k, src_id)?;
    // The new token's type and level.
    let level = match ty {
        token::TOKEN_PRIMARY => {
            if qos_level.is_some_and(|l| l > token::SECURITY_DELEGATION)
                || (src.kind == token::TOKEN_IMPERSONATION && src.level < token::SECURITY_IMPERSONATION)
            {
                return Err(STATUS_BAD_IMPERSONATION_LEVEL);
            }
            token::SECURITY_IMPERSONATION
        }
        _ => {
            let default =
                if src.kind == token::TOKEN_IMPERSONATION { src.level } else { token::SECURITY_IMPERSONATION };
            let l = qos_level.unwrap_or(default);
            if l > token::SECURITY_DELEGATION || (src.kind == token::TOKEN_IMPERSONATION && l > src.level) {
                return Err(STATUS_BAD_IMPERSONATION_LEVEL);
            }
            l
        }
    };
    let mut copy = src.duplicate(ty, level, 0);
    if effective_only as u8 != 0 {
        copy.privileges.retain(|&(_, a)| a & token::SE_PRIVILEGE_ENABLED != 0);
    }
    let source_access =
        k.with_state(|sys, _, _| sys.objects.granted_access(&caller, Handle(h)).unwrap_or(token::TOKEN_ALL_ACCESS));
    let mapped = poly_ntobj::ObjectType::Token.generic_mapping().map(desired as u32);
    let granted = if desired as u32 == 0 {
        source_access & TOKEN_VALID
    } else if mapped & MAXIMUM_ALLOWED != 0 {
        token::TOKEN_ALL_ACCESS
    } else {
        mapped & TOKEN_VALID
    };
    let handle = k.with_state(|sys, _, b| {
        let creator = sys.objects.security.effective(caller.process, caller.thread);
        let (_, obj) = sys.objects.new_token(copy, caller.thread)?;
        sys.objects.assign_security(obj, ObjectType::Token, creator, attrs.security.as_deref());
        let handle = open_exact(sys, &caller, obj, granted, attrs.attributes);
        let _ = sys.objects.dereference(obj);
        sys.reap(b);
        handle
    })?;
    return_handle(k, out, handle)?;
    Ok(STATUS_SUCCESS)
}

/// `NtFilterToken(ExistingTokenHandle, Flags, SidsToDisable*,
/// PrivilegesToDelete*, RestrictedSids*, NewTokenHandle*)`
/// (`CreateRestrictedToken`): the lists are `TOKEN_GROUPS` and
/// `TOKEN_PRIVILEGES`; the new handle has the access the old one has.
pub fn filter_token<K: NtKernel>(k: &mut K, regs: &SyscallRegs) -> Result<NtStatus> {
    let [h, flags, disable, delete, restrict, out] = abi::args::<K, 6>(k, regs)?;
    let caller = k.caller();
    let src_id = token_of(k, h, token::TOKEN_DUPLICATE)?;
    let disable_sids: Vec<Sid> = if disable == 0 {
        Vec::new()
    } else {
        read_groups(k, disable)?.into_iter().filter_map(|(s, _)| Sid::from_bytes(&s)).collect()
    };
    let restrict_sids: Vec<(Sid, u32)> = if restrict == 0 {
        Vec::new()
    } else {
        let mut v = Vec::new();
        for (s, a) in read_groups(k, restrict)? {
            v.push((Sid::from_bytes(&s).ok_or(STATUS_INVALID_PARAMETER)?, a));
        }
        v
    };
    let delete_luids: Vec<u32> = if delete == 0 {
        Vec::new()
    } else {
        let n = abi::read_u32(k, delete)? as usize;
        if n > MAX_NEW_STATE {
            return Err(STATUS_INVALID_PARAMETER);
        }
        let mut b = alloc::vec![0u8; 4 + 12 * n];
        abi::read(k, delete, &mut b)?;
        token::parse_token_privileges(&b).ok_or(STATUS_INVALID_PARAMETER)?.into_iter().map(|(l, _)| l).collect()
    };
    let src = copy_of(k, src_id)?;
    let new = src.filter(flags as u32, &disable_sids, &delete_luids, &restrict_sids, 0).map_err(|e| e.nt())?;
    let granted = k
        .with_state(|sys, _, _| sys.objects.granted_access(&caller, Handle(h)).unwrap_or(token::TOKEN_ALL_ACCESS))
        & TOKEN_VALID;
    let handle = k.with_state(|sys, _, b| {
        let creator = sys.objects.security.effective(caller.process, caller.thread);
        let (_, obj) = sys.objects.new_token(new, caller.thread)?;
        sys.objects.assign_security(obj, ObjectType::Token, creator, None);
        let handle = open_exact(sys, &caller, obj, granted, 0);
        let _ = sys.objects.dereference(obj);
        sys.reap(b);
        handle
    })?;
    return_handle(k, out, handle)?;
    Ok(STATUS_SUCCESS)
}

/// `NtCompareTokens(FirstToken, SecondToken, Equal*)`: `*Equal` is stored
/// (as 0) even when the call fails.
pub fn compare_tokens<K: NtKernel>(k: &mut K, regs: &SyscallRegs) -> Result<NtStatus> {
    let [a, b, equal] = abi::args::<K, 3>(k, regs)?;
    if equal != 0 {
        abi::write(k, equal, &[0])?;
    }
    let ia = token_of(k, a, token::TOKEN_QUERY)?;
    let ib = token_of(k, b, token::TOKEN_QUERY)?;
    let (ta, tb) = (copy_of(k, ia)?, copy_of(k, ib)?);
    if equal != 0 {
        abi::write(k, equal, &[u8::from(ta.same_context(&tb))])?;
    }
    Ok(STATUS_SUCCESS)
}

/// `NtPrivilegeCheck(ClientToken, RequiredPrivileges*, Result*)`.
pub fn privilege_check<K: NtKernel>(k: &mut K, regs: &SyscallRegs) -> Result<NtStatus> {
    let [h, set, result] = abi::args::<K, 3>(k, regs)?;
    let id = token_of(k, h, token::TOKEN_QUERY)?;
    let t = copy_of(k, id)?;
    if t.kind == token::TOKEN_IMPERSONATION && t.level == token::SECURITY_ANONYMOUS {
        return Err(STATUS_BAD_IMPERSONATION_LEVEL);
    }
    let n = abi::read_u32(k, set)? as usize;
    if n > MAX_NEW_STATE {
        return Err(STATUS_INVALID_PARAMETER);
    }
    let control = abi::read_u32(k, set + 4)?;
    let mut b = alloc::vec![0u8; 8 + 12 * n];
    abi::read(k, set, &mut b)?;
    let mut entries: Vec<(u32, u32)> = (0..n)
        .map(|i| {
            let c = &b[8 + 12 * i..];
            (u32::from_le_bytes([c[0], c[1], c[2], c[3]]), u32::from_le_bytes([c[8], c[9], c[10], c[11]]))
        })
        .collect();
    let ok = t.privilege_check(&mut entries, control & 1 != 0);
    for (i, (_, a)) in entries.iter().enumerate() {
        abi::write_u32(k, set + 8 + 12 * i as u64 + 8, *a)?;
    }
    abi::write(k, result, &[u8::from(ok)])?;
    Ok(STATUS_SUCCESS)
}

/// `NtAccessCheck(SecurityDescriptor*, ClientToken, DesiredAccess,
/// GenericMapping*, PrivilegeSet*, PrivilegeSetLength*, GrantedAccess*,
/// AccessStatus*)`. The call fails for a descriptor, token or access mask
/// it cannot check with; the answer is `*AccessStatus` and
/// `*GrantedAccess`.
pub fn access_check<K: NtKernel>(k: &mut K, regs: &SyscallRegs) -> Result<NtStatus> {
    let [sd_at, h, desired, mapping_at, priv_at, priv_len_at, granted_at, status_at] = abi::args::<K, 8>(k, regs)?;
    let id = token_of(k, h, token::TOKEN_QUERY)?;
    let t = copy_of(k, id)?;
    if t.kind != token::TOKEN_IMPERSONATION {
        return Err(STATUS_NO_IMPERSONATION_TOKEN);
    }
    if t.level == token::SECURITY_ANONYMOUS {
        return Err(STATUS_BAD_IMPERSONATION_LEVEL);
    }
    let rel = abi::read_security_descriptor(k, sd_at)?;
    let parts = Parts::from_relative(&rel).ok_or(STATUS_INVALID_SECURITY_DESCR)?;
    if parts.owner.is_none() || parts.group.is_none() {
        return Err(STATUS_INVALID_SECURITY_DESCR);
    }
    let desired = desired as u32;
    if desired & sacc::GENERIC_BITS != 0 {
        return Err(STATUS_GENERIC_NOT_MAPPED);
    }
    let mut m = [0u8; 16];
    abi::read(k, mapping_at, &mut m)?;
    let w = |i: usize| u32::from_le_bytes([m[i], m[i + 1], m[i + 2], m[i + 3]]);
    let mapping = sacc::GenericMapping { read: w(0), write: w(4), execute: w(8), all: w(12) };
    let outcome = sacc::check(&parts, &t.check_view(), desired, &mapping);
    // The privileges used go in the caller's PRIVILEGE_SET.
    let used = outcome.privileges.len();
    if used > 0 {
        let need = 8 + 12 * used as u32;
        let have = abi::read_u32(k, priv_len_at)?;
        if have < need {
            abi::write_u32(k, priv_len_at, need)?;
            return Err(STATUS_BUFFER_TOO_SMALL);
        }
        let mut set = Vec::new();
        set.extend_from_slice(&(used as u32).to_le_bytes());
        set.extend_from_slice(&0u32.to_le_bytes());
        for luid in &outcome.privileges {
            set.extend_from_slice(&luid.to_le_bytes());
            set.extend_from_slice(&0u32.to_le_bytes());
            set.extend_from_slice(&sacc::SE_PRIVILEGE_USED_FOR_ACCESS.to_le_bytes());
        }
        abi::write(k, priv_at, &set)?;
    } else if priv_at != 0 {
        // No privilege used: an empty set (count 0; the control word is
        // cleared too unless a missing privilege failed the call).
        let not_held = matches!(outcome.status, Err(SecErr::PrivilegeNotHeld));
        abi::write(k, priv_at, if not_held { &[0u8; 4] } else { &[0u8; 8] })?;
    }
    abi::write_u32(k, granted_at, outcome.granted)?;
    let status = match outcome.status {
        Ok(()) => STATUS_SUCCESS,
        Err(SecErr::PrivilegeNotHeld) => STATUS_PRIVILEGE_NOT_HELD,
        Err(_) => STATUS_ACCESS_DENIED,
    };
    abi::write_u32(k, status_at, status.0)?;
    Ok(STATUS_SUCCESS)
}

// ----- impersonation -----------------------------------------------------------------------

/// `NtSetInformationThread(ThreadHandle, ThreadInformationClass,
/// ThreadInformation, ThreadInformationLength)`: `ThreadImpersonationToken`
/// (5), a `HANDLE`: a token handle (needs `TOKEN_IMPERSONATE`; an
/// impersonation token) or NULL to revert. The thread handle needs
/// `THREAD_SET_THREAD_TOKEN`. The thread keeps the token when the handle is
/// closed.
pub fn set_information_thread<K: NtKernel>(k: &mut K, regs: &SyscallRegs) -> Result<NtStatus> {
    let [h, info_class, info, length] = abi::args::<K, 4>(k, regs)?;
    const THREAD_IMPERSONATION_TOKEN: u64 = 5;
    if info_class != THREAD_IMPERSONATION_TOKEN {
        return Err(STATUS_INVALID_INFO_CLASS);
    }
    let tid = thread_of(k, Handle(h), THREAD_SET_THREAD_TOKEN)?;
    if length as u32 != 8 {
        return Err(token::STATUS_INFO_LENGTH_MISMATCH);
    }
    let th = abi::read_u64(k, info)?;
    if th == 0 {
        k.with_state(|sys, _, _| sys.objects.set_thread_token(tid, None))?;
        return Ok(STATUS_SUCCESS);
    }
    let id = token_of(k, th, token::TOKEN_IMPERSONATE)?;
    k.with_state(|sys, _, _| {
        let t = sys.objects.security.token(id).ok_or(STATUS_INVALID_HANDLE)?;
        if t.kind != token::TOKEN_IMPERSONATION {
            return Err(STATUS_BAD_TOKEN_TYPE);
        }
        sys.objects.set_thread_token(tid, Some(id))
    })?;
    Ok(STATUS_SUCCESS)
}

// ----- security descriptors of objects -----------------------------------------------------

/// The object a handle names for the security services (the pseudo-handles
/// of the current process and thread included) and the access it grants.
fn secured_object(sys: &mut NtSystem, caller: &Caller, h: u64) -> Result<(poly_ntobj::ObjectId, u32)> {
    let hh = Handle(h);
    if hh == Handle::NULL {
        return Err(STATUS_INVALID_HANDLE);
    }
    if hh == Handle::CURRENT_PROCESS {
        let o = sys.objects.process_object(caller.process).ok_or(STATUS_INVALID_HANDLE)?;
        return Ok((o, ObjectType::Process.valid_access()));
    }
    if hh == Handle::CURRENT_THREAD {
        let o = sys.objects.thread_object(caller.thread).ok_or(STATUS_INVALID_HANDLE)?;
        return Ok((o, ObjectType::Thread.valid_access()));
    }
    let o = sys.objects.object_by_handle(caller, hh, 0, None)?;
    Ok((o, sys.objects.granted_access(caller, hh)?))
}

/// The information bits that need `ACCESS_SYSTEM_SECURITY` to read or
/// write: the SACL, and anything the manager does not know.
fn needs_system_security(info: u32) -> bool {
    const OK: u32 = 0x1 | 0x2 | 0x4 | 0x10 | 0x20 | 0x40 | 0x80 | 0x100 | 0xF000_0000;
    info & !OK != 0
}

/// `NtQuerySecurityObject(Handle, SecurityInformation, SecurityDescriptor*,
/// Length, LengthNeeded*)`.
pub fn query_security_object<K: NtKernel>(k: &mut K, regs: &SyscallRegs) -> Result<NtStatus> {
    let [h, info, out, length, needed] = abi::args::<K, 5>(k, regs)?;
    let info = info as u32;
    let caller = k.caller();
    let bytes = k.with_state(|sys, _, _| -> Result<Vec<u8>> {
        let (obj, granted) = secured_object(sys, &caller, h)?;
        if info
            & (sd::OWNER_SECURITY_INFORMATION
                | sd::GROUP_SECURITY_INFORMATION
                | sd::DACL_SECURITY_INFORMATION
                | sd::LABEL_SECURITY_INFORMATION)
            != 0
            && granted & READ_CONTROL == 0
        {
            return Err(STATUS_ACCESS_DENIED);
        }
        if (needs_system_security(info) || info & sd::SACL_SECURITY_INFORMATION != 0)
            && granted & ACCESS_SYSTEM_SECURITY == 0
        {
            return Err(STATUS_ACCESS_DENIED);
        }
        let stored = sys.objects.security.sd(obj).and_then(Parts::from_relative);
        // An object without a descriptor answers an empty one.
        let parts = stored.unwrap_or(Parts { revision: sd::SECURITY_DESCRIPTOR_REVISION, ..Parts::default() });
        Ok(sdops::select(&parts, info).to_relative_kernel())
    })?;
    if needed != 0 {
        abi::write_u32(k, needed, bytes.len() as u32)?;
    }
    if bytes.len() > length as u32 as usize {
        return Err(STATUS_BUFFER_TOO_SMALL);
    }
    abi::write(k, out, &bytes)?;
    Ok(STATUS_SUCCESS)
}

/// `NtSetSecurityObject(Handle, SecurityInformation, SecurityDescriptor*)`.
pub fn set_security_object<K: NtKernel>(k: &mut K, regs: &SyscallRegs) -> Result<NtStatus> {
    let [h, info, sd_at] = abi::args::<K, 3>(k, regs)?;
    let info = info as u32;
    let caller = k.caller();
    // Rights first: the handle and the information decide before the descriptor is read.
    let (obj, kind) = k.with_state(|sys, _, _| -> Result<(poly_ntobj::ObjectId, ObjectType)> {
        let (obj, granted) = secured_object(sys, &caller, h)?;
        let kind = sys.objects.object_type(obj).ok_or(STATUS_INVALID_HANDLE)?;
        if sys.objects.security.sd(obj).is_none() {
            return Err(STATUS_NO_SECURITY_ON_OBJECT);
        }
        let need_owner = info
            & (sd::OWNER_SECURITY_INFORMATION | sd::GROUP_SECURITY_INFORMATION | sd::LABEL_SECURITY_INFORMATION)
            != 0;
        let need_dac = info & sd::DACL_SECURITY_INFORMATION != 0;
        // The scope and process-trust labels (0x40, 0x80) are written like a
        // SACL (the sheets: refused without `ACCESS_SYSTEM_SECURITY`).
        let need_sys = info & (sd::SACL_SECURITY_INFORMATION | 0x40 | 0x80) != 0 || needs_system_security(info);
        if (need_owner && granted & WRITE_OWNER == 0)
            || (need_dac && granted & WRITE_DAC == 0)
            || (need_sys && granted & ACCESS_SYSTEM_SECURITY == 0)
        {
            return Err(STATUS_ACCESS_DENIED);
        }
        Ok((obj, kind))
    })?;
    let rel = abi::read_security_descriptor(k, sd_at)?;
    let new = Parts::from_relative(&rel).ok_or(STATUS_INVALID_SECURITY_DESCR)?;
    k.with_state(|sys, _, b| -> Result<()> {
        let (_, who) = effective(sys, &caller)?;
        let stored = sys.objects.security.sd(obj).and_then(Parts::from_relative).ok_or(STATUS_NO_SECURITY_ON_OBJECT)?;
        let merged = sdops::merge(&stored, &new, info).map_err(|e| match e {
            SecErr::InvalidSd => STATUS_INVALID_SECURITY_DESCR,
            e => e.nt(),
        })?;
        if info & sd::OWNER_SECURITY_INFORMATION != 0
            && !merged.owner.as_deref().is_some_and(|o| sdops::owner_allowed(&who, o))
        {
            return Err(STATUS_INVALID_OWNER);
        }
        if info & sd::LABEL_SECURITY_INFORMATION != 0 && !sdops::label_allowed(&who, &new) {
            return Err(token::STATUS_INVALID_LABEL);
        }
        if let Some(d) = &merged.dacl {
            // SAFETY: the bytes are a whole ACL.
            if !unsafe { poly_security::acl::is_valid(poly_security::acl::AclPtr(d.as_ptr() as *mut u8)) } {
                return Err(SecErr::InvalidAcl.nt());
            }
        }
        sys.objects.replace_security(obj, kind, merged);
        let _ = b;
        Ok(())
    })?;
    Ok(STATUS_SUCCESS)
}

/// The `CheckToken` of a token (for the personality's own checks).
pub fn check_view(t: &Token) -> CheckToken<'_> {
    t.check_view()
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
        assert_eq!(f.u64_at(buf), buf + 16);
        let sid = Sid::from_bytes(&f.bytes_at(buf + 16, 28)).unwrap();
        assert_eq!(sddl::to_string(&sid), "S-1-5-21-520193847-884726105-1936024571-1002");
        let small = f.alloc(8);
        assert_eq!(f.call(Service::QueryInformationToken, &[h, 1, small, 8, ret]), STATUS_BUFFER_TOO_SMALL);
        assert_eq!(f.u32_at(ret), 44);
        assert_eq!(f.u64_at(small), 0);
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
        let h = open(&mut f, 0x80);
        let (buf, ret) = (f.alloc(64), f.alloc(4));
        assert_eq!(f.call(Service::QueryInformationToken, &[h, 20, buf, 4, ret]), STATUS_ACCESS_DENIED);
        let q = open(&mut f, TOKEN_QUERY);
        assert_eq!(f.call(Service::QueryInformationToken, &[q, 7, buf, 64, ret]), STATUS_ACCESS_DENIED);
        let s = open(&mut f, 0x10);
        assert_eq!(f.call(Service::QueryInformationToken, &[s, 7, buf, 64, ret]), STATUS_SUCCESS);
        let out = f.alloc(8);
        assert_eq!(f.call(Service::OpenProcessToken, &[SELF, 0x10_0000, out]), STATUS_ACCESS_DENIED);
        assert_eq!(f.call(Service::OpenProcessToken, &[SELF, 0, out]), STATUS_ACCESS_DENIED);
        let g = open(&mut f, 0x8000_0000);
        assert_eq!(f.call(Service::QueryInformationToken, &[g, 20, buf, 4, ret]), STATUS_SUCCESS);
    }

    #[test]
    fn threads_have_no_token_until_one_is_set() {
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
        const SHUTDOWN: u32 = 19;
        const TCB: u32 = 7;
        let mut f = Fake::new();
        let h = open(&mut f, ADJUST);
        let q = open(&mut f, TOKEN_QUERY);
        assert_eq!(privilege(&mut f, q, SHUTDOWN), Some(3));
        // Disable it.
        let new = f.put(&[1, 0, 0, 0, SHUTDOWN as u8, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0]);
        assert_eq!(f.call(Service::AdjustPrivilegesToken, &[h, 0, new, 0, 0, 0]), STATUS_SUCCESS);
        assert_eq!(privilege(&mut f, q, SHUTDOWN), Some(1));
        assert_eq!(privilege(&mut f, (-4i64) as u64, SHUTDOWN), Some(1));
        // A previous state needs TOKEN_QUERY too.
        let (prev, ret) = (f.alloc(64), f.alloc(4));
        assert_eq!(f.call(Service::AdjustPrivilegesToken, &[h, 1, 0, 64, prev, ret]), STATUS_ACCESS_DENIED);
        // Not held: success, not all assigned.
        let tcb = f.put(&[1, 0, 0, 0, TCB as u8, 0, 0, 0, 0, 0, 0, 0, 2, 0, 0, 0]);
        assert_eq!(f.call(Service::AdjustPrivilegesToken, &[h, 0, tcb, 0, 0, 0]), NtStatus(0x106));
        assert_eq!(f.call(Service::AdjustPrivilegesToken, &[h, 0, 0, 0, 0, 0]), STATUS_INVALID_PARAMETER);
    }

    /// The sandbox (ADR 0171): a restricted token (one restricting SID the
    /// object's descriptor does not name) cannot open a registry key that
    /// the unrestricted token of the same process can, and the same token
    /// restricted by a SID the key does name (Users) can.
    #[test]
    fn a_restricted_token_cannot_open_what_the_process_can() {
        const KEY_READ: u64 = 0x2_0019;
        const SOFTWARE: &str = r"\Registry\Machine\SOFTWARE";
        const CURRENT_THREAD: u64 = (-2i64) as u64;
        let mut f = Fake::new();
        let open_key = |f: &mut Fake| {
            let out = f.alloc(8);
            let oa = f.attrs_in(0, SOFTWARE, 0x40);
            let st = f.call(Service::OpenKey, &[out, KEY_READ, oa]);
            if st == STATUS_SUCCESS {
                let h = f.u64_at(out);
                assert_eq!(f.call(Service::Close, &[h]), STATUS_SUCCESS);
            }
            st
        };
        // The process token reads it.
        assert_eq!(open_key(&mut f), STATUS_SUCCESS);
        let source = open(&mut f, 0x2);
        // Impersonation tokens restricted by `sid`, made current on the thread.
        let impersonate = |f: &mut Fake, sid: &[u8]| {
            let ptr = f.put(sid);
            let mut g = [0u8; 24];
            g[0..4].copy_from_slice(&1u32.to_le_bytes());
            g[8..16].copy_from_slice(&ptr.to_le_bytes());
            g[16..20].copy_from_slice(&0u32.to_le_bytes());
            let groups_at = f.put(&g);
            let filtered = f.alloc(8);
            assert_eq!(f.call(Service::FilterToken, &[source, 0, 0, 0, groups_at, filtered]), STATUS_SUCCESS);
            let imp = f.alloc(8);
            let oa = 0;
            assert_eq!(f.call(Service::DuplicateToken, &[f.u64_at(filtered), 0xf01ff, oa, 0, 2, imp]), STATUS_SUCCESS);
            let h = f.put_u64(f.u64_at(imp));
            assert_eq!(f.call(Service::SetInformationThread, &[CURRENT_THREAD, 5, h, 8]), STATUS_SUCCESS);
        };
        // S-1-0-0 (the null SID): not in the key's descriptor.
        impersonate(&mut f, &[1, 1, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0]);
        assert_eq!(open_key(&mut f), STATUS_ACCESS_DENIED);
        // Revert: allowed again.
        let none = f.put_u64(0);
        assert_eq!(f.call(Service::SetInformationThread, &[CURRENT_THREAD, 5, none, 8]), STATUS_SUCCESS);
        assert_eq!(open_key(&mut f), STATUS_SUCCESS);
        // S-1-5-32-545 (Users): named by the key.
        impersonate(&mut f, &[1, 2, 0, 0, 0, 0, 0, 5, 32, 0, 0, 0, 0x21, 2, 0, 0]);
        assert_eq!(open_key(&mut f), STATUS_SUCCESS);
    }
}

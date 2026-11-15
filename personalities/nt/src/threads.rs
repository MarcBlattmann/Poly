//! The process and thread services: create and end threads, open a process
//! or thread by client id, query basic information, suspend and resume, and
//! read and write a thread's CPU context.
//!
//! Decoding and handle resolution are here (host-tested through
//! [`crate::dispatch_with`]); the acts that need threads, the scheduler or
//! another thread's saved registers are the kernel's ([`NtKernel`]).

use crate::abi;
use crate::consts::*;
use crate::kernel::{NtKernel, ThreadCreate};
use crate::objects::{Result, return_handle};
use crate::procattrs::ProcAttrs;
use alloc::vec::Vec;
use poly_abi::SyscallRegs;
use poly_ntobj::access::{
    PROCESS_CREATE_THREAD, PROCESS_QUERY_INFORMATION, PROCESS_SET_INFORMATION, THREAD_GET_CONTEXT,
    THREAD_QUERY_INFORMATION, THREAD_SET_CONTEXT, THREAD_SET_INFORMATION, THREAD_SUSPEND_RESUME, THREAD_TERMINATE,
};
use poly_ntobj::status::*;
use poly_ntobj::{Handle, NtStatus, ObjectType, ProcessId, ThreadRef};

/// Resolve a thread handle to its thread id (the object's cookie), checking
/// `access`. `NtCurrentThread()` is the caller's own thread.
pub(crate) fn thread_of<K: NtKernel>(k: &mut K, h: Handle, access: u32) -> Result<ThreadRef> {
    if h == Handle::CURRENT_THREAD {
        return Ok(k.caller().thread);
    }
    let caller = k.caller();
    k.with_state(|sys, _, _| {
        let id = sys.objects.object_by_handle(&caller, h, access, Some(ObjectType::Thread))?;
        Ok(ThreadRef(sys.objects.cookie(id).expect("live thread object")))
    })
}

/// Resolve a process handle to its process id. `NtCurrentProcess()` is the
/// caller's own process; a null handle is `STATUS_INVALID_HANDLE`.
pub(crate) fn process_of<K: NtKernel>(k: &mut K, h: Handle, access: u32) -> Result<ProcessId> {
    if h == Handle::CURRENT_PROCESS {
        return Ok(k.caller().process);
    }
    if h == Handle::NULL {
        return Err(STATUS_INVALID_HANDLE);
    }
    let caller = k.caller();
    k.with_state(|sys, _, _| {
        let id = sys.objects.object_by_handle(&caller, h, access, Some(ObjectType::Process))?;
        Ok(ProcessId(sys.objects.cookie(id).expect("live process object") as u32))
    })
}

/// [`process_of`] with any one of `rights` (a handle opened with
/// `PROCESS_QUERY_LIMITED_INFORMATION` or with `PROCESS_QUERY_INFORMATION`
/// answers a basic query, Learn: "GetExitCodeProcess", "GetProcessId").
pub(crate) fn process_of_any<K: NtKernel>(k: &mut K, h: Handle, rights: &[u32]) -> Result<ProcessId> {
    let mut denied = STATUS_ACCESS_DENIED;
    for &r in rights {
        match process_of(k, h, r) {
            Ok(p) => return Ok(p),
            Err(e) if e == STATUS_ACCESS_DENIED => denied = e,
            Err(e) => return Err(e),
        }
    }
    Err(denied)
}

/// [`thread_of`] with any one of `rights`.
pub(crate) fn thread_of_any<K: NtKernel>(k: &mut K, h: Handle, rights: &[u32]) -> Result<ThreadRef> {
    let mut denied = STATUS_ACCESS_DENIED;
    for &r in rights {
        match thread_of(k, h, r) {
            Ok(t) => return Ok(t),
            Err(e) if e == STATUS_ACCESS_DENIED => denied = e,
            Err(e) => return Err(e),
        }
    }
    Err(denied)
}

/// `NtCreateThreadEx(ThreadHandle*, DesiredAccess, ObjectAttributes*,
/// ProcessHandle, StartRoutine, Argument, CreateFlags, ZeroBits, StackSize,
/// MaximumStackSize, AttributeList)`. Only the calling process is a valid
/// target for now (cross-process creation is `STATUS_NOT_IMPLEMENTED` until
/// the ntdll milestone).
pub fn create_thread_ex<K: NtKernel>(k: &mut K, regs: &SyscallRegs) -> Result<NtStatus> {
    let out = abi::arg(k, regs, 0)?;
    let access = abi::arg(k, regs, 1)? as u32;
    let process = Handle(abi::arg(k, regs, 3)?);
    let start_routine = abi::arg(k, regs, 4)?;
    let argument = abi::arg(k, regs, 5)?;
    let flags = abi::arg(k, regs, 6)? as u32;
    let stack_commit = abi::arg(k, regs, 8)?;
    let stack_reserve = abi::arg(k, regs, 9)?;
    let pid = process_of(k, process, PROCESS_CREATE_THREAD)?;
    if pid != k.caller().process {
        return Err(STATUS_NOT_IMPLEMENTED);
    }
    // `ObjectAttributes` (`lpThreadAttributes`): the descriptor of the new thread object.
    let attributes_at = abi::arg(k, regs, 2)?;
    let security = abi::read_attributes(k, attributes_at)?.security;
    let req = ThreadCreate {
        security,
        start_routine,
        argument,
        stack_reserve,
        stack_commit,
        suspended: flags & THREAD_CREATE_FLAGS_CREATE_SUSPENDED != 0,
        desired_access: if access == 0 { poly_ntobj::access::THREAD_ALL_ACCESS } else { access },
    };
    let created = k.create_thread(req)?;
    return_handle(k, out, created.handle)?;
    Ok(STATUS_SUCCESS)
}

/// The NT name of an executable's DOS path (`ImagePathName`): a full
/// drive path `X:\...` becomes `\??\X:\...`, a `\\?\` path keeps its
/// rest under `\??\`, and an NT path stays. Anything else (a relative
/// path, UNC) is `STATUS_OBJECT_PATH_SYNTAX_BAD`: `kernelbase` always
/// passes a full path (ADR 0079).
pub fn image_nt_name(dos: &[u16]) -> Result<alloc::vec::Vec<u16>> {
    let w = |s: &str| s.encode_utf16().collect::<alloc::vec::Vec<u16>>();
    let is_sep = |c: u16| c == u16::from(b'\\') || c == u16::from(b'/');
    if dos.starts_with(&w("\\??\\")) {
        return Ok(dos.to_vec());
    }
    let mut out = w("\\??\\");
    if dos.starts_with(&w("\\\\?\\")) {
        out.extend_from_slice(&dos[4..]);
        return Ok(out);
    }
    let letter = dos.first().is_some_and(|&c| c < 0x80 && (c as u8).is_ascii_alphabetic());
    if dos.len() >= 3 && letter && dos[1] == u16::from(b':') && is_sep(dos[2]) {
        out.extend(dos.iter().map(|&c| if c == u16::from(b'/') { u16::from(b'\\') } else { c }));
        return Ok(out);
    }
    Err(STATUS_OBJECT_PATH_SYNTAX_BAD)
}

/// `NtCreateUserProcess(ProcessHandle*, ThreadHandle*, ProcessDesiredAccess,
/// ThreadDesiredAccess, ProcessObjectAttributes*, ThreadObjectAttributes*,
/// ProcessFlags, ThreadFlags, ProcessParameters*, CreateInfo*,
/// AttributeList*)`, as Poly reads it (ADR 0079; the WDK does not document
/// it, and the `PS_CREATE_INFO` / `PS_ATTRIBUTE_LIST` structures are in no
/// public header):
///
/// - `ProcessParameters` (required, `STATUS_INVALID_PARAMETER`) is an
///   `RTL_USER_PROCESS_PARAMETERS` in the caller's memory
///   ([`crate::params::read`]): its `ImagePathName` names the executable
///   ([`image_nt_name`]), which is opened for execute and read; its
///   command line, environment and current directory go to the child, and
///   so does the creator's `STARTUPINFO` with, under
///   `STARTF_USESTDHANDLES`, the standard handles it chose (ADR 0089).
/// - `ThreadFlags` `THREAD_CREATE_FLAGS_CREATE_SUSPENDED`: the first
///   thread waits for `NtResumeThread` (ADR 0089).
/// - `ProcessFlags` [`PROCESS_CREATE_FLAGS_INHERIT_HANDLES`] (Poly's own
///   bit): the child's handle table starts with the creator's inheritable
///   handles at the same values (ADR 0089). Other bits are ignored.
/// - The object attributes, `CreateInfo` and `AttributeList` are not used:
///   the new process is unnamed, and its client id is read back with
///   `NtQueryInformationProcess` / `NtQueryInformationThread`.
/// - Desired access 0 means all access.
pub fn create_user_process<K: NtKernel>(k: &mut K, regs: &SyscallRegs) -> Result<NtStatus> {
    use crate::io::consts::*;
    let process_out = abi::arg(k, regs, 0)?;
    let thread_out = abi::arg(k, regs, 1)?;
    let process_access = abi::arg(k, regs, 2)? as u32;
    let thread_access = abi::arg(k, regs, 3)? as u32;
    let process_flags = abi::arg(k, regs, 6)? as u32;
    let thread_flags = abi::arg(k, regs, 7)? as u32;
    let params_at = abi::arg(k, regs, 8)?;
    if params_at == 0 || process_out == 0 || thread_out == 0 {
        return Err(STATUS_INVALID_PARAMETER);
    }
    // The block is in the caller's own layout (a 32-bit parent's has 4-byte pointers, ADR 0186).
    let layout = k.with_state(|_, p, _| p.layout());
    let params = crate::params::read_for(&layout, params_at, &mut |addr, buf| abi::read(k, addr, buf))?;
    let name = image_nt_name(&params.image_path)?;
    let attrs =
        abi::Attributes { root: None, name: Some(name), attributes: poly_ntobj::OBJ_CASE_INSENSITIVE, security: None };
    let image = crate::io::services::with_named(
        k,
        &attrs,
        FILE_EXECUTE | FILE_READ_DATA | SYNCHRONIZE,
        FILE_NON_DIRECTORY_FILE | FILE_SYNCHRONOUS_IO_NONALERT,
        |sys, cookie| crate::image::ImageSection::from_reader(sys.io.image_file(cookie)?),
    )?;
    let all = |a: u32, all: u32| if a == 0 { all } else { a };
    // Poly's own flag: CreateInfo is a handle to the primary token to give it.
    let given_token = if process_flags & PROCESS_CREATE_FLAGS_POLY_TOKEN != 0 {
        let h = abi::arg(k, regs, 9)?;
        let id = crate::security::token_of(
            k,
            h,
            poly_security::token::TOKEN_ASSIGN_PRIMARY | poly_security::token::TOKEN_QUERY,
        )?;
        // A primary token, or an impersonation token of at least Impersonation level.
        let ok = k.with_state(|sys, _, _| {
            sys.objects.security.token(id).is_some_and(|t| {
                t.kind == poly_security::token::TOKEN_PRIMARY || t.level >= poly_security::token::SECURITY_IMPERSONATION
            })
        });
        if !ok {
            return Err(poly_ntobj::NtStatus(0xC000_00A5));
        }
        Some(id)
    } else {
        None
    };
    let attributes = abi::arg(k, regs, 10)?;
    let extra = read_create_attributes(k, attributes)?;
    // The descriptors of the new process and thread objects (`lpProcessAttributes`,
    // `lpThreadAttributes`).
    let (process_at, thread_at) = (abi::arg(k, regs, 4)?, abi::arg(k, regs, 5)?);
    let process_security = abi::read_attributes(k, process_at)?.security;
    let thread_security = abi::read_attributes(k, thread_at)?.security;
    let caller = k.caller();
    let (jobs, join) = k.with_state(|sys, _, _| -> Result<(Vec<u64>, bool)> {
        let jobs = crate::job::resolve_jobs(sys, &caller, &extra.jobs)?;
        Ok((jobs, sys.jobs.check_create(caller.process, extra.breakaway)?))
    })?;
    let mitigation = extra.mitigation_state();
    let desktop = {
        let spec = params.startup.desktop.clone();
        k.with_state(|sys, proc, _| sys.winsta.child_start(&mut sys.objects, proc.pid, caller.thread, spec))
    };
    let req = crate::kernel::ProcessCreate {
        image,
        params,
        process_access: all(process_access, poly_ntobj::access::PROCESS_ALL_ACCESS),
        thread_access: all(thread_access, poly_ntobj::access::THREAD_ALL_ACCESS),
        inherit_handles: process_flags & PROCESS_CREATE_FLAGS_INHERIT_HANDLES != 0,
        suspended: thread_flags & THREAD_CREATE_FLAGS_CREATE_SUSPENDED != 0,
        token: given_token,
        inherit_only: extra.handle_list,
        jobs,
        join_creator_jobs: join,
        mitigation,
        desktop,
        error_mode: extra.error_mode,
        process_security,
        thread_security,
    };
    let created = k.create_process(req)?;
    k.with_state(|sys, _, _| sys.process_parents.insert(created.client.process, caller.process));
    return_handle(k, process_out, created.process)?;
    return_handle(k, thread_out, created.thread)?;
    Ok(STATUS_SUCCESS)
}

/// Poly's own attributes of `NtCreateUserProcess` (ADR 0169): what
/// `CreateProcessW` does with a `PROC_THREAD_ATTRIBUTE_LIST` and
/// `CREATE_BREAKAWAY_FROM_JOB`. The attribute list is `{ count: u64,
/// { kind: u64, value: u64, size: u64 } * count }`; `kind` 1 is the handle
/// list (`value` points at `size` bytes of handles), 2 the mitigation policy
/// (`value` points at the two 64-bit words, `size` 8 or 16), 3 the job list (as the handle list),
/// 4 the breakaway request, 5 the creator's error mode (the 32-bit `value`), 6 the child
/// process policy (`value` is the 32-bit value).
#[derive(Default)]
struct CreateAttributes {
    handle_list: Option<Vec<Handle>>,
    /// The two mitigation words (`PROC_THREAD_ATTRIBUTE_MITIGATION_POLICY`; the second is 0 for a
    /// value of 8 bytes or less).
    mitigation: [u64; 2],
    /// `PROC_THREAD_ATTRIBUTE_CHILD_PROCESS_POLICY`'s value.
    child_policy: u32,
    jobs: Vec<u64>,
    breakaway: bool,
    /// The creator's error mode (`SetErrorMode`), which the child starts with.
    error_mode: u32,
}

impl CreateAttributes {
    /// The mitigation state the child starts with.
    fn mitigation_state(&self) -> poly_kernelbase::mitigation::State {
        poly_kernelbase::mitigation::State::new()
            .with_creation_words(self.mitigation[0], self.mitigation[1])
            .with_child_policy(self.child_policy)
    }
}

fn read_create_attributes<K: NtKernel>(k: &mut K, at: u64) -> Result<CreateAttributes> {
    let mut out = CreateAttributes::default();
    if at == 0 {
        return Ok(out);
    }
    let count = abi::read_u64(k, at)?;
    if count > 16 {
        return Err(STATUS_INVALID_PARAMETER);
    }
    for i in 0..count {
        let base = at + 8 + 24 * i;
        let (kind, value, size) = (abi::read_u64(k, base)?, abi::read_u64(k, base + 8)?, abi::read_u64(k, base + 16)?);
        let list = |k: &mut K| -> Result<Vec<u64>> {
            if size > 8 * 1024 {
                return Err(STATUS_INVALID_PARAMETER);
            }
            (0..size / 8).map(|j| abi::read_u64(k, value + 8 * j)).collect()
        };
        match kind {
            1 => out.handle_list = Some(list(k)?.into_iter().map(Handle).collect()),
            2 => {
                // `value` points at the caller's words (ADR 0192).
                out.mitigation[0] = abi::read_u64(k, value)?;
                if size >= 16 {
                    out.mitigation[1] = abi::read_u64(k, value + 8)?;
                }
            }
            6 => out.child_policy = value as u32,
            3 => out.jobs = list(k)?,
            4 => out.breakaway = value != 0,
            5 => out.error_mode = value as u32,
            _ => {}
        }
    }
    Ok(out)
}

/// `NtTerminateThread(ThreadHandle, ExitStatus)`. A null handle ends every
/// other thread of the caller's process (left `STATUS_SUCCESS` as a no-op
/// stub beyond our needs; to be confirmed by the spec agent).
pub fn terminate_thread<K: NtKernel>(k: &mut K, regs: &SyscallRegs) -> Result<NtStatus> {
    let [h, status] = abi::args::<K, 2>(k, regs)?;
    if h == Handle::NULL.0 {
        return Ok(STATUS_SUCCESS);
    }
    let thread = thread_of(k, Handle(h), THREAD_TERMINATE)?;
    let status = NtStatus(status as u32);
    if thread == k.caller().thread {
        k.terminate_current_thread(status);
        Ok(STATUS_SUCCESS)
    } else {
        k.terminate_thread(thread, status).map(|()| STATUS_SUCCESS)
    }
}

/// `NtOpenProcess(ProcessHandle*, DesiredAccess, ObjectAttributes*,
/// ClientId*)`. We open by client id (the `UniqueProcess` field); opening by
/// name is not supported (no `\Device` process names).
pub fn open_process<K: NtKernel>(k: &mut K, regs: &SyscallRegs) -> Result<NtStatus> {
    let [out, access, attrs, client_ptr] = abi::args::<K, 4>(k, regs)?;
    if client_ptr == 0 {
        return Err(STATUS_INVALID_PARAMETER);
    }
    let attrs = abi::read_attributes(k, attrs)?;
    let pid = ProcessId(abi::read_u64(k, client_ptr)? as u32);
    // No access at all is refused (Learn: "OpenProcess"); id 4 is Windows'
    // System process, which Poly does not have and nobody may open.
    if access as u32 == 0 {
        return Err(STATUS_ACCESS_DENIED);
    }
    let caller = k.caller();
    let h = k.with_state(|sys, _, _| {
        let id = sys.objects.process_object(pid).ok_or(if pid.0 == SYSTEM_PROCESS_ID {
            STATUS_ACCESS_DENIED
        } else {
            STATUS_INVALID_CID
        })?;
        sys.objects.open_by_pointer(&caller, id, access as u32, attrs.attributes & poly_ntobj::OBJ_INHERIT)
    })?;
    return_handle(k, out, h)?;
    Ok(STATUS_SUCCESS)
}

/// The process id Windows gives its System process; Poly has none, and
/// where no process has that id the open is refused rather than not found
/// (the spec sheet of `OpenProcess`).
const SYSTEM_PROCESS_ID: u32 = 4;

/// `NtOpenThread(ThreadHandle*, DesiredAccess, ObjectAttributes*,
/// ClientId*)`: open by the `UniqueThread` client-id field.
pub fn open_thread<K: NtKernel>(k: &mut K, regs: &SyscallRegs) -> Result<NtStatus> {
    let [out, access, attrs, client_ptr] = abi::args::<K, 4>(k, regs)?;
    if client_ptr == 0 {
        return Err(STATUS_INVALID_PARAMETER);
    }
    let attrs = abi::read_attributes(k, attrs)?;
    if access as u32 == 0 {
        return Err(STATUS_ACCESS_DENIED);
    }
    let tid = ThreadRef(abi::read_u64(k, client_ptr + 8)?);
    let caller = k.caller();
    let h = k.with_state(|sys, _, _| {
        let id = sys.objects.thread_object(tid).ok_or(STATUS_INVALID_CID)?;
        sys.objects.open_by_pointer(&caller, id, access as u32, attrs.attributes & poly_ntobj::OBJ_INHERIT)
    })?;
    return_handle(k, out, h)?;
    Ok(STATUS_SUCCESS)
}

/// `NtSetInformationProcess(Process, ProcessInformationClass,
/// ProcessInformation, ProcessInformationLength)`: `ProcessMitigationPolicy`
/// (52), a `PROCESS_MITIGATION_POLICY_INFORMATION` (`Policy` u32, flags u32)
/// of the calling process, which sets the policies the kernel enforces: 2
/// dynamic code (`ProhibitDynamicCode`) and 4 system call disable
/// (`DisallowWin32kSystemCalls`). The others are `kernelbase`'s state.
pub fn set_information_process<K: NtKernel>(k: &mut K, regs: &SyscallRegs) -> Result<NtStatus> {
    let [h, class, info, length] = abi::args::<K, 4>(k, regs)?;
    const PROCESS_MITIGATION_POLICY: u64 = 52;
    // The values a process keeps for itself and a child inherits (`crate::procattrs`): any
    // process whose handle has `PROCESS_SET_INFORMATION`.
    if ProcAttrs::class_size(class).is_some() {
        let size = ProcAttrs::class_size(class).unwrap_or(0);
        if length as usize != size {
            return Err(STATUS_INFO_LENGTH_MISMATCH);
        }
        let pid = process_of(k, Handle(h), PROCESS_SET_INFORMATION)?;
        let mut data = alloc::vec![0u8; size];
        abi::read(k, info, &mut data)?;
        return k.with_state(|sys, _, _| {
            let attrs = sys.proc_attrs.entry(pid).or_insert_with(|| ProcAttrs::new(0));
            attrs.set(class, &data).map(|()| STATUS_SUCCESS)
        });
    }
    if class != PROCESS_MITIGATION_POLICY {
        return Err(STATUS_INVALID_INFO_CLASS);
    }
    if length != 8 {
        return Err(STATUS_INFO_LENGTH_MISMATCH);
    }
    let pid = process_of(k, Handle(h), 0x0200)?;
    if pid != k.caller().process {
        return Err(STATUS_ACCESS_DENIED);
    }
    let policy = abi::read_u32(k, info)?;
    let flags = abi::read_u32(k, info + 4)?;
    let done = k.with_state(|_, proc, _| {
        let mut state = proc.mitigation;
        state.set(policy, flags).map(|()| proc.set_mitigation(state))
    });
    // The Win32 error travels in the status (`STATUS_UNSUCCESSFUL` carries nothing): the failures
    // are `kernelbase`'s to name; this service answers invalid parameter, not supported or denied.
    done.map(|()| STATUS_SUCCESS).map_err(|e| match e {
        poly_kernelbase::mitigation::SetError::InvalidParameter => STATUS_INVALID_PARAMETER,
        poly_kernelbase::mitigation::SetError::NotSupported => STATUS_NOT_SUPPORTED,
        poly_kernelbase::mitigation::SetError::AccessDenied => STATUS_ACCESS_DENIED,
    })
}

/// `NtQueryInformationProcess(ProcessMitigationPolicy)` (class 52, Poly's reading: `info` is a
/// `PROCESS_MITIGATION_POLICY_INFORMATION` whose `Policy` the caller fills and whose `Flags` the
/// service writes): the flags word of the policy for the process `h` names (the caller or another
/// process the handle may query).
fn query_mitigation<K: NtKernel>(k: &mut K, h: u64, info: u64, length: u64) -> Result<NtStatus> {
    if length != 8 {
        return Err(STATUS_INFO_LENGTH_MISMATCH);
    }
    let pid = process_of_any(k, Handle(h), &[PROCESS_QUERY_LIMITED_INFORMATION, PROCESS_QUERY_INFORMATION])?;
    let policy = abi::read_u32(k, info)?;
    let flags = if pid == k.caller().process {
        k.with_state(|_, proc, _| proc.mitigation.get(policy))
    } else {
        k.with_process(pid, |_, proc, _| proc.mitigation.get(policy)).ok_or(STATUS_PROCESS_IS_TERMINATING)?
    };
    abi::write_u32(k, info + 4, flags)?;
    Ok(STATUS_SUCCESS)
}

/// `NtQueryInformationProcess(ProcessBasicInformation)`. ExitStatus is
/// `STATUS_PENDING` for a running process and its exit status once it has
/// ended ([`crate::system::NtSystem::process_exits`]); the PEB base (of the
/// caller's own process only) and the process id are reported.
pub fn query_information_process<K: NtKernel>(k: &mut K, regs: &SyscallRegs) -> Result<NtStatus> {
    let [h, class, buf, length, ret] = abi::args::<K, 5>(k, regs)?;
    if class as u32 == PROCESS_TIMES {
        return query_process_times(k, h, buf, length, ret);
    }
    if class == 52 {
        return query_mitigation(k, h, buf, length);
    }
    // `ProcessHandleCount` (20, `ntddk.h`): a `ULONG`, the open handles of the process
    // (`GetProcessHandleCount`).
    if class == 20 {
        let pid = process_of_any(k, Handle(h), &[PROCESS_QUERY_LIMITED_INFORMATION, PROCESS_QUERY_INFORMATION])?;
        let n = k.with_state(|sys, _, _| sys.objects.process_handle_count(pid).unwrap_or(0));
        return write_info(k, buf, length, ret, &(n as u32).to_le_bytes());
    }
    if ProcAttrs::class_size(class).is_some() {
        let pid = process_of_any(k, Handle(h), &[PROCESS_QUERY_LIMITED_INFORMATION, PROCESS_QUERY_INFORMATION])?;
        let bytes = k.with_state(|sys, _, _| {
            sys.proc_attrs.get(&pid).copied().unwrap_or(ProcAttrs::new(0)).get(class).expect("a class it keeps")
        });
        return write_info(k, buf, length, ret, &bytes);
    }
    if class as u32 == PROCESS_VM_COUNTERS {
        let pid = process_of_any(k, Handle(h), &[PROCESS_QUERY_LIMITED_INFORMATION, PROCESS_QUERY_INFORMATION])?;
        let c = k.memory_counters(pid).unwrap_or_default();
        // The plain structure, or the extended one when the buffer is big enough for it.
        let size = if length as usize >= VM_COUNTERS_EX_SIZE { VM_COUNTERS_EX_SIZE } else { VM_COUNTERS_SIZE };
        return write_info(k, buf, length, ret, &c.bytes()[..size]);
    }
    if class as u32 == PROCESS_WOW64_INFORMATION {
        let pid = process_of_any(k, Handle(h), &[PROCESS_QUERY_LIMITED_INFORMATION, PROCESS_QUERY_INFORMATION])?;
        let peb32 = |p: &crate::system::NtProcess| if p.wow64 { p.peb } else { 0 };
        let own = k.with_state(|_, proc, _| (proc.pid == pid).then(|| peb32(proc)));
        let value = match own {
            Some(v) => v,
            None => k.with_process(pid, |_, p, _| peb32(p)).unwrap_or(0),
        };
        return write_info(k, buf, length, ret, &value.to_le_bytes());
    }
    if class as u32 != PROCESS_BASIC_INFORMATION {
        return Err(STATUS_INVALID_INFO_CLASS);
    }
    let pid = process_of_any(k, Handle(h), &[PROCESS_QUERY_LIMITED_INFORMATION, PROCESS_QUERY_INFORMATION])?;
    let (exit, own_peb) = k.with_state(|sys, proc, _| {
        if proc.pid == pid {
            (proc.exit_status.unwrap_or(STATUS_PENDING), Some(proc.peb))
        } else {
            // Another process: its exit status once it has ended (ADR 0079).
            (sys.process_exits.get(&pid).copied().unwrap_or(STATUS_PENDING), None)
        }
    });
    // The PEB of another process is its own (`PebBaseAddress`, Learn: "PROCESS_BASIC_INFORMATION"):
    // the sandbox's broker writes into its target's PEB (ADR 0196); a process that has ended has none.
    let peb = match own_peb {
        Some(p) => p,
        None => k.with_process(pid, |_, proc, _| proc.peb).unwrap_or(0),
    };
    let parent = k.with_state(|sys, _, _| sys.process_parents.get(&pid).copied());
    let mut b = [0u8; PROCESS_BASIC_INFORMATION_SIZE];
    b[0..4].copy_from_slice(&exit.0.to_le_bytes());
    b[8..16].copy_from_slice(&peb.to_le_bytes());
    // `BasePriority`: the normal priority class's base priority, 8 (the sheet's `basic_child`).
    b[24..28].copy_from_slice(&8u32.to_le_bytes());
    b[32..40].copy_from_slice(&u64::from(pid.0).to_le_bytes());
    b[40..48].copy_from_slice(&parent.map_or(0, |p| u64::from(p.0)).to_le_bytes());
    // A buffer that is too small is refused and `ReturnLength` is not written (the sheet).
    if (length as usize) < b.len() {
        return Ok(STATUS_INFO_LENGTH_MISMATCH);
    }
    write_info(k, buf, length, ret, &b)
}

/// `NtQueryInformationThread(ThreadBasicInformation)`: ExitStatus (pending
/// while live), the TEB base and the client id.
pub fn query_information_thread<K: NtKernel>(k: &mut K, regs: &SyscallRegs) -> Result<NtStatus> {
    let [h, class, buf, length, ret] = abi::args::<K, 5>(k, regs)?;
    if class as u32 == THREAD_TIMES {
        return query_thread_times(k, h, buf, length, ret);
    }
    if class as u32 != THREAD_BASIC_INFORMATION {
        return Err(STATUS_INVALID_INFO_CLASS);
    }
    let tid = thread_of_any(k, Handle(h), &[THREAD_QUERY_LIMITED_INFORMATION, THREAD_QUERY_INFORMATION])?;
    let (exit, teb, pid) = k.with_state(|_sys, proc, _| {
        let teb = proc.thread(tid).map(|t| t.teb).unwrap_or(0);
        let exit = proc.thread_exit_status(tid).unwrap_or(STATUS_PENDING);
        (exit, teb, proc.pid)
    });
    let mut b = [0u8; THREAD_BASIC_INFORMATION_SIZE];
    b[0..4].copy_from_slice(&exit.0.to_le_bytes());
    b[8..16].copy_from_slice(&teb.to_le_bytes());
    b[16..24].copy_from_slice(&u64::from(pid.0).to_le_bytes());
    b[24..32].copy_from_slice(&tid.0.to_le_bytes());
    // `AffinityMask`: the CPUs the thread may run on (a thread that has ended: 0).
    let affinity = k.thread_affinity(tid).unwrap_or(0);
    b[32..40].copy_from_slice(&affinity.to_le_bytes());
    write_info(k, buf, length, ret, &b)
}

/// `ThreadAffinityMask` (`ntddk.h`'s `THREADINFOCLASS`): `NtSetInformationThread` takes a
/// `KAFFINITY`.
pub const THREAD_AFFINITY_MASK: u64 = 4;

/// `NtSetInformationThread(ThreadAffinityMask)`: the thread may run only on the CPUs of the mask
/// (those the system has; a mask with none is `STATUS_INVALID_PARAMETER`). Needs
/// `THREAD_SET_INFORMATION`.
pub fn set_thread_affinity<K: NtKernel>(k: &mut K, h: Handle, info: u64, length: u64) -> Result<NtStatus> {
    let tid = thread_of(k, h, THREAD_SET_INFORMATION)?;
    if length != 8 {
        return Err(STATUS_INFO_LENGTH_MISMATCH);
    }
    let mask = abi::read_u64(k, info)?;
    k.set_thread_affinity(tid, mask)?;
    Ok(STATUS_SUCCESS)
}

/// `NtGetCurrentProcessorNumber()`: the number of the CPU the calling thread runs on.
pub fn get_current_processor_number<K: NtKernel>(k: &mut K, _regs: &SyscallRegs) -> Result<NtStatus> {
    Ok(NtStatus(k.current_processor()))
}

pub fn suspend_thread<K: NtKernel>(k: &mut K, regs: &SyscallRegs) -> Result<NtStatus> {
    let [h, prev] = abi::args::<K, 2>(k, regs)?;
    let tid = thread_of(k, Handle(h), THREAD_SUSPEND_RESUME)?;
    let was = k.suspend_thread(tid)?;
    abi::write_opt_u32(k, prev, was)?;
    Ok(STATUS_SUCCESS)
}

pub fn resume_thread<K: NtKernel>(k: &mut K, regs: &SyscallRegs) -> Result<NtStatus> {
    let [h, prev] = abi::args::<K, 2>(k, regs)?;
    let tid = thread_of(k, Handle(h), THREAD_SUSPEND_RESUME)?;
    let was = k.resume_thread(tid)?;
    abi::write_opt_u32(k, prev, was)?;
    Ok(STATUS_SUCCESS)
}

/// `PROCESS_SUSPEND_RESUME` (`winnt.h`): "required to suspend or resume a
/// process" (Learn, "Process Security and Access Rights").
const PROCESS_SUSPEND_RESUME: u32 = 0x0800;

/// `NtSuspendProcess(ProcessHandle)` (ADR 0155 step 9). The service is not
/// documented; what is public is its name and the access right
/// `PROCESS_SUSPEND_RESUME`. Poly suspends every thread of the process once.
pub fn suspend_process<K: NtKernel>(k: &mut K, regs: &SyscallRegs) -> Result<NtStatus> {
    let [h] = abi::args::<K, 1>(k, regs)?;
    let pid = process_of(k, Handle(h), PROCESS_SUSPEND_RESUME)?;
    // The sheet: a process that has exited is STATUS_PROCESS_IS_TERMINATING, after the access
    // check.
    if k.with_state(|sys, _, _| sys.process_exits.contains_key(&pid)) {
        return Err(STATUS_PROCESS_IS_TERMINATING);
    }
    k.suspend_process(pid)?;
    Ok(STATUS_SUCCESS)
}

/// `NtResumeProcess(ProcessHandle)`: every thread of the process is resumed
/// once.
pub fn resume_process<K: NtKernel>(k: &mut K, regs: &SyscallRegs) -> Result<NtStatus> {
    let [h] = abi::args::<K, 1>(k, regs)?;
    let pid = process_of(k, Handle(h), PROCESS_SUSPEND_RESUME)?;
    // The sheet: a process that has exited is STATUS_PROCESS_IS_TERMINATING, after the access
    // check.
    if k.with_state(|sys, _, _| sys.process_exits.contains_key(&pid)) {
        return Err(STATUS_PROCESS_IS_TERMINATING);
    }
    k.resume_process(pid)?;
    Ok(STATUS_SUCCESS)
}

/// `NtGetContextThread(ThreadHandle, Context*)`: the `ContextFlags` in the
/// buffer say which groups the caller wants; we fill those we support.
pub fn get_context_thread<K: NtKernel>(k: &mut K, regs: &SyscallRegs) -> Result<NtStatus> {
    let [h, ctx_ptr] = abi::args::<K, 2>(k, regs)?;
    let want = abi::read_u32(k, ctx_ptr + CTX_FLAGS as u64)?;
    let tid = thread_of(k, Handle(h), THREAD_GET_CONTEXT)?;
    let ctx = k.get_context(tid)?;
    let mut buf = alloc::vec![0u8; CONTEXT_SIZE];
    abi::read(k, ctx_ptr, &mut buf)?;
    ctx.write_to(&mut buf, want);
    abi::write(k, ctx_ptr, &buf)?;
    Ok(STATUS_SUCCESS)
}

/// `NtSetContextThread(ThreadHandle, Context*)`.
pub fn set_context_thread<K: NtKernel>(k: &mut K, regs: &SyscallRegs) -> Result<NtStatus> {
    let [h, ctx_ptr] = abi::args::<K, 2>(k, regs)?;
    let want = abi::read_u32(k, ctx_ptr + CTX_FLAGS as u64)?;
    let tid = thread_of(k, Handle(h), THREAD_SET_CONTEXT)?;
    let mut buf = alloc::vec![0u8; CONTEXT_SIZE];
    abi::read(k, ctx_ptr, &mut buf)?;
    // Seed from the thread's current context so groups the caller did not
    // select keep their values.
    let mut ctx = k.get_context(tid)?;
    ctx.read_from(&buf, want);
    k.set_context(tid, &ctx, want)?;
    Ok(STATUS_SUCCESS)
}

/// `NtQueryInformationProcess(ProcessTimes)` (ADR 0087): the creation time
/// Poly noted when the process was made (a `FILETIME`), an exit time of 0
/// (Poly keeps no times of ended processes), and the kernel and user CPU
/// time the kernel counted ([`NtKernel::process_times`]), in 100 ns units.
fn query_process_times<K: NtKernel>(k: &mut K, h: u64, buf: u64, length: u64, ret: u64) -> Result<NtStatus> {
    // Learn, `GetProcessTimes`: "The handle must have the PROCESS_QUERY_INFORMATION or
    // PROCESS_QUERY_LIMITED_INFORMATION access right".
    let pid = process_of_any(k, Handle(h), &[PROCESS_QUERY_LIMITED_INFORMATION, PROCESS_QUERY_INFORMATION])?;
    let [created, kernel, user] = k.process_times(pid).unwrap_or([0; 3]);
    let mut b = [0u8; KERNEL_USER_TIMES_SIZE];
    b[0..8].copy_from_slice(&created.to_le_bytes());
    b[16..24].copy_from_slice(&kernel.to_le_bytes());
    b[24..32].copy_from_slice(&user.to_le_bytes());
    write_info(k, buf, length, ret, &b)
}

/// A process's memory in pages, what `VM_COUNTERS_EX` (`ntddk.h`) reports in bytes: the page
/// faults, the pages it maps now (its working set) and the most it mapped at once, and its own
/// pages (not shared with another process or the page cache: its commit charge and private
/// usage) now and at most. Poly does not count reserved address space: the virtual sizes are
/// the working set's.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct VmCounters {
    pub faults: u64,
    pub resident: u64,
    pub peak_resident: u64,
    pub private: u64,
    pub peak_private: u64,
}

impl VmCounters {
    /// The `VM_COUNTERS_EX` bytes (x64): `PeakVirtualSize`, `VirtualSize`, `PageFaultCount`
    /// (a `ULONG`, padded), `PeakWorkingSetSize`, `WorkingSetSize`, the four quota pool sizes
    /// (0), `PagefileUsage`, `PeakPagefileUsage`, `PrivateUsage`.
    pub fn bytes(&self) -> [u8; VM_COUNTERS_EX_SIZE] {
        const PAGE: u64 = 4096;
        let peak_resident = self.peak_resident.max(self.resident) * PAGE;
        let resident = self.resident * PAGE;
        let private = self.private * PAGE;
        let peak_private = self.peak_private.max(self.private) * PAGE;
        let mut b = [0u8; VM_COUNTERS_EX_SIZE];
        let mut put = |at: usize, v: u64| b[at..at + 8].copy_from_slice(&v.to_le_bytes());
        put(0, peak_resident);
        put(8, resident);
        put(24, peak_resident);
        put(32, resident);
        put(72, private);
        put(80, peak_private);
        put(88, private);
        b[16..20].copy_from_slice(&(self.faults.min(u64::from(u32::MAX)) as u32).to_le_bytes());
        b
    }
}

/// `NtQueryInformationThread(ThreadTimes)` (Learn, `GetThreadTimes`: the handle needs
/// `THREAD_QUERY_INFORMATION` or `THREAD_QUERY_LIMITED_INFORMATION`): the creation time the
/// kernel knows, an exit time of 0 while the thread runs, and the kernel and user CPU time
/// ([`NtKernel::thread_times`]), in 100 ns units.
fn query_thread_times<K: NtKernel>(k: &mut K, h: u64, buf: u64, length: u64, ret: u64) -> Result<NtStatus> {
    let tid = thread_of_any(k, Handle(h), &[THREAD_QUERY_LIMITED_INFORMATION, THREAD_QUERY_INFORMATION])?;
    let pid = k.with_state(|_, proc, _| proc.pid);
    let [created, kernel, user] = k.thread_times(pid, tid).unwrap_or([0; 3]);
    let mut b = [0u8; KERNEL_USER_TIMES_SIZE];
    b[0..8].copy_from_slice(&created.to_le_bytes());
    b[16..24].copy_from_slice(&kernel.to_le_bytes());
    b[24..32].copy_from_slice(&user.to_le_bytes());
    write_info(k, buf, length, ret, &b)
}

fn write_info<K: NtKernel>(k: &mut K, buf: u64, length: u64, ret: u64, info: &[u8]) -> Result<NtStatus> {
    abi::write_opt_u32(k, ret, info.len() as u32)?;
    if (length as usize) < info.len() {
        return Ok(STATUS_INFO_LENGTH_MISMATCH);
    }
    abi::write(k, buf, info)?;
    Ok(STATUS_SUCCESS)
}

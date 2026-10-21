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
use poly_abi::SyscallRegs;
use poly_ntobj::access::{
    PROCESS_CREATE_THREAD, PROCESS_QUERY_INFORMATION, THREAD_GET_CONTEXT, THREAD_QUERY_INFORMATION,
    THREAD_SET_CONTEXT, THREAD_SUSPEND_RESUME, THREAD_TERMINATE,
};
use poly_ntobj::status::*;
use poly_ntobj::{Handle, NtStatus, ObjectType, ProcessId, ThreadRef};

/// Resolve a thread handle to its thread id (the object's cookie), checking
/// `access`. `NtCurrentThread()` is the caller's own thread.
fn thread_of<K: NtKernel>(k: &mut K, h: Handle, access: u32) -> Result<ThreadRef> {
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
fn process_of<K: NtKernel>(k: &mut K, h: Handle, access: u32) -> Result<ProcessId> {
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
    let req = ThreadCreate {
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
    let [out, access, _attrs, client_ptr] = abi::args::<K, 4>(k, regs)?;
    if client_ptr == 0 {
        return Err(STATUS_INVALID_PARAMETER);
    }
    let pid = ProcessId(abi::read_u64(k, client_ptr)? as u32);
    let caller = k.caller();
    let h = k.with_state(|sys, _, _| {
        let id = sys.objects.process_object(pid).ok_or(STATUS_INVALID_CID)?;
        sys.objects.open_by_pointer(&caller, id, access as u32, 0)
    })?;
    return_handle(k, out, h)?;
    Ok(STATUS_SUCCESS)
}

/// `NtOpenThread(ThreadHandle*, DesiredAccess, ObjectAttributes*,
/// ClientId*)`: open by the `UniqueThread` client-id field.
pub fn open_thread<K: NtKernel>(k: &mut K, regs: &SyscallRegs) -> Result<NtStatus> {
    let [out, access, _attrs, client_ptr] = abi::args::<K, 4>(k, regs)?;
    if client_ptr == 0 {
        return Err(STATUS_INVALID_PARAMETER);
    }
    let tid = ThreadRef(abi::read_u64(k, client_ptr + 8)?);
    let caller = k.caller();
    let h = k.with_state(|sys, _, _| {
        let id = sys.objects.thread_object(tid).ok_or(STATUS_INVALID_CID)?;
        sys.objects.open_by_pointer(&caller, id, access as u32, 0)
    })?;
    return_handle(k, out, h)?;
    Ok(STATUS_SUCCESS)
}

/// `NtQueryInformationProcess(ProcessBasicInformation)`. ExitStatus is
/// `STATUS_PENDING` for a running process; the PEB base and the process id
/// are reported. Querying an exited *child* process's status needs
/// cross-process creation (left for the ntdll milestone).
pub fn query_information_process<K: NtKernel>(k: &mut K, regs: &SyscallRegs) -> Result<NtStatus> {
    let [h, class, buf, length, ret] = abi::args::<K, 5>(k, regs)?;
    if class as u32 != PROCESS_BASIC_INFORMATION {
        return Err(STATUS_INVALID_INFO_CLASS);
    }
    let pid = process_of(k, Handle(h), PROCESS_QUERY_INFORMATION)?;
    let caller = k.caller();
    let (exit, peb) = k.with_state(|_sys, proc, _| {
        if proc.pid == pid {
            (proc.exit_status.unwrap_or(STATUS_PENDING), proc.peb)
        } else {
            (STATUS_PENDING, 0)
        }
    });
    let _ = caller;
    let mut b = [0u8; PROCESS_BASIC_INFORMATION_SIZE];
    b[0..4].copy_from_slice(&exit.0.to_le_bytes());
    b[8..16].copy_from_slice(&peb.to_le_bytes());
    b[32..40].copy_from_slice(&u64::from(pid.0).to_le_bytes());
    write_info(k, buf, length, ret, &b)
}

/// `NtQueryInformationThread(ThreadBasicInformation)`: ExitStatus (pending
/// while live), the TEB base and the client id.
pub fn query_information_thread<K: NtKernel>(k: &mut K, regs: &SyscallRegs) -> Result<NtStatus> {
    let [h, class, buf, length, ret] = abi::args::<K, 5>(k, regs)?;
    if class as u32 != THREAD_BASIC_INFORMATION {
        return Err(STATUS_INVALID_INFO_CLASS);
    }
    let tid = thread_of(k, Handle(h), THREAD_QUERY_INFORMATION)?;
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
    write_info(k, buf, length, ret, &b)
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

fn write_info<K: NtKernel>(k: &mut K, buf: u64, length: u64, ret: u64, info: &[u8]) -> Result<NtStatus> {
    abi::write_opt_u32(k, ret, info.len() as u32)?;
    if (length as usize) < info.len() {
        return Ok(STATUS_INFO_LENGTH_MISMATCH);
    }
    abi::write(k, buf, info)?;
    Ok(STATUS_SUCCESS)
}

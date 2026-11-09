//! Named pipes (Learn: "Named Pipes", "CreateNamedPipeW" and the
//! functions around it; ADR 0169) over `NtCreateNamedPipeFile`,
//! `NtCreateFile` on `\??\pipe\<name>` and `NtFsControlFile`'s pipe codes.
//! The argument rules are `poly_kernelbase::namedpipe`'s.

use crate::base::*;
use crate::file::Overlapped;
use crate::nt::{self, Handle, IoStatusBlock};
use alloc::vec::Vec;
use poly_kernelbase::error::*;
use poly_kernelbase::namedpipe::{self as np, Local, Peek};
use poly_ntobj::NtStatus;

const FSCTL_PIPE_DISCONNECT: u32 = 0x0011_0004;
const FSCTL_PIPE_LISTEN: u32 = 0x0011_0008;
const FSCTL_PIPE_PEEK: u32 = 0x0011_400C;
const FSCTL_PIPE_TRANSCEIVE: u32 = 0x0011_C017;
const FSCTL_PIPE_WAIT: u32 = 0x0011_0018;
/// Poly's own (ADR 0169): client and server process, sessions, flags.
const FSCTL_POLY_PIPE_PROCESSES: u32 = 0x0011_0100;

const FILE_PIPE_INFORMATION: u32 = 23;
const FILE_PIPE_LOCAL_INFORMATION: u32 = 24;
const FILE_PIPE_REMOTE_INFORMATION: u32 = 25;

const STATUS_BUFFER_OVERFLOW: NtStatus = NtStatus(0x8000_0005);

const FILE_READ_ATTRIBUTES: u32 = 0x80;
const SYNCHRONIZE: u32 = 0x0010_0000;
const FILE_SYNCHRONOUS_IO_NONALERT: u32 = 0x20;
const GENERIC_READ: u32 = 0x8000_0000;
const GENERIC_WRITE: u32 = 0x4000_0000;
const OPEN_EXISTING: u32 = 3;

/// `NtFsControlFile`, completed in the call: on an overlapped handle with no
/// `OVERLAPPED` the request is waited for; with one, its status block, event
/// and (unless the event's low bit is set) completion context are the
/// `OVERLAPPED`'s. Returns the status and `IO_STATUS_BLOCK.Information`.
pub(crate) unsafe fn fsctl(
    handle: Handle,
    code: u32,
    input: &[u8],
    output: &mut [u8],
    ov: *mut Overlapped,
) -> (NtStatus, u64) {
    let mut local = IoStatusBlock::default();
    let (iosb, event, context): (*mut IoStatusBlock, Handle, *const u8) = if ov.is_null() {
        (&mut local, 0, core::ptr::null())
    } else {
        // SAFETY: the caller's OVERLAPPED; its first two fields are the
        // IO_STATUS_BLOCK the kernel completes.
        unsafe {
            (*ov).internal = nt::STATUS_PENDING.0 as u64;
            let e = (*ov).event;
            (ov as *mut IoStatusBlock, e & !1, if e & 1 != 0 { core::ptr::null() } else { ov as *const u8 })
        }
    };
    // SAFETY: valid buffers and a status block that outlives a wait.
    let mut st = unsafe {
        nt::NtFsControlFile(
            handle,
            event,
            core::ptr::null(),
            context,
            iosb,
            code,
            input.as_ptr(),
            input.len() as u32,
            output.as_mut_ptr(),
            output.len() as u32,
        )
    };
    if st == nt::STATUS_PENDING && ov.is_null() {
        // SAFETY: a handle value.
        unsafe { nt::NtWaitForSingleObject(handle, 0, core::ptr::null()) };
        // SAFETY: the kernel completed the status block.
        st = NtStatus(unsafe { (*iosb).status } as u32);
    }
    // SAFETY: the status block is ours or the caller's.
    (st, unsafe { (*iosb).information })
}

fn query(handle: Handle, class: u32, out: &mut [u8]) -> Result<(), NtStatus> {
    let mut iosb = IoStatusBlock::default();
    // SAFETY: valid pointers; `out` is the class's size.
    let st = unsafe { nt::NtQueryInformationFile(handle, &mut iosb, out.as_mut_ptr(), out.len() as u32, class) };
    if st.is_success() { Ok(()) } else { Err(st) }
}

fn local_info(handle: Handle) -> Result<Local, NtStatus> {
    let mut b = [0u8; 36];
    query(handle, FILE_PIPE_LOCAL_INFORMATION, &mut b)?;
    Ok(Local::parse(&b).expect("36 bytes"))
}

/// `CreateNamedPipeW`.
#[unsafe(no_mangle)]
pub unsafe extern "system" fn CreateNamedPipeW(
    name: *const u16,
    open_mode: u32,
    pipe_mode: u32,
    max_instances: u32,
    out_size: u32,
    in_size: u32,
    default_timeout: u32,
    security: *const u8,
) -> Handle {
    // SAFETY: the caller's string.
    let Some(name) = (unsafe { wide_arg(name) }) else { return fail(ERROR_PATH_NOT_FOUND, INVALID_HANDLE_VALUE) };
    if let Err(e) = np::pipe_name(name) {
        // A path that is no device path is the file system's to refuse.
        if e != ERROR_BAD_PATHNAME {
            return fail(e, INVALID_HANDLE_VALUE);
        }
    }
    let p = match np::server_params(open_mode, pipe_mode, max_instances, out_size, in_size, default_timeout) {
        Ok(p) => p,
        Err(e) => return fail(e, INVALID_HANDLE_VALUE),
    };
    let nt_name = match nt_name(name) {
        Ok(n) => n,
        Err(e) => return fail(e, INVALID_HANDLE_VALUE),
    };
    let mut oa = nt_name.attributes();
    // SECURITY_ATTRIBUTES.bInheritHandle (minwinbase.h: at 16 on x64).
    // SAFETY: the caller's structure when non-null.
    if !security.is_null() && unsafe { (security.add(16) as *const i32).read_unaligned() } != 0 {
        oa.attributes |= nt::OBJ_INHERIT;
    }
    let mut h: Handle = 0;
    let mut iosb = IoStatusBlock::default();
    // SAFETY: valid pointers.
    let st = unsafe {
        nt::NtCreateNamedPipeFile(
            &mut h,
            p.access,
            &oa,
            &mut iosb,
            p.share,
            p.disposition,
            p.options,
            p.pipe_type,
            p.read_mode,
            p.completion_mode,
            p.max_instances,
            p.inbound_quota,
            p.outbound_quota,
            &p.timeout,
        )
    };
    if !st.is_success() {
        return fail_status(st, INVALID_HANDLE_VALUE);
    }
    set_last_error(np::created_error(iosb.information));
    h
}

/// `ConnectNamedPipe`: `FSCTL_PIPE_LISTEN`.
#[unsafe(no_mangle)]
pub unsafe extern "system" fn ConnectNamedPipe(pipe: Handle, ov: *mut Overlapped) -> i32 {
    // SAFETY: the caller's OVERLAPPED.
    let (st, _) = unsafe { fsctl(pipe, FSCTL_PIPE_LISTEN, &[], &mut [], ov) };
    if st == nt::STATUS_PENDING {
        return fail(ERROR_IO_PENDING, FALSE);
    }
    if st.is_success() { TRUE } else { fail_status(st, FALSE) }
}

/// `DisconnectNamedPipe`: `FSCTL_PIPE_DISCONNECT`.
#[unsafe(no_mangle)]
pub extern "system" fn DisconnectNamedPipe(pipe: Handle) -> i32 {
    // SAFETY: no OVERLAPPED.
    let (st, _) = unsafe { fsctl(pipe, FSCTL_PIPE_DISCONNECT, &[], &mut [], core::ptr::null_mut()) };
    if st.is_success() { TRUE } else { fail_status(st, FALSE) }
}

/// `PeekNamedPipe`: `FSCTL_PIPE_PEEK`, whose answer is a header and the data.
#[unsafe(no_mangle)]
pub unsafe extern "system" fn PeekNamedPipe(
    pipe: Handle,
    buffer: *mut u8,
    size: u32,
    read: *mut u32,
    available: *mut u32,
    left: *mut u32,
) -> i32 {
    let want = if buffer.is_null() { 0 } else { size as usize };
    let mut out = alloc::vec![0u8; 16 + want];
    // SAFETY: no OVERLAPPED.
    let (st, info) = unsafe { fsctl(pipe, FSCTL_PIPE_PEEK, &[], &mut out, core::ptr::null_mut()) };
    if !st.is_success() && st != STATUS_BUFFER_OVERFLOW {
        return fail_status(st, FALSE);
    }
    let Some(header) = Peek::parse(&out) else { return fail(ERROR_INVALID_FUNCTION, FALSE) };
    let copied = (info as usize).saturating_sub(16).min(want);
    // SAFETY: the caller's buffer holds `size` bytes; the out-pointers are the caller's.
    unsafe {
        if copied > 0 {
            core::ptr::copy_nonoverlapping(out.as_ptr().add(16), buffer, copied);
        }
        if !read.is_null() {
            read.write(copied as u32);
        }
        if !available.is_null() {
            available.write(header.available);
        }
        if !left.is_null() {
            left.write(header.bytes_left(copied as u32));
        }
    }
    TRUE
}

/// `GetNamedPipeInfo`.
#[unsafe(no_mangle)]
pub unsafe extern "system" fn GetNamedPipeInfo(
    pipe: Handle,
    flags: *mut u32,
    out_size: *mut u32,
    in_size: *mut u32,
    max_instances: *mut u32,
) -> i32 {
    let info = match local_info(pipe) {
        Ok(i) => i,
        Err(st) => return fail_status(st, FALSE),
    };
    // SAFETY: the caller's optional out-pointers.
    unsafe {
        if !flags.is_null() {
            flags.write(info.flags());
        }
        if !out_size.is_null() {
            out_size.write(info.outbound);
        }
        if !in_size.is_null() {
            in_size.write(info.inbound);
        }
        if !max_instances.is_null() {
            max_instances.write(info.max_instances());
        }
    }
    TRUE
}

/// `GetNamedPipeHandleStateW`. The collection values are not supported on a
/// local pipe; the user name needs the server to have read from the client.
#[unsafe(no_mangle)]
pub unsafe extern "system" fn GetNamedPipeHandleStateW(
    pipe: Handle,
    state: *mut u32,
    instances: *mut u32,
    max_collection: *mut u32,
    collect_timeout: *mut u32,
    user_name: *mut u16,
    user_name_size: u32,
) -> i32 {
    let mut mode = [0u8; 8];
    if let Err(st) = query(pipe, FILE_PIPE_INFORMATION, &mut mode) {
        return fail_status(st, FALSE);
    }
    if !max_collection.is_null() || !collect_timeout.is_null() {
        // Learn: only for a client of a remote pipe.
        let mut remote = [0u8; 16];
        let _ = query(pipe, FILE_PIPE_REMOTE_INFORMATION, &mut remote);
        return fail(ERROR_INVALID_PARAMETER, FALSE);
    }
    let local = match local_info(pipe) {
        Ok(l) => Some(l),
        Err(_) if instances.is_null() => None,
        Err(st) => return fail_status(st, FALSE),
    };
    if !user_name.is_null() {
        let Some(l) = local else { return fail(ERROR_ACCESS_DENIED, FALSE) };
        if l.end != 1 {
            return fail(ERROR_INVALID_FUNCTION, FALSE);
        }
        let mut out = [0u8; 20];
        // SAFETY: no OVERLAPPED.
        let (st, _) = unsafe { fsctl(pipe, FSCTL_POLY_PIPE_PROCESSES, &[], &mut out, core::ptr::null_mut()) };
        if !st.is_success() {
            return fail_status(st, FALSE);
        }
        let flags = u32::from_le_bytes(out[16..20].try_into().expect("4 bytes"));
        if flags & 2 == 0 {
            return fail(ERROR_CANNOT_IMPERSONATE, FALSE);
        }
        let name: Vec<u16> = "POLY\\user".encode_utf16().collect();
        if (user_name_size as usize) < name.len() + 1 {
            return fail(ERROR_INSUFFICIENT_BUFFER, FALSE);
        }
        // SAFETY: the caller's buffer holds `user_name_size` characters.
        unsafe { put_wide(user_name, &name) };
    }
    let read_mode = u32::from_le_bytes(mode[0..4].try_into().expect("4 bytes"));
    let completion = u32::from_le_bytes(mode[4..8].try_into().expect("4 bytes"));
    // SAFETY: the caller's optional out-pointers.
    unsafe {
        if !state.is_null() {
            state.write(np::handle_state(read_mode, completion));
        }
        if !instances.is_null() {
            instances.write(local.map_or(0, |l| l.current));
        }
    }
    TRUE
}

/// `SetNamedPipeHandleState`.
#[unsafe(no_mangle)]
pub unsafe extern "system" fn SetNamedPipeHandleState(
    pipe: Handle,
    mode: *const u32,
    max_collection: *const u32,
    collect_timeout: *const u32,
) -> i32 {
    let mut cur = [0u8; 8];
    if let Err(st) = query(pipe, FILE_PIPE_INFORMATION, &mut cur) {
        return fail_status(st, FALSE);
    }
    if !max_collection.is_null() || !collect_timeout.is_null() {
        return fail(ERROR_INVALID_PARAMETER, FALSE);
    }
    if mode.is_null() {
        return TRUE;
    }
    // SAFETY: the caller's pointer.
    let info = match np::set_state(unsafe { mode.read() }) {
        Ok(i) => i,
        Err(e) => return fail(e, FALSE),
    };
    let mut iosb = IoStatusBlock::default();
    // SAFETY: valid pointers.
    let st = unsafe { nt::NtSetInformationFile(pipe, &mut iosb, info.as_ptr(), 8, FILE_PIPE_INFORMATION) };
    if st.is_success() { TRUE } else { fail_status(st, FALSE) }
}

/// `TransactNamedPipe`: `FSCTL_PIPE_TRANSCEIVE`.
#[unsafe(no_mangle)]
pub unsafe extern "system" fn TransactNamedPipe(
    pipe: Handle,
    input: *const u8,
    input_size: u32,
    output: *mut u8,
    output_size: u32,
    read: *mut u32,
    ov: *mut Overlapped,
) -> i32 {
    // SAFETY: the caller's buffers.
    let (inp, out) = unsafe {
        (
            if input.is_null() { &[][..] } else { core::slice::from_raw_parts(input, input_size as usize) },
            if output.is_null() { &mut [][..] } else { core::slice::from_raw_parts_mut(output, output_size as usize) },
        )
    };
    // SAFETY: the caller's OVERLAPPED.
    let (st, info) = unsafe { fsctl(pipe, FSCTL_PIPE_TRANSCEIVE, inp, out, ov) };
    if st == nt::STATUS_PENDING {
        return fail(ERROR_IO_PENDING, FALSE);
    }
    if !read.is_null() {
        // SAFETY: the caller's out-pointer.
        unsafe { read.write(info as u32) };
    }
    if st.is_success() { TRUE } else { fail_status(st, FALSE) }
}

/// `WaitNamedPipeW`: `FSCTL_PIPE_WAIT` sent to the pipe directory.
#[unsafe(no_mangle)]
pub unsafe extern "system" fn WaitNamedPipeW(name: *const u16, timeout: u32) -> i32 {
    // SAFETY: the caller's string.
    let Some(name) = (unsafe { wide_arg(name) }) else { return fail(ERROR_INVALID_PARAMETER, FALSE) };
    let rest = match np::pipe_name(name) {
        Ok(r) => r,
        Err(ERROR_INVALID_NAME) => return fail(ERROR_INVALID_PARAMETER, FALSE),
        Err(e) => return fail(e, FALSE),
    };
    let root = NtName::new("\\??\\pipe\\".encode_utf16().collect());
    let oa = root.attributes();
    let mut h: Handle = 0;
    let mut iosb = IoStatusBlock::default();
    // SAFETY: valid pointers.
    let st = unsafe {
        nt::NtOpenFile(&mut h, FILE_READ_ATTRIBUTES | SYNCHRONIZE, &oa, &mut iosb, 3, FILE_SYNCHRONOUS_IO_NONALERT)
    };
    if !st.is_success() {
        return fail_status(st, FALSE);
    }
    let input = np::wait_input(rest, timeout);
    // SAFETY: no OVERLAPPED.
    let (st, _) = unsafe { fsctl(h, FSCTL_PIPE_WAIT, &input, &mut [], core::ptr::null_mut()) };
    close(h);
    if st.is_success() { TRUE } else { fail_status(st, FALSE) }
}

/// `CallNamedPipeW`: open (waiting for a free instance when busy), switch to
/// message mode, transact, close.
#[unsafe(no_mangle)]
pub unsafe extern "system" fn CallNamedPipeW(
    name: *const u16,
    input: *const u8,
    input_size: u32,
    output: *mut u8,
    output_size: u32,
    read: *mut u32,
    timeout: u32,
) -> i32 {
    let pipe = loop {
        // SAFETY: the caller's string.
        let h = unsafe {
            crate::file::CreateFileW(name, GENERIC_READ | GENERIC_WRITE, 0, core::ptr::null(), OPEN_EXISTING, 0, 0)
        };
        if h != INVALID_HANDLE_VALUE {
            break h;
        }
        if last_error() != ERROR_PIPE_BUSY {
            return FALSE;
        }
        // SAFETY: the caller's string.
        if unsafe { WaitNamedPipeW(name, timeout) } == FALSE {
            // Learn: a pipe that stays busy is ERROR_PIPE_BUSY.
            if last_error() == ERROR_SEM_TIMEOUT {
                set_last_error(ERROR_PIPE_BUSY);
            }
            return FALSE;
        }
    };
    let message = np::PIPE_READMODE_MESSAGE;
    // SAFETY: a valid handle and pointer; a byte pipe refuses and is used as it is.
    unsafe { SetNamedPipeHandleState(pipe, &message, core::ptr::null(), core::ptr::null()) };
    // SAFETY: the caller's buffers.
    let ok = unsafe { TransactNamedPipe(pipe, input, input_size, output, output_size, read, core::ptr::null_mut()) };
    let error = last_error();
    close(pipe);
    set_last_error(error);
    ok
}

/// The ids the pipe's two ends have: `(client process, server process,
/// flags)`; flags bit 0 is set when a client connected.
fn processes(pipe: Handle) -> Result<(u32, u32, u32), NtStatus> {
    let mut out = [0u8; 20];
    // SAFETY: no OVERLAPPED.
    let (st, _) = unsafe { fsctl(pipe, FSCTL_POLY_PIPE_PROCESSES, &[], &mut out, core::ptr::null_mut()) };
    if !st.is_success() {
        return Err(st);
    }
    let d = |i: usize| u32::from_le_bytes(out[i * 4..i * 4 + 4].try_into().expect("4 bytes"));
    Ok((d(0), d(1), d(4)))
}

fn put(out: *mut u32, v: u32) -> i32 {
    // SAFETY: the caller's out-pointer.
    unsafe { out.write(v) };
    TRUE
}

/// `GetNamedPipeClientProcessId`: `ERROR_NOT_FOUND` for an instance with no client.
#[unsafe(no_mangle)]
pub unsafe extern "system" fn GetNamedPipeClientProcessId(pipe: Handle, pid: *mut u32) -> i32 {
    match processes(pipe) {
        Ok((client, _, flags)) if flags & 1 != 0 => put(pid, client),
        Ok(_) => fail(ERROR_NOT_FOUND, FALSE),
        Err(st) => fail_status(st, FALSE),
    }
}

/// `GetNamedPipeServerProcessId`.
#[unsafe(no_mangle)]
pub unsafe extern "system" fn GetNamedPipeServerProcessId(pipe: Handle, pid: *mut u32) -> i32 {
    match processes(pipe) {
        Ok((_, server, _)) => put(pid, server),
        Err(st) => fail_status(st, FALSE),
    }
}

/// `GetNamedPipeClientSessionId`: Poly has the one session, 0.
#[unsafe(no_mangle)]
pub unsafe extern "system" fn GetNamedPipeClientSessionId(pipe: Handle, id: *mut u32) -> i32 {
    match processes(pipe) {
        Ok((_, _, flags)) if flags & 1 != 0 => put(id, 0),
        Ok(_) => fail(ERROR_NOT_FOUND, FALSE),
        Err(st) => fail_status(st, FALSE),
    }
}

/// `GetNamedPipeServerSessionId`.
#[unsafe(no_mangle)]
pub unsafe extern "system" fn GetNamedPipeServerSessionId(pipe: Handle, id: *mut u32) -> i32 {
    match processes(pipe) {
        Ok(_) => put(id, 0),
        Err(st) => fail_status(st, FALSE),
    }
}

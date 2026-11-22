//! Endpoints on `\Device\Afd` and the device's requests (ADR 0095,
//! `poly_winsock::afd`), through our `ntdll`.

use crate::Handle;
use crate::lock::{Held, Locked};
use core::sync::atomic::{AtomicIsize, Ordering};
use poly_winsock::afd;
use poly_winsock::error::{STATUS_PENDING, STATUS_SUCCESS};
use poly_winsock::known::Known;

#[cfg_attr(target_arch = "x86", link(name = "ntdll", kind = "raw-dylib", import_name_type = "undecorated"))]
#[cfg_attr(not(target_arch = "x86"), link(name = "ntdll", kind = "raw-dylib"))]
unsafe extern "system" {
    fn NtCreateFile(
        handle: *mut Handle,
        access: u32,
        attributes: *const ObjectAttributes,
        iosb: *mut IoStatusBlock,
        allocation: *const i64,
        file_attributes: u32,
        share: u32,
        disposition: u32,
        options: u32,
        ea: *const u8,
        ea_len: u32,
    ) -> u32;
    fn NtDeviceIoControlFile(
        handle: Handle,
        event: Handle,
        apc: *const u8,
        apc_context: *const u8,
        iosb: *mut IoStatusBlock,
        code: u32,
        input: *const u8,
        input_len: u32,
        output: *mut u8,
        output_len: u32,
    ) -> u32;
    pub fn NtClose(handle: Handle) -> u32;
    fn NtQueryEvent(handle: Handle, class: u32, info: *mut u8, len: u32, returned: *mut u32) -> u32;
}

/// `IO_STATUS_BLOCK` (`wdm.h`): the status in a pointer-sized slot, then
/// `Information`. An `OVERLAPPED`'s `Internal` and `InternalHigh` have the
/// same places (Learn: "OVERLAPPED").
#[repr(C)]
#[derive(Debug, Default)]
pub struct IoStatusBlock {
    pub status: usize,
    pub information: usize,
}

/// `UNICODE_STRING` (`ntdef.h`).
#[repr(C)]
struct UnicodeString {
    length: u16,
    maximum_length: u16,
    buffer: *const u16,
}

/// `OBJECT_ATTRIBUTES` (`ntdef.h`).
#[repr(C)]
struct ObjectAttributes {
    length: u32,
    root: Handle,
    name: *const UnicodeString,
    attributes: u32,
    security: *const u8,
    qos: *const u8,
}

/// `\Device\Afd\Endpoint` in UTF-16.
const ENDPOINT: [u16; 20] = {
    let s = afd::ENDPOINT.as_bytes();
    let mut w = [0u16; 20];
    let mut i = 0;
    while i < 20 {
        w[i] = s[i] as u16;
        i += 1;
    }
    w
};

const GENERIC_READ: u32 = 0x8000_0000;
const GENERIC_WRITE: u32 = 0x4000_0000;
const SYNCHRONIZE: u32 = 0x0010_0000;
const FILE_SHARE_READ_WRITE: u32 = 3;
const FILE_OPEN_IF: u32 = 3;
const OBJ_CASE_INSENSITIVE: u32 = 0x40;
/// `OBJ_INHERIT` (`ntdef.h`): the handle is inheritable (Winsock's sockets are, unless
/// `WSA_FLAG_NO_HANDLE_INHERIT` says not).
const OBJ_INHERIT: u32 = 0x02;

/// The sockets this process made that `closesocket` has not closed (a `CloseHandle` leaves them).
static KNOWN: Locked<Known> = Locked::new(Known::new());

fn with_known<R>(f: impl FnOnce(&mut Known) -> R) -> R {
    KNOWN.with(f)
}

/// `closesocket` of a value that is no socket any more: it was a socket of this process whose
/// handle was closed some other way (the sheet of `WSASocketW`, rule `handle`). True once.
pub fn forget_orphan(h: Handle) -> bool {
    with_known(|k| k.take(h as usize))
}

/// `SO_ERROR` takes the stack's pending error and keeps it on the endpoint in two steps: two
/// threads reading it at once (the event helper and the program) must not interleave, or the one
/// that comes second reads 0 while the first has not stored the error yet (a refused connect
/// seen as `FD_CONNECT`, then `SO_ERROR` 0).
static SO_ERROR_LOCK: Locked<()> = Locked::new(());

/// The `SO_ERROR` lock, held.
pub type SoErrorGuard = Held<'static, ()>;

/// Wait for and take the `SO_ERROR` lock.
pub fn so_error_guard() -> SoErrorGuard {
    SO_ERROR_LOCK.hold()
}

/// Is `h` an event object (`NtQueryEvent(EventBasicInformation)` answers)?
pub fn is_event(h: Handle) -> bool {
    let mut info = [0u8; 8];
    let mut ret = 0u32;
    // SAFETY: the buffers are live locals of the lengths given.
    unsafe { NtQueryEvent(h, 0, info.as_mut_ptr(), info.len() as u32, &mut ret) == STATUS_SUCCESS }
}

/// Open a new endpoint (no socket yet); `inherit`: the handle is inheritable by child processes.
pub fn open_endpoint(inherit: bool) -> Result<Handle, u32> {
    const _: () = assert!(afd::ENDPOINT.len() == 20);
    let name = UnicodeString { length: 40, maximum_length: 40, buffer: ENDPOINT.as_ptr() };
    let oa = ObjectAttributes {
        length: core::mem::size_of::<ObjectAttributes>() as u32,
        root: 0,
        name: &name,
        attributes: OBJ_CASE_INSENSITIVE | if inherit { OBJ_INHERIT } else { 0 },
        security: core::ptr::null(),
        qos: core::ptr::null(),
    };
    let mut h: Handle = 0;
    let mut iosb = IoStatusBlock::default();
    // SAFETY: every pointer is to a live local.
    let s = unsafe {
        NtCreateFile(
            &mut h,
            GENERIC_READ | GENERIC_WRITE | SYNCHRONIZE,
            &oa,
            &mut iosb,
            core::ptr::null(),
            0,
            FILE_SHARE_READ_WRITE,
            FILE_OPEN_IF,
            0,
            core::ptr::null(),
            0,
        )
    };
    if s != STATUS_SUCCESS {
        return Err(s);
    }
    with_known(|k| k.add(h as usize));
    Ok(h)
}

/// Close a handle.
pub fn close(h: Handle) {
    with_known(|k| {
        k.take(h as usize);
    });
    // SAFETY: closing a handle has no memory preconditions.
    unsafe { NtClose(h) };
}

/// A synchronous request: its status (a warning such as
/// `STATUS_BUFFER_OVERFLOW` included) and `Information`.
pub fn ioctl(h: Handle, code: u32, input: &[u8], output: &mut [u8]) -> (u32, usize) {
    let mut iosb = IoStatusBlock::default();
    // SAFETY: the buffers are live slices of the given lengths.
    let s = unsafe {
        NtDeviceIoControlFile(
            h,
            0,
            core::ptr::null(),
            core::ptr::null(),
            &mut iosb,
            code,
            input.as_ptr(),
            input.len() as u32,
            output.as_mut_ptr(),
            output.len() as u32,
        )
    };
    if s == STATUS_PENDING {
        // Only overlapped requests pend; wait for the status block anyway.
        // SAFETY: waiting on our own handle.
        unsafe { crate::WaitForSingleObjectEx(h, u32::MAX, 0) };
        return (iosb.status as u32, iosb.information);
    }
    // Success and warnings fill the status block; errors do not.
    let info = if s < 0xC000_0000 { iosb.information } else { 0 };
    (s, info)
}

/// An overlapped request: completes through `event`, the status block at
/// `iosb` (an `OVERLAPPED`) and the APC.
#[allow(clippy::too_many_arguments)]
pub unsafe fn ioctl_overlapped(
    h: Handle,
    event: Handle,
    apc: *const u8,
    apc_context: *const u8,
    iosb: *mut IoStatusBlock,
    code: u32,
    input: &[u8],
    output: &mut [u8],
) -> u32 {
    // SAFETY: the caller's OVERLAPPED outlives the request (Learn: it
    // must stay valid until the operation completes).
    unsafe {
        NtDeviceIoControlFile(
            h,
            event,
            apc,
            apc_context,
            iosb,
            code,
            input.as_ptr(),
            input.len() as u32,
            output.as_mut_ptr(),
            output.len() as u32,
        )
    }
}

/// A process-wide endpoint for the requests that need no socket of their
/// own (`POLL`, `NET_INFO`), opened at first use.
static CONTROL: AtomicIsize = AtomicIsize::new(0);

pub fn control() -> Result<Handle, u32> {
    let h = CONTROL.load(Ordering::SeqCst);
    if h != 0 {
        return Ok(h);
    }
    let new = open_endpoint(false)?;
    match CONTROL.compare_exchange(0, new, Ordering::SeqCst, Ordering::SeqCst) {
        Ok(_) => Ok(new),
        Err(other) => {
            close(new);
            Ok(other)
        }
    }
}

/// What the device says about the network (DNS servers, host name).
pub fn net_info() -> afd::NetInfo {
    let mut out = [0u8; afd::NetInfo::SIZE];
    let fallback = afd::NetInfo::new(&[], "poly");
    let Ok(h) = control() else { return fallback };
    match ioctl(h, afd::NET_INFO, &[], &mut out) {
        (STATUS_SUCCESS, _) => afd::NetInfo::decode(&out).unwrap_or(fallback),
        _ => fallback,
    }
}

/// The machine's interface addresses (`(is_loopback, ip)`) from `\Device\Afd`'s `IF_LIST`.
pub fn local_addrs() -> alloc::vec::Vec<(bool, core::net::IpAddr)> {
    let mut out = [0u8; afd::IF_LIST_MAX_BYTES];
    let Ok(h) = control() else { return alloc::vec::Vec::new() };
    match ioctl(h, afd::IF_LIST, &[], &mut out) {
        (STATUS_SUCCESS, n) => afd::decode_if_addrs(&out[..n.min(out.len())]),
        _ => alloc::vec::Vec::new(),
    }
}

/// What an endpoint is (`None` if `h` is not a socket).
pub fn info(h: Handle) -> Option<afd::Info> {
    let mut out = [0u8; afd::Info::SIZE];
    match ioctl(h, afd::INFO, &[], &mut out) {
        (STATUS_SUCCESS, _) => afd::Info::decode(&out).filter(|i| i.family != 0),
        _ => None,
    }
}

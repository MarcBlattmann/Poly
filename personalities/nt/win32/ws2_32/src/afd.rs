//! Endpoints on `\Device\Afd` and the device's requests (ADR 0095,
//! `poly_winsock::afd`), through our `ntdll`.

use crate::Handle;
use core::sync::atomic::{AtomicIsize, Ordering};
use poly_winsock::afd;
use poly_winsock::error::{STATUS_PENDING, STATUS_SUCCESS};

#[link(name = "ntdll", kind = "raw-dylib")]
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

/// Open a new endpoint (no socket yet).
pub fn open_endpoint() -> Result<Handle, u32> {
    const _: () = assert!(afd::ENDPOINT.len() == 20);
    let name = UnicodeString { length: 40, maximum_length: 40, buffer: ENDPOINT.as_ptr() };
    let oa = ObjectAttributes {
        length: core::mem::size_of::<ObjectAttributes>() as u32,
        root: 0,
        name: &name,
        attributes: OBJ_CASE_INSENSITIVE,
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
    Ok(h)
}

/// Close a handle.
pub fn close(h: Handle) {
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
    let new = open_endpoint()?;
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

/// What an endpoint is (`None` if `h` is not a socket).
pub fn info(h: Handle) -> Option<afd::Info> {
    let mut out = [0u8; afd::Info::SIZE];
    match ioctl(h, afd::INFO, &[], &mut out) {
        (STATUS_SUCCESS, _) => afd::Info::decode(&out).filter(|i| i.family != 0),
        _ => None,
    }
}

//! `\Device\PolyAudio` (ADR 0244) for the engine: a stream handle, its requests, and whether the
//! machine has a sound card.

use core::ffi::c_void;
use core::ptr::{null, null_mut};
use core::sync::atomic::{AtomicU8, Ordering};
use poly_audio::Format;
use poly_audio::nt::{self as wire, DeviceInfo, OpenReq, StreamInfo};

pub type Handle = *mut c_void;

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
}

/// `IO_STATUS_BLOCK` (`wdm.h`).
#[repr(C)]
#[derive(Debug, Default)]
pub struct IoStatusBlock {
    pub status: usize,
    pub information: usize,
}

#[repr(C)]
struct UnicodeString {
    length: u16,
    maximum_length: u16,
    buffer: *const u16,
}

#[repr(C)]
struct ObjectAttributes {
    length: u32,
    root: Handle,
    name: *const UnicodeString,
    attributes: u32,
    security: *const u8,
    qos: *const u8,
}

pub const STATUS_SUCCESS: u32 = 0;
const SYNCHRONIZE: u32 = 0x0010_0000;
const FILE_READ_ATTRIBUTES: u32 = 0x80;
const FILE_OPEN: u32 = 1;
const OBJ_CASE_INSENSITIVE: u32 = 0x40;

const DEVICE_NAME: [u16; 17] = {
    let s = wire::DEVICE_NAME.as_bytes();
    let mut w = [0u16; 17];
    let mut i = 0;
    while i < 17 {
        w[i] = s[i] as u16;
        i += 1;
    }
    w
};

/// A handle on the device. It asks for no access to the
/// device's data: its requests need none (`FILE_ANY_ACCESS`), and a sandboxed caller's token
/// would be refused write access to a device it has no label for.
pub fn open() -> Option<Handle> {
    let name = UnicodeString { length: 34, maximum_length: 34, buffer: DEVICE_NAME.as_ptr() };
    let attrs = ObjectAttributes {
        length: core::mem::size_of::<ObjectAttributes>() as u32,
        root: null_mut(),
        name: &name,
        attributes: OBJ_CASE_INSENSITIVE,
        security: null(),
        qos: null(),
    };
    let mut h: Handle = null_mut();
    let mut iosb = IoStatusBlock::default();
    // SAFETY: every pointer is to a live local; no EA buffer.
    let s = unsafe {
        NtCreateFile(
            &mut h,
            FILE_READ_ATTRIBUTES | SYNCHRONIZE,
            &attrs,
            &mut iosb,
            null(),
            0,
            3,
            FILE_OPEN,
            0,
            null(),
            0,
        )
    };
    (s == STATUS_SUCCESS).then_some(h)
}

/// A request that finishes at once: its status and `Information`.
pub fn ioctl(h: Handle, code: u32, input: &[u8], out: &mut [u8]) -> (u32, usize) {
    let mut iosb = IoStatusBlock::default();
    // SAFETY: the buffers and the status block outlive the call; these requests never pend.
    let s = unsafe {
        NtDeviceIoControlFile(
            h,
            null_mut(),
            null(),
            null(),
            &mut iosb,
            code,
            input.as_ptr(),
            input.len() as u32,
            out.as_mut_ptr(),
            out.len() as u32,
        )
    };
    (s, iosb.information)
}

pub fn query(h: Handle) -> StreamInfo {
    let mut out = [0u8; StreamInfo::SIZE];
    ioctl(h, wire::QUERY, &[], &mut out);
    StreamInfo::decode(&out).unwrap_or_default()
}

/// A stream handle of `format` with a buffer of `frames`.
pub fn open_stream(format: Format, frames: u32) -> Option<Handle> {
    let h = open()?;
    let req = OpenReq { format, buffer_frames: frames }.encode();
    if ioctl(h, wire::OPEN, &req, &mut []).0 != STATUS_SUCCESS {
        // SAFETY: our handle.
        unsafe { NtClose(h) };
        return None;
    }
    Some(h)
}

/// Whether the machine has a sound card (asked once).
pub fn present() -> bool {
    static PRESENT: AtomicU8 = AtomicU8::new(0);
    match PRESENT.load(Ordering::SeqCst) {
        1 => return true,
        2 => return false,
        _ => {}
    }
    let yes = open().is_some_and(|h| {
        let mut out = [0u8; DeviceInfo::SIZE];
        let ok = ioctl(h, wire::DEVICE, &[], &mut out).0 == STATUS_SUCCESS;
        // SAFETY: our handle.
        unsafe { NtClose(h) };
        ok
    });
    PRESENT.store(if yes { 1 } else { 2 }, Ordering::SeqCst);
    yes
}

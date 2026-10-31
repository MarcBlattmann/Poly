//! `CreatePipe` (Learn: "CreatePipe", "Anonymous Pipe Operations"), ADR
//! 0093: two opens of a new `\Device\PolyPipe` stream
//! (`poly_kernelbase::pipe`), the read end and the write end, both
//! inheritable when the security attributes say so.

use crate::base::*;
use crate::nt::{self, Handle, IoStatusBlock};
use core::sync::atomic::{AtomicU32, Ordering};
use poly_kernelbase::error::*;
use poly_kernelbase::file::{FILE_CREATE, FILE_OPEN, FILE_SYNCHRONOUS_IO_NONALERT};
use poly_kernelbase::pipe;

/// `FILE_SHARE_READ | FILE_SHARE_WRITE` (`winnt.h`).
const SHARE_READ_WRITE: u32 = 3;

static SERIAL: AtomicU32 = AtomicU32::new(0);

/// One end: `name` opened (or created) with `access`.
fn open_end(name: &NtName, access: u32, disposition: u32, inherit: bool) -> Result<Handle, u32> {
    let mut oa = name.attributes();
    if inherit {
        oa.attributes |= nt::OBJ_INHERIT;
    }
    let mut h: Handle = 0;
    let mut iosb = IoStatusBlock::default();
    // SAFETY: valid pointers.
    let st = unsafe {
        nt::NtCreateFile(
            &mut h,
            access,
            &oa,
            &mut iosb,
            core::ptr::null(),
            0,
            SHARE_READ_WRITE,
            disposition,
            FILE_SYNCHRONOUS_IO_NONALERT,
            core::ptr::null(),
            0,
        )
    };
    if st.is_success() { Ok(h) } else { Err(from_status(st)) }
}

/// `CreatePipe(&read, &write, attributes, size)`.
///
/// # Safety
/// `read` and `write` are writable; `attributes` is null or a
/// `SECURITY_ATTRIBUTES`.
#[unsafe(no_mangle)]
pub unsafe extern "system" fn CreatePipe(
    read: *mut Handle,
    write: *mut Handle,
    attributes: *const u8,
    size: u32,
) -> i32 {
    let _ = size;
    if read.is_null() || write.is_null() {
        return fail(ERROR_INVALID_PARAMETER, FALSE);
    }
    // SECURITY_ATTRIBUTES.bInheritHandle (minwinbase.h: at 16 on x64).
    // SAFETY: the caller's structure when non-null.
    let inherit = !attributes.is_null() && unsafe { (attributes.add(16) as *const i32).read_unaligned() } != 0;
    let pid = crate::process::GetCurrentProcessId();
    let serial = SERIAL.fetch_add(1, Ordering::Relaxed);
    let name = NtName::new(pipe::nt_name(pid, serial));
    let r = match open_end(&name, pipe::READ_ACCESS, FILE_CREATE, inherit) {
        Ok(h) => h,
        Err(e) => return fail(e, FALSE),
    };
    let w = match open_end(&name, pipe::WRITE_ACCESS, FILE_OPEN, inherit) {
        Ok(h) => h,
        Err(e) => {
            // SAFETY: the handle just opened.
            unsafe { nt::NtClose(r) };
            return fail(e, FALSE);
        }
    };
    // SAFETY: the caller's out-pointers (checked).
    unsafe {
        read.write(r);
        write.write(w);
    }
    set_last_error(ERROR_SUCCESS);
    TRUE
}

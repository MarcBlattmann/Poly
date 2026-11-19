//! `CreateProcessW`'s User Account Control check (ADR 0222, `poly_kernelbase::elevation`):
//! an image whose manifest requests `requireAdministrator`, or `highestAvailable` for an
//! administrator, is not started for a token that is not elevated (`ERROR_ELEVATION_REQUIRED`).
//! `CreateProcessW` never asks the user; `ShellExecuteEx` does (our `shell32`).

use crate::nt::{self, Handle, IoStatusBlock};
use alloc::vec::Vec;
use poly_kernelbase::elevation::{self, Level};
use poly_ntobj::NtStatus;

#[cfg_attr(target_arch = "x86", link(name = "ntdll", kind = "raw-dylib", import_name_type = "undecorated"))]
#[cfg_attr(not(target_arch = "x86"), link(name = "ntdll", kind = "raw-dylib"))]
unsafe extern "system" {
    fn NtQueryInformationToken(token: Handle, class: u32, info: *mut u8, length: u32, ret: *mut u32) -> NtStatus;
}

/// `GetCurrentProcessToken()` (`processthreadsapi.h`): the pseudo-handle of the process token.
const CURRENT_PROCESS_TOKEN: Handle = -4;
/// `TokenElevationType`, `TokenElevation` (`winnt.h`).
const TOKEN_ELEVATION_TYPE: u32 = 18;
const TOKEN_ELEVATION: u32 = 20;
/// `TokenElevationTypeLimited`: the filtered token of an administrator.
const TOKEN_ELEVATION_TYPE_LIMITED: u32 = 3;
/// `FILE_READ_DATA`, `FILE_NON_DIRECTORY_FILE` (`winnt.h`, `winternl.h`).
const FILE_READ_DATA: u32 = 0x1;
const FILE_NON_DIRECTORY_FILE: u32 = 0x40;
/// `STATUS_END_OF_FILE` (`ntstatus.h`).
const STATUS_END_OF_FILE: u32 = 0xC000_0011;

fn token_u32(token: Handle, class: u32) -> Option<u32> {
    let mut v = [0u8; 4];
    let mut ret = 0;
    // SAFETY: a 4-byte buffer and a local length.
    let st = unsafe { NtQueryInformationToken(token, class, v.as_mut_ptr(), 4, &mut ret) };
    st.is_success().then(|| u32::from_le_bytes(v))
}

/// Whether `token` (the process's when `None`) is elevated, and whether its user is an
/// administrator who could elevate (a filtered token).
pub(crate) fn token_elevation(token: Option<Handle>) -> (bool, bool) {
    let t = token.unwrap_or(CURRENT_PROCESS_TOKEN);
    let elevated = token_u32(t, TOKEN_ELEVATION).is_none_or(|e| e != 0);
    let admin = token_u32(t, TOKEN_ELEVATION_TYPE) == Some(TOKEN_ELEVATION_TYPE_LIMITED);
    (elevated, admin)
}

/// The file `win32` (a full DOS path) read through `read`; `None` if it cannot be opened.
fn with_file<R>(win32: &[u16], f: impl FnOnce(elevation::ReadAt<'_>) -> R) -> Option<R> {
    let h = crate::file::open_for(win32, FILE_READ_DATA, FILE_NON_DIRECTORY_FILE).ok()?;
    let mut read = |at: u64, buf: &mut [u8]| -> Result<usize, u32> {
        let mut iosb = IoStatusBlock::default();
        let offset = at as i64;
        // SAFETY: `buf` is writable for its length; the handle was opened for synchronous I/O.
        let st = unsafe {
            nt::NtReadFile(
                h,
                0,
                core::ptr::null(),
                core::ptr::null(),
                &mut iosb,
                buf.as_mut_ptr(),
                buf.len() as u32,
                &offset,
                core::ptr::null(),
            )
        };
        if st.0 == STATUS_END_OF_FILE {
            return Ok(0);
        }
        if !st.is_success() {
            return Err(1);
        }
        Ok(iosb.information)
    };
    let r = f(&mut read);
    crate::base::close(h);
    Some(r)
}

/// The execution level the image at `image` requests: its embedded manifest, else
/// `<image>.manifest` beside it.
pub(crate) fn image_level(image: &[u16]) -> Level {
    let external_path = elevation::external_manifest_path(image);
    let external: Option<Vec<u8>> = with_file(&external_path, |read| {
        let mut buf = alloc::vec![0u8; elevation::MAX_EXTERNAL_MANIFEST];
        let mut done = 0;
        while done < buf.len() {
            match read(done as u64, &mut buf[done..]) {
                Ok(0) | Err(_) => break,
                Ok(n) => done += n,
            }
        }
        buf.truncate(done);
        buf
    });
    with_file(image, |read| elevation::image_level(read, external.as_deref())).unwrap_or(Level::AsInvoker)
}

/// Whether `CreateProcessW` of `image` with `token` (the creator's when `None`) must fail with
/// `ERROR_ELEVATION_REQUIRED`.
pub(crate) fn elevation_required(image: &[u16], token: Option<Handle>) -> bool {
    let (elevated, admin) = token_elevation(token);
    // An elevated token starts everything: no file is read.
    !elevated && elevation::needs_elevation(image_level(image), elevated, admin)
}

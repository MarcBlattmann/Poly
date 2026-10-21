//! What `ntdll` reads from the process parameters the kernel wrote (ADR
//! 0073): `PEB.ProcessParameters` (PEB + 0x20, `winternl.h`), its
//! `ImagePathName` (+ 0x60, `winternl.h`) and Poly's own environment block
//! pointer and size (+ 0x80, + 0x88, `personalities/nt/src/params.rs`).
//! From them the loader gets the executable's DOS path and the directories
//! of the DLL search order (ADR 0071).

use alloc::vec::Vec;
use poly_ldr::path::{Dirs, dir_name, wide};

const PEB_PROCESS_PARAMETERS: u64 = 0x20;
const PP_IMAGE_PATH_NAME: u64 = 0x60;
const PP_ENVIRONMENT: u64 = 0x80;
const PP_ENVIRONMENT_SIZE: u64 = 0x88;

/// The name an executable gets when the kernel gave no parameters.
const UNNAMED: &str = "C:\\Windows\\System32\\poly-image.exe";

/// The process parameters' address (0: none).
fn parameters(peb: u64) -> u64 {
    if peb == 0 {
        return 0;
    }
    // SAFETY: the kernel made the PEB readable; the field is a pointer.
    unsafe { ((peb + PEB_PROCESS_PARAMETERS) as *const u64).read() }
}

/// A `UNICODE_STRING` (`ntdef.h`: `Length` in bytes at + 0, `Buffer` at
/// + 8) as a vector.
///
/// # Safety
/// `at` is a readable `UNICODE_STRING` whose buffer holds `Length` bytes.
unsafe fn unicode_string(at: u64) -> Vec<u16> {
    // SAFETY: the caller's contract.
    unsafe {
        let len = usize::from((at as *const u16).read()) / 2;
        let buf = ((at + 8) as *const *const u16).read();
        if buf.is_null() || len == 0 {
            return Vec::new();
        }
        core::slice::from_raw_parts(buf, len).to_vec()
    }
}

/// The executable's DOS path.
pub fn image_path(peb: u64) -> Vec<u16> {
    let pp = parameters(peb);
    // SAFETY: the kernel wrote a valid RTL_USER_PROCESS_PARAMETERS.
    let path = if pp == 0 { Vec::new() } else { unsafe { unicode_string(pp + PP_IMAGE_PATH_NAME) } };
    if path.is_empty() { wide(UNNAMED) } else { path }
}

/// The value of environment variable `name` (compared without case), if
/// the environment block has it.
pub fn environment_variable(peb: u64, name: &str) -> Option<Vec<u16>> {
    let pp = parameters(peb);
    if pp == 0 {
        return None;
    }
    // SAFETY: the kernel wrote the block's address and size.
    let block: &[u16] = unsafe {
        let at = ((pp + PP_ENVIRONMENT) as *const *const u16).read();
        let bytes = ((pp + PP_ENVIRONMENT_SIZE) as *const u64).read() as usize;
        if at.is_null() {
            return None;
        }
        core::slice::from_raw_parts(at, bytes / 2)
    };
    let want = wide(name);
    for entry in block.split(|&c| c == 0).take_while(|e| !e.is_empty()) {
        // A leading '=' belongs to the name (the drive-current-directory
        // variables), so the separator is the first '=' after it.
        let Some(eq) = entry.iter().skip(1).position(|&c| c == u16::from(b'=')).map(|i| i + 1) else { continue };
        if poly_ntobj::name::eq_ignore_case(&entry[..eq], &want) {
            return Some(entry[eq + 1..].to_vec());
        }
    }
    None
}

/// The search-order directories for this process (Learn: "Dynamic-link
/// library search order"): the executable's directory, the system
/// directories of ADR 0073's layout, the current directory (the
/// executable's until `kernelbase` keeps one) and `Path`.
pub fn dirs(peb: u64) -> Dirs {
    let image = image_path(peb);
    let app = dir_name(&image).to_vec();
    let path = environment_variable(peb, "Path")
        .map(|p| p.split(|&c| c == u16::from(b';')).filter(|d| !d.is_empty()).map(<[u16]>::to_vec).collect())
        .unwrap_or_default();
    Dirs {
        current: app.clone(),
        app,
        system32: wide("C:\\Windows\\System32"),
        system16: wide("C:\\Windows\\System"),
        windows: wide("C:\\Windows"),
        path,
    }
}

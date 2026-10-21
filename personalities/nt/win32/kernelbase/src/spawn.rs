//! Creating processes (Learn's `processthreadsapi.h` pages:
//! `CreateProcessW`, `GetExitCodeProcess`, `GetProcessId`), ADR 0079's
//! second slice, over `NtCreateUserProcess`.
//!
//! `kernelbase` decides which file runs (`poly_kernelbase::process`: the
//! command line's candidates, `.exe`, the search order), builds the
//! child's process parameters (image path, command line, environment,
//! current directory) in its own memory and passes them to
//! `NtCreateUserProcess`, which opens the image and starts the child in
//! our `ntdll`'s `LdrInitializeThunk` like any process. Not supported
//! yet (ADR 0079): `CREATE_SUSPENDED` (`ERROR_NOT_SUPPORTED`), handle
//! inheritance (`bInheritHandles` is ignored: the child gets the console
//! as its standard handles, like every Poly NT process), the security
//! attributes, and `STARTUPINFO`'s fields.

use crate::base::*;
use crate::nt::{self, Handle};
use alloc::vec::Vec;
use poly_kernelbase::error::*;
use poly_kernelbase::process::{self as p, CREATE_SUSPENDED, CREATE_UNICODE_ENVIRONMENT};

/// `FILE_ATTRIBUTE_DIRECTORY` (`winnt.h`).
const FILE_ATTRIBUTE_DIRECTORY: u32 = 0x10;
/// `ProcessBasicInformation`: `ExitStatus` at 0, `UniqueProcessId` at 32
/// (`winternl.h`, 48 bytes).
const PROCESS_BASIC_INFORMATION: u32 = 0;
/// `ThreadBasicInformation`: `ClientId.UniqueThread` at 24.
const THREAD_BASIC_INFORMATION: u32 = 0;
/// The longest environment block read from a caller (Poly's limit).
const MAX_BLOCK: usize = 1 << 20;

/// `path` names a file (not a directory) that exists.
fn is_file(path: &[u16]) -> Result<(), u32> {
    let a = crate::file::attributes_of(path)?;
    if a & FILE_ATTRIBUTE_DIRECTORY != 0 { Err(ERROR_ACCESS_DENIED) } else { Ok(()) }
}

/// The full path of the module a command line names (see the module
/// documentation), or the Win32 error.
fn find_program(cmd: &[u16]) -> Result<Vec<u16>, u32> {
    let image = crate::process::image_path();
    let app_dir = match image.iter().rposition(|&c| c == u16::from(b'\\')) {
        Some(i) => image[..i].to_vec(),
        None => Vec::new(),
    };
    let current = current_dir();
    let path_var = crate::process::env_var(&poly_kernelbase::wide("PATH"));
    let mut last = ERROR_FILE_NOT_FOUND;
    for candidate in p::program_candidates(cmd) {
        let name = p::with_default_extension(&candidate);
        let tries: Vec<Vec<u16>> = if p::has_path(&name) {
            alloc::vec![name]
        } else {
            p::search_dirs(&app_dir, &current, path_var.as_deref()).iter().map(|d| p::join(d, &name)).collect()
        };
        for t in tries {
            let Ok((full, _)) = full_path(&t) else { continue };
            match is_file(&full) {
                Ok(()) => return Ok(full),
                Err(ERROR_FILE_NOT_FOUND | ERROR_PATH_NOT_FOUND) => {}
                Err(e) => last = e,
            }
        }
    }
    Err(last)
}

/// The child's environment block from `lpEnvironment`.
///
/// # Safety
/// `env` is null or a block as `CreateProcessW` documents.
unsafe fn child_environment(env: *const u8, flags: u32) -> Result<Vec<u16>, u32> {
    if env.is_null() {
        return Ok(crate::process::env_block());
    }
    if flags & CREATE_UNICODE_ENVIRONMENT != 0 {
        let w = env as *const u16;
        let mut n = 0;
        // SAFETY: the caller's block, read up to its end.
        while n < MAX_BLOCK && unsafe { !(*w.add(n) == 0 && (n == 0 || *w.add(n - 1) == 0)) } {
            n += 1;
        }
        // SAFETY: as above; `n + 1` units were read.
        let units = unsafe { core::slice::from_raw_parts(w, n + 1) };
        let len = p::wide_block_len(units).ok_or(ERROR_INVALID_PARAMETER)?;
        Ok(p::normalized_block(&units[..len]))
    } else {
        let mut n = 0;
        // SAFETY: as above, for an ANSI block.
        while n < MAX_BLOCK && unsafe { !(*env.add(n) == 0 && (n == 0 || *env.add(n - 1) == 0)) } {
            n += 1;
        }
        // SAFETY: as above.
        let bytes = unsafe { core::slice::from_raw_parts(env, n + 1) };
        let len = p::ansi_block_len(bytes).ok_or(ERROR_INVALID_PARAMETER)?;
        Ok(p::normalized_block(&poly_kernelbase::text::from_ansi(&bytes[..len])))
    }
}

fn basic_information(process: Handle) -> Result<[u8; 48], u32> {
    let mut info = [0u8; 48];
    // SAFETY: valid pointers.
    let st = unsafe {
        nt::NtQueryInformationProcess(process, PROCESS_BASIC_INFORMATION, info.as_mut_ptr(), 48, core::ptr::null_mut())
    };
    if st.is_success() { Ok(info) } else { Err(from_status(st)) }
}

/// `CreateProcessW`.
#[unsafe(no_mangle)]
pub unsafe extern "system" fn CreateProcessW(
    application: *const u16,
    command_line: *mut u16,
    process_attributes: *const u8,
    thread_attributes: *const u8,
    inherit_handles: i32,
    flags: u32,
    environment: *const u8,
    current_directory: *const u16,
    startup_info: *const u8,
    information: *mut u8,
) -> i32 {
    let _ = (process_attributes, thread_attributes, inherit_handles, startup_info);
    // SAFETY: the caller's strings.
    let (app, cmd) = unsafe { (wide_arg(application), wide_arg(command_line)) };
    if information.is_null() || (app.is_none() && cmd.is_none()) {
        return fail(ERROR_INVALID_PARAMETER, FALSE);
    }
    if flags & CREATE_SUSPENDED != 0 {
        return fail(ERROR_NOT_SUPPORTED, FALSE);
    }
    let image = match app {
        Some(a) => full_path(a).map(|(f, _)| f).and_then(|f| is_file(&f).map(|()| f)),
        None => find_program(cmd.unwrap_or(&[])),
    };
    let image = match image {
        Ok(i) => i,
        Err(e) => return fail(e, FALSE),
    };
    // Learn: without a command line, the application name is it.
    let line: Vec<u16> = cmd.or(app).unwrap_or(&[]).to_vec();
    // SAFETY: the caller's block.
    let env = match unsafe { child_environment(environment, flags) } {
        Ok(e) => e,
        Err(e) => return fail(e, FALSE),
    };
    // SAFETY: the caller's string.
    let cwd = match unsafe { wide_arg(current_directory) } {
        Some(d) => match full_path(d) {
            Ok((f, _)) => f,
            Err(e) => return fail(e, FALSE),
        },
        None => current_dir(),
    };
    // The parameters, laid out where they will stay during the call.
    let size = match p::parameters(0, &image, &line, &env, &cwd) {
        Ok(b) => b.len(),
        Err(e) => return fail(e, FALSE),
    };
    let mut buf: Vec<u8> = alloc::vec![0u8; size];
    let base = buf.as_mut_ptr() as u64;
    match p::parameters(base, &image, &line, &env, &cwd) {
        Ok(b) => buf.copy_from_slice(&b),
        Err(e) => return fail(e, FALSE),
    }
    let (mut ph, mut th): (Handle, Handle) = (0, 0);
    // SAFETY: valid pointers; the kernel reads the parameters.
    let st = unsafe {
        nt::NtCreateUserProcess(
            &mut ph,
            &mut th,
            0,
            0,
            core::ptr::null(),
            core::ptr::null(),
            0,
            0,
            buf.as_ptr(),
            core::ptr::null_mut(),
            core::ptr::null(),
        )
    };
    if !st.is_success() {
        return fail_status(st, FALSE);
    }
    let pid = basic_information(ph).map_or(0, |i| u64::from_le_bytes(i[32..40].try_into().unwrap()) as u32);
    let mut ti = [0u8; 48];
    // SAFETY: valid pointers.
    let q = unsafe {
        nt::NtQueryInformationThread(th, THREAD_BASIC_INFORMATION, ti.as_mut_ptr(), 48, core::ptr::null_mut())
    };
    let tid = if q.is_success() { u64::from_le_bytes(ti[24..32].try_into().unwrap()) as u32 } else { 0 };
    // PROCESS_INFORMATION (`processthreadsapi.h`): hProcess, hThread,
    // dwProcessId, dwThreadId.
    // SAFETY: the caller's 24-byte structure.
    unsafe {
        (information as *mut Handle).write_unaligned(ph);
        (information.add(8) as *mut Handle).write_unaligned(th);
        (information.add(16) as *mut u32).write_unaligned(pid);
        (information.add(20) as *mut u32).write_unaligned(tid);
    }
    TRUE
}

/// `GetExitCodeProcess`: `STILL_ACTIVE` (259) while it runs.
#[unsafe(no_mangle)]
pub unsafe extern "system" fn GetExitCodeProcess(process: Handle, code: *mut u32) -> i32 {
    match basic_information(process) {
        Ok(info) => {
            // SAFETY: the caller's out-pointer.
            unsafe { code.write(u32::from_le_bytes(info[0..4].try_into().unwrap())) };
            TRUE
        }
        Err(e) => fail(e, FALSE),
    }
}

/// `GetProcessId`: 0 with the error on failure.
#[unsafe(no_mangle)]
pub extern "system" fn GetProcessId(process: Handle) -> u32 {
    match basic_information(process) {
        Ok(info) => u64::from_le_bytes(info[32..40].try_into().unwrap()) as u32,
        Err(e) => fail(e, 0),
    }
}

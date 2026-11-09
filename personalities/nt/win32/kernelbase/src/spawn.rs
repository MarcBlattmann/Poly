//! Creating processes (Learn's `processthreadsapi.h` pages:
//! `CreateProcessW`, `GetExitCodeProcess`, `GetProcessId`), ADR 0079's
//! second slice, over `NtCreateUserProcess`.
//!
//! `kernelbase` decides which file runs (`poly_kernelbase::process`: the
//! command line's candidates, `.exe`, the search order), builds the
//! child's process parameters (image path, command line, environment,
//! current directory) in its own memory and passes them to
//! `NtCreateUserProcess`, which opens the image and starts the child in
//! our `ntdll`'s `LdrInitializeThunk` like any process.
//!
//! ADR 0089 adds what was left: `bInheritHandles` (the child's handle
//! table starts with the creator's inheritable handles at the same
//! values), the `STARTUPINFO` (kept in the child's parameters, where
//! `GetStartupInfoW` finds it; with `STARTF_USESTDHANDLES` its three
//! handles are the child's standard handles), `CREATE_SUSPENDED` (the
//! first thread waits for `ResumeThread`) and `CreateProcessA`. The
//! security attributes' descriptors are not used (Poly has one user, ADR
//! 0083); their `bInheritHandle` would make the returned process and
//! thread handles inheritable.

use crate::base::*;
use crate::nt::{self, Handle};
use alloc::vec::Vec;
use poly_kernelbase::error::*;
use poly_kernelbase::process::{self as p, CREATE_SUSPENDED, CREATE_UNICODE_ENVIRONMENT, STARTUPINFO_SIZE};

/// `EXTENDED_STARTUPINFO_PRESENT`, `CREATE_BREAKAWAY_FROM_JOB` (`winbase.h`).
const EXTENDED_STARTUPINFO_PRESENT: u32 = 0x0008_0000;
const CREATE_BREAKAWAY_FROM_JOB: u32 = 0x0100_0000;
/// `sizeof(STARTUPINFOEXW)`: the `STARTUPINFO` and `lpAttributeList` (8 bytes).
const STARTUPINFOEX_SIZE: usize = STARTUPINFO_SIZE + 8;
/// `HANDLE_FLAG_INHERIT` (`winbase.h`).
const HANDLE_FLAG_INHERIT: u32 = 1;

/// The attribute list of a `STARTUPINFOEX` (null if none), or
/// `ERROR_INVALID_PARAMETER` when the flag is given with a structure of the
/// plain size (Learn: "STARTUPINFOEXW").
///
/// # Safety
/// `si` is null or a `STARTUPINFO` as large as its `cb` says.
unsafe fn attribute_list(si: *const u8, flags: u32) -> Result<*const u8, u32> {
    if flags & EXTENDED_STARTUPINFO_PRESENT == 0 || si.is_null() {
        return Ok(core::ptr::null());
    }
    // SAFETY: `cb` is the first field.
    let cb = unsafe { (si as *const u32).read_unaligned() } as usize;
    if cb < STARTUPINFOEX_SIZE {
        return Err(ERROR_INVALID_PARAMETER);
    }
    // SAFETY: `cb` covers `lpAttributeList`.
    Ok(unsafe { (si.add(STARTUPINFO_SIZE) as *const *const u8).read_unaligned() })
}

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

/// `SECURITY_ATTRIBUTES.bInheritHandle` (offset 16 on x64,
/// `minwinbase.h`).
///
/// # Safety
/// `sa` is null or a `SECURITY_ATTRIBUTES`.
unsafe fn inherits(sa: *const u8) -> bool {
    // SAFETY: the caller's structure when non-null.
    !sa.is_null() && unsafe { (sa.add(16) as *const i32).read_unaligned() } != 0
}

/// The caller's `STARTUPINFOW` or `STARTUPINFOA` (the same layout,
/// `processthreadsapi.h`): its fixed part, its strings (`string` reads one
/// as UTF-16) and its `lpReserved2` bytes. Null: none given.
///
/// # Safety
/// `si` is null or a `STARTUPINFO` whose pointers are valid.
unsafe fn startup_from(si: *const u8, string: &dyn Fn(*const u8) -> Option<Vec<u16>>) -> Option<p::Startup> {
    if si.is_null() {
        return None;
    }
    let mut raw = [0u8; STARTUPINFO_SIZE];
    // SAFETY: the caller's structure (Learn: `cb` is its size, which is
    // this one's).
    unsafe { core::ptr::copy_nonoverlapping(si, raw.as_mut_ptr(), STARTUPINFO_SIZE) };
    let ptr = |o: usize| u64::from_le_bytes(raw[o..o + 8].try_into().unwrap()) as *const u8;
    let n = usize::from(u16::from_le_bytes([raw[p::SI_CB_RESERVED2], raw[p::SI_CB_RESERVED2 + 1]]));
    let reserved2 = if n > 0 && !ptr(p::SI_RESERVED2).is_null() {
        // SAFETY: `cbReserved2` bytes at `lpReserved2` (Learn).
        unsafe { core::slice::from_raw_parts(ptr(p::SI_RESERVED2), n) }.to_vec()
    } else {
        Vec::new()
    };
    let (desktop, title, reserved) =
        (string(ptr(p::SI_DESKTOP)), string(ptr(p::SI_TITLE)), string(ptr(p::SI_RESERVED)));
    Some(p::Startup::from_raw(&raw, desktop, title, reserved, reserved2))
}

/// A UTF-16 string argument, copied.
fn wide_string(s: *const u8) -> Option<Vec<u16>> {
    // SAFETY: a NUL-terminated string from the caller's STARTUPINFOW.
    unsafe { wide_arg(s as *const u16) }.map(<[u16]>::to_vec)
}

/// An ANSI string argument, in UTF-16.
fn ansi_string(s: *const u8) -> Option<Vec<u16>> {
    if s.is_null() {
        return None;
    }
    // SAFETY: a NUL-terminated string from the caller's STARTUPINFOA.
    let bytes = unsafe { core::slice::from_raw_parts(s, poly_kernelbase::text::strlen(s)) };
    Some(poly_kernelbase::text::from_ansi(bytes))
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
    // SAFETY: the caller's arguments, as Learn documents them.
    unsafe {
        let startup = startup_from(startup_info, &wide_string);
        let attributes = (inherits(process_attributes), inherits(thread_attributes));
        let request = Request {
            app: wide_arg(application),
            cmd: wide_arg(command_line),
            inherit_handles: inherit_handles != 0,
            flags,
            environment,
            current_directory: wide_arg(current_directory),
            attribute_list: match attribute_list(startup_info, flags) {
                Ok(l) => l,
                Err(e) => return fail(e, FALSE),
            },
        };
        create(&request, startup.as_ref(), attributes, information)
    }
}

/// `CreateProcessA`: the strings (and `STARTUPINFOA`'s) from the ANSI
/// code page, then as `CreateProcessW`. An environment block stays as it
/// is (ANSI unless `CREATE_UNICODE_ENVIRONMENT`, Learn).
#[unsafe(no_mangle)]
pub unsafe extern "system" fn CreateProcessA(
    application: *const u8,
    command_line: *mut u8,
    process_attributes: *const u8,
    thread_attributes: *const u8,
    inherit_handles: i32,
    flags: u32,
    environment: *const u8,
    current_directory: *const u8,
    startup_info: *const u8,
    information: *mut u8,
) -> i32 {
    let (app, cmd, cwd) = (ansi_string(application), ansi_string(command_line), ansi_string(current_directory));
    // SAFETY: the caller's arguments, as Learn documents them.
    unsafe {
        let startup = startup_from(startup_info, &ansi_string);
        let attributes = (inherits(process_attributes), inherits(thread_attributes));
        let request = Request {
            app: app.as_deref(),
            cmd: cmd.as_deref(),
            inherit_handles: inherit_handles != 0,
            flags,
            environment,
            current_directory: cwd.as_deref(),
            attribute_list: match attribute_list(startup_info, flags) {
                Ok(l) => l,
                Err(e) => return fail(e, FALSE),
            },
        };
        create(&request, startup.as_ref(), attributes, information)
    }
}

/// `CreateProcessW`'s arguments, decoded.
struct Request<'a> {
    app: Option<&'a [u16]>,
    cmd: Option<&'a [u16]>,
    inherit_handles: bool,
    flags: u32,
    environment: *const u8,
    current_directory: Option<&'a [u16]>,
    /// The `PROC_THREAD_ATTRIBUTE_LIST` of a `STARTUPINFOEX`, or null.
    attribute_list: *const u8,
}

/// Make the handle `h` inheritable (`SECURITY_ATTRIBUTES.bInheritHandle`
/// of the process or thread).
fn make_inheritable(h: Handle) {
    let info = [1u8, 0];
    // SAFETY: a handle just returned; Poly's ObjectHandleFlagInformation.
    let _ = unsafe {
        crate::handles::NtSetInformationObject(h, crate::handles::OBJECT_HANDLE_FLAG_INFORMATION, info.as_ptr(), 2)
    };
}

/// The work of `CreateProcessW` / `CreateProcessA`.
///
/// # Safety
/// `information` is null or a `PROCESS_INFORMATION`; `r.environment` is
/// null or a block as `CreateProcessW` documents.
unsafe fn create(r: &Request<'_>, startup: Option<&p::Startup>, attributes: (bool, bool), information: *mut u8) -> i32 {
    let (app, cmd, flags) = (r.app, r.cmd, r.flags);
    if information.is_null() || (app.is_none() && cmd.is_none()) {
        return fail(ERROR_INVALID_PARAMETER, FALSE);
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
    let env = match unsafe { child_environment(r.environment, flags) } {
        Ok(e) => e,
        Err(e) => return fail(e, FALSE),
    };
    let cwd = match r.current_directory {
        Some(d) => match full_path(d) {
            Ok((f, _)) => f,
            Err(e) => return fail(e, FALSE),
        },
        None => current_dir(),
    };
    // The parameters, laid out where they will stay during the call.
    let child = p::Child { image: &image, command_line: &line, env: &env, current: &cwd, startup };
    let size = match p::child_parameters(0, &child) {
        Ok(b) => b.len(),
        Err(e) => return fail(e, FALSE),
    };
    let mut buf: Vec<u8> = alloc::vec![0u8; size];
    let base = buf.as_mut_ptr() as u64;
    match p::child_parameters(base, &child) {
        Ok(b) => buf.copy_from_slice(&b),
        Err(e) => return fail(e, FALSE),
    }
    // The attribute list and CREATE_BREAKAWAY_FROM_JOB as Poly's attributes
    // of NtCreateUserProcess (ADR 0169): kind, value, size per record.
    // SAFETY: the caller's list.
    let parsed = match unsafe { crate::attrlist::parse(r.attribute_list) } {
        Ok(a) => a,
        Err(e) => return fail(e, FALSE),
    };
    if let Some(list) = &parsed.handle_list {
        // Learn: the handles must be inheritable, and bInheritHandles TRUE.
        let mut seen: Vec<usize> = Vec::new();
        let valid = r.inherit_handles
            && list.iter().all(|&h| {
                let mut f = 0u32;
                // SAFETY: a handle value and an out-pointer.
                let ok = unsafe { crate::handles::GetHandleInformation(h as Handle, &mut f) } != 0;
                let fresh = !seen.contains(&h);
                seen.push(h);
                ok && f & HANDLE_FLAG_INHERIT != 0 && fresh
            });
        if !valid {
            return fail(ERROR_INVALID_PARAMETER, FALSE);
        }
    }
    if !crate::attrlist::mitigation_defined(parsed.mitigation) {
        return fail(ERROR_INVALID_PARAMETER, FALSE);
    }
    let handle_words: Vec<u64> = parsed.handle_list.iter().flatten().map(|&h| h as u64).collect();
    let job_words: Vec<u64> = parsed.jobs.iter().map(|&h| h as u64).collect();
    let mut blob: Vec<u64> = Vec::new();
    let mut records = 0u64;
    let mut record = |kind: u64, value: u64, size: u64| {
        blob.extend_from_slice(&[kind, value, size]);
        records += 1;
    };
    if parsed.handle_list.is_some() {
        record(1, handle_words.as_ptr() as u64, handle_words.len() as u64 * 8);
    }
    if parsed.mitigation != 0 {
        record(2, parsed.mitigation, 8);
    }
    if !job_words.is_empty() {
        record(3, job_words.as_ptr() as u64, job_words.len() as u64 * 8);
    }
    if flags & CREATE_BREAKAWAY_FROM_JOB != 0 {
        record(4, 1, 0);
    }
    blob.insert(0, records);
    let attribute_blob = if records == 0 { core::ptr::null() } else { blob.as_ptr() as *const u8 };
    let process_flags = if r.inherit_handles { p::NT_PROCESS_INHERIT_HANDLES } else { 0 };
    let thread_flags = if flags & CREATE_SUSPENDED != 0 { p::NT_THREAD_CREATE_SUSPENDED } else { 0 };
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
            process_flags,
            thread_flags,
            buf.as_ptr(),
            core::ptr::null_mut(),
            attribute_blob,
        )
    };
    if !st.is_success() {
        return fail_status(st, FALSE);
    }
    if attributes.0 {
        make_inheritable(ph);
    }
    if attributes.1 {
        make_inheritable(th);
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

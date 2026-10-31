//! Processes (Learn: "_spawn, _wspawn functions", "_exec, _wexec
//! functions", "system, _wsystem", "_popen, _wpopen", "_pclose",
//! "_cwait"; ADR 0093) over `CreateProcessW`. What is decided before a
//! process exists — the command line, the file search, the command
//! interpreter's line, the descriptor block — is `poly_crt::spawn`'s.
//!
//! - A child gets the descriptors not opened with `_O_NOINHERIT`: each
//!   handle is duplicated as inheritable for the call (so the program's own
//!   handles keep their flags), passed in the `lpReserved2` block and, for
//!   0, 1 and 2, as `STARTUPINFO`'s standard handles (`STARTF_USESTDHANDLES`,
//!   so a child that is not on this runtime writes where `_dup2` pointed
//!   descriptor 1 too: Poly's choice).
//! - `_P_WAIT` returns the child's exit code; `_P_NOWAIT` and `_P_NOWAITO`
//!   its process handle (Learn: "The return value from an asynchronous
//!   _spawn ... is the process handle"), which `_cwait` waits for and
//!   closes; `_P_DETACH` starts it with `DETACHED_PROCESS` and returns 0;
//!   `_P_OVERLAY` and the `_exec*` functions start it and end this process
//!   at once with exit code 0, without `atexit` functions or writing
//!   streams (Learn: flush them first). The last three are Poly's readings
//!   (to be confirmed by the spec agent).
//! - `system` and `_popen` run `<interpreter> /c <command>`; with no
//!   interpreter (Poly has no `cmd.exe`, so it takes `COMSPEC` when that
//!   names a file) `system(NULL)` is 0 and the rest fail with `ENOENT`
//!   (Learn, `system`). `_popen`'s child gets this process's standard
//!   handles but the pipe's end for the one it reads or writes, and no
//!   other descriptors.
//!
//! The variadic `_spawnl*` / `_execl*` are assembly thunks
//! ([`crate::va_thunk`]) that hand their arguments on as an array.

use crate::global::{self, Global, Lock};
use crate::io;
use crate::lowio;
use crate::stdio::{self, File};
use crate::sys::{self, Handle};
use crate::text;
use alloc::vec::Vec;
use poly_crt::errno::{E2BIG, ECHILD, EINVAL, ENOENT};
use poly_crt::spawn::{self as sp, Passed};
use poly_crt::unit::Unit;

/// A started child: `PROCESS_INFORMATION`'s handles.
struct Child {
    process: Handle,
    thread: Handle,
}

fn with_nul(s: &[u16]) -> Vec<u16> {
    let mut v = Vec::with_capacity(s.len() + 1);
    v.extend_from_slice(s);
    v.push(0);
    v
}

/// An environment variable's value, if set.
fn env_var(name: &str) -> Option<Vec<u16>> {
    let n: Vec<u16> = name.encode_utf16().chain(core::iter::once(0)).collect();
    let mut buf = alloc::vec![0u16; 260];
    loop {
        // SAFETY: `n` is NUL-terminated and `buf` holds its length.
        let got = unsafe { sys::GetEnvironmentVariableW(n.as_ptr(), buf.as_mut_ptr(), buf.len() as u32) } as usize;
        if got == 0 {
            return None;
        }
        if got < buf.len() {
            buf.truncate(got);
            return Some(buf);
        }
        buf.resize(got, 0);
    }
}

/// `path` names a file, not a directory.
fn is_file(path: &[u16]) -> bool {
    let p = with_nul(path);
    // SAFETY: a NUL-terminated name.
    let a = unsafe { sys::GetFileAttributesW(p.as_ptr()) };
    a != sys::INVALID_FILE_ATTRIBUTES && a & sys::FILE_ATTRIBUTE_DIRECTORY == 0
}

/// The command interpreter: `COMSPEC` if it names a file, else `cmd.exe`
/// in a directory of `PATH` (Learn, `system`).
pub fn interpreter() -> Option<Vec<u16>> {
    if let Some(c) = env_var("COMSPEC").filter(|c| is_file(c)) {
        return Some(c);
    }
    let path = env_var("PATH")?;
    let name: Vec<u16> = "cmd.exe".encode_utf16().collect();
    sp::path_dirs(&path).into_iter().map(|d| sp::join(&d, &name)).find(|p| is_file(p))
}

/// The full path of the file `cmdname` stands for, if one exists.
fn resolve(cmdname: &[u16], search: bool) -> Option<Vec<u16>> {
    let path = if search { env_var("PATH") } else { None };
    let found = sp::search_order(cmdname, search, path.as_deref()).into_iter().find(|p| is_file(p))?;
    Some(io::full_path(&with_nul(&found)).unwrap_or(found))
}

/// Start `app` with command line `line`. `env`: a wide environment block,
/// or this process's. `std`: the child's standard handles, and no
/// descriptors (`_popen`); `None`: every inheritable descriptor.
fn start(app: &[u16], line: &[u16], env: Option<&[u16]>, flags: u32, std: Option<[Handle; 3]>) -> Option<Child> {
    if line.len() > sp::MAX_COMMAND_LINE {
        global::set_errno(E2BIG);
        return None;
    }
    let mut dups: Vec<Handle> = Vec::new();
    let mut dup = |h: Handle| -> Option<Handle> {
        let d = io::duplicate(h, true)?;
        dups.push(d);
        Some(d)
    };
    let (handles, block) = match std {
        Some(given) => {
            (given.map(|h| if h == 0 || h == lowio::NO_HANDLE { 0 } else { dup(h).unwrap_or(0) }), Vec::new())
        }
        None => {
            let passed: Vec<Option<Passed>> = lowio::inheritable()
                .into_iter()
                .map(|f| {
                    let f = f?;
                    Some(Passed { mode: f.mode, append: f.append, handle: dup(f.handle)? as u64 })
                })
                .collect();
            let at = |i: usize| passed.get(i).copied().flatten().map_or(0, |p| p.handle as Handle);
            ([at(0), at(1), at(2)], sp::encode(&passed))
        }
    };
    let mut si = [0u8; sys::STARTUPINFO_SIZE];
    si[..4].copy_from_slice(&(sys::STARTUPINFO_SIZE as u32).to_le_bytes());
    si[sys::SI_FLAGS..sys::SI_FLAGS + 4].copy_from_slice(&sys::STARTF_USESTDHANDLES.to_le_bytes());
    if block.len() > 4 {
        si[sys::SI_CB_RESERVED2..sys::SI_CB_RESERVED2 + 2].copy_from_slice(&(block.len() as u16).to_le_bytes());
        si[sys::SI_RESERVED2..sys::SI_RESERVED2 + 8].copy_from_slice(&(block.as_ptr() as u64).to_le_bytes());
    }
    for (k, h) in handles.iter().enumerate() {
        let at = sys::SI_STD_INPUT + 8 * k;
        si[at..at + 8].copy_from_slice(&(*h as u64).to_le_bytes());
    }
    let app = with_nul(app);
    let mut line = with_nul(line);
    let mut info: [Handle; 3] = [0; 3];
    let flags = flags | if env.is_some() { sys::CREATE_UNICODE_ENVIRONMENT } else { 0 };
    // SAFETY: NUL-terminated strings, a STARTUPINFOW whose block lives
    // through the call, a PROCESS_INFORMATION (24 bytes).
    let ok = unsafe {
        sys::CreateProcessW(
            app.as_ptr(),
            line.as_mut_ptr(),
            core::ptr::null(),
            core::ptr::null(),
            1,
            flags,
            env.map_or(core::ptr::null(), <[u16]>::as_ptr),
            core::ptr::null(),
            si.as_ptr(),
            info.as_mut_ptr(),
        )
    };
    if ok == 0 {
        global::set_errno_from_win32();
    }
    for d in dups {
        // SAFETY: the copies made for the call.
        unsafe { sys::CloseHandle(d) };
    }
    (ok != 0).then_some(Child { process: info[0], thread: info[1] })
}

/// Wait for a process and close its handle: its exit code.
fn wait(process: Handle) -> Option<u32> {
    let mut code = 0u32;
    // SAFETY: a process handle; `code` writable.
    let ok = unsafe {
        sys::WaitForSingleObject(process, sys::INFINITE) == sys::WAIT_OBJECT_0
            && sys::GetExitCodeProcess(process, &mut code) != 0
    };
    // SAFETY: as above.
    unsafe { sys::CloseHandle(process) };
    ok.then_some(code)
}

/// What `mode` does with a started child.
fn finish(mode: i32, child: Child) -> isize {
    // SAFETY: the child's thread handle, not needed.
    unsafe { sys::CloseHandle(child.thread) };
    match mode {
        sp::P_WAIT => wait(child.process).map_or(-1, |c| c as i32 as isize),
        sp::P_NOWAIT | sp::P_NOWAITO => child.process,
        sp::P_DETACH => {
            // SAFETY: the child's process handle.
            unsafe { sys::CloseHandle(child.process) };
            0
        }
        _ => {
            // _P_OVERLAY: the child replaces this process.
            // SAFETY: no preconditions.
            unsafe { sys::ExitProcess(0) }
        }
    }
}

/// `_spawn*`'s work with wide strings.
pub fn spawn(mode: i32, cmdname: &[u16], args: &[Vec<u16>], env: Option<&[Vec<u16>]>, search: bool) -> isize {
    if !(sp::P_WAIT..=sp::P_DETACH).contains(&mode) || cmdname.is_empty() || args.first().is_none_or(Vec::is_empty) {
        global::invalid(EINVAL);
        return -1;
    }
    let Some(file) = resolve(cmdname, search) else {
        global::set_errno(ENOENT);
        return -1;
    };
    let refs: Vec<&[u16]> = args.iter().map(Vec::as_slice).collect();
    let (app, line) = if sp::is_batch(&file) {
        let Some(shell) = interpreter() else {
            global::set_errno(ENOENT);
            return -1;
        };
        let mut parts: Vec<&[u16]> = alloc::vec![&file];
        parts.extend_from_slice(&refs[1..]);
        let line = sp::interpreter_line(&shell, &sp::command_line(&parts));
        (shell, line)
    } else {
        let line = sp::command_line(&refs);
        (file, line)
    };
    let block = env.map(|e| sp::environment_block(&e.iter().map(Vec::as_slice).collect::<Vec<&[u16]>>()));
    let flags = if mode == sp::P_DETACH { sys::DETACHED_PROCESS } else { 0 };
    match start(&app, &line, block.as_deref(), flags, None) {
        Some(child) => finish(mode, child),
        None => -1,
    }
}

/// A C string of either width as UTF-16 (narrow ones through the ANSI code
/// page).
///
/// # Safety
/// `p` is a valid C string of its width.
unsafe fn wide_of<C: Unit>(p: *const C) -> Vec<u16> {
    // SAFETY: the caller's string.
    let s = unsafe { poly_crt::unit::slice(p) };
    if C::WIDE {
        s.iter().map(|c| c.to_u32() as u16).collect()
    } else {
        text::ansi_to_wide(&s.iter().map(|c| c.to_u32() as u8).collect::<Vec<u8>>())
    }
}

/// A NULL-terminated array of C strings, as UTF-16; `None` for NULL.
///
/// # Safety
/// `p` is NULL or such an array.
unsafe fn strings<C: Unit>(p: *const *const C) -> Option<Vec<Vec<u16>>> {
    if p.is_null() {
        return None;
    }
    let mut v = Vec::new();
    let mut i = 0;
    // SAFETY: the caller's array, read up to its NULL.
    while let Some(s) = unsafe { (*p.add(i)).as_ref() } {
        // SAFETY: a C string of the array.
        v.push(unsafe { wide_of(s as *const C) });
        i += 1;
    }
    Some(v)
}

/// `_spawnv*` / `_execv*` with the caller's arrays.
///
/// # Safety
/// `cmdname` a C string, `argv` and `envp` NULL or NULL-terminated arrays.
unsafe fn spawn_v<C: Unit>(
    mode: i32,
    cmdname: *const C,
    argv: *const *const C,
    envp: Option<*const *const C>,
    search: bool,
) -> isize {
    if cmdname.is_null() || argv.is_null() {
        global::invalid(EINVAL);
        return -1;
    }
    // SAFETY: the caller's strings and arrays.
    let (name, args) = unsafe { (wide_of(cmdname), strings(argv).unwrap_or_default()) };
    // SAFETY: as above.
    let env = envp.and_then(|e| unsafe { strings(e) });
    spawn(mode, &name, &args, env.as_deref(), search)
}

/// `_spawnl*` / `_execl*`: the arguments from `va` (the first variadic
/// slot) up to a NULL, then — `with_env` — `envp`.
///
/// # Safety
/// `va` holds the C strings and the NULL (and `envp`) the caller passed.
unsafe fn spawn_l<C: Unit>(mode: i32, cmdname: *const C, va: *const u64, with_env: bool, search: bool) -> isize {
    let mut ptrs: Vec<*const C> = Vec::new();
    let mut i = 0;
    loop {
        // SAFETY: the caller's variadic slots, up to the NULL.
        let p = unsafe { va.add(i).read() } as *const C;
        i += 1;
        if p.is_null() {
            break;
        }
        ptrs.push(p);
    }
    ptrs.push(core::ptr::null());
    // SAFETY: the slot after the NULL is `envp` in the `e` forms.
    let envp = with_env.then(|| unsafe { va.add(i).read() } as *const *const C);
    // SAFETY: as documented.
    unsafe { spawn_v(mode, cmdname, ptrs.as_ptr(), envp, search) }
}

macro_rules! v_forms {
    ($($name:ident($($mode:ident)?) = $c:ty, $env:tt, $search:tt;)*) => {$(
        v_forms!(@ $name, $c, $env, $search, $($mode)?);
    )*};
    (@ $name:ident, $c:ty, false, $search:literal, mode) => {
        /// # Safety
        /// The C contract (Learn): a C string and a NULL-terminated array.
        #[unsafe(no_mangle)]
        pub unsafe extern "C" fn $name(mode: i32, cmdname: *const $c, argv: *const *const $c) -> isize {
            // SAFETY: as documented.
            unsafe { spawn_v(mode, cmdname, argv, None, $search) }
        }
    };
    (@ $name:ident, $c:ty, true, $search:literal, mode) => {
        /// # Safety
        /// The C contract (Learn): a C string and NULL-terminated arrays.
        #[unsafe(no_mangle)]
        pub unsafe extern "C" fn $name(mode: i32, cmdname: *const $c, argv: *const *const $c, envp: *const *const $c) -> isize {
            // SAFETY: as documented.
            unsafe { spawn_v(mode, cmdname, argv, Some(envp), $search) }
        }
    };
    (@ $name:ident, $c:ty, false, $search:literal,) => {
        /// # Safety
        /// The C contract (Learn): a C string and a NULL-terminated array.
        #[unsafe(no_mangle)]
        pub unsafe extern "C" fn $name(cmdname: *const $c, argv: *const *const $c) -> isize {
            // SAFETY: as documented.
            unsafe { spawn_v(sp::P_OVERLAY, cmdname, argv, None, $search) }
        }
    };
    (@ $name:ident, $c:ty, true, $search:literal,) => {
        /// # Safety
        /// The C contract (Learn): a C string and NULL-terminated arrays.
        #[unsafe(no_mangle)]
        pub unsafe extern "C" fn $name(cmdname: *const $c, argv: *const *const $c, envp: *const *const $c) -> isize {
            // SAFETY: as documented.
            unsafe { spawn_v(sp::P_OVERLAY, cmdname, argv, Some(envp), $search) }
        }
    };
}

v_forms! {
    _spawnv(mode) = u8, false, false;
    _spawnve(mode) = u8, true, false;
    _spawnvp(mode) = u8, false, true;
    _spawnvpe(mode) = u8, true, true;
    _wspawnv(mode) = u16, false, false;
    _wspawnve(mode) = u16, true, false;
    _wspawnvp(mode) = u16, false, true;
    _wspawnvpe(mode) = u16, true, true;
    _execv() = u8, false, false;
    _execve() = u8, true, false;
    _execvp() = u8, false, true;
    _execvpe() = u8, true, true;
    _wexecv() = u16, false, false;
    _wexecve() = u16, true, false;
    _wexecvp() = u16, false, true;
    _wexecvpe() = u16, true, true;
}

/// The `va_list` forms the variadic thunks call.
pub mod va {
    use super::*;

    macro_rules! l_forms {
        ($($name:ident($($mode:ident)?) = $c:ty, $env:tt, $search:tt;)*) => {$(
            l_forms!(@ $name, $c, $env, $search, $($mode)?);
        )*};
        (@ $name:ident, $c:ty, $env:literal, $search:literal, mode) => {
            /// # Safety
            /// `va` holds the caller's arguments after `cmdname`.
            pub unsafe extern "C" fn $name(mode: i32, cmdname: *const $c, va: *const u64) -> isize {
                // SAFETY: as documented.
                unsafe { spawn_l(mode, cmdname, va, $env, $search) }
            }
        };
        (@ $name:ident, $c:ty, $env:literal, $search:literal,) => {
            /// # Safety
            /// `va` holds the caller's arguments after `cmdname`.
            pub unsafe extern "C" fn $name(cmdname: *const $c, va: *const u64) -> isize {
                // SAFETY: as documented.
                unsafe { spawn_l(sp::P_OVERLAY, cmdname, va, $env, $search) }
            }
        };
    }

    l_forms! {
        spawnl(mode) = u8, false, false;
        spawnle(mode) = u8, true, false;
        spawnlp(mode) = u8, false, true;
        spawnlpe(mode) = u8, true, true;
        wspawnl(mode) = u16, false, false;
        wspawnle(mode) = u16, true, false;
        wspawnlp(mode) = u16, false, true;
        wspawnlpe(mode) = u16, true, true;
        execl() = u8, false, false;
        execle() = u8, true, false;
        execlp() = u8, false, true;
        execlpe() = u8, true, true;
        wexecl() = u16, false, false;
        wexecle() = u16, true, false;
        wexeclp() = u16, false, true;
        wexeclpe() = u16, true, true;
    }
}

crate::va_thunk!("_spawnl", 2, va::spawnl);
crate::va_thunk!("_spawnle", 2, va::spawnle);
crate::va_thunk!("_spawnlp", 2, va::spawnlp);
crate::va_thunk!("_spawnlpe", 2, va::spawnlpe);
crate::va_thunk!("_wspawnl", 2, va::wspawnl);
crate::va_thunk!("_wspawnle", 2, va::wspawnle);
crate::va_thunk!("_wspawnlp", 2, va::wspawnlp);
crate::va_thunk!("_wspawnlpe", 2, va::wspawnlpe);
crate::va_thunk!("_execl", 1, va::execl);
crate::va_thunk!("_execle", 1, va::execle);
crate::va_thunk!("_execlp", 1, va::execlp);
crate::va_thunk!("_execlpe", 1, va::execlpe);
crate::va_thunk!("_wexecl", 1, va::wexecl);
crate::va_thunk!("_wexecle", 1, va::wexecle);
crate::va_thunk!("_wexeclp", 1, va::wexeclp);
crate::va_thunk!("_wexeclpe", 1, va::wexeclpe);

/// `_cwait(&status, handle, action)`: wait for the process, store its exit
/// code, close the handle (Poly's choice: the handle `_spawn` returned is
/// used up); the handle, or −1 (`ECHILD` for a bad handle or a failed
/// wait, `EINVAL` for an unknown action).
///
/// # Safety
/// `status` is NULL or writable.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn _cwait(status: *mut i32, handle: isize, action: i32) -> isize {
    if action != sp::WAIT_CHILD && action != sp::WAIT_GRANDCHILD {
        global::invalid(EINVAL);
        return -1;
    }
    if handle == 0 || handle == -1 {
        global::set_errno(ECHILD);
        return -1;
    }
    match wait(handle) {
        Some(code) => {
            if !status.is_null() {
                // SAFETY: the caller's int.
                unsafe { status.write(code as i32) };
            }
            handle
        }
        None => {
            global::set_errno(ECHILD);
            -1
        }
    }
}

/// `system`'s work: `None` asks whether there is an interpreter.
fn system_wide(command: Option<&[u16]>) -> i32 {
    let shell = interpreter();
    let Some(command) = command else {
        if shell.is_none() {
            global::set_errno(ENOENT);
        }
        return i32::from(shell.is_some());
    };
    let Some(shell) = shell else {
        global::set_errno(ENOENT);
        return -1;
    };
    let line = sp::interpreter_line(&shell, command);
    match start(&shell, &line, None, 0, None) {
        Some(child) => {
            // SAFETY: the child's thread handle.
            unsafe { sys::CloseHandle(child.thread) };
            wait(child.process).map_or(-1, |c| c as i32)
        }
        None => -1,
    }
}

/// # Safety
/// `command` is NULL or a C string.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn system(command: *const u8) -> i32 {
    // SAFETY: the caller's string.
    let c = (!command.is_null()).then(|| unsafe { wide_of(command) });
    system_wide(c.as_deref())
}

/// # Safety
/// As `system`.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn _wsystem(command: *const u16) -> i32 {
    // SAFETY: the caller's string.
    let c = (!command.is_null()).then(|| unsafe { wide_of(command) });
    system_wide(c.as_deref())
}

/// The streams `_popen` made and their processes.
static POPEN: Global<Vec<(usize, Handle)>> = Global::new(Vec::new());
static POPEN_LOCK: Lock = Lock::new();

/// `_popen`'s work with wide strings.
fn popen_wide(command: &[u16], mode: &[u16]) -> *mut File {
    let Some((read, translation)) = sp::popen_mode(mode) else {
        global::invalid(EINVAL);
        return core::ptr::null_mut();
    };
    let Some(shell) = interpreter() else {
        global::set_errno(ENOENT);
        return core::ptr::null_mut();
    };
    let (mut r, mut w): (Handle, Handle) = (0, 0);
    // SAFETY: `r` and `w` writable.
    if unsafe { sys::CreatePipe(&mut r, &mut w, core::ptr::null(), 0) } == 0 {
        global::set_errno_from_win32();
        return core::ptr::null_mut();
    }
    let (mine, theirs) = if read { (r, w) } else { (w, r) };
    let std = |fd: i32| lowio::get(fd).map_or(0, |f| f.handle);
    let handles = if read { [std(0), theirs, std(2)] } else { [theirs, std(1), std(2)] };
    let child = start(&shell, &sp::interpreter_line(&shell, command), None, 0, Some(handles));
    // SAFETY: the child's end, which only the child keeps.
    unsafe { sys::CloseHandle(theirs) };
    let Some(child) = child else {
        // SAFETY: our end.
        unsafe { sys::CloseHandle(mine) };
        return core::ptr::null_mut();
    };
    // SAFETY: the child's thread handle.
    unsafe { sys::CloseHandle(child.thread) };
    let fd = lowio::allocate_with(mine, translation, false, true);
    let f = if fd < 0 {
        core::ptr::null_mut()
    } else {
        // SAFETY: a valid descriptor and a C string.
        unsafe { stdio::_fdopen(fd, if read { c"r".as_ptr() } else { c"w".as_ptr() } as *const u8) }
    };
    if f.is_null() {
        if fd >= 0 {
            io::_close(fd);
        } else {
            // SAFETY: our end, never given a descriptor.
            unsafe { sys::CloseHandle(mine) };
        }
        // SAFETY: the child's process handle.
        unsafe { sys::CloseHandle(child.process) };
        return core::ptr::null_mut();
    }
    let _g = POPEN_LOCK.lock();
    // SAFETY: under the lock.
    unsafe { (*POPEN.get()).push((f as usize, child.process)) };
    f
}

/// # Safety
/// `command` and `mode` are C strings.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn _popen(command: *const u8, mode: *const u8) -> *mut File {
    if command.is_null() || mode.is_null() {
        global::invalid(EINVAL);
        return core::ptr::null_mut();
    }
    // SAFETY: the caller's strings.
    unsafe { popen_wide(&wide_of(command), &wide_of(mode)) }
}

/// # Safety
/// As `_popen`.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn _wpopen(command: *const u16, mode: *const u16) -> *mut File {
    if command.is_null() || mode.is_null() {
        global::invalid(EINVAL);
        return core::ptr::null_mut();
    }
    // SAFETY: the caller's strings.
    unsafe { popen_wide(&wide_of(command), &wide_of(mode)) }
}

/// `_pclose(f)`: close the stream, wait for its command: the command's
/// exit code, or −1 (`EINVAL` for a stream `_popen` did not make).
#[unsafe(no_mangle)]
pub extern "C" fn _pclose(f: *mut File) -> i32 {
    let process = {
        let _g = POPEN_LOCK.lock();
        // SAFETY: under the lock.
        let list = unsafe { &mut *POPEN.get() };
        list.iter().position(|&(s, _)| s == f as usize).map(|i| list.swap_remove(i).1)
    };
    let Some(process) = process else {
        global::invalid(EINVAL);
        return -1;
    };
    stdio::fclose(f);
    wait(process).map_or(-1, |c| c as i32)
}

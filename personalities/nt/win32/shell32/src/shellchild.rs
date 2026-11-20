//! Starting a program as the session's shell does (`ShellDispatch.ShellExecute.yaml`,
//! `child_of_the_desktop_chain`): `ShellExecute` on the `Shell.Application` that the desktop's
//! `ShellWindows` chain hands out runs in Explorer on Windows, so the program is Explorer's child,
//! with Explorer's token (the user's filtered one), in the interactive session, whoever asked and
//! from whatever session. Poly's shell process is the `explorer.exe` stand-in (ADR 0222); the
//! program is created with it as its parent (`PROC_THREAD_ATTRIBUTE_PARENT_PROCESS`: Learn,
//! "UpdateProcThreadAttribute": the child gets the token of the process it is made a child of).
//!
//! The shell process is the one `GetShellWindow`'s window belongs to; a caller in session 0 has no
//! shell window (the sheet: `GetShellWindow` is NULL there, yet the chain reaches the shell of the
//! interactive session), so it is then the `explorer.exe` of a session other than 0.

use crate::com::nul;
use alloc::vec::Vec;
use core::ffi::c_void;

#[cfg_attr(target_arch = "x86", link(name = "user32", kind = "raw-dylib", import_name_type = "undecorated"))]
#[cfg_attr(not(target_arch = "x86"), link(name = "user32", kind = "raw-dylib"))]
unsafe extern "system" {
    fn GetShellWindow() -> usize;
    fn GetWindowThreadProcessId(hwnd: usize, pid: *mut u32) -> u32;
}

#[cfg_attr(target_arch = "x86", link(name = "kernel32", kind = "raw-dylib", import_name_type = "undecorated"))]
#[cfg_attr(not(target_arch = "x86"), link(name = "kernel32", kind = "raw-dylib"))]
unsafe extern "system" {
    fn OpenProcess(access: u32, inherit: i32, pid: u32) -> usize;
    fn CloseHandle(h: usize) -> i32;
    fn CreateToolhelp32Snapshot(flags: u32, pid: u32) -> usize;
    fn Process32FirstW(snapshot: usize, entry: *mut u8) -> i32;
    fn Process32NextW(snapshot: usize, entry: *mut u8) -> i32;
    fn ProcessIdToSessionId(pid: u32, session: *mut u32) -> i32;
    fn InitializeProcThreadAttributeList(list: *mut u8, count: u32, flags: u32, size: *mut usize) -> i32;
    fn UpdateProcThreadAttribute(
        list: *mut u8,
        flags: u32,
        attribute: usize,
        value: *const c_void,
        size: usize,
        previous: *mut c_void,
        return_size: *mut usize,
    ) -> i32;
    fn DeleteProcThreadAttributeList(list: *mut u8);
    fn CreateProcessW(
        app: *const u16,
        cmd: *mut u16,
        pa: *mut c_void,
        ta: *mut c_void,
        inherit: i32,
        flags: u32,
        env: *mut c_void,
        dir: *const u16,
        si: *mut c_void,
        pi: *mut [usize; 3],
    ) -> i32;
    fn GetLastError() -> u32;
}

/// `PROCESS_CREATE_PROCESS`, `PROCESS_QUERY_LIMITED_INFORMATION` (`winnt.h`).
const PROCESS_CREATE_PROCESS: u32 = 0x0080;
const PROCESS_QUERY_LIMITED_INFORMATION: u32 = 0x1000;
/// `TH32CS_SNAPPROCESS` (`tlhelp32.h`); `PROCESSENTRY32W`: `dwSize` first, `th32ProcessID` at 8,
/// `szExeFile` (260 units) at 44 (x64) or 36 (x86).
const TH32CS_SNAPPROCESS: u32 = 2;
const ENTRY_SIZE: usize = if core::mem::size_of::<usize>() == 8 { 568 } else { 556 };
const EXE_AT: usize = if core::mem::size_of::<usize>() == 8 { 44 } else { 36 };
/// `PROC_THREAD_ATTRIBUTE_PARENT_PROCESS`, `EXTENDED_STARTUPINFO_PRESENT` (`winbase.h`).
const PROC_THREAD_ATTRIBUTE_PARENT_PROCESS: usize = 0x0002_0000;
const EXTENDED_STARTUPINFO_PRESENT: u32 = 0x0008_0000;
/// `STARTUPINFOW`'s size, `dwFlags` and `wShowWindow` offsets (`processthreadsapi.h`); the
/// attribute list follows it in `STARTUPINFOEXW`.
const SI_SIZE: usize = if core::mem::size_of::<usize>() == 8 { 104 } else { 68 };
const SI_FLAGS_AT: usize = if core::mem::size_of::<usize>() == 8 { 60 } else { 44 };
const SI_SHOW_AT: usize = SI_FLAGS_AT + 4;
const STARTF_USESHOWWINDOW: u32 = 1;

/// Whether `name` (UTF-16, any case) is `explorer.exe`.
fn is_explorer(name: &[u16]) -> bool {
    let want = b"explorer.exe";
    name.len() == want.len()
        && name.iter().zip(want).all(|(&c, &w)| {
            let c = if (u16::from(b'A')..=u16::from(b'Z')).contains(&c) { c + 32 } else { c };
            c == u16::from(w)
        })
}

/// The id of the session's shell process, if there is one.
fn shell_pid() -> Option<u32> {
    let mut pid = 0u32;
    // SAFETY: plain calls; an out-pointer.
    let hwnd = unsafe { GetShellWindow() };
    if hwnd != 0 && unsafe { GetWindowThreadProcessId(hwnd, &mut pid) } != 0 && pid != 0 {
        return Some(pid);
    }
    // SAFETY: a snapshot of the processes, closed below; a buffer of the entry's size.
    unsafe {
        let snap = CreateToolhelp32Snapshot(TH32CS_SNAPPROCESS, 0);
        if snap == usize::MAX || snap == 0 {
            return None;
        }
        let mut entry = [0u8; ENTRY_SIZE];
        entry[..4].copy_from_slice(&(ENTRY_SIZE as u32).to_le_bytes());
        let mut found = None;
        let mut more = Process32FirstW(snap, entry.as_mut_ptr());
        while more != 0 {
            let id = u32::from_le_bytes([entry[8], entry[9], entry[10], entry[11]]);
            let exe: Vec<u16> = entry[EXE_AT..EXE_AT + 520]
                .chunks_exact(2)
                .map(|b| u16::from_le_bytes([b[0], b[1]]))
                .take_while(|&c| c != 0)
                .collect();
            let mut session = 0u32;
            if is_explorer(&exe) && ProcessIdToSessionId(id, &mut session) != 0 && session != 0 {
                found = Some(id);
                break;
            }
            more = Process32NextW(snap, entry.as_mut_ptr());
        }
        CloseHandle(snap);
        found
    }
}

/// Start `command` (a program and its arguments, quoted) in `dir` with show command `show` as a
/// child of the session's shell process. `None` when there is no shell process to start it from
/// (the caller then starts it itself); else whether it started (the Win32 error if not).
pub fn start(command: &[u16], dir: Option<&[u16]>, show: i32) -> Option<Result<(), u32>> {
    let pid = shell_pid()?;
    // SAFETY: a plain call; the handle is closed below.
    let shell = unsafe { OpenProcess(PROCESS_CREATE_PROCESS | PROCESS_QUERY_LIMITED_INFORMATION, 0, pid) };
    if shell == 0 {
        return None;
    }
    let result = (|| {
        let mut size = 0usize;
        // SAFETY: the size query (fails with ERROR_INSUFFICIENT_BUFFER and says the size).
        unsafe { InitializeProcThreadAttributeList(core::ptr::null_mut(), 1, 0, &mut size) };
        let mut list = alloc::vec![0u8; size.max(64)];
        // SAFETY: a buffer of the size asked for; the handle value outlives the list's use.
        unsafe {
            if InitializeProcThreadAttributeList(list.as_mut_ptr(), 1, 0, &mut size) == 0 {
                return Err(GetLastError());
            }
            let ok = UpdateProcThreadAttribute(
                list.as_mut_ptr(),
                0,
                PROC_THREAD_ATTRIBUTE_PARENT_PROCESS,
                (&shell as *const usize).cast(),
                core::mem::size_of::<usize>(),
                core::ptr::null_mut(),
                core::ptr::null_mut(),
            );
            if ok == 0 {
                let e = GetLastError();
                DeleteProcThreadAttributeList(list.as_mut_ptr());
                return Err(e);
            }
        }
        // STARTUPINFOEXW: the STARTUPINFOW with its size and show state, then the list.
        let mut si = alloc::vec![0u8; SI_SIZE + core::mem::size_of::<usize>()];
        si[..4].copy_from_slice(&((SI_SIZE + core::mem::size_of::<usize>()) as u32).to_le_bytes());
        si[SI_FLAGS_AT..SI_FLAGS_AT + 4].copy_from_slice(&STARTF_USESHOWWINDOW.to_le_bytes());
        si[SI_SHOW_AT..SI_SHOW_AT + 2].copy_from_slice(&(show as u16).to_le_bytes());
        si[SI_SIZE..].copy_from_slice(&(list.as_mut_ptr() as usize).to_le_bytes());
        let mut cmd = nul(command);
        let dir = dir.map(nul);
        let mut pi = [0usize; 3];
        // SAFETY: NUL-terminated strings (the command line writable), the structures above.
        let ok = unsafe {
            CreateProcessW(
                core::ptr::null(),
                cmd.as_mut_ptr(),
                core::ptr::null_mut(),
                core::ptr::null_mut(),
                0,
                EXTENDED_STARTUPINFO_PRESENT,
                core::ptr::null_mut(),
                dir.as_ref().map_or(core::ptr::null(), |d| d.as_ptr()),
                si.as_mut_ptr().cast(),
                &mut pi,
            )
        };
        // SAFETY: the list made above; the new process's handles, ours to close.
        unsafe {
            let e = if ok == 0 { GetLastError() } else { 0 };
            DeleteProcThreadAttributeList(list.as_mut_ptr());
            if ok == 0 {
                return Err(e);
            }
            CloseHandle(pi[0]);
            CloseHandle(pi[1]);
        }
        Ok(())
    })();
    // SAFETY: the handle opened above.
    unsafe { CloseHandle(shell) };
    Some(result)
}

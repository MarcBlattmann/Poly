//! 32-bit Windows programs on Poly (ADR 0186).
//!
//! A PE32 executable (machine I386) runs in a **32-bit process**: the kernel maps it and Poly's
//! own i686 `ntdll.dll` (built from the same sources as the x64 one) below 4 GiB, makes a PEB,
//! TEBs and process parameters with the 32-bit layout (`poly_rtl::layout::X86`), and runs its
//! threads in compatibility mode (a 32-bit code segment) with FS based at the TEB. Nothing
//! 64-bit runs in such a process: unlike Windows, Poly has no 64-bit `ntdll`/`wow64.dll`
//! layer inside it. The i686 `ntdll` enters the kernel with `int 0xF4` (EAX = the service
//! number, EDX = the address of the arguments on the caller's stack), and the personality
//! converts each call ([`thunk`]): it reads the 4-byte arguments, widens them (handles sign
//! extended, pointers and sizes zero extended), copies the structures whose layout depends on
//! the pointer size (`OBJECT_ATTRIBUTES`, `UNICODE_STRING`, `IO_STATUS_BLOCK`, `CLIENT_ID`,
//! `HANDLE*`, `SIZE_T*`, `PVOID*` and some information classes) into a 64-bit copy in a
//! scratch window above 4 GiB that only the call sees, runs the ordinary 64-bit service, and
//! copies the results back in the 32-bit layout.
//!
//! Exceptions reach the i686 `ntdll`'s `KiUserExceptionDispatcher` with an x86 `CONTEXT` and
//! `EXCEPTION_RECORD` on the thread's stack ([`context`]), which walks the thread's SEH frames
//! (`fs:[0]`); `NtContinue` and `NtRaiseException` take the x86 structures. What is not done yet
//! is listed in ADR 0186's staged plan (APCs, `NtGetContextThread`, the GUI).

pub mod context;
pub mod sigs;
pub mod thunk;

#[cfg(test)]
mod tests;

use crate::backend::Backend;
use poly_ntobj::NtStatus;

pub use thunk::dispatch32;

/// Where a 32-bit process's address space ends (exclusive): 2 GiB, or 4 GiB for an executable
/// marked `IMAGE_FILE_LARGE_ADDRESS_AWARE` (Learn: "Memory Limits for Windows and Windows
/// Server Releases": a 32-bit process on 64-bit Windows gets 2 GB, 4 GB with
/// `/LARGEADDRESSAWARE`), less the top 64 KiB as on Windows (the highest user address is
/// `0x7FFEFFFF` or `0xFFFEFFFF`).
pub fn address_end(characteristics: u16) -> u64 {
    if characteristics & poly_pe::file_flags::LARGE_ADDRESS_AWARE != 0 { 0xFFFF_0000 } else { 0x7FFF_0000 }
}

/// Where Poly keeps the 32-bit system DLLs (Learn: "File System Redirector": a 32-bit program's
/// `%windir%\System32` is `%windir%\SysWOW64`).
pub const SYSTEM_DIR: &str = "C:\\Windows\\SysWOW64";
/// The i686 `ntdll`'s path, as its loader entry names it.
pub const NTDLL_PATH: &str = "C:\\Windows\\SysWOW64\\ntdll.dll";

/// Write a 32-bit thread's first stack frame below `top` and return its ESP. Poly's own start
/// convention (both sides are ours, like the x64 one of ADR 0071): the thread starts at the i686
/// `ntdll`'s `LdrInitializeThunk` as if it had been called with three `cdecl` arguments, the
/// start routine, its argument and the executable's base, under a zero return address that ends
/// a stack walk.
pub fn start_esp(b: &mut dyn Backend, top: u64, args: [u64; 3]) -> Result<u64, NtStatus> {
    let esp = top - 16;
    let mut frame = [0u8; 16];
    for (i, a) in args.iter().enumerate() {
        frame[4 + 4 * i..8 + 4 * i].copy_from_slice(&(*a as u32).to_le_bytes());
    }
    b.write(esp, &frame)?;
    Ok(esp)
}

/// Where a thread's "file-system redirection is off" flag lives in its TEB
/// ([`poly_rtl::layout::TEB_FS_REDIRECTION_OFF`]): a `u32`, zero (a new thread's value) for
/// "redirection on". `kernelbase`'s `Wow64DisableWow64FsRedirection` family writes it (it owns
/// the TEB's user half), [`thunk`] reads it before it redirects a file name; no system call is
/// needed to change it, and it is per thread by construction (Learn: "Disabling file system
/// redirection affects only operations made by the current thread"). ADR 0186, addendum to
/// stage 2.
pub use poly_rtl::layout::TEB_FS_REDIRECTION_OFF;

fn lower(path: &[u16]) -> alloc::vec::Vec<u16> {
    path.iter().map(|&c| if (b'A' as u16..=b'Z' as u16).contains(&c) { c + 32 } else { c }).collect()
}

fn wide(s: &str) -> alloc::vec::Vec<u16> {
    s.encode_utf16().collect()
}

/// The length of the drive prefix of `lower` (`\??\c:` or `c:`) when it names drive C: (the
/// system drive; Poly's `%windir%` is `C:\Windows`, ADR 0073).
fn drive_prefix(lower: &[u16]) -> Option<usize> {
    ["\\??\\c:", "c:"].iter().map(|p| wide(p)).find(|p| lower.starts_with(p)).map(|p| p.len())
}

/// If `lower[at..]` starts with the directory `dir` (lower case) followed by the end of the path
/// or a backslash, the index just after `dir`.
fn directory_at(lower: &[u16], at: usize, dir: &str) -> Option<usize> {
    let dir = wide(dir);
    let rest = lower.get(at..)?;
    if !rest.starts_with(&dir) {
        return None;
    }
    let end = at + dir.len();
    (end == lower.len() || lower[end] == b'\\' as u16).then_some(end)
}

/// The file-system redirection of a 32-bit process (Learn: "File System Redirector"): a path
/// under `%windir%\System32` names the same path under `%windir%\SysWOW64`, except the
/// subdirectories Learn exempts (`catroot`, `catroot2`, `driverstore`, `drivers\etc`,
/// `logfiles`, `spool`); `%windir%\lastgood\system32` becomes `%windir%\lastgood\SysWOW64` and
/// `%windir%\regedit.exe` `%windir%\SysWOW64\regedit.exe` (both in Learn's table). `path` is a
/// DOS or NT path (`C:\...` or `\??\C:\...`); `None` when it is not redirected.
pub fn redirect_system32(path: &[u16]) -> Option<alloc::vec::Vec<u16>> {
    const EXEMPT: [&str; 6] = ["catroot", "catroot2", "driverstore", "drivers\\etc", "logfiles", "spool"];
    let lower = lower(path);
    let prefix = drive_prefix(&lower)?;
    if let Some(at) = directory_at(&lower, prefix, "\\windows\\lastgood\\system32") {
        let from = prefix + "\\windows\\lastgood".len();
        let mut out = path[..from].to_vec();
        out.extend("\\SysWOW64".encode_utf16());
        out.extend_from_slice(&path[at..]);
        return Some(out);
    }
    if directory_at(&lower, prefix, "\\windows\\regedit.exe").is_some() {
        let from = prefix + "\\windows".len();
        let mut out = path[..from].to_vec();
        out.extend("\\SysWOW64\\regedit.exe".encode_utf16());
        return Some(out);
    }
    let at = directory_at(&lower, prefix, "\\windows\\system32")?;
    let tail = &lower[at..];
    let sub = if tail.is_empty() { tail } else { &tail[1..] };
    for e in EXEMPT {
        let e = wide(e);
        if sub.starts_with(&e) && (sub.len() == e.len() || sub[e.len()] == b'\\' as u16) {
            return None;
        }
    }
    let mut out = path[..prefix].to_vec();
    out.extend("\\Windows\\SysWOW64".encode_utf16());
    out.extend_from_slice(&path[at..]);
    Some(out)
}

/// `%windir%\Sysnative` (Learn: "File System Redirector": "32-bit applications can access the
/// native system directory by substituting %windir%\Sysnative for %windir%\System32. WOW64
/// recognizes Sysnative as a special alias used to indicate that the file system should not
/// redirect the access"): the same path with `System32` for `Sysnative`, `None` for any other.
/// The alias is not a real directory, so it works with redirection on or off.
pub fn sysnative_to_system32(path: &[u16]) -> Option<alloc::vec::Vec<u16>> {
    let lower = lower(path);
    let prefix = drive_prefix(&lower)?;
    let at = directory_at(&lower, prefix, "\\windows\\sysnative")?;
    let mut out = path[..prefix].to_vec();
    out.extend("\\Windows\\System32".encode_utf16());
    out.extend_from_slice(&path[at..]);
    Some(out)
}

/// The name a 32-bit process's file service really opens for `path`: `Sysnative` is the native
/// `System32`; otherwise `System32` is `SysWOW64` ([`redirect_system32`]) unless the thread has
/// switched redirection off ([`TEB_FS_REDIRECTION_OFF`]). `None`: the name stays.
pub fn file_view(path: &[u16], redirection_off: bool) -> Option<alloc::vec::Vec<u16>> {
    if let Some(native) = sysnative_to_system32(path) {
        return Some(native);
    }
    if redirection_off { None } else { redirect_system32(path) }
}

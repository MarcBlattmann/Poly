//! The Windows API tracer (roadmap Phase 5; ADR 0090): record the calls a
//! program makes into the DLLs it imports, the same way on real Windows
//! and on Poly, and compare two recordings.
//!
//! `cargo xtask win trace-kit <exe> --out <dir>` writes a self-contained
//! directory: a copy of the program whose import directory names
//! `pt_<dll>.dll` instead of each `<dll>.dll` (only the name strings
//! change: a section with the new names is appended and the descriptors
//! point at it), one proxy DLL per imported DLL, `polytrace.dll` (the
//! logger, `tools/wintrace/polytrace`) and `polytrace-kit.json`. Run the
//! copy in that directory and it writes `polytrace.jsonl` (see the logger's
//! documentation for the format).
//!
//! A proxy exports exactly what the program imports from the real DLL,
//! by the same names and ordinals. A function export is a thunk
//! (`tools/wintrace/proxy/common.rs`) that calls the logger, then the real
//! function through the proxy's own import of it (the loader binds it,
//! as it binds the program's), then the logger again. Data exports (the C
//! runtime's `_iob`, `_fmode` …) are PE forwarders to the real DLL, so the
//! program gets the real object's address. The proxies are Rust `no_std`
//! cdylibs generated into `target/wintrace/build` and built like the
//! Windows test programs (ADR 0075): `-Zbuild-std` from `rust-src`,
//! `rust-lld`, offline.
//!
//! What each function's arguments are (how many, and which are integers,
//! pointers, handles or strings) comes from [`SIGNATURES`], written from
//! Microsoft Learn's prototypes; a function without an entry is logged with
//! four arguments of unknown kind. [`NORETURN`], [`TAIL`] and [`FLUSH`] say
//! which functions do not return, must see their caller's frame, or may end
//! the process.
//!
//! `cargo xtask win trace-diff <a.jsonl> <b.jsonl>` aligns two recordings
//! thread by thread in call order and reports where they part: a call only
//! one side made, or the same call with another result, last error or
//! integer or string argument (pointers and handles are compared only as
//! NULL, `INVALID_HANDLE_VALUE` or "some value").
//!
//! Only export and import *names* of the program are read (CLEAN_ROOM.md);
//! the tracer observes calls, it does not look inside any DLL.

use crate::Result;
use crate::winimports::{Import, Sym};
use serde_json::Value;
use std::collections::BTreeMap;
use std::fmt::Write as _;
use std::path::{Path, PathBuf};
use std::process::Command;

// ----- the tables -------------------------------------------------------------------------

/// Signatures `ret=args` from Microsoft Learn's prototypes, one letter per
/// value (`tools/wintrace/polytrace`: `i` 32-bit integer, `q` 64-bit
/// integer, `s` 16-bit, `c` 8-bit, `p` pointer, `h` handle, `w`/`a` wide /
/// ANSI string, `v` void). `oleaut32`'s functions are listed by name; the
/// program imports them by ordinal ([`ordinal_name`]).
pub const SIGNATURES: &[(&str, &str)] = &[
    // kernel32 (processthreadsapi.h, synchapi.h, fileapi.h, memoryapi.h, ...)
    ("CloseHandle", "i=h"),
    ("CompareFileTime", "i=pp"),
    ("CreateDirectoryW", "i=wp"),
    ("CreateEventW", "h=piiw"),
    ("CreateFileW", "h=wiipiih"),
    ("CreateHardLinkW", "i=wwp"),
    ("CreateSemaphoreW", "h=piiw"),
    ("CreateThread", "h=pqppip"),
    ("DeleteCriticalSection", "v=p"),
    ("DeleteFileW", "i=w"),
    ("DeviceIoControl", "i=hipipipp"),
    ("DosDateTimeToFileTime", "i=ssp"),
    ("EnterCriticalSection", "v=p"),
    ("ExitProcess", "v=i"),
    ("ExitThread", "v=i"),
    ("FileTimeToDosDateTime", "i=ppp"),
    ("FileTimeToLocalFileTime", "i=pp"),
    ("FileTimeToSystemTime", "i=pp"),
    ("FindClose", "i=h"),
    ("FindFirstFileW", "h=wp"),
    ("FindFirstStreamW", "h=wipi"),
    ("FindNextFileW", "i=hp"),
    ("FindNextStreamW", "i=hp"),
    ("FormatMessageW", "i=ipiipip"),
    ("FreeLibrary", "i=h"),
    ("GetCommandLineW", "p="),
    ("GetConsoleMode", "i=hp"),
    ("GetConsoleScreenBufferInfo", "i=hp"),
    ("GetCurrentDirectoryW", "i=ip"),
    ("GetCurrentProcess", "h="),
    ("GetCurrentProcessId", "i="),
    ("GetCurrentThreadId", "i="),
    ("GetDiskFreeSpaceExW", "i=wppp"),
    ("GetDiskFreeSpaceW", "i=wpppp"),
    ("GetEnvironmentVariableW", "i=wpi"),
    ("GetExitCodeThread", "i=hp"),
    ("GetFileAttributesW", "i=w"),
    ("GetFileInformationByHandle", "i=hp"),
    ("GetFileSize", "i=hp"),
    ("GetFileSizeEx", "i=hp"),
    ("GetFullPathNameW", "i=wipp"),
    ("GetLargePageMinimum", "q="),
    ("GetLastError", "i="),
    ("GetLogicalDriveStringsW", "i=ip"),
    ("GetModuleFileNameW", "i=hpi"),
    ("GetModuleHandleA", "h=a"),
    ("GetModuleHandleW", "h=w"),
    ("GetOEMCP", "i="),
    ("GetACP", "i="),
    ("GetProcAddress", "p=ha"),
    ("GetProcessAffinityMask", "i=hpp"),
    ("GetProcessHeap", "h="),
    ("GetProcessTimes", "i=hpppp"),
    ("GetStartupInfoW", "v=p"),
    ("GetStdHandle", "h=i"),
    ("GetSystemInfo", "v=p"),
    ("GetSystemTimeAsFileTime", "v=p"),
    ("GetTempPathW", "i=ip"),
    ("GetTickCount", "i="),
    ("GetTickCount64", "q="),
    ("GetVersion", "i="),
    ("GlobalMemoryStatusEx", "i=p"),
    ("HeapAlloc", "p=hiq"),
    ("HeapFree", "i=hip"),
    ("InitializeCriticalSection", "v=p"),
    ("IsProcessorFeaturePresent", "i=i"),
    ("LeaveCriticalSection", "v=p"),
    ("LoadLibraryExW", "h=whi"),
    ("LoadLibraryW", "h=w"),
    ("LocalFileTimeToFileTime", "i=pp"),
    ("LocalFree", "p=p"),
    ("MapViewOfFile", "p=hiiiq"),
    ("MoveFileW", "i=ww"),
    ("MoveFileWithProgressW", "i=wwppi"),
    ("MultiByteToWideChar", "i=iipipi"),
    ("OpenEventW", "h=iiw"),
    ("OpenFileMappingW", "h=iiw"),
    ("QueryPerformanceCounter", "i=p"),
    ("QueryPerformanceFrequency", "i=p"),
    ("RaiseException", "v=iiip"),
    ("ReadConsoleW", "i=hpipp"),
    ("ReadFile", "i=hpipp"),
    ("ReleaseSemaphore", "i=hip"),
    ("RemoveDirectoryW", "i=w"),
    ("ResetEvent", "i=h"),
    ("ResumeThread", "i=h"),
    ("SetConsoleCtrlHandler", "i=pi"),
    ("SetConsoleMode", "i=hi"),
    ("SetCurrentDirectoryW", "i=w"),
    ("SetEndOfFile", "i=h"),
    ("SetEvent", "i=h"),
    ("SetFileApisToOEM", "v="),
    ("SetFileAttributesW", "i=wi"),
    ("SetFilePointer", "i=hipi"),
    ("SetFilePointerEx", "i=hqpi"),
    ("SetFileTime", "i=hppp"),
    ("SetLastError", "v=i"),
    ("SetProcessAffinityMask", "i=hq"),
    ("SetThreadAffinityMask", "q=hq"),
    ("SetThreadPriority", "i=hi"),
    ("Sleep", "v=i"),
    ("SuspendThread", "i=h"),
    ("TerminateProcess", "i=hi"),
    ("UnmapViewOfFile", "i=p"),
    ("VirtualAlloc", "p=pqii"),
    ("VirtualFree", "i=pqi"),
    ("WaitForMultipleObjects", "i=ipii"),
    ("WaitForSingleObject", "i=hi"),
    ("WideCharToMultiByte", "i=iipipipp"),
    ("WriteConsoleW", "i=hpipp"),
    ("WriteFile", "i=hpipp"),
    // advapi32 (securitybaseapi.h, winreg.h, ntsecapi.h)
    ("AdjustTokenPrivileges", "i=hipipp"),
    ("GetFileSecurityW", "i=wipip"),
    ("LookupPrivilegeValueW", "i=wwp"),
    ("OpenProcessToken", "i=hip"),
    ("RegCloseKey", "i=h"),
    ("RegOpenKeyExW", "i=hwiip"),
    ("RegQueryValueExW", "i=hwpppp"),
    ("SetFileSecurityW", "i=wip"),
    ("SystemFunction036", "c=pi"),
    // msvcrt (the C runtime reference)
    ("??1type_info@@UEAA@XZ", "v=p"),
    ("?terminate@@YAXXZ", "v="),
    ("_CxxThrowException", "v=pp"),
    ("_XcptFilter", "i=ip"),
    ("__C_specific_handler", "i=pppp"),
    ("__CxxFrameHandler", "i=pppp"),
    ("__CxxFrameHandler3", "i=pppp"),
    ("__dllonexit", "p=ppp"),
    ("__getmainargs", "i=pppip"),
    ("__set_app_type", "v=i"),
    ("__setusermatherr", "v=p"),
    ("__wgetmainargs", "i=pppip"),
    ("_beginthreadex", "h=pippip"),
    ("_c_exit", "v="),
    ("_cexit", "v="),
    ("_exit", "v=i"),
    ("_get_osfhandle", "h=i"),
    ("_initterm", "v=pp"),
    ("_isatty", "i=i"),
    ("_onexit", "p=p"),
    ("_purecall", "i="),
    ("abort", "v="),
    ("calloc", "p=qq"),
    ("exit", "v=i"),
    ("fflush", "i=p"),
    ("fgetc", "i=p"),
    ("fputc", "i=ip"),
    ("fputs", "i=ap"),
    ("free", "v=p"),
    ("malloc", "p=q"),
    ("memcmp", "i=ppq"),
    ("memcpy", "p=ppq"),
    ("memmove", "p=ppq"),
    ("memset", "p=piq"),
    ("realloc", "p=pq"),
    ("strcmp", "i=aa"),
    ("strlen", "q=a"),
    ("wcscmp", "i=ww"),
    ("wcslen", "q=w"),
    ("wcsstr", "p=ww"),
    // oleaut32 (oleauto.h), imported by ordinal
    ("SysAllocString", "p=w"),
    ("SysAllocStringLen", "p=pi"),
    ("SysFreeString", "v=p"),
    ("SysReAllocString", "i=pw"),
    ("SysReAllocStringLen", "i=ppi"),
    ("SysStringLen", "i=p"),
    ("VariantClear", "i=p"),
    ("VariantCopy", "i=pp"),
    ("VariantInit", "v=p"),
    // user32 (winuser.h)
    ("CharPrevExA", "p=sppi"),
    ("CharUpperW", "p=w"),
];

/// Functions that do not return: their record is written when they begin
/// and the buffer goes to the file.
pub const NORETURN: &[&str] = &[
    "ExitProcess",
    "ExitThread",
    "FatalExit",
    "FatalAppExitA",
    "FatalAppExitW",
    "FreeLibraryAndExitThread",
    "RaiseException",
    "RaiseFailFastException",
    "exit",
    "_exit",
    "_Exit",
    "quick_exit",
    "abort",
    "_amsg_exit",
    "?terminate@@YAXXZ",
    "terminate",
    "_purecall",
    "_endthread",
    "_endthreadex",
    "_invalid_parameter_noinfo_noreturn",
];

/// Functions that capture, unwind or resume their caller's frame: the
/// thunk jumps to them, so they see the caller's own frame (their record
/// is written when they begin).
pub const TAIL: &[&str] = &[
    "_CxxThrowException",
    "longjmp",
    "_longjmp",
    "setjmp",
    "_setjmp",
    "_setjmpex",
    "__intrinsic_setjmp",
    "__intrinsic_setjmpex",
    "__C_specific_handler",
    "__CxxFrameHandler",
    "__CxxFrameHandler2",
    "__CxxFrameHandler3",
    "__CxxFrameHandler4",
    "__GSHandlerCheck",
    "__GSHandlerCheck_EH",
    "__GSHandlerCheck_SEH",
    "_local_unwind",
    "RtlUnwind",
    "RtlUnwindEx",
    "RtlCaptureContext",
    "RtlRestoreContext",
];

/// Functions that may end the process (the buffer is written out first).
pub const FLUSH: &[&str] = &["TerminateProcess", "TerminateThread"];

/// The C runtimes' exported variables (Learn: "Global variables"; and
/// those of our own `msvcrt.dll`): forwarders in a proxy, not thunks.
pub const DATA: &[&str] = &[
    "_iob",
    "__argc",
    "__argv",
    "__wargv",
    "_environ",
    "_wenviron",
    "__initenv",
    "__winitenv",
    "_acmdln",
    "_wcmdln",
    "_pgmptr",
    "_wpgmptr",
    "_fmode",
    "_commode",
    "_ctype",
    "_pctype",
    "_pwctype",
    "__mb_cur_max",
    "_timezone",
    "_daylight",
    "_dstbias",
    "_tzname",
    "_sys_errlist",
    "_sys_nerr",
    "_osver",
    "_winver",
    "_winmajor",
    "_winminor",
    "_HUGE",
    "__badioinfo",
    "__pioinfo",
];

/// The logger's flags (`tools/wintrace/polytrace`: `NORETURN`, `TAIL`,
/// `FLUSH`).
const F_NORETURN: u32 = 1;
const F_TAIL: u32 = 2;
const F_FLUSH: u32 = 4;

/// The name of an `oleaut32` ordinal, from our `oleaut32.dll`'s export
/// table (`poly_kernelbase::exports::OLEAUT32_EXPORTS`, the coverage
/// map's ordinals).
pub fn ordinal_name(dll: &str, ordinal: u16) -> Option<&'static str> {
    if !dll.eq_ignore_ascii_case("oleaut32.dll") {
        return None;
    }
    poly_kernelbase::exports::OLEAUT32_EXPORTS.iter().find(|&&(_, o)| o == ordinal).map(|&(n, _)| n)
}

/// A function's signature, if [`SIGNATURES`] has it.
pub fn signature(dll: &str, sym: &Sym) -> Option<&'static str> {
    let name = match sym {
        Sym::Name(n) => n.as_str(),
        Sym::Ordinal(o) => ordinal_name(dll, *o)?,
    };
    SIGNATURES.iter().find(|&&(n, _)| n == name).map(|&(_, s)| s)
}

fn flags_of(sym: &Sym) -> u32 {
    let Sym::Name(n) = sym else { return 0 };
    let mut f = 0;
    if NORETURN.contains(&n.as_str()) {
        f |= F_NORETURN;
    }
    if TAIL.contains(&n.as_str()) {
        f |= F_TAIL;
    }
    if FLUSH.contains(&n.as_str()) {
        f |= F_FLUSH;
    }
    f
}

// ----- the proxies ------------------------------------------------------------------------

/// One function a proxy exports.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ProxyFunc {
    pub sym: Sym,
    /// `ret=args`, or empty when not known.
    pub sig: String,
    pub flags: u32,
}

/// One proxy DLL.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Proxy {
    /// The DLL as the program names it (`KERNEL32.dll`).
    pub dll: String,
    /// Its file name, lower case (`kernel32.dll`), and its stem.
    pub real: String,
    /// The proxy's file name (`pt_kernel32.dll`) and crate name.
    pub file: String,
    pub crate_name: String,
    pub funcs: Vec<ProxyFunc>,
    /// Exported variables, forwarded to the real DLL.
    pub data: Vec<String>,
}

impl Proxy {
    fn stem(&self) -> &str {
        self.real.strip_suffix(".dll").unwrap_or(&self.real)
    }
}

/// A name the generated assembly and linker options can carry as it is.
fn plain_name(name: &str) -> bool {
    !name.is_empty() && name.bytes().all(|b| b.is_ascii_graphic() && !b"\"\\,=;".contains(&b))
}

/// The proxies for a program's imports (in the order the DLLs first
/// appear); `data` adds names to treat as variables.
pub fn plan(imports: &[Import], data: &[String]) -> Result<Vec<Proxy>> {
    let mut out: Vec<Proxy> = Vec::new();
    for imp in imports {
        let real = imp.dll.to_ascii_lowercase();
        if !real.ends_with(".dll") || !plain_name(&real) || real.starts_with("pt_") {
            return Err(format!("cannot make a proxy for {:?}", imp.dll));
        }
        let at = match out.iter().position(|p| p.real == real) {
            Some(i) => i,
            None => {
                let stem = &real[..real.len() - 4];
                let crate_name: String =
                    format!("pt_{stem}").chars().map(|c| if c.is_ascii_alphanumeric() { c } else { '_' }).collect();
                out.push(Proxy {
                    dll: imp.dll.clone(),
                    real: real.clone(),
                    file: format!("pt_{real}"),
                    crate_name,
                    funcs: Vec::new(),
                    data: Vec::new(),
                });
                out.len() - 1
            }
        };
        let p = &mut out[at];
        match &imp.sym {
            Sym::Name(n) if !plain_name(n) => return Err(format!("{}: cannot export {n:?}", imp.dll)),
            Sym::Name(n) if DATA.contains(&n.as_str()) || data.contains(n) => {
                if !p.data.contains(n) {
                    p.data.push(n.clone());
                }
            }
            sym => {
                if !p.funcs.iter().any(|f| &f.sym == sym) {
                    let sig = signature(&real, sym).unwrap_or("").to_string();
                    p.funcs.push(ProxyFunc { sym: sym.clone(), sig, flags: flags_of(sym) });
                }
            }
        }
    }
    Ok(out)
}

/// The stack slots a thunk copies for a signature: the arguments beyond
/// the four in registers, or all sixteen it can when the signature is not
/// known.
pub fn stack_slots(sig: &str) -> usize {
    match sig.split_once('=') {
        Some((_, args)) => args.len().saturating_sub(4),
        None => 16,
    }
}

/// The generated `src/lib.rs` of a proxy; `common` is the path of
/// `tools/wintrace/proxy/common.rs`.
pub fn lib_rs(p: &Proxy, common: &Path) -> String {
    let mut s = format!(
        "// Generated by `cargo xtask win trace-kit` (ADR 0090); do not edit.\n\
         // {}: the proxy of {}.\n\
         #![no_std]\n#![allow(dead_code)]\n\n\
         #[path = {:?}]\nmod common;\n\n\
         #[link(name = {:?}, kind = \"raw-dylib\")]\nunsafe extern \"C\" {{\n",
        p.file,
        p.real,
        common.to_string_lossy(),
        p.stem()
    );
    for (i, f) in p.funcs.iter().enumerate() {
        match &f.sym {
            Sym::Name(n) => {
                let _ = writeln!(s, "    #[link_name = {n:?}]\n    fn real_{i}();");
            }
            Sym::Ordinal(o) => {
                let _ = writeln!(s, "    #[link_ordinal({o})]\n    fn real_{i}();");
            }
        }
    }
    s.push_str("}\n\n// Each function's record (`polytrace::Func`) and thunk.\ncore::arch::global_asm!(\n    r##\"\n");
    s.push_str("    .section .rdata,\"dr\"\n    .p2align 3\n");
    for (i, f) in p.funcs.iter().enumerate() {
        let slot = match &f.sym {
            Sym::Name(n) => format!("__imp_{n}"),
            Sym::Ordinal(_) => format!("__imp_real_{i}"),
        };
        let _ = write!(
            s,
            "pt_f{i}:\n    .quad \"{slot}\"\n    .quad pt_dll\n    .quad pt_n{i}\n    .quad pt_g{i}\n    .long {}\n    .long {}\n",
            f.flags,
            stack_slots(&f.sig)
        );
    }
    let _ = writeln!(s, "pt_dll:\n    .asciz \"{}\"", p.real);
    for (i, f) in p.funcs.iter().enumerate() {
        let _ = writeln!(s, "pt_n{i}:\n    .asciz \"{}\"\npt_g{i}:\n    .asciz \"{}\"", f.sym, f.sig);
    }
    s.push_str("    .text\n");
    for i in 0..p.funcs.len() {
        let _ = writeln!(s, "    .globl pt_t{i}\npt_t{i}:\n    lea r10, [rip + pt_f{i}]\n    jmp pt_common");
    }
    s.push_str("\"##\n);\n");
    s
}

/// The linker options of a proxy (its `build.rs`): the entry point and
/// one `/EXPORT:` per function and variable.
pub fn link_args(p: &Proxy) -> Vec<String> {
    let mut v = vec!["/ENTRY:pt_proxy_main".to_string()];
    for (i, f) in p.funcs.iter().enumerate() {
        v.push(match &f.sym {
            Sym::Name(n) => format!("/EXPORT:{n}=pt_t{i}"),
            Sym::Ordinal(o) => format!("/EXPORT:pt_ordinal_{o}=pt_t{i},@{o},NONAME"),
        });
    }
    for d in &p.data {
        v.push(format!("/EXPORT:{d}={}.{d}", p.stem()));
    }
    v
}

fn build_rs(p: &Proxy) -> String {
    let mut s = String::from("// Generated by `cargo xtask win trace-kit` (ADR 0090); do not edit.\nfn main() {\n");
    for a in link_args(p) {
        let _ = writeln!(s, "    println!(\"cargo:rustc-link-arg={}\");", a.replace('{', "{{").replace('}', "}}"));
    }
    s.push_str("}\n");
    s
}

fn package_toml(name: &str, lib: &str, build: &str, lib_name: &str) -> String {
    format!(
        "# Generated by `cargo xtask win trace-kit` (ADR 0090); do not edit.\n\
         [package]\nname = {name:?}\nversion = \"0.0.0\"\nedition = \"2024\"\npublish = false\nbuild = {build:?}\n\n\
         [lib]\nname = {lib_name:?}\npath = {lib:?}\ncrate-type = [\"cdylib\"]\n"
    )
}

/// Write the generated workspace (the logger and every proxy) into `dir`.
pub fn generate(dir: &Path, repo: &Path, proxies: &[Proxy]) -> Result<()> {
    let write = |path: PathBuf, text: String| -> Result<()> {
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent).map_err(|e| format!("{}: {e}", parent.display()))?;
        }
        std::fs::write(&path, text).map_err(|e| format!("{}: {e}", path.display()))
    };
    let _ = std::fs::remove_dir_all(dir);
    let tracer = repo.join("tools/wintrace");
    let mut members = vec!["\"polytrace\"".to_string()];
    members.extend(proxies.iter().map(|p| format!("{:?}", p.crate_name)));
    write(
        dir.join("Cargo.toml"),
        format!(
            "# Generated by `cargo xtask win trace-kit` (ADR 0090); do not edit.\n[workspace]\nresolver = \"3\"\nmembers = [{}]\n\n\
             [profile.release]\npanic = \"abort\"\nopt-level = \"s\"\ncodegen-units = 1\ndebug = false\n",
            members.join(", ")
        ),
    )?;
    write(
        dir.join("polytrace/Cargo.toml"),
        package_toml(
            "polytrace",
            &tracer.join("polytrace/src/lib.rs").to_string_lossy(),
            &tracer.join("polytrace/build.rs").to_string_lossy(),
            "polytrace",
        ),
    )?;
    let common = tracer.join("proxy/common.rs");
    for p in proxies {
        let d = dir.join(&p.crate_name);
        write(d.join("Cargo.toml"), package_toml(&p.crate_name, "src/lib.rs", "build.rs", &p.crate_name))?;
        write(d.join("build.rs"), build_rs(p))?;
        write(d.join("src/lib.rs"), lib_rs(p, &common))?;
    }
    Ok(())
}

// ----- the instrumented program -----------------------------------------------------------

fn rd16(b: &[u8], at: usize) -> Result<u16> {
    b.get(at..at + 2).map(|s| u16::from_le_bytes([s[0], s[1]])).ok_or_else(|| "truncated PE headers".to_string())
}
fn rd32(b: &[u8], at: usize) -> Result<u32> {
    b.get(at..at + 4)
        .map(|s| u32::from_le_bytes([s[0], s[1], s[2], s[3]]))
        .ok_or_else(|| "truncated PE headers".to_string())
}
fn wr32(b: &mut [u8], at: usize, v: u32) {
    b[at..at + 4].copy_from_slice(&v.to_le_bytes());
}
fn align_up(v: u32, a: u32) -> u32 {
    v.div_ceil(a.max(1)) * a.max(1)
}

/// The PE checksum (`CheckSumMappedFile`'s algorithm as the PE format
/// describes the field: the 16-bit one's-complement-style sum of the file
/// with the checksum field taken as zero, plus the file's length).
pub fn pe_checksum(b: &[u8], checksum_at: usize) -> u32 {
    let mut sum: u64 = 0;
    for (i, chunk) in b.chunks(2).enumerate() {
        let at = i * 2;
        if at >= checksum_at && at < checksum_at + 4 {
            continue;
        }
        let w = u16::from_le_bytes([chunk[0], *chunk.get(1).unwrap_or(&0)]) as u64;
        sum += w;
        sum = (sum & 0xFFFF) + (sum >> 16);
    }
    sum = (sum & 0xFFFF) + (sum >> 16);
    (sum as u32).wrapping_add(b.len() as u32)
}

/// A copy of a PE image whose import (and delay-load import) directory
/// names `rename(dll)` instead of each DLL it returns a name for. The new
/// names go into an appended section `.ptname`; the descriptors' name
/// fields point there, bound imports are dropped (the directory and the
/// descriptors' time stamps), as is an Authenticode signature (no longer
/// valid); the checksum is recomputed. Nothing else changes.
pub fn rename_imports(bytes: &[u8], rename: impl Fn(&str) -> Option<String>) -> Result<Vec<u8>> {
    let pe = poly_pe::Pe::parse(bytes).map_err(|e| format!("not a PE image: {e:?}"))?;
    if pe.info().bits() != poly_pe::Bits::Pe32Plus {
        return Err("only PE32+ (x64) images are supported".into());
    }
    let mut b = bytes.to_vec();
    let e = rd32(&b, 0x3C)? as usize;
    if b.get(e..e + 4) != Some(b"PE\0\0") {
        return Err("no PE signature".into());
    }
    let fh = e + 4;
    let nsect = rd16(&b, fh + 2)? as usize;
    let opt = fh + 20;
    let opt_size = rd16(&b, fh + 16)? as usize;
    let sect_align = rd32(&b, opt + 32)?;
    let file_align = rd32(&b, opt + 36)?;
    let size_of_headers = rd32(&b, opt + 60)? as usize;
    let checksum_at = opt + 64;
    let ndirs = rd32(&b, opt + 108)? as usize;
    let dir_at = |i: usize| opt + 112 + 8 * i;
    let table = opt + opt_size;
    let sections: Vec<poly_pe::SectionHeader> = pe.sections().iter().collect();
    // Room for one more section header in the headers.
    let new_hdr = table + 40 * nsect;
    let first_raw = sections.iter().map(|s| s.pointer_to_raw_data).filter(|&p| p != 0).min().unwrap_or(0) as usize;
    if new_hdr + 40 > size_of_headers
        || (first_raw != 0 && new_hdr + 40 > first_raw)
        || b.get(new_hdr..new_hdr + 40).is_none_or(|h| h.iter().any(|&x| x != 0))
    {
        return Err("no room for another section header".into());
    }
    let to_off = |rva: u32| -> Result<usize> {
        sections
            .iter()
            .find(|s| rva >= s.virtual_address && rva < s.virtual_address + s.virtual_size.max(s.size_of_raw_data))
            .map(|s| (rva - s.virtual_address + s.pointer_to_raw_data) as usize)
            .ok_or_else(|| format!("RVA {rva:#x} is in no section"))
    };
    // The new names, and where each descriptor's name field is.
    let mut names = Vec::new();
    let mut fixups: Vec<(usize, u32)> = Vec::new();
    let mut stamps: Vec<usize> = Vec::new();
    let mut add = |dll: &[u8], field: usize, names: &mut Vec<u8>| -> bool {
        match rename(&String::from_utf8_lossy(dll)) {
            Some(new) => {
                fixups.push((field, names.len() as u32));
                names.extend_from_slice(new.as_bytes());
                names.push(0);
                if names.len() % 2 == 1 {
                    names.push(0);
                }
                true
            }
            None => false,
        }
    };
    let image_base = pe.info().optional.image_base;
    if ndirs > 1 && rd32(&b, dir_at(1) + 4)? != 0 {
        let mut d = to_off(rd32(&b, dir_at(1))?)?;
        loop {
            let desc = b.get(d..d + 20).ok_or("import directory runs off the file")?;
            if desc.iter().all(|&x| x == 0) {
                break;
            }
            let name = to_off(rd32(&b, d + 12)?)?;
            let len = b[name..].iter().position(|&c| c == 0).ok_or("unterminated DLL name")?;
            if add(&b[name..name + len], d + 12, &mut names) && rd32(&b, d)? != 0 {
                stamps.push(d + 4);
            }
            d += 20;
        }
    }
    if ndirs > 13 && rd32(&b, dir_at(13) + 4)? != 0 {
        let mut d = to_off(rd32(&b, dir_at(13))?)?;
        loop {
            let desc = b.get(d..d + 32).ok_or("delay-load directory runs off the file")?;
            if desc.iter().all(|&x| x == 0) {
                break;
            }
            let attrs = rd32(&b, d)?;
            let raw = rd32(&b, d + 4)?;
            // Attribute bit 0: the fields are RVAs; old-style ones are VAs.
            let rva = if attrs & 1 != 0 { raw } else { (raw as u64).wrapping_sub(image_base) as u32 };
            if attrs & 1 == 0 {
                return Err("old-style (VA) delay-load descriptors are not supported".into());
            }
            let name = to_off(rva)?;
            let len = b[name..].iter().position(|&c| c == 0).ok_or("unterminated DLL name")?;
            add(&b[name..name + len], d + 4, &mut names);
            d += 32;
        }
    }
    if fixups.is_empty() {
        return Err("the image imports none of the DLLs to rename".into());
    }
    // Drop an Authenticode signature (it sits at the end of the file).
    if ndirs > 4 {
        let (off, size) = (rd32(&b, dir_at(4))? as usize, rd32(&b, dir_at(4) + 4)? as usize);
        if size != 0 {
            if off + size >= b.len() && off <= b.len() {
                b.truncate(off);
            }
            wr32(&mut b, dir_at(4), 0);
            wr32(&mut b, dir_at(4) + 4, 0);
        }
    }
    if ndirs > 11 {
        wr32(&mut b, dir_at(11), 0);
        wr32(&mut b, dir_at(11) + 4, 0);
    }
    for at in stamps {
        // A bound descriptor (time stamp set, lookup table present): unbind.
        wr32(&mut b, at, 0);
        wr32(&mut b, at + 4, 0);
    }
    // The new section: after the last one in memory, at the end of the file.
    let va = sections
        .iter()
        .map(|s| align_up(s.virtual_address + s.virtual_size.max(s.size_of_raw_data), sect_align))
        .max()
        .unwrap_or(sect_align);
    let raw = align_up(b.len() as u32, file_align);
    let raw_size = align_up(names.len() as u32, file_align);
    b.resize(raw as usize, 0);
    b.extend_from_slice(&names);
    b.resize((raw + raw_size) as usize, 0);
    let mut h = [0u8; 40];
    h[..8].copy_from_slice(b".ptname\0");
    h[8..12].copy_from_slice(&(names.len() as u32).to_le_bytes());
    h[12..16].copy_from_slice(&va.to_le_bytes());
    h[16..20].copy_from_slice(&raw_size.to_le_bytes());
    h[20..24].copy_from_slice(&raw.to_le_bytes());
    // IMAGE_SCN_CNT_INITIALIZED_DATA | IMAGE_SCN_MEM_READ (winnt.h)
    h[36..40].copy_from_slice(&0x4000_0040u32.to_le_bytes());
    b[new_hdr..new_hdr + 40].copy_from_slice(&h);
    b[fh + 2..fh + 4].copy_from_slice(&((nsect + 1) as u16).to_le_bytes());
    wr32(&mut b, opt + 56, align_up(va + names.len() as u32, sect_align));
    let init = rd32(&b, opt + 8)?;
    wr32(&mut b, opt + 8, init.wrapping_add(raw_size));
    for (field, at) in fixups {
        wr32(&mut b, field, va + at);
    }
    wr32(&mut b, checksum_at, 0);
    let sum = pe_checksum(&b, checksum_at);
    wr32(&mut b, checksum_at, sum);
    Ok(b)
}

// ----- building the kit -------------------------------------------------------------------

/// The kit's manifest.
pub const KIT_FILE: &str = "polytrace-kit.json";
/// Where the generated workspace and its build go.
fn work_dir() -> PathBuf {
    crate::root().join("target/wintrace")
}

fn sha256_hex(path: &Path) -> String {
    let out = Command::new("sha256sum").arg(path).output();
    out.ok()
        .and_then(|o| String::from_utf8_lossy(&o.stdout).split_whitespace().next().map(str::to_string))
        .unwrap_or_default()
}

fn output_of(cmd: &mut Command) -> Result<String> {
    let out = cmd.output().map_err(|e| format!("{cmd:?}: {e}"))?;
    if !out.status.success() {
        return Err(format!("{cmd:?}: {}", String::from_utf8_lossy(&out.stderr).trim()));
    }
    Ok(String::from_utf8_lossy(&out.stdout).trim().to_string())
}

/// Build the logger and the proxies of `proxies` (`-Zbuild-std`, as ADR
/// 0075's test programs); returns the directory with the DLLs.
fn build(proxies: &[Proxy]) -> Result<PathBuf> {
    let repo = crate::root();
    let rustc = std::env::var("RUSTC").unwrap_or_else(|_| "rustc".into());
    let sysroot = PathBuf::from(output_of(Command::new(&rustc).current_dir(&repo).args(["--print", "sysroot"]))?);
    let vendor = sysroot.join("lib/rustlib/src/rust/library/vendor");
    if !vendor.is_dir() {
        return Err(format!("{} not found: the rust-src component is needed", vendor.display()));
    }
    let gen_dir = work_dir().join("build");
    generate(&gen_dir, &repo, proxies)?;
    let target_dir = work_dir().join("cargo");
    let flags = [
        "-Clinker=rust-lld".to_string(),
        "-Clinker-flavor=lld-link".into(),
        "-Clink-arg=/NODEFAULTLIB".into(),
        "-Clink-arg=/Brepro".into(),
        "-Clink-arg=/DEBUG:NONE".into(),
        "-Cdebuginfo=0".into(),
        format!("--remap-path-prefix={}=/poly", repo.display()),
        format!("--remap-path-prefix={}=/rust", sysroot.display()),
    ]
    .join("\u{1f}");
    eprintln!("==> build polytrace.dll and {} proxy DLLs", proxies.len());
    let status = Command::new(std::env::var("CARGO").unwrap_or_else(|_| "cargo".into()))
        .current_dir(&repo)
        .env("RUSTC_BOOTSTRAP", "1")
        .env("CARGO_INCREMENTAL", "0")
        .env("CARGO_ENCODED_RUSTFLAGS", flags)
        .env_remove("RUSTFLAGS")
        .arg("build")
        .arg("--release")
        .arg("--manifest-path")
        .arg(gen_dir.join("Cargo.toml"))
        .args(["-Zbuild-std=core,compiler_builtins", "-Zbuild-std-features=compiler-builtins-mem"])
        .args(["--target", "x86_64-pc-windows-msvc"])
        .arg("--target-dir")
        .arg(&target_dir)
        .arg("--offline")
        .arg("--config")
        .arg("source.crates-io.replace-with=\"rustsrc\"")
        .arg("--config")
        .arg(format!("source.rustsrc.directory={:?}", vendor.to_string_lossy()))
        .status()
        .map_err(|e| format!("cargo: {e}"))?;
    if !status.success() {
        return Err(format!("building the tracer's DLLs failed ({status})"));
    }
    Ok(target_dir.join("x86_64-pc-windows-msvc/release"))
}

/// What [`build_kit`] made.
#[derive(Clone, Debug)]
pub struct Kit {
    pub dir: PathBuf,
    pub exe: String,
    pub proxies: Vec<Proxy>,
}

/// `trace-kit`: the instrumented copy of `exe`, its proxies and the logger
/// in `out` (emptied first).
pub fn build_kit(exe: &Path, out: &Path, data: &[String]) -> Result<Kit> {
    let bytes = std::fs::read(exe).map_err(|e| format!("{}: {e}", exe.display()))?;
    let imports = crate::winimports::imports_of(&bytes)?;
    let proxies = plan(&imports, data)?;
    let renamed = rename_imports(&bytes, |dll| {
        let low = dll.to_ascii_lowercase();
        proxies.iter().find(|p| p.real == low).map(|p| p.file.clone())
    })?;
    let built = build(&proxies)?;
    let _ = std::fs::remove_dir_all(out);
    std::fs::create_dir_all(out).map_err(|e| format!("{}: {e}", out.display()))?;
    let name = exe.file_name().ok_or("no file name")?.to_string_lossy().into_owned();
    std::fs::write(out.join(&name), &renamed).map_err(|e| format!("{}: {e}", out.display()))?;
    let copy = |from: &str, to: &str| -> Result<()> {
        std::fs::copy(built.join(from), out.join(to)).map(|_| ()).map_err(|e| format!("{from}: {e}"))
    };
    copy("polytrace.dll", "polytrace.dll")?;
    for p in &proxies {
        copy(&format!("{}.dll", p.crate_name), &p.file)?;
    }
    let mut files = serde_json::Map::new();
    for f in
        std::iter::once(name.clone()).chain(["polytrace.dll".to_string()]).chain(proxies.iter().map(|p| p.file.clone()))
    {
        files.insert(f.clone(), Value::String(sha256_hex(&out.join(&f))));
    }
    let manifest = serde_json::json!({
        "format": "polytrace-kit/1",
        "program": name,
        "program_sha256": sha256_hex(exe),
        "files": files,
        "proxies": proxies.iter().map(|p| serde_json::json!({
            "dll": p.real,
            "file": p.file,
            "functions": p.funcs.iter().map(|f| serde_json::json!({
                "name": f.sym.to_string(),
                "signature": f.sig,
                "flags": f.flags,
            })).collect::<Vec<_>>(),
            "forwarded": p.data,
        })).collect::<Vec<_>>(),
        "run": "run the program in this directory; it writes polytrace.jsonl in its current directory (POLYTRACE_OUT names another file)",
    });
    let text = serde_json::to_string_pretty(&manifest).map_err(|e| e.to_string())? + "\n";
    std::fs::write(out.join(KIT_FILE), text).map_err(|e| format!("{KIT_FILE}: {e}"))?;
    Ok(Kit { dir: out.to_path_buf(), exe: name, proxies })
}

// ----- comparing traces -------------------------------------------------------------------

/// One recorded call.
#[derive(Clone, Debug, PartialEq)]
pub struct Call {
    pub seq: u64,
    pub t: u64,
    pub dll: String,
    pub func: String,
    pub args: Vec<Value>,
    /// `Value::Null` for `void` and calls that did not return.
    pub ret: Value,
    pub err: Option<u64>,
}

impl Call {
    fn key(&self) -> (String, &str) {
        (self.dll.to_ascii_lowercase(), self.func.as_str())
    }
}

impl std::fmt::Display for Call {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let args: Vec<String> = self.args.iter().map(Value::to_string).collect();
        write!(f, "{}!{}({}) = {}", self.dll, self.func, args.join(", "), self.ret)?;
        if let Some(e) = self.err {
            write!(f, " [last error {e}]")?;
        }
        write!(f, "  (seq {})", self.seq)
    }
}

/// The calls of a `polytrace.jsonl` (header lines skipped; an unfinished
/// last line, as a cut-off file has, is ignored).
pub fn parse_trace(text: &str) -> Result<Vec<Call>> {
    let lines: Vec<&str> = text.lines().filter(|l| !l.trim().is_empty()).collect();
    let mut out = Vec::new();
    for (i, line) in lines.iter().enumerate() {
        let v: Value = match serde_json::from_str(line) {
            Ok(v) => v,
            Err(_) if i + 1 == lines.len() => break,
            Err(e) => return Err(format!("line {}: {e}", i + 1)),
        };
        let Some(seq) = v.get("seq").and_then(Value::as_u64) else { continue };
        let s = |k: &str| v.get(k).and_then(Value::as_str).unwrap_or_default().to_string();
        out.push(Call {
            seq,
            t: v.get("t").and_then(Value::as_u64).unwrap_or(0),
            dll: s("dll"),
            func: s("fn"),
            args: v.get("args").and_then(Value::as_array).cloned().unwrap_or_default(),
            ret: v.get("ret").cloned().unwrap_or(Value::Null),
            err: v.get("err").and_then(Value::as_u64),
        });
    }
    Ok(out)
}

/// What is compared of a value.
#[derive(Clone, Debug, PartialEq, Eq)]
enum Norm {
    Num(u64),
    Null,
    /// A pointer or handle: NULL, all ones (`INVALID_HANDLE_VALUE`, -1),
    /// or something else, whose value is not compared.
    NullPtr,
    AllOnes,
    Ptr,
    Str(String),
    Other(String),
}

fn norm(v: &Value, map: &[(String, String)]) -> Norm {
    match v {
        Value::Null => Norm::Null,
        Value::Number(n) => n.as_u64().map_or(Norm::Other(n.to_string()), Norm::Num),
        Value::String(s) => match s.strip_prefix("0x").map(|h| u64::from_str_radix(h, 16)) {
            Some(Ok(0)) => Norm::NullPtr,
            Some(Ok(u64::MAX)) => Norm::AllOnes,
            Some(Ok(_)) => Norm::Ptr,
            _ => Norm::Other(s.clone()),
        },
        Value::Object(o) => match o.get("w").or_else(|| o.get("a")).and_then(Value::as_str) {
            Some(s) => {
                let mut s = s.to_string();
                for (from, to) in map {
                    s = s.replace(from.as_str(), to.as_str());
                }
                Norm::Str(s)
            }
            None => Norm::Other(v.to_string()),
        },
        _ => Norm::Other(v.to_string()),
    }
}

/// Functions whose results differ from run to run (times, ids).
const VOLATILE: &[&str] = &[
    "GetTickCount",
    "GetTickCount64",
    "GetCurrentProcessId",
    "GetCurrentThreadId",
    "QueryPerformanceCounter",
    "QueryPerformanceFrequency",
    "GetSystemTimeAsFileTime",
];

/// Results that describe the machine, not the system's behaviour: two
/// correct systems on different machines differ (ADR 0110). `GetOEMCP`:
/// the OEM code page of the machine's locale (850 on the oracle, 437 on
/// Poly; ADR 0088 lists it as a machine fact). Not compared, like
/// [`VOLATILE`]'s.
const MACHINE_RESULTS: &[&str] = &["GetOEMCP"];

/// Functions taken out of both traces before they are aligned (ADR 0110):
/// `IsProcessorFeaturePresent` answers for the processor (`PF_XSAVE_ENABLED`
/// is 1 on the oracle, 0 on QEMU's CPU), and how often a program asks
/// follows `CPUID`, which is not a traced call.
const MACHINE_CALLS: &[&str] = &["IsProcessorFeaturePresent"];

/// Arguments of which only the low byte is the argument (ADR 0110):
/// `memset`'s fill value is converted to `unsigned char` (Learn: "memset");
/// the caller may leave anything in the register's other bits.
const BYTE_ARGS: &[(&str, usize)] = &[("memset", 1)];

/// The documented normalisations of the comparison (ADR 0110), applied to
/// both traces before [`diff_traces`]: [`MACHINE_CALLS`] left out,
/// `GetVersion`'s result as its major and minor version only (the build
/// number in its high word is the machine's Windows build), [`BYTE_ARGS`]
/// masked. Results of [`VOLATILE`] and [`MACHINE_RESULTS`] functions are
/// not compared either (in [`differences`] and [`canonical`]).
pub fn normalise(calls: &[Call]) -> Vec<Call> {
    calls
        .iter()
        .filter(|c| !MACHINE_CALLS.contains(&c.func.as_str()))
        .map(|c| {
            let mut c = c.clone();
            if c.func == "GetVersion"
                && let Some(v) = c.ret.as_u64()
            {
                c.ret = Value::from(format!("version {}.{}", v & 0xff, (v >> 8) & 0xff));
            }
            for &(f, i) in BYTE_ARGS {
                if c.func == f
                    && let Some(v) = c.args.get(i).and_then(Value::as_u64)
                {
                    c.args[i] = Value::from(v & 0xff);
                }
            }
            c
        })
        .collect()
}

/// What the comparison sees of a value, as a value: numbers as they are,
/// strings with `map` applied (`{"w":…}` / `{"a":…}`), pointers and
/// handles as `"NULL"`, `"-1"` (all ones) or `"ptr"`. Two values compare
/// equal in [`diff_traces`] exactly when their canonical forms are equal,
/// and canonical forms compare among themselves the same way, so a trace
/// can be kept in this form (the committed reference of ADR 0110).
pub fn canonical_value(v: &Value, map: &[(String, String)]) -> Value {
    match norm(v, map) {
        Norm::Num(n) => Value::from(n),
        Norm::Null => Value::Null,
        Norm::NullPtr => Value::from("NULL"),
        Norm::AllOnes => Value::from("-1"),
        Norm::Ptr => Value::from("ptr"),
        Norm::Str(s) => {
            let key = if v.get("w").is_some() { "w" } else { "a" };
            serde_json::json!({ key: s })
        }
        Norm::Other(_) => v.clone(),
    }
}

/// `calls` in canonical form: [`normalise`]d, per thread in call order,
/// `seq` renumbered from 0 in each thread (the call's index in its
/// thread), every value [`canonical_value`], and the results
/// [`diff_traces`] does not compare ([`VOLATILE`], [`MACHINE_RESULTS`])
/// replaced by `"*"`.
pub fn canonical(calls: &[Call], map: &[(String, String)]) -> Vec<Call> {
    let calls = normalise(calls);
    let mut sorted: Vec<&Call> = calls.iter().collect();
    sorted.sort_by_key(|c| (c.t, c.seq));
    let mut index: BTreeMap<u64, u64> = BTreeMap::new();
    sorted
        .into_iter()
        .map(|c| {
            let n = index.entry(c.t).or_default();
            let seq = *n;
            *n += 1;
            Call {
                seq,
                t: c.t,
                dll: c.dll.to_ascii_lowercase(),
                func: c.func.clone(),
                args: c.args.iter().map(|a| canonical_value(a, map)).collect(),
                ret: if volatile(c) { Value::from("*") } else { canonical_value(&c.ret, map) },
                err: c.err,
            }
        })
        .collect()
}

/// Whether `c`'s result is not compared.
fn volatile(c: &Call) -> bool {
    VOLATILE.contains(&c.func.as_str()) || MACHINE_RESULTS.contains(&c.func.as_str())
}

/// How two calls of the same function differ (empty: they do not).
fn differences(a: &Call, b: &Call, map_a: &[(String, String)], map_b: &[(String, String)]) -> Vec<String> {
    let mut out = Vec::new();
    if !volatile(a) && norm(&a.ret, map_a) != norm(&b.ret, map_b) {
        out.push(format!("result {} vs {}", a.ret, b.ret));
    }
    if a.err != b.err {
        let e = |v: Option<u64>| v.map_or_else(|| "unchanged".to_string(), |e| e.to_string());
        out.push(format!("last error {} vs {}", e(a.err), e(b.err)));
    }
    for (i, (x, y)) in a.args.iter().zip(&b.args).enumerate() {
        if norm(x, map_a) != norm(y, map_b) {
            out.push(format!("argument {} {} vs {}", i + 1, x, y));
        }
    }
    out
}

/// One place where two traces part.
#[derive(Clone, Debug, PartialEq)]
pub enum Divergence {
    /// Calls only one side made (the other side's next matching call is
    /// further on); `before`: the last call both made.
    Extra { thread: u64, only_a: Vec<Call>, only_b: Vec<Call>, before: Option<Call> },
    /// The same function, called differently or with another outcome.
    Differs { thread: u64, a: Call, b: Call, what: Vec<String> },
}

/// The comparison of two traces.
#[derive(Clone, Debug, Default)]
pub struct TraceDiff {
    /// Calls per DLL (lower case): A's and B's counts.
    pub per_dll: BTreeMap<String, (usize, usize)>,
    /// Per thread: calls in A, in B, and calls matched with no difference.
    pub threads: BTreeMap<u64, (usize, usize, usize)>,
    pub divergences: Vec<Divergence>,
}

/// How far ahead a resynchronisation looks.
const WINDOW: usize = 64;

/// Compare traces `a` and `b`, thread by thread in call order; `map_a` /
/// `map_b` replace text in string arguments first (e.g. each side's
/// working directory by the same word).
pub fn diff_traces(a: &[Call], b: &[Call], map_a: &[(String, String)], map_b: &[(String, String)]) -> TraceDiff {
    let mut d = TraceDiff::default();
    for c in a {
        d.per_dll.entry(c.dll.to_ascii_lowercase()).or_default().0 += 1;
    }
    for c in b {
        d.per_dll.entry(c.dll.to_ascii_lowercase()).or_default().1 += 1;
    }
    let by_thread = |calls: &[Call]| {
        let mut m: BTreeMap<u64, Vec<Call>> = BTreeMap::new();
        for c in calls {
            m.entry(c.t).or_default().push(c.clone());
        }
        for v in m.values_mut() {
            v.sort_by_key(|c| c.seq);
        }
        m
    };
    let (ta, tb) = (by_thread(a), by_thread(b));
    let mut threads: Vec<u64> = ta.keys().chain(tb.keys()).copied().collect();
    threads.sort_unstable();
    threads.dedup();
    let empty = Vec::new();
    for t in threads {
        let (xa, xb) = (ta.get(&t).unwrap_or(&empty), tb.get(&t).unwrap_or(&empty));
        let (mut i, mut j, mut same) = (0, 0, 0);
        let mut before: Option<Call> = None;
        while i < xa.len() && j < xb.len() {
            if xa[i].key() == xb[j].key() {
                let what = differences(&xa[i], &xb[j], map_a, map_b);
                if what.is_empty() {
                    same += 1;
                } else {
                    d.divergences.push(Divergence::Differs { thread: t, a: xa[i].clone(), b: xb[j].clone(), what });
                }
                before = Some(xa[i].clone());
                i += 1;
                j += 1;
                continue;
            }
            // The nearest pair (i + di, j + dj) that matches again.
            let mut found = None;
            'search: for k in 1..=2 * WINDOW {
                for di in 0..=k.min(WINDOW) {
                    let dj = k - di;
                    if dj > WINDOW || i + di >= xa.len() || j + dj >= xb.len() {
                        continue;
                    }
                    if xa[i + di].key() == xb[j + dj].key() {
                        found = Some((di, dj));
                        break 'search;
                    }
                }
            }
            let (di, dj) = found.unwrap_or((xa.len() - i, xb.len() - j));
            d.divergences.push(Divergence::Extra {
                thread: t,
                only_a: xa[i..i + di].to_vec(),
                only_b: xb[j..j + dj].to_vec(),
                before: before.clone(),
            });
            i += di;
            j += dj;
        }
        if i < xa.len() || j < xb.len() {
            d.divergences.push(Divergence::Extra {
                thread: t,
                only_a: xa[i..].to_vec(),
                only_b: xb[j..].to_vec(),
                before: before.clone(),
            });
        }
        d.threads.insert(t, (xa.len(), xb.len(), same));
    }
    d
}

/// A readable report of `d`, at most `limit` divergences in full.
pub fn report(d: &TraceDiff, name_a: &str, name_b: &str, limit: usize) -> String {
    let mut s = String::new();
    let (na, nb): (usize, usize) = d.per_dll.values().fold((0, 0), |acc, &(x, y)| (acc.0 + x, acc.1 + y));
    let _ = writeln!(s, "A: {name_a} ({na} calls)\nB: {name_b} ({nb} calls)\n");
    let _ = writeln!(s, "{:<24} {:>8} {:>8}", "calls per DLL", "A", "B");
    for (dll, (x, y)) in &d.per_dll {
        let _ = writeln!(s, "{dll:<24} {x:>8} {y:>8}");
    }
    let _ = writeln!(s);
    for (t, (x, y, same)) in &d.threads {
        let _ = writeln!(s, "thread {t}: {x} calls in A, {y} in B, {same} matched without a difference");
    }
    let _ = writeln!(s, "\n{} divergences{}", d.divergences.len(), if d.divergences.is_empty() { "" } else { ":" });
    const SHOWN: usize = 6;
    for (n, div) in d.divergences.iter().take(limit).enumerate() {
        match div {
            Divergence::Differs { thread, a, b, what } => {
                let _ = writeln!(s, "\n{}. thread {thread}: {}!{}: {}", n + 1, a.dll, a.func, what.join("; "));
                let _ = writeln!(s, "   A: {a}\n   B: {b}");
            }
            Divergence::Extra { thread, only_a, only_b, before } => {
                let _ = writeln!(
                    s,
                    "\n{}. thread {thread}: {} calls only in A, {} only in B",
                    n + 1,
                    only_a.len(),
                    only_b.len()
                );
                if let Some(c) = before {
                    let _ = writeln!(s, "   after: {c}");
                }
                for (side, calls) in [("A", only_a), ("B", only_b)] {
                    for c in calls.iter().take(SHOWN) {
                        let _ = writeln!(s, "   {side}: {c}");
                    }
                    if calls.len() > SHOWN {
                        let _ = writeln!(s, "   {side}: ... {} more", calls.len() - SHOWN);
                    }
                }
            }
        }
    }
    if d.divergences.len() > limit {
        let _ = writeln!(s, "\n... {} more divergences", d.divergences.len() - limit);
    }
    s
}

// ----- the command ------------------------------------------------------------------------

const USAGE: &str = "usage: cargo xtask win trace-kit <exe | 7zip> [--out <dir>] [--data <name>]...\n       \
                     cargo xtask win trace-diff <a.jsonl> <b.jsonl> [--map-a <from>=<to>] [--map-b <from>=<to>] [--limit <n>] [--check]";

fn mapping(s: &str) -> Result<(String, String)> {
    s.split_once('=').map(|(a, b)| (a.to_string(), b.to_string())).ok_or_else(|| format!("{s:?}: expected <from>=<to>"))
}

/// `cargo xtask win trace-kit ...`.
pub fn trace_kit_command(args: &[String]) -> Result {
    let mut exe = None;
    let mut out = None;
    let mut data = Vec::new();
    let mut it = args.iter();
    while let Some(a) = it.next() {
        match a.as_str() {
            "--out" => out = Some(PathBuf::from(it.next().ok_or(USAGE)?)),
            "--data" => data.push(it.next().ok_or(USAGE)?.clone()),
            s if exe.is_none() && !s.starts_with("--") => exe = Some(s.to_string()),
            _ => return Err(USAGE.into()),
        }
    }
    let exe = match exe.as_deref() {
        Some("7zip") => {
            crate::sevenzip::cached().ok_or("7za.exe is not in the cache; run `cargo xtask win fetch-7zip`")?
        }
        Some(p) => PathBuf::from(p),
        None => return Err(USAGE.into()),
    };
    let out = out.unwrap_or_else(|| work_dir().join("kit"));
    let kit = build_kit(&exe, &out, &data)?;
    for p in &kit.proxies {
        let unknown: Vec<String> = p.funcs.iter().filter(|f| f.sig.is_empty()).map(|f| f.sym.to_string()).collect();
        eprintln!(
            "    {} for {}: {} functions, {} forwarded variables{}",
            p.file,
            p.real,
            p.funcs.len(),
            p.data.len(),
            if unknown.is_empty() {
                String::new()
            } else {
                format!("; no signature (4 arguments logged): {}", unknown.join(", "))
            }
        );
    }
    eprintln!("==> trace kit for {} in {}", kit.exe, kit.dir.display());
    Ok(())
}

/// `cargo xtask win trace-diff ...`.
pub fn trace_diff_command(args: &[String]) -> Result {
    let (mut files, mut map_a, mut map_b, mut limit, mut check) = (Vec::new(), Vec::new(), Vec::new(), 10, false);
    let mut it = args.iter();
    while let Some(a) = it.next() {
        match a.as_str() {
            "--map-a" => map_a.push(mapping(it.next().ok_or(USAGE)?)?),
            "--map-b" => map_b.push(mapping(it.next().ok_or(USAGE)?)?),
            "--limit" => limit = it.next().ok_or(USAGE)?.parse().map_err(|_| USAGE)?,
            "--check" => check = true,
            s if !s.starts_with("--") => files.push(s.to_string()),
            _ => return Err(USAGE.into()),
        }
    }
    let [fa, fb] = files.as_slice() else { return Err(USAGE.into()) };
    let read = |f: &str| -> Result<Vec<Call>> {
        let text = std::fs::read_to_string(f).map_err(|e| format!("{f}: {e}"))?;
        parse_trace(&text).map_err(|e| format!("{f}: {e}"))
    };
    let d = diff_traces(&normalise(&read(fa)?), &normalise(&read(fb)?), &map_a, &map_b);
    print!("{}", report(&d, fa, fb, limit));
    if check && !d.divergences.is_empty() {
        return Err(format!("the traces differ ({} divergences)", d.divergences.len()));
    }
    Ok(())
}

// ----- the boot test ----------------------------------------------------------------------

/// The boot test `wintrace`: the traced 7-Zip scenario
/// (`sevenzip_trace.rs`, ADR 0110), which runs ADR 0088's steps under the
/// tracer and compares each step's trace with the committed Windows
/// reference (report only until `TRACE_REQUIRED`). It replaced ADR 0090's
/// three single runs.
pub fn boot_test(kernel: &Path, _syms: &std::sync::Arc<crate::symbols::Symbols>) -> Result {
    crate::sevenzip_trace::boot_test(kernel)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn imp(dll: &str, sym: Sym) -> Import {
        Import { dll: dll.into(), sym, delay: false }
    }
    fn name(n: &str) -> Sym {
        Sym::Name(n.into())
    }

    #[test]
    fn plans_one_proxy_per_dll_with_variables_forwarded() {
        let imports = [
            imp("KERNEL32.dll", name("CreateFileW")),
            imp("msvcrt.dll", name("_iob")),
            imp("msvcrt.dll", name("memcpy")),
            imp("KERNEL32.dll", name("ExitProcess")),
            imp("OLEAUT32.dll", Sym::Ordinal(2)),
            imp("msvcrt.dll", name("memcpy")),
            imp("msvcrt.dll", name("_CxxThrowException")),
            imp("msvcrt.dll", name("my_var")),
            imp("kernel32.dll", name("FooBar")),
        ];
        let p = plan(&imports, &["my_var".to_string()]).unwrap();
        assert_eq!(p.len(), 3);
        assert_eq!(
            (p[0].real.as_str(), p[0].file.as_str(), p[0].crate_name.as_str()),
            ("kernel32.dll", "pt_kernel32.dll", "pt_kernel32")
        );
        assert_eq!(p[0].funcs.len(), 3);
        assert_eq!(p[0].funcs[0].sig, "h=wiipiih");
        assert_eq!(p[0].funcs[1].flags, F_NORETURN);
        assert_eq!(p[0].funcs[2].sig, "", "an unknown function has no signature");
        assert_eq!(p[1].data, ["_iob", "my_var"]);
        assert_eq!(p[1].funcs.len(), 2, "duplicates are exported once");
        assert_eq!(p[1].funcs[1].flags, F_TAIL);
        // oleaut32's ordinal 2 is SysAllocString.
        assert_eq!(p[2].funcs[0].sig, "p=w");
        assert!(plan(&[imp("pt_kernel32.dll", name("X"))], &[]).is_err());
        assert!(plan(&[imp("x.drv", name("X"))], &[]).is_err());
        assert!(plan(&[imp("a.dll", name("bad\"name"))], &[]).is_err());
    }

    #[test]
    fn generates_thunks_tables_and_exports() {
        let imports = [
            imp("msvcrt.dll", name("??1type_info@@UEAA@XZ")),
            imp("msvcrt.dll", name("_fmode")),
            imp("msvcrt.dll", name("exit")),
        ];
        let p = &plan(&imports, &[]).unwrap()[0];
        let lib = lib_rs(p, Path::new("/r/tools/wintrace/proxy/common.rs"));
        assert!(lib.contains("#[path = \"/r/tools/wintrace/proxy/common.rs\"]"));
        assert!(lib.contains("#[link(name = \"msvcrt\", kind = \"raw-dylib\")]"));
        assert!(lib.contains("#[link_name = \"??1type_info@@UEAA@XZ\"]\n    fn real_0();"));
        assert!(lib.contains("pt_f0:\n    .quad \"__imp_??1type_info@@UEAA@XZ\""));
        assert!(lib.contains(
            "pt_f1:\n    .quad \"__imp_exit\"\n    .quad pt_dll\n    .quad pt_n1\n    .quad pt_g1\n    .long 1\n"
        ));
        assert!(lib.contains("pt_g1:\n    .asciz \"v=i\""));
        assert!(lib.contains("pt_t1:\n    lea r10, [rip + pt_f1]\n    jmp pt_common"));
        // The raw string survives names with `#`.
        assert!(lib.contains("r##\"") && lib.ends_with("\"##\n);\n"));
        assert_eq!(
            link_args(p),
            [
                "/ENTRY:pt_proxy_main",
                "/EXPORT:??1type_info@@UEAA@XZ=pt_t0",
                "/EXPORT:exit=pt_t1",
                "/EXPORT:_fmode=msvcrt._fmode"
            ]
        );
        let o = &plan(&[imp("OLEAUT32.dll", Sym::Ordinal(9))], &[]).unwrap()[0];
        assert_eq!(link_args(o)[1], "/EXPORT:pt_ordinal_9=pt_t0,@9,NONAME");
        let lib = lib_rs(o, Path::new("c.rs"));
        assert!(lib.contains("#[link_ordinal(9)]\n    fn real_0();"));
        assert!(lib.contains(".quad \"__imp_real_0\""));
        assert!(lib.contains("pt_n0:\n    .asciz \"#9\""));
        let b = build_rs(o);
        assert!(b.contains("println!(\"cargo:rustc-link-arg=/EXPORT:pt_ordinal_9=pt_t0,@9,NONAME\");"));
    }

    #[test]
    fn signatures_are_well_formed_and_unique() {
        let mut seen = std::collections::BTreeSet::new();
        for (n, s) in SIGNATURES {
            assert!(seen.insert(*n), "{n} twice");
            let (ret, args) = s.split_once('=').unwrap_or_else(|| panic!("{n}: {s}"));
            assert!(ret.len() == 1 && "iqscphwav".contains(ret), "{n}: {s}");
            assert!(args.len() <= 20 && args.chars().all(|c| "iqscphwa".contains(c)), "{n}: {s}");
        }
        assert_eq!((stack_slots("h=wiipiih"), stack_slots("i="), stack_slots("")), (3, 0, 16));
        for n in NORETURN.iter().chain(TAIL).chain(FLUSH) {
            assert!(!DATA.contains(n), "{n}");
        }
    }

    /// A minimal PE32+ image: headers, `.rdata` with an import directory
    /// for `dlls` (each importing `F` by name), optionally a signature.
    fn image(dlls: &[&str], signed: bool) -> Vec<u8> {
        let mut b = vec![0u8; 0x400];
        b[0..2].copy_from_slice(b"MZ");
        b[0x3C..0x40].copy_from_slice(&0x80u32.to_le_bytes());
        b[0x80..0x84].copy_from_slice(b"PE\0\0");
        let fh = 0x84;
        b[fh..fh + 2].copy_from_slice(&0x8664u16.to_le_bytes());
        b[fh + 2..fh + 4].copy_from_slice(&1u16.to_le_bytes());
        b[fh + 16..fh + 18].copy_from_slice(&240u16.to_le_bytes());
        b[fh + 18..fh + 20].copy_from_slice(&0x22u16.to_le_bytes());
        let opt = fh + 20;
        b[opt..opt + 2].copy_from_slice(&0x20Bu16.to_le_bytes());
        b[opt + 24..opt + 32].copy_from_slice(&0x1_4000_0000u64.to_le_bytes());
        b[opt + 32..opt + 36].copy_from_slice(&0x1000u32.to_le_bytes());
        b[opt + 36..opt + 40].copy_from_slice(&0x200u32.to_le_bytes());
        b[opt + 40..opt + 42].copy_from_slice(&6u16.to_le_bytes());
        b[opt + 48..opt + 50].copy_from_slice(&6u16.to_le_bytes());
        b[opt + 56..opt + 60].copy_from_slice(&0x2000u32.to_le_bytes());
        b[opt + 60..opt + 64].copy_from_slice(&0x400u32.to_le_bytes());
        b[opt + 68..opt + 70].copy_from_slice(&3u16.to_le_bytes());
        b[opt + 108..opt + 112].copy_from_slice(&16u32.to_le_bytes());
        // .rdata at RVA 0x1000, file 0x400, 0x200 bytes.
        let mut sec = vec![0u8; 0x200];
        let n = dlls.len();
        let desc = 0usize;
        let mut at = 20 * (n + 1);
        for (i, dll) in dlls.iter().enumerate() {
            let ilt = at;
            sec[ilt..ilt + 8].copy_from_slice(&(0x1000 + ilt as u64 + 32).to_le_bytes());
            let iat = ilt + 16;
            sec[iat..iat + 8].copy_from_slice(&(0x1000 + ilt as u64 + 32).to_le_bytes());
            let hint = ilt + 32;
            sec[hint + 2] = b'F';
            let name = hint + 8;
            sec[name..name + dll.len()].copy_from_slice(dll.as_bytes());
            let d = desc + 20 * i;
            sec[d..d + 4].copy_from_slice(&(0x1000 + ilt as u32).to_le_bytes());
            sec[d + 12..d + 16].copy_from_slice(&(0x1000 + name as u32).to_le_bytes());
            sec[d + 16..d + 20].copy_from_slice(&(0x1000 + iat as u32).to_le_bytes());
            at = name + 24;
        }
        let dir = |b: &mut Vec<u8>, i: usize, rva: u32, size: u32| {
            b[opt + 112 + 8 * i..opt + 116 + 8 * i].copy_from_slice(&rva.to_le_bytes());
            b[opt + 116 + 8 * i..opt + 120 + 8 * i].copy_from_slice(&size.to_le_bytes());
        };
        dir(&mut b, 1, 0x1000, 20 * (n as u32 + 1));
        let t = opt + 240;
        b[t..t + 8].copy_from_slice(b".rdata\0\0");
        b[t + 8..t + 12].copy_from_slice(&0x200u32.to_le_bytes());
        b[t + 12..t + 16].copy_from_slice(&0x1000u32.to_le_bytes());
        b[t + 16..t + 20].copy_from_slice(&0x200u32.to_le_bytes());
        b[t + 20..t + 24].copy_from_slice(&0x400u32.to_le_bytes());
        b[t + 36..t + 40].copy_from_slice(&0x4000_0040u32.to_le_bytes());
        b.extend_from_slice(&sec);
        if signed {
            let off = b.len() as u32;
            b.extend_from_slice(&[0xAA; 16]);
            dir(&mut b, 4, off, 16);
        }
        b
    }

    #[test]
    fn renames_only_the_dll_names() {
        let orig = image(&["KERNEL32.dll", "msvcrt.dll", "keep.dll"], true);
        let before = crate::winimports::imports_of(&orig).unwrap();
        assert_eq!(before.len(), 3);
        let renamed =
            rename_imports(&orig, |d| (d != "keep.dll").then(|| format!("pt_{}", d.to_ascii_lowercase()))).unwrap();
        let after = crate::winimports::imports_of(&renamed).unwrap();
        let dlls: Vec<&str> = after.iter().map(|i| i.dll.as_str()).collect();
        assert_eq!(dlls, ["pt_kernel32.dll", "pt_msvcrt.dll", "keep.dll"]);
        assert!(after.iter().all(|i| i.sym == Sym::Name("F".into())));
        let pe = poly_pe::Pe::parse(&renamed).unwrap();
        let secs: Vec<_> = pe.sections().iter().collect();
        assert_eq!(secs.len(), 2);
        assert_eq!(secs[1].short_name(), b".ptname");
        assert_eq!((secs[1].virtual_address, secs[1].pointer_to_raw_data), (0x2000, 0x600));
        assert_eq!(pe.info().optional.size_of_image, 0x3000);
        // The signature is gone, the section's data is where it was, and
        // the original section is untouched but for the name fields.
        assert_eq!(renamed.len(), 0x800);
        assert_eq!(&renamed[0x400 + 0x80..0x400 + 0x200], &orig[0x400 + 0x80..0x400 + 0x200]);
        let ck = 0x84 + 20 + 64;
        let sum = u32::from_le_bytes(renamed[ck..ck + 4].try_into().unwrap());
        assert_eq!(sum, pe_checksum(&renamed, ck));
        assert_ne!(sum, 0);
        // Nothing to rename, or no room for a header.
        assert!(rename_imports(&orig, |_| None).is_err());
        let mut full = orig.clone();
        full[0x84 + 20 + 240 + 40] = 1;
        assert!(rename_imports(&full, |d| Some(d.into())).is_err());
    }

    #[test]
    fn checksum_matches_a_hand_computed_one() {
        // Words 0x0201, 0x0403 (the field at 4..8 skipped), 0x0009; length 9.
        let b = [1, 2, 3, 4, 0xFF, 0xFF, 0xFF, 0xFF, 9];
        assert_eq!(pe_checksum(&b, 4), 0x0201 + 0x0403 + 0x0009 + 9);
        // The carry folds back in.
        let b = [0xFF, 0xFF, 0x02, 0x00];
        assert_eq!(pe_checksum(&b, 100), 0x0001 + 1 + 4);
    }

    const A: &str = r#"{"format":"polytrace/1"}
{"seq":1,"t":0,"dll":"kernel32.dll","fn":"GetFileAttributesW","args":[{"w":"C:\\w\\x.txt"}],"ret":4294967295,"err":2}
{"seq":0,"t":0,"dll":"kernel32.dll","fn":"GetTickCount","args":[],"ret":123}
{"seq":2,"t":0,"dll":"msvcrt.dll","fn":"malloc","args":[16],"ret":"0x1000"}
{"seq":3,"t":1,"dll":"kernel32.dll","fn":"CloseHandle","args":["0x8"],"ret":1}
{"seq":4,"t":0,"dll":"kernel32.dll","fn":"ExitProcess","args":[0],"ret":null}
{"seq":5,"t":0,"dll":"kernel32.dll","fn":"Cut"#;

    const B: &str = r#"{"seq":10,"t":0,"dll":"kernel32.dll","fn":"GetTickCount","args":[],"ret":999}
{"seq":11,"t":0,"dll":"kernel32.dll","fn":"GetFileAttributesW","args":[{"w":"Z:\\7zip\\x.txt"}],"ret":32}
{"seq":12,"t":0,"dll":"msvcrt.dll","fn":"malloc","args":[16],"ret":"0x7f00"}
{"seq":13,"t":0,"dll":"msvcrt.dll","fn":"free","args":["0x7f00"],"ret":null}
{"seq":14,"t":0,"dll":"kernel32.dll","fn":"ExitProcess","args":[0],"ret":null}
{"seq":15,"t":1,"dll":"kernel32.dll","fn":"CloseHandle","args":["0x30"],"ret":1}
"#;

    #[test]
    fn parses_traces() {
        let a = parse_trace(A).unwrap();
        assert_eq!(a.len(), 5, "header skipped, cut-off last line ignored");
        assert_eq!(a[0].func, "GetFileAttributesW");
        assert_eq!(a[0].err, Some(2));
        assert_eq!(a[2].ret, Value::String("0x1000".into()));
        assert_eq!(a[4].ret, Value::Null);
        assert!(parse_trace("{bad\n{\"seq\":1}\n").is_err());
        assert_eq!(
            a[0].to_string(),
            "kernel32.dll!GetFileAttributesW({\"w\":\"C:\\\\w\\\\x.txt\"}) = 4294967295 [last error 2]  (seq 1)"
        );
    }

    #[test]
    fn normalises_machine_facts_and_byte_arguments() {
        let t = "{\"seq\":0,\"t\":0,\"dll\":\"kernel32.dll\",\"fn\":\"IsProcessorFeaturePresent\",\"args\":[17],\"ret\":1}\n\
                 {\"seq\":1,\"t\":0,\"dll\":\"kernel32.dll\",\"fn\":\"GetVersion\",\"args\":[],\"ret\":1717043210}\n\
                 {\"seq\":2,\"t\":0,\"dll\":\"msvcrt.dll\",\"fn\":\"memset\",\"args\":[\"0x10\",4294775809,1],\"ret\":\"0x10\"}\n\
                 {\"seq\":3,\"t\":0,\"dll\":\"kernel32.dll\",\"fn\":\"GetOEMCP\",\"args\":[],\"ret\":850}\n";
        let n = normalise(&parse_trace(t).unwrap());
        let funcs: Vec<&str> = n.iter().map(|c| c.func.as_str()).collect();
        assert_eq!(funcs, ["GetVersion", "memset", "GetOEMCP"]);
        assert_eq!(n[0].ret, Value::from("version 10.0"));
        assert_eq!(n[1].args[1], Value::from(1));
        // Another build, another register's junk, another code page: the same.
        let other = t.replace("1717043210", "1248133130").replace("4294775809", "1").replace("850", "437");
        let d = diff_traces(&n, &normalise(&parse_trace(&other).unwrap()), &[], &[]);
        assert!(d.divergences.is_empty(), "{d:?}");
        let c = canonical(&parse_trace(t).unwrap(), &[]);
        assert_eq!((c[2].ret.clone(), c[1].args[0].clone()), (Value::from("*"), Value::from("ptr")));
    }

    #[test]
    fn diffs_align_by_thread_and_ignore_pointers() {
        let (a, b) = (parse_trace(A).unwrap(), parse_trace(B).unwrap());
        let map_a = [("C:\\w".to_string(), "<dir>".to_string())];
        let map_b = [("Z:\\7zip".to_string(), "<dir>".to_string())];
        let d = diff_traces(&a, &b, &map_a, &map_b);
        assert_eq!(d.per_dll["kernel32.dll"], (4, 4));
        assert_eq!(d.per_dll["msvcrt.dll"], (1, 2));
        // Thread 0: GetTickCount (volatile) and malloc match; the
        // attributes differ; free is only in B. Thread 1: the handles are
        // both "some value".
        assert_eq!(d.threads[&0], (4, 5, 3));
        assert_eq!(d.threads[&1], (1, 1, 1));
        assert_eq!(d.divergences.len(), 2);
        match &d.divergences[0] {
            Divergence::Differs { what, a, .. } => {
                assert_eq!(a.func, "GetFileAttributesW");
                assert_eq!(what, &["result 4294967295 vs 32", "last error 2 vs unchanged"]);
            }
            other => panic!("{other:?}"),
        }
        match &d.divergences[1] {
            Divergence::Extra { only_a, only_b, before, .. } => {
                assert!(only_a.is_empty());
                assert_eq!(only_b.len(), 1);
                assert_eq!(only_b[0].func, "free");
                assert_eq!(before.as_ref().unwrap().func, "malloc");
            }
            other => panic!("{other:?}"),
        }
        // Without the path mapping the string argument differs too.
        let d = diff_traces(&a, &b, &[], &[]);
        let Divergence::Differs { what, .. } = &d.divergences[0] else { panic!() };
        assert_eq!(what.len(), 3);
        let text = report(&d, "a", "b", 1);
        assert!(text.contains("A: a (5 calls)\nB: b (6 calls)"), "{text}");
        assert!(text.contains("1. thread 0: kernel32.dll!GetFileAttributesW: result"), "{text}");
        assert!(text.contains("... 1 more divergences"), "{text}");
        // Identical traces: nothing to report.
        assert!(diff_traces(&a, &a, &[], &[]).divergences.is_empty());
    }

    #[test]
    fn diff_resynchronises_after_extra_calls() {
        let call = |seq, f: &str| Call {
            seq,
            t: 0,
            dll: "k.dll".into(),
            func: f.into(),
            args: vec![],
            ret: Value::Null,
            err: None,
        };
        let a: Vec<Call> = ["A", "B", "C", "D"].iter().enumerate().map(|(i, f)| call(i as u64, f)).collect();
        let b: Vec<Call> = ["A", "X", "Y", "C", "D", "E"].iter().enumerate().map(|(i, f)| call(i as u64, f)).collect();
        let d = diff_traces(&a, &b, &[], &[]);
        assert_eq!(d.threads[&0], (4, 6, 3));
        let kinds: Vec<(usize, usize)> = d
            .divergences
            .iter()
            .map(|x| match x {
                Divergence::Extra { only_a, only_b, .. } => (only_a.len(), only_b.len()),
                Divergence::Differs { .. } => (9, 9),
            })
            .collect();
        assert_eq!(kinds, [(1, 2), (0, 1)]);
    }
}

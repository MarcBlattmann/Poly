//! System DLLs Poly does not implement yet, generated (ADR 0155, step 10).
//!
//! The Windows applications of the survey import DLLs Poly lacks (`comdlg32`, `rpcrt4`, `setupapi`,
//! the Direct3D family ...), and a missing DLL ends a program before its first line, however little
//! of it the program uses. Each DLL in [`crate::sysdlls::SYSTEM_DLLS`] is made here, from its table,
//! with the PE builder (no toolchain): it exports the names the programs import (and, for a
//! few, names Learn lists), each a small stub that logs `stub: dll!name` (the same line
//! `polystub.dll` prints) and fails the way Learn documents for a function the system does not
//! provide, see [`Fail`]. A real implementation replaces the table entry by a crate in
//! `personalities/nt/win32`, which `xtask` builds instead.
//!
//! The DLL has no entry point and imports `PolyDebugPrint` and `RtlSetLastWin32Error` from `ntdll.dll`.
//! A forwarder (`psapi!EnumProcesses` is `kernel32!K32EnumProcesses`) is an export whose target names
//! another DLL (PE format, "Export Address Table").

use alloc::collections::{BTreeMap, BTreeSet};
use alloc::format;
use alloc::string::{String, ToString};
use alloc::vec::Vec;
use poly_ldr::builder::{Export, Import, PeBuilder, RDATA, Section, Sym, TEXT, Target};

/// `ERROR_CALL_NOT_IMPLEMENTED` (`winerror.h`).
const ERROR_CALL_NOT_IMPLEMENTED: u32 = 120;
/// `E_NOTIMPL` (`winerror.h`).
const E_NOTIMPL: u32 = 0x8000_4001;
/// `STATUS_NOT_IMPLEMENTED` (`ntstatus.h`).
const STATUS_NOT_IMPLEMENTED: u32 = 0xC000_0002;

/// What an export does when it is called, after it logs.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub enum Fail {
    /// 0 (`FALSE`, `NULL`) with the last error 120.
    Win32,
    /// `(HANDLE)-1` / `-1` with the last error 120.
    Invalid,
    /// `E_NOTIMPL`.
    Hresult,
    /// `STATUS_NOT_IMPLEMENTED`.
    Status,
    /// The function returns this (its documented error, or its documented answer for "nothing
    /// there"), the last error untouched.
    Value(u32),
    /// The last error, then this return value.
    Error(u32, u32),
    /// Store `store` (a `DWORD`) through the pointer that is argument `arg` (0 or 1) unless it is
    /// NULL, then return `ret`: a function that reports "no items" through an out parameter.
    Out { arg: u8, store: u32, ret: u32 },
}

/// One system DLL to make.
#[derive(Clone, Copy, Debug)]
pub struct SystemDll {
    /// The file name in `System32` (`winspool.drv`, `vulkan-1.dll`).
    pub file: &'static str,
    /// `(name, fail)`; a name `#<n>` is an export by ordinal `n` only.
    pub exports: &'static [(&'static str, Fail)],
    /// `(name, "dll.name")` forwarders (a name `#<n>` by ordinal).
    pub forwards: &'static [(&'static str, &'static str)],
}

const TEXT_RVA: u32 = 0x1000;
const COMMON_SIZE: usize = 0x40;
const STUB_SIZE: usize = 32;

/// The common routine of `fail` at `.text` offset `at`. Entered by `jmp` from a stub with RCX = the
/// text and RDX = its length, the original arguments of the two first parameters in their home
/// space (kinds that need them) and the return address on the stack (RSP = 8 mod 16).
fn common(fail: Fail, at: usize, iat_print: u32, iat_last_error: u32) -> Vec<u8> {
    let call = |code: &mut Vec<u8>, iat: u32| {
        let end = TEXT_RVA + (at + code.len() + 6) as u32;
        code.extend_from_slice(&[0xFF, 0x15]); // call [rip+rel32]
        code.extend_from_slice(&iat.wrapping_sub(end).to_le_bytes());
    };
    let set_error = |code: &mut Vec<u8>, error: u32| {
        code.push(0xB9); // mov ecx,error
        code.extend_from_slice(&error.to_le_bytes());
        call(code, iat_last_error);
    };
    let mov_eax = |code: &mut Vec<u8>, v: u32| {
        code.push(0xB8);
        code.extend_from_slice(&v.to_le_bytes());
    };
    let mut c: Vec<u8> = alloc::vec![0x48, 0x83, 0xEC, 0x28]; // sub rsp,0x28
    call(&mut c, iat_print);
    match fail {
        Fail::Win32 => {
            set_error(&mut c, ERROR_CALL_NOT_IMPLEMENTED);
            c.extend_from_slice(&[0x31, 0xC0]); // xor eax,eax
        }
        Fail::Invalid => {
            set_error(&mut c, ERROR_CALL_NOT_IMPLEMENTED);
            c.extend_from_slice(&[0x48, 0xC7, 0xC0, 0xFF, 0xFF, 0xFF, 0xFF]); // mov rax,-1
        }
        Fail::Hresult => mov_eax(&mut c, E_NOTIMPL),
        Fail::Status => mov_eax(&mut c, STATUS_NOT_IMPLEMENTED),
        Fail::Value(v) => mov_eax(&mut c, v),
        Fail::Error(e, v) => {
            set_error(&mut c, e);
            mov_eax(&mut c, v);
        }
        Fail::Out { arg, store, ret } => {
            // mov rax,[rsp+0x30+8*arg]: the argument's home slot (0x28 below the entry RSP, plus 8).
            c.extend_from_slice(&[0x48, 0x8B, 0x44, 0x24, 0x30 + 8 * arg]);
            c.extend_from_slice(&[0x48, 0x85, 0xC0]); // test rax,rax
            c.extend_from_slice(&[0x74, 0x06]); // jz +6
            c.extend_from_slice(&[0xC7, 0x00]); // mov dword [rax],store
            c.extend_from_slice(&store.to_le_bytes());
            mov_eax(&mut c, ret);
        }
    }
    c.extend_from_slice(&[0x48, 0x83, 0xC4, 0x28, 0xC3]); // add rsp,0x28; ret
    assert!(c.len() <= COMMON_SIZE);
    c
}

/// `stub: <name of the DLL without its extension>!<export>`.
fn shown(file: &str, name: &str) -> String {
    let lower = file.to_ascii_lowercase();
    let stem = lower.rsplit_once('.').map_or(lower.as_str(), |(s, _)| s);
    format!("stub: {stem}!{name}")
}

/// The ordinal of a name `#<n>`.
fn explicit_ordinal(name: &str) -> Option<u32> {
    name.strip_prefix('#').and_then(|s| s.parse::<u32>().ok())
}

/// The image of `dll`.
pub fn build_system_dll(dll: &SystemDll) -> Vec<u8> {
    let kinds: Vec<Fail> = dll.exports.iter().map(|&(_, f)| f).collect::<BTreeSet<_>>().into_iter().collect();
    let first_stub = COMMON_SIZE * kinds.len().max(1);
    let n = dll.exports.len();
    let strings_at = first_stub + STUB_SIZE * n;
    let mut strings: Vec<u8> = Vec::new();
    let mut spans: Vec<(usize, usize)> = Vec::new();
    for &(name, _) in dll.exports {
        let text = shown(dll.file, name);
        spans.push((strings_at + strings.len(), text.len()));
        strings.extend_from_slice(text.as_bytes());
    }
    let text_len = strings_at + strings.len();
    let generated = TEXT_RVA + (text_len as u32).div_ceil(0x1000) * 0x1000 + 0x1000;
    let (iat_print, iat_last_error) = (generated, generated + 8);

    let mut text = alloc::vec![0xCCu8; first_stub];
    let mut at_of: BTreeMap<Fail, usize> = BTreeMap::new();
    for (i, &kind) in kinds.iter().enumerate() {
        let at = COMMON_SIZE * i;
        let code = common(kind, at, iat_print, iat_last_error);
        text[at..at + code.len()].copy_from_slice(&code);
        at_of.insert(kind, at);
    }
    // Ordinals: explicit ones (`#n`) keep theirs; the named exports take the free ones from 1 up.
    let mut taken: BTreeSet<u32> = BTreeSet::new();
    let all_names = dll.exports.iter().map(|&(n, _)| n).chain(dll.forwards.iter().map(|&(n, _)| n));
    for name in all_names {
        if let Some(o) = explicit_ordinal(name) {
            taken.insert(o);
        }
    }
    let mut next = 1u32;
    let mut allocate = |name: &str| -> (Option<String>, u32) {
        if let Some(o) = explicit_ordinal(name) {
            return (None, o);
        }
        while taken.contains(&next) {
            next += 1;
        }
        let o = next;
        taken.insert(o);
        (Some(name.to_string()), o)
    };
    let mut exports = Vec::with_capacity(n + dll.forwards.len());
    for (i, &(name, fail)) in dll.exports.iter().enumerate() {
        let at = first_stub + STUB_SIZE * i;
        let (str_at, len) = spans[i];
        let mut s: Vec<u8> = Vec::with_capacity(STUB_SIZE);
        if let Fail::Out { .. } = fail {
            s.extend_from_slice(&[0x48, 0x89, 0x4C, 0x24, 0x08]); // mov [rsp+8],rcx
            s.extend_from_slice(&[0x48, 0x89, 0x54, 0x24, 0x10]); // mov [rsp+16],rdx
        }
        let lea_at = at + s.len();
        s.extend_from_slice(&[0x48, 0x8D, 0x0D]); // lea rcx,[rip+text]
        s.extend_from_slice(&((str_at as i64 - (lea_at as i64 + 7)) as i32).to_le_bytes());
        s.push(0xBA); // mov edx,length
        s.extend_from_slice(&(len as u32).to_le_bytes());
        let jmp_at = at + s.len();
        s.push(0xE9); // jmp common
        s.extend_from_slice(&((at_of[&fail] as i64 - (jmp_at as i64 + 5)) as i32).to_le_bytes());
        assert!(s.len() <= STUB_SIZE);
        s.resize(STUB_SIZE, 0xCC);
        text.extend_from_slice(&s);
        let (name, ordinal) = allocate(name);
        exports.push(Export { name, ordinal, target: Target::Rva(TEXT_RVA + (first_stub + STUB_SIZE * i) as u32) });
    }
    for &(name, target) in dll.forwards {
        let (name, ordinal) = allocate(name);
        exports.push(Export { name, ordinal, target: Target::Forward(target.to_string()) });
    }
    text.extend_from_slice(&strings);
    let b = PeBuilder {
        sections: alloc::vec![Section::new(".text", text, TEXT), Section::new(".rdata", alloc::vec![0], RDATA)],
        export_name: dll.file.to_string(),
        exports,
        imports: alloc::vec![Import {
            dll: "ntdll.dll".to_string(),
            symbols: alloc::vec![Sym::name("PolyDebugPrint"), Sym::name("RtlSetLastWin32Error")],
            ..Import::default()
        }],
        generated_rva: Some(generated),
        ..PeBuilder::default()
    };
    b.write()
}

#[cfg(test)]
mod tests {
    use super::*;
    use poly_pe::{ExportTarget, Pe};

    const DLL: SystemDll = SystemDll {
        file: "winspool.drv",
        exports: &[
            ("OpenPrinterW", Fail::Win32),
            ("ClosePrinter", Fail::Win32),
            ("#203", Fail::Hresult),
            ("DocumentPropertiesW", Fail::Invalid),
            ("vkEnumerateInstanceVersion", Fail::Out { arg: 0, store: 0x0040_0000, ret: 0 }),
            ("Err", Fail::Error(10045, 0xFFFF_FFFF)),
            ("Val", Fail::Value(8)),
        ],
        forwards: &[("Fwd", "kernel32.K32EnumProcesses")],
    };

    fn le32(b: &[u8], at: usize) -> i64 {
        i64::from(i32::from_le_bytes(b[at..at + 4].try_into().unwrap()))
    }

    #[test]
    fn exports_by_name_ordinal_and_forwarder() {
        let bytes = build_system_dll(&DLL);
        let pe = Pe::parse(&bytes).expect("a PE image");
        let ex = pe.exports().unwrap().unwrap();
        let names: Vec<String> = ex.names().map(|n| String::from_utf8_lossy(n.unwrap().0).into_owned()).collect();
        assert_eq!(
            names,
            ["ClosePrinter", "DocumentPropertiesW", "Err", "Fwd", "OpenPrinterW", "Val", "vkEnumerateInstanceVersion"]
        );
        assert!(
            matches!(ex.by_name(b"Fwd").unwrap(), Some(ExportTarget::Forwarder(f)) if f.raw == b"kernel32.K32EnumProcesses")
        );
        assert!(matches!(ex.by_name(b"OpenPrinterW").unwrap(), Some(ExportTarget::Rva(_))));
        // The ordinal-only export keeps its number; the named ones take the others.
        assert!(matches!(ex.by_ordinal(203).unwrap(), Some(ExportTarget::Rva(_))));
        assert!(matches!(ex.by_ordinal(1).unwrap(), Some(ExportTarget::Rva(_))));
    }

    #[test]
    fn each_kind_gets_its_routine_and_the_text_it_logs() {
        let bytes = build_system_dll(&DLL);
        let pe = Pe::parse(&bytes).unwrap();
        let ex = pe.exports().unwrap().unwrap();
        let Some(ExportTarget::Rva(rva)) = ex.by_name(b"OpenPrinterW").unwrap() else { panic!() };
        let at = rva as usize;
        assert_eq!(&bytes[at..at + 3], &[0x48, 0x8D, 0x0D]);
        let text = (at as i64 + 7 + le32(&bytes, at + 3)) as usize;
        let len = le32(&bytes, at + 8) as usize;
        assert_eq!(&bytes[text..text + len], b"stub: winspool!OpenPrinterW");
        // The out-parameter kind saves its two first arguments before it overwrites them.
        let Some(ExportTarget::Rva(rva)) = ex.by_name(b"vkEnumerateInstanceVersion").unwrap() else { panic!() };
        assert_eq!(&bytes[rva as usize..rva as usize + 5], &[0x48, 0x89, 0x4C, 0x24, 0x08]);
        // Seven exports of six kinds; the routine of `Value(8)` loads 8 after its logging call.
        let kinds: BTreeSet<Fail> = DLL.exports.iter().map(|&(_, f)| f).collect();
        assert_eq!(kinds.len(), 6);
        let Some(ExportTarget::Rva(rva)) = ex.by_name(b"Val").unwrap() else { panic!() };
        let jmp = rva as usize + 12;
        assert_eq!(bytes[jmp], 0xE9);
        let target = (jmp as i64 + 5 + le32(&bytes, jmp + 1)) as usize;
        // sub rsp,0x28 (4), call [iat] (6), then mov eax,8.
        assert_eq!(&bytes[target + 10..target + 15], &[0xB8, 8, 0, 0, 0]);
    }

    #[test]
    fn the_imports_are_the_two_ntdll_services() {
        let bytes = build_system_dll(&DLL);
        let pe = Pe::parse(&bytes).unwrap();
        assert_eq!(pe.imports().unwrap().unwrap().count(), 1);
    }
}

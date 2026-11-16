//! `polystub.dll` (ADR 0155): what an import binds to when Poly lacks the
//! function, or the whole DLL.
//!
//! An app brings far more imports than Poly implements. A static import
//! that does not bind ends the process before its first line runs, with
//! one `NTSTATUS` and no word about which of a thousand imports it was.
//! Poly's loader instead binds such an import (and a delay-load import) to
//! an export of `polystub.dll` when that DLL is on the search path
//! ([`poly_ldr::STUB_DLL`]), and logs it. The stub, when called, logs
//! `stub: dll!name` on the kernel log (`PolyDebugPrint`) and fails the way
//! Microsoft documents for a function the system does not provide:
//!
//! | [`Kind`] | Returns | Last error | For |
//! |---|---|---|---|
//! | `Win32` | 0 (`FALSE`, `NULL`, 0) | `ERROR_CALL_NOT_IMPLEMENTED` (120) | the Win32 API |
//! | `Hresult` | `E_NOTIMPL` (0x80004001) | unchanged | COM and DirectX style APIs |
//! | `Status` | `STATUS_NOT_IMPLEMENTED` (0xC0000002) | unchanged | the native API |
//!
//! (`winerror.h`, `ntstatus.h`; Learn: "System Error Codes".) The kind is
//! per DLL by default ([`default_kind`]) and can be given per function.
//! Missing behaviour thus shows up as a clear log line and a plausible
//! failure instead of a load failure; real behaviour replaces a stub by
//! being implemented in a real DLL, which the loader prefers, so nothing
//! here changes when that happens.
//!
//! The DLL is generated, from the imports an app is missing: one export
//! per `(DLL, function)` named `<dll without .dll, lower case>$<function>`
//! (`<dll>$#<ordinal>` for an import by ordinal), a 32-byte stub each:
//! `lea rcx,[text]`, `mov edx,length`, `jmp common`. Three common routines
//! (one per kind) call `ntdll!PolyDebugPrint(text, length)`, set the
//! result and return; the Win32 one also calls
//! `ntdll!RtlSetLastWin32Error(120)`. Built with `poly_ldr::builder` (the
//! PE format specification), no toolchain involved.
//!
//! **The 32-bit `polystub.dll`** ([`build_x86`], ADR 0186) cannot always return: a 32-bit Win32
//! function is `stdcall` and pops its own arguments, and how many a missing function takes is
//! not known from its import (a name, no count). Returning with the wrong `ret N` would corrupt
//! the caller's stack silently. So a function whose signature is in the table
//! (`x86_signatures.txt`, from the prototypes on Learn: [`x86_signature`]) gets a stub that logs
//! `stub: dll!name`, fails the way the x64 stub does (the table above) and returns with the
//! right `ret 4n`; any other logs and ends the process with `STATUS_NOT_IMPLEMENTED`: the load
//! still succeeds, and the first unknown function the program really calls is named. An
//! ending stub is `push length; push text; jmp common` (16 bytes; the text's address is
//! relocated with the image), and the common routine calls `ntdll!PolyDebugPrint(text, length)`
//! and `ntdll!NtTerminateProcess(-1, STATUS_NOT_IMPLEMENTED)` through the import address table;
//! a returning stub is 32 bytes with the same calls inline.

#![no_std]

extern crate alloc;

use alloc::collections::BTreeMap;
use alloc::format;
use alloc::string::{String, ToString};
use alloc::vec::Vec;
use poly_ldr::builder::{Export, Import, PeBuilder, RDATA, Section, Sym, TEXT, Target};

pub mod sysdlls;
pub mod system;

pub use poly_ldr::{STUB_DLL, stub_key};

/// How a stub fails.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub enum Kind {
    Win32,
    Hresult,
    Status,
}

/// DLLs whose functions return an `HRESULT` (COM and DirectX style) or an error code that is 0 for success.
const HRESULT_DLLS: &[&str] = &[
    // `msi.dll` functions return a `UINT` error code: any non-zero value is a failure, and `E_NOTIMPL` is one.
    "msi",
    "ole32",
    "oleaut32",
    "oleacc",
    "dwrite",
    "dxgi",
    "d3d11",
    "d3d12",
    "d3d9",
    "dcomp",
    "dwmapi",
    "mf",
    "mfplat",
    "mfreadwrite",
    "mmdevapi",
    "propsys",
    "uiautomationcore",
    "urlmon",
    "uxtheme",
    "wintrust",
    "ncrypt",
    "tbs",
    "ndfapi",
    "cryptui",
    "credui",
    "shcore",
    "combase",
    "wincodec",
];

/// How the functions of `dll` (as an import names it, with or without
/// `.dll`) fail unless given otherwise.
pub fn default_kind(dll: &str) -> Kind {
    let d = dll.to_ascii_lowercase();
    let d = d.strip_suffix(".dll").unwrap_or(&d);
    if d == "ntdll" {
        Kind::Status
    } else if HRESULT_DLLS.contains(&d) {
        Kind::Hresult
    } else {
        Kind::Win32
    }
}

/// What to make a stub for.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Want {
    /// The DLL as the importing module names it.
    pub dll: String,
    /// The function name, or `#<ordinal>`.
    pub symbol: String,
    /// `None`: [`default_kind`].
    pub kind: Option<Kind>,
}

/// `ERROR_CALL_NOT_IMPLEMENTED` (`winerror.h`).
pub const ERROR_CALL_NOT_IMPLEMENTED: u32 = 120;
/// `E_NOTIMPL` (`winerror.h`).
pub const E_NOTIMPL: u32 = 0x8000_4001;
/// `STATUS_NOT_IMPLEMENTED` (`ntstatus.h`).
pub const STATUS_NOT_IMPLEMENTED: u32 = 0xC000_0002;

/// `.text` is the first section: RVA 0x1000.
const TEXT_RVA: u32 = 0x1000;
const COMMON_WIN32: usize = 0x00;
const COMMON_HRESULT: usize = 0x20;
const COMMON_STATUS: usize = 0x40;
const FIRST_STUB: usize = 0x60;
const STUB_SIZE: usize = 32;

/// The common routine of `kind`, placed at `.text` offset `at`.
fn common(kind: Kind, at: usize, iat_print: u32, iat_last_error: u32) -> Vec<u8> {
    // Entered by `jmp` from a stub: RCX = text, RDX = length, and the
    // return address of the original call on the stack (RSP = 8 mod 16),
    // so `sub rsp,0x28` aligns for the calls and leaves the shadow space.
    let call = |code: &mut Vec<u8>, iat: u32| {
        let end = TEXT_RVA + (at + code.len() + 6) as u32;
        code.extend_from_slice(&[0xFF, 0x15]); // call [rip+rel32]
        code.extend_from_slice(&iat.wrapping_sub(end).to_le_bytes());
    };
    let mut c: Vec<u8> = alloc::vec![0x48, 0x83, 0xEC, 0x28]; // sub rsp,0x28
    call(&mut c, iat_print); // call [PolyDebugPrint]
    match kind {
        Kind::Win32 => {
            c.push(0xB9); // mov ecx,120
            c.extend_from_slice(&ERROR_CALL_NOT_IMPLEMENTED.to_le_bytes());
            call(&mut c, iat_last_error); // call [RtlSetLastWin32Error]
            c.extend_from_slice(&[0x31, 0xC0]); // xor eax,eax
        }
        Kind::Hresult => {
            c.push(0xB8); // mov eax,E_NOTIMPL
            c.extend_from_slice(&E_NOTIMPL.to_le_bytes());
        }
        Kind::Status => {
            c.push(0xB8); // mov eax,STATUS_NOT_IMPLEMENTED
            c.extend_from_slice(&STATUS_NOT_IMPLEMENTED.to_le_bytes());
        }
    }
    c.extend_from_slice(&[0x48, 0x83, 0xC4, 0x28, 0xC3]); // add rsp,0x28; ret
    c
}

/// The export name of a stub and the text it logs: `dll$name` and
/// `stub: dll!name`.
pub fn stub_text(dll: &str, symbol: &str) -> (String, String) {
    let key = stub_key(dll.as_bytes(), symbol);
    let short = dll.to_ascii_lowercase();
    let short = short.strip_suffix(".dll").unwrap_or(&short).to_string();
    (key, format!("stub: {short}!{symbol}"))
}

/// The image of `polystub.dll` with a stub for each of `wants` (once per
/// `(dll, symbol)`, whatever the case of the DLL name).
pub fn build(wants: &[Want]) -> Vec<u8> {
    // One entry per export name; the first kind given wins.
    let mut by_key: BTreeMap<String, (String, Kind)> = BTreeMap::new();
    for w in wants {
        let kind = w.kind.unwrap_or_else(|| default_kind(&w.dll));
        let (key, shown) = stub_text(&w.dll, &w.symbol);
        by_key.entry(key).or_insert((shown, kind));
    }
    let n = by_key.len();
    let strings_at = FIRST_STUB + STUB_SIZE * n;
    let mut strings: Vec<u8> = Vec::new();
    let mut spans: Vec<(usize, usize)> = Vec::new();
    for (shown, _) in by_key.values() {
        spans.push((strings_at + strings.len(), shown.len()));
        strings.extend_from_slice(shown.as_bytes());
    }
    let text_len = strings_at + strings.len();
    // `.poly` (the generated tables; the import address table comes first)
    // goes on the page after `.text` and the one-page `.rdata`.
    let generated = TEXT_RVA + (text_len as u32).div_ceil(0x1000) * 0x1000 + 0x1000;
    let (iat_print, iat_last_error) = (generated, generated + 8);

    let mut text = alloc::vec![0xCCu8; FIRST_STUB];
    for (kind, at) in [(Kind::Win32, COMMON_WIN32), (Kind::Hresult, COMMON_HRESULT), (Kind::Status, COMMON_STATUS)] {
        let code = common(kind, at, iat_print, iat_last_error);
        assert!(code.len() <= 0x20);
        text[at..at + code.len()].copy_from_slice(&code);
    }
    let mut exports = Vec::with_capacity(n);
    for (i, (key, (_, kind))) in by_key.iter().enumerate() {
        let at = FIRST_STUB + STUB_SIZE * i;
        let target = match kind {
            Kind::Win32 => COMMON_WIN32,
            Kind::Hresult => COMMON_HRESULT,
            Kind::Status => COMMON_STATUS,
        };
        let (str_at, len) = spans[i];
        let mut s: Vec<u8> = Vec::with_capacity(STUB_SIZE);
        s.extend_from_slice(&[0x48, 0x8D, 0x0D]); // lea rcx,[rip+text]
        s.extend_from_slice(&((str_at as i64 - (at as i64 + 7)) as i32).to_le_bytes());
        s.push(0xBA); // mov edx,length
        s.extend_from_slice(&(len as u32).to_le_bytes());
        s.push(0xE9); // jmp common
        s.extend_from_slice(&((target as i64 - (at as i64 + 17)) as i32).to_le_bytes());
        s.resize(STUB_SIZE, 0xCC);
        text.extend_from_slice(&s);
        exports.push(Export {
            name: Some(key.clone()),
            ordinal: i as u32 + 1,
            target: Target::Rva(TEXT_RVA + at as u32),
        });
    }
    text.extend_from_slice(&strings);
    // An `.rdata` page keeps the image a normal-looking two-section DLL;
    // `.poly` (tables) is placed by RVA so the code can address the import
    // address table.
    let b = PeBuilder {
        sections: alloc::vec![Section::new(".text", text, TEXT), Section::new(".rdata", alloc::vec![0], RDATA)],
        export_name: STUB_DLL.to_string(),
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

/// What the 32-bit stub of a function with a known signature does on return.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Signature {
    /// The 4-byte stack slots the arguments take (`stdcall`: the stub pops them).
    pub slots: u8,
    /// A `cdecl` function: the caller pops, the stub is a plain `ret`.
    pub cdecl: bool,
}

/// The table of known 32-bit signatures (`x86_signatures.txt`): `dll!name slots [cdecl]`.
static X86_SIGNATURES: &str = include_str!("x86_signatures.txt");

/// The signature of `symbol` of `dll` (as an import names it), when the table has it.
pub fn x86_signature(dll: &str, symbol: &str) -> Option<Signature> {
    let d = dll.to_ascii_lowercase();
    let d = d.strip_suffix(".dll").unwrap_or(&d);
    X86_SIGNATURES.lines().filter_map(parse_signature).find(|(k, _)| k.0 == d && k.1 == symbol).map(|(_, s)| s)
}

/// One line of the table: `((dll, name), signature)`.
fn parse_signature(line: &str) -> Option<((&str, &str), Signature)> {
    let line = line.trim();
    if line.is_empty() || line.starts_with('#') {
        return None;
    }
    let mut parts = line.split_whitespace();
    let (dll, name) = parts.next()?.split_once('!')?;
    let slots = parts.next()?.parse().ok()?;
    Some(((dll, name), Signature { slots, cdecl: parts.next() == Some("cdecl") }))
}

/// `polystub.dll` for a 32-bit process (see the module documentation): one stub per `(dll, symbol)`
/// of `wants`. A function with a known signature ([`x86_signature`]) is failed the way the x64
/// stub fails it and returned from, popping its arguments; any other logs its name and ends the
/// process.
pub fn build_x86(wants: &[Want]) -> Vec<u8> {
    /// A stub that ends the process: `push length; push text; jmp common`.
    const ENDING: usize = 16;
    /// A returning stub: the log call, the failure and the `ret`.
    const RETURNING: usize = 32;
    const FIRST32: usize = 0x20;
    struct Stub {
        shown: String,
        kind: Kind,
        signature: Option<Signature>,
    }
    let mut by_key: BTreeMap<String, Stub> = BTreeMap::new();
    for w in wants {
        let (key, shown) = stub_text(&w.dll, &w.symbol);
        by_key.entry(key).or_insert_with(|| Stub {
            shown,
            kind: w.kind.unwrap_or_else(|| default_kind(&w.dll)),
            signature: x86_signature(&w.dll, &w.symbol),
        });
    }
    let n = by_key.len();
    let size_of = |s: &Stub| if s.signature.is_some() { RETURNING } else { ENDING };
    let strings_at = FIRST32 + by_key.values().map(size_of).sum::<usize>();
    let mut strings: Vec<u8> = Vec::new();
    let mut spans: Vec<(usize, usize)> = Vec::new();
    for stub in by_key.values() {
        spans.push((strings_at + strings.len(), stub.shown.len()));
        strings.extend_from_slice(stub.shown.as_bytes());
    }
    let text_len = strings_at + strings.len();
    let generated = TEXT_RVA + (text_len as u32).div_ceil(0x1000) * 0x1000 + 0x1000;
    let builder = |text: Vec<u8>, exports: Vec<Export>, relocations: Vec<u32>| PeBuilder {
        sections: alloc::vec![Section::new(".text", text, TEXT), Section::new(".rdata", alloc::vec![0], RDATA)],
        export_name: STUB_DLL.to_string(),
        exports,
        imports: alloc::vec![Import {
            dll: "ntdll.dll".to_string(),
            symbols: alloc::vec![
                Sym::name("PolyDebugPrint"),
                Sym::name("NtTerminateProcess"),
                Sym::name("RtlSetLastWin32Error")
            ],
            ..Import::default()
        }],
        relocations,
        generated_rva: Some(generated),
        ..PeBuilder::x86()
    };
    // Where the import address table's slots are (the same for any text of this length).
    let layout = builder(alloc::vec![0xCC; text_len], Vec::new(), Vec::new()).layout();
    let base = PeBuilder::x86().image_base as u32;
    let (iat_print, iat_end, iat_error) = (base + layout.iat[0][0], base + layout.iat[0][1], base + layout.iat[0][2]);
    let mut relocations = Vec::new();
    // The common routine of the stubs that end the process: they left [text, length] on the
    // stack above the caller's return address; PolyDebugPrint (stdcall) takes them, then the
    // process ends.
    let mut text: Vec<u8> = Vec::with_capacity(text_len);
    text.extend_from_slice(&[0xFF, 0x15]); // call [PolyDebugPrint]
    relocations.push(TEXT_RVA + text.len() as u32);
    text.extend_from_slice(&iat_print.to_le_bytes());
    text.push(0x68); // push STATUS_NOT_IMPLEMENTED
    text.extend_from_slice(&STATUS_NOT_IMPLEMENTED.to_le_bytes());
    text.extend_from_slice(&[0x6A, 0xFF]); // push -1 (NtCurrentProcess)
    text.extend_from_slice(&[0xFF, 0x15]); // call [NtTerminateProcess]
    relocations.push(TEXT_RVA + text.len() as u32);
    text.extend_from_slice(&iat_end.to_le_bytes());
    text.push(0xCC);
    assert!(text.len() <= FIRST32);
    text.resize(FIRST32, 0xCC);
    let mut exports = Vec::with_capacity(n);
    for (i, (key, stub)) in by_key.iter().enumerate() {
        let at = text.len();
        let (str_at, len) = spans[i];
        let text_address = base + TEXT_RVA + str_at as u32;
        text.push(0x68); // push length
        text.extend_from_slice(&(len as u32).to_le_bytes());
        text.push(0x68); // push text
        relocations.push(TEXT_RVA + text.len() as u32);
        text.extend_from_slice(&text_address.to_le_bytes());
        match stub.signature {
            None => {
                text.push(0xE9); // jmp common
                text.extend_from_slice(&(-((at + 15) as i32)).to_le_bytes());
            }
            Some(sig) => {
                text.extend_from_slice(&[0xFF, 0x15]); // call [PolyDebugPrint] (pops both)
                relocations.push(TEXT_RVA + text.len() as u32);
                text.extend_from_slice(&iat_print.to_le_bytes());
                match stub.kind {
                    Kind::Win32 => {
                        text.extend_from_slice(&[0x6A, ERROR_CALL_NOT_IMPLEMENTED as u8]); // push 120
                        text.extend_from_slice(&[0xFF, 0x15]); // call [RtlSetLastWin32Error]
                        relocations.push(TEXT_RVA + text.len() as u32);
                        text.extend_from_slice(&iat_error.to_le_bytes());
                        text.extend_from_slice(&[0x31, 0xC0]); // xor eax,eax
                    }
                    Kind::Hresult | Kind::Status => {
                        text.push(0xB8); // mov eax,E_NOTIMPL / STATUS_NOT_IMPLEMENTED
                        let value = if stub.kind == Kind::Hresult { E_NOTIMPL } else { STATUS_NOT_IMPLEMENTED };
                        text.extend_from_slice(&value.to_le_bytes());
                    }
                }
                if sig.cdecl || sig.slots == 0 {
                    text.push(0xC3); // ret
                } else {
                    text.push(0xC2); // ret 4n
                    text.extend_from_slice(&(u16::from(sig.slots) * 4).to_le_bytes());
                }
            }
        }
        text.resize(at + size_of(stub), 0xCC);
        exports.push(Export {
            name: Some(key.clone()),
            ordinal: i as u32 + 1,
            target: Target::Rva(TEXT_RVA + at as u32),
        });
    }
    text.extend_from_slice(&strings);
    builder(text, exports, relocations).write()
}

#[cfg(test)]
mod tests {
    use super::*;
    use poly_pe::{ExportTarget, Pe};

    fn want(dll: &str, symbol: &str) -> Want {
        Want { dll: dll.into(), symbol: symbol.into(), kind: None }
    }

    fn le32(b: &[u8], at: usize) -> i32 {
        i32::from_le_bytes(b[at..at + 4].try_into().unwrap())
    }

    /// The file offset of an RVA (the builder lays the file out as the image).
    fn img(rva: u32) -> usize {
        rva as usize
    }

    #[test]
    fn the_keys_name_the_dll_without_extension_in_lower_case() {
        assert_eq!(stub_text("SHELL32.dll", "SHGetFolderPathW").0, "shell32$SHGetFolderPathW");
        assert_eq!(
            stub_text("api-ms-win-power-base-l1-1-0.dll", "CallNtPowerInformation").0,
            "api-ms-win-power-base-l1-1-0$CallNtPowerInformation"
        );
        assert_eq!(stub_text("ws2_32", "#111").0, "ws2_32$#111");
        assert_eq!(stub_text("ole32.dll", "CoInitializeEx").1, "stub: ole32!CoInitializeEx");
    }

    #[test]
    fn kinds_follow_the_dll() {
        assert_eq!(default_kind("KERNEL32.dll"), Kind::Win32);
        assert_eq!(default_kind("ole32.dll"), Kind::Hresult);
        assert_eq!(default_kind("dxgi"), Kind::Hresult);
        assert_eq!(default_kind("NTDLL.DLL"), Kind::Status);
    }

    #[test]
    fn the_image_exports_one_stub_per_function_once() {
        let bytes = build(&[
            want("shell32.dll", "SHGetKnownFolderPath"),
            want("SHELL32.DLL", "SHGetKnownFolderPath"),
            want("dxgi.dll", "CreateDXGIFactory1"),
            want("shell32.dll", "#243"),
        ]);
        let pe = Pe::parse(&bytes).expect("a PE image");
        let ex = pe.exports().unwrap().unwrap();
        let names: Vec<String> = ex.names().map(|n| String::from_utf8_lossy(n.unwrap().0).into_owned()).collect();
        assert_eq!(names, ["dxgi$CreateDXGIFactory1", "shell32$#243", "shell32$SHGetKnownFolderPath"]);
        // Each export is a stub in .text that ends in a jump to the common
        // routine of its kind.
        let hresult = |rva: u32| {
            let at = img(rva);
            assert_eq!(&bytes[at..at + 3], &[0x48, 0x8D, 0x0D], "lea rcx");
            assert_eq!(bytes[at + 7], 0xBA, "mov edx");
            assert_eq!(bytes[at + 12], 0xE9, "jmp");
            let target = (at as i64 + 17 + le32(&bytes, at + 13) as i64) as usize;
            // The text it logs.
            let text = (at as i64 + 7 + le32(&bytes, at + 3) as i64) as usize;
            let len = le32(&bytes, at + 8) as usize;
            (target - 0x1000, String::from_utf8(bytes[text..text + len].to_vec()).unwrap())
        };
        let Some(ExportTarget::Rva(rva)) = ex.by_name(b"dxgi$CreateDXGIFactory1").unwrap() else { panic!() };
        assert_eq!(hresult(rva), (COMMON_HRESULT, "stub: dxgi!CreateDXGIFactory1".to_string()));
        let Some(ExportTarget::Rva(rva)) = ex.by_name(b"shell32$#243").unwrap() else { panic!() };
        assert_eq!(hresult(rva), (COMMON_WIN32, "stub: shell32!#243".to_string()));
    }

    #[test]
    fn the_common_routines_call_through_the_import_address_table() {
        let bytes = build(&[want("a.dll", "F")]);
        let pe = Pe::parse(&bytes).unwrap();
        let imports: Vec<_> = pe.imports().unwrap().unwrap().collect();
        assert_eq!(imports.len(), 1);
        // The first call of the Win32 routine reaches the first import slot.
        let call_at = 4; // after sub rsp,0x28
        assert_eq!(&bytes[0x1000 + call_at..0x1000 + call_at + 2], &[0xFF, 0x15]);
        let slot = (0x1000 + call_at as i64 + 6 + le32(&bytes, 0x1000 + call_at + 2) as i64) as u32;
        let first = pe.imports().unwrap().unwrap().next().unwrap().unwrap();
        let _ = first;
        // 8-byte aligned slot in the generated section, after `.text` and `.rdata`.
        assert_eq!(slot % 8, 0);
        assert!(slot >= 0x3000, "slot at {slot:#x}");
        // The routines are exactly as long as the sequences above.
        assert_eq!(common(Kind::Win32, 0, slot, slot + 8).len(), 4 + 6 + 5 + 6 + 2 + 4 + 1);
        assert_eq!(common(Kind::Hresult, 0, slot, slot + 8).len(), 4 + 6 + 5 + 4 + 1);
    }

    #[test]
    fn the_32_bit_stubs_log_and_end_the_process() {
        let wants = [want("WS2_32.DLL", "socket"), want("ws2_32.dll", "socket"), want("msi.dll", "#12")];
        let bytes = build_x86(&wants);
        let pe = poly_pe::Pe::parse(&bytes).unwrap();
        assert_eq!(pe.info().machine(), poly_pe::machine::I386);
        let names: Vec<String> = pe
            .exports()
            .unwrap()
            .unwrap()
            .names()
            .map(|e| String::from_utf8_lossy(e.unwrap().0).into_owned())
            .collect();
        assert_eq!(names, ["msi$#12", "ws2_32$socket"]);
        // The first stub: push length, push the text's address (relocated), jmp to offset 0.
        let at = img(0x1000) + 0x20;
        assert_eq!(bytes[at], 0x68);
        assert_eq!(le32(&bytes, at + 1), "stub: msi!#12".len() as i32);
        assert_eq!(bytes[at + 5], 0x68);
        assert_eq!(bytes[at + 10], 0xE9);
        assert_eq!(le32(&bytes, at + 11), -(0x20 + 15));
        // The common routine calls through the IAT and ends the process.
        assert_eq!(&bytes[img(0x1000)..img(0x1000) + 2], &[0xFF, 0x15]);
        assert_eq!(bytes[img(0x1000) + 6], 0x68);
        assert_eq!(le32(&bytes, img(0x1000) + 7) as u32, STATUS_NOT_IMPLEMENTED);
    }

    #[test]
    fn the_signature_table_is_well_formed() {
        let mut seen = alloc::collections::BTreeSet::new();
        for line in X86_SIGNATURES.lines() {
            let t = line.trim();
            if t.is_empty() || t.starts_with('#') {
                continue;
            }
            let ((dll, name), sig) = parse_signature(line).unwrap_or_else(|| panic!("bad line: {line}"));
            assert_eq!(dll, dll.to_ascii_lowercase(), "{line}: the DLL is lower case, without .dll");
            assert!(!name.is_empty() && sig.slots <= 12, "{line}");
            assert!(seen.insert((dll, name)), "{line}: twice");
        }
        assert!(seen.len() > 250);
        assert_eq!(x86_signature("USER32.DLL", "AppendMenuW"), Some(Signature { slots: 4, cdecl: false }));
        assert_eq!(x86_signature("user32", "wsprintfA"), Some(Signature { slots: 0, cdecl: true }));
        assert_eq!(x86_signature("user32.dll", "NoSuchFunction"), None);
    }

    #[test]
    fn a_32_bit_stub_with_a_signature_fails_and_pops_its_arguments() {
        let bytes = build_x86(&[
            want("user32.dll", "AppendMenuW"),
            want("ole32.dll", "OleGetClipboard"),
            want("user32.dll", "wsprintfA"),
            want("msi.dll", "#12"),
        ]);
        let pe = poly_pe::Pe::parse(&bytes).unwrap();
        let ex = pe.exports().unwrap().unwrap();
        let stub_at = |name: &[u8]| {
            let Some(ExportTarget::Rva(rva)) = ex.by_name(name).unwrap() else { panic!() };
            img(rva)
        };
        // Sorted by key: msi$#12 (ending, 16 bytes), ole32$OleGetClipboard, user32$AppendMenuW,
        // user32$wsprintfA (returning, 32 bytes each).
        let ole = stub_at(b"ole32$OleGetClipboard");
        let append = stub_at(b"user32$AppendMenuW");
        let wsprintf = stub_at(b"user32$wsprintfA");
        assert_eq!(stub_at(b"msi$#12"), img(0x1000) + 0x20);
        assert_eq!(ole, img(0x1000) + 0x30);
        assert_eq!(append, ole + 32);
        assert_eq!(wsprintf, append + 32);
        // push length; push text; call [PolyDebugPrint]; ...
        assert_eq!(bytes[append], 0x68);
        assert_eq!(le32(&bytes, append + 1), "stub: user32!AppendMenuW".len() as i32);
        assert_eq!(&bytes[append + 10..append + 12], &[0xFF, 0x15]);
        // Win32: push 120; call [RtlSetLastWin32Error]; xor eax,eax; ret 16.
        assert_eq!(&bytes[append + 16..append + 18], &[0x6A, 120]);
        assert_eq!(&bytes[append + 18..append + 20], &[0xFF, 0x15]);
        assert_eq!(&bytes[append + 24..append + 26], &[0x31, 0xC0]);
        assert_eq!(&bytes[append + 26..append + 29], &[0xC2, 16, 0]);
        // HRESULT: mov eax,E_NOTIMPL; ret 4.
        assert_eq!(bytes[ole + 16], 0xB8);
        assert_eq!(le32(&bytes, ole + 17) as u32, E_NOTIMPL);
        assert_eq!(&bytes[ole + 21..ole + 24], &[0xC2, 4, 0]);
        // cdecl: a plain ret.
        assert_eq!(bytes[wsprintf + 26], 0xC3);
    }
}

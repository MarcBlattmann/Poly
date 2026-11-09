//! Generate `win32u.dll`'s system-call stubs from `poly-win32k`'s service
//! table (`crates/poly-win32k/src/services.rs`, ADR 0128), so the numbering
//! has one source, as `ntdll`'s build does for the executive services (ADR
//! 0071). Every `(Win32kService::X, <number>, "PolyUserX")` row becomes an
//! exported function `PolyUserX`: `mov r10, rcx; mov eax, <number>;
//! syscall; ret` (the x64 stub convention: number in EAX, RCX moved to R10
//! because `syscall` clobbers it). `cargo xtask win32` checks the bytes of
//! every export against the table.

use std::fmt::Write as _;
use std::path::Path;

fn main() {
    let table = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../../../crates/poly-win32k/src/services.rs");
    println!("cargo:rerun-if-changed={}", table.display());
    let text = std::fs::read_to_string(&table).expect("read poly-win32k's services.rs");
    let mut asm = String::from(".text\n");
    let mut seen: Vec<String> = Vec::new();
    for (num, name) in parse(&text) {
        assert!(name.starts_with("PolyUser") || name.starts_with("PolyGdi"), "{name} is not one of Poly's names");
        assert!(num >> 12 == 1, "{name}: {num:#x} is not in table 1");
        assert!(!seen.contains(&name), "duplicate service name {name}");
        seen.push(name.clone());
        writeln!(asm, ".globl {name}\n{name}:\n    mov r10, rcx\n    mov eax, {num}\n    syscall\n    ret\n").unwrap();
        println!("cargo:rustc-link-arg=/EXPORT:{name}");
    }
    assert!(!seen.is_empty(), "no win32k services found");
    let out = std::path::PathBuf::from(std::env::var("OUT_DIR").unwrap()).join("win32k.s");
    std::fs::write(&out, asm).expect("write stubs");
    println!("cargo:rustc-env=WIN32U_SYSCALLS={}", out.display());
    println!("cargo:rustc-link-arg=/ENTRY:win32u_main");
}

/// Every `(number, "Name")` of a `(Win32kService::X, 0x..., "Name")` row.
fn parse(text: &str) -> Vec<(u32, String)> {
    let mut out = Vec::new();
    let mut rest = text;
    while let Some(start) = rest.find("(Win32kService::") {
        let row = &rest[start + 1..];
        let Some(end) = row.find(')') else { break };
        rest = &row[end..];
        let parts: Vec<&str> = row[..end].splitn(3, ',').map(str::trim).collect();
        if parts.len() != 3 {
            continue;
        }
        let Some(hex) = parts[1].strip_prefix("0x") else { continue };
        let Ok(num) = u32::from_str_radix(&hex.replace('_', ""), 16) else { continue };
        let name = parts[2].trim_matches('"');
        if !name.is_empty() && name.chars().all(|c| c.is_ascii_alphanumeric()) {
            out.push((num, name.to_string()));
        }
    }
    out
}

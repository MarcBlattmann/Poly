//! Generate `ntdll`'s system-call stubs from
//! `personalities/nt/src/services.rs` (ADR 0063: "`cargo xtask` will
//! generate our `ntdll`'s stubs from this table"), so the numbering has one
//! source. Every service whose exported name begins with `Nt` becomes an
//! exported naked function `NtX` and an alias `ZwX`, both
//! `mov r10, rcx; mov eax, <number>; syscall; ret` (the x64 convention of
//! `ntdll`'s stubs: service number in EAX, the kernel reads arguments the
//! same as a `call`, moving RCX into R10 because `syscall` clobbers RCX).

use std::fmt::Write as _;
use std::path::Path;

fn main() {
    let services = Path::new(env!("CARGO_MANIFEST_DIR")).join("../src/services.rs");
    println!("cargo:rerun-if-changed={}", services.display());
    let text = std::fs::read_to_string(&services).expect("read services.rs");
    let mut asm = String::from(".text\n");
    let mut seen: Vec<String> = Vec::new();
    for (num, name) in parse(&text) {
        if !name.starts_with("Nt") {
            continue;
        }
        assert!(!seen.contains(&name), "duplicate service name {name}");
        seen.push(name.clone());
        let zw = format!("Zw{}", &name[2..]);
        // Two exported entry points sharing one body.
        writeln!(
            asm,
            ".globl {name}\n.globl {zw}\n{name}:\n{zw}:\n    mov r10, rcx\n    mov eax, {num}\n    syscall\n    ret\n"
        )
        .unwrap();
        // Export both names from the DLL (apps import Nt*/Zw* from ntdll);
        // /EXPORT also keeps the symbol from being GC'd.
        println!("cargo:rustc-link-arg=/EXPORT:{name}");
        println!("cargo:rustc-link-arg=/EXPORT:{zw}");
    }
    let out = std::path::PathBuf::from(std::env::var("OUT_DIR").unwrap()).join("syscalls.s");
    std::fs::write(&out, asm).expect("write stubs");
    println!("cargo:rustc-env=NTDLL_SYSCALLS={}", out.display());
    assert!(seen.len() > 40, "only {} Nt* services found", seen.len());
}

/// Every `(number, "Name")` of a `(Service::X, <number>, "Name")` row whose
/// number is a plain integer literal (every `Nt*` service is).
fn parse(text: &str) -> Vec<(u32, String)> {
    let mut out = Vec::new();
    for line in text.lines() {
        let line = line.trim();
        if !line.starts_with("(Service::") {
            continue;
        }
        let inside = match (line.find('('), line.rfind(')')) {
            (Some(a), Some(b)) if b > a => &line[a + 1..b],
            _ => continue,
        };
        let parts: Vec<&str> = inside.splitn(3, ',').map(str::trim).collect();
        if parts.len() != 3 {
            continue;
        }
        let Some(num) = parse_int(parts[1]) else { continue };
        let name = parts[2].trim().trim_matches('"');
        if !name.is_empty() && name.chars().all(|c| c.is_ascii_alphanumeric()) {
            out.push((num, name.to_string()));
        }
    }
    out
}

fn parse_int(s: &str) -> Option<u32> {
    let s = s.trim();
    if let Some(hex) = s.strip_prefix("0x").or_else(|| s.strip_prefix("0X")) {
        u32::from_str_radix(&hex.replace('_', ""), 16).ok()
    } else {
        s.replace('_', "").parse().ok()
    }
}

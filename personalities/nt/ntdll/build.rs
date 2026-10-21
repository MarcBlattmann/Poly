//! Generate `ntdll`'s system-call stubs from
//! `personalities/nt/src/services.rs` (ADR 0063: "`cargo xtask` will
//! generate our `ntdll`'s stubs from this table"), so the numbering has one
//! source. Every service whose exported name begins with `Nt` becomes an
//! exported naked function `NtX` and an alias `ZwX`, both
//! `mov r10, rcx; mov eax, <number>; syscall; ret` (the x64 convention of
//! `ntdll`'s stubs: service number in EAX, the kernel reads arguments the
//! same as a `call`, moving RCX into R10 because `syscall` clobbers RCX).
//! Poly's own services (`Poly*`, numbered `(TABLE_POLY << 12) | n`) get a
//! stub too but are not exported: only `ntdll` itself calls them
//! (`KiUserApcDispatcher` ends with `PolyResumeApc`).
//!
//! It also gives the linker what makes the DLL ours (ADR 0071): no entry
//! point (`/NOENTRY`: `ntdll` has no `DllMain`; the kernel starts threads
//! at `LdrInitializeThunk`), the preferred base `0x7FF0_0000_0000` (every
//! process maps it there unless the range is taken, then it is relocated),
//! and the exports defined in assembly.

use std::fmt::Write as _;
use std::path::Path;

fn main() {
    let services = Path::new(env!("CARGO_MANIFEST_DIR")).join("../src/services.rs");
    println!("cargo:rerun-if-changed={}", services.display());
    let text = std::fs::read_to_string(&services).expect("read services.rs");
    let mut asm = String::from(".text\n");
    let mut seen: Vec<String> = Vec::new();
    for (num, name) in parse(&text) {
        if name.starts_with("Poly") {
            assert!(!seen.contains(&name), "duplicate service name {name}");
            seen.push(name.clone());
            writeln!(asm, ".globl {name}\n{name}:\n    mov r10, rcx\n    mov eax, {num}\n    syscall\n    ret\n").unwrap();
            continue;
        }
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
    assert!(seen.iter().any(|n| n == "PolyResumeApc"), "no PolyResumeApc service found");
    for link in ["/NOENTRY", "/BASE:0x7FF000000000", "/EXPORT:LdrInitializeThunk", "/EXPORT:KiUserApcDispatcher"] {
        println!("cargo:rustc-link-arg={link}");
    }
}

/// Every `(number, "Name")` of a `(Service::X, <number>, "Name")` tuple in
/// the text (one per line or several on a line), whose number is an
/// integer literal or Poly's table expression.
fn parse(text: &str) -> Vec<(u32, String)> {
    let mut out = Vec::new();
    let mut rest = text;
    while let Some(start) = rest.find("(Service::") {
        let tuple = &rest[start..];
        // The tuple ends where its parenthesis closes (the number may
        // hold parentheses of its own).
        let mut depth = 0usize;
        let mut end = None;
        for (i, c) in tuple.char_indices() {
            match c {
                '(' => depth += 1,
                ')' => {
                    depth -= 1;
                    if depth == 0 {
                        end = Some(i);
                        break;
                    }
                }
                _ => {}
            }
        }
        let Some(end) = end else { break };
        rest = &tuple[end..];
        let parts: Vec<&str> = tuple[1..end].splitn(3, ',').map(str::trim).collect();
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
    // Poly's own table: `TABLE_POLY << 12` or `(TABLE_POLY << 12) | n`
    // (`TABLE_POLY` is 3 in `services.rs`).
    let poly = |t: &str| {
        let t = t.trim().trim_start_matches('(').trim_end_matches(')').trim();
        (t == "TABLE_POLY << 12").then_some(3u32 << 12)
    };
    if let Some((table, low)) = s.split_once('|') {
        return Some(poly(table)? | parse_int(low)?);
    }
    if let Some(v) = poly(s) {
        return Some(v);
    }
    if let Some(hex) = s.strip_prefix("0x").or_else(|| s.strip_prefix("0X")) {
        u32::from_str_radix(&hex.replace('_', ""), 16).ok()
    } else {
        s.replace('_', "").parse().ok()
    }
}

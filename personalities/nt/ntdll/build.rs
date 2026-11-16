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
    let x86 = std::env::var("CARGO_CFG_TARGET_ARCH").as_deref() == Ok("x86");
    // The i686 stubs pop their arguments (`stdcall`): their counts come from the 32-bit
    // signature table (ADR 0186).
    let sigs = Path::new(env!("CARGO_MANIFEST_DIR")).join("../src/wow64/sigs.rs");
    println!("cargo:rerun-if-changed={}", sigs.display());
    let counts = if x86 { arg_counts(&std::fs::read_to_string(&sigs).expect("read sigs.rs")) } else { Vec::new() };
    let mut asm = String::from(".text\n");
    let mut seen: Vec<String> = Vec::new();
    for (num, variant, name) in parse(&text) {
        let poly = name.starts_with("Poly");
        if !poly && !name.starts_with("Nt") {
            continue;
        }
        assert!(!seen.contains(&name), "duplicate service name {name}");
        seen.push(name.clone());
        let zw = format!("Zw{}", &name[2..]);
        let names: Vec<&str> = if poly { vec![name.as_str()] } else { vec![name.as_str(), zw.as_str()] };
        if x86 {
            let Some(&(_, n)) = counts.iter().find(|(v, _)| *v == variant) else {
                panic!("{name} has no row in wow64/sigs.rs");
            };
            // `int 0xF4` with EAX = the number, EDX = the arguments (ADR 0186); `stdcall`
            // names (`_NtClose@4`), so Rust's declarations of them link.
            for label in &names {
                writeln!(asm, ".globl \"_{label}@{}\"\n\"_{label}@{}\":", 4 * n, 4 * n).unwrap();
            }
            writeln!(asm, "    mov eax, {num}\n    lea edx, [esp + 4]\n    int 0xF4\n    ret {}\n", 4 * n).unwrap();
        } else {
            for label in &names {
                writeln!(asm, ".globl {label}\n{label}:").unwrap();
            }
            writeln!(asm, "    mov r10, rcx\n    mov eax, {num}\n    syscall\n    ret\n").unwrap();
        }
        if !poly {
            // Export both names from the DLL (apps import Nt*/Zw* from ntdll);
            // /EXPORT also keeps the symbol from being GC'd.
            println!("cargo:rustc-link-arg=/EXPORT:{name}");
            println!("cargo:rustc-link-arg=/EXPORT:{zw}");
        }
    }
    let out = std::path::PathBuf::from(std::env::var("OUT_DIR").unwrap()).join("syscalls.s");
    std::fs::write(&out, asm).expect("write stubs");
    println!("cargo:rustc-env=NTDLL_SYSCALLS={}", out.display());
    assert!(seen.len() > 40, "only {} Nt* services found", seen.len());
    assert!(seen.iter().any(|n| n == "PolyResumeApc"), "no PolyResumeApc service found");
    // The diagnostics service is exported for Poly's own stub DLLs (ADR 0155).
    println!("cargo:rustc-link-arg=/EXPORT:PolyDebugPrint");
    // The memory functions the Windows applications import from ntdll (ADR 0155, step 10): the
    // `compiler_builtins` ones, as `ucrtbase.dll` exports them.
    if !x86 {
        for name in ["memcpy", "memmove", "memset", "memcmp"] {
            println!("cargo:rustc-link-arg=/EXPORT:{name}");
        }
    }
    // RtlCaptureContext is assembly too (ADR 0087).
    let mut exports = vec!["/EXPORT:LdrInitializeThunk", "/EXPORT:KiUserApcDispatcher", "/EXPORT:RtlCaptureContext"];
    if x86 {
        // The i686 build's frame-based exception exports are assembly too (ADR 0186).
        exports.extend(["/EXPORT:RtlRaiseException", "/EXPORT:RtlUnwind"]);
    }
    // The i686 ntdll lives below 2 GiB (ADR 0186); the x64 one high up (ADR 0071).
    let base = if x86 { "/BASE:0x77000000" } else { "/BASE:0x7FF000000000" };
    for link in ["/NOENTRY", base].into_iter().chain(exports) {
        println!("cargo:rustc-link-arg={link}");
    }
}

/// The number of arguments of each row `(Service::Variant, &[A, B, ...])` of the 32-bit
/// signature table.
fn arg_counts(text: &str) -> Vec<(String, usize)> {
    let mut out = Vec::new();
    let mut rest = text;
    while let Some(start) = rest.find("(Service::") {
        let row = &rest[start + "(Service::".len()..];
        let Some(comma) = row.find(',') else { break };
        let variant = row[..comma].trim().to_string();
        let after = row[comma + 1..].trim_start();
        rest = &row[comma..];
        let Some(list) = after.strip_prefix("&[") else { continue };
        let Some(end) = list.find(']') else { break };
        let items = list[..end].split(',').filter(|i| !i.trim().is_empty()).count();
        out.push((variant, items));
    }
    out
}

/// Every `(number, "Name")` of a `(Service::X, <number>, "Name")` tuple in
/// the text (one per line or several on a line), whose number is an
/// integer literal or Poly's table expression.
fn parse(text: &str) -> Vec<(u32, String, String)> {
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
        let variant = parts[0].trim_start_matches("(Service::").trim_start_matches("Service::").to_string();
        if !name.is_empty() && name.chars().all(|c| c.is_ascii_alphanumeric()) {
            out.push((num, variant, name.to_string()));
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

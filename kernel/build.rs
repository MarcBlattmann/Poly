use std::path::{Path, PathBuf};
use std::process::Command;

fn main() {
    let dir = std::env::var("CARGO_MANIFEST_DIR").unwrap();
    println!("cargo:rustc-link-arg-bins=-T{dir}/linker.ld");
    println!("cargo:rerun-if-changed=linker.ld");
    build_vdso(Path::new(&dir));
}

/// The vDSO's source, linker script and version script, and the crate it
/// shares with the kernel (ADR 0044).
const VDSO_DIR: &str = "src/arch/x86_64/vdso";
const VDSO_DATA_CRATE: &str = "../crates/poly-vdso/src/lib.rs";

/// Build the vDSO (`src/arch/x86_64/vdso/vdso.rs`) into `$OUT_DIR/vdso.so`,
/// which the kernel embeds. The pinned `rustc` compiles `poly-vdso` and the
/// vDSO for the bare-metal target, position independent, and its
/// `rust-lld` links them as a shared object with our linker and version
/// scripts. The result must have one loadable segment and no relocations:
/// nothing relocates a vDSO.
fn build_vdso(kernel: &Path) {
    let out = PathBuf::from(std::env::var("OUT_DIR").unwrap());
    let rustc = std::env::var("RUSTC").unwrap_or_else(|_| "rustc".into());
    let src = kernel.join(VDSO_DIR);
    for file in ["vdso.rs", "vdso.lds", "vdso.ver"] {
        println!("cargo:rerun-if-changed={VDSO_DIR}/{file}");
    }
    println!("cargo:rerun-if-changed={VDSO_DATA_CRATE}");
    let common = [
        "--edition=2024",
        "--target=x86_64-unknown-none",
        "-Cpanic=abort",
        "-Copt-level=2",
        "-Crelocation-model=pie",
        "-Cdebuginfo=0",
        "-Cforce-unwind-tables=yes",
    ];
    run(Command::new(&rustc)
        .args(common)
        .args(["--crate-type=rlib", "--crate-name=poly_vdso", "--out-dir"])
        .arg(&out)
        .arg(kernel.join(VDSO_DATA_CRATE)));
    let image = out.join("vdso.so");
    run(Command::new(&rustc)
        .args(common)
        .args(["--crate-type=bin", "--crate-name=vdso", "-Clto=fat", "-Ccodegen-units=1", "--extern"])
        .arg(format!("poly_vdso={}", out.join("libpoly_vdso.rlib").display()))
        .args(["-Clink-arg=--no-pie", "-Clink-arg=-shared", "-Clink-arg=-znorelro"])
        .arg(format!("-Clink-arg=-T{}", src.join("vdso.lds").display()))
        .arg(format!("-Clink-arg=--version-script={}", src.join("vdso.ver").display()))
        .args(["-Clink-arg=--hash-style=both", "-Clink-arg=--build-id=none", "-Clink-arg=-soname=linux-vdso.so.1"])
        .arg("-o")
        .arg(&image)
        .arg(src.join("vdso.rs")));
    let bytes = std::fs::read(&image).expect("read the vDSO image");
    if let Err(why) = check_vdso(&bytes) {
        panic!("{}: {why}", image.display());
    }
}

fn run(cmd: &mut Command) {
    let status = cmd.status().unwrap_or_else(|e| panic!("{cmd:?}: {e}"));
    assert!(status.success(), "{cmd:?} failed ({status})");
}

fn u16_at(b: &[u8], at: usize) -> u64 {
    u16::from_le_bytes(b[at..at + 2].try_into().unwrap()).into()
}

fn u32_at(b: &[u8], at: usize) -> u64 {
    u32::from_le_bytes(b[at..at + 4].try_into().unwrap()).into()
}

fn u64_at(b: &[u8], at: usize) -> u64 {
    u64::from_le_bytes(b[at..at + 8].try_into().unwrap())
}

/// What the kernel relies on (ELF-64 gABI layouts): a shared object with
/// exactly one `PT_LOAD`, at address 0, file offset 0, readable and
/// executable but not writable, file size equal to memory size, and a
/// `PT_DYNAMIC` without relocation tables (`DT_RELA`, `DT_REL`,
/// `DT_JMPREL`, `DT_TEXTREL`).
fn check_vdso(b: &[u8]) -> Result<(), String> {
    const ET_DYN: u64 = 3;
    const PT_LOAD: u64 = 1;
    const PT_DYNAMIC: u64 = 2;
    const PF_W: u64 = 2;
    const RELOCATION_TAGS: [u64; 4] = [7, 17, 23, 22];
    if b.len() < 64 || &b[..4] != b"\x7fELF" || u16_at(b, 16) != ET_DYN {
        return Err("not an ELF shared object".into());
    }
    let (phoff, phentsize, phnum) = (u64_at(b, 32) as usize, u16_at(b, 54) as usize, u16_at(b, 56) as usize);
    let mut loads = 0;
    for i in 0..phnum {
        let ph = phoff + i * phentsize;
        let (ty, flags, offset) = (u32_at(b, ph), u32_at(b, ph + 4), u64_at(b, ph + 8) as usize);
        let (vaddr, filesz, memsz) = (u64_at(b, ph + 16), u64_at(b, ph + 32), u64_at(b, ph + 40));
        match ty {
            PT_LOAD => {
                loads += 1;
                if vaddr != 0 || offset != 0 || flags & PF_W != 0 || filesz != memsz {
                    return Err(format!("unexpected PT_LOAD: vaddr {vaddr:#x}, offset {offset:#x}, flags {flags:#x}"));
                }
            }
            PT_DYNAMIC => {
                for entry in b[offset..offset + filesz as usize].chunks_exact(16) {
                    let tag = u64_at(entry, 0);
                    if RELOCATION_TAGS.contains(&tag) {
                        return Err(format!("the vDSO needs relocations (dynamic tag {tag})"));
                    }
                }
            }
            _ => {}
        }
    }
    if loads != 1 {
        return Err(format!("{loads} loadable segments, expected one"));
    }
    Ok(())
}

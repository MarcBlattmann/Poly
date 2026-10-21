use std::path::{Path, PathBuf};
use std::process::Command;

fn main() {
    let dir = std::env::var("CARGO_MANIFEST_DIR").unwrap();
    println!("cargo:rustc-link-arg-bins=-T{dir}/linker.ld");
    println!("cargo:rerun-if-changed=linker.ld");
    build_vdso(Path::new(&dir));
    embed_ntdll();
    embed_win32();
}

/// Where `cargo xtask` names the directory with our `kernelbase.dll`,
/// `kernel32.dll` and the test program `k32test.exe` (ADR 0076), cached by
/// content hash like `ntdll` (`tools/xtask/src/win32.rs`).
const WIN32_ENV: &str = "POLY_WIN32_DIR";

/// Copy the Win32 modules to `$OUT_DIR`, which the kernel embeds
/// (`nt::KERNELBASE`, `nt::KERNEL32`, `nt::K32TEST`). Without `xtask` they
/// are empty files: that kernel starts Windows programs that need only
/// `ntdll`.
fn embed_win32() {
    let out = PathBuf::from(std::env::var("OUT_DIR").unwrap());
    println!("cargo:rerun-if-env-changed={WIN32_ENV}");
    let dir = std::env::var_os(WIN32_ENV).map(PathBuf::from);
    if dir.is_none() {
        println!("cargo:warning={WIN32_ENV} is not set: this kernel has no kernel32.dll (build with `cargo xtask`)");
    }
    for name in ["kernelbase.dll", "kernel32.dll", "k32test.exe"] {
        match &dir {
            Some(d) => {
                let path = d.join(name);
                println!("cargo:rerun-if-changed={}", path.display());
                std::fs::copy(&path, out.join(name)).unwrap_or_else(|e| panic!("{}: {e}", path.display()));
            }
            None => std::fs::write(out.join(name), []).expect("write an empty module"),
        }
    }
}

/// Where `cargo xtask` names our `ntdll.dll` (ADR 0071): built once per
/// source version and cached by content hash (`tools/xtask/src/ntdll.rs`),
/// so the kernel build itself never runs the `-Zbuild-std` compile.
const NTDLL_ENV: &str = "POLY_NTDLL_DLL";

/// Copy our `ntdll.dll` to `$OUT_DIR/ntdll.dll`, which the kernel embeds
/// (`nt::ntdll`). A build without `xtask` (a bare `cargo build` in
/// `kernel/`) gets an empty file and a warning: that kernel boots, but
/// cannot start Windows programs.
fn embed_ntdll() {
    let out = PathBuf::from(std::env::var("OUT_DIR").unwrap()).join("ntdll.dll");
    println!("cargo:rerun-if-env-changed={NTDLL_ENV}");
    match std::env::var_os(NTDLL_ENV) {
        Some(path) => {
            let path = PathBuf::from(path);
            println!("cargo:rerun-if-changed={}", path.display());
            std::fs::copy(&path, &out).unwrap_or_else(|e| panic!("{}: {e}", path.display()));
        }
        None => {
            println!("cargo:warning={NTDLL_ENV} is not set: this kernel has no ntdll.dll (build with `cargo xtask`)");
            std::fs::write(&out, []).expect("write an empty ntdll.dll");
        }
    }
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

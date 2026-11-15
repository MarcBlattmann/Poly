//! A 32-bit Windows program end to end (ADR 0186): `hello32.exe`, built for
//! `i686-pc-windows-msvc` from `tests/windows/wow64/hello32.rs`, imports only from
//! `kernel32.dll`. The kernel maps it and the i686 `ntdll.dll` below 4 GiB, starts its thread in
//! compatibility mode, and every system call it makes (through the i686 `kernel32` and
//! `kernelbase` in `C:\Windows\SysWOW64`) enters with `int 0xF4` and is converted to the
//! 64-bit service. The program writes `hello` and the name of each check it passed to its
//! standard output (the console) and ends with 0 when all passed, else with the number of the
//! first that failed (`hello32.rs`).

/// Where the test puts the program.
pub const APP_DIR: &str = "Program Files (x86)\\PolyHello32";
pub const APP_EXE: &str = "C:\\Program Files (x86)\\PolyHello32\\hello32.exe";

mod tests {
    use super::*;
    use crate::kernel_test;
    use crate::process::Process;
    use crate::thread;
    use crate::user::memory::Layout;
    use crate::user::{self, Memory};
    use alloc::string::String;
    use alloc::sync::Arc;
    use alloc::vec::Vec;
    use poly_binfmt::{Arch, Bits, Format};
    use poly_personality_nt::io::VfsRoot;
    use poly_personality_nt::layout::create;
    use poly_vfs::{At, Cred};

    const FORMAT: Format = Format::Pe { bits: Bits::B32, arch: Arch::X86 };

    kernel_test! {
        /// A 32-bit console program prints and exits 0 (ADR 0186): see [`super`].
        fn nt_wow64_32_bit_program_runs_in_compatibility_mode() -> TestResult {
            if crate::fs::vfs().root_context().is_err() {
                return Err("no VFS for the WOW64 test");
            }
            let hello = super::super::HELLO32;
            if super::super::NTDLL.is_empty() || super::super::NTDLL32.is_empty() || hello.is_empty() {
                return Err("this kernel was built without the 32-bit modules (cargo xtask wow64)");
            }
            // The first NT use makes C: and installs the 32-bit DLLs in SysWOW64.
            super::super::with(|_| ());
            let vfs = crate::fs::vfs();
            let ctx = vfs.root_context().map_err(|_| "no VFS")?;
            let root = vfs.lookup(&ctx, &Cred::root(), At::Cwd, b"/nt/c", true).map_err(|_| "no C:")?;
            for name in ["ntdll.dll", "kernelbase.dll", "kernel32.dll"] {
                let path = alloc::format!("Windows/SysWOW64/{name}");
                if vfs.lookup(&ctx, &Cred::root(), At::Node(&root), path.as_bytes(), true).is_err() {
                    return Err("SysWOW64 lacks a 32-bit module");
                }
            }
            let vr = VfsRoot::new(vfs, ctx, Cred::root());
            create::install_file(&vr, &root, APP_DIR, "hello32.exe", hello).map_err(|_| "cannot write hello32.exe")?;

            let memory = Arc::new(Memory::new(Layout::default()).map_err(|_| "no memory map")?);
            let pid = crate::process::alloc_pid();
            let process = Arc::new(Process::new_nt(pid, FORMAT, false));
            let launch = super::super::Launch { image_path: String::from(APP_EXE), args: Vec::new() };
            let start = super::super::start_with(&process, &memory, hello, pid, &launch)
                .map_err(|_| "the kernel rejected hello32.exe")?;
            if !super::super::is_wow64(pid) {
                return Err("hello32.exe did not become a 32-bit process");
            }
            if start.entry.rip >= 1 << 32 || start.entry.rsp >= 1 << 32 || start.teb >= 1 << 32 {
                return Err("the 32-bit process's entry, stack or TEB is above 4 GiB");
            }
            let tid = super::super::spawn_thread(&process, &memory, start.entry, start.teb, user::DEFAULT_HOOK, None)
                .map_err(|_| "could not start the 32-bit thread")?;
            let code = thread::join(tid).map_err(|_| "join failed")?;
            if code != 0 {
                crate::kprintln!("nt: hello32.exe ended with {code:#x}, not 0 (1: no output; 2 + n: check n failed)");
                return Err("hello32.exe did not pass every check");
            }
            if !thread::wait_until(10, || Arc::strong_count(&memory) == 1) {
                return Err("the 32-bit process kept its memory after it exited");
            }
            if super::super::is_nt_process(pid) {
                return Err("the 32-bit process's state was not cleaned up");
            }
            Ok(())
        }
    }
}

//! Our `kernel32` end to end (ADR 0078): `k32test.exe`, a program built
//! from Rust like our DLLs (`personalities/nt/win32/k32test`), imports
//! only from `kernel32.dll`. Its imports reach `kernelbase.dll` through
//! `kernel32`'s API-set forwarders, which our `ntdll`'s loader resolves;
//! `kernelbase` runs on our `ntdll`. The program uses files and
//! directories, the environment, the last error after failing calls, the
//! heaps and virtual memory, a thread with events, a critical section and
//! an SRW lock, mutexes and semaphores, time, the version, messages and
//! modules, and ends through `ExitProcess` with [`PASS_CODE`]; a failed
//! check ends it with `0x80 + step` after printing the step to its standard
//! output (the console).

/// `k32test.exe`'s exit status when every check passed.
pub const PASS_CODE: u8 = 0x4B;
/// Where the test puts the program (its process parameters name it).
pub const APP_DIR: &str = "Program Files\\PolyK32Test";
pub const APP_EXE: &str = "C:\\Program Files\\PolyK32Test\\k32test.exe";

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

    const FORMAT: Format = Format::Pe { bits: Bits::B64, arch: Arch::X86_64 };

    kernel_test! {
        /// A program that imports from our `kernel32.dll` runs end to end
        /// (ADR 0078): see [`super`].
        fn nt_kernel32_program_runs_through_forwarders_to_kernelbase() -> TestResult {
            if crate::fs::vfs().root_context().is_err() {
                return Err("no VFS for the kernel32 test");
            }
            let nt = super::super::NTDLL;
            if nt.is_empty() || super::super::KERNEL32.is_empty() || super::super::K32TEST.is_empty() {
                return Err("this kernel was built without ntdll.dll or the Win32 modules");
            }
            // The first NT use makes C: and installs our DLLs in System32.
            super::super::with(|_| ());
            let vfs = crate::fs::vfs();
            let ctx = vfs.root_context().map_err(|_| "no VFS")?;
            let root = vfs.lookup(&ctx, &Cred::root(), At::Cwd, b"/nt/c", true).map_err(|_| "no C:")?;
            for name in ["kernelbase.dll", "kernel32.dll"] {
                let path = alloc::format!("Windows/System32/{name}");
                if vfs.lookup(&ctx, &Cred::root(), At::Node(&root), path.as_bytes(), true).is_err() {
                    return Err("System32 lacks kernelbase.dll or kernel32.dll");
                }
            }
            let vr = VfsRoot { vfs, ctx, cred: Cred::root() };
            let exe = super::super::K32TEST;
            create::install_file(&vr, &root, APP_DIR, "k32test.exe", exe).map_err(|_| "cannot write k32test.exe")?;

            let memory = Arc::new(Memory::new(Layout::default()).map_err(|_| "no memory map")?);
            let pid = crate::process::alloc_pid();
            let process = Arc::new(Process::new_nt(pid, FORMAT, false));
            let launch = super::super::Launch { image_path: String::from(APP_EXE), args: Vec::from([String::from("--poly")]) };
            let start = super::super::start_with(&process, &memory, exe, pid, &launch)
                .map_err(|_| "the kernel rejected k32test.exe")?;
            let tid =
                user::spawn_nt(Arc::clone(&process), Arc::clone(&memory), start.entry, start.teb, user::DEFAULT_HOOK)
                    .map_err(|_| "could not start the NT thread")?;
            let code = thread::join(tid).map_err(|_| "join failed")?;
            if code != PASS_CODE as usize {
                crate::kprintln!(
                    "nt: k32test.exe ended with {code:#x}, not {PASS_CODE:#x} (0x80 + step: a failed step)"
                );
                return Err("k32test.exe did not pass every check");
            }
            if !thread::wait_until(10, || Arc::strong_count(&memory) == 1) {
                return Err("the NT process kept its memory after it exited");
            }
            if super::super::is_nt_process(pid) {
                return Err("the NT process state was not cleaned up");
            }
            Ok(())
        }
    }
}

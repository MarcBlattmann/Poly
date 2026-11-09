//! Chrome's headless window calls end to end (ADR 0155, batch 4):
//! `s1test.exe`, a program built from Rust like our DLLs
//! (`personalities/nt/win32/s1test`), imports from `ole32.dll`, the
//! `api-ms-win-core-winrt-*` contracts (which resolve to `ole32.dll`),
//! `user32.dll`, `gdi32.dll` and `kernel32.dll`. It makes the calls a
//! headless Chrome makes: COM apartments and activation, `HSTRING`s,
//! message-only windows, window properties, registered messages,
//! `SystemParametersInfoW`, the keyboard layout list, DPI, icons, fonts and
//! device caps, and ends through `ExitProcess` with [`PASS_CODE`]; a failed
//! check ends it with `0x80 + step` after printing the step.

/// `s1test.exe`'s exit status when every check passed.
pub const PASS_CODE: u8 = 0x42;
/// Where the test puts the program (its process parameters name it).
pub const APP_DIR: &str = "Program Files\\PolyS1Test";
pub const APP_EXE: &str = "C:\\Program Files\\PolyS1Test\\s1test.exe";

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
        /// A program that makes the calls of a headless Chrome runs end to end
        /// (ADR 0155): see [`super`].
        fn nt_chrome_headless_surface_program_runs() -> TestResult {
            if crate::fs::vfs().root_context().is_err() {
                return Err("no VFS for the s1test test");
            }
            let nt = super::super::NTDLL;
            if nt.is_empty() || super::super::OLE32.is_empty() || super::super::S1TEST.is_empty() {
                return Err("this kernel was built without ntdll.dll or the Win32 modules");
            }
            // The first NT use makes C: and installs our DLLs in System32.
            super::super::with(|_| ());
            let vfs = crate::fs::vfs();
            let ctx = vfs.root_context().map_err(|_| "no VFS")?;
            let root = vfs.lookup(&ctx, &Cred::root(), At::Cwd, b"/nt/c", true).map_err(|_| "no C:")?;
            if vfs.lookup(&ctx, &Cred::root(), At::Node(&root), b"Windows/System32/ole32.dll", true).is_err() {
                return Err("System32 lacks ole32.dll");
            }
            let vr = VfsRoot::new(vfs, ctx, Cred::root());
            let exe = super::super::S1TEST;
            create::install_file(&vr, &root, APP_DIR, "s1test.exe", exe).map_err(|_| "cannot write s1test.exe")?;

            let memory = Arc::new(Memory::new(Layout::default()).map_err(|_| "no memory map")?);
            let pid = crate::process::alloc_pid();
            let process = Arc::new(Process::new_nt(pid, FORMAT, false));
            let launch = super::super::Launch { image_path: String::from(APP_EXE), args: Vec::new() };
            let start = super::super::start_with(&process, &memory, exe, pid, &launch)
                .map_err(|_| "the kernel rejected s1test.exe")?;
            let tid =
                user::spawn_nt(Arc::clone(&process), Arc::clone(&memory), start.entry, start.teb, user::DEFAULT_HOOK)
                    .map_err(|_| "could not start the NT thread")?;
            let code = thread::join(tid).map_err(|_| "join failed")?;
            if code != PASS_CODE as usize {
                crate::kprintln!(
                    "nt: s1test.exe ended with {code:#x}, not {PASS_CODE:#x} (0x80 + step: a failed step)"
                );
                return Err("s1test.exe did not pass every check");
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

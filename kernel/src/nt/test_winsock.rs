//! Our `ws2_32` end to end (ADR 0095): `wstest.exe`, a program built from
//! Rust like our DLLs (`personalities/nt/win32/wstest`), imports from
//! `ws2_32.dll` and `kernel32.dll` and talks to itself over the loopback
//! interface: TCP over 127.0.0.1 and `::1` (also with an echo server in a
//! second thread blocked in `accept` and `recv`), UDP, `select`,
//! `WSAPoll`, options and timeouts, an overlapped `WSARecv` that the
//! kernel's `nt-afd` worker completes, names of `localhost`. It ends
//! through `ExitProcess` with [`PASS_CODE`]; a failed check ends it with
//! `0x80 + step` after printing the step to its standard output.

/// `wstest.exe`'s exit status when every check passed.
pub const PASS_CODE: u8 = 0x57;
/// Where the test puts the program (its process parameters name it).
pub const APP_DIR: &str = "Program Files\\PolyWsTest";
pub const APP_EXE: &str = "C:\\Program Files\\PolyWsTest\\wstest.exe";

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
        /// A program that imports from our `ws2_32.dll` talks to itself
        /// over loopback (ADR 0095): see [`super`].
        fn nt_winsock_program_talks_over_loopback() -> TestResult {
            if crate::fs::vfs().root_context().is_err() {
                return Err("no VFS for the Winsock test");
            }
            if super::super::NTDLL.is_empty() || super::super::WS2_32.is_empty() || super::super::WSTEST.is_empty() {
                return Err("this kernel was built without ntdll.dll or the Win32 modules");
            }
            // The first NT use makes C: and installs our DLLs in System32.
            super::super::with(|_| ());
            let vfs = crate::fs::vfs();
            let ctx = vfs.root_context().map_err(|_| "no VFS")?;
            let root = vfs.lookup(&ctx, &Cred::root(), At::Cwd, b"/nt/c", true).map_err(|_| "no C:")?;
            if vfs.lookup(&ctx, &Cred::root(), At::Node(&root), b"Windows/System32/ws2_32.dll", true).is_err() {
                return Err("System32 lacks ws2_32.dll");
            }
            let vr = VfsRoot::new(vfs, ctx, Cred::root());
            let exe = super::super::WSTEST;
            create::install_file(&vr, &root, APP_DIR, "wstest.exe", exe).map_err(|_| "cannot write wstest.exe")?;

            let memory = Arc::new(Memory::new(Layout::default()).map_err(|_| "no memory map")?);
            let pid = crate::process::alloc_pid();
            let process = Arc::new(Process::new_nt(pid, FORMAT, false));
            let launch = super::super::Launch { image_path: String::from(APP_EXE), args: Vec::new() };
            let start = super::super::start_with(&process, &memory, exe, pid, &launch)
                .map_err(|_| "the kernel rejected wstest.exe")?;
            let tid =
                user::spawn_nt(Arc::clone(&process), Arc::clone(&memory), start.entry, start.teb, user::DEFAULT_HOOK)
                    .map_err(|_| "could not start the NT thread")?;
            let code = thread::join(tid).map_err(|_| "join failed")?;
            if code != PASS_CODE as usize {
                crate::kprintln!("nt: wstest.exe ended with {code:#x}, not {PASS_CODE:#x} (0x80 + step: a failed step)");
                return Err("wstest.exe did not pass every check");
            }
            if !thread::wait_until(10, || Arc::strong_count(&memory) == 1) {
                return Err("the NT process kept its memory after it exited");
            }
            if super::super::is_nt_process(pid) {
                return Err("the NT process state was not cleaned up");
            }
            if !thread::wait_until(10, || super::super::afd::pending() == 0) {
                return Err("overlapped socket requests were left with the worker");
            }
            Ok(())
        }
    }
}

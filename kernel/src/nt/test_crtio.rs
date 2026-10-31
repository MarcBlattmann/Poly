//! Our C runtime's files, processes and console end to end (ADR 0093):
//! `crtiotest.exe` (`personalities/nt/win32/crtiotest`) checks
//! `_findfirst`, `_dup` / `_pipe`, `_spawn*` / `_exec*` / `system` /
//! `_popen` (starting itself as the child and as the command interpreter),
//! wildcard `argv` and console I/O, and ends through `exit` with
//! [`PASS_CODE`]. This test gives it a file as standard output and error
//! and checks that a child it started wrote there through the descriptor
//! it inherited.

/// `crtiotest.exe`'s exit status when every check passed.
pub const PASS_CODE: u8 = 0x44;
/// Where the test puts the program.
pub const APP_DIR: &str = "Program Files\\PolyCrtIoTest";
pub const APP_EXE: &str = "C:\\Program Files\\PolyCrtIoTest\\crtiotest.exe";

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
    use poly_vfs::{At, Cred, OpenFlags};

    const FORMAT: Format = Format::Pe { bits: Bits::B64, arch: Arch::X86_64 };

    fn contains(hay: &[u8], needle: &[u8]) -> bool {
        hay.windows(needle.len()).any(|w| w == needle)
    }

    kernel_test! {
        /// The C runtime's files, processes and console (ADR 0093): see
        /// [`super`].
        fn nt_c_runtime_files_processes_and_console() -> TestResult {
            if crate::fs::vfs().root_context().is_err() {
                return Err("no VFS for the C runtime test");
            }
            if super::super::NTDLL.is_empty() || super::super::UCRTBASE.is_empty() || super::super::CRTIOTEST.is_empty() {
                return Err("this kernel was built without ntdll.dll or the Win32 modules");
            }
            // The first NT use makes C: and installs our DLLs in System32.
            super::super::with(|_| ());
            let vfs = crate::fs::vfs();
            let ctx = vfs.root_context().map_err(|_| "no VFS")?;
            let root = vfs.lookup(&ctx, &Cred::root(), At::Cwd, b"/nt/c", true).map_err(|_| "no C:")?;
            let vr = VfsRoot::new(vfs, ctx, Cred::root());
            let exe = super::super::CRTIOTEST;
            create::install_file(&vr, &root, APP_DIR, "crtiotest.exe", exe).map_err(|_| "cannot write crtiotest.exe")?;
            let flags = OpenFlags::CREAT | OpenFlags::TRUNC | OpenFlags::RDWR;
            let out_path = b"/nt/c/Program Files/PolyCrtIoTest/stdout.txt";
            let out = vr.vfs.open(&vr.ctx, &vr.cred, At::Cwd, out_path, flags, 0o644).map_err(|_| "no stdout file")?;

            let memory = Arc::new(Memory::new(Layout::default()).map_err(|_| "no memory map")?);
            let pid = crate::process::alloc_pid();
            let process = Arc::new(Process::new_nt(pid, FORMAT, false));
            let launch = super::super::Launch { image_path: String::from(APP_EXE), args: Vec::from([String::from("--poly")]) };
            use super::super::console::StdSource;
            let sources = [StdSource::None, StdSource::File(Arc::clone(&out)), StdSource::File(Arc::clone(&out))];
            let start = super::super::start_with_std(&process, &memory, exe, pid, &launch, &sources)
                .map_err(|_| "the kernel rejected crtiotest.exe")?;
            let tid =
                user::spawn_nt(Arc::clone(&process), Arc::clone(&memory), start.entry, start.teb, user::DEFAULT_HOOK)
                    .map_err(|_| "could not start the NT thread")?;
            let code = thread::join(tid).map_err(|_| "join failed")?;
            let mut printed = alloc::vec![0u8; 4096];
            let n = out.read_at(0, &mut printed).map_err(|_| "cannot read the stdout file")?;
            printed.truncate(n);
            if code != PASS_CODE as usize {
                crate::kprintln!(
                    "nt: crtiotest.exe ended with {code:#x}, not {PASS_CODE:#x} (0x80 + step: a failed step); it printed {:?}",
                    String::from_utf8_lossy(&printed)
                );
                return Err("crtiotest.exe did not pass every check");
            }
            let expected: [&[u8]; 2] = [b"crtiotest: child says hello\r\n", b"crtiotest: done\r\n"];
            for e in expected {
                if !contains(&printed, e) {
                    crate::kprintln!(
                        "nt: crtiotest's standard output lacks {:?}: {:?}",
                        String::from_utf8_lossy(e),
                        String::from_utf8_lossy(&printed)
                    );
                    return Err("crtiotest.exe's or its child's output did not reach the standard output");
                }
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

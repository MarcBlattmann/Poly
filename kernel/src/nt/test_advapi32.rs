//! Our `advapi32` end to end (ADR 0083): `a32test.exe`, a program built
//! from Rust like our DLLs (`personalities/nt/win32/a32test`), imports from
//! `advapi32.dll` and `kernel32.dll`. `advapi32`'s registry, token, SID,
//! ACL and security-descriptor functions reach `kernelbase.dll` through its
//! API-set forwarders; its SID strings and `GetUserNameW` are its own. The
//! program reads and writes the registry (`HKLM`, `HKCU`, the
//! `HKEY_CLASSES_ROOT` view), opens and queries the single user's token,
//! and builds SIDs, ACLs and security descriptors, ending through
//! `ExitProcess` with [`PASS_CODE`]; a failed check ends it with
//! `0x80 + step` after printing the step to its standard output.

/// `a32test.exe`'s exit status when every check passed.
pub const PASS_CODE: u8 = 0x41;
/// Where the test puts the program (its process parameters name it).
pub const APP_DIR: &str = "Program Files\\PolyA32Test";
pub const APP_EXE: &str = "C:\\Program Files\\PolyA32Test\\a32test.exe";

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
        /// A program that imports from our `advapi32.dll` runs end to end
        /// (ADR 0083): see [`super`].
        fn nt_advapi32_program_runs_through_forwarders_to_kernelbase() -> TestResult {
            if crate::fs::vfs().root_context().is_err() {
                return Err("no VFS for the advapi32 test");
            }
            let nt = super::super::NTDLL;
            if nt.is_empty() || super::super::ADVAPI32.is_empty() || super::super::A32TEST.is_empty() {
                return Err("this kernel was built without ntdll.dll or the Win32 modules");
            }
            // The first NT use makes C: and installs our DLLs in System32.
            super::super::with(|_| ());
            let vfs = crate::fs::vfs();
            let ctx = vfs.root_context().map_err(|_| "no VFS")?;
            let root = vfs.lookup(&ctx, &Cred::root(), At::Cwd, b"/nt/c", true).map_err(|_| "no C:")?;
            if vfs.lookup(&ctx, &Cred::root(), At::Node(&root), b"Windows/System32/advapi32.dll", true).is_err() {
                return Err("System32 lacks advapi32.dll");
            }
            let vr = VfsRoot::new(vfs, ctx, Cred::root());
            let exe = super::super::A32TEST;
            create::install_file(&vr, &root, APP_DIR, "a32test.exe", exe).map_err(|_| "cannot write a32test.exe")?;

            let memory = Arc::new(Memory::new(Layout::default()).map_err(|_| "no memory map")?);
            let pid = crate::process::alloc_pid();
            let process = Arc::new(Process::new_nt(pid, FORMAT, false));
            let launch = super::super::Launch { image_path: String::from(APP_EXE), args: Vec::new() };
            let start = super::super::start_with(&process, &memory, exe, pid, &launch)
                .map_err(|_| "the kernel rejected a32test.exe")?;
            let tid =
                user::spawn_nt(Arc::clone(&process), Arc::clone(&memory), start.entry, start.teb, user::DEFAULT_HOOK)
                    .map_err(|_| "could not start the NT thread")?;
            let code = thread::join(tid).map_err(|_| "join failed")?;
            if code != PASS_CODE as usize {
                crate::kprintln!(
                    "nt: a32test.exe ended with {code:#x}, not {PASS_CODE:#x} (0x80 + step: a failed step)"
                );
                return Err("a32test.exe did not pass every check");
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

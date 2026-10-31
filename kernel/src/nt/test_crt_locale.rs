//! Locales and code pages of our C runtime end to end (ADR 0094):
//! `crtloctest.exe` (`personalities/nt/win32/crtloctest`) imports the UCRT
//! through `api-ms-win-crt-*` names and loads `msvcrt.dll`; it checks
//! `setlocale`, `localeconv`, classes and case, `printf`/`strtod` with a
//! decimal comma, multibyte conversions through `kernelbase`'s code page
//! tables (1252, 1251, 932, UTF-8), the `_mbs*` functions on code page 932,
//! the `_l` forms, collation and a thread's own locale, then ends with
//! [`PASS_CODE`]. This test gives it a file as standard output and checks
//! the status and the line it printed.

/// `crtloctest.exe`'s exit status when every check passed.
pub const PASS_CODE: u8 = 0x4C;
/// Where the test puts the program.
pub const APP_DIR: &str = "Program Files\\PolyCrtLocTest";
pub const APP_EXE: &str = "C:\\Program Files\\PolyCrtLocTest\\crtloctest.exe";

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

    kernel_test! {
        /// Locales other than "C" and multibyte code pages in our C runtime
        /// (ADR 0094): see [`super`].
        fn nt_c_runtime_locales_and_code_pages() -> TestResult {
            if crate::fs::vfs().root_context().is_err() {
                return Err("no VFS for the C runtime locale test");
            }
            if super::super::NTDLL.is_empty() || super::super::UCRTBASE.is_empty() || super::super::CRTLOCTEST.is_empty() {
                return Err("this kernel was built without ntdll.dll or the Win32 modules");
            }
            // The first NT use makes C: and installs our DLLs in System32.
            super::super::with(|_| ());
            let vfs = crate::fs::vfs();
            let ctx = vfs.root_context().map_err(|_| "no VFS")?;
            let root = vfs.lookup(&ctx, &Cred::root(), At::Cwd, b"/nt/c", true).map_err(|_| "no C:")?;
            let vr = VfsRoot::new(vfs, ctx, Cred::root());
            let exe = super::super::CRTLOCTEST;
            create::install_file(&vr, &root, APP_DIR, "crtloctest.exe", exe).map_err(|_| "cannot write crtloctest.exe")?;
            let flags = OpenFlags::CREAT | OpenFlags::TRUNC | OpenFlags::RDWR;
            let out_path = b"/nt/c/Program Files/PolyCrtLocTest/stdout.txt";
            let out = vr.vfs.open(&vr.ctx, &vr.cred, At::Cwd, out_path, flags, 0o644).map_err(|_| "no stdout file")?;

            let memory = Arc::new(Memory::new(Layout::default()).map_err(|_| "no memory map")?);
            let pid = crate::process::alloc_pid();
            let process = Arc::new(Process::new_nt(pid, FORMAT, false));
            let launch = super::super::Launch { image_path: String::from(APP_EXE), args: Vec::new() };
            use super::super::console::StdSource;
            let sources = [StdSource::None, StdSource::File(Arc::clone(&out)), StdSource::File(Arc::clone(&out))];
            let start = super::super::start_with_std(&process, &memory, exe, pid, &launch, &sources)
                .map_err(|_| "the kernel rejected crtloctest.exe")?;
            let tid =
                user::spawn_nt(Arc::clone(&process), Arc::clone(&memory), start.entry, start.teb, user::DEFAULT_HOOK)
                    .map_err(|_| "could not start the NT thread")?;
            let code = thread::join(tid).map_err(|_| "join failed")?;
            let mut printed = alloc::vec![0u8; 512];
            let n = out.read_at(0, &mut printed).map_err(|_| "cannot read the stdout file")?;
            printed.truncate(n);
            if code != PASS_CODE as usize {
                crate::kprintln!(
                    "nt: crtloctest.exe ended with {code:#x}, not {PASS_CODE:#x} (0x80 + step: a failed step); it printed {:?}",
                    String::from_utf8_lossy(&printed)
                );
                return Err("crtloctest.exe did not pass every check");
            }
            if printed != b"crtloctest: ok\r\n" {
                crate::kprintln!("nt: crtloctest printed {:?}", String::from_utf8_lossy(&printed));
                return Err("crtloctest.exe's output did not reach its standard output");
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

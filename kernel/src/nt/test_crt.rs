//! Our C runtime end to end (ADR 0084): `crttest.exe`, a program built from
//! Rust like our DLLs (`personalities/nt/win32/crttest`), imports the C
//! runtime as an MSVC-built program does, through `api-ms-win-crt-*`
//! names, which our `ntdll`'s loader resolves to `ucrtbase.dll` in
//! `C:\Windows\System32`; it loads `msvcrt.dll` too. It checks the
//! runtime itself (see the program) and ends through `exit` with
//! [`PASS_CODE`]. This test gives it a file as standard output and error,
//! then checks what the program printed there (in text mode, so every line
//! ends in `\r\n`) and the file it left open for `exit` to write.

/// `crttest.exe`'s exit status when every check passed.
pub const PASS_CODE: u8 = 0x43;
/// Where the test puts the program.
pub const APP_DIR: &str = "Program Files\\PolyCrtTest";
pub const APP_EXE: &str = "C:\\Program Files\\PolyCrtTest\\crttest.exe";

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
        /// A program on our C runtime runs end to end (ADR 0084): see
        /// [`super`].
        fn nt_c_runtime_program_runs_on_ucrtbase() -> TestResult {
            if crate::fs::vfs().root_context().is_err() {
                return Err("no VFS for the C runtime test");
            }
            if super::super::NTDLL.is_empty() || super::super::UCRTBASE.is_empty() || super::super::CRTTEST.is_empty() {
                return Err("this kernel was built without ntdll.dll or the Win32 modules");
            }
            // The first NT use makes C: and installs our DLLs in System32.
            super::super::with(|_| ());
            let vfs = crate::fs::vfs();
            let ctx = vfs.root_context().map_err(|_| "no VFS")?;
            let root = vfs.lookup(&ctx, &Cred::root(), At::Cwd, b"/nt/c", true).map_err(|_| "no C:")?;
            for name in ["ucrtbase.dll", "msvcrt.dll"] {
                let path = alloc::format!("Windows/System32/{name}");
                if vfs.lookup(&ctx, &Cred::root(), At::Node(&root), path.as_bytes(), true).is_err() {
                    return Err("System32 lacks ucrtbase.dll or msvcrt.dll");
                }
            }
            let vr = VfsRoot { vfs, ctx, cred: Cred::root() };
            let exe = super::super::CRTTEST;
            create::install_file(&vr, &root, APP_DIR, "crttest.exe", exe).map_err(|_| "cannot write crttest.exe")?;
            let flags = OpenFlags::CREAT | OpenFlags::TRUNC | OpenFlags::RDWR;
            let out_path = b"/nt/c/Program Files/PolyCrtTest/stdout.txt";
            let out = vr.vfs.open(&vr.ctx, &vr.cred, At::Cwd, out_path, flags, 0o644).map_err(|_| "no stdout file")?;

            let memory = Arc::new(Memory::new(Layout::default()).map_err(|_| "no memory map")?);
            let pid = crate::process::alloc_pid();
            let process = Arc::new(Process::new_nt(pid, FORMAT, false));
            let args = ["--poly", "two words", "q\"uote"].map(String::from);
            let launch = super::super::Launch { image_path: String::from(APP_EXE), args: Vec::from(args) };
            use super::super::console::StdSource;
            let sources = [StdSource::None, StdSource::File(Arc::clone(&out)), StdSource::File(Arc::clone(&out))];
            let start = super::super::start_with_std(&process, &memory, exe, pid, &launch, &sources)
                .map_err(|_| "the kernel rejected crttest.exe")?;
            let tid =
                user::spawn_nt(Arc::clone(&process), Arc::clone(&memory), start.entry, start.teb, user::DEFAULT_HOOK)
                    .map_err(|_| "could not start the NT thread")?;
            let code = thread::join(tid).map_err(|_| "join failed")?;
            let mut printed = alloc::vec![0u8; 4096];
            let n = out.read_at(0, &mut printed).map_err(|_| "cannot read the stdout file")?;
            printed.truncate(n);
            if code != PASS_CODE as usize {
                crate::kprintln!(
                    "nt: crttest.exe ended with {code:#x}, not {PASS_CODE:#x} (0x80 + step: a failed step); it printed {:?}",
                    String::from_utf8_lossy(&printed)
                );
                return Err("crttest.exe did not pass every check");
            }
            let expected: [&[u8]; 3] =
                [b"crttest: printing through the UCRT\r\n", b"crttest: puts\r\n", b"crttest: msvcrt fprintf 1\r\n"];
            for e in expected {
                if !contains(&printed, e) {
                    crate::kprintln!("nt: crttest's standard output lacks {:?}: {:?}", String::from_utf8_lossy(e), String::from_utf8_lossy(&printed));
                    return Err("crttest.exe's output did not reach its standard output");
                }
            }
            // The file `exit` had to write.
            let left = vr
                .vfs
                .open(&vr.ctx, &vr.cred, At::Cwd, b"/nt/c/Program Files/PolyCrtTest/crtexit.txt", OpenFlags::RDONLY, 0)
                .map_err(|_| "crtexit.txt was not made")?;
            let mut text = [0u8; 64];
            let n = left.read_at(0, &mut text).map_err(|_| "cannot read crtexit.txt")?;
            if &text[..n] != b"flushed at exit\r\n" {
                crate::kprintln!("nt: crtexit.txt holds {:?}", String::from_utf8_lossy(&text[..n]));
                return Err("exit did not write the open stream");
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

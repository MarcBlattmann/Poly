//! Our `winhttp` and `wininet` end to end (ADR 0095): `httptest.exe`
//! (`personalities/nt/win32/httptest`) runs a tiny HTTP server in one
//! thread on 127.0.0.1 (our `ws2_32`) and makes GET requests to it from
//! another through `winhttp.dll` and `wininet.dll`: status and header
//! queries, bodies framed by `Content-Length`, chunks and the end of the
//! connection, `localhost`, and the errors for `https` and other schemes.
//! It ends through `ExitProcess` with [`PASS_CODE`]; a failed check ends
//! it with `0x80 + step` after printing the step to its standard output.

/// `httptest.exe`'s exit status when every check passed.
pub const PASS_CODE: u8 = 0x48;
/// Where the test puts the program (its process parameters name it).
pub const APP_DIR: &str = "Program Files\\PolyHttpTest";
pub const APP_EXE: &str = "C:\\Program Files\\PolyHttpTest\\httptest.exe";

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
        /// GET requests through our `winhttp.dll` and `wininet.dll`
        /// against a tiny server (ADR 0095): see [`super`].
        fn nt_http_get_through_winhttp_and_wininet() -> TestResult {
            if crate::fs::vfs().root_context().is_err() {
                return Err("no VFS for the HTTP test");
            }
            let modules = [super::super::NTDLL, super::super::WS2_32, super::super::WINHTTP, super::super::WININET];
            if modules.iter().any(|m| m.is_empty()) || super::super::HTTPTEST.is_empty() {
                return Err("this kernel was built without ntdll.dll or the Win32 modules");
            }
            // The first NT use makes C: and installs our DLLs in System32.
            super::super::with(|_| ());
            let vfs = crate::fs::vfs();
            let ctx = vfs.root_context().map_err(|_| "no VFS")?;
            let root = vfs.lookup(&ctx, &Cred::root(), At::Cwd, b"/nt/c", true).map_err(|_| "no C:")?;
            for dll in [&b"Windows/System32/winhttp.dll"[..], b"Windows/System32/wininet.dll"] {
                if vfs.lookup(&ctx, &Cred::root(), At::Node(&root), dll, true).is_err() {
                    return Err("System32 lacks winhttp.dll or wininet.dll");
                }
            }
            let vr = VfsRoot { vfs, ctx, cred: Cred::root() };
            let exe = super::super::HTTPTEST;
            create::install_file(&vr, &root, APP_DIR, "httptest.exe", exe).map_err(|_| "cannot write httptest.exe")?;

            let memory = Arc::new(Memory::new(Layout::default()).map_err(|_| "no memory map")?);
            let pid = crate::process::alloc_pid();
            let process = Arc::new(Process::new_nt(pid, FORMAT, false));
            let launch = super::super::Launch { image_path: String::from(APP_EXE), args: Vec::new() };
            let start = super::super::start_with(&process, &memory, exe, pid, &launch)
                .map_err(|_| "the kernel rejected httptest.exe")?;
            let tid =
                user::spawn_nt(Arc::clone(&process), Arc::clone(&memory), start.entry, start.teb, user::DEFAULT_HOOK)
                    .map_err(|_| "could not start the NT thread")?;
            let code = thread::join(tid).map_err(|_| "join failed")?;
            if code != PASS_CODE as usize {
                crate::kprintln!("nt: httptest.exe ended with {code:#x}, not {PASS_CODE:#x} (0x80 + step: a failed step)");
                return Err("httptest.exe did not pass every check");
            }
            if !thread::wait_until(10, || Arc::strong_count(&memory) == 1) {
                return Err("the NT process kept its memory after it exited");
            }
            Ok(())
        }
    }
}

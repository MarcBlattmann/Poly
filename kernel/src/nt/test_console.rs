//! The console host end to end (ADR 0080): `contest.exe`, built from Rust
//! like our DLLs (`personalities/nt/win32/contest`), runs on a
//! pseudo-terminal — standard input and error on the terminal, standard
//! output redirected to a file — while the test plays the user at the
//! terminal's master side: it types two lines and a key when the program
//! says `READY-INPUT`, and Ctrl+C at `READY-CTRLC`. The program checks the
//! console functions itself (see its documentation) and exits with
//! [`PASS_CODE`]; the test checks what reached the terminal and the file,
//! and that the terminal got its settings back.

/// `contest.exe`'s exit status when every check passed.
pub const PASS_CODE: u8 = 0x4C;
pub const APP_DIR: &str = "Program Files\\PolyConsoleTest";
pub const APP_EXE: &str = "C:\\Program Files\\PolyConsoleTest\\contest.exe";

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
        /// A console program on a terminal (ADR 0080): see [`super`].
        fn nt_console_program_on_a_terminal() -> TestResult {
            if crate::fs::vfs().root_context().is_err() {
                return Err("no VFS for the console test");
            }
            if super::super::NTDLL.is_empty() || super::super::KERNEL32.is_empty() || super::super::CONTEST.is_empty() {
                return Err("this kernel was built without ntdll.dll or the Win32 modules");
            }
            super::super::with(|_| ());
            let vfs = crate::fs::vfs();
            let ctx = vfs.root_context().map_err(|_| "no VFS")?;
            let root = vfs.lookup(&ctx, &Cred::root(), At::Cwd, b"/nt/c", true).map_err(|_| "no C:")?;
            let vr = VfsRoot::new(vfs, ctx, Cred::root());
            let exe = super::super::CONTEST;
            create::install_file(&vr, &root, APP_DIR, "contest.exe", exe).map_err(|_| "cannot write contest.exe")?;
            // Standard output: a file.
            let flags = OpenFlags::CREAT | OpenFlags::TRUNC | OpenFlags::RDWR;
            let out_path = b"/nt/c/Program Files/PolyConsoleTest/stdout.txt";
            let out = vr.vfs.open(&vr.ctx, &vr.cred, At::Cwd, out_path, flags, 0o644).map_err(|_| "no stdout file")?;

            // A pseudo-terminal: the test holds the master.
            let ptys = crate::tty::ptys().ok_or("no ptys")?;
            let before = ptys.indices();
            let files = crate::fs::Files::new();
            let master = files.open(b"/dev/ptmx", OpenFlags::RDWR | OpenFlags::NONBLOCK, 0).map_err(|_| "no ptmx")?;
            let index = ptys.indices().into_iter().find(|i| !before.contains(i)).ok_or("no new pty")?;
            let slave = Arc::clone(ptys.get(index).ok_or("no pty pair")?.slave());
            slave.set_winsize(poly_tty::termios::Winsize::new(25, 80));
            let sane = slave.termios();

            let memory = Arc::new(Memory::new(Layout::default()).map_err(|_| "no memory map")?);
            let pid = crate::process::alloc_pid();
            let process = Arc::new(Process::new_nt(pid, FORMAT, false));
            let launch = super::super::Launch { image_path: String::from(APP_EXE), args: Vec::new() };
            use super::super::console::StdSource;
            let sources =
                [StdSource::Terminal(Arc::clone(&slave)), StdSource::File(Arc::clone(&out)), StdSource::Terminal(Arc::clone(&slave))];
            let start = super::super::start_with_std(&process, &memory, exe, pid, &launch, &sources)
                .map_err(|_| "the kernel rejected contest.exe")?;
            if super::super::console::console_of(&slave).is_none() {
                return Err("the program got no console on its terminal");
            }
            let tid =
                user::spawn_nt(Arc::clone(&process), Arc::clone(&memory), start.entry, start.teb, user::DEFAULT_HOOK)
                    .map_err(|_| "could not start the NT thread")?;

            // Play the user until the program ends (at most a minute).
            let mut shown = Vec::new();
            let (mut typed, mut interrupted) = (false, false);
            let mut buf = [0u8; 512];
            for _ in 0..6000 {
                while let Ok(n) = files.read(master, &mut buf) {
                    if n == 0 {
                        break;
                    }
                    shown.extend_from_slice(&buf[..n]);
                }
                if !typed && contains(&shown, b"READY-INPUT") {
                    slave.receive(b"hello\rabc\x7fd\rx");
                    typed = true;
                }
                if !interrupted && contains(&shown, b"READY-CTRLC") {
                    slave.receive(b"\x03");
                    interrupted = true;
                }
                if matches!(thread::state(tid), Some(thread::State::Exited) | None) {
                    break;
                }
                thread::sleep(core::time::Duration::from_millis(10));
            }
            let code = thread::join(tid).map_err(|_| "join failed")?;
            while let Ok(n) = files.read(master, &mut buf) {
                if n == 0 {
                    break;
                }
                shown.extend_from_slice(&buf[..n]);
            }
            if code != PASS_CODE as usize {
                crate::kprintln!(
                    "nt: contest.exe ended with {code:#x}, not {PASS_CODE:#x} (0x80 + step: a failed step); its terminal showed {:?}",
                    String::from_utf8_lossy(&shown)
                );
                return Err("contest.exe did not pass every check");
            }
            let expected: [&[u8]; 9] = [
                "Hello, W\u{f6}rld \u{2713}".as_bytes(),
                "UTF-8: \u{fc}n\u{ef}code".as_bytes(),
                b"\x1b[0;92;40mgreen",
                b"\x1b[11;6H\x1b[0m@",
                b"\x1b[0;31;40mVT",
                b"\x1b]0;Poly console test\x07",
                b"hello",
                b"via CONOUT$",
                b"\x1b[0m\x1b[r",
            ];
            for e in expected {
                if !contains(&shown, e) {
                    crate::kprintln!("nt: the terminal lacks {:?}: {:?}", String::from_utf8_lossy(e), String::from_utf8_lossy(&shown));
                    return Err("contest.exe's console output did not reach its terminal");
                }
            }
            let mut written = [0u8; 64];
            let n = out.read_at(0, &mut written).map_err(|_| "cannot read stdout file")?;
            if &written[..n] != b"redirected\r\n" {
                return Err("the redirected standard output did not reach its file");
            }
            if slave.termios() != sane {
                return Err("the terminal did not get its settings back");
            }
            if super::super::console::console_of(&slave).is_some() {
                return Err("the console outlived its last process");
            }
            let _ = files.close(master);
            if !thread::wait_until(10, || Arc::strong_count(&memory) == 1) {
                return Err("the NT process kept its memory after it exited");
            }
            Ok(())
        }
    }
}

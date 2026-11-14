//! Executables bigger than the kernel heap can allocate at once (ADR 0155).
//!
//! The heap hands out at most 4 MiB in one piece, and process creation used
//! to read the whole executable into one buffer: Notepad++ (8 MB), WinMerge,
//! Node.js (100 MB), VS Code (238 MB) and six more panicked the kernel the
//! moment they were started. These tests generate their executables (a few
//! instructions that call `RtlExitUserProcess`, padded with a data section
//! of 64 MiB that only exists as the file's length) and check that
//!
//! - such an executable starts, runs and exits with its status, its pages
//!   coming from the file when they are touched, and
//! - memory that runs out while an image is set up is `STATUS_NO_MEMORY`
//!   for the caller, not a kernel panic: a relocation table too big for one
//!   heap allocation (the heap's `try_reserve` fails there).
//!
//! No Microsoft toolchain or DLLs: the images are written with
//! `poly_ldr::builder`.

use alloc::sync::Arc;
use alloc::vec;
use alloc::vec::Vec;
use poly_ldr::builder::{DATA, Import, PeBuilder, RDATA, Section, Sym, TEXT, append_section, set_directory};

/// The exit status the generated program ends with.
const EXIT_CODE: u8 = 0x2A;

/// The program: `RtlExitUserProcess(EXIT_CODE)` from `ntdll`, preferring
/// `image_base`.
fn exit_exe(image_base: u64) -> Vec<u8> {
    // sub rsp, 0x28; mov ecx, EXIT_CODE; call [rip + iat]; int3
    let mut code = vec![0x48, 0x83, 0xEC, 0x28, 0xB9, EXIT_CODE, 0, 0, 0, 0xFF, 0x15, 0, 0, 0, 0, 0xCC];
    let mut b = PeBuilder {
        image_base,
        entry: 0x1000,
        sections: vec![Section::new(".text", code.clone(), TEXT)],
        imports: vec![Import {
            dll: "ntdll.dll".into(),
            symbols: vec![Sym::name("RtlExitUserProcess")],
            ..Import::default()
        }],
        stack_reserve: 0x1_0000,
        stack_commit: 0x4000,
        ..PeBuilder::default()
    };
    // The call ends at 0x1000 + 15.
    let slot = i64::from(b.layout().iat[0][0]) - 0x100F;
    code[11..15].copy_from_slice(&(slot as i32).to_le_bytes());
    b.sections[0].data = code;
    b.write()
}

mod tests {
    use super::*;
    use crate::kernel_test;
    use crate::process::ExitStatus;
    use crate::program::{self, Prepared};
    use poly_ntobj::status::STATUS_NO_MEMORY;
    use poly_personality_nt::image::{ImageSection, VfsFile};
    use poly_vfs::{At, Cred, OpenFlags};

    const BIG: &str = "/tmp/poly-bigimage.exe";
    const BAD_RELOC: &str = "/tmp/poly-bigreloc.exe";

    /// Make file `path` hold `head` and be `len` bytes long (a sparse tail
    /// of zeros: nothing near `len` bytes is held in memory by the test).
    fn write_file(path: &str, head: &[u8], len: u64) -> Result<(), &'static str> {
        let vfs = crate::fs::vfs();
        let ctx = vfs.root_context().map_err(|_| "no VFS")?;
        let _ = vfs.unlink(&ctx, &Cred::root(), At::Cwd, path.as_bytes());
        let f = vfs
            .open(&ctx, &Cred::root(), At::Cwd, path.as_bytes(), OpenFlags::CREAT | OpenFlags::RDWR, 0o755)
            .map_err(|_| "cannot create the test executable")?;
        let mut at = 0;
        while at < head.len() {
            at += f.write_at(at as u64, &head[at..]).map_err(|_| "cannot write the test executable")?;
        }
        f.truncate(len).map_err(|_| "cannot extend the test executable")?;
        Ok(())
    }

    fn remove(path: &str) {
        if let Ok(ctx) = crate::fs::vfs().root_context() {
            let _ = crate::fs::vfs().unlink(&ctx, &Cred::root(), At::Cwd, path.as_bytes());
        }
    }

    kernel_test! {
        /// A 64 MiB executable starts and exits (ADR 0155): before, the
        /// kernel read the whole file into one heap buffer and panicked
        /// with "memory allocation of 67108xxx bytes failed" (the 4 MiB
        /// heap limit); now only the headers are read, and its pages come
        /// from the file as they are touched.
        fn nt_starts_an_executable_bigger_than_the_heap_can_allocate() -> TestResult {
            if crate::fs::vfs().root_context().is_err() {
                return Err("no VFS for the big image test");
            }
            if super::super::NTDLL.is_empty() {
                return Err("this kernel was built without ntdll.dll");
            }
            let pad: u32 = 64 << 20;
            let mut file = exit_exe(0x1_4000_0000);
            let at = append_section(&mut file, ".pad", b"a big data section", pad, DATA).ok_or("no room for .pad")?;
            if at.file_len < 64 << 20 {
                return Err("the generated executable is not big");
            }
            write_file(BIG, &file, at.file_len)?;
            let started = (|| {
                let prepared = program::prepare(BIG.as_bytes(), &[BIG.as_bytes()], &[], false)
                    .map_err(|_| "a 64 MiB executable was refused")?;
                let pid = prepared.process().pid;
                let process = prepared.start().map_err(|_| "a 64 MiB executable did not start")?;
                Ok::<_, &'static str>((pid, process.wait()))
            })();
            remove(BIG);
            let (pid, status) = started?;
            let _ = super::super::take_exit_status(pid);
            if status != ExitStatus::Exited(EXIT_CODE) {
                crate::kprintln!("nt: the big executable ended with {status}, not {EXIT_CODE:#x}");
                return Err("the big executable did not exit with its status");
            }
            Ok(())
        }
    }

    kernel_test! {
        /// Memory that runs out while an image is set up is an error for the
        /// caller, never a kernel panic (ADR 0155). The image prefers a base
        /// that is not a multiple of 64 KiB, which no view can have, so it
        /// is relocated, and its
        /// relocation table (300,000 blocks: 16 bytes each in the kernel, so the vector doubles past 4 MiB)
        /// needs a vector bigger than the heap's 4 MiB limit: `try_reserve`
        /// fails, and process creation answers `STATUS_NO_MEMORY`.
        fn nt_reports_no_memory_when_an_image_outgrows_the_heap() -> TestResult {
            if crate::fs::vfs().root_context().is_err() {
                return Err("no VFS for the big image test");
            }
            if super::super::NTDLL.is_empty() {
                return Err("this kernel was built without ntdll.dll");
            }
            // 300,000 blocks of no entries: the header (page 0, size 8) each.
            let mut table = Vec::with_capacity(300_000 * 8);
            for _ in 0..300_000u32 {
                table.extend_from_slice(&0u32.to_le_bytes());
                table.extend_from_slice(&8u32.to_le_bytes());
            }
            let mut file = exit_exe(0x1_4000_1000);
            let size = (table.len() as u32).next_multiple_of(4096);
            let at = append_section(&mut file, ".rel2", &table, size, RDATA).ok_or("no room for .rel2")?;
            set_directory(&mut file, 5, at.rva, table.len() as u32);
            write_file(BAD_RELOC, &file, at.file_len)?;
            let result = (|| {
                let prepared = program::prepare(BAD_RELOC.as_bytes(), &[BAD_RELOC.as_bytes()], &[], false)
                    .map_err(|_| "the image was refused before its relocation")?;
                match prepared {
                    Prepared::Nt { process, image, launch } => {
                        Ok(super::super::spawn_pe_with(&process, image, crate::user::DEFAULT_HOOK, &launch))
                    }
                    _ => Err("not a PE"),
                }
            })();
            remove(BAD_RELOC);
            match result? {
                Err(super::super::SpawnError::Image(s)) if s == STATUS_NO_MEMORY => {}
                Err(e) => {
                    crate::kprintln!("nt: the unrelocatable big image failed with {e:?}, not STATUS_NO_MEMORY");
                    return Err("process creation did not report STATUS_NO_MEMORY");
                }
                Ok(()) => return Err("an image whose relocation table does not fit started"),
            }
            // The kernel still allocates: nothing was left locked or broken.
            let again = vec![0u8; 1 << 20];
            if again.len() != 1 << 20 {
                return Err("the heap is broken after the failed allocation");
            }
            // And a plain image file of the same kind opens on its own: only
            // its relocation is what does not fit.
            let vfs = crate::fs::vfs();
            let ctx = vfs.root_context().map_err(|_| "no VFS")?;
            write_file(BAD_RELOC, &file, at.file_len)?;
            let f = vfs
                .open(&ctx, &Cred::root(), At::Cwd, BAD_RELOC.as_bytes(), OpenFlags::RDONLY, 0)
                .map_err(|_| "cannot reopen the image");
            let opened = f.map(|f| ImageSection::from_reader(Arc::new(VfsFile(f))));
            remove(BAD_RELOC);
            match opened? {
                Ok(img) => {
                    let r = img.relocated(img.image_base() + 0x10_0000);
                    if r.err() != Some(STATUS_NO_MEMORY) {
                        return Err("relocating the big table did not fail with STATUS_NO_MEMORY");
                    }
                }
                Err(_) => return Err("the big image did not open"),
            }
            Ok(())
        }
    }
}

//! arm64 Mach-O programs on x86-64 Poly (ADR 0257).
//!
//! The process runs the translator's runtime ([`RUNTIME`], an x86-64 ELF
//! program loaded by `program.rs`) with the arm64 program mapped beside it:
//! [`prepare`] links the program with Poly's dyld for arm64, lets
//! `poly_a64_xnu::load` map and fix it up through [`KernelTarget`], maps the
//! runtime's heap and its code area (readable, writable and executable) and
//! writes the hand-off block, whose address the runtime gets in `rdi`. The
//! process is an ordinary XNU process: the runtime's system calls, its own
//! and the guest's, come in through `crate::xnu::syscall`.

use super::{Loaded, VfsFiles};
use crate::user::Entry;
use crate::user::memory::{Memory, Placement, Prot};
use alloc::format;
use alloc::string::{String, ToString};
use alloc::sync::Arc;
use poly_a64_xnu::layout::{self, Layout};
use poly_a64_xnu::load::{Args, Target};

/// Where the runtime is installed.
pub const RUNTIME: &str = "/usr/libexec/poly-a64";

/// The new process's memory, as `poly_a64_xnu::load` sees it.
struct KernelTarget<'a>(&'a Memory);

impl Target for KernelTarget<'_> {
    fn map(&mut self, addr: u64, len: u64) -> Result<(), String> {
        self.0.map_anonymous(Placement::FixedNoReplace(addr), len, Prot::RW).map(|_| ()).map_err(|e| format!("{e:?}"))
    }

    fn write(&mut self, addr: u64, data: &[u8]) -> Result<(), String> {
        self.0.write(addr, data).map_err(|e| format!("{addr:#x}: {e:?}"))
    }

    fn read(&mut self, addr: u64, buf: &mut [u8]) -> Result<(), String> {
        self.0.read(addr, buf).map_err(|e| format!("{addr:#x}: {e:?}"))
    }
}

/// Whether the universal Mach-O file at `path` runs through the translator:
/// it has an arm64 slice and no x86-64 one.
pub fn is_arm64_only(path: &[u8]) -> Result<bool, String> {
    let path = core::str::from_utf8(path).map_err(|_| "the path is not UTF-8".to_string())?;
    let data = poly_dyld::FileSystem::read(&mut VfsFiles, path).ok_or("the file cannot be read")?;
    match poly_a64_xnu::runnable_cpu(&data) {
        Some(poly_a64_xnu::CpuKind::X86_64) => Ok(false),
        Some(_) => Ok(true),
        None => Err("a universal Mach-O file without an x86-64 or arm64 slice".into()),
    }
}

/// Link and map the arm64 program at `path` into `memory`, which holds the
/// runtime loaded at `entry`; the runtime is not started.
pub fn prepare(
    memory: Arc<Memory>,
    mut entry: Entry,
    path: &[u8],
    argv: &[&[u8]],
    envp: &[&[u8]],
) -> Result<Loaded, String> {
    let path = core::str::from_utf8(path).map_err(|_| "the path is not UTF-8".to_string())?;
    let program = poly_dyld::link(&mut VfsFiles, path, &poly_dyld::Options::ARM64).map_err(|e| e.to_string())?;
    let mut handoff = poly_a64_xnu::load::load(
        &mut KernelTarget(&memory),
        &program,
        &Args { path, argv, envp, layout: &Layout::POLY },
    )?;

    // The runtime's heap and the translator's code area.
    let map = |at: u64, len: u64, prot: Prot| {
        memory.map_anonymous(Placement::FixedNoReplace(at), len, prot).map_err(|e| format!("no memory: {e:?}"))
    };
    let rwx = Prot::from_bits(0x7).expect("rwx");
    map(layout::RUNTIME_HEAP, layout::RUNTIME_HEAP_SIZE, Prot::RW)?;
    map(layout::CODE_AREA, layout::CODE_AREA_SIZE, rwx)?;
    handoff.heap = layout::RUNTIME_HEAP;
    handoff.heap_size = layout::RUNTIME_HEAP_SIZE;
    handoff.code = layout::CODE_AREA;
    handoff.code_size = layout::CODE_AREA_SIZE;

    let bytes = handoff.to_bytes();
    if bytes.len() as u64 > layout::HANDOFF_SIZE {
        return Err("the program has too many segments for the translator's hand-off".into());
    }
    let len = (bytes.len() as u64 + 4095) & !4095;
    map(layout::HANDOFF, len, Prot::RW)?;
    memory.write(layout::HANDOFF, &bytes).map_err(|_| "the hand-off block cannot be written".to_string())?;
    memory.protect(layout::HANDOFF, len, Prot::READ).map_err(|e| format!("{e:?}"))?;
    entry.args[0] = layout::HANDOFF;
    Ok(Loaded { memory, entry })
}

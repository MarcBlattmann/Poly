//! An arm64 Mach-O program in guest memory, and its system calls.
//!
//! This is the minimal loading a translated test program needs: map the
//! segments on 16 KiB pages with their protections, apply chained rebases,
//! give `main` a stack and a return address that exits. Programs with
//! imports are refused; real programs go through Poly's dyld (ADR 0237),
//! which places images the same way and hands over a `Cpu` and memory.

use alloc::vec::Vec;

use poly_a64::Cpu;
use poly_macho::{Entry, FixupKind, MachO};

use crate::mem::{GuestMem, PAGE_SIZE, R, W, X};

/// Why loading failed.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum LoadError {
    /// The file is not a Mach-O file poly-macho accepts.
    Parse,
    /// It is not an arm64 file, or it needs something this loader lacks.
    Unsupported(&'static str),
    /// A segment does not fit in the guest window.
    Range,
}

/// Size of the main thread's stack.
pub const STACK_SIZE: u64 = 1 << 20;

/// The `svc` the return stub uses: `main`'s return value goes to `exit`.
const EXIT_STUB: [u32; 2] = [
    0xD280_0030, // mov x16, #1 (SYS_exit)
    0xD400_1001, // svc #0x80
];

/// A loaded program: its memory and the main thread's registers, ready to
/// run at `main(argc, argv, envp, apple)`.
#[derive(Debug)]
pub struct Loaded {
    /// The guest memory.
    pub mem: GuestMem,
    /// The main thread, at `main`.
    pub cpu: Cpu,
}

/// Load an arm64 program into a guest window of `size` bytes starting at the
/// program's `__TEXT` (its preferred address). The stack and the return
/// stub go at the top of the window.
pub fn load(file: &[u8], size: u64, args: &[&str]) -> Result<Loaded, LoadError> {
    let m = MachO::parse(file).map_err(|_| LoadError::Parse)?;
    let header = m.header_segment().ok_or(LoadError::Unsupported("no __TEXT"))?;
    let base = header.vmaddr;
    let lo = base & !(PAGE_SIZE - 1);
    let mut mem = GuestMem::new(lo, size);

    for s in m.segment_table() {
        if s.is_pagezero() || s.vmsize == 0 {
            continue;
        }
        let start = s.vmaddr & !(PAGE_SIZE - 1);
        let end = (s.vm_end() + PAGE_SIZE - 1) & !(PAGE_SIZE - 1);
        // Mapped writable first so the file bytes and fixups go in.
        mem.map(start, end - start, R | W, 0).map_err(|_| LoadError::Range)?;
        let range = s.file_range();
        let bytes = file.get(range.start as usize..range.end as usize).ok_or(LoadError::Parse)?;
        mem.poke(s.vmaddr, bytes).map_err(|_| LoadError::Range)?;
    }

    if let Some(cf) = m.chained_fixups().map_err(|_| LoadError::Parse)? {
        for f in cf.fixups() {
            let f = f.map_err(|_| LoadError::Parse)?;
            let value = match f.kind {
                FixupKind::Rebase(r) => {
                    let t = if r.target_is_vmaddr { r.target } else { base + r.target };
                    t | (u64::from(r.high8) << 56)
                }
                FixupKind::Bind(_) => return Err(LoadError::Unsupported("imports need dyld")),
            };
            mem.poke(base + f.vm_offset, &value.to_le_bytes()).map_err(|_| LoadError::Range)?;
        }
    }
    if m.dyld_info().map_err(|_| LoadError::Parse)?.is_some() {
        return Err(LoadError::Unsupported("classic rebase and bind opcodes need dyld"));
    }

    // Final protections.
    for s in m.segment_table() {
        if s.is_pagezero() || s.vmsize == 0 {
            continue;
        }
        let start = s.vmaddr & !(PAGE_SIZE - 1);
        let end = (s.vm_end() + PAGE_SIZE - 1) & !(PAGE_SIZE - 1);
        let p = s.initprot.0 as u8 & (R | W | X);
        mem.protect(start, end - start, p).map_err(|_| LoadError::Range)?;
    }

    // Stack at the top of the window, the exit stub one page below it.
    let top = lo + size;
    let stack_lo = top - STACK_SIZE;
    mem.map(stack_lo, STACK_SIZE, R | W, 0).map_err(|_| LoadError::Range)?;
    let stub = stack_lo - 2 * PAGE_SIZE;
    mem.map(stub, PAGE_SIZE, R | W, 0).map_err(|_| LoadError::Range)?;
    let code: Vec<u8> = EXIT_STUB.iter().flat_map(|w| w.to_le_bytes()).collect();
    mem.poke(stub, &code).map_err(|_| LoadError::Range)?;
    mem.protect(stub, PAGE_SIZE, R | X).map_err(|_| LoadError::Range)?;

    // argv strings, then the pointer arrays: argv, envp (empty), apple (empty).
    let mut sp = top - 64;
    let mut ptrs = Vec::new();
    for a in args {
        sp -= a.len() as u64 + 1;
        mem.poke(sp, a.as_bytes()).map_err(|_| LoadError::Range)?;
        mem.poke(sp + a.len() as u64, &[0]).map_err(|_| LoadError::Range)?;
        ptrs.push(sp);
    }
    sp &= !15;
    let words = ptrs.len() + 3;
    sp -= (words as u64 * 8 + 15) & !15;
    let argv = sp;
    for (i, p) in ptrs.iter().enumerate() {
        mem.poke(argv + 8 * i as u64, &p.to_le_bytes()).map_err(|_| LoadError::Range)?;
    }
    let envp = argv + 8 * (ptrs.len() as u64 + 1);
    let apple = envp + 8;

    let pc = match m.entry().map_err(|_| LoadError::Parse)? {
        Some(Entry::Main { entryoff, .. }) => base + entryoff,
        Some(Entry::UnixThread { pc, .. }) => pc,
        None => return Err(LoadError::Unsupported("no entry point")),
    };
    let mut cpu = Cpu::new();
    cpu.pc = pc;
    cpu.sp = sp;
    cpu.x[0] = args.len() as u64;
    cpu.x[1] = argv;
    cpu.x[2] = envp;
    cpu.x[3] = apple;
    cpu.x[30] = stub;
    Ok(Loaded { mem, cpu })
}

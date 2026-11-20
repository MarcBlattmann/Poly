//! Loading an arm64 program for the translator: everything the kernel does
//! after `poly_dyld::link`.
//!
//! [`load`] maps every segment of every image at its address and writes its
//! bytes, writes every fixup, maps the arm64 start code
//! (`poly_dyld::start::ARM64_START`) with its data block, one thread-local
//! block per image for the main thread, the main thread's TSD page and its
//! stack, and the area the runtime places the guest's `mmap` mappings in. All of it is
//! readable and writable for the host; what the guest may do is in the
//! [`Handoff`]'s regions, which the runtime turns into its page flags.
//!
//! The thread starts like an x86-64 one (ADR 0237): a static program
//! (`LC_UNIXTHREAD` without `LC_LOAD_DYLINKER`) at its entry point with
//! `sp` at `argc`; everything else in the start code with `sp` at the
//! program's header address, below `argc`.

use alloc::format;
use alloc::string::{String, ToString};
use alloc::vec;
use alloc::vec::Vec;

use poly_a64_jit::mem::{R, W, X};
use poly_dyld::{Entry, Program, start};

use crate::handoff::{Handoff, Region};
use crate::layout::{Layout, page_up};

/// Where [`load`] puts things: the kernel's view of the new process's
/// address space.
pub trait Target {
    /// Map `len` bytes at `addr` (16 KiB aligned), zero-filled, readable
    /// and writable by the host.
    fn map(&mut self, addr: u64, len: u64) -> Result<(), String>;
    /// Write bytes into mapped memory.
    fn write(&mut self, addr: u64, data: &[u8]) -> Result<(), String>;
    /// Read bytes from mapped memory.
    fn read(&mut self, addr: u64, buf: &mut [u8]) -> Result<(), String>;
}

/// What the program is started with.
#[derive(Clone, Copy, Debug)]
pub struct Args<'a> {
    pub path: &'a str,
    pub argv: &'a [&'a [u8]],
    pub envp: &'a [&'a [u8]],
    pub layout: &'a Layout,
}

/// Load the linked arm64 `program` into `t` (see the module docs). The
/// result's heap and code area are left 0 for the caller.
pub fn load(t: &mut dyn Target, program: &Program, args: &Args<'_>) -> Result<Handoff, String> {
    let l = args.layout;
    let mut regions = Vec::new();

    // Every segment, writable while its bytes and the fixups go in.
    for im in &program.images {
        for s in &im.segments {
            let end = s.addr.checked_add(s.mem_size).ok_or("a segment wraps")?;
            if s.addr < l.window_lo || end > l.mmap_lo || s.addr % 16384 != 0 {
                return Err(format!(
                    "{}: segment {} at {:#x} is outside the translated program's window ({:#x}..{:#x})",
                    im.path, s.name, s.addr, l.window_lo, l.mmap_lo
                ));
            }
            let len = page_up(s.mem_size);
            t.map(s.addr, len).map_err(|e| format!("{}: segment {}: {e}", im.path, s.name))?;
            let n = s.file_size.min(s.mem_size) as usize;
            let from = s.file_offset as usize;
            let bytes = im
                .file
                .get(from..from.checked_add(n).ok_or("a segment wraps")?)
                .ok_or_else(|| format!("{}: segment {} outside the file", im.path, s.name))?;
            t.write(s.addr, bytes)?;
            let mut prot = 0;
            if s.prot.read {
                prot |= R;
            }
            if s.prot.write && !s.read_only_after_fixups {
                prot |= W;
            }
            if s.prot.execute {
                prot |= X | R;
            }
            regions.push(Region { addr: s.addr, len, prot });
        }
    }
    let mut failed = None;
    program
        .fixups(&mut |addr, value| {
            if failed.is_none()
                && let Err(e) = t.write(addr, &value.to_le_bytes())
            {
                failed = Some(format!("a fixup at {addr:#x}: {e}"));
            }
        })
        .map_err(|e| e.to_string())?;
    if let Some(e) = failed {
        return Err(e);
    }

    // The system area: start code, thread-local blocks, the TSD page.
    let mut system = Bump { next: l.system, end: l.system_end };
    let header = program.images[0].base;
    let (pc, with_header) = match (program.dynamic, program.entry) {
        (false, Entry::Thread { pc, .. }) => (pc, false),
        (_, entry) => (start_code(t, &mut system, &mut regions, program, entry)?, true),
    };
    let tsd = system.take(16384)?;
    t.map(tsd, 16384)?;
    regions.push(Region { addr: tsd, len: 16384, prot: R | W });

    // The stack: the header address (when the start code runs), argc, argv,
    // envp, apple, the strings.
    let stack_lo = l.stack_top - l.stack_size;
    t.map(stack_lo, l.stack_size)?;
    regions.push(Region { addr: stack_lo, len: l.stack_size, prot: R | W });
    let apple0 = format!("executable_path={}", args.path);
    let apple: [&[u8]; 1] = [apple0.as_bytes()];
    let (stack, _) = poly_dyld::stack::build(l.stack_top, args.argv, args.envp, &apple, with_header.then_some(header))
        .ok_or("the arguments do not fit the stack")?;
    if stack.bytes.len() as u64 > l.stack_size / 2 {
        return Err("the arguments do not fit the stack".into());
    }
    t.write(stack.sp, &stack.bytes)?;

    // The area of the guest's mmap mappings: reserved here, so nothing else
    // the kernel maps for the process lands in it; the runtime places each
    // mapping in it with MAP_FIXED (run::Bridge).
    t.map(l.mmap_lo, l.mmap_end - l.mmap_lo)?;

    let (engine, stats) = crate::handoff::settings_from_env(args.envp);
    Ok(Handoff {
        pc,
        sp: stack.sp,
        tpidrro: tsd,
        window_lo: l.window_lo,
        window_end: l.window_end,
        mmap_lo: l.mmap_lo,
        mmap_end: l.mmap_end,
        heap: 0,
        heap_size: 0,
        code: 0,
        code_size: 0,
        engine,
        stats,
        regions,
        path: args.path.to_string(),
    })
}

/// Hands out 16 KiB pages of the system area.
struct Bump {
    next: u64,
    end: u64,
}

impl Bump {
    fn take(&mut self, len: u64) -> Result<u64, String> {
        let at = self.next;
        let end = at.checked_add(page_up(len.max(1))).ok_or("the system area is full")?;
        if end > self.end {
            return Err("the system area is full".into());
        }
        self.next = end;
        Ok(at)
    }
}

/// Map the start code and its data block, and the thread-local blocks of
/// the main thread; returns where the thread starts.
fn start_code(
    t: &mut dyn Target,
    system: &mut Bump,
    regions: &mut Vec<Region>,
    program: &Program,
    entry: Entry,
) -> Result<u64, String> {
    let (target, mode) = match entry {
        Entry::Main { addr, .. } => (addr, start::Mode::CallMain),
        Entry::Thread { pc, .. } => (pc, start::Mode::Jump),
    };
    let data = start::StartData {
        entry: target,
        mode,
        exit: program.exit_function().map_err(|e| e.to_string())?.unwrap_or(0),
        initializers: program.initializers(),
    }
    .to_bytes();
    let len = page_up(start::DATA_OFFSET + data.len() as u64);
    let code = system.take(len)?;
    t.map(code, len)?;
    t.write(code, &start::arm64_start_bytes())?;
    t.write(code + start::DATA_OFFSET, &data)?;
    regions.push(Region { addr: code, len, prot: R | X });

    // Thread-local variables: one block per image, a copy of its template
    // as the fixups left it.
    let thunk = code + start::ARM64_TLV_GET;
    for (i, im) in program.images.iter().enumerate() {
        let Some(tlv) = &im.tlv else { continue };
        let size = page_up(tlv.size.max(1));
        let block = system.take(size)?;
        t.map(block, size)?;
        if tlv.init_size > 0 {
            let mut init = vec![0u8; usize::try_from(tlv.init_size).map_err(|_| "a huge thread-local template")?];
            t.read(tlv.template, &mut init).map_err(|e| format!("{}: thread-local template: {e}", im.path))?;
            t.write(block, &init)?;
        }
        regions.push(Region { addr: block, len: size, prot: R | W });
        let mut failed = None;
        program.tlv_writes(i, block, thunk, &mut |a, v| {
            if failed.is_none()
                && let Err(e) = t.write(a, &v.to_le_bytes())
            {
                failed = Some(e);
            }
        });
        if let Some(e) = failed {
            return Err(format!("{}: thread-local descriptors: {e}", im.path));
        }
    }
    Ok(code)
}

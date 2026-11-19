//! The code cache, block chaining, invalidation and the run loop (ADR 0239).
//!
//! [`Engine::run`] runs a guest thread until it exits, faults, or uses up its
//! instruction budget. On an x86-64 host with a code area ([`Engine::with_code`])
//! it translates each basic block once and runs the translation; everywhere
//! else, and for every instruction the translator leaves out, poly-a64's
//! interpreter runs. The two give the same results (the differential tests
//! check it), so the interpreter is the reference and the fallback.
//!
//! **Code cache.** Translations live in one code area the host provides
//! (readable, writable and executable). A block is found by its guest
//! address in a map. When the area is full, everything is dropped and
//! translation starts over.
//!
//! **Chaining.** A direct exit of a block first jumps to a stub that returns
//! to the run loop and reports the exit. The run loop then patches the
//! exit's jump to go straight to the next block's translation, so hot loops
//! never leave translated code (except to check the budget). Indirect
//! branches look the target up in the indirect branch cache.
//!
//! **Invalidation.** Translating a block marks its 16 KiB page as code
//! ([`crate::mem::CODE`]). A write to such a page (from translated code, the
//! interpreter, the loader, `mmap`/`mprotect`, or a `MAP_JIT` mode change)
//! records the page; before running anything the run loop drops every block
//! of a recorded page: chained jumps into it are pointed back at their
//! stubs, its entry leaves the map and the indirect branch cache is cleared.
//! `IC IVAU` needs no work of its own: every write that can change code
//! already goes through this.

use alloc::collections::BTreeMap;
use alloc::vec;
use alloc::vec::Vec;

use poly_a64::{Cpu, Inst, MemFault, Memory, Outcome, decode};

use crate::mem::{GuestMem, PAGE_SIZE};
use crate::translate::{self, IBTC_BITS, exit};
use crate::x64::{Asm, R, Sz, mem};

/// What the translated code reads from `rbp`. The offsets are those of
/// [`crate::translate`]'s `ctx` module.
#[repr(C)]
#[derive(Debug)]
pub struct Ctx {
    cpu: *mut Cpu,
    host: *mut u8,
    flags: *const u8,
    lo: u64,
    size: u64,
    /// Blocks left to enter before returning to the run loop.
    budget: i64,
    /// Where the entry routine jumps.
    entry: *const u8,
    /// The `SVC` immediate.
    arg: u64,
    /// The code-area offset of the exit jump that returned (-1: none).
    site: i32,
    _pad: i32,
    /// The indirect branch cache: `(guest pc, host address)` pairs.
    ibtc: *mut [u64; 2],
}

/// Why [`Engine::run`] returned.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Stop {
    /// The kernel callback asked to stop (for example `exit`), with a code.
    Exit(i32),
    /// The instruction budget is used up; `run` can be called again.
    Budget,
    /// A memory fault; `PC` is at the instruction.
    Fault(MemFault),
    /// `BRK #imm`.
    Brk(u16),
    /// An undefined instruction.
    Undefined(u32),
    /// An instruction poly-a64 does not implement.
    Unsupported(u32),
}

/// The system call handler of a translated thread.
pub trait Kernel {
    /// Handle `SVC #imm` (`PC` is already past it). `Some(code)` stops the
    /// thread (an `exit`).
    fn svc(&mut self, imm: u16, cpu: &mut Cpu, mem: &mut GuestMem) -> Option<i32>;
}

/// Counters for tests and measurements.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Stats {
    /// Blocks translated.
    pub translated: u64,
    /// Guest instructions in translated blocks (counted once per block).
    pub translated_insts: u64,
    /// Instructions run by the interpreter.
    pub interpreted: u64,
    /// Entries into translated code from the run loop.
    pub entries: u64,
    /// Exits that were chained.
    pub chained: u64,
    /// Blocks dropped by invalidation.
    pub invalidated: u64,
    /// Times the whole cache was flushed.
    pub flushes: u64,
}

#[derive(Debug)]
struct BlockInfo {
    /// Offset of the translation in the code area.
    host: usize,
    /// This block's exits: (offset of the jmp rel32 field, offset of its stub).
    exits: Vec<(usize, usize)>,
    /// Exits of other blocks patched to jump here: (jmp field, stub).
    incoming: Vec<(usize, usize)>,
}

#[derive(Debug)]
struct Code {
    base: *mut u8,
    len: usize,
    used: usize,
    exit_routine: usize,
    entry_routine: usize,
}

/// The run loop of one guest thread, with its code cache.
#[derive(Debug)]
pub struct Engine {
    code: Option<Code>,
    blocks: BTreeMap<u64, BlockInfo>,
    /// Exit-site offset → (owning block, target pc), for chaining.
    sites: BTreeMap<usize, (u64, u64)>,
    ibtc: Vec<[u64; 2]>,
    /// Blocks per guest page (for invalidation).
    pages: BTreeMap<u64, Vec<u64>>,
    /// Counters.
    pub stats: Stats,
    /// Most instructions per translated block.
    pub max_block: usize,
    ctx_arg: u64,
}

impl Default for Engine {
    fn default() -> Self {
        Engine::interpreter()
    }
}

impl Engine {
    /// An engine that only interprets (any host).
    pub fn interpreter() -> Engine {
        Engine {
            code: None,
            blocks: BTreeMap::new(),
            sites: BTreeMap::new(),
            ibtc: vec![[u64::MAX, 0]; 1 << IBTC_BITS],
            pages: BTreeMap::new(),
            stats: Stats::default(),
            max_block: 64,
            ctx_arg: 0,
        }
    }

    /// An engine that translates into the code area `base..base + len`.
    ///
    /// # Safety
    ///
    /// The area must be readable, writable and executable by this thread,
    /// used by nothing else, and outlive the engine. The host must be
    /// x86-64 (`run` executes the area).
    pub unsafe fn with_code(base: *mut u8, len: usize) -> Engine {
        let mut e = Engine::interpreter();
        let mut a = Asm::new();
        // Entry: extern "sysv64" fn(ctx: *mut Ctx) -> u32.
        let entry_routine = a.pos();
        for r in [R::Rbx, R::Rbp, R::R12, R::R13, R::R14, R::R15] {
            a.push(r);
        }
        a.alu_ri(Sz::Q, crate::x64::Alu::Sub, R::Rsp, 8);
        a.mov_rr(Sz::Q, R::Rbp, R::Rdi);
        a.load(Sz::Q, R::Rbx, mem(R::Rbp, 0));
        a.load(Sz::Q, R::R12, mem(R::Rbp, 8));
        a.load(Sz::Q, R::R13, mem(R::Rbp, 16));
        a.load(Sz::Q, R::R14, mem(R::Rbp, 24));
        a.load(Sz::Q, R::R15, mem(R::Rbp, 32));
        a.jmp_m(mem(R::Rbp, 48));
        // Exit: eax holds the exit code.
        let exit_routine = a.pos();
        a.alu_ri(Sz::Q, crate::x64::Alu::Add, R::Rsp, 8);
        for r in [R::R15, R::R14, R::R13, R::R12, R::Rbp, R::Rbx] {
            a.pop(r);
        }
        a.ret();
        assert!(a.buf.len() <= len);
        // SAFETY: the caller gave us `len` writable bytes at `base`.
        unsafe { core::ptr::copy_nonoverlapping(a.buf.as_ptr(), base, a.buf.len()) };
        let used = (a.buf.len() + 15) & !15;
        e.code = Some(Code { base, len, used, exit_routine, entry_routine });
        e
    }

    /// Whether this engine translates.
    pub fn translates(&self) -> bool {
        self.code.is_some()
    }

    /// Drop every translation.
    pub fn flush(&mut self) {
        self.blocks.clear();
        self.sites.clear();
        self.pages.clear();
        for e in self.ibtc.iter_mut() {
            *e = [u64::MAX, 0];
        }
        if let Some(c) = &mut self.code {
            c.used = (c.exit_routine + 64) & !15;
            self.stats.flushes += 1;
        }
    }

    fn invalidate_page(&mut self, page: u64) {
        let Some(list) = self.pages.remove(&page) else { return };
        for pc in list {
            let Some(b) = self.blocks.remove(&pc) else { continue };
            self.stats.invalidated += 1;
            let base = self.code.as_ref().map(|c| c.base).unwrap_or(core::ptr::null_mut());
            // Unchain the jumps into this block.
            for (field, stub) in b.incoming {
                patch_rel32(base, field, stub);
            }
            for (field, _) in &b.exits {
                self.sites.remove(field);
            }
        }
        for e in self.ibtc.iter_mut() {
            *e = [u64::MAX, 0];
        }
    }

    fn apply_invalidations(&mut self, mem: &mut GuestMem) {
        if mem.has_written_code() {
            for p in mem.take_written_code() {
                self.invalidate_page(p);
            }
        }
    }

    /// Translate the block at `pc` if possible; its code-area offset.
    fn block_at(&mut self, pc: u64, mem: &mut GuestMem) -> Option<usize> {
        if let Some(b) = self.blocks.get(&pc) {
            return Some(b.host);
        }
        let code = self.code.as_ref()?;
        // Decode up to the end of the page (fetch checks execute permission).
        let page_end = (pc | (PAGE_SIZE - 1)).wrapping_add(1);
        let mut insts: Vec<Inst> = Vec::new();
        let mut a = pc;
        while a < page_end && insts.len() < self.max_block {
            let Ok(w) = mem.fetch(a) else { break };
            let i = decode(w);
            let stop = !translate::translatable(&i);
            insts.push(i);
            if stop {
                break;
            }
            a += 4;
        }
        let mut block = translate::translate(pc, &insts, code.exit_routine)?;
        // Place it.
        let need = block.asm.buf.len();
        if code.used + need + 64 > code.len {
            self.flush();
        }
        let code = self.code.as_mut()?;
        if code.used + need + 64 > code.len {
            return None;
        }
        let at = code.used;
        block.asm.place(at);
        // Stubs report their exit's offset in the code area.
        for e in &block.exits {
            let site = (at + e.jmp_field) as u32;
            block.asm.buf[e.report_field..e.report_field + 4].copy_from_slice(&site.to_le_bytes());
        }
        // SAFETY: `at + need` is inside the code area (checked above).
        unsafe { core::ptr::copy_nonoverlapping(block.asm.buf.as_ptr(), code.base.add(at), need) };
        code.used = (at + need + 15) & !15;
        let exits: Vec<(usize, usize)> = block.exits.iter().map(|e| (at + e.jmp_field, at + e.stub)).collect();
        for (e, &(field, _)) in block.exits.iter().zip(&exits) {
            self.sites.insert(field, (pc, e.target));
        }
        mem.mark_code(pc, block.end - pc);
        self.pages.entry(pc & !(PAGE_SIZE - 1)).or_default().push(pc);
        self.blocks.insert(pc, BlockInfo { host: at, exits, incoming: Vec::new() });
        self.stats.translated += 1;
        self.stats.translated_insts += block.insts as u64;
        Some(at)
    }

    /// Point the exit at code-area offset `site` to the block at `target`.
    fn chain(&mut self, site: usize, target_host: usize, target_pc: u64) {
        let Some(&(owner, t)) = self.sites.get(&site) else { return };
        if t != target_pc || !self.blocks.contains_key(&owner) {
            return;
        }
        let Some(stub) = self.blocks.get(&owner).and_then(|b| b.exits.iter().find(|e| e.0 == site).map(|e| e.1)) else {
            return;
        };
        let base = self.code.as_ref().map(|c| c.base).unwrap_or(core::ptr::null_mut());
        patch_rel32(base, site, target_host);
        if let Some(b) = self.blocks.get_mut(&target_pc) {
            b.incoming.push((site, stub));
        }
        self.stats.chained += 1;
    }

    /// Run the thread for at most about `budget` blocks (translated) or
    /// instructions (interpreted).
    pub fn run<K: Kernel>(&mut self, cpu: &mut Cpu, mem: &mut GuestMem, kernel: &mut K, budget: u64) -> Stop {
        let mut left = budget as i64;
        let mut pending_site: Option<usize> = None;
        loop {
            self.apply_invalidations(mem);
            if left <= 0 {
                return Stop::Budget;
            }
            let pc = cpu.pc;
            let host = if self.code.is_some() { self.block_at(pc, mem) } else { None };
            let Some(host) = host else {
                pending_site = None;
                // Interpret one instruction.
                left -= 1;
                self.stats.interpreted += 1;
                match cpu.step(mem) {
                    Outcome::Continue => {}
                    Outcome::Svc(imm) => {
                        if let Some(code) = kernel.svc(imm, cpu, mem) {
                            return Stop::Exit(code);
                        }
                    }
                    Outcome::Brk(i) => return Stop::Brk(i),
                    Outcome::Fault(f) => return Stop::Fault(f),
                    Outcome::Undefined(w) => return Stop::Undefined(w),
                    Outcome::Unsupported(w) => return Stop::Unsupported(w),
                }
                continue;
            };
            if let Some(site) = pending_site.take() {
                self.chain(site, host, pc);
            }
            // Remember it for indirect branches.
            if let Some(code) = &self.code {
                let slot = ((pc >> 2) as usize) & ((1 << IBTC_BITS) - 1);
                // SAFETY: `host` is an offset inside the code area.
                self.ibtc[slot] = [pc, unsafe { code.base.add(host) } as u64];
            }
            let r = self.enter(cpu, mem, host, &mut left);
            match r {
                (exit::NEXT, site) => {
                    if site >= 0 {
                        pending_site = Some(site as usize);
                    }
                }
                (exit::INTERPRET, _) => {
                    self.stats.interpreted += 1;
                    match cpu.step(mem) {
                        Outcome::Continue => {}
                        Outcome::Svc(imm) => {
                            if let Some(code) = kernel.svc(imm, cpu, mem) {
                                return Stop::Exit(code);
                            }
                        }
                        Outcome::Brk(i) => return Stop::Brk(i),
                        Outcome::Fault(f) => return Stop::Fault(f),
                        Outcome::Undefined(w) => return Stop::Undefined(w),
                        Outcome::Unsupported(w) => return Stop::Unsupported(w),
                    }
                }
                (_, _) => {
                    // exit::SVC
                    let imm = self.ctx_arg as u16;
                    if let Some(code) = kernel.svc(imm, cpu, mem) {
                        return Stop::Exit(code);
                    }
                }
            }
        }
    }

    #[cfg(target_arch = "x86_64")]
    fn enter(&mut self, cpu: &mut Cpu, mem: &mut GuestMem, host: usize, left: &mut i64) -> (u32, i32) {
        let code = self.code.as_ref().expect("translating engine");
        let mut ctx = Ctx {
            cpu,
            host: mem.host_base(),
            flags: mem.flag_table(),
            lo: mem.lo(),
            size: mem.size(),
            budget: *left,
            // SAFETY: `host` is inside the code area.
            entry: unsafe { code.base.add(host) },
            arg: 0,
            site: -1,
            _pad: 0,
            ibtc: self.ibtc.as_mut_ptr(),
        };
        // SAFETY: the entry routine was written at `entry_routine` by
        // `with_code` and follows the System V calling convention; the code
        // it runs only touches the Cpu, the guest window and the flag table
        // (bounds-checked), the context and its own stack frame.
        let f: extern "sysv64" fn(*mut Ctx) -> u32 =
            unsafe { core::mem::transmute(code.base.add(code.entry_routine)) };
        let r = f(&mut ctx);
        self.stats.entries += 1;
        *left = ctx.budget.min(*left - 1);
        self.ctx_arg = ctx.arg;
        (r, ctx.site)
    }

    #[cfg(not(target_arch = "x86_64"))]
    fn enter(&mut self, _cpu: &mut Cpu, _mem: &mut GuestMem, _host: usize, _left: &mut i64) -> (u32, i32) {
        unreachable!("translated code runs only on x86-64 hosts")
    }
}

fn patch_rel32(base: *mut u8, field: usize, target: usize) {
    let rel = (target as i64 - (field as i64 + 4)) as i32;
    // SAFETY: `field` is the rel32 of a jump this engine placed in its code
    // area; nothing executes while the run loop patches.
    unsafe { core::ptr::copy_nonoverlapping(rel.to_le_bytes().as_ptr(), base.add(field), 4) };
}

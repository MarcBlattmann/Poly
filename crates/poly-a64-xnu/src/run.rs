//! The runtime's side of a translated process: the guest from a hand-off,
//! the run loop, the guest's system calls, and how a guest that stops ends.
//!
//! [`Bridge`] is the translator's [`Kernel`]: a guest's `svc #0x80` comes
//! here with the call in `x16`. Calls go to the XNU personality through
//! [`Host::syscall`] in the x86-64 convention (`arm64::to_frame`, then
//! `arm64::from_frame` for `x0` and the carry flag), so the personality has
//! one copy of every call and does not know the caller was translated.
//! Guest addresses are host addresses, so pointer arguments need no change.
//!
//! The calls that change the guest's memory go to the kernel too, but the
//! bridge shapes them first, because guest memory must stay in the window
//! and the window's page flags must follow it (the kernel's BSD layer, ADR
//! 0246, places a mapping wherever it likes and knows 4 KiB pages):
//!
//! - `mmap` without `MAP_FIXED` gets an address in the window's anonymous
//!   area from the bridge, and the kernel maps there with `MAP_FIXED`;
//!   `MAP_FIXED` must land in the window. Lengths and file offsets are in
//!   16 KiB guest pages (an unaligned offset is that many bytes into a
//!   guest page, as the mmap sheet says).
//! - The host protection asked of the kernel is the guest's read and write
//!   bits (never execute: guest code is only read); the guest's own
//!   protection, execute included, goes into the page flags. `MAP_JIT`
//!   memory is readable and writable for the host in both of its modes.
//! - `munmap` and `mprotect` must stay in the window.
//!
//! `exit` is reported to the runtime, which prints its statistics and then
//! makes the call.
//!
//! **Threads.** The runtime runs guest threads on host threads, one at a
//! time (it holds one lock while translated code runs). With
//! [`Bridge::defer`] set, a call that goes to the kernel is not made inside
//! the run loop: the bridge keeps it ([`Bridge::pending`]) and stops the
//! guest with [`YIELD`], so the runtime can release its lock for a call
//! that may block, make it, and hand the result back
//! ([`Bridge::complete`]). `bsdthread_register` and `bsdthread_create` stay
//! here: a new guest thread gets a table of its own (its `TPIDRRO_EL0`:
//! slot 0 its thread structure, then a copy of every image's thread-local
//! template) and starts at the registered entry with Poly's contract (x0
//! the thread structure, x1 the start function, x2 its argument), on a host
//! thread the [`Host`] starts. `bsdthread_terminate` stops the guest thread
//! with [`YIELD`] and [`Bridge::terminated`] set.

use alloc::format;
use alloc::string::String;

use poly_a64::Cpu;
use poly_a64_jit::jit::{Engine, Kernel, Stop};
use poly_a64_jit::mem::{GuestMem, MAP_JIT, PAGE_SIZE, R, W, X};
use poly_abi::{RFLAGS_CF, SyscallRegs};
use poly_personality_xnu::Class;
use poly_personality_xnu::arm64::{self, SVC_IMM};

use crate::handoff::{EngineKind, Handoff, TlvImage};
use alloc::vec::Vec;

/// BSD call numbers (`sys/syscall.h`).
pub mod sys {
    pub const EXIT: u64 = 1;
    pub const BSDTHREAD_CREATE: u64 = 360;
    pub const BSDTHREAD_TERMINATE: u64 = 361;
    pub const BSDTHREAD_REGISTER: u64 = 366;
    pub const GETPID: u64 = 20;
    pub const KILL: u64 = 37;
    pub const MUNMAP: u64 = 73;
    pub const MPROTECT: u64 = 74;
    pub const MMAP: u64 = 197;
}

/// The x86-64 frame's class 2 (BSD) in the top byte of `rax`.
const BSD: u64 = 2 << 24;

/// `sys/mman.h`.
const MAP_FIXED: u64 = 0x10;
const MAP_ANON: u64 = 0x1000;
/// Darwin errno values (`sys/errno.h`).
const ENOMEM: u64 = 12;
const EINVAL: u64 = 22;

/// The `svc` result that stops the guest for the runtime: a deferred call
/// or the end of a guest thread (see the module docs).
pub const YIELD: i32 = i32::MIN;

/// `PTHREAD_START_CUSTOM`: the caller made the stack and thread structure.
const PTHREAD_START_CUSTOM: u64 = 0x0100_0000;

/// A new guest thread for the [`Host`] to start.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct GuestThread {
    pub pc: u64,
    pub sp: u64,
    /// x0, x1, x2: the thread structure, the start function, its argument.
    pub x: [u64; 3],
    pub tpidrro: u64,
}

impl GuestThread {
    /// The thread's first registers.
    pub fn cpu(&self) -> Cpu {
        let mut cpu = Cpu::new();
        cpu.pc = self.pc;
        cpu.sp = self.sp;
        cpu.x[..3].copy_from_slice(&self.x);
        cpu.tpidrro_el0 = self.tpidrro;
        cpu
    }
}

/// A call the bridge kept for the runtime to make ([`Bridge::defer`]).
#[derive(Clone, Copy, Debug)]
pub struct Pending {
    pub frame: SyscallRegs,
    pub stack: [u64; 2],
    pub x16: u64,
}

/// Signal numbers (`sys/signal.h`).
pub mod sig {
    pub const SIGILL: u32 = 4;
    pub const SIGTRAP: u32 = 5;
    pub const SIGBUS: u32 = 10;
    pub const SIGSEGV: u32 = 11;
    pub const SIGSYS: u32 = 12;
}

/// Where the runtime sends the guest's system calls.
pub trait Host {
    /// Make one x86-64 XNU system call: `frame` holds the class and number
    /// in `rax` and the first six arguments; `stack` the seventh and
    /// eighth. Afterwards `frame.rax` is the result and `RFLAGS_CF` in
    /// `frame.rflags` the carry flag. `mem` is the guest's memory, for a
    /// host that cannot read guest addresses directly (tests).
    fn syscall(&mut self, frame: &mut SyscallRegs, stack: [u64; 2], mem: &mut GuestMem);

    /// Start a host thread that runs the guest thread `t`; the errno if it
    /// cannot.
    fn create_thread(&mut self, _t: GuestThread) -> Result<(), u64> {
        Err(EAGAIN)
    }
}

const EAGAIN: u64 = 35;

/// The guest's system calls (see the module docs).
#[derive(Debug)]
pub struct Bridge<H> {
    pub host: H,
    /// The window.
    window_lo: u64,
    window_end: u64,
    /// The next free page of the anonymous area and its end.
    mmap_next: u64,
    mmap_end: u64,
    /// The guest called `exit` with this status.
    pub exit: Option<i32>,
    /// The guest made an `svc` with an immediate that is not `0x80`.
    pub bad_svc: Option<u16>,
    /// System calls made.
    pub calls: u64,
    /// Keep the calls that go to the kernel for the runtime to make.
    pub defer: bool,
    /// The kept call.
    pub pending: Option<Pending>,
    /// A guest thread ended (bsdthread_terminate): its stack and size.
    pub terminated: Option<(u64, u64)>,
    /// libpthread's thread entry (bsdthread_register), 0 until then.
    thread_start: u64,
    /// The images with thread-local variables.
    tlv: Vec<TlvImage>,
}

fn page_up(x: u64) -> Option<u64> {
    Some(x.checked_add(PAGE_SIZE - 1)? & !(PAGE_SIZE - 1))
}

impl<H: Host> Bridge<H> {
    pub fn new(host: H, h: &Handoff) -> Bridge<H> {
        Bridge {
            host,
            window_lo: h.window_lo,
            window_end: h.window_end,
            mmap_next: h.mmap_lo,
            mmap_end: h.mmap_end,
            exit: None,
            bad_svc: None,
            calls: 0,
            defer: false,
            pending: None,
            terminated: None,
            thread_start: 0,
            tlv: h.tlv.clone(),
        }
    }

    /// Hand the result of the kept call back to the guest thread `cpu`.
    pub fn complete(cpu: &mut Cpu, p: &Pending) {
        arm64::from_frame(&p.frame, p.x16, &mut cpu.x, &mut cpu.nzcv);
    }

    /// Anonymous guest memory, readable and writable, as the guest's mmap
    /// would map it.
    fn map_anon(&mut self, len: u64, mem: &mut GuestMem) -> Result<u64, u64> {
        let mut a = [0u64; 31];
        a[..5].copy_from_slice(&[0, len, u64::from(R | W), MAP_ANON | 2, u64::MAX]);
        self.mmap(&a, mem)
    }

    /// bsdthread_create(func, arg, stack, pthread, flags): the checks of the
    /// sheet (as the kernel's), the thread's table, then the host thread.
    fn create_thread(&mut self, a: &[u64; 31], mem: &mut GuestMem) -> Result<u64, u64> {
        let (func, arg, stack, pthread, flags) = (a[0], a[1], a[2], a[3], a[4]);
        if self.thread_start == 0 || func == 0 || flags & PTHREAD_START_CUSTOM == 0 || stack == 0 || pthread == 0 {
            return Err(EINVAL);
        }
        // The table (16 KiB) and one block per image after it.
        let blocks: Vec<u64> = self.tlv.iter().map(|t| page_up(t.size.max(1)).unwrap_or(PAGE_SIZE)).collect();
        let total = PAGE_SIZE + blocks.iter().sum::<u64>();
        let table = self.map_anon(total, mem)?;
        let put = |mem: &mut GuestMem, at: u64, v: u64| mem.poke(at, &v.to_le_bytes()).map_err(|_| ENOMEM);
        put(mem, table, pthread)?;
        let mut at = table + PAGE_SIZE;
        for (t, len) in self.tlv.clone().iter().zip(&blocks) {
            put(mem, table + 8 * t.slot, at)?;
            let mut init = alloc::vec![0u8; t.init_size as usize];
            mem.peek(t.template, &mut init).map_err(|_| EINVAL)?;
            mem.poke(at, &init).map_err(|_| ENOMEM)?;
            at += len;
        }
        let t = GuestThread { pc: self.thread_start, sp: stack, x: [pthread, func, arg], tpidrro: table };
        self.host.create_thread(t)?;
        Ok(pthread)
    }

    fn in_window(&self, addr: u64, len: u64) -> bool {
        addr >= self.window_lo && addr.checked_add(len).is_some_and(|e| e <= self.window_end)
    }

    /// One BSD call to the host: `Ok(rax)` or `Err(errno)`.
    fn bsd(&mut self, number: u64, args: [u64; 6], mem: &mut GuestMem) -> Result<u64, u64> {
        let [rdi, rsi, rdx, r10, r8, r9] = args;
        let mut f = SyscallRegs { rax: BSD | number, rdi, rsi, rdx, r10, r8, r9, rflags: 0, rsp: 0 };
        self.host.syscall(&mut f, [0, 0], mem);
        if f.rflags & RFLAGS_CF != 0 { Err(f.rax) } else { Ok(f.rax) }
    }

    fn mmap(&mut self, a: &[u64; 31], mem: &mut GuestMem) -> Result<u64, u64> {
        let (addr, len, prot, flags, fd, offset) = (a[0], a[1], a[2], a[3], a[4], a[5]);
        let anon = flags & MAP_ANON != 0;
        if len == 0 || (len as i64) < 0 {
            // The kernel's answers: 0, or the descriptor's and length's errors.
            return self.bsd(sys::MMAP, [addr, len, prot, flags, fd, offset], mem);
        }
        let skew = if anon { 0 } else { offset % PAGE_SIZE };
        let total = len.checked_add(skew).and_then(page_up).filter(|&t| t <= 1 << 46).ok_or(ENOMEM)?;
        let at = if flags & MAP_FIXED != 0 {
            if !addr.is_multiple_of(PAGE_SIZE) {
                return Err(EINVAL);
            }
            if !self.in_window(addr, total) {
                return Err(ENOMEM);
            }
            addr
        } else {
            let at = self.mmap_next;
            if at.checked_add(total).is_none_or(|e| e > self.mmap_end) {
                return Err(ENOMEM);
            }
            at
        };
        let jit = flags & u64::from(MAP_JIT) != 0;
        let host_prot = if jit { u64::from(R | W) } else { prot & u64::from(R | W) };
        let host_flags = (flags & !u64::from(MAP_JIT)) | MAP_FIXED;
        let got = self.bsd(sys::MMAP, [at, total, host_prot, host_flags, fd, offset - skew], mem)?;
        if got != at {
            return Err(ENOMEM);
        }
        if flags & MAP_FIXED == 0 {
            self.mmap_next = at + total;
        }
        let guest_prot = (prot as u8) & (R | W | X);
        let marked = if jit {
            mem.map(at, total, guest_prot, MAP_JIT)
        } else {
            // The kernel's fresh mapping holds zeros or the file.
            mem.set_mapped(at, total, guest_prot)
        };
        marked.map_err(|_| ENOMEM)?;
        Ok(at + skew)
    }

    fn munmap(&mut self, a: &[u64; 31], mem: &mut GuestMem) -> Result<u64, u64> {
        let (addr, len) = (a[0], a[1]);
        if !addr.is_multiple_of(PAGE_SIZE) || len == 0 || (len as i64) < 0 {
            return Err(EINVAL);
        }
        let len = page_up(len).ok_or(EINVAL)?;
        if !self.in_window(addr, len) {
            return Err(EINVAL);
        }
        self.bsd(sys::MUNMAP, [addr, len, 0, 0, 0, 0], mem)?;
        mem.unmap(addr, len).map_err(|_| EINVAL)?;
        Ok(0)
    }

    fn mprotect(&mut self, a: &[u64; 31], mem: &mut GuestMem) -> Result<u64, u64> {
        let (addr, len, prot) = (a[0], a[1], a[2]);
        if !addr.is_multiple_of(PAGE_SIZE) {
            return Err(EINVAL);
        }
        let len = page_up(len).ok_or(EINVAL)?;
        if !self.in_window(addr, len) {
            return Err(ENOMEM);
        }
        // The kernel checks the mapping (a shared file opened read-only
        // cannot become writable); JIT pages keep their host protection.
        let jit = mem.page_flags(addr) & poly_a64_jit::mem::JIT != 0;
        if !jit {
            self.bsd(sys::MPROTECT, [addr, len, prot & u64::from(R | W), 0, 0, 0], mem)?;
        }
        mem.protect(addr, len, (prot as u8) & (R | W | X)).map_err(|_| ENOMEM)?;
        Ok(0)
    }

    /// End the process with `signal`, as the kernel would end a native
    /// program that faulted: `kill(getpid(), signal)`. Returns only if the
    /// signal did not end it (the guest catches or ignores it: running the
    /// guest's handler in translated code comes with signal frames).
    pub fn kill_self(&mut self, signal: u32, mem: &mut GuestMem) {
        if let Ok(pid) = self.bsd(sys::GETPID, [0; 6], mem) {
            let _ = self.bsd(sys::KILL, [pid, u64::from(signal), 0, 0, 0, 0], mem);
        }
    }
}

impl<H: Host> Kernel for Bridge<H> {
    fn svc(&mut self, imm: u16, cpu: &mut Cpu, mem: &mut GuestMem) -> Option<i32> {
        self.calls += 1;
        if imm != SVC_IMM {
            self.bad_svc = Some(imm);
            return Some(-1);
        }
        let x16 = cpu.x[16];
        let local = match arm64::decode(x16) {
            (Class::Bsd, sys::EXIT) => {
                let status = cpu.x[0] as i32;
                self.exit = Some(status);
                return Some(status);
            }
            (Class::Bsd, sys::BSDTHREAD_REGISTER) => Some(if self.thread_start != 0 {
                Err(EINVAL)
            } else {
                self.thread_start = cpu.x[0].max(1);
                Ok(0)
            }),
            (Class::Bsd, sys::BSDTHREAD_CREATE) => Some(self.create_thread(&cpu.x, mem)),
            (Class::Bsd, sys::BSDTHREAD_TERMINATE) => {
                self.terminated = Some((cpu.x[0], cpu.x[1]));
                return Some(YIELD);
            }
            (Class::Bsd, sys::MMAP) => Some(self.mmap(&cpu.x, mem)),
            (Class::Bsd, sys::MUNMAP) => Some(self.munmap(&cpu.x, mem)),
            (Class::Bsd, sys::MPROTECT) => Some(self.mprotect(&cpu.x, mem)),
            _ => None,
        };
        let mut frame = arm64::to_frame(&cpu.x, cpu.sp);
        match local {
            Some(Ok(v)) => frame.rax = v,
            Some(Err(e)) => {
                frame.rax = e;
                frame.rflags |= RFLAGS_CF;
            }
            None if self.defer => {
                self.pending = Some(Pending { frame, stack: [cpu.x[6], cpu.x[7]], x16 });
                return Some(YIELD);
            }
            None => self.host.syscall(&mut frame, [cpu.x[6], cpu.x[7]], mem),
        }
        arm64::from_frame(&frame, x16, &mut cpu.x, &mut cpu.nzcv);
        None
    }
}

/// The guest's memory and main thread from a hand-off: `mem` is the window
/// (`handoff.window_lo..window_end`), its regions are marked with their
/// protections.
pub fn guest(h: &Handoff, mem: &mut GuestMem) -> Result<Cpu, String> {
    for r in &h.regions {
        mem.set_mapped(r.addr, r.len, r.prot)
            .map_err(|e| format!("region {:#x}+{:#x} does not fit the window: {e:?}", r.addr, r.len))?;
    }
    let mut cpu = Cpu::new();
    cpu.pc = h.pc;
    cpu.sp = h.sp;
    cpu.tpidrro_el0 = h.tpidrro;
    Ok(cpu)
}

/// Run the guest until it stops for something other than its budget.
pub fn run<H: Host>(engine: &mut Engine, cpu: &mut Cpu, mem: &mut GuestMem, bridge: &mut Bridge<H>) -> Stop {
    loop {
        match engine.run(cpu, mem, bridge, 1 << 20) {
            Stop::Budget => continue,
            stop => return stop,
        }
    }
}

/// The message for a guest that stopped other than by `exit`.
pub fn stop_message(path: &str, stop: Stop, cpu: &Cpu, bad_svc: Option<u16>) -> String {
    let what = match (stop, bad_svc) {
        (_, Some(imm)) => format!("svc #{imm:#x} (system calls use svc #0x80)"),
        (Stop::Fault(f), _) => format!("a {:?} fault at {:#x}", f.access, f.addr),
        (Stop::Brk(i), _) => format!("brk #{i:#x}"),
        (Stop::Undefined(w), _) => format!("an undefined instruction {w:#010x}"),
        (Stop::Unsupported(w), _) => format!("an instruction the translator does not have yet {w:#010x}"),
        (Stop::Exit(c), _) => format!("exit {c}"),
        (Stop::Budget, _) => String::from("the budget"),
    };
    format!("poly-a64: {path}: stopped at pc {:#x}: {what}\n", cpu.pc)
}

/// The signal a guest that stopped other than by `exit` ends with: SIGSEGV
/// for an access to an unmapped page, SIGBUS for one the protection
/// forbids (`poly_personality_xnu::kernel::fault_signal`), SIGTRAP for
/// `brk`, SIGILL for an undefined or untranslated instruction, SIGSYS for
/// an `svc` that is not `#0x80`.
pub fn stop_signal(stop: Stop, mem: &GuestMem, bad_svc: Option<u16>) -> u32 {
    match (stop, bad_svc) {
        (_, Some(_)) => sig::SIGSYS,
        (Stop::Fault(f), _) => poly_personality_xnu::kernel::fault_signal(mem.page_flags(f.addr) != 0),
        (Stop::Brk(_), _) => sig::SIGTRAP,
        _ => sig::SIGILL,
    }
}

/// The statistics line: the engine, its counters, the system calls and the
/// time taken (`cycles` of the host's time stamp counter).
pub fn stats_line(kind: EngineKind, engine: &Engine, calls: u64, cycles: u64) -> String {
    let s = engine.stats;
    let name = match kind {
        EngineKind::Jit if engine.translates() => "jit",
        EngineKind::Jit => "jit (no code area: interpreter)",
        EngineKind::Interpreter => "interpreter",
    };
    format!(
        "poly-a64: {name}: {} blocks translated ({} instructions), {} entries, {} chained, {} interpreted, \
         {} system calls, {} cycles\n",
        s.translated, s.translated_insts, s.entries, s.chained, s.interpreted, calls, cycles
    )
}

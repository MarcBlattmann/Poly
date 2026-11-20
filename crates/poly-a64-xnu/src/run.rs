//! The runtime's side of a translated process: the guest from a hand-off,
//! the run loop, and the guest's system calls.
//!
//! [`Bridge`] is the translator's [`Kernel`]: a guest's `svc #0x80` comes
//! here with the call in `x16`. Calls go to the XNU personality through
//! [`Host::syscall`] in the x86-64 convention (`arm64::to_frame`, then
//! `arm64::from_frame` for `x0` and the carry flag), so the personality has
//! one copy of every call and does not know the caller was translated.
//! Guest addresses are host addresses, so pointer arguments need no change.
//!
//! Kept here are the calls that change the guest's memory, because the
//! window's page flags must follow them: `mmap`, `munmap` and `mprotect`
//! work on the window alone, `mmap` handing out pages of the anonymous area
//! the kernel mapped at exec. TODO(#390): once the XNU BSD layer (PR #390,
//! ADR 0246) has `mmap`, `munmap` and `mprotect` in the kernel, forward them
//! (with addresses inside the window) and update the flags from the result;
//! file mappings come with that. `exit` is reported to the runtime, which
//! prints its statistics and then makes the call.

use alloc::format;
use alloc::string::String;

use poly_a64::Cpu;
use poly_a64_jit::jit::{Engine, Kernel, Stop};
use poly_a64_jit::mem::{GuestMem, MAP_JIT, PAGE_SIZE, R, W, X};
use poly_abi::{RFLAGS_CF, SyscallRegs};
use poly_personality_xnu::Class;
use poly_personality_xnu::arm64::{self, SVC_IMM};

use crate::handoff::{EngineKind, Handoff};

/// BSD call numbers the bridge keeps (`sys/syscall.h`).
pub mod sys {
    pub const EXIT: u64 = 1;
    pub const MUNMAP: u64 = 73;
    pub const MPROTECT: u64 = 74;
    pub const MMAP: u64 = 197;
}

/// `sys/mman.h`.
const MAP_FIXED: u64 = 0x10;
const MAP_ANON: u64 = 0x1000;
/// Darwin errno values (`sys/errno.h`).
const EBADF: u64 = 9;
const ENOMEM: u64 = 12;
const EINVAL: u64 = 22;
const ENOSYS: u64 = 78;

/// Where the runtime sends the guest's system calls.
pub trait Host {
    /// Make one x86-64 XNU system call: `frame` holds the class and number
    /// in `rax` and the first six arguments; `stack` the seventh and
    /// eighth. Afterwards `frame.rax` is the result and `RFLAGS_CF` in
    /// `frame.rflags` the carry flag. `mem` is the guest's memory, for a
    /// host that cannot read guest addresses directly (tests).
    fn syscall(&mut self, frame: &mut SyscallRegs, stack: [u64; 2], mem: &mut GuestMem);
}

/// The guest's system calls (see the module docs).
#[derive(Debug)]
pub struct Bridge<H> {
    pub host: H,
    /// The next free page of the anonymous area and its end.
    mmap_next: u64,
    mmap_lo: u64,
    mmap_end: u64,
    /// The guest called `exit` with this status.
    pub exit: Option<i32>,
    /// The guest made an `svc` with an immediate that is not `0x80`.
    pub bad_svc: Option<u16>,
    /// System calls made.
    pub calls: u64,
}

impl<H: Host> Bridge<H> {
    pub fn new(host: H, h: &Handoff) -> Bridge<H> {
        Bridge {
            host,
            mmap_next: h.mmap_lo,
            mmap_lo: h.mmap_lo,
            mmap_end: h.mmap_end,
            exit: None,
            bad_svc: None,
            calls: 0,
        }
    }

    /// `mmap` of anonymous memory in the window.
    fn mmap(&mut self, a: &[u64; 31], mem: &mut GuestMem) -> Result<u64, u64> {
        let (addr, len, prot, flags, fd) = (a[0], a[1], a[2], a[3], a[4] as i32);
        if len == 0 {
            return Ok(0);
        }
        if flags & MAP_ANON == 0 {
            // TODO(#390): file mappings go to the kernel's mmap.
            return Err(if fd == -1 { EBADF } else { ENOSYS });
        }
        if len > 1 << 46 {
            return Err(ENOMEM);
        }
        let len = (len + PAGE_SIZE - 1) & !(PAGE_SIZE - 1);
        let at = if flags & MAP_FIXED != 0 {
            if !addr.is_multiple_of(PAGE_SIZE) {
                return Err(EINVAL);
            }
            // Only the anonymous area has host pages behind it.
            if addr < self.mmap_lo || addr.checked_add(len).is_none_or(|e| e > self.mmap_end) {
                return Err(ENOMEM);
            }
            addr
        } else {
            let at = self.mmap_next;
            if at.checked_add(len).is_none_or(|e| e > self.mmap_end) {
                return Err(ENOMEM);
            }
            self.mmap_next = at + len;
            at
        };
        let guest_prot = (prot as u8) & (R | W | X);
        let done = if flags & MAP_FIXED != 0 || flags & u64::from(MAP_JIT) != 0 {
            // Replaced memory reads as zero again; JIT pages follow the
            // thread's write-protect mode.
            mem.map(at, len, guest_prot, (flags as u32) & MAP_JIT)
        } else {
            // A fresh part of the area: the kernel mapped it zero-filled.
            mem.set_mapped(at, len, guest_prot)
        };
        done.map_err(|_| ENOMEM)?;
        Ok(at)
    }

    fn munmap(&mut self, a: &[u64; 31], mem: &mut GuestMem) -> Result<u64, u64> {
        let (addr, len) = (a[0], a[1]);
        if !addr.is_multiple_of(PAGE_SIZE) || len == 0 {
            return Err(EINVAL);
        }
        let len = (len + PAGE_SIZE - 1) & !(PAGE_SIZE - 1);
        mem.unmap(addr, len).map_err(|_| EINVAL)?;
        Ok(0)
    }

    fn mprotect(&mut self, a: &[u64; 31], mem: &mut GuestMem) -> Result<u64, u64> {
        let (addr, len, prot) = (a[0], a[1], a[2]);
        if !addr.is_multiple_of(PAGE_SIZE) {
            return Err(EINVAL);
        }
        let len = (len + PAGE_SIZE - 1) & !(PAGE_SIZE - 1);
        mem.protect(addr, len, (prot as u8) & (R | W | X)).map_err(|_| ENOMEM)?;
        Ok(0)
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

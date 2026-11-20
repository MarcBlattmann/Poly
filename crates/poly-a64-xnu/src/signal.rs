//! Signals of a translated process (ADR 0257 addendum): the guest's
//! actions, its blocked and pending sets, and the arm64 frames a handler
//! runs with.
//!
//! The kernel keeps a process's actions (`sigaction`, the sigaction
//! sheet) but does not run handlers yet, and a translated thread's
//! registers are the runtime's anyway, so the runtime delivers: the
//! signals a fault, a breakpoint or an undefined instruction raises, and
//! those the process sends itself (`kill`, `raise`). A handler is entered
//! as the sheet describes: through the trampoline its `sigaction` named,
//! with (handler, info style, signal, siginfo, ucontext) in x0..x4, on the
//! interrupted stack below a frame holding the `siginfo_t`, the
//! `ucontext_t` and its arm64 `mcontext` (`<sys/signal.h>`,
//! `<sys/_types/_ucontext.h>`, `<mach/arm/_structs.h>`); the trampoline
//! calls the handler and then `sigreturn(ucontext, info style)`, which
//! restores every register from the frame. The blocked set is the
//! process's (one for every guest thread, a v0 limit).

use poly_a64::Cpu;
use poly_a64_jit::mem::GuestMem;

/// `SA_RESETHAND`, `SA_NODEFER`, `SA_SIGINFO` (`<sys/signal.h>`).
pub const SA_RESETHAND: u32 = 0x4;
pub const SA_NODEFER: u32 = 0x10;
pub const SA_SIGINFO: u32 = 0x40;
/// `SIG_BLOCK`, `SIG_UNBLOCK`, `SIG_SETMASK`.
pub const SIG_BLOCK: u64 = 1;
pub const SIG_UNBLOCK: u64 = 2;
pub const SIG_SETMASK: u64 = 3;
/// SIGKILL and SIGSTOP: never caught, ignored or blocked.
const SIGKILL: u32 = 9;
const SIGSTOP: u32 = 17;
/// The info styles Poly's trampoline hands back to `sigreturn` (its own
/// values: only Poly's trampoline and runtime see them).
pub const STYLE_PLAIN: u64 = 1;
pub const STYLE_SIGINFO: u64 = 30;

/// The sizes and offsets of the frame (checked against the SDK headers).
pub const SIGINFO_SIZE: u64 = 104;
pub const UCONTEXT_SIZE: u64 = 56;
pub const MCONTEXT_SIZE: u64 = 816;
const MC_SS: u64 = 16;
const MC_NS: u64 = 288;
/// Below the interrupted stack pointer: the 128 bytes Apple's arm64 ABI
/// lets a leaf function use without moving it.
const RED_ZONE: u64 = 128;

/// A signal's action as the guest set it (the kernel's `__sigaction`).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Action {
    pub handler: u64,
    pub tramp: u64,
    pub mask: u32,
    pub flags: u32,
}

impl Action {
    /// A handler, not `SIG_DFL` (0) or `SIG_IGN` (1).
    pub fn catches(&self) -> bool {
        self.handler > 1
    }
}

/// The process's signal state.
#[derive(Clone, Debug, Default)]
pub struct Signals {
    actions: [Action; 32],
    pub blocked: u32,
    pub pending: u32,
}

fn bit(sig: u32) -> u32 {
    1 << (sig - 1)
}

impl Signals {
    /// `sigaction`: the old action; `EINVAL` for a bad signal, or for
    /// setting SIGKILL or SIGSTOP.
    pub fn sigaction(&mut self, sig: u32, new: Option<Action>) -> Result<Action, u64> {
        if !(1..32).contains(&sig) || (new.is_some() && (sig == SIGKILL || sig == SIGSTOP)) {
            return Err(22);
        }
        let old = self.actions[sig as usize];
        if let Some(a) = new {
            self.actions[sig as usize] = a;
        }
        Ok(old)
    }

    pub fn action(&self, sig: u32) -> Action {
        self.actions.get(sig as usize).copied().unwrap_or_default()
    }

    /// `sigprocmask`: the old mask (SIGKILL and SIGSTOP stay unblocked).
    pub fn procmask(&mut self, how: u64, set: Option<u32>) -> Result<u32, u64> {
        let old = self.blocked;
        if let Some(s) = set {
            self.blocked = match how {
                SIG_BLOCK => old | s,
                SIG_UNBLOCK => old & !s,
                SIG_SETMASK => s,
                _ => return Err(22),
            } & !(bit(SIGKILL) | bit(SIGSTOP));
        }
        Ok(old)
    }

    /// A pending signal that is no longer blocked, taken off the set.
    pub fn take_deliverable(&mut self) -> Option<u32> {
        let ready = self.pending & !self.blocked;
        if ready == 0 {
            return None;
        }
        let sig = ready.trailing_zeros() + 1;
        self.pending &= !bit(sig);
        Some(sig)
    }

    pub fn is_blocked(&self, sig: u32) -> bool {
        self.blocked & bit(sig) != 0
    }

    pub fn set_pending(&mut self, sig: u32) {
        self.pending |= bit(sig);
    }
}

/// A signal frame could not be written or read: the guest's stack is not
/// mapped there.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct FrameError;

/// What a handler is told about the signal (`siginfo_t`).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Info {
    pub code: i32,
    pub pid: i32,
    pub addr: u64,
}

fn put(mem: &mut GuestMem, at: u64, bytes: &[u8]) -> Result<(), FrameError> {
    mem.poke(at, bytes).map_err(|_| FrameError)
}

fn get(mem: &GuestMem, at: u64, n: usize) -> Result<[u8; 16], FrameError> {
    let mut b = [0u8; 16];
    mem.peek(at, &mut b[..n]).map_err(|_| FrameError)?;
    Ok(b)
}

/// Enter `act`'s handler for `sig` on the thread `cpu`: the frame below
/// its stack, the registers for the trampoline, the blocked set while the
/// handler runs. `Err` if the frame cannot be written (the process then
/// ends with the signal).
pub fn deliver(cpu: &mut Cpu, mem: &mut GuestMem, s: &mut Signals, sig: u32, info: Info) -> Result<(), FrameError> {
    let act = s.action(sig);
    let frame_size = (SIGINFO_SIZE + UCONTEXT_SIZE + MCONTEXT_SIZE + 15) & !15;
    let sp = (cpu.sp.wrapping_sub(RED_ZONE).wrapping_sub(frame_size)) & !15;
    let siginfo = sp;
    let uc = sp + SIGINFO_SIZE;
    let mc = (uc + UCONTEXT_SIZE + 15) & !15;
    if mc + MCONTEXT_SIZE > sp + frame_size + 16 {
        return Err(FrameError);
    }
    // siginfo: signo, errno, code, pid, uid, status, addr.
    let mut si = [0u8; SIGINFO_SIZE as usize];
    si[0..4].copy_from_slice(&(sig as i32).to_le_bytes());
    si[8..12].copy_from_slice(&info.code.to_le_bytes());
    si[12..16].copy_from_slice(&info.pid.to_le_bytes());
    si[24..32].copy_from_slice(&info.addr.to_le_bytes());
    put(mem, siginfo, &si)?;
    // ucontext: the mask to restore, no alternate stack, the mcontext.
    let mut u = [0u8; UCONTEXT_SIZE as usize];
    u[4..8].copy_from_slice(&s.blocked.to_le_bytes());
    u[40..48].copy_from_slice(&MCONTEXT_SIZE.to_le_bytes());
    u[48..56].copy_from_slice(&mc.to_le_bytes());
    put(mem, uc, &u)?;
    // mcontext: the exception state (the fault address), the registers,
    // the vector registers.
    let mut m = alloc::vec![0u8; MCONTEXT_SIZE as usize];
    m[0..8].copy_from_slice(&info.addr.to_le_bytes());
    let ss = MC_SS as usize;
    for i in 0..29 {
        m[ss + 8 * i..ss + 8 * i + 8].copy_from_slice(&cpu.x[i].to_le_bytes());
    }
    m[ss + 232..ss + 240].copy_from_slice(&cpu.x[29].to_le_bytes());
    m[ss + 240..ss + 248].copy_from_slice(&cpu.x[30].to_le_bytes());
    m[ss + 248..ss + 256].copy_from_slice(&cpu.sp.to_le_bytes());
    m[ss + 256..ss + 264].copy_from_slice(&cpu.pc.to_le_bytes());
    m[ss + 264..ss + 268].copy_from_slice(&cpu.nzcv.to_le_bytes());
    let ns = MC_NS as usize;
    for i in 0..32 {
        m[ns + 16 * i..ns + 16 * i + 16].copy_from_slice(&cpu.v[i].to_le_bytes());
    }
    m[ns + 512..ns + 516].copy_from_slice(&cpu.fpsr.to_le_bytes());
    m[ns + 516..ns + 520].copy_from_slice(&cpu.fpcr.to_le_bytes());
    put(mem, mc, &m)?;
    // The handler runs with its own signal (unless SA_NODEFER) and the
    // action's mask blocked; SA_RESETHAND makes the action the default.
    let mut add = act.mask;
    if act.flags & SA_NODEFER == 0 {
        add |= bit(sig);
    }
    s.blocked |= add & !(bit(SIGKILL) | bit(SIGSTOP));
    if act.flags & SA_RESETHAND != 0 {
        let _ = s.sigaction(sig, Some(Action::default()));
    }
    cpu.x[0] = act.handler;
    cpu.x[1] = if act.flags & SA_SIGINFO != 0 { STYLE_SIGINFO } else { STYLE_PLAIN };
    cpu.x[2] = u64::from(sig);
    cpu.x[3] = siginfo;
    cpu.x[4] = uc;
    cpu.x[30] = 0;
    cpu.sp = sp;
    cpu.pc = act.tramp;
    Ok(())
}

/// `sigreturn(ucontext, style)`: every register and the blocked set from
/// the frame.
pub fn sigreturn(cpu: &mut Cpu, mem: &GuestMem, s: &mut Signals, uc: u64) -> Result<(), FrameError> {
    let word = |at: u64| -> Result<u64, FrameError> {
        Ok(u64::from_le_bytes(get(mem, at, 8)?[..8].try_into().map_err(|_| FrameError)?))
    };
    let mask = u32::from_le_bytes(get(mem, uc + 4, 4)?[..4].try_into().map_err(|_| FrameError)?);
    let mc = word(uc + 48)?;
    let ss = mc + MC_SS;
    for i in 0..29 {
        cpu.x[i] = word(ss + 8 * i as u64)?;
    }
    cpu.x[29] = word(ss + 232)?;
    cpu.x[30] = word(ss + 240)?;
    cpu.sp = word(ss + 248)?;
    cpu.pc = word(ss + 256)?;
    cpu.nzcv = u32::from_le_bytes(get(mem, ss + 264, 4)?[..4].try_into().map_err(|_| FrameError)?) & 0xf000_0000;
    let ns = mc + MC_NS;
    for i in 0..32 {
        cpu.v[i] = u128::from_le_bytes(get(mem, ns + 16 * i as u64, 16)?);
    }
    cpu.fpsr = u32::from_le_bytes(get(mem, ns + 512, 4)?[..4].try_into().map_err(|_| FrameError)?);
    cpu.fpcr = u32::from_le_bytes(get(mem, ns + 516, 4)?[..4].try_into().map_err(|_| FrameError)?);
    s.blocked = mask & !(bit(SIGKILL) | bit(SIGSTOP));
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use poly_a64_jit::mem::{PAGE_SIZE, R, W};

    #[test]
    fn a_frame_round_trips() {
        let lo = 0x1_0000_0000;
        let mut mem = GuestMem::new(lo, 16 * PAGE_SIZE);
        mem.map(lo + 8 * PAGE_SIZE, 4 * PAGE_SIZE, R | W, 0).unwrap();
        let mut cpu = Cpu::new();
        for i in 0..31 {
            cpu.x[i] = 0x1000 + i as u64;
        }
        cpu.sp = lo + 12 * PAGE_SIZE - 64;
        cpu.pc = 0x1_0000_4444;
        cpu.nzcv = 0x6000_0000;
        cpu.v[7] = 0x1234_5678_9abc_def0_0fed_cba9_8765_4321;
        cpu.fpsr = 0x10;
        let before = cpu.clone();
        let mut s = Signals::default();
        s.sigaction(30, Some(Action { handler: 0x5000, tramp: 0x6000, mask: 1 << 1, flags: SA_SIGINFO })).unwrap();
        deliver(&mut cpu, &mut mem, &mut s, 30, Info { code: 0, pid: 7, addr: 0 }).unwrap();
        assert_eq!((cpu.pc, cpu.x[0], cpu.x[1], cpu.x[2]), (0x6000, 0x5000, STYLE_SIGINFO, 30));
        assert!(cpu.sp < before.sp - RED_ZONE && cpu.sp.is_multiple_of(16));
        // The signal and the action's mask are blocked inside the handler.
        assert_eq!(s.blocked, 1 << 29 | 1 << 1);
        // siginfo: signo 30, pid 7.
        let mut si = [0u8; 16];
        mem.peek(cpu.x[3], &mut si).unwrap();
        assert_eq!((si[0], si[12]), (30, 7));
        // The handler clobbers registers; sigreturn brings them back.
        let uc = cpu.x[4];
        cpu.x = [0xdead; 31];
        cpu.v = [0; 32];
        sigreturn(&mut cpu, &mem, &mut s, uc).unwrap();
        assert_eq!(cpu.x, before.x);
        assert_eq!(
            (cpu.sp, cpu.pc, cpu.nzcv, cpu.v[7], cpu.fpsr),
            (before.sp, before.pc, before.nzcv, before.v[7], 0x10)
        );
        assert_eq!(s.blocked, 0);
    }

    #[test]
    fn actions_masks_and_pending() {
        let mut s = Signals::default();
        assert_eq!(s.sigaction(9, Some(Action::default())), Err(22));
        assert_eq!(s.sigaction(0, None), Err(22));
        assert!(s.sigaction(9, None).is_ok());
        let a = Action { handler: 0x10, tramp: 0x20, mask: 0, flags: SA_RESETHAND };
        assert_eq!(s.sigaction(2, Some(a)), Ok(Action::default()));
        assert!(s.action(2).catches());
        assert_eq!(s.procmask(SIG_BLOCK, Some(bit(2) | bit(9))), Ok(0));
        assert_eq!(s.blocked, bit(2));
        s.set_pending(2);
        assert_eq!(s.take_deliverable(), None);
        assert_eq!(s.procmask(SIG_UNBLOCK, Some(bit(2))), Ok(bit(2)));
        assert_eq!(s.take_deliverable(), Some(2));
        assert_eq!(s.take_deliverable(), None);
        assert_eq!(s.procmask(7, Some(0)), Err(22));
    }
}

//! The x64 `CONTEXT` a thread's registers are read into and written from
//! (`NtGetContextThread` / `NtSetContextThread`).
//!
//! [`NtContext`] is the registers Poly fills in: the control registers
//! (`Rip`, `Rsp`, `EFlags`, `SegCs`, `SegSs`) and the integer registers
//! (`Rax`..`R15`). The legacy floating-point / XMM save area and the debug
//! registers are left for a later milestone (floating-point context is
//! `CONTEXT_FLOATING_POINT`, not asked for by this milestone). The kernel
//! turns an [`NtContext`] to and from a thread's trap frame; this module
//! only lays it out in the `CONTEXT` buffer the user passes, by the
//! `winnt.h` offsets in [`crate::consts`].

use crate::consts::*;

/// The registers of a thread Poly reads and writes. A plain value so the
/// kernel can fill it from, and apply it to, a trap frame without knowing
/// the `CONTEXT` layout.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct NtContext {
    pub rax: u64,
    pub rcx: u64,
    pub rdx: u64,
    pub rbx: u64,
    pub rsp: u64,
    pub rbp: u64,
    pub rsi: u64,
    pub rdi: u64,
    pub r8: u64,
    pub r9: u64,
    pub r10: u64,
    pub r11: u64,
    pub r12: u64,
    pub r13: u64,
    pub r14: u64,
    pub r15: u64,
    pub rip: u64,
    pub eflags: u32,
    pub seg_cs: u16,
    pub seg_ss: u16,
}

fn put64(buf: &mut [u8], at: usize, v: u64) {
    buf[at..at + 8].copy_from_slice(&v.to_le_bytes());
}

fn get64(buf: &[u8], at: usize) -> u64 {
    u64::from_le_bytes(buf[at..at + 8].try_into().expect("8 bytes"))
}

impl NtContext {
    /// The integer registers, in the `winnt.h` order from `Rax` at
    /// [`CTX_RAX`] to `R15` at [`CTX_R15`].
    fn integer(&self) -> [(usize, u64); 15] {
        [
            (CTX_RAX, self.rax),
            (CTX_RCX, self.rcx),
            (CTX_RDX, self.rdx),
            (CTX_RBX, self.rbx),
            (CTX_RBP, self.rbp),
            (CTX_RSI, self.rsi),
            (CTX_RDI, self.rdi),
            (CTX_R8, self.r8),
            (CTX_R8 + 8, self.r9),
            (CTX_R8 + 16, self.r10),
            (CTX_R8 + 24, self.r11),
            (CTX_R8 + 32, self.r12),
            (CTX_R8 + 40, self.r13),
            (CTX_R8 + 48, self.r14),
            (CTX_R15, self.r15),
        ]
    }

    /// Lay this context out in a `CONTEXT` buffer, writing only the groups
    /// `want` selects (`CONTEXT_CONTROL`, `CONTEXT_INTEGER`) and setting
    /// `ContextFlags` to exactly those. The buffer is left otherwise as it
    /// was (the caller zeroed or pre-filled it).
    pub fn write_to(&self, buf: &mut [u8], want: u32) {
        let mut flags = CONTEXT_AMD64;
        if want & CONTEXT_CONTROL == CONTEXT_CONTROL {
            put64(buf, CTX_RSP, self.rsp);
            put64(buf, CTX_RIP, self.rip);
            buf[CTX_EFLAGS..CTX_EFLAGS + 4].copy_from_slice(&self.eflags.to_le_bytes());
            buf[CTX_SEG_CS..CTX_SEG_CS + 2].copy_from_slice(&self.seg_cs.to_le_bytes());
            buf[CTX_SEG_SS..CTX_SEG_SS + 2].copy_from_slice(&self.seg_ss.to_le_bytes());
            flags |= CONTEXT_CONTROL;
        }
        if want & CONTEXT_INTEGER == CONTEXT_INTEGER {
            for (at, v) in self.integer() {
                put64(buf, at, v);
            }
            flags |= CONTEXT_INTEGER;
        }
        buf[CTX_FLAGS..CTX_FLAGS + 4].copy_from_slice(&flags.to_le_bytes());
    }

    /// Read a context from a `CONTEXT` buffer, taking the groups `want`
    /// selects from it and leaving the rest of `self` unchanged (so an
    /// `NtSetContextThread` that sets only the control registers keeps the
    /// thread's other registers, which the caller seeds from the current
    /// context).
    pub fn read_from(&mut self, buf: &[u8], want: u32) {
        if want & CONTEXT_CONTROL == CONTEXT_CONTROL {
            self.rsp = get64(buf, CTX_RSP);
            self.rip = get64(buf, CTX_RIP);
            self.eflags = u32::from_le_bytes(buf[CTX_EFLAGS..CTX_EFLAGS + 4].try_into().expect("4 bytes"));
            self.seg_cs = u16::from_le_bytes(buf[CTX_SEG_CS..CTX_SEG_CS + 2].try_into().expect("2 bytes"));
            self.seg_ss = u16::from_le_bytes(buf[CTX_SEG_SS..CTX_SEG_SS + 2].try_into().expect("2 bytes"));
        }
        if want & CONTEXT_INTEGER == CONTEXT_INTEGER {
            self.rax = get64(buf, CTX_RAX);
            self.rcx = get64(buf, CTX_RCX);
            self.rdx = get64(buf, CTX_RDX);
            self.rbx = get64(buf, CTX_RBX);
            self.rbp = get64(buf, CTX_RBP);
            self.rsi = get64(buf, CTX_RSI);
            self.rdi = get64(buf, CTX_RDI);
            self.r8 = get64(buf, CTX_R8);
            self.r9 = get64(buf, CTX_R8 + 8);
            self.r10 = get64(buf, CTX_R8 + 16);
            self.r11 = get64(buf, CTX_R8 + 24);
            self.r12 = get64(buf, CTX_R8 + 32);
            self.r13 = get64(buf, CTX_R8 + 40);
            self.r14 = get64(buf, CTX_R8 + 48);
            self.r15 = get64(buf, CTX_R15);
        }
    }
}

#[cfg(test)]
mod tests {
    extern crate std;
    use super::*;
    use alloc::vec;

    #[test]
    fn round_trips_control_and_integer() {
        let ctx = NtContext {
            rax: 0x1111,
            rcx: 0x2222,
            r15: 0xF00F,
            rip: 0x1_4000_1234,
            rsp: 0x5_0000,
            eflags: 0x202,
            seg_cs: 0x33,
            seg_ss: 0x2b,
            ..Default::default()
        };
        let mut buf = vec![0u8; CONTEXT_SIZE];
        ctx.write_to(&mut buf, CONTEXT_CONTROL | CONTEXT_INTEGER);
        // ContextFlags records both groups plus the architecture bits.
        let flags = u32::from_le_bytes(buf[CTX_FLAGS..CTX_FLAGS + 4].try_into().unwrap());
        assert_eq!(flags, CONTEXT_CONTROL | CONTEXT_INTEGER);
        let mut back = NtContext::default();
        back.read_from(&buf, CONTEXT_CONTROL | CONTEXT_INTEGER);
        assert_eq!(back, ctx);
    }

    #[test]
    fn control_only_keeps_integer_registers() {
        let mut buf = vec![0u8; CONTEXT_SIZE];
        let ctx = NtContext { rip: 0xABCD, rsp: 0x1000, rax: 0x9999, ..Default::default() };
        ctx.write_to(&mut buf, CONTEXT_CONTROL);
        // Rax was not written: still zero in the buffer.
        assert_eq!(get64(&buf, CTX_RAX), 0);
        assert_eq!(get64(&buf, CTX_RIP), 0xABCD);
        // Reading control only into a seeded context keeps its integers.
        let mut seed = NtContext { rax: 0x7, ..Default::default() };
        seed.read_from(&buf, CONTEXT_CONTROL);
        assert_eq!(seed.rax, 0x7);
        assert_eq!(seed.rip, 0xABCD);
    }
}

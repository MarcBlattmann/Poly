//! The x64 `CONTEXT` (`winnt.h`; Learn: "CONTEXT structure (winnt.h)") as
//! a plain value, and its byte layout.
//!
//! The integer registers sit in `CONTEXT` in the order of their register
//! numbers (`Rax` at 0x78 is register 0, ..., `R15` at 0xF0 is 15), so
//! [`Context::gpr`] is indexed by the numbers unwind codes use. The legacy
//! floating-point save area `FltSave` is an `XMM_SAVE_AREA32`, the 512-byte
//! `FXSAVE` image (`winnt.h`), with `Xmm0`..`Xmm15` at its offset 160.

use alloc::vec;
use alloc::vec::Vec;

/// `sizeof(CONTEXT)` on x64.
pub const CONTEXT_SIZE: usize = 0x4D0;
/// The alignment `CONTEXT` is declared with (`DECLSPEC_ALIGN(16)`).
pub const CONTEXT_ALIGN: u64 = 16;

/// `CONTEXT_AMD64`: the architecture bit every `CONTEXT_*` group carries.
pub const CONTEXT_AMD64: u32 = 0x0010_0000;
/// `SegSs`, `Rsp`, `SegCs`, `Rip`, `EFlags`.
pub const CONTEXT_CONTROL: u32 = CONTEXT_AMD64 | 0x01;
/// `Rax`..`R15` (`Rsp` belongs to the control group).
pub const CONTEXT_INTEGER: u32 = CONTEXT_AMD64 | 0x02;
/// `SegDs`, `SegEs`, `SegFs`, `SegGs`.
pub const CONTEXT_SEGMENTS: u32 = CONTEXT_AMD64 | 0x04;
/// `MxCsr` and `FltSave` (x87, `Xmm0`..`Xmm15`).
pub const CONTEXT_FLOATING_POINT: u32 = CONTEXT_AMD64 | 0x08;
/// `Dr0`..`Dr3`, `Dr6`, `Dr7`.
pub const CONTEXT_DEBUG_REGISTERS: u32 = CONTEXT_AMD64 | 0x10;
pub const CONTEXT_FULL: u32 = CONTEXT_CONTROL | CONTEXT_INTEGER | CONTEXT_FLOATING_POINT;
pub const CONTEXT_ALL: u32 =
    CONTEXT_CONTROL | CONTEXT_INTEGER | CONTEXT_SEGMENTS | CONTEXT_FLOATING_POINT | CONTEXT_DEBUG_REGISTERS;

// Field offsets (`winnt.h`).
pub const OFF_P1_HOME: usize = 0x00;
pub const OFF_CONTEXT_FLAGS: usize = 0x30;
pub const OFF_MX_CSR: usize = 0x34;
pub const OFF_SEG_CS: usize = 0x38;
pub const OFF_SEG_DS: usize = 0x3A;
pub const OFF_SEG_ES: usize = 0x3C;
pub const OFF_SEG_FS: usize = 0x3E;
pub const OFF_SEG_GS: usize = 0x40;
pub const OFF_SEG_SS: usize = 0x42;
pub const OFF_EFLAGS: usize = 0x44;
pub const OFF_DR0: usize = 0x48;
pub const OFF_DR6: usize = 0x68;
pub const OFF_DR7: usize = 0x70;
pub const OFF_RAX: usize = 0x78;
pub const OFF_RSP: usize = 0x98;
pub const OFF_RIP: usize = 0xF8;
pub const OFF_FLT_SAVE: usize = 0x100;
/// `Xmm0` (inside `FltSave`, at its offset 160).
pub const OFF_XMM0: usize = 0x1A0;
pub const OFF_VECTOR_REGISTER: usize = 0x300;
pub const OFF_VECTOR_CONTROL: usize = 0x4A0;
pub const OFF_DEBUG_CONTROL: usize = 0x4A8;
pub const OFF_LAST_BRANCH_TO_RIP: usize = 0x4B0;
pub const OFF_LAST_BRANCH_FROM_RIP: usize = 0x4B8;
pub const OFF_LAST_EXCEPTION_TO_RIP: usize = 0x4C0;
pub const OFF_LAST_EXCEPTION_FROM_RIP: usize = 0x4C8;

/// Bytes of `FltSave` (`XMM_SAVE_AREA32`, the `FXSAVE` image).
pub const FLT_SAVE_SIZE: usize = 512;
/// `MxCsr` inside `FltSave` (`XMM_SAVE_AREA32.MxCsr`).
pub const FLT_SAVE_MX_CSR: usize = 24;
/// `XmmRegisters[0]` inside `FltSave`.
pub const FLT_SAVE_XMM0: usize = 160;
/// Bytes of `VectorRegister[26]`.
const VECTOR_REGISTER_SIZE: usize = 26 * 16;

/// A thread's registers as a `CONTEXT` holds them.
#[derive(Clone, PartialEq, Eq)]
pub struct Context {
    /// `P1Home`..`P6Home`: register parameter home addresses.
    pub p_home: [u64; 6],
    /// Which groups of fields are valid (`CONTEXT_*`).
    pub context_flags: u32,
    pub mx_csr: u32,
    pub seg_cs: u16,
    pub seg_ds: u16,
    pub seg_es: u16,
    pub seg_fs: u16,
    pub seg_gs: u16,
    pub seg_ss: u16,
    pub eflags: u32,
    /// `Dr0`, `Dr1`, `Dr2`, `Dr3`, `Dr6`, `Dr7`.
    pub dr: [u64; 6],
    /// `Rax`..`R15` by register number (`gpr[4]` is `Rsp`).
    pub gpr: [u64; 16],
    pub rip: u64,
    /// `FltSave`: the 512-byte `FXSAVE` image (x87, `MxCsr`, `Xmm0`..`Xmm15`).
    pub flt_save: [u8; FLT_SAVE_SIZE],
    /// `VectorRegister[26]`, `M128A` each.
    pub vector_register: [u8; VECTOR_REGISTER_SIZE],
    pub vector_control: u64,
    pub debug_control: u64,
    pub last_branch_to_rip: u64,
    pub last_branch_from_rip: u64,
    pub last_exception_to_rip: u64,
    pub last_exception_from_rip: u64,
}

impl Default for Context {
    fn default() -> Self {
        Context {
            p_home: [0; 6],
            context_flags: 0,
            mx_csr: 0,
            seg_cs: 0,
            seg_ds: 0,
            seg_es: 0,
            seg_fs: 0,
            seg_gs: 0,
            seg_ss: 0,
            eflags: 0,
            dr: [0; 6],
            gpr: [0; 16],
            rip: 0,
            flt_save: [0; FLT_SAVE_SIZE],
            vector_register: [0; VECTOR_REGISTER_SIZE],
            vector_control: 0,
            debug_control: 0,
            last_branch_to_rip: 0,
            last_branch_from_rip: 0,
            last_exception_to_rip: 0,
            last_exception_from_rip: 0,
        }
    }
}

impl core::fmt::Debug for Context {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.debug_struct("Context")
            .field("context_flags", &format_args!("{:#x}", self.context_flags))
            .field("rip", &format_args!("{:#x}", self.rip))
            .field("rsp", &format_args!("{:#x}", self.rsp()))
            .field("eflags", &format_args!("{:#x}", self.eflags))
            .field("gpr", &format_args!("{:x?}", self.gpr))
            .finish_non_exhaustive()
    }
}

fn get16(b: &[u8], at: usize) -> u16 {
    u16::from_le_bytes([b[at], b[at + 1]])
}

fn get32(b: &[u8], at: usize) -> u32 {
    let mut v = [0u8; 4];
    v.copy_from_slice(&b[at..at + 4]);
    u32::from_le_bytes(v)
}

fn get64(b: &[u8], at: usize) -> u64 {
    let mut v = [0u8; 8];
    v.copy_from_slice(&b[at..at + 8]);
    u64::from_le_bytes(v)
}

fn put16(b: &mut [u8], at: usize, v: u16) {
    b[at..at + 2].copy_from_slice(&v.to_le_bytes());
}

fn put32(b: &mut [u8], at: usize, v: u32) {
    b[at..at + 4].copy_from_slice(&v.to_le_bytes());
}

fn put64(b: &mut [u8], at: usize, v: u64) {
    b[at..at + 8].copy_from_slice(&v.to_le_bytes());
}

impl Context {
    pub fn rsp(&self) -> u64 {
        self.gpr[usize::from(crate::consts::RSP)]
    }

    pub fn set_rsp(&mut self, v: u64) {
        self.gpr[usize::from(crate::consts::RSP)] = v;
    }

    /// `Xmm<n>` (`n` < 16), from `FltSave`.
    pub fn xmm(&self, n: usize) -> u128 {
        let at = FLT_SAVE_XMM0 + 16 * (n & 15);
        let mut v = [0u8; 16];
        v.copy_from_slice(&self.flt_save[at..at + 16]);
        u128::from_le_bytes(v)
    }

    pub fn set_xmm(&mut self, n: usize, v: u128) {
        let at = FLT_SAVE_XMM0 + 16 * (n & 15);
        self.flt_save[at..at + 16].copy_from_slice(&v.to_le_bytes());
    }

    /// The `CONTEXT` bytes (every field, whatever `ContextFlags` says).
    pub fn to_bytes(&self) -> Vec<u8> {
        let mut b = vec![0u8; CONTEXT_SIZE];
        for (i, v) in self.p_home.iter().enumerate() {
            put64(&mut b, OFF_P1_HOME + 8 * i, *v);
        }
        put32(&mut b, OFF_CONTEXT_FLAGS, self.context_flags);
        put32(&mut b, OFF_MX_CSR, self.mx_csr);
        put16(&mut b, OFF_SEG_CS, self.seg_cs);
        put16(&mut b, OFF_SEG_DS, self.seg_ds);
        put16(&mut b, OFF_SEG_ES, self.seg_es);
        put16(&mut b, OFF_SEG_FS, self.seg_fs);
        put16(&mut b, OFF_SEG_GS, self.seg_gs);
        put16(&mut b, OFF_SEG_SS, self.seg_ss);
        put32(&mut b, OFF_EFLAGS, self.eflags);
        for (i, v) in self.dr.iter().enumerate() {
            put64(&mut b, OFF_DR0 + 8 * i, *v);
        }
        for (i, v) in self.gpr.iter().enumerate() {
            put64(&mut b, OFF_RAX + 8 * i, *v);
        }
        put64(&mut b, OFF_RIP, self.rip);
        b[OFF_FLT_SAVE..OFF_FLT_SAVE + FLT_SAVE_SIZE].copy_from_slice(&self.flt_save);
        b[OFF_VECTOR_REGISTER..OFF_VECTOR_REGISTER + VECTOR_REGISTER_SIZE].copy_from_slice(&self.vector_register);
        put64(&mut b, OFF_VECTOR_CONTROL, self.vector_control);
        put64(&mut b, OFF_DEBUG_CONTROL, self.debug_control);
        put64(&mut b, OFF_LAST_BRANCH_TO_RIP, self.last_branch_to_rip);
        put64(&mut b, OFF_LAST_BRANCH_FROM_RIP, self.last_branch_from_rip);
        put64(&mut b, OFF_LAST_EXCEPTION_TO_RIP, self.last_exception_to_rip);
        put64(&mut b, OFF_LAST_EXCEPTION_FROM_RIP, self.last_exception_from_rip);
        b
    }

    /// Read a `CONTEXT`. `None` if `b` is shorter than [`CONTEXT_SIZE`].
    pub fn from_bytes(b: &[u8]) -> Option<Context> {
        if b.len() < CONTEXT_SIZE {
            return None;
        }
        let mut c = Context::default();
        for (i, v) in c.p_home.iter_mut().enumerate() {
            *v = get64(b, OFF_P1_HOME + 8 * i);
        }
        c.context_flags = get32(b, OFF_CONTEXT_FLAGS);
        c.mx_csr = get32(b, OFF_MX_CSR);
        c.seg_cs = get16(b, OFF_SEG_CS);
        c.seg_ds = get16(b, OFF_SEG_DS);
        c.seg_es = get16(b, OFF_SEG_ES);
        c.seg_fs = get16(b, OFF_SEG_FS);
        c.seg_gs = get16(b, OFF_SEG_GS);
        c.seg_ss = get16(b, OFF_SEG_SS);
        c.eflags = get32(b, OFF_EFLAGS);
        for (i, v) in c.dr.iter_mut().enumerate() {
            *v = get64(b, OFF_DR0 + 8 * i);
        }
        for (i, v) in c.gpr.iter_mut().enumerate() {
            *v = get64(b, OFF_RAX + 8 * i);
        }
        c.rip = get64(b, OFF_RIP);
        c.flt_save.copy_from_slice(&b[OFF_FLT_SAVE..OFF_FLT_SAVE + FLT_SAVE_SIZE]);
        c.vector_register.copy_from_slice(&b[OFF_VECTOR_REGISTER..OFF_VECTOR_REGISTER + VECTOR_REGISTER_SIZE]);
        c.vector_control = get64(b, OFF_VECTOR_CONTROL);
        c.debug_control = get64(b, OFF_DEBUG_CONTROL);
        c.last_branch_to_rip = get64(b, OFF_LAST_BRANCH_TO_RIP);
        c.last_branch_from_rip = get64(b, OFF_LAST_BRANCH_FROM_RIP);
        c.last_exception_to_rip = get64(b, OFF_LAST_EXCEPTION_TO_RIP);
        c.last_exception_from_rip = get64(b, OFF_LAST_EXCEPTION_FROM_RIP);
        Some(c)
    }
}

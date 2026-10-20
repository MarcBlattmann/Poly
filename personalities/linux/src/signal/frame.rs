//! The x86-64 signal frame: what a handler finds on its stack, and what
//! rt_sigreturn(2) reads back.
//!
//! Layouts from the UAPI headers: `struct ucontext` (`asm-generic/ucontext.h`:
//! `uc_flags`, `uc_link`, `uc_stack`, `uc_mcontext`, `uc_sigmask`),
//! `struct sigcontext` (x86 `asm/sigcontext.h`, the `__x86_64__` branch),
//! `stack_t` (x86 `asm/signal.h`), `siginfo_t` (`asm-generic/siginfo.h`),
//! the FPU frame with its software-reserved bytes and XSAVE markers
//! (`struct _fpstate_64`, `struct _fpx_sw_bytes`, `FP_XSTATE_MAGIC1/2`) and
//! the `uc_flags` bits (x86 `asm/ucontext.h`).
//!
//! ```text
//!   higher addresses
//!   ┌──────────────────────────────┐ ← top: the interrupted rsp minus the
//!   │ (red zone, 128 bytes)        │   128-byte red zone of the x86-64 psABI,
//!   ├──────────────────────────────┤   or the top of the alternate stack
//!   │ FPU state: FXSAVE / XSAVE    │ ← 64-byte aligned; sigcontext.fpstate
//!   │ image (+ FP_XSTATE_MAGIC2)   │
//!   ├──────────────────────────────┤
//!   │ (padding)                    │
//!   │ siginfo_t        128 bytes   │ ← frame + 312: rsi of the handler
//!   │ struct ucontext  304 bytes   │ ← frame + 8: rdx of the handler
//!   │ return address = sa_restorer │ ← frame: the handler's rsp,
//!   └──────────────────────────────┘   frame + 8 is 16-byte aligned
//! ```
//!
//! The handler is entered as if called (the return address on the stack,
//! `rsp + 8` 16-byte aligned, as the psABI requires at function entry).
//! When it returns into `sa_restorer`, which calls rt_sigreturn, `rsp`
//! points at the `ucontext`. The order of the three parts (return address,
//! `ucontext`, `siginfo`) and the FPU area above them is the conventional
//! x86-64 frame; user space sees only the pointers it is given, so the
//! exact placement is for the diff runner to confirm.

use crate::abi::signal::{FP_XSTATE_MAGIC1, FP_XSTATE_MAGIC2, UC_FP_XSTATE, UC_SIGCONTEXT_SS, UC_STRICT_RESTORE_SS};
use crate::abi::{SigInfo, StackT, le_u64};
use crate::kernel::{FpuState, SigSet, UserContext};

/// `sizeof(struct sigcontext)` on x86-64.
pub const SIGCONTEXT_SIZE: usize = 256;
/// `sizeof(struct ucontext)` on x86-64 (the kernel's, with an 8-byte mask).
pub const UCONTEXT_SIZE: usize = 304;
/// Offsets in `struct ucontext`.
pub const UC_FLAGS: usize = 0;
pub const UC_LINK: usize = 8;
pub const UC_STACK: usize = 16;
pub const UC_MCONTEXT: usize = 40;
pub const UC_SIGMASK: usize = 296;

/// Offsets in `struct sigcontext` (x86-64): `r8`-`r15`, `rdi`, `rsi`, `rbp`,
/// `rbx`, `rdx`, `rax`, `rcx`, `rsp`, `rip`, `eflags` (8 bytes each), then
/// `cs`, `gs`, `fs`, `ss` (2 bytes each), `err`, `trapno`, `oldmask`,
/// `cr2`, `fpstate`, `reserved1[8]`.
pub mod sc {
    pub const R8: usize = 0;
    pub const RDI: usize = 64;
    pub const RSI: usize = 72;
    pub const RBP: usize = 80;
    pub const RBX: usize = 88;
    pub const RDX: usize = 96;
    pub const RAX: usize = 104;
    pub const RCX: usize = 112;
    pub const RSP: usize = 120;
    pub const RIP: usize = 128;
    pub const EFLAGS: usize = 136;
    pub const CS: usize = 144;
    pub const GS: usize = 146;
    pub const FS: usize = 148;
    pub const SS: usize = 150;
    pub const ERR: usize = 152;
    pub const TRAPNO: usize = 160;
    pub const OLDMASK: usize = 168;
    pub const CR2: usize = 176;
    pub const FPSTATE: usize = 184;
    pub const RESERVED1: usize = 192;
}

/// Offsets in the frame.
pub const FRAME_UCONTEXT: usize = 8;
pub const FRAME_SIGINFO: usize = FRAME_UCONTEXT + UCONTEXT_SIZE;
/// The return address, the `ucontext` and the `siginfo`.
pub const FRAME_SIZE: usize = FRAME_SIGINFO + SigInfo::SIZE;

/// The x86-64 psABI red zone below `rsp`, which a signal frame must not
/// overwrite.
pub const RED_ZONE: u64 = 128;

/// The legacy (`FXSAVE`) part of the FPU state.
pub const FXSAVE_SIZE: usize = 512;
/// `struct _fpx_sw_bytes` sits in the FXSAVE area's bytes 464-511.
pub const SW_BYTES: usize = 464;
/// The XSAVE header follows the legacy area.
pub const XSAVE_HEADER_SIZE: usize = 64;

/// Where the parts of one frame go.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Layout {
    /// The lowest address: the return address, and the handler's `rsp`.
    pub frame: u64,
    pub ucontext: u64,
    pub siginfo: u64,
    pub fpstate: u64,
    /// One past the last byte written (the end of the FPU area).
    pub end: u64,
}

impl Layout {
    /// The frame below `top` for an FPU image of `fpu_len` bytes (`xsave`:
    /// with the 4-byte `FP_XSTATE_MAGIC2` after it). `None` if it would
    /// wrap below address 0.
    pub fn below(top: u64, fpu_len: usize, xsave: bool) -> Option<Layout> {
        let fpu_total = fpu_len as u64 + if xsave { 4 } else { 0 };
        let fpstate = top.checked_sub(fpu_total)? & !63;
        let frame = (fpstate.checked_sub(FRAME_SIZE as u64)? & !15).checked_sub(8)?;
        Some(Layout {
            frame,
            ucontext: frame + FRAME_UCONTEXT as u64,
            siginfo: frame + FRAME_SIGINFO as u64,
            fpstate,
            end: fpstate + fpu_total,
        })
    }

    /// The bytes from `frame` to `end`.
    pub const fn len(&self) -> usize {
        (self.end - self.frame) as usize
    }

    pub const fn is_empty(&self) -> bool {
        self.end == self.frame
    }
}

/// What goes into one frame.
#[derive(Clone, Copy, Debug)]
pub struct Frame<'a> {
    /// The interrupted registers.
    pub ctx: &'a UserContext,
    pub info: &'a SigInfo,
    /// Copy the siginfo (`SA_SIGINFO`); otherwise its place stays zero.
    pub with_siginfo: bool,
    /// The mask to restore on return (`uc_sigmask`).
    pub mask: SigSet,
    /// The alternate stack settings at delivery (`uc_stack`).
    pub stack: StackT,
    /// `sa_restorer`: the handler's return address.
    pub restorer: u64,
    pub fpu: FpuState<'a>,
}

impl Frame<'_> {
    /// Write the frame for `layout` into `buf` (`layout.len()` bytes, the
    /// bytes from `layout.frame` on).
    pub fn write(&self, layout: &Layout, buf: &mut [u8]) {
        assert_eq!(buf.len(), layout.len(), "the buffer covers the frame");
        buf.fill(0);
        let at = |addr: u64| (addr - layout.frame) as usize;
        put64(buf, 0, self.restorer);

        let uc = &mut buf[at(layout.ucontext)..at(layout.ucontext) + UCONTEXT_SIZE];
        let xsave = self.fpu.xfeatures.is_some();
        let flags = UC_SIGCONTEXT_SS | UC_STRICT_RESTORE_SS | if xsave { UC_FP_XSTATE } else { 0 };
        put64(uc, UC_FLAGS, flags);
        put64(uc, UC_LINK, 0);
        uc[UC_STACK..UC_STACK + StackT::SIZE].copy_from_slice(&self.stack.to_bytes());
        let sc = encode_sigcontext(self.ctx, self.mask, layout.fpstate);
        uc[UC_MCONTEXT..UC_MCONTEXT + SIGCONTEXT_SIZE].copy_from_slice(&sc);
        put64(uc, UC_SIGMASK, self.mask.0);

        if self.with_siginfo {
            buf[at(layout.siginfo)..at(layout.siginfo) + SigInfo::SIZE].copy_from_slice(&self.info.to_bytes());
        }

        let fp = at(layout.fpstate);
        let image = self.fpu.image;
        buf[fp..fp + image.len()].copy_from_slice(image);
        if image.len() >= FXSAVE_SIZE {
            let sw = &mut buf[fp + SW_BYTES..fp + FXSAVE_SIZE];
            sw.fill(0);
            if let Some(xfeatures) = self.fpu.xfeatures {
                // struct _fpx_sw_bytes: magic1, extended_size, xfeatures,
                // xstate_size.
                sw[0..4].copy_from_slice(&FP_XSTATE_MAGIC1.to_le_bytes());
                sw[4..8].copy_from_slice(&(image.len() as u32 + 4).to_le_bytes());
                sw[8..16].copy_from_slice(&xfeatures.to_le_bytes());
                sw[16..20].copy_from_slice(&(image.len() as u32).to_le_bytes());
                let end = fp + image.len();
                buf[end..end + 4].copy_from_slice(&FP_XSTATE_MAGIC2.to_le_bytes());
            }
        }
    }
}

/// `struct sigcontext` for `ctx`. `oldmask` gets the saved mask (all 64
/// signals fit; to be confirmed by the diff runner), the `gs` and `fs`
/// slots stay 0 (the header says they must not be reused), `fpstate`
/// points at the FPU area.
pub fn encode_sigcontext(ctx: &UserContext, mask: SigSet, fpstate: u64) -> [u8; SIGCONTEXT_SIZE] {
    let mut b = [0; SIGCONTEXT_SIZE];
    let gprs = [
        ctx.r8, ctx.r9, ctx.r10, ctx.r11, ctx.r12, ctx.r13, ctx.r14, ctx.r15, ctx.rdi, ctx.rsi, ctx.rbp, ctx.rbx,
        ctx.rdx, ctx.rax, ctx.rcx, ctx.rsp, ctx.rip, ctx.rflags,
    ];
    for (i, v) in gprs.iter().enumerate() {
        put64(&mut b, sc::R8 + 8 * i, *v);
    }
    b[sc::CS..sc::CS + 2].copy_from_slice(&ctx.cs.to_le_bytes());
    b[sc::SS..sc::SS + 2].copy_from_slice(&ctx.ss.to_le_bytes());
    put64(&mut b, sc::ERR, ctx.err);
    put64(&mut b, sc::TRAPNO, ctx.trapno);
    put64(&mut b, sc::OLDMASK, mask.0);
    put64(&mut b, sc::CR2, ctx.cr2);
    put64(&mut b, sc::FPSTATE, fpstate);
    b
}

/// What a `ucontext` read back by rt_sigreturn holds, unvalidated.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct SavedContext {
    pub flags: u64,
    pub stack: StackT,
    pub regs: UserContext,
    pub mask: SigSet,
    /// `sigcontext.fpstate`: the FPU area, or 0 for none.
    pub fpstate: u64,
}

/// Decode a `struct ucontext`.
pub fn decode_ucontext(uc: &[u8; UCONTEXT_SIZE]) -> SavedContext {
    let sc = &uc[UC_MCONTEXT..UC_MCONTEXT + SIGCONTEXT_SIZE];
    let r = |i: usize| le_u64(&sc[sc::R8 + 8 * i..]);
    let u16_at = |o: usize| u16::from_le_bytes([sc[o], sc[o + 1]]);
    let regs = UserContext {
        r8: r(0),
        r9: r(1),
        r10: r(2),
        r11: r(3),
        r12: r(4),
        r13: r(5),
        r14: r(6),
        r15: r(7),
        rdi: r(8),
        rsi: r(9),
        rbp: r(10),
        rbx: r(11),
        rdx: r(12),
        rax: r(13),
        rcx: r(14),
        rsp: r(15),
        rip: r(16),
        rflags: r(17),
        cs: u16_at(sc::CS),
        ss: u16_at(sc::SS),
        err: le_u64(&sc[sc::ERR..]),
        trapno: le_u64(&sc[sc::TRAPNO..]),
        cr2: le_u64(&sc[sc::CR2..]),
    };
    let mut stack = [0; StackT::SIZE];
    stack.copy_from_slice(&uc[UC_STACK..UC_STACK + StackT::SIZE]);
    SavedContext {
        flags: le_u64(&uc[UC_FLAGS..]),
        stack: StackT::from_bytes(&stack),
        regs,
        mask: SigSet(le_u64(&uc[UC_SIGMASK..])),
        fpstate: le_u64(&sc[sc::FPSTATE..]),
    }
}

/// How rt_sigreturn reads the FPU area at `sigcontext.fpstate`, given its
/// first 512 bytes: `Some(len)` for an extended (XSAVE) area of `len`
/// bytes (`xstate_size`), when `magic1` is `FP_XSTATE_MAGIC1`, the sizes
/// agree (`extended_size = xstate_size + 4`) and `xstate_size` is at
/// least the legacy area plus the XSAVE header and at most `max`.
/// `None`: use the 512 legacy bytes only (the header: "If
/// sw_reserved.magic1 == FP_XSTATE_MAGIC1 then there's a
/// sw_reserved.extended_size bytes large extended context area present").
/// The caller still checks `FP_XSTATE_MAGIC2` after the area.
pub fn xstate_size(legacy: &[u8; FXSAVE_SIZE], max: usize) -> Option<usize> {
    let word = |o: usize| u32::from_le_bytes([legacy[o], legacy[o + 1], legacy[o + 2], legacy[o + 3]]);
    let magic1 = word(SW_BYTES);
    let extended_size = word(SW_BYTES + 4) as usize;
    let xstate_size = word(SW_BYTES + 16) as usize;
    (magic1 == FP_XSTATE_MAGIC1
        && extended_size == xstate_size + 4
        && xstate_size >= FXSAVE_SIZE + XSAVE_HEADER_SIZE
        && xstate_size <= max)
        .then_some(xstate_size)
}

fn put64(b: &mut [u8], at: usize, v: u64) {
    b[at..at + 8].copy_from_slice(&v.to_le_bytes());
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::abi::signal::*;

    fn ctx() -> UserContext {
        UserContext {
            r8: 8,
            r9: 9,
            r10: 10,
            r11: 11,
            r12: 12,
            r13: 13,
            r14: 14,
            r15: 15,
            rdi: 0xd1,
            rsi: 0x51,
            rbp: 0xb9,
            rbx: 0xb1,
            rdx: 0xd0,
            rax: 0xa0,
            rcx: 0xc0,
            rsp: 0x7fff_ffff_e123,
            rip: 0x40_1234,
            rflags: 0x246,
            cs: 0x33,
            ss: 0x2b,
            trapno: 14,
            err: 6,
            cr2: 0xdead_0000,
        }
    }

    #[test]
    fn sizes_match_the_uapi_headers() {
        assert_eq!(sc::RESERVED1 + 64, SIGCONTEXT_SIZE);
        assert_eq!(UC_MCONTEXT + SIGCONTEXT_SIZE, UC_SIGMASK);
        assert_eq!(UC_SIGMASK + 8, UCONTEXT_SIZE);
        assert_eq!(UC_STACK + StackT::SIZE, UC_MCONTEXT);
        assert_eq!(FRAME_SIZE, 440);
    }

    #[test]
    fn layout_alignment() {
        for top in [0x7fff_ffff_f000u64, 0x7fff_ffff_e123 - RED_ZONE, 0x1_0007] {
            for (len, xsave) in [(512, false), (832, true), (2696, true)] {
                let l = Layout::below(top, len, xsave).unwrap();
                assert_eq!(l.fpstate % 64, 0);
                assert_eq!((l.frame + 8) % 16, 0, "rsp + 8 aligned as at function entry");
                assert!(l.end <= top && l.siginfo + 128 <= l.fpstate);
                assert_eq!(l.end - l.fpstate, len as u64 + if xsave { 4 } else { 0 });
                assert_eq!(l.len(), (l.end - l.frame) as usize);
            }
        }
        assert_eq!(Layout::below(100, 512, false), None);
    }

    #[test]
    fn frame_bytes_match_the_uapi_layouts() {
        let c = ctx();
        let info = SigInfo::from_process(SIGUSR1, SI_USER, 77, 1000);
        let mut image = [0u8; 832];
        for (i, b) in image.iter_mut().enumerate() {
            *b = i as u8;
        }
        let fpu = FpuState { image: &image, xfeatures: Some(0x7) };
        let stack = StackT { sp: 0x5000, flags: SS_DISABLE, size: 0 };
        let mask = SigSet(0x8000_0000_0000_0001);
        let frame = Frame { ctx: &c, info: &info, with_siginfo: true, mask, stack, restorer: 0x40_9000, fpu };
        let layout = Layout::below(c.rsp - RED_ZONE, image.len(), true).unwrap();
        let mut buf = alloc::vec![0xee; layout.len()];
        frame.write(&layout, &mut buf);

        let q = |o: usize| le_u64(&buf[o..]);
        assert_eq!(q(0), 0x40_9000, "pretcode");
        let uc = (layout.ucontext - layout.frame) as usize;
        assert_eq!(uc, 8);
        assert_eq!(q(uc), UC_FP_XSTATE | UC_SIGCONTEXT_SS | UC_STRICT_RESTORE_SS, "uc_flags");
        assert_eq!(q(uc + 8), 0, "uc_link");
        assert_eq!((q(uc + 16), q(uc + 24) as u32, q(uc + 32)), (0x5000, SS_DISABLE, 0), "uc_stack");
        let m = uc + 40;
        let expect = [8u64, 9, 10, 11, 12, 13, 14, 15, 0xd1, 0x51, 0xb9, 0xb1, 0xd0, 0xa0, 0xc0];
        for (i, v) in expect.iter().enumerate() {
            assert_eq!(q(m + 8 * i), *v, "register {i}");
        }
        assert_eq!(q(m + 120), c.rsp);
        assert_eq!(q(m + 128), c.rip);
        assert_eq!(q(m + 136), 0x246);
        assert_eq!(&buf[m + 144..m + 152], &[0x33, 0, 0, 0, 0, 0, 0x2b, 0], "cs, gs, fs, ss");
        assert_eq!((q(m + 152), q(m + 160), q(m + 168), q(m + 176)), (6, 14, mask.0, 0xdead_0000));
        assert_eq!(q(m + 184), layout.fpstate, "fpstate pointer");
        assert!(buf[m + 192..m + 256].iter().all(|&b| b == 0), "reserved1");
        assert_eq!(q(uc + 296), mask.0, "uc_sigmask");
        let si = (layout.siginfo - layout.frame) as usize;
        assert_eq!(si, 312);
        assert_eq!(&buf[si..si + 128], &info.to_bytes());

        let fp = (layout.fpstate - layout.frame) as usize;
        assert_eq!(&buf[fp..fp + 464], &image[..464], "legacy area copied");
        let w = |o: usize| u32::from_le_bytes(buf[fp + o..fp + o + 4].try_into().unwrap());
        assert_eq!(w(464), FP_XSTATE_MAGIC1);
        assert_eq!(w(468), 832 + 4, "extended_size");
        assert_eq!(q(fp + 472), 0x7, "xfeatures");
        assert_eq!(w(480), 832, "xstate_size");
        assert!(buf[fp + 484..fp + 512].iter().all(|&b| b == 0), "padding");
        assert_eq!(&buf[fp + 512..fp + 832], &image[512..], "extended area copied");
        assert_eq!(w(832), FP_XSTATE_MAGIC2);
        let mut legacy = [0; FXSAVE_SIZE];
        legacy.copy_from_slice(&buf[fp..fp + 512]);
        assert_eq!(xstate_size(&legacy, 4096), Some(832));
        assert_eq!(xstate_size(&legacy, 800), None, "larger than the CPU's");
    }

    #[test]
    fn legacy_fpu_frames_have_no_xstate_markers() {
        let c = ctx();
        let info = SigInfo::new(SIGINT, SI_KERNEL);
        let image = [0x55u8; 512];
        let fpu = FpuState { image: &image, xfeatures: None };
        let frame = Frame {
            ctx: &c,
            info: &info,
            with_siginfo: false,
            mask: SigSet::EMPTY,
            stack: StackT::default(),
            restorer: 1,
            fpu,
        };
        let layout = Layout::below(0x10_0000, 512, false).unwrap();
        let mut buf = alloc::vec![0; layout.len()];
        frame.write(&layout, &mut buf);
        assert_eq!(le_u64(&buf[8..]), UC_SIGCONTEXT_SS | UC_STRICT_RESTORE_SS);
        assert!(buf[312..440].iter().all(|&b| b == 0), "no SA_SIGINFO: no siginfo");
        let fp = (layout.fpstate - layout.frame) as usize;
        assert!(buf[fp + 464..fp + 512].iter().all(|&b| b == 0), "magic1 = 0: legacy frame");
        let mut legacy = [0; FXSAVE_SIZE];
        legacy.copy_from_slice(&buf[fp..fp + 512]);
        assert_eq!(xstate_size(&legacy, 4096), None);
    }

    #[test]
    fn ucontext_round_trip() {
        let c = ctx();
        let info = SigInfo::new(SIGINT, SI_KERNEL);
        let image = [0u8; 512];
        let fpu = FpuState { image: &image, xfeatures: None };
        let stack = StackT { sp: 0x9000, flags: SS_ONSTACK | SS_AUTODISARM, size: 0x4000 };
        let mask = SigSet(0x1234);
        let frame = Frame { ctx: &c, info: &info, with_siginfo: true, mask, stack, restorer: 1, fpu };
        let layout = Layout::below(0x10_0000, 512, false).unwrap();
        let mut buf = alloc::vec![0; layout.len()];
        frame.write(&layout, &mut buf);
        let mut uc = [0; UCONTEXT_SIZE];
        uc.copy_from_slice(&buf[8..8 + UCONTEXT_SIZE]);
        let saved = decode_ucontext(&uc);
        assert_eq!(saved.regs, c);
        assert_eq!(saved.mask, mask);
        assert_eq!(saved.stack, stack);
        assert_eq!(saved.fpstate, layout.fpstate);
        assert_eq!(saved.flags, UC_SIGCONTEXT_SS | UC_STRICT_RESTORE_SS);
    }
}

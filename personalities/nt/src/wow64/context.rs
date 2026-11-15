//! The x86 `CONTEXT` and `EXCEPTION_RECORD` of a 32-bit process (ADR 0186), and their conversion
//! to and from the 64-bit ones the rest of the personality works with (`poly_seh`).
//!
//! Layouts from `winnt.h` (the `_X86_` definitions): `CONTEXT` is 0x2CC bytes —
//! `ContextFlags` (0), `Dr0`–`Dr3`, `Dr6`, `Dr7` (4–0x18), `FloatSave` (`FLOATING_SAVE_AREA`, 112
//! bytes at 0x1C), `SegGs`, `SegFs`, `SegEs`, `SegDs` (0x8C–0x98), `Edi`, `Esi`, `Ebx`, `Edx`,
//! `Ecx`, `Eax` (0x9C–0xB0), `Ebp` (0xB4), `Eip` (0xB8), `SegCs` (0xBC), `EFlags` (0xC0), `Esp`
//! (0xC4), `SegSs` (0xC8), `ExtendedRegisters` (512 bytes, the `FXSAVE` image, at 0xCC). The
//! flags: `CONTEXT_i386` (0x10000) with `CONTROL` 1, `INTEGER` 2, `SEGMENTS` 4,
//! `FLOATING_POINT` 8, `DEBUG_REGISTERS` 0x10, `EXTENDED_REGISTERS` 0x20. `EXCEPTION_RECORD` is
//! 0x50 bytes: `ExceptionCode`, `ExceptionFlags`, `ExceptionRecord`, `ExceptionAddress`,
//! `NumberParameters`, `ExceptionInformation[15]` (4 bytes each).
//!
//! The x87/SSE state travels as the `FXSAVE` image both structures hold (`ExtendedRegisters`
//! here, `FltSave` in x64's); the `FNSAVE`-format `FloatSave` is not filled.

use poly_seh::context::{CONTEXT_CONTROL, CONTEXT_FLOATING_POINT, CONTEXT_INTEGER, CONTEXT_SEGMENTS, FLT_SAVE_SIZE};
use poly_seh::{Context, ExceptionRecord};

pub const CONTEXT32_SIZE: usize = 0x2CC;
pub const EXCEPTION_RECORD32_SIZE: usize = 0x50;
pub const CONTEXT_I386: u32 = 0x1_0000;
pub const CONTEXT32_CONTROL: u32 = CONTEXT_I386 | 0x01;
pub const CONTEXT32_INTEGER: u32 = CONTEXT_I386 | 0x02;
pub const CONTEXT32_SEGMENTS: u32 = CONTEXT_I386 | 0x04;
pub const CONTEXT32_EXTENDED_REGISTERS: u32 = CONTEXT_I386 | 0x20;

const OFF_SEG_GS: usize = 0x8C;
const OFF_SEG_FS: usize = 0x90;
const OFF_SEG_ES: usize = 0x94;
const OFF_SEG_DS: usize = 0x98;
const OFF_EDI: usize = 0x9C;
const OFF_ESI: usize = 0xA0;
const OFF_EBX: usize = 0xA4;
const OFF_EDX: usize = 0xA8;
const OFF_ECX: usize = 0xAC;
const OFF_EAX: usize = 0xB0;
const OFF_EBP: usize = 0xB4;
const OFF_EIP: usize = 0xB8;
const OFF_SEG_CS: usize = 0xBC;
const OFF_EFLAGS: usize = 0xC0;
const OFF_ESP: usize = 0xC4;
const OFF_SEG_SS: usize = 0xC8;
const OFF_EXTENDED: usize = 0xCC;

/// `gpr` index of each 32-bit register field (`Rax`..`R15` numbering).
const GPRS: [(usize, usize); 8] =
    [(OFF_EAX, 0), (OFF_ECX, 1), (OFF_EDX, 2), (OFF_EBX, 3), (OFF_ESP, 4), (OFF_EBP, 5), (OFF_ESI, 6), (OFF_EDI, 7)];

fn put(b: &mut [u8], at: usize, v: u32) {
    b[at..at + 4].copy_from_slice(&v.to_le_bytes());
}

fn get(b: &[u8], at: usize) -> u32 {
    u32::from_le_bytes(b[at..at + 4].try_into().expect("4 bytes"))
}

/// The x86 `CONTEXT` of a 32-bit thread whose registers are `ctx` (control, integer and segment
/// registers, and the `FXSAVE` image when `ctx` has it).
pub fn to_context32(ctx: &Context) -> [u8; CONTEXT32_SIZE] {
    let mut b = [0u8; CONTEXT32_SIZE];
    let mut flags = CONTEXT32_CONTROL | CONTEXT32_INTEGER | CONTEXT32_SEGMENTS;
    for (at, reg) in GPRS {
        put(&mut b, at, ctx.gpr[reg] as u32);
    }
    put(&mut b, OFF_EIP, ctx.rip as u32);
    put(&mut b, OFF_EFLAGS, ctx.eflags);
    for (at, seg) in [
        (OFF_SEG_CS, ctx.seg_cs),
        (OFF_SEG_SS, ctx.seg_ss),
        (OFF_SEG_DS, ctx.seg_ds),
        (OFF_SEG_ES, ctx.seg_es),
        (OFF_SEG_FS, ctx.seg_fs),
        (OFF_SEG_GS, ctx.seg_gs),
    ] {
        put(&mut b, at, u32::from(seg));
    }
    if ctx.context_flags & CONTEXT_FLOATING_POINT == CONTEXT_FLOATING_POINT {
        b[OFF_EXTENDED..OFF_EXTENDED + FLT_SAVE_SIZE].copy_from_slice(&ctx.flt_save);
        flags |= CONTEXT32_EXTENDED_REGISTERS;
    }
    put(&mut b, 0, flags);
    b
}

/// The registers an x86 `CONTEXT` (`b`, [`CONTEXT32_SIZE`] bytes) holds, as a 64-bit `Context`
/// whose `context_flags` select the same groups (the high halves are zero).
pub fn from_context32(b: &[u8]) -> Context {
    let flags = get(b, 0);
    let mut c = Context::default();
    let mut f = 0u32;
    let has = |bit: u32| flags & CONTEXT_I386 != 0 && flags & bit != 0;
    for (at, reg) in GPRS {
        c.gpr[reg] = u64::from(get(b, at));
    }
    c.rip = u64::from(get(b, OFF_EIP));
    c.eflags = get(b, OFF_EFLAGS);
    c.seg_cs = get(b, OFF_SEG_CS) as u16;
    c.seg_ss = get(b, OFF_SEG_SS) as u16;
    c.seg_ds = get(b, OFF_SEG_DS) as u16;
    c.seg_es = get(b, OFF_SEG_ES) as u16;
    c.seg_fs = get(b, OFF_SEG_FS) as u16;
    c.seg_gs = get(b, OFF_SEG_GS) as u16;
    if has(0x01) {
        f |= CONTEXT_CONTROL;
    }
    if has(0x02) {
        f |= CONTEXT_INTEGER;
    }
    if has(0x04) {
        f |= CONTEXT_SEGMENTS;
    }
    if has(0x20) {
        c.flt_save.copy_from_slice(&b[OFF_EXTENDED..OFF_EXTENDED + FLT_SAVE_SIZE]);
        // `MxCsr` is the FXSAVE image's dword at 24.
        c.mx_csr = get(&c.flt_save, 24);
        f |= CONTEXT_FLOATING_POINT;
    }
    c.context_flags = f;
    c
}

/// The x86 `EXCEPTION_RECORD` of `r` (addresses and parameters cut to 32 bits).
pub fn to_record32(r: &ExceptionRecord) -> [u8; EXCEPTION_RECORD32_SIZE] {
    let mut b = [0u8; EXCEPTION_RECORD32_SIZE];
    put(&mut b, 0, r.code);
    put(&mut b, 4, r.flags);
    put(&mut b, 8, r.record as u32);
    put(&mut b, 12, r.address as u32);
    put(&mut b, 16, r.number_parameters);
    for (i, p) in r.information.iter().enumerate() {
        put(&mut b, 20 + 4 * i, *p as u32);
    }
    b
}

/// The record an x86 `EXCEPTION_RECORD` (`b`, [`EXCEPTION_RECORD32_SIZE`] bytes) holds; `None`
/// when `NumberParameters` is above 15 (`EXCEPTION_MAXIMUM_PARAMETERS`).
pub fn from_record32(b: &[u8]) -> Option<ExceptionRecord> {
    let n = get(b, 16);
    if n > 15 {
        return None;
    }
    let mut r = ExceptionRecord::new(get(b, 0), get(b, 4), u64::from(get(b, 12)), &[]);
    r.record = u64::from(get(b, 8));
    r.number_parameters = n;
    for i in 0..r.information.len() {
        r.information[i] = u64::from(get(b, 20 + 4 * i));
    }
    Some(r)
}

/// Where an exception's frame goes on a 32-bit thread's stack, below `esp`: the `CONTEXT`
/// (16-byte aligned, for its `FXSAVE` image), the `EXCEPTION_RECORD` below it, and below that the
/// dispatcher's call frame — a zero return address, then the record's and the context's
/// addresses, as if `KiUserExceptionDispatcher(record, context)` had been called (`cdecl`;
/// Poly's own convention, both sides ours). `None` when it would leave the 32-bit space.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Frame32 {
    pub esp: u64,
    pub record: u64,
    pub context: u64,
}

pub fn frame32_below(esp: u64) -> Option<Frame32> {
    if esp >= 1 << 32 {
        return None;
    }
    let context = esp.checked_sub(CONTEXT32_SIZE as u64)? & !15;
    let record = context.checked_sub(EXCEPTION_RECORD32_SIZE as u64)?;
    let sp = record.checked_sub(12)?;
    (sp >= 0x1_0000).then_some(Frame32 { esp: sp, record, context })
}

/// The call frame's 12 bytes: a zero return address, the record's and the context's addresses.
pub fn call_frame(f: &Frame32) -> [u8; 12] {
    let mut b = [0u8; 12];
    put(&mut b, 4, f.record as u32);
    put(&mut b, 8, f.context as u32);
    b
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_context_survives_the_trip_to_32_bits_and_back() {
        let mut c =
            Context { context_flags: CONTEXT_CONTROL | CONTEXT_INTEGER | CONTEXT_FLOATING_POINT, ..Context::default() };
        for (i, g) in c.gpr.iter_mut().enumerate().take(8) {
            *g = 0x1000 + i as u64;
        }
        c.rip = 0x40_1234;
        c.eflags = 0x246;
        c.seg_cs = 0x3B;
        c.seg_ss = 0x1B;
        c.flt_save[24..28].copy_from_slice(&0x1F80u32.to_le_bytes());
        c.flt_save[160] = 0xAB;
        let b = to_context32(&c);
        assert_eq!(get(&b, 0), CONTEXT_I386 | 0x27);
        assert_eq!(get(&b, OFF_EAX), 0x1000);
        assert_eq!(get(&b, OFF_ESP), 0x1004);
        assert_eq!(get(&b, OFF_EDI), 0x1007);
        assert_eq!(get(&b, OFF_EIP), 0x40_1234);
        assert_eq!(get(&b, OFF_SEG_CS), 0x3B);
        assert_eq!(b[OFF_EXTENDED + 160], 0xAB);
        let back = from_context32(&b);
        assert_eq!(back.context_flags, CONTEXT_CONTROL | CONTEXT_INTEGER | CONTEXT_SEGMENTS | CONTEXT_FLOATING_POINT);
        assert_eq!(&back.gpr[..8], &c.gpr[..8]);
        assert_eq!((back.rip, back.eflags, back.mx_csr), (0x40_1234, 0x246, 0x1F80));
        assert_eq!(back.flt_save[160], 0xAB);
    }

    #[test]
    fn only_the_groups_a_32_bit_context_names_are_taken() {
        let mut b = [0u8; CONTEXT32_SIZE];
        put(&mut b, 0, CONTEXT32_CONTROL);
        put(&mut b, OFF_EIP, 0x1234);
        let c = from_context32(&b);
        assert_eq!(c.context_flags, CONTEXT_CONTROL);
        // Without CONTEXT_i386 the flags mean nothing.
        put(&mut b, 0, 0x3);
        assert_eq!(from_context32(&b).context_flags, 0);
    }

    #[test]
    fn a_record_has_four_byte_addresses_and_parameters() {
        let r = ExceptionRecord::new(0xC000_0005, 0, 0x40_1000, &[1, 0x10]);
        let b = to_record32(&r);
        assert_eq!(
            (get(&b, 0), get(&b, 12), get(&b, 16), get(&b, 20), get(&b, 24)),
            (0xC000_0005, 0x40_1000, 2, 1, 0x10)
        );
        assert_eq!(from_record32(&b), Some(r));
        let mut bad = b;
        put(&mut bad, 16, 16);
        assert_eq!(from_record32(&bad), None);
    }

    #[test]
    fn the_frame_lies_below_the_stack_pointer_and_inside_32_bits() {
        let f = frame32_below(0x0012_FF00).unwrap();
        assert_eq!(f.context % 16, 0);
        assert!(f.context + CONTEXT32_SIZE as u64 <= 0x0012_FF00);
        assert_eq!(f.record + EXCEPTION_RECORD32_SIZE as u64, f.context);
        assert_eq!(f.esp + 12, f.record);
        assert_eq!(call_frame(&f)[4..8], (f.record as u32).to_le_bytes());
        assert_eq!(frame32_below(1 << 32), None);
        assert_eq!(frame32_below(0x1_0100), None);
    }
}

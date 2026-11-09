//! Unit tests. Encodings come from [`crate::vectors`] (assembled with the
//! system assembler, the text beside each word) or are built from field
//! helpers written from the Arm ARM encoding diagrams.

use crate::cpu::{Access, Cpu, MemFault, Memory, NZCV_C, NZCV_N, NZCV_V, NZCV_Z, Outcome, SliceMemory};
use crate::decode::{decode, decode_bit_masks};
use crate::inst::*;
use crate::vectors::asm;
use std::vec::Vec;

const CODE: u64 = 0x1000;
const DATA: u64 = 0x2000;
const SIZE: usize = 0x4000;

/// A CPU with a flat 16 KiB memory at 0x1000 (code at the start, data at 0x2000).
struct T {
    cpu: Cpu,
    mem: Vec<u8>,
}

impl T {
    fn new() -> T {
        let mut cpu = Cpu::new();
        cpu.pc = CODE;
        cpu.sp = DATA + 0x1000;
        T { cpu, mem: std::vec![0; SIZE] }
    }

    fn mem_at(&mut self, addr: u64) -> &mut [u8] {
        &mut self.mem[(addr - CODE) as usize..]
    }

    fn write(&mut self, addr: u64, bytes: &[u8]) {
        self.mem_at(addr)[..bytes.len()].copy_from_slice(bytes);
    }

    fn w64(&mut self, addr: u64, v: u64) {
        self.write(addr, &v.to_le_bytes());
    }

    fn r64(&self, addr: u64) -> u64 {
        let o = (addr - CODE) as usize;
        u64::from_le_bytes(self.mem[o..o + 8].try_into().unwrap())
    }

    fn r32(&self, addr: u64) -> u32 {
        let o = (addr - CODE) as usize;
        u32::from_le_bytes(self.mem[o..o + 4].try_into().unwrap())
    }

    fn r8(&self, addr: u64) -> u8 {
        self.mem[(addr - CODE) as usize]
    }

    /// Load `words` at the code address and step once per word (stopping at the first outcome
    /// that is not `Continue`).
    fn run_words(&mut self, words: &[u32]) -> Outcome {
        for (i, w) in words.iter().enumerate() {
            self.write(CODE + 4 * i as u64, &w.to_le_bytes());
        }
        self.cpu.pc = CODE;
        let mut mem = SliceMemory::new(CODE, &mut self.mem);
        for _ in 0..words.len() {
            match self.cpu.step(&mut mem) {
                Outcome::Continue => {}
                other => return other,
            }
        }
        Outcome::Continue
    }

    fn run(&mut self, lines: &[&str]) -> Outcome {
        let words: Vec<u32> = lines.iter().map(|l| asm(l)).collect();
        self.run_words(&words)
    }

    fn ok(&mut self, lines: &[&str]) -> &mut T {
        assert_eq!(self.run(lines), Outcome::Continue, "{lines:?}");
        self
    }

    fn x(&self, n: usize) -> u64 {
        self.cpu.x[n]
    }

    fn flags(&self) -> u32 {
        self.cpu.nzcv
    }
}

fn nzcv(n: bool, z: bool, c: bool, v: bool) -> u32 {
    (if n { NZCV_N } else { 0 })
        | (if z { NZCV_Z } else { 0 })
        | (if c { NZCV_C } else { 0 })
        | (if v { NZCV_V } else { 0 })
}

// ---------------------------------------------------------------------
// Integer
// ---------------------------------------------------------------------

#[test]
fn add_sub_immediate_and_flags() {
    let mut t = T::new();
    t.cpu.x[1] = 0xFFFF_FFFF;
    t.cpu.x[0] = 0xDEAD_BEEF_0000_0000;
    t.ok(&["adds w0, w1, #1"]);
    assert_eq!(t.x(0), 0, "32-bit result zeroes the upper half");
    assert_eq!(t.flags(), nzcv(false, true, true, false));

    t.cpu.x[1] = 0x7FFF_FFFF;
    t.ok(&["adds w0, w1, #1"]);
    assert_eq!(t.x(0), 0x8000_0000);
    assert_eq!(t.flags(), nzcv(true, false, false, true), "signed overflow");

    t.cpu.x[1] = 5;
    t.ok(&["add x0, x1, #5"]);
    assert_eq!(t.x(0), 10);
    assert_eq!(t.flags(), nzcv(true, false, false, true), "plain ADD leaves the flags");

    // SUB with the 12-bit shifted immediate, to SP.
    t.cpu.sp = 0x10000;
    t.ok(&["sub sp, sp, #4096"]);
    assert_eq!(t.cpu.sp, 0xF000);
    t.ok(&["add sp, sp, #16"]);
    assert_eq!(t.cpu.sp, 0xF010);
}

#[test]
fn compare_with_sp_and_xzr() {
    let mut t = T::new();
    t.cpu.sp = 16;
    t.ok(&["cmp sp, #16"]);
    assert_eq!(t.flags(), nzcv(false, true, true, false));
    // SUBS with rd = 31 is XZR: no register changes.
    t.cpu.x[1] = 1;
    let before = t.cpu.clone().x;
    t.ok(&["subs xzr, x1, #1"]);
    assert_eq!(t.cpu.x, before);
    assert_eq!(t.cpu.sp, 16);
    assert_eq!(t.flags(), nzcv(false, true, true, false));
}

#[test]
fn subtract_borrow_flags() {
    let mut t = T::new();
    t.cpu.x[1] = 0;
    t.cpu.x[2] = 1;
    t.ok(&["cmp w1, w2"]);
    assert_eq!(t.flags(), nzcv(true, false, false, false));
    t.cpu.x[1] = 0x8000_0000;
    t.ok(&["cmp w1, w2"]);
    assert_eq!(t.flags(), nzcv(false, false, true, true));
}

#[test]
fn shifted_and_extended_register_operands() {
    let mut t = T::new();
    t.cpu.x[1] = 100;
    t.cpu.x[2] = 3;
    t.ok(&["add x0, x1, x2, lsl #3"]);
    assert_eq!(t.x(0), 124);
    t.cpu.x[1] = 0x100;
    t.cpu.x[2] = 0x80;
    t.ok(&["subs x0, x1, x2, asr #4"]);
    assert_eq!(t.x(0), 0x100 - 8);
    t.cpu.sp = 0x1000;
    t.cpu.x[2] = 0xFFFF_FFFF_0000_0010;
    t.ok(&["add x0, sp, w2, uxtw #2"]);
    assert_eq!(t.x(0), 0x1000 + 0x40, "UXTW takes the low 32 bits");
    t.cpu.x[1] = 50;
    t.cpu.x[2] = 0xFF;
    t.ok(&["add x0, x1, w2, sxtb #1"]);
    assert_eq!(t.x(0), 48, "SXTB of 0xFF is -1, shifted left to -2");
    t.cpu.x[1] = u64::MAX;
    t.cpu.x[2] = 1;
    t.ok(&["adds x0, x1, x2, uxtx"]);
    assert_eq!(t.x(0), 0);
    assert_eq!(t.flags(), nzcv(false, true, true, false));
}

#[test]
fn add_with_carry_and_subtract_with_carry() {
    let mut t = T::new();
    t.cpu.x[1] = 5;
    t.cpu.x[2] = 6;
    t.cpu.nzcv = NZCV_C;
    t.ok(&["adc x0, x1, x2"]);
    assert_eq!(t.x(0), 12);
    t.cpu.nzcv = 0;
    t.ok(&["adc x0, x1, x2"]);
    assert_eq!(t.x(0), 11);
    // SBCS: 5 - 6 - !C. With C clear it is 5 - 6 - 1 = -2.
    t.cpu.x[1] = 5;
    t.cpu.x[2] = 6;
    t.cpu.nzcv = 0;
    t.ok(&["sbcs w0, w1, w2"]);
    assert_eq!(t.x(0), 0xFFFF_FFFE);
    assert_eq!(t.flags(), nzcv(true, false, false, false));
}

#[test]
fn logical_immediates_and_shifted_registers() {
    let mut t = T::new();
    t.cpu.x[1] = u64::MAX;
    t.ok(&["and x0, x1, #0xaaaaaaaaaaaaaaaa"]);
    assert_eq!(t.x(0), 0xAAAA_AAAA_AAAA_AAAA);
    t.cpu.x[1] = 0;
    t.ok(&["orr x0, x1, #0x00ff00ff00ff00ff"]);
    assert_eq!(t.x(0), 0x00FF_00FF_00FF_00FF);
    t.cpu.x[1] = 0xFFFF_FFFF_FFFF_FFF0;
    t.ok(&["eor w0, w1, #0x7"]);
    assert_eq!(t.x(0), 0xFFFF_FFF7, "W form zeroes the upper half");
    t.cpu.x[1] = 0x8000_0000_0000_FFFF;
    t.ok(&["ands x0, x1, #0xffff"]);
    assert_eq!(t.x(0), 0xFFFF);
    assert_eq!(t.flags(), 0);
    t.cpu.x[1] = 0x8000_0000_0000_0000;
    t.ok(&["tst x1, x2"]);
    assert_eq!(t.flags(), nzcv(false, true, false, false), "no common bits");
    // AND to SP: the destination of the non-flag-setting immediate form is SP.
    t.cpu.x[1] = 0x1234_5678_9ABC_DEF7;
    t.ok(&["and sp, x1, #0xfffffffffffffff0"]);
    assert_eq!(t.cpu.sp, 0x1234_5678_9ABC_DEF0);
    t.ok(&["orr x0, xzr, #0x1"]);
    assert_eq!(t.x(0), 1);

    t.cpu.x[1] = 0xF0F0;
    t.cpu.x[2] = 0x00FF;
    t.ok(&["bic x0, x1, x2"]);
    assert_eq!(t.x(0), 0xF000);
    t.ok(&["orn x0, x1, x2, lsl #4"]);
    assert_eq!(t.x(0), 0xF0F0 | !0x0FF0u64);
    t.ok(&["eon w0, w1, w2"]);
    assert_eq!(t.x(0), u64::from(!(0xF0F0u32 ^ 0xFF)));
    t.ok(&["mvn x0, x2"]);
    assert_eq!(t.x(0), !0xFFu64);
    t.ok(&["mov x0, x1"]);
    assert_eq!(t.x(0), 0xF0F0);
}

#[test]
fn bitmask_immediate_decoding() {
    // (N, immr, imms) -> value, from the Arm ARM's DecodeBitMasks.
    assert_eq!(decode_bit_masks(0, 0b111100, 0, true, 64).map(|m| m.0), Some(0x5555_5555_5555_5555));
    assert_eq!(decode_bit_masks(0, 0b111100, 1, true, 64).map(|m| m.0), Some(0xAAAA_AAAA_AAAA_AAAA));
    assert_eq!(decode_bit_masks(1, 0, 0, true, 64).map(|m| m.0), Some(1));
    assert_eq!(decode_bit_masks(1, 62, 0, true, 64).map(|m| m.0), Some(0x7FFF_FFFF_FFFF_FFFF));
    assert_eq!(decode_bit_masks(0, 0b000111, 0, true, 32).map(|m| m.0), Some(0xFF));
    // Reserved: all ones in an element, and a zero length.
    assert_eq!(decode_bit_masks(1, 63, 0, true, 64), None);
    assert_eq!(decode_bit_masks(0, 0b111110, 0, true, 32), None);
    assert_eq!(decode_bit_masks(0, 0b111111, 0, false, 32), None);
    // Through the decoder: 64-bit N=1 with a 32-bit register is unallocated.
    assert!(matches!(
        decode(asm("and x0, x1, #0xaaaaaaaaaaaaaaaa")),
        Inst::LogicalImm { imm: 0xAAAA_AAAA_AAAA_AAAA, .. }
    ));
    let w = asm("eor w0, w1, #0x7");
    assert!(matches!(decode(w), Inst::LogicalImm { w: Width::W, imm: 7, .. }));
    assert!(matches!(decode(w | 1 << 22), Inst::Undefined(_)));
}

#[test]
fn move_wide_and_pc_relative() {
    let mut t = T::new();
    t.ok(&["movz x0, #0x1234, lsl #16"]);
    assert_eq!(t.x(0), 0x1234_0000);
    t.ok(&["movk x0, #0xbeef, lsl #48"]);
    assert_eq!(t.x(0), 0xBEEF_0000_1234_0000);
    t.ok(&["movn x0, #0x1234"]);
    assert_eq!(t.x(0), !0x1234u64);
    t.cpu.x[0] = u64::MAX;
    t.ok(&["movn w0, #0x1234"]);
    assert_eq!(t.x(0), u64::from(!0x1234u32), "W form zeroes the upper half");

    t.ok(&["adr x0, #0x100"]);
    assert_eq!(t.x(0), CODE + 0x100);
    t.ok(&["adr x0, #-4"]);
    assert_eq!(t.x(0), CODE - 4);
    // ADRP works from the 4 KiB page of the instruction.
    t.cpu.pc = CODE;
    let w = asm("adrp x0, #0x5000");
    t.write(CODE + 0x44, &w.to_le_bytes());
    t.cpu.pc = CODE + 0x44;
    let mut mem = SliceMemory::new(CODE, &mut t.mem);
    assert_eq!(t.cpu.step(&mut mem), Outcome::Continue);
    assert_eq!(t.x(0), 0x1000 + 0x5000);
}

#[test]
fn bitfield_aliases() {
    let mut t = T::new();
    t.cpu.x[1] = 0x0123_4567_89AB_CDEF;
    t.ok(&["lsl x0, x1, #4"]);
    assert_eq!(t.x(0), 0x1234_5678_9ABC_DEF0);
    t.ok(&["lsr x0, x1, #4"]);
    assert_eq!(t.x(0), 0x0012_3456_789A_BCDE);
    t.cpu.x[1] = 0x8000_0000_0000_0000;
    t.ok(&["asr x0, x1, #4"]);
    assert_eq!(t.x(0), 0xF800_0000_0000_0000);
    t.cpu.x[1] = 0xFFFF_FFFF;
    t.cpu.x[0] = u64::MAX;
    t.ok(&["lsl w0, w1, #4"]);
    assert_eq!(t.x(0), 0xFFFF_FFF0);
    t.cpu.x[1] = 0xABCD_EF12;
    t.ok(&["ubfx x0, x1, #8, #8"]);
    assert_eq!(t.x(0), 0xEF);
    t.ok(&["sbfx x0, x1, #4, #12"]);
    assert_eq!(t.x(0), 0xFFFF_FFFF_FFFF_FEF1, "bits 15:4 are 0xef1, sign-extended from bit 11");
    // BFI keeps the other bits of the destination.
    t.cpu.x[0] = 0xFFFF_FFFF_FFFF_FFFF;
    t.cpu.x[1] = 0;
    t.ok(&["bfi x0, x1, #8, #8"]);
    assert_eq!(t.x(0), 0xFFFF_FFFF_FFFF_00FF);
    t.cpu.x[0] = 0;
    t.cpu.x[1] = 0xABCD;
    t.ok(&["bfxil x0, x1, #4, #8"]);
    assert_eq!(t.x(0), 0xBC);
    t.cpu.x[1] = 0x1234_5680;
    t.ok(&["uxtb w0, w1"]);
    assert_eq!(t.x(0), 0x80);
    t.ok(&["sxtb x0, w1"]);
    assert_eq!(t.x(0), 0xFFFF_FFFF_FFFF_FF80);
    t.cpu.x[1] = 0x8000;
    t.ok(&["sxth x0, w1"]);
    assert_eq!(t.x(0), 0xFFFF_FFFF_FFFF_8000);
    t.cpu.x[1] = 0x8000_0000;
    t.ok(&["sxtw x0, w1"]);
    assert_eq!(t.x(0), 0xFFFF_FFFF_8000_0000);
}

#[test]
fn extract_and_rotate() {
    let mut t = T::new();
    t.cpu.x[1] = 0x1111_2222_3333_4444;
    t.cpu.x[2] = 0x5555_6666_7777_8888;
    t.ok(&["extr x0, x1, x2, #12"]);
    assert_eq!(t.x(0), (0x1111_2222_3333_4444u64 << 52) | (0x5555_6666_7777_8888u64 >> 12));
    t.cpu.x[1] = 0x1234_5678_9ABC_DEF1;
    t.ok(&["ror x0, x1, #12"]);
    assert_eq!(t.x(0), 0x1234_5678_9ABC_DEF1u64.rotate_right(12));
    t.cpu.x[1] = 0x8000_0001;
    t.ok(&["ror w0, w1, #3"]);
    assert_eq!(t.x(0), u64::from(0x8000_0001u32.rotate_right(3)));
}

#[test]
fn conditional_compare_and_select() {
    let mut t = T::new();
    t.cpu.x[1] = 5;
    t.cpu.x[2] = 5;
    t.cpu.nzcv = NZCV_Z;
    t.ok(&["ccmp x1, x2, #4, eq"]);
    assert_eq!(t.flags(), nzcv(false, true, true, false), "condition true: compare 5 - 5");
    t.cpu.nzcv = 0;
    t.ok(&["ccmp x1, x2, #4, eq"]);
    assert_eq!(t.flags(), NZCV_Z, "condition false: the immediate flags");
    t.cpu.nzcv = 0;
    t.ok(&["ccmn x1, #5, #2, ne"]);
    assert_eq!(t.flags(), 0, "5 + 5 sets no flag");
    t.cpu.x[1] = u64::MAX;
    t.ok(&["ccmn x1, #5, #2, ne"]);
    assert_eq!(t.flags(), nzcv(false, false, true, false), "-1 + 5 carries");
    t.cpu.x[1] = 0xFFFF_FFFF;
    t.cpu.nzcv = NZCV_C;
    t.ok(&["ccmp w1, #31, #8, hi"]);
    assert_eq!(t.flags(), nzcv(true, false, true, false), "0xffffffff - 31 = 0xffffffe0: negative, carry");

    t.cpu.x[1] = 10;
    t.cpu.x[2] = 20;
    t.cpu.nzcv = NZCV_N; // N != V: lt
    t.ok(&["csel x0, x1, x2, lt"]);
    assert_eq!(t.x(0), 10);
    t.ok(&["csinc x0, x1, x2, ge"]);
    assert_eq!(t.x(0), 21);
    t.ok(&["csinv x0, x1, x2, eq"]);
    assert_eq!(t.x(0), !20u64);
    t.ok(&["csneg w0, w1, w2, ne"]);
    assert_eq!(t.x(0), 10, "ne holds with Z clear");
    t.cpu.nzcv = NZCV_Z;
    t.ok(&["csneg w0, w1, w2, ne"]);
    assert_eq!(t.x(0), 0xFFFF_FFEC, "-20 in 32 bits");
    t.ok(&["cset x0, eq"]);
    assert_eq!(t.x(0), 1);
    t.ok(&["cinc x0, x1, ne"]);
    assert_eq!(t.x(0), 10);
}

#[test]
fn multiply_and_divide() {
    let mut t = T::new();
    t.cpu.x[1] = 6;
    t.cpu.x[2] = 7;
    t.cpu.x[3] = 100;
    t.ok(&["madd x0, x1, x2, x3"]);
    assert_eq!(t.x(0), 142);
    t.ok(&["msub x0, x1, x2, x3"]);
    assert_eq!(t.x(0), 58);
    t.cpu.x[1] = 0xFFFF_FFFF;
    t.cpu.x[2] = 2;
    t.ok(&["mul w0, w1, w2"]);
    assert_eq!(t.x(0), 0xFFFF_FFFE, "32-bit wrap, upper half zero");
    t.cpu.x[1] = 0xFFFF_FFFF; // -1 as a word
    t.cpu.x[2] = 3;
    t.cpu.x[3] = 10;
    t.ok(&["smaddl x0, w1, w2, x3"]);
    assert_eq!(t.x(0), 7);
    t.ok(&["smsubl x0, w1, w2, x3"]);
    assert_eq!(t.x(0), 13);
    t.ok(&["umaddl x0, w1, w2, x3"]);
    assert_eq!(t.x(0), 0xFFFF_FFFFu64 * 3 + 10);
    t.ok(&["umsubl x0, w1, w2, x3"]);
    assert_eq!(t.x(0), 10u64.wrapping_sub(0xFFFF_FFFFu64 * 3));
    t.ok(&["smull x0, w1, w2"]);
    assert_eq!(t.x(0), (-3i64) as u64);
    t.cpu.x[1] = 0x8000_0000_0000_0000;
    t.cpu.x[2] = 2;
    t.ok(&["smulh x0, x1, x2"]);
    assert_eq!(t.x(0), u64::MAX, "-2^63 * 2 = -2^64: high half is -1");
    t.ok(&["umulh x0, x1, x2"]);
    assert_eq!(t.x(0), 1);

    // Division edge cases.
    t.cpu.x[1] = 100;
    t.cpu.x[2] = 0;
    t.ok(&["udiv x0, x1, x2"]);
    assert_eq!(t.x(0), 0, "divide by zero is 0");
    t.ok(&["sdiv x0, x1, x2"]);
    assert_eq!(t.x(0), 0);
    t.cpu.x[1] = 0x8000_0000_0000_0000;
    t.cpu.x[2] = u64::MAX;
    t.ok(&["sdiv x0, x1, x2"]);
    assert_eq!(t.x(0), 0x8000_0000_0000_0000, "INT_MIN / -1 is INT_MIN");
    t.cpu.x[1] = 0x8000_0000;
    t.cpu.x[2] = 0xFFFF_FFFF;
    t.ok(&["sdiv w0, w1, w2"]);
    assert_eq!(t.x(0), 0x8000_0000);
    t.cpu.x[1] = (-7i64) as u64;
    t.cpu.x[2] = 2;
    t.ok(&["sdiv x0, x1, x2"]);
    assert_eq!(t.x(0) as i64, -3, "rounds towards zero");
    t.ok(&["udiv x0, x1, x2"]);
    assert_eq!(t.x(0), ((-7i64) as u64) / 2);
}

#[test]
fn variable_shifts_and_bit_operations() {
    let mut t = T::new();
    t.cpu.x[1] = 0x8000_0000_0000_0001;
    t.cpu.x[2] = 65; // taken modulo 64
    t.ok(&["lsl x0, x1, x2"]);
    assert_eq!(t.x(0), 2);
    t.ok(&["lsr x0, x1, x2"]);
    assert_eq!(t.x(0), 0x4000_0000_0000_0000);
    t.ok(&["asr x0, x1, x2"]);
    assert_eq!(t.x(0), 0xC000_0000_0000_0000);
    t.ok(&["ror x0, x1, x2"]);
    assert_eq!(t.x(0), 0xC000_0000_0000_0000);
    t.cpu.x[1] = 0x1_8000_0001;
    t.cpu.x[2] = 33; // modulo 32
    t.ok(&["lsr w0, w1, w2"]);
    assert_eq!(t.x(0), 0x4000_0000);

    t.cpu.x[1] = 0x0000_0000_0010_0000;
    t.ok(&["clz x0, x1"]);
    assert_eq!(t.x(0), 43);
    t.ok(&["clz w0, w1"]);
    assert_eq!(t.x(0), 11);
    t.cpu.x[1] = u64::MAX;
    t.ok(&["cls x0, x1"]);
    assert_eq!(t.x(0), 63);
    t.cpu.x[1] = 0xFFFF_FFFF_0000_0000;
    t.ok(&["cls x0, x1"]);
    assert_eq!(t.x(0), 31);
    t.cpu.x[1] = 0x0123_4567_89AB_CDEF;
    t.ok(&["rbit x0, x1"]);
    assert_eq!(t.x(0), 0x0123_4567_89AB_CDEFu64.reverse_bits());
    t.ok(&["rev x0, x1"]);
    assert_eq!(t.x(0), 0xEFCD_AB89_6745_2301);
    t.ok(&["rev w0, w1"]);
    assert_eq!(t.x(0), 0xEFCD_AB89);
    t.ok(&["rev16 x0, x1"]);
    assert_eq!(t.x(0), 0x2301_6745_AB89_EFCD);
    t.ok(&["rev32 x0, x1"]);
    assert_eq!(t.x(0), 0x6745_2301_EFCD_AB89);
}

// ---------------------------------------------------------------------
// Branches
// ---------------------------------------------------------------------

#[test]
fn branches() {
    let mut t = T::new();
    t.ok(&["b #0x100"]);
    assert_eq!(t.cpu.pc, CODE + 0x100);
    t.ok(&["b #-8"]);
    assert_eq!(t.cpu.pc, CODE.wrapping_sub(8));
    t.ok(&["bl #0x40"]);
    assert_eq!(t.cpu.pc, CODE + 0x40);
    assert_eq!(t.x(30), CODE + 4);

    t.cpu.nzcv = NZCV_Z;
    t.ok(&["b.eq #0x20"]);
    assert_eq!(t.cpu.pc, CODE + 0x20);
    t.ok(&["b.ne #-0x20"]);
    assert_eq!(t.cpu.pc, CODE + 4, "not taken falls through");

    t.cpu.x[1] = 0;
    t.ok(&["cbz x1, #0x10"]);
    assert_eq!(t.cpu.pc, CODE + 0x10);
    t.cpu.x[2] = 0x1_0000_0000; // low word is zero
    t.ok(&["cbnz w2, #-0x10"]);
    assert_eq!(t.cpu.pc, CODE + 4);
    t.cpu.x[1] = 1 << 40;
    t.ok(&["tbnz x1, #40, #-0x8"]);
    assert_eq!(t.cpu.pc, CODE.wrapping_sub(8));
    t.ok(&["tbz x1, #3, #0x8"]);
    assert_eq!(t.cpu.pc, CODE + 8);

    t.cpu.x[1] = 0x4000;
    t.ok(&["br x1"]);
    assert_eq!(t.cpu.pc, 0x4000);
    t.cpu.x[2] = 0x5000;
    t.ok(&["blr x2"]);
    assert_eq!(t.cpu.pc, 0x5000);
    assert_eq!(t.x(30), CODE + 4);
    t.cpu.x[30] = 0x6000;
    t.ok(&["ret"]);
    assert_eq!(t.cpu.pc, 0x6000);
    t.cpu.x[5] = 0x7000;
    t.ok(&["ret x5"]);
    assert_eq!(t.cpu.pc, 0x7000);
    // BLR with the link register as the target reads it before writing it.
    t.cpu.x[30] = 0x8000;
    let w = 0xD63F_03C0; // blr x30
    t.run_words(&[w]);
    assert_eq!(t.cpu.pc, 0x8000);
    assert_eq!(t.x(30), CODE + 4);
}

#[test]
fn unaligned_pc_faults() {
    let mut t = T::new();
    t.cpu.pc = CODE + 2;
    let mut mem = SliceMemory::new(CODE, &mut t.mem);
    assert_eq!(
        t.cpu.step(&mut mem),
        Outcome::Fault(MemFault { addr: CODE + 2, access: Access::Fetch, alignment: true })
    );
    t.cpu.pc = 0x10_0000;
    let mut mem = SliceMemory::new(CODE, &mut t.mem);
    assert_eq!(
        t.cpu.step(&mut mem),
        Outcome::Fault(MemFault { addr: 0x10_0000, access: Access::Fetch, alignment: false })
    );
}

// ---------------------------------------------------------------------
// System
// ---------------------------------------------------------------------

#[test]
fn svc_and_brk() {
    let mut t = T::new();
    assert_eq!(t.run(&["svc #0x80"]), Outcome::Svc(0x80));
    assert_eq!(t.cpu.pc, CODE + 4, "PC is past the SVC");
    assert_eq!(t.run(&["brk #0x1234"]), Outcome::Brk(0x1234));
    assert_eq!(t.cpu.pc, CODE, "PC stays at the BRK");
}

#[test]
fn hints_are_no_ops() {
    let mut t = T::new();
    t.cpu.x[30] = 0x1234;
    for h in ["nop", "yield", "bti c", "paciasp", "autiasp"] {
        t.ok(&[h]);
        assert_eq!(t.x(30), 0x1234, "{h} must not change LR");
        assert_eq!(t.cpu.pc, CODE + 4);
    }
    // Any hint number is a no-op, including ones nothing defines.
    t.run_words(&[0xD503_201F | 100 << 5]);
    assert_eq!(t.cpu.pc, CODE + 4);
}

#[test]
fn system_registers() {
    let mut t = T::new();
    t.cpu.x[1] = 0xF000_0000;
    t.ok(&["msr nzcv, x1"]);
    assert_eq!(t.flags(), 0xF000_0000);
    t.ok(&["mrs x0, nzcv"]);
    assert_eq!(t.x(0), 0xF000_0000);
    t.cpu.x[1] = u64::MAX;
    t.ok(&["msr fpcr, x1", "mrs x0, fpcr"]);
    assert_eq!(t.x(0), 0x07C8_0000, "only the implemented bits are kept");
    t.ok(&["msr fpsr, x1", "mrs x0, fpsr"]);
    assert_eq!(t.x(0), 0x0800_009F);
    t.cpu.x[1] = 0xCAFE;
    t.ok(&["msr tpidr_el0, x1", "mrs x0, tpidr_el0"]);
    assert_eq!(t.x(0), 0xCAFE);
    t.cpu.tpidrro_el0 = 77;
    t.ok(&["mrs x0, tpidrro_el0"]);
    assert_eq!(t.x(0), 77);
    t.cpu.cntvct = 1000;
    t.ok(&["mrs x0, cntvct_el0", "mrs x1, cntvct_el0"]);
    assert_eq!((t.x(0), t.x(1)), (1000, 1001), "each read advances the counter");
    t.ok(&["mrs x0, cntfrq_el0"]);
    assert_eq!(t.x(0), 24_000_000);
    t.ok(&["mrs x0, dczid_el0"]);
    assert_eq!(t.x(0), 4);
    // The counters are read-only: a write is undefined.
    let w = 0xD51B_E040; // msr cntvct_el0, x0
    assert!(matches!(t.run_words(&[w]), Outcome::Undefined(x) if x == w));
}

#[test]
fn barriers_and_dc_zva() {
    let mut t = T::new();
    for b in ["dmb ish", "dsb sy", "isb"] {
        t.ok(&[b]);
    }
    for i in 0..128 {
        t.mem[0x1000 + i] = 0xFF;
    }
    t.cpu.x[1] = DATA + 0x47;
    t.ok(&["dc zva, x1"]);
    for i in 0..128u64 {
        let expect = if (0x40..0x80).contains(&i) { 0 } else { 0xFF };
        assert_eq!(t.r8(DATA + i), expect, "byte {i:#x}: ZVA zeroes the 64-byte block holding the address");
    }
}

// ---------------------------------------------------------------------
// Loads and stores
// ---------------------------------------------------------------------

#[test]
fn load_store_addressing_modes() {
    let mut t = T::new();
    t.cpu.x[1] = DATA;
    t.w64(DATA + 8, 0x1122_3344_5566_7788);
    t.ok(&["ldr x0, [x1, #8]"]);
    assert_eq!(t.x(0), 0x1122_3344_5566_7788);
    t.ok(&["ldr w0, [x1, #8]"]);
    assert_eq!(t.x(0), 0x5566_7788);
    t.ok(&["ldrb w0, [x1, #9]"]);
    assert_eq!(t.x(0), 0x77);
    t.ok(&["ldrh w0, [x1, #10]"]);
    assert_eq!(t.x(0), 0x5566);
    t.cpu.x[0] = 0xAABB_CCDD_EEFF_1122;
    t.ok(&["str x0, [x1, #16]"]);
    assert_eq!(t.r64(DATA + 16), 0xAABB_CCDD_EEFF_1122);
    t.ok(&["strb w0, [x1, #32]", "strh w0, [x1, #34]", "str w0, [x1, #36]"]);
    assert_eq!(t.r8(DATA + 32), 0x22);
    assert_eq!(t.r32(DATA + 36), 0xEEFF_1122);
    // Unscaled.
    t.w64(DATA + 0x40, 0x0807_0605_0403_0201);
    t.cpu.x[1] = DATA + 0x48;
    t.ok(&["ldur x0, [x1, #-8]"]);
    assert_eq!(t.x(0), 0x0807_0605_0403_0201);
    t.cpu.x[0] = 0xDDCC_BBAA;
    t.ok(&["stur w0, [x1, #-3]"]);
    assert_eq!(t.r32(DATA + 0x45), 0xDDCC_BBAA);
}

#[test]
fn sign_extending_loads() {
    let mut t = T::new();
    t.cpu.x[1] = DATA;
    t.write(DATA, &[0x80, 0xFF, 0x00, 0x80, 0x00, 0x00, 0x00, 0x80]);
    t.ok(&["ldrsb x0, [x1]"]);
    assert_eq!(t.x(0), 0xFFFF_FFFF_FFFF_FF80);
    t.ok(&["ldrsb w0, [x1]"]);
    assert_eq!(t.x(0), 0xFFFF_FF80, "W destination: upper half zero");
    t.ok(&["ldrsh x0, [x1, #2]"]);
    assert_eq!(t.x(0), 0xFFFF_FFFF_FFFF_8000);
    t.ok(&["ldrsw x0, [x1, #4]"]);
    assert_eq!(t.x(0), 0xFFFF_FFFF_8000_0000);
}

#[test]
fn writeback_modes() {
    let mut t = T::new();
    t.cpu.x[1] = DATA + 0x100;
    t.w64(DATA + 0x108, 0x55);
    t.ok(&["ldr x0, [x1, #8]!"]);
    assert_eq!((t.x(0), t.x(1)), (0x55, DATA + 0x108));
    t.w64(DATA + 0x100, 0x66);
    t.ok(&["ldr x0, [x1], #-8"]);
    assert_eq!((t.x(0), t.x(1)), (0x55, DATA + 0x100), "post-index loads from the old address");
    t.cpu.x[0] = 0x77;
    t.ok(&["str x0, [x1, #-16]!"]);
    assert_eq!(t.r64(DATA + 0xF0), 0x77);
    assert_eq!(t.x(1), DATA + 0xF0);
    t.ok(&["str x0, [x1], #8"]);
    assert_eq!(t.x(1), DATA + 0xF8);
    // With SP as the base.
    t.cpu.sp = DATA + 0x200;
    t.cpu.x[0] = 1;
    t.cpu.x[1] = 2;
    t.ok(&["stp x0, x1, [sp, #-16]!"]);
    assert_eq!(t.cpu.sp, DATA + 0x1F0);
    assert_eq!((t.r64(DATA + 0x1F0), t.r64(DATA + 0x1F8)), (1, 2));
    t.ok(&["ldp x2, x3, [sp], #16"]);
    assert_eq!((t.x(2), t.x(3)), (1, 2));
    assert_eq!(t.cpu.sp, DATA + 0x200);
}

#[test]
fn register_offset_with_extend_and_shift() {
    let mut t = T::new();
    t.cpu.x[1] = DATA;
    t.cpu.x[2] = 3;
    for i in 0..8u64 {
        t.w64(DATA + 8 * i, 100 + i);
    }
    t.ok(&["ldr x0, [x1, x2]"]);
    assert_eq!(t.x(0), 101 << 40, "byte offset 3 reads across words");
    t.ok(&["ldr x0, [x1, x2, lsl #3]"]);
    assert_eq!(t.x(0), 103);
    t.cpu.x[2] = 0xFFFF_FFFF_0000_0002;
    t.ok(&["ldr w0, [x1, w2, uxtw #2]"]);
    assert_eq!(t.x(0), u64::from(t.r32(DATA + 8)));
    t.cpu.x[1] = DATA + 16;
    t.cpu.x[2] = 0x0000_0000_FFFF_FFFE; // -2 as a word
    t.ok(&["ldr w0, [x1, w2, sxtw]"]);
    assert_eq!(t.x(0), u64::from(t.r32(DATA + 14)));
}

#[test]
fn literal_loads_and_prefetch() {
    let mut t = T::new();
    t.w64(CODE + 0x20, 0x1122_3344_5566_7788);
    t.ok(&["ldr x0, #0x20"]);
    assert_eq!(t.x(0), 0x1122_3344_5566_7788);
    t.w64(CODE + 0x28, 0xFFFF_FFFF_8000_0000);
    t.ok(&["ldrsw x0, #0x28"]);
    assert_eq!(t.x(0), 0xFFFF_FFFF_8000_0000);
    t.cpu.x[1] = 0xDEAD_0000_0000_0000; // an unmapped address: PRFM must not fault
    t.ok(&["prfm pldl1keep, [x1]"]);
    t.ok(&["ldr s0, #0x10"]);
}

#[test]
fn load_store_pairs() {
    let mut t = T::new();
    t.cpu.x[2] = DATA;
    t.w64(DATA, 0x1111);
    t.w64(DATA + 8, 0x2222);
    t.w64(DATA + 16, 0xFFFF_FFFF_8000_0001);
    t.ok(&["ldp x0, x1, [x2]"]);
    assert_eq!((t.x(0), t.x(1)), (0x1111, 0x2222));
    t.ok(&["ldp w0, w1, [x2, #16]"]);
    assert_eq!((t.x(0), t.x(1)), (0x8000_0001, 0xFFFF_FFFF));
    t.ok(&["ldpsw x0, x1, [x2, #16]"]);
    assert_eq!((t.x(0), t.x(1)), (0xFFFF_FFFF_8000_0001, u64::MAX));
    t.cpu.x[0] = 0xA;
    t.cpu.x[1] = 0xB;
    t.ok(&["stnp w0, w1, [x2, #32]"]);
    assert_eq!((t.r32(DATA + 32), t.r32(DATA + 36)), (0xA, 0xB));
    t.ok(&["ldnp x0, x1, [x2]"]);
    assert_eq!((t.x(0), t.x(1)), (0x1111, 0x2222));
}

#[test]
fn faults_leave_the_state_alone() {
    let mut t = T::new();
    t.cpu.x[1] = 0x10_0000;
    t.cpu.x[0] = 99;
    let r = t.run(&["ldr x0, [x1, #8]!"]);
    assert_eq!(r, Outcome::Fault(MemFault { addr: 0x10_0008, access: Access::Read, alignment: false }));
    assert_eq!(t.cpu.pc, CODE, "PC stays at the faulting instruction");
    assert_eq!((t.x(0), t.x(1)), (99, 0x10_0000), "no register changes, no writeback");
    let r = t.run(&["str x0, [x1]"]);
    assert_eq!(r, Outcome::Fault(MemFault { addr: 0x10_0000, access: Access::Write, alignment: false }));
    // A pair that runs off the end of memory reports the first missing address.
    t.cpu.x[1] = CODE + SIZE as u64 - 8;
    let r = t.run(&["ldp x2, x3, [x1]"]);
    assert_eq!(r, Outcome::Fault(MemFault { addr: CODE + SIZE as u64, access: Access::Read, alignment: false }));
    // Misaligned ordinary accesses work.
    t.cpu.x[1] = DATA + 1;
    t.ok(&["ldr x0, [x1]"]);
    t.ok(&["ldp x2, x3, [x1]"]);
}

#[test]
fn exclusives() {
    let mut t = T::new();
    t.cpu.x[1] = DATA;
    t.w64(DATA, 5);
    t.cpu.x[0] = 9;
    t.ok(&["ldxr x3, [x1]", "stxr w2, x0, [x1]"]);
    assert_eq!((t.x(3), t.x(2), t.r64(DATA)), (5, 0, 9), "success writes 0 to the status register");
    // A second store-exclusive has no monitor.
    t.cpu.x[0] = 11;
    t.ok(&["stxr w2, x0, [x1]"]);
    assert_eq!((t.x(2), t.r64(DATA)), (1, 9));
    // CLREX closes the monitor.
    t.ok(&["ldxr x3, [x1]", "clrex", "stxr w2, x0, [x1]"]);
    assert_eq!((t.x(2), t.r64(DATA)), (1, 9));
    // A different address fails.
    t.cpu.x[4] = DATA + 8;
    t.ok(&["ldxr x3, [x1]"]);
    let w = asm("stxr w2, x0, [x1]") & !(0x1F << 5) | 4 << 5;
    assert_eq!(t.run_words(&[w]), Outcome::Continue);
    assert_eq!(t.x(2), 1);
    // The byte and pair forms.
    t.write(DATA + 16, &[1, 2, 3, 4, 5, 6, 7, 8, 9, 10, 11, 12, 13, 14, 15, 16]);
    t.cpu.x[1] = DATA + 16;
    t.ok(&["ldxp x3, x4, [x1]"]);
    assert_eq!((t.x(3), t.x(4)), (0x0807_0605_0403_0201, 0x100F_0E0D_0C0B_0A09));
    t.cpu.x[5] = 0xAA;
    t.cpu.x[6] = 0xBB;
    let w = asm("stxp w3, x0, x1, [x2]") & !(0x1F | 0x1F << 5 | 0x1F << 10) | 5 | 1 << 5 | 6 << 10;
    t.run_words(&[w]);
    assert_eq!(t.x(3), 0);
    assert_eq!((t.r64(DATA + 16), t.r64(DATA + 24)), (0xAA, 0xBB));
    t.ok(&["ldxrb w0, [x1]"]);
    assert_eq!(t.x(0), 0xAA);
}

#[test]
fn exclusive_and_atomic_accesses_must_be_aligned() {
    let mut t = T::new();
    t.cpu.x[1] = DATA + 4;
    let r = t.run(&["ldxr x0, [x1]"]);
    assert_eq!(r, Outcome::Fault(MemFault { addr: DATA + 4, access: Access::Read, alignment: true }));
    assert!(matches!(t.run(&["ldar x0, [x1]"]), Outcome::Fault(MemFault { alignment: true, .. })));
    assert!(matches!(t.run(&["ldadd x0, x1, [x1]"]), Outcome::Fault(MemFault { alignment: true, .. })));
    assert!(matches!(t.run(&["cas x0, x1, [x1]"]), Outcome::Fault(MemFault { alignment: true, .. })));
    // CASP needs the whole pair aligned.
    t.cpu.x[4] = DATA + 8;
    assert!(matches!(t.run(&["casp x0, x1, x2, x3, [x4]"]), Outcome::Fault(MemFault { alignment: true, .. })));
    t.ok(&["ldar w0, [x1]"]);
}

#[test]
fn lse_atomics() {
    let mut t = T::new();
    t.cpu.x[2] = DATA;
    t.w64(DATA, 10);
    t.cpu.x[0] = 5;
    t.ok(&["ldadd x0, x1, [x2]"]);
    assert_eq!((t.x(1), t.r64(DATA)), (10, 15));
    t.ok(&["ldclr x0, x1, [x2]"]);
    assert_eq!((t.x(1), t.r64(DATA)), (15, 10), "15 AND NOT 5");
    t.ok(&["ldeor x0, x1, [x2]"]);
    assert_eq!((t.x(1), t.r64(DATA)), (10, 15));
    t.cpu.x[0] = 0x30;
    t.ok(&["ldset x0, x1, [x2]"]);
    assert_eq!(t.r64(DATA), 0x3F);
    t.ok(&["swp x0, x1, [x2]"]);
    assert_eq!((t.x(1), t.r64(DATA)), (0x3F, 0x30));
    // Signed and unsigned maxima at 32 bits.
    t.w64(DATA, 0xFFFF_FFFF); // -1 as a word
    t.cpu.x[0] = 1;
    t.ok(&["ldsmax w0, w1, [x2]"]);
    assert_eq!((t.x(1), t.r32(DATA)), (0xFFFF_FFFF, 1), "signed max of -1 and 1 is 1");
    t.w64(DATA, 0xFFFF_FFFF);
    t.ok(&["ldumin w0, w1, [x2]"]);
    assert_eq!(t.r32(DATA), 1);
    t.w64(DATA, 0xFFFF_FFFF);
    t.ok(&["ldsmin x0, x1, [x2]"]);
    assert_eq!(t.r64(DATA), 1, "the 64-bit value 0xffffffff is positive");
    // The ST alias (Rt = XZR) writes memory and no register.
    t.w64(DATA, 7);
    t.cpu.x[0] = 1;
    t.cpu.x[1] = DATA;
    t.ok(&["stadd w0, [x1]"]);
    assert_eq!((t.x(1), t.r32(DATA)), (DATA, 8));
    // CAS: success, then failure; Rs gets the old value either way.
    t.cpu.x[1] = DATA;
    t.w64(DATA, 20);
    t.cpu.x[3] = 20;
    t.cpu.x[4] = 30;
    t.ok(&["cas x3, x4, [x1]"]);
    assert_eq!((t.x(3), t.r64(DATA)), (20, 30));
    t.cpu.x[3] = 20;
    t.cpu.x[4] = 40;
    t.ok(&["cas x3, x4, [x1]"]);
    assert_eq!((t.x(3), t.r64(DATA)), (30, 30), "mismatch: memory unchanged, Rs = old");
    // CASB compares one byte and zero-extends the old value.
    t.w64(DATA, 0x1234);
    t.cpu.x[3] = 0xFF34;
    t.cpu.x[4] = 0xAB;
    t.ok(&["casb w3, w4, [x1]"]);
    assert_eq!((t.x(3), t.r64(DATA)), (0x34, 0x12AB));
    // CASP on two doublewords.
    t.write(DATA + 16, &[1, 0, 0, 0, 0, 0, 0, 0, 2, 0, 0, 0, 0, 0, 0, 0]);
    t.cpu.x[4] = DATA + 16;
    t.cpu.x[0] = 1;
    t.cpu.x[1] = 2;
    t.cpu.x[2] = 7;
    t.cpu.x[3] = 8;
    t.ok(&["casp x0, x1, x2, x3, [x4]"]);
    assert_eq!((t.r64(DATA + 16), t.r64(DATA + 24)), (7, 8));
    assert_eq!((t.x(0), t.x(1)), (1, 2));
}

#[test]
fn memory_trait_cas_hook_is_used() {
    struct Spy {
        data: Vec<u8>,
        cas_calls: u32,
    }
    impl Memory for Spy {
        fn read(&mut self, addr: u64, buf: &mut [u8]) -> Result<(), MemFault> {
            buf.copy_from_slice(&self.data[addr as usize..addr as usize + buf.len()]);
            Ok(())
        }
        fn write(&mut self, addr: u64, d: &[u8]) -> Result<(), MemFault> {
            self.data[addr as usize..addr as usize + d.len()].copy_from_slice(d);
            Ok(())
        }
        fn cas(&mut self, addr: u64, size: usize, expected: u128, new: u128) -> Result<u128, MemFault> {
            self.cas_calls += 1;
            let old = self.read_uint(addr, size)?;
            if old == expected {
                self.write_uint(addr, size, new)?;
            }
            Ok(old)
        }
    }
    let mut mem = Spy { data: std::vec![0; 256], cas_calls: 0 };
    let words = [asm("ldxr x3, [x1]"), asm("stxr w2, x0, [x1]"), asm("ldadd x0, x4, [x1]")];
    for (i, w) in words.iter().enumerate() {
        mem.data[4 * i..4 * i + 4].copy_from_slice(&w.to_le_bytes());
    }
    let mut cpu = Cpu::new();
    cpu.x[1] = 128;
    cpu.x[0] = 3;
    for _ in 0..3 {
        assert_eq!(cpu.step(&mut mem), Outcome::Continue);
    }
    assert_eq!(mem.cas_calls, 2, "STXR and LDADD go through the hook");
    assert_eq!(mem.read_uint(128, 8).unwrap(), 6);
}

// ---------------------------------------------------------------------
// Floating point
// ---------------------------------------------------------------------

fn setd(t: &mut T, r: usize, v: f64) {
    t.cpu.v[r] = u128::from(v.to_bits());
}

fn getd(t: &T, r: usize) -> f64 {
    f64::from_bits(t.cpu.v[r] as u64)
}

#[test]
fn fp_moves_and_immediates() {
    let mut t = T::new();
    t.ok(&["fmov d0, #1.5"]);
    assert_eq!(getd(&t, 0), 1.5);
    t.ok(&["fmov s0, #-0.125"]);
    assert_eq!(t.cpu.v[0], u128::from((-0.125f32).to_bits()), "scalar writes zero the rest of the register");
    t.cpu.v[1] = u128::MAX;
    t.ok(&["fmov d0, d1"]);
    assert_eq!(t.cpu.v[0], u128::from(u64::MAX));
    t.cpu.x[1] = 0x1122_3344_5566_7788;
    t.ok(&["fmov d0, x1"]);
    assert_eq!(t.cpu.v[0], 0x1122_3344_5566_7788);
    t.ok(&["fmov x0, d0"]);
    assert_eq!(t.x(0), 0x1122_3344_5566_7788);
    t.ok(&["fmov v0.d[1], x1"]);
    assert_eq!(t.cpu.v[0], (0x1122_3344_5566_7788u128 << 64) | 0x1122_3344_5566_7788);
    t.cpu.x[0] = 0;
    t.ok(&["fmov x0, v0.d[1]"]);
    assert_eq!(t.x(0), 0x1122_3344_5566_7788);
    t.cpu.x[1] = 0xFFFF_FFFF_3F80_0000;
    t.ok(&["fmov s0, w1"]);
    assert_eq!(t.cpu.v[0], 0x3F80_0000);
    t.cpu.x[0] = u64::MAX;
    t.ok(&["fmov w0, s0"]);
    assert_eq!(t.x(0), 0x3F80_0000);
}

#[test]
fn fp_arithmetic_and_special_values() {
    let mut t = T::new();
    setd(&mut t, 1, 1.5);
    setd(&mut t, 2, 0.25);
    t.ok(&["fadd d0, d1, d2"]);
    assert_eq!(getd(&t, 0), 1.75);
    t.ok(&["fmul d0, d1, d2"]);
    assert_eq!(getd(&t, 0), 0.375);
    t.ok(&["fdiv d0, d1, d2"]);
    assert_eq!(getd(&t, 0), 6.0);
    t.ok(&["fnmul d0, d1, d2"]);
    assert_eq!(getd(&t, 0), -0.375);
    setd(&mut t, 1, 2.0);
    t.ok(&["fsqrt d0, d1"]);
    assert_eq!(getd(&t, 0), core::f64::consts::SQRT_2);
    assert_eq!(t.cpu.fpsr, 0);

    // x / 0 is an infinity and raises DZC; 0 / 0 is the default NaN and raises IOC.
    setd(&mut t, 1, 1.0);
    setd(&mut t, 2, 0.0);
    t.ok(&["fdiv d0, d1, d2"]);
    assert_eq!(getd(&t, 0), f64::INFINITY);
    assert_eq!(t.cpu.fpsr, 2);
    t.cpu.fpsr = 0;
    t.ok(&["fdiv d0, d2, d2"]);
    assert_eq!(t.cpu.v[0] as u64, 0x7FF8_0000_0000_0000, "the default NaN is positive");
    assert_eq!(t.cpu.fpsr, 1);

    // A signalling NaN operand is quieted (payload kept) and raises IOC; a quiet NaN raises nothing.
    t.cpu.fpsr = 0;
    t.cpu.v[1] = 0x7FF0_0000_0000_0001;
    setd(&mut t, 2, 1.0);
    t.ok(&["fadd d0, d1, d2"]);
    assert_eq!(t.cpu.v[0] as u64, 0x7FF8_0000_0000_0001);
    assert_eq!(t.cpu.fpsr, 1);
    t.cpu.fpsr = 0;
    t.cpu.v[1] = 0xFFF8_0000_0000_1234;
    t.ok(&["fadd d0, d2, d1"]);
    assert_eq!(t.cpu.v[0] as u64, 0xFFF8_0000_0000_1234, "the quiet NaN of the second operand passes through");
    assert_eq!(t.cpu.fpsr, 0);
    // Default-NaN mode (FPCR.DN) replaces every NaN result.
    t.cpu.fpcr = 1 << 25;
    t.ok(&["fadd d0, d2, d1"]);
    assert_eq!(t.cpu.v[0] as u64, 0x7FF8_0000_0000_0000);
    t.cpu.fpcr = 0;
    // inf - inf is invalid.
    setd(&mut t, 1, f64::INFINITY);
    t.ok(&["fsub d0, d1, d1"]);
    assert_eq!(t.cpu.v[0] as u64, 0x7FF8_0000_0000_0000);

    // Min and max: signed zeros, quiet NaNs for the NM forms.
    setd(&mut t, 1, 0.0);
    setd(&mut t, 2, -0.0);
    t.ok(&["fmax d0, d1, d2"]);
    assert_eq!(t.cpu.v[0] as u64, 0);
    t.ok(&["fmin d0, d1, d2"]);
    assert_eq!(t.cpu.v[0] as u64, 0x8000_0000_0000_0000);
    t.cpu.v[1] = 0x7FF8_0000_0000_0000;
    setd(&mut t, 2, 3.0);
    t.ok(&["fmaxnm d0, d1, d2"]);
    assert_eq!(getd(&t, 0), 3.0);
    t.ok(&["fmax d0, d1, d2"]);
    assert_eq!(t.cpu.v[0] as u64, 0x7FF8_0000_0000_0000);
}

#[test]
fn fp_fused_multiply_add_has_one_rounding() {
    let mut t = T::new();
    // (1 + 2^-30)^2 = 1 + 2^-29 + 2^-60; unfused rounding loses the 2^-60 term.
    let x = 1.0 + 2f64.powi(-30);
    setd(&mut t, 1, x);
    setd(&mut t, 2, x);
    setd(&mut t, 3, -(1.0 + 2f64.powi(-29)));
    t.ok(&["fmadd d0, d1, d2, d3"]);
    assert_eq!(getd(&t, 0), 2f64.powi(-60));
    setd(&mut t, 1, 2.0);
    setd(&mut t, 2, 3.0);
    setd(&mut t, 3, 10.0);
    t.ok(&["fmsub d0, d1, d2, d3"]);
    assert_eq!(getd(&t, 0), 4.0);
    t.ok(&["fnmadd d0, d1, d2, d3"]);
    assert_eq!(getd(&t, 0), -16.0);
    t.ok(&["fnmsub d0, d1, d2, d3"]);
    assert_eq!(getd(&t, 0), -4.0);
}

#[test]
fn fp_compare_and_select() {
    let mut t = T::new();
    setd(&mut t, 0, 1.0);
    setd(&mut t, 1, 2.0);
    t.ok(&["fcmp d0, d1"]);
    assert_eq!(t.flags(), nzcv(true, false, false, false));
    t.ok(&["fcmp d1, d0"]);
    assert_eq!(t.flags(), nzcv(false, false, true, false));
    t.ok(&["fcmp d0, d0"]);
    assert_eq!(t.flags(), nzcv(false, true, true, false));
    t.cpu.v[2] = 0x7FF8_0000_0000_0000;
    t.cpu.v[3] = 0x7FF0_0000_0000_0001;
    let w = asm("fcmp d0, d1") & !(0x1F << 16) & !(0x1F << 5) | 2 << 5 | 1 << 16;
    t.run_words(&[w]);
    assert_eq!(t.flags(), nzcv(false, false, true, true), "unordered: C and V");
    assert_eq!(t.cpu.fpsr, 0, "FCMP with a quiet NaN raises nothing");
    t.ok(&["fcmpe d0, d1"]);
    t.cpu.fpsr = 0;
    let w = asm("fcmpe d0, d1") & !(0x1F << 16) & !(0x1F << 5) | 2 << 5 | 1 << 16;
    t.run_words(&[w]);
    assert_eq!(t.cpu.fpsr, 1, "FCMPE with a quiet NaN raises invalid operation");
    t.cpu.fpsr = 0;
    t.ok(&["fcmp s0, #0.0"]);

    t.cpu.nzcv = NZCV_N;
    t.ok(&["fcsel d5, d0, d1, lt"]);
    assert_eq!(getd(&t, 5), 1.0);
    t.cpu.nzcv = nzcv(false, false, true, false);
    t.ok(&["fccmp d0, d1, #4, eq"]);
    assert_eq!(t.flags(), 4 << 28, "condition false: immediate flags");
}

#[test]
fn fp_conversions() {
    let mut t = T::new();
    setd(&mut t, 1, -1.5);
    t.ok(&["fcvtzs x0, d1"]);
    assert_eq!(t.x(0) as i64, -1);
    t.ok(&["fcvtns x0, d1"]);
    assert_eq!(t.x(0) as i64, -2, "ties to even");
    t.ok(&["fcvtas x0, d1"]);
    assert_eq!(t.x(0) as i64, -2, "ties away from zero");
    t.ok(&["fcvtps x0, d1"]);
    assert_eq!(t.x(0) as i64, -1);
    t.ok(&["fcvtms x0, d1"]);
    assert_eq!(t.x(0) as i64, -2);
    t.ok(&["fcvtzu x0, d1"]);
    assert_eq!(t.x(0), 0, "negative to unsigned saturates at 0");
    assert_eq!(t.cpu.fpsr, 1);
    t.cpu.fpsr = 0;
    setd(&mut t, 1, 1e30);
    t.ok(&["fcvtzs x0, d1"]);
    assert_eq!(t.x(0), i64::MAX as u64);
    t.ok(&["fcvtzs w0, d1"]);
    assert_eq!(t.x(0), 0x7FFF_FFFF);
    t.cpu.v[1] = 0x7FF8_0000_0000_0000;
    t.ok(&["fcvtzs x0, d1"]);
    assert_eq!(t.x(0), 0, "NaN converts to 0");
    setd(&mut t, 1, 2.5);
    t.ok(&["fcvtzs w0, d1, #4"]);
    assert_eq!(t.x(0), 40, "fixed point with 4 fraction bits");

    t.cpu.x[1] = (-5i64) as u64;
    t.ok(&["scvtf d0, x1"]);
    assert_eq!(getd(&t, 0), -5.0);
    t.cpu.x[1] = u64::MAX;
    t.ok(&["ucvtf d0, x1"]);
    assert_eq!(getd(&t, 0), 18446744073709551616.0);
    t.cpu.x[1] = 0x1_0000_0003;
    t.ok(&["scvtf s0, w1"]);
    assert_eq!(f32::from_bits(t.cpu.v[0] as u32), 3.0);
    t.cpu.x[1] = 0x100;
    t.ok(&["ucvtf d0, w1, #8"]);
    assert_eq!(getd(&t, 0), 1.0);

    setd(&mut t, 1, 1.0 + 2f64.powi(-40));
    t.ok(&["fcvt s0, d1"]);
    assert_eq!(f32::from_bits(t.cpu.v[0] as u32), 1.0);
    t.cpu.v[1] = u128::from(0x7F80_0001u32); // single sNaN
    t.ok(&["fcvt d0, s1"]);
    assert_eq!(t.cpu.v[0] as u64, 0x7FF8_0000_2000_0000, "payload kept, quiet bit set");

    setd(&mut t, 1, -2.5);
    t.ok(&["frintn d0, d1"]);
    assert_eq!(getd(&t, 0), -2.0);
    t.ok(&["frinta d0, d1"]);
    assert_eq!(getd(&t, 0), -3.0);
    t.ok(&["frintm d0, d1"]);
    assert_eq!(getd(&t, 0), -3.0);
    t.ok(&["frintz d0, d1"]);
    assert_eq!(getd(&t, 0), -2.0);
    setd(&mut t, 1, -0.25);
    t.ok(&["frintp d0, d1"]);
    assert_eq!(t.cpu.v[0] as u64, 0x8000_0000_0000_0000, "rounds to -0");
}

// ---------------------------------------------------------------------
// Advanced SIMD
// ---------------------------------------------------------------------

fn lanes8(v: u128) -> [u8; 16] {
    v.to_le_bytes()
}

#[test]
fn simd_immediates_and_lane_moves() {
    let mut t = T::new();
    t.ok(&["movi v0.16b, #0x5a"]);
    assert_eq!(t.cpu.v[0], u128::from_le_bytes([0x5A; 16]));
    t.ok(&["movi v0.2d, #0xff00ff00ff00ff00"]);
    assert_eq!(t.cpu.v[0], (0xFF00_FF00_FF00_FF00u128 << 64) | 0xFF00_FF00_FF00_FF00);
    t.ok(&["movi d0, #0xff00ff00ff00ff00"]);
    assert_eq!(t.cpu.v[0], 0xFF00_FF00_FF00_FF00);
    t.ok(&["movi v0.4s, #0x12, lsl #8"]);
    assert_eq!(
        t.cpu.v[0],
        u128::from_le_bytes([0x00, 0x12, 0, 0, 0x00, 0x12, 0, 0, 0x00, 0x12, 0, 0, 0x00, 0x12, 0, 0])
    );
    t.ok(&["mvni v0.4s, #0x12, lsl #16"]);
    assert_eq!(lanes8(t.cpu.v[0])[..4], (!0x0012_0000u32).to_le_bytes());
    t.cpu.v[0] = u128::MAX;
    t.ok(&["bic v0.8h, #0xf, lsl #8"]);
    assert_eq!(lanes8(t.cpu.v[0])[..4], [0xFF, 0xF0, 0xFF, 0xF0]);
    t.cpu.v[0] = 0;
    t.ok(&["orr v0.4s, #0x1"]);
    assert_eq!(lanes8(t.cpu.v[0])[..8], [1, 0, 0, 0, 1, 0, 0, 0]);
    t.ok(&["fmov v0.4s, #1.0"]);
    assert_eq!(t.cpu.v[0] as u32, 0x3F80_0000);

    t.cpu.x[1] = 0xABCD_EF01_2345_6789;
    t.ok(&["dup v0.16b, w1"]);
    assert_eq!(t.cpu.v[0], u128::from_le_bytes([0x89; 16]));
    t.ok(&["dup v0.2d, x1"]);
    assert_eq!(t.cpu.v[0], (u128::from(t.cpu.x[1]) << 64) | u128::from(t.cpu.x[1]));
    t.cpu.v[1] = 0x1111_1111_2222_2222_3333_3333_4444_4444;
    t.ok(&["dup v0.4s, v1.s[2]"]);
    assert_eq!(t.cpu.v[0], u128::from_le_bytes([0x22; 4].repeat(4).try_into().unwrap()));
    t.ok(&["dup s0, v1.s[2]"]);
    assert_eq!(t.cpu.v[0], 0x2222_2222, "the scalar form zeroes the rest");
    t.cpu.v[0] = 0;
    t.cpu.x[1] = 0xAAAA_BBBB;
    t.ok(&["ins v0.s[1], w1"]);
    assert_eq!(t.cpu.v[0], 0xAAAA_BBBBu128 << 32);
    t.cpu.v[1] = 0x0007_0006_0005_0004_0003_0002_0001_0000;
    t.ok(&["ins v0.h[3], v1.h[5]"]);
    assert_eq!((t.cpu.v[0] >> 48) as u16, 5);
    t.cpu.v[1] = 0x100F_0E0D_0C0B_0A09_0807_0605_0403_0201;
    t.ok(&["umov w0, v1.b[3]"]);
    assert_eq!(t.x(0), 4);
    t.ok(&["umov x0, v1.d[1]"]);
    assert_eq!(t.x(0), 0x100F_0E0D_0C0B_0A09);
    t.cpu.v[1] = 0x0000_0000_0000_0000_8000_0080_8000_0080;
    t.ok(&["smov x0, v1.b[7]"]);
    assert_eq!(t.x(0), 0xFFFF_FFFF_FFFF_FF80);
    t.ok(&["smov w0, v1.h[1]"]);
    assert_eq!(t.x(0), 0xFFFF_8000);
}

#[test]
fn simd_integer_and_logic() {
    let mut t = T::new();
    t.cpu.v[1] = u128::from_le_bytes([0xFF; 16]);
    t.cpu.v[2] = u128::from_le_bytes([1; 16]);
    t.ok(&["add v0.16b, v1.16b, v2.16b"]);
    assert_eq!(t.cpu.v[0], 0, "wraps per byte");
    t.cpu.v[1] = 0x0000_0001_0000_0001_0000_0001_0000_0001;
    t.cpu.v[2] = 0x0000_0002_0000_0002_0000_0002_0000_0002;
    t.ok(&["sub v0.4s, v1.4s, v2.4s"]);
    assert_eq!(t.cpu.v[0], u128::from_le_bytes([0xFF; 16]));
    t.ok(&["add v0.2d, v1.2d, v2.2d"]);
    assert_eq!(t.cpu.v[0], 0x0000_0003_0000_0003_0000_0003_0000_0003);
    t.cpu.v[1] = 0xF0F0;
    t.cpu.v[2] = 0xFF00;
    t.ok(&["and v0.8b, v1.8b, v2.8b"]);
    assert_eq!(t.cpu.v[0], 0xF000);
    t.cpu.v[0] = u128::MAX;
    t.ok(&["orr v0.8b, v1.8b, v2.8b"]);
    assert_eq!(t.cpu.v[0], 0xFFF0, "a 64-bit operation zeroes the upper half");
    t.ok(&["eor v0.16b, v1.16b, v2.16b"]);
    assert_eq!(t.cpu.v[0], 0x0FF0);
    t.ok(&["bic v0.16b, v1.16b, v2.16b"]);
    assert_eq!(t.cpu.v[0], 0x00F0);
    t.ok(&["orn v0.16b, v1.16b, v2.16b"]);
    assert_eq!(t.cpu.v[0] & 0xFFFF, 0xF0FF);
    // Bit select: Vd chooses between Vn and Vm.
    t.cpu.v[0] = 0x00FF;
    t.cpu.v[1] = 0xAAAA;
    t.cpu.v[2] = 0x5555;
    t.ok(&["bsl v0.16b, v1.16b, v2.16b"]);
    assert_eq!(t.cpu.v[0], 0x55AA);
    t.cpu.v[0] = 0x1234;
    t.cpu.v[2] = 0x00FF;
    t.ok(&["bit v0.16b, v1.16b, v2.16b"]);
    assert_eq!(t.cpu.v[0], 0x12AA);
    t.cpu.v[0] = 0x1234;
    t.ok(&["bif v0.16b, v1.16b, v2.16b"]);
    assert_eq!(t.cpu.v[0], 0xAA34);

    // Compares give all ones or zero per lane.
    t.cpu.v[1] = u128::from_le_bytes([1, 2, 3, 4, 5, 6, 7, 8, 9, 10, 11, 12, 13, 14, 15, 0x80]);
    t.cpu.v[2] = u128::from_le_bytes([1, 0, 3, 0, 5, 0, 7, 0, 9, 0, 11, 0, 13, 0, 15, 1]);
    t.ok(&["cmeq v0.16b, v1.16b, v2.16b"]);
    assert_eq!(lanes8(t.cpu.v[0])[..4], [0xFF, 0, 0xFF, 0]);
    t.ok(&["cmhi v0.16b, v1.16b, v2.16b"]);
    assert_eq!(lanes8(t.cpu.v[0])[14..], [0, 0xFF], "unsigned: 0x80 > 1");
    t.ok(&["cmgt v0.16b, v1.16b, v2.16b"]);
    assert_eq!(lanes8(t.cpu.v[0])[14..], [0, 0], "signed: -128 < 1");
    t.ok(&["cmge v0.16b, v1.16b, v2.16b"]);
    assert_eq!(lanes8(t.cpu.v[0])[14], 0xFF);
    t.ok(&["cmtst v0.16b, v1.16b, v2.16b"]);
    assert_eq!(lanes8(t.cpu.v[0])[1], 0);
    t.ok(&["cmeq v0.16b, v1.16b, #0"]);
    assert_eq!(t.cpu.v[0], 0);
    t.ok(&["cmlt v0.8h, v1.8h, #0"]);
    assert_eq!(lanes8(t.cpu.v[0])[14..], [0xFF, 0xFF], "the last halfword is 0x800f");
    t.cpu.v[1] = 0x8000_0000_0000_0000_0000_0000_0000_0001;
    t.ok(&["cmgt v0.4s, v1.4s, #0"]);
    assert_eq!(lanes8(t.cpu.v[0])[..4], [0xFF; 4]);
    t.ok(&["cmle v0.2d, v1.2d, #0"]);
    assert_eq!(t.cpu.v[0] >> 64, u128::from(u64::MAX));
    t.ok(&["neg v0.4s, v1.4s"]);
    assert_eq!(lanes8(t.cpu.v[0])[..4], (-1i32).to_le_bytes());
    t.ok(&["abs v0.2d, v1.2d"]);
    assert_eq!(lanes8(t.cpu.v[0])[8..], 0x8000_0000_0000_0000u64.to_le_bytes(), "ABS of the minimum wraps");
}

#[test]
fn simd_reductions_permutes_and_tables() {
    let mut t = T::new();
    t.cpu.v[1] = u128::from_le_bytes([1, 2, 3, 4, 5, 6, 7, 8, 9, 10, 11, 12, 13, 14, 15, 200]);
    t.ok(&["umaxv b0, v1.16b"]);
    assert_eq!(t.cpu.v[0], 200);
    t.ok(&["uminv h0, v1.8h"]);
    assert_eq!(t.cpu.v[0], 0x0201);
    t.ok(&["addv s0, v1.4s"]);
    assert_eq!(
        t.cpu.v[0],
        u128::from(0x0403_0201u32.wrapping_add(0x0807_0605).wrapping_add(0x0C0B_0A09).wrapping_add(0xC80F_0E0D))
    );
    t.ok(&["uaddlv h0, v1.16b"]);
    assert_eq!(t.cpu.v[0], u128::from((1..=15).sum::<u32>() + 200));

    t.cpu.v[2] = u128::from_le_bytes([16, 17, 18, 19, 20, 21, 22, 23, 24, 25, 26, 27, 28, 29, 30, 31]);
    t.ok(&["ext v0.16b, v1.16b, v2.16b, #5"]);
    assert_eq!(lanes8(t.cpu.v[0])[..3], [6, 7, 8]);
    assert_eq!(lanes8(t.cpu.v[0])[11..14], [16, 17, 18]);
    t.ok(&["ext v0.8b, v1.8b, v2.8b, #3"]);
    assert_eq!(lanes8(t.cpu.v[0])[..8], [4, 5, 6, 7, 8, 16, 17, 18]);
    assert_eq!(t.cpu.v[0] >> 64, 0);

    t.ok(&["zip1 v0.16b, v1.16b, v2.16b"]);
    assert_eq!(lanes8(t.cpu.v[0])[..4], [1, 16, 2, 17]);
    t.ok(&["zip2 v0.8h, v1.8h, v2.8h"]);
    assert_eq!(lanes8(t.cpu.v[0])[..4], [9, 10, 24, 25]);
    t.ok(&["uzp1 v0.4s, v1.4s, v2.4s"]);
    assert_eq!(lanes8(t.cpu.v[0])[..8], [1, 2, 3, 4, 9, 10, 11, 12]);
    t.ok(&["uzp2 v0.16b, v1.16b, v2.16b"]);
    assert_eq!(lanes8(t.cpu.v[0])[..3], [2, 4, 6]);
    t.ok(&["trn1 v0.4s, v1.4s, v2.4s"]);
    assert_eq!(lanes8(t.cpu.v[0])[..8], [1, 2, 3, 4, 16, 17, 18, 19]);

    t.cpu.v[1] = u128::from_le_bytes([
        0xA0, 0xA1, 0xA2, 0xA3, 0xA4, 0xA5, 0xA6, 0xA7, 0xA8, 0xA9, 0xAA, 0xAB, 0xAC, 0xAD, 0xAE, 0xAF,
    ]);
    t.cpu.v[2] = u128::from_le_bytes([15, 0, 16, 3, 255, 1, 2, 4, 0, 0, 0, 0, 0, 0, 0, 0]);
    t.cpu.v[0] = u128::from_le_bytes([0x55; 16]);
    t.ok(&["tbl v0.16b, {v1.16b}, v2.16b"]);
    assert_eq!(lanes8(t.cpu.v[0])[..8], [0xAF, 0xA0, 0, 0xA3, 0, 0xA1, 0xA2, 0xA4], "out-of-range indexes give 0");
    t.cpu.v[0] = u128::from_le_bytes([0x55; 16]);
    t.ok(&["tbx v0.8b, {v1.16b}, v2.8b"]);
    assert_eq!(lanes8(t.cpu.v[0])[..8], [0xAF, 0xA0, 0x55, 0xA3, 0x55, 0xA1, 0xA2, 0xA4], "TBX keeps them");
    assert_eq!(t.cpu.v[0] >> 64, 0);
    // The two-register table: v1:v2 are the table.
    t.cpu.v[2] = u128::from_le_bytes([0xC0; 16]);
    t.cpu.v[3] = u128::from_le_bytes([16, 17, 31, 32, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0]);
    t.ok(&["tbl v0.16b, {v1.16b, v2.16b}, v3.16b"]);
    assert_eq!(lanes8(t.cpu.v[0])[..5], [0xC0, 0xC0, 0xC0, 0, 0xA0], "bytes 16..31 come from the second register");
}

#[test]
fn simd_narrow_shift_and_misc() {
    let mut t = T::new();
    t.cpu.v[1] = 0x0004_0003_0002_0001_FF08_0007_0006_0005;
    t.ok(&["xtn v0.8b, v1.8h"]);
    assert_eq!(lanes8(t.cpu.v[0])[..8], [5, 6, 7, 8, 1, 2, 3, 4]);
    t.cpu.v[0] = u128::MAX;
    t.ok(&["xtn2 v0.16b, v1.8h"]);
    assert_eq!(lanes8(t.cpu.v[0])[8..], [5, 6, 7, 8, 1, 2, 3, 4]);
    assert_eq!(t.cpu.v[0] as u64, u64::MAX, "XTN2 keeps the lower half");
    t.cpu.v[1] = 0x8000_0000_0000_0010_0000_0100_0000_0001;
    t.ok(&["ushr v0.4s, v1.4s, #3"]);
    assert_eq!(lanes8(t.cpu.v[0])[12..], (0x8000_0000u32 >> 3).to_le_bytes());
    t.ok(&["sshr v0.8h, v1.8h, #2"]);
    assert_eq!(lanes8(t.cpu.v[0])[14..], (0x8000u16 as i16 >> 2).to_le_bytes());
    t.ok(&["shl v0.2d, v1.2d, #5"]);
    assert_eq!(t.cpu.v[0] as u64, 0x0000_0100_0000_0001u64 << 5);
    t.cpu.v[0] = 0xFFFF_FFFF;
    t.ok(&["sli v0.4s, v1.4s, #7"]);
    assert_eq!(t.cpu.v[0] as u32, 0x7F | (1 << 7));
    t.cpu.v[1] = 0xFFFF_FFFF_FFFF_FFFF;
    t.ok(&["ushll v0.8h, v1.8b, #0"]);
    assert_eq!(
        t.cpu.v[0],
        u128::from_le_bytes([0xFF, 0, 0xFF, 0, 0xFF, 0, 0xFF, 0, 0xFF, 0, 0xFF, 0, 0xFF, 0, 0xFF, 0])
    );
    t.cpu.v[1] = 0x8000_0000_0000_0000_0000_0000_0000_0000;
    t.ok(&["sshll2 v0.4s, v1.8h, #3"]);
    assert_eq!(t.cpu.v[0] >> 96, u128::from((0xFFFF_8000u32) << 3));
    t.cpu.v[1] = 0x0F0F_0000_0000_FFFF;
    t.ok(&["cnt v0.16b, v1.16b"]);
    assert_eq!(lanes8(t.cpu.v[0])[..2], [8, 8]);
    t.ok(&["not v0.16b, v1.16b"]);
    assert_eq!(t.cpu.v[0], !0x0F0F_0000_0000_FFFFu128);
    t.cpu.v[1] = u128::from_le_bytes([1, 2, 3, 4, 5, 6, 7, 8, 9, 10, 11, 12, 13, 14, 15, 16]);
    t.ok(&["rev64 v0.4s, v1.4s"]);
    assert_eq!(lanes8(t.cpu.v[0])[..8], [5, 6, 7, 8, 1, 2, 3, 4]);
    t.cpu.v[1] = 5;
    t.cpu.v[2] = 7;
    t.ok(&["add d0, d1, d2"]);
    assert_eq!(t.cpu.v[0], 12);
    t.ok(&["cmeq d0, d1, d2"]);
    assert_eq!(t.cpu.v[0], 0);
    t.ok(&["mul v0.4s, v1.4s, v2.4s"]);
    assert_eq!(t.cpu.v[0], 35);
    t.cpu.v[1] = u128::from_le_bytes([200, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 3]);
    t.cpu.v[2] = u128::from_le_bytes([100; 16]);
    t.ok(&["umax v0.16b, v1.16b, v2.16b"]);
    assert_eq!(lanes8(t.cpu.v[0])[..2], [200, 100]);
    t.ok(&["smin v0.8h, v1.8h, v2.8h"]);
    assert_eq!(lanes8(t.cpu.v[0])[..2], [200, 0], "signed halfwords: 200 < 25700");
    t.cpu.v[1] = 0x0000_0004_0000_0003_0000_0002_0000_0001;
    t.cpu.v[2] = 0x0000_0008_0000_0007_0000_0006_0000_0005;
    t.ok(&["addp v0.4s, v1.4s, v2.4s"]);
    assert_eq!(t.cpu.v[0], 0x0000_000F_0000_000B_0000_0007_0000_0003);
}

#[test]
fn simd_structure_loads_and_stores() {
    let mut t = T::new();
    for i in 0..64u8 {
        t.mem[0x1000 + usize::from(i)] = i;
    }
    t.cpu.x[1] = DATA;
    t.ok(&["ld1 {v0.16b}, [x1]"]);
    assert_eq!(lanes8(t.cpu.v[0]), core::array::from_fn(|i| i as u8));
    t.ok(&["ld1 {v0.16b, v1.16b}, [x1], #32"]);
    assert_eq!(t.x(1), DATA + 32);
    assert_eq!(lanes8(t.cpu.v[1])[0], 16);
    t.cpu.x[1] = DATA;
    t.ok(&["ld2 {v0.8h, v1.8h}, [x1]"]);
    // De-interleave: even halfwords to v0, odd to v1.
    assert_eq!(lanes8(t.cpu.v[0])[..4], [0, 1, 4, 5]);
    assert_eq!(lanes8(t.cpu.v[1])[..4], [2, 3, 6, 7]);
    t.ok(&["ld3 {v0.4s, v1.4s, v2.4s}, [x1]"]);
    assert_eq!(lanes8(t.cpu.v[1])[..4], [4, 5, 6, 7]);
    t.ok(&["ld1 {v0.s}[2], [x1]"]);
    assert_eq!(lanes8(t.cpu.v[0])[8..12], [0, 1, 2, 3]);
    t.ok(&["ld1r {v0.4s}, [x1]"]);
    assert_eq!(t.cpu.v[0], u128::from_le_bytes([0, 1, 2, 3].repeat(4).try_into().unwrap()));
    t.cpu.x[1] = DATA + 0x100;
    t.cpu.v[0] = u128::from_le_bytes(core::array::from_fn(|i| 0xA0 + i as u8));
    t.cpu.v[1] = u128::from_le_bytes(core::array::from_fn(|i| 0xB0 + i as u8));
    t.cpu.v[2] = u128::from_le_bytes(core::array::from_fn(|i| 0xC0 + i as u8));
    t.cpu.v[3] = u128::from_le_bytes(core::array::from_fn(|i| 0xD0 + i as u8));
    t.ok(&["st4 {v0.16b, v1.16b, v2.16b, v3.16b}, [x1]"]);
    assert_eq!(
        [t.r8(DATA + 0x100), t.r8(DATA + 0x101), t.r8(DATA + 0x102), t.r8(DATA + 0x103)],
        [0xA0, 0xB0, 0xC0, 0xD0]
    );
    assert_eq!(t.r8(DATA + 0x104), 0xA1, "interleaved");
    t.ok(&["st1 {v0.b}[15], [x1], #1"]);
    assert_eq!(t.r8(DATA + 0x100), 0xAF);
    assert_eq!(t.x(1), DATA + 0x101);
    t.cpu.x[2] = 8;
    t.ok(&["st1 {v0.8b}, [x1], x2"]);
    assert_eq!(t.x(1), DATA + 0x109);
}

#[test]
fn simd_register_loads_and_stores() {
    let mut t = T::new();
    t.cpu.x[1] = DATA;
    for i in 0..48u8 {
        t.mem[0x1000 + usize::from(i)] = i + 1;
    }
    t.ok(&["ldr q0, [x1, #16]"]);
    assert_eq!(lanes8(t.cpu.v[0])[0], 17);
    t.ok(&["ldr d0, [x1, #8]"]);
    assert_eq!(t.cpu.v[0], 0x100F_0E0D_0C0B_0A09, "narrower loads zero the rest");
    t.ok(&["ldr s0, [x1, #4]"]);
    assert_eq!(t.cpu.v[0], 0x0807_0605);
    t.ok(&["ldr h0, [x1, #2]"]);
    assert_eq!(t.cpu.v[0], 0x0403);
    t.ok(&["ldr b0, [x1, #1]"]);
    assert_eq!(t.cpu.v[0], 2);
    t.ok(&["ldp q0, q1, [x1]"]);
    assert_eq!((lanes8(t.cpu.v[0])[0], lanes8(t.cpu.v[1])[0]), (1, 17));
    t.ok(&["str q0, [x1, #32]"]);
    assert_eq!(t.r8(DATA + 32), 1);
    t.cpu.x[2] = DATA + 0x100;
    t.ok(&["stp d0, d1, [x2, #-16]!"]);
    assert_eq!(t.x(2), DATA + 0xF0);
    t.ok(&["ldp s0, s1, [x2], #8"]);
    assert_eq!(t.x(2), DATA + 0xF8);
}

// ---------------------------------------------------------------------
// Decoding
// ---------------------------------------------------------------------

#[test]
fn undefined_and_unsupported_encodings() {
    assert!(matches!(decode(0), Inst::Undefined(0)), "UDF #0");
    assert!(matches!(decode(0x0000_1234), Inst::Undefined(_)), "UDF #0x1234");
    // SVE, crypto and others are allocated but not implemented.
    assert!(matches!(decode(0x0420_0000), Inst::Unsupported(_)));
    assert!(matches!(decode(0x4E28_4800), Inst::Unsupported(_)), "AESE");
    // A reserved bit pattern: add/sub shifted register with shift 3, and a 32-bit shift by 32.
    assert!(matches!(decode(asm("add x0, x1, x2, lsl #3") | 3 << 22), Inst::Undefined(_)));
    assert!(matches!(decode(asm("add w0, w1, w2, lsl #3") | 1 << 15), Inst::Undefined(_)));
    // Execution reports them.
    let mut t = T::new();
    assert_eq!(t.run_words(&[0x0000_1234]), Outcome::Undefined(0x0000_1234));
    assert_eq!(t.cpu.pc, CODE);
    assert_eq!(t.run_words(&[0x4E28_4800]), Outcome::Unsupported(0x4E28_4800));
}

#[test]
fn every_test_vector_decodes() {
    for (text, w) in crate::vectors::VECTORS {
        let i = decode(*w);
        assert!(!matches!(i, Inst::Undefined(_) | Inst::Unsupported(_)), "{text}: {i:?}");
        // Decoding is a pure function of the word.
        assert_eq!(i, decode(*w));
    }
}

#[test]
fn decoder_fields() {
    assert_eq!(
        decode(asm("add x0, x1, #5")),
        Inst::AddSubImm { w: Width::X, sub: false, s: false, rd: 0, rn: 1, imm: 5 }
    );
    assert_eq!(
        decode(asm("sub sp, sp, #4096")),
        Inst::AddSubImm { w: Width::X, sub: true, s: false, rd: 31, rn: 31, imm: 4096 }
    );
    assert_eq!(decode(asm("b #-8")), Inst::Branch { link: false, offset: -8 });
    assert_eq!(decode(asm("b.ne #-0x20")), Inst::BranchCond { cond: Cond::Ne, offset: -0x20 });
    assert_eq!(decode(asm("tbnz x1, #40, #-0x8")), Inst::TestBranch { nz: true, rt: 1, bit: 40, offset: -8 });
    assert_eq!(
        decode(asm("ldr x0, [x1, x2, lsl #3]")),
        Inst::LoadStore {
            kind: MemKind::Load,
            size: 3,
            fp: false,
            rt: 0,
            addr: Addr::Reg { rn: 1, rm: 2, ext: Extend::Uxtx, shift: 3 }
        }
    );
    assert_eq!(
        decode(asm("ldp x0, x1, [x2, #16]")),
        Inst::LoadStorePair {
            kind: PairKind::Load,
            size: 3,
            fp: false,
            rt: 0,
            rt2: 1,
            addr: Addr::Imm { rn: 2, offset: 16 }
        }
    );
    assert_eq!(decode(asm("svc #0x80")), Inst::Svc(0x80));
    assert_eq!(decode(asm("mrs x0, tpidr_el0")), Inst::Mrs { rt: 0, reg: SysReg::TPIDR_EL0 });
    assert_eq!(decode(asm("fmov d0, #1.5")), Inst::FpMovImm { ft: FpType::D, rd: 0, bits: 1.5f64.to_bits() });
    assert_eq!(
        decode(asm("fmov s0, #-0.125")),
        Inst::FpMovImm { ft: FpType::S, rd: 0, bits: u64::from((-0.125f32).to_bits()) }
    );
    assert_eq!(
        decode(asm("cset x0, eq")),
        Inst::CondSel { w: Width::X, op: CselOp::Csinc, rd: 0, rn: 31, rm: 31, cond: Cond::Ne }
    );
    assert_eq!(Cond::Ne.invert(), Cond::Eq);
}

struct Rng(u64);
impl Rng {
    fn next(&mut self) -> u64 {
        let mut x = self.0;
        x ^= x >> 12;
        x ^= x << 25;
        x ^= x >> 27;
        self.0 = x;
        x.wrapping_mul(0x2545_F491_4F6C_DD1D)
    }
}

#[test]
fn decoding_arbitrary_words_never_panics() {
    let mut r = Rng(0x1234_5678_9ABC_DEF1);
    let mut defined = 0u32;
    for _ in 0..3_000_000 {
        let w = (r.next() >> 16) as u32;
        if !matches!(decode(w), Inst::Undefined(_) | Inst::Unsupported(_)) {
            defined += 1;
        }
    }
    assert!(defined > 100_000, "random words should often decode ({defined})");
    // Every combination of the top ten bits with a few payloads, and each single-bit and
    // two-bit pattern.
    for top in 0..1024u32 {
        for payload in [0u32, 0x3FFFFF, 0x155555, 0x2AAAAA, 0x100001] {
            let _ = decode(top << 22 | payload);
        }
    }
    for a in 0..32 {
        let _ = decode(1 << a);
        for b in 0..32 {
            let _ = decode(1 << a | 1 << b);
        }
    }
}

#[test]
fn executing_arbitrary_words_never_panics() {
    // Random instructions on random state against a small memory: every outcome is fine, a
    // panic (an out-of-range register, an overflowing shift, ...) is not.
    let mut r = Rng(0xDEAD_BEEF_0BAD_F00D);
    let mut mem_bytes = std::vec![0u8; 0x2000];
    for _ in 0..600_000 {
        let w = (r.next() >> 16) as u32;
        let mut cpu = Cpu::new();
        for x in cpu.x.iter_mut() {
            *x = match r.next() % 4 {
                0 => r.next(),
                1 => 0x1800 + (r.next() % 0x100),
                2 => r.next() % 64,
                _ => u64::MAX - (r.next() % 8),
            };
        }
        for v in cpu.v.iter_mut() {
            *v = (u128::from(r.next()) << 64) | u128::from(r.next());
        }
        cpu.sp = 0x1800;
        cpu.pc = 0x1000;
        cpu.nzcv = (r.next() as u32) & 0xF000_0000;
        mem_bytes[..4].copy_from_slice(&w.to_le_bytes());
        let mut mem = SliceMemory::new(0x1000, &mut mem_bytes);
        let _ = cpu.step(&mut mem);
    }
}

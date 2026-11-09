//! Test vectors: assembly text and the instruction word the system assembler
//! (`llvm-mc --triple=aarch64`, used as a black box, see CLEAN_ROOM.md and ADR 0152) makes
//! of it. The unit tests look words up by their text with [`asm`], so every encoding a test
//! uses has its assembly beside it. Where the assembler prints the instruction differently
//! (an alias), the comment shows its form.

/// `(assembly as written, word)`.
pub const VECTORS: &[(&str, u32)] = &[
    ("add x0, x1, #5", 0x91001420),
    ("add sp, sp, #16", 0x910043ff),
    ("sub sp, sp, #4096", 0xd14007ff), // sub sp, sp, #1, lsl #12
    ("subs xzr, x1, #1", 0xf100043f),  // cmp x1, #1
    ("cmp sp, #16", 0xf10043ff),
    ("adds w0, w1, #1", 0x31000420),
    ("add x0, x1, x2, lsl #3", 0x8b020c20),
    ("subs x0, x1, x2, asr #4", 0xeb821020),
    ("cmp w1, w2", 0x6b02003f),
    ("add x0, sp, w2, uxtw #2", 0x8b224be0),
    ("add x0, x1, w2, sxtb #1", 0x8b228420),
    ("adds x0, x1, x2, uxtx", 0xab226020),
    ("adc x0, x1, x2", 0x9a020020),
    ("sbcs w0, w1, w2", 0x7a020020),
    ("and x0, x1, #0xaaaaaaaaaaaaaaaa", 0x9201f020),
    ("orr x0, x1, #0x00ff00ff00ff00ff", 0xb2009c20), // orr x0, x1, #0xff00ff00ff00ff
    ("eor w0, w1, #0x7", 0x52000820),
    ("ands x0, x1, #0xffff", 0xf2403c20),
    ("and sp, x1, #0xfffffffffffffff0", 0x927cec3f),
    ("orr x0, xzr, #0x1", 0xb24003e0),
    ("bic x0, x1, x2", 0x8a220020),
    ("orn x0, x1, x2, lsl #4", 0xaa221020),
    ("eon w0, w1, w2", 0x4a220020),
    ("bics x0, x1, x2", 0xea220020),
    ("tst x1, x2", 0xea02003f),
    ("mov x0, x1", 0xaa0103e0),
    ("mvn x0, x2", 0xaa2203e0),
    ("movz x0, #0x1234, lsl #16", 0xd2a24680), // mov x0, #305397760
    ("movn x0, #0x1234", 0x92824680),          // mov x0, #-4661
    ("movn w0, #0x1234", 0x12824680),          // mov w0, #-4661
    ("movk x0, #0xbeef, lsl #48", 0xf2f7dde0), // movk x0, #48879, lsl #48
    ("adr x0, #0x100", 0x10000800),            // adr x0, #256
    ("adr x0, #-4", 0x10ffffe0),
    ("adrp x0, #0x5000", 0xb0000020), // adrp x0, #20480
    ("lsl x0, x1, #4", 0xd37cec20),
    ("lsr x0, x1, #4", 0xd344fc20),
    ("asr x0, x1, #4", 0x9344fc20),
    ("lsl w0, w1, #4", 0x531c6c20),
    ("ubfx x0, x1, #8, #8", 0xd3483c20),
    ("sbfx x0, x1, #4, #12", 0x93443c20),
    ("bfi x0, x1, #8, #8", 0xb3781c20),
    ("bfxil x0, x1, #4, #8", 0xb3442c20),
    ("uxtb w0, w1", 0x53001c20),
    ("sxtb x0, w1", 0x93401c20),
    ("sxth x0, w1", 0x93403c20),
    ("sxtw x0, w1", 0x93407c20),
    ("extr x0, x1, x2, #12", 0x93c23020),
    ("ror x0, x1, #12", 0x93c13020),
    ("ror w0, w1, #3", 0x13810c20),
    ("ccmp x1, x2, #4, eq", 0xfa420024),
    ("ccmn x1, #5, #2, ne", 0xba451822),
    ("ccmp w1, #31, #8, hi", 0x7a5f8828),
    ("csel x0, x1, x2, lt", 0x9a82b020),
    ("csinc x0, x1, x2, ge", 0x9a82a420),
    ("csinv x0, x1, x2, eq", 0xda820020),
    ("csneg w0, w1, w2, ne", 0x5a821420),
    ("cset x0, eq", 0x9a9f17e0),
    ("cinc x0, x1, ne", 0x9a810420),
    ("madd x0, x1, x2, x3", 0x9b020c20),
    ("msub x0, x1, x2, x3", 0x9b028c20),
    ("mul w0, w1, w2", 0x1b027c20),
    ("smaddl x0, w1, w2, x3", 0x9b220c20),
    ("smsubl x0, w1, w2, x3", 0x9b228c20),
    ("umaddl x0, w1, w2, x3", 0x9ba20c20),
    ("umsubl x0, w1, w2, x3", 0x9ba28c20),
    ("smull x0, w1, w2", 0x9b227c20),
    ("smulh x0, x1, x2", 0x9b427c20),
    ("umulh x0, x1, x2", 0x9bc27c20),
    ("udiv x0, x1, x2", 0x9ac20820),
    ("sdiv x0, x1, x2", 0x9ac20c20),
    ("sdiv w0, w1, w2", 0x1ac20c20),
    ("lsl x0, x1, x2", 0x9ac22020),
    ("lsr w0, w1, w2", 0x1ac22420),
    ("asr x0, x1, x2", 0x9ac22820),
    ("ror x0, x1, x2", 0x9ac22c20),
    ("clz x0, x1", 0xdac01020),
    ("clz w0, w1", 0x5ac01020),
    ("cls x0, x1", 0xdac01420),
    ("rbit x0, x1", 0xdac00020),
    ("rev x0, x1", 0xdac00c20),
    ("rev w0, w1", 0x5ac00820),
    ("rev16 x0, x1", 0xdac00420),
    ("rev32 x0, x1", 0xdac00820),
    ("b #0x100", 0x14000040), // b #256
    ("b #-8", 0x17fffffe),
    ("bl #0x40", 0x94000010),            // bl #64
    ("b.eq #0x20", 0x54000100),          // b.eq #32
    ("b.ne #-0x20", 0x54ffff01),         // b.ne #-32
    ("cbz x1, #0x10", 0xb4000081),       // cbz x1, #16
    ("cbnz w2, #-0x10", 0x35ffff82),     // cbnz w2, #-16
    ("tbz x1, #3, #0x8", 0x36180041),    // tbz w1, #3, #8
    ("tbnz x1, #40, #-0x8", 0xb747ffc1), // tbnz x1, #40, #-8
    ("br x1", 0xd61f0020),
    ("blr x2", 0xd63f0040),
    ("ret", 0xd65f03c0),
    ("ret x5", 0xd65f00a0),
    ("svc #0x80", 0xd4001001),
    ("brk #0x1234", 0xd4224680),
    ("nop", 0xd503201f),
    ("yield", 0xd503203f),
    ("bti c", 0xd503245f), // hint #34
    ("paciasp", 0xd503233f),
    ("autiasp", 0xd50323bf),
    ("mrs x0, nzcv", 0xd53b4200),       // mrs x0, NZCV
    ("msr nzcv, x1", 0xd51b4201),       // msr NZCV, x1
    ("mrs x0, fpcr", 0xd53b4400),       // mrs x0, FPCR
    ("msr fpcr, x1", 0xd51b4401),       // msr FPCR, x1
    ("mrs x0, fpsr", 0xd53b4420),       // mrs x0, FPSR
    ("msr fpsr, x1", 0xd51b4421),       // msr FPSR, x1
    ("mrs x0, tpidr_el0", 0xd53bd040),  // mrs x0, TPIDR_EL0
    ("msr tpidr_el0, x1", 0xd51bd041),  // msr TPIDR_EL0, x1
    ("mrs x0, cntvct_el0", 0xd53be040), // mrs x0, CNTVCT_EL0
    ("mrs x0, cntfrq_el0", 0xd53be000), // mrs x0, CNTFRQ_EL0
    ("mrs x0, dczid_el0", 0xd53b00e0),  // mrs x0, DCZID_EL0
    ("dmb ish", 0xd5033bbf),
    ("dsb sy", 0xd5033f9f),
    ("isb", 0xd5033fdf),
    ("clrex", 0xd5033f5f),
    ("dc zva, x1", 0xd50b7421),
    ("ldr x0, [x1]", 0xf9400020),
    ("ldr x0, [x1, #8]", 0xf9400420),
    ("ldr w0, [x1, #4]", 0xb9400420),
    ("ldrb w0, [x1, #1]", 0x39400420),
    ("ldrh w0, [x1, #2]", 0x79400420),
    ("ldrsb x0, [x1, #1]", 0x39800420),
    ("ldrsb w0, [x1, #1]", 0x39c00420),
    ("ldrsh x0, [x1, #2]", 0x79800420),
    ("ldrsw x0, [x1, #4]", 0xb9800420),
    ("str x0, [x1, #8]", 0xf9000420),
    ("strb w0, [x1, #1]", 0x39000420),
    ("strh w0, [x1, #2]", 0x79000420),
    ("str w0, [x1, #4]", 0xb9000420),
    ("ldur x0, [x1, #-8]", 0xf85f8020),
    ("stur w0, [x1, #-3]", 0xb81fd020),
    ("ldr x0, [x1, #8]!", 0xf8408c20),
    ("ldr x0, [x1], #-8", 0xf85f8420),
    ("str x0, [x1, #-16]!", 0xf81f0c20),
    ("str x0, [x1], #8", 0xf8008420),
    ("ldr x0, [x1, x2]", 0xf8626820),
    ("ldr x0, [x1, x2, lsl #3]", 0xf8627820),
    ("ldr w0, [x1, w2, uxtw #2]", 0xb8625820),
    ("ldr w0, [x1, w2, sxtw]", 0xb862c820),
    ("ldrb w0, [x1, x2]", 0x38626820),
    ("ldr x0, #0x20", 0x58000100),  // ldr x0, #32
    ("ldr w0, #-0x10", 0x18ffff80), // ldr w0, #-16
    ("ldrsw x0, #0x8", 0x98000040), // ldrsw x0, #8
    ("prfm pldl1keep, [x1]", 0xf9800020),
    ("ldr q0, [x1, #16]", 0x3dc00420),
    ("ldr d0, [x1, #8]", 0xfd400420),
    ("ldr s0, [x1, #4]", 0xbd400420),
    ("ldr h0, [x1, #2]", 0x7d400420),
    ("ldr b0, [x1, #1]", 0x3d400420),
    ("str q0, [x1, #32]", 0x3d800820),
    ("ldr q0, [x1, x2, lsl #4]", 0x3ce27820),
    ("ldr s0, #0x10", 0x1c000080), // ldr s0, #16
    ("ldp x0, x1, [x2]", 0xa9400440),
    ("ldp x0, x1, [x2, #16]", 0xa9410440),
    ("ldp w0, w1, [x2, #-8]", 0x297f0440),
    ("stp x0, x1, [sp, #-16]!", 0xa9bf07e0),
    ("ldp x0, x1, [sp], #16", 0xa8c107e0),
    ("ldpsw x0, x1, [x2, #8]", 0x69410440),
    ("ldnp x0, x1, [x2]", 0xa8400440),
    ("stnp w0, w1, [x2, #8]", 0x28010440),
    ("ldp q0, q1, [x2, #32]", 0xad410440),
    ("stp d0, d1, [x2, #-16]!", 0x6dbf0440),
    ("ldp s0, s1, [x2], #8", 0x2cc10440),
    ("ldxr x0, [x1]", 0xc85f7c20),
    ("ldxrb w0, [x1]", 0x085f7c20),
    ("ldaxr w0, [x1]", 0x885ffc20),
    ("stxr w2, x0, [x1]", 0xc8027c20),
    ("stlxr w2, w0, [x1]", 0x8802fc20),
    ("stxrh w2, w0, [x1]", 0x48027c20),
    ("ldxp x0, x1, [x2]", 0xc87f0440),
    ("stxp w3, x0, x1, [x2]", 0xc8230440),
    ("ldaxp w0, w1, [x2]", 0x887f8440),
    ("ldar x0, [x1]", 0xc8dffc20),
    ("stlr w0, [x1]", 0x889ffc20),
    ("ldarb w0, [x1]", 0x08dffc20),
    ("ldadd x0, x1, [x2]", 0xf8200041),
    ("ldaddal w0, w1, [x2]", 0xb8e00041),
    ("ldclr x0, x1, [x2]", 0xf8201041),
    ("ldeor x0, x1, [x2]", 0xf8202041),
    ("ldset x0, x1, [x2]", 0xf8203041),
    ("ldsmax w0, w1, [x2]", 0xb8204041),
    ("ldsmin x0, x1, [x2]", 0xf8205041),
    ("ldumax x0, x1, [x2]", 0xf8206041),
    ("ldumin w0, w1, [x2]", 0xb8207041),
    ("swp x0, x1, [x2]", 0xf8208041),
    ("stadd w0, [x1]", 0xb820003f),
    ("cas x0, x1, [x2]", 0xc8a07c41),
    ("casal w0, w1, [x2]", 0x88e0fc41),
    ("casb w0, w1, [x2]", 0x08a07c41),
    ("casp x0, x1, x2, x3, [x4]", 0x48207c82),
    ("casp w2, w3, w4, w5, [x6]", 0x08227cc4),
    ("fmov d0, #1.5", 0x1e6f1000),    // fmov d0, #1.50000000
    ("fmov s0, #-0.125", 0x1e381000), // fmov s0, #-0.12500000
    ("fmov d0, d1", 0x1e604020),
    ("fmov x0, d1", 0x9e660020),
    ("fmov d0, x1", 0x9e670020),
    ("fmov w0, s1", 0x1e260020),
    ("fmov s0, w1", 0x1e270020),
    ("fmov x0, v1.d[1]", 0x9eae0020),
    ("fmov v0.d[1], x1", 0x9eaf0020),
    ("fadd d0, d1, d2", 0x1e622820),
    ("fsub s0, s1, s2", 0x1e223820),
    ("fmul d0, d1, d2", 0x1e620820),
    ("fdiv s0, s1, s2", 0x1e221820),
    ("fsqrt d0, d1", 0x1e61c020),
    ("fabs d0, d1", 0x1e60c020),
    ("fneg s0, s1", 0x1e214020),
    ("fmin d0, d1, d2", 0x1e625820),
    ("fmax s0, s1, s2", 0x1e224820),
    ("fminnm d0, d1, d2", 0x1e627820),
    ("fmaxnm d0, d1, d2", 0x1e626820),
    ("fnmul d0, d1, d2", 0x1e628820),
    ("fmadd d0, d1, d2, d3", 0x1f420c20),
    ("fmsub s0, s1, s2, s3", 0x1f028c20),
    ("fnmadd d0, d1, d2, d3", 0x1f620c20),
    ("fnmsub d0, d1, d2, d3", 0x1f628c20),
    ("fcmp d0, d1", 0x1e612000),
    ("fcmp s0, #0.0", 0x1e202008),
    ("fcmpe d0, d1", 0x1e612010),
    ("fccmp d0, d1, #4, eq", 0x1e610404),
    ("fcsel d0, d1, d2, lt", 0x1e62bc20),
    ("fcvt s0, d1", 0x1e624020),
    ("fcvt d0, s1", 0x1e22c020),
    ("scvtf d0, x1", 0x9e620020),
    ("scvtf s0, w1", 0x1e220020),
    ("ucvtf d0, x1", 0x9e630020),
    ("ucvtf d0, w1, #8", 0x1e43e020),
    ("fcvtzs x0, d1", 0x9e780020),
    ("fcvtzs w0, s1", 0x1e380020),
    ("fcvtzu x0, d1", 0x9e790020),
    ("fcvtzs w0, d1, #4", 0x1e58f020),
    ("fcvtns x0, d1", 0x9e600020),
    ("fcvtas x0, d1", 0x9e640020),
    ("fcvtps x0, d1", 0x9e680020),
    ("fcvtms x0, d1", 0x9e700020),
    ("frintn d0, d1", 0x1e644020),
    ("frintm d0, d1", 0x1e654020),
    ("frintp s0, s1", 0x1e24c020),
    ("frintz d0, d1", 0x1e65c020),
    ("frinta d0, d1", 0x1e664020),
    ("movi v0.16b, #0x5a", 0x4f02e740), // movi v0.16b, #90
    ("movi v0.2d, #0xff00ff00ff00ff00", 0x6f05e540),
    ("movi v0.4s, #0x12, lsl #8", 0x4f002640), // movi v0.4s, #18, lsl #8
    ("movi d0, #0xff00ff00ff00ff00", 0x2f05e540),
    ("mvni v0.4s, #0x12, lsl #16", 0x6f004640), // mvni v0.4s, #18, lsl #16
    ("orr v0.4s, #0x1", 0x4f001420),            // orr v0.4s, #1
    ("bic v0.8h, #0xf, lsl #8", 0x6f00b5e0),    // bic v0.8h, #15, lsl #8
    ("fmov v0.4s, #1.0", 0x4f03f600),           // fmov v0.4s, #1.00000000
    ("dup v0.16b, w1", 0x4e010c20),
    ("dup v0.4s, v1.s[2]", 0x4e140420),
    ("dup v0.2d, x1", 0x4e080c20),
    ("ins v0.s[1], w1", 0x4e0c1c20),      // mov v0.s[1], w1
    ("ins v0.h[3], v1.h[5]", 0x6e0e5420), // mov v0.h[3], v1.h[5]
    ("umov w0, v1.b[3]", 0x0e073c20),
    ("umov x0, v1.d[1]", 0x4e183c20), // mov x0, v1.d[1]
    ("smov x0, v1.b[7]", 0x4e0f2c20),
    ("smov w0, v1.h[1]", 0x0e062c20),
    ("add v0.16b, v1.16b, v2.16b", 0x4e228420),
    ("sub v0.4s, v1.4s, v2.4s", 0x6ea28420),
    ("add v0.2d, v1.2d, v2.2d", 0x4ee28420),
    ("and v0.16b, v1.16b, v2.16b", 0x4e221c20),
    ("orr v0.8b, v1.8b, v2.8b", 0x0ea21c20),
    ("eor v0.16b, v1.16b, v2.16b", 0x6e221c20),
    ("bic v0.16b, v1.16b, v2.16b", 0x4e621c20),
    ("orn v0.16b, v1.16b, v2.16b", 0x4ee21c20),
    ("bsl v0.16b, v1.16b, v2.16b", 0x6e621c20),
    ("bit v0.16b, v1.16b, v2.16b", 0x6ea21c20),
    ("bif v0.16b, v1.16b, v2.16b", 0x6ee21c20),
    ("cmeq v0.16b, v1.16b, v2.16b", 0x6e228c20),
    ("cmhi v0.4s, v1.4s, v2.4s", 0x6ea23420),
    ("cmhs v0.8h, v1.8h, v2.8h", 0x6e623c20),
    ("cmgt v0.2d, v1.2d, v2.2d", 0x4ee23420),
    ("cmge v0.16b, v1.16b, v2.16b", 0x4e223c20),
    ("cmtst v0.16b, v1.16b, v2.16b", 0x4e228c20),
    ("cmeq v0.16b, v1.16b, #0", 0x4e209820),
    ("cmgt v0.4s, v1.4s, #0", 0x4ea08820),
    ("cmlt v0.8h, v1.8h, #0", 0x4e60a820),
    ("cmle v0.2d, v1.2d, #0", 0x6ee09820),
    ("cmge v0.16b, v1.16b, #0", 0x6e208820),
    ("mul v0.4s, v1.4s, v2.4s", 0x4ea29c20),
    ("umax v0.16b, v1.16b, v2.16b", 0x6e226420),
    ("smin v0.8h, v1.8h, v2.8h", 0x4e626c20),
    ("addp v0.4s, v1.4s, v2.4s", 0x4ea2bc20),
    ("umaxv b0, v1.16b", 0x6e30a820),
    ("uminv h0, v1.8h", 0x6e71a820),
    ("addv s0, v1.4s", 0x4eb1b820),
    ("uaddlv h0, v1.16b", 0x6e303820),
    ("ext v0.16b, v1.16b, v2.16b, #5", 0x6e022820),
    ("ext v0.8b, v1.8b, v2.8b, #3", 0x2e021820),
    ("tbl v0.16b, {v1.16b}, v2.16b", 0x4e020020), // tbl v0.16b, { v1.16b }, v2.16b
    ("tbl v0.16b, {v1.16b, v2.16b}, v3.16b", 0x4e032020), // tbl v0.16b, { v1.16b, v2.16b }, v3.16b
    ("tbx v0.8b, {v1.16b}, v2.8b", 0x0e021020),   // tbx v0.8b, { v1.16b }, v2.8b
    ("xtn v0.8b, v1.8h", 0x0e212820),
    ("xtn2 v0.16b, v1.8h", 0x4e212820),
    ("uzp1 v0.4s, v1.4s, v2.4s", 0x4e821820),
    ("uzp2 v0.16b, v1.16b, v2.16b", 0x4e025820),
    ("zip1 v0.16b, v1.16b, v2.16b", 0x4e023820),
    ("zip2 v0.8h, v1.8h, v2.8h", 0x4e427820),
    ("trn1 v0.4s, v1.4s, v2.4s", 0x4e822820),
    ("not v0.16b, v1.16b", 0x6e205820), // mvn v0.16b, v1.16b
    ("cnt v0.16b, v1.16b", 0x4e205820),
    ("rev64 v0.4s, v1.4s", 0x4ea00820),
    ("neg v0.4s, v1.4s", 0x6ea0b820),
    ("abs v0.2d, v1.2d", 0x4ee0b820),
    ("ushr v0.4s, v1.4s, #3", 0x6f3d0420),
    ("sshr v0.8h, v1.8h, #2", 0x4f1e0420),
    ("shl v0.2d, v1.2d, #5", 0x4f455420),
    ("sli v0.4s, v1.4s, #7", 0x6f275420),
    ("usra v0.16b, v1.16b, #1", 0x6f0f1420),
    ("shrn v0.8b, v1.8h, #4", 0x0f0c8420),
    ("ushll v0.8h, v1.8b, #0", 0x2f08a420),
    ("sshll2 v0.4s, v1.8h, #3", 0x4f13a420),
    ("add d0, d1, d2", 0x5ee28420),
    ("cmeq d0, d1, d2", 0x7ee28c20),
    ("dup s0, v1.s[2]", 0x5e140420),                            // mov s0, v1.s[2]
    ("ld1 {v0.16b}, [x1]", 0x4c407020),                         // ld1 { v0.16b }, [x1]
    ("ld1 {v0.16b, v1.16b}, [x1], #32", 0x4cdfa020),            // ld1 { v0.16b, v1.16b }, [x1], #32
    ("ld1 {v0.4s, v1.4s, v2.4s, v3.4s}, [x1]", 0x4c402820),     // ld1 { v0.4s, v1.4s, v2.4s, v3.4s }, [x1]
    ("st1 {v0.8b}, [x1], x2", 0x0c827020),                      // st1 { v0.8b }, [x1], x2
    ("ld2 {v0.8h, v1.8h}, [x1]", 0x4c408420),                   // ld2 { v0.8h, v1.8h }, [x1]
    ("ld3 {v0.4s, v1.4s, v2.4s}, [x1]", 0x4c404820),            // ld3 { v0.4s, v1.4s, v2.4s }, [x1]
    ("st4 {v0.16b, v1.16b, v2.16b, v3.16b}, [x1]", 0x4c000020), // st4 { v0.16b, v1.16b, v2.16b, v3.16b }, [x1]
    ("ld1 {v0.s}[2], [x1]", 0x4d408020),                        // ld1 { v0.s }[2], [x1]
    ("st1 {v0.b}[15], [x1], #1", 0x4d9f1c20),                   // st1 { v0.b }[15], [x1], #1
    ("ld1r {v0.4s}, [x1]", 0x4d40c820),                         // ld1r { v0.4s }, [x1]
    ("ld2r {v0.8h, v1.8h}, [x1], #4", 0x4dffc420),              // ld2r { v0.8h, v1.8h }, [x1], #4
    ("ld4 {v0.d, v1.d, v2.d, v3.d}[1], [x1]", 0x4d60a420),      // ld4 { v0.d, v1.d, v2.d, v3.d }[1], [x1]
    ("lsr x0, x1, x2", 0x9ac22420),
    ("mrs x0, tpidrro_el0", 0xd53bd060), // mrs x0, TPIDRRO_EL0
    ("mrs x1, cntvct_el0", 0xd53be041),  // mrs x1, CNTVCT_EL0
    ("ldr w0, [x1, #8]", 0xb9400820),
    ("ldrb w0, [x1, #9]", 0x39402420),
    ("ldrh w0, [x1, #10]", 0x79401420),
    ("str x0, [x1, #16]", 0xf9000820),
    ("strb w0, [x1, #32]", 0x39008020),
    ("strh w0, [x1, #34]", 0x79004420),
    ("str w0, [x1, #36]", 0xb9002420),
    ("ldrsb x0, [x1]", 0x39800020),
    ("ldrsb w0, [x1]", 0x39c00020),
    ("ldp x2, x3, [sp], #16", 0xa8c10fe2),
    ("ldrsw x0, #0x28", 0x98000140), // ldrsw x0, #40
    ("ldp w0, w1, [x2, #16]", 0x29420440),
    ("ldpsw x0, x1, [x2, #16]", 0x69420440),
    ("stnp w0, w1, [x2, #32]", 0x28040440),
    ("str x0, [x1]", 0xf9000020),
    ("ldp x2, x3, [x1]", 0xa9400c22),
    ("ldxr x3, [x1]", 0xc85f7c23),
    ("ldxp x3, x4, [x1]", 0xc87f1023),
    ("ldadd x0, x1, [x1]", 0xf8200021),
    ("cas x0, x1, [x1]", 0xc8a07c21),
    ("ldar w0, [x1]", 0x88dffc20),
    ("cas x3, x4, [x1]", 0xc8a37c24),
    ("casb w3, w4, [x1]", 0x08a37c24),
    ("ldadd x0, x4, [x1]", 0xf8200024),
    ("fmov x0, d0", 0x9e660000),
    ("fmov x0, v0.d[1]", 0x9eae0000),
    ("fmov w0, s0", 0x1e260000),
    ("fdiv d0, d1, d2", 0x1e621820),
    ("fdiv d0, d2, d2", 0x1e621840),
    ("fadd d0, d2, d1", 0x1e612840),
    ("fsub d0, d1, d1", 0x1e613820),
    ("fmax d0, d1, d2", 0x1e624820),
    ("fmsub d0, d1, d2, d3", 0x1f428c20),
    ("fcmp d1, d0", 0x1e602020),
    ("fcmp d0, d0", 0x1e602000),
    ("fcsel d5, d0, d1, lt", 0x1e61bc05),
    ("fcvtzs w0, d1", 0x1e780020),
    ("frintp d0, d1", 0x1e64c020),
    ("and v0.8b, v1.8b, v2.8b", 0x0e221c20),
    ("cmhi v0.16b, v1.16b, v2.16b", 0x6e223420),
    ("cmgt v0.16b, v1.16b, v2.16b", 0x4e223420),
    ("ldp q0, q1, [x1]", 0xad400420),
    ("add w0, w1, w2, lsl #3", 0x0b020c20),
];

/// The word the assembler made of `text`.
pub fn asm(text: &str) -> u32 {
    let norm: std::string::String = text.split_whitespace().collect::<std::vec::Vec<_>>().join(" ");
    VECTORS.iter().find(|(t, _)| *t == norm).map(|(_, w)| *w).unwrap_or_else(|| panic!("no test vector for {norm:?}"))
}

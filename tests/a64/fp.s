// Scalar floating point: arithmetic, fused multiply-add, conversions, compares.
.include "dump.inc"
.globl _start
_start:
    fmov d0, #1.5
    fmov d1, #-0.125
    fadd d2, d0, d1
    fmul d3, d0, d1
    fdiv d4, d0, d1
    fsqrt d5, d0
    fmadd d6, d0, d1, d2
    fnmsub d7, d0, d1, d2
    fmov d8, #2.0
    fcvt s9, d5
    fcvt d10, s9
    fcvtzs x1, d4
    fcvtzu w2, d3
    fcvtns x3, d0
    fcvtas x4, d8
    scvtf d11, x1
    ucvtf s12, w2
    scvtf d13, x1, #3
    fcmp d0, d1
    mrs x5, nzcv
    fcmpe d0, d0
    mrs x6, nzcv
    fmov d14, xzr
    fdiv d15, d0, d14           // +inf, division by zero flag not set for x/0? (it is)
    fdiv d16, d14, d14          // invalid operation: default NaN
    fcmp d16, d16
    mrs x7, nzcv
    fcsel d17, d0, d1, lt
    fmax d18, d0, d1
    fminnm d19, d16, d1
    fabs d20, d1
    fneg d21, d0
    frintm d22, d1
    frintp d23, d1
    frinta d24, d0
    fmov x8, d0
    fmov d25, x8
    movz x9, #0x7FF0, lsl #48       // signalling NaN pattern 0x7FF0000000000001
    movk x9, #1
    fmov d26, x9
    fadd d27, d26, d0
    mov x10, #-1
    scvtf d28, x10
    fcvtzs w11, d27
    mov v0.16b, v0.16b
    // v0..v7 are dumped; move some results there
    fmov d1, d27
    fmov d2, d15
    fmov d3, d16
    fmov d4, d22
    fmov d5, d24
    DUMP_AND_EXIT

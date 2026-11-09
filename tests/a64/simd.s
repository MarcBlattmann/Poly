// Advanced SIMD: moves, lane operations, compares, reductions, permutes, table lookup.
.include "dump.inc"
.bss
.balign 16
vbuf: .space 128
.text
.globl _start
_start:
    msr nzcv, xzr
    movi v0.16b, #0x5A
    movi v1.2d, #0xFF00FF00FF00FF00
    mvni v2.4s, #0x12, lsl #8
    dup v3.8h, w1
    movz x1, #0x0102, lsl #48
    movk x1, #0x0304, lsl #32
    movk x1, #0x0506, lsl #16
    movk x1, #0x0708
    dup v4.2d, x1
    ins v3.s[2], w1
    umov w2, v4.b[3]
    smov x3, v4.b[7]
    add v5.16b, v0.16b, v1.16b
    sub v6.4s, v2.4s, v3.4s
    and v7.16b, v0.16b, v1.16b
    orr v7.16b, v7.16b, v4.16b
    eor v7.16b, v7.16b, v5.16b
    bic v6.16b, v6.16b, v0.16b
    cmeq v5.16b, v0.16b, v0.16b
    cmhi v6.8h, v1.8h, v2.8h
    cmgt v6.4s, v3.4s, v2.4s
    cmeq v4.16b, v0.16b, #0
    addv b0, v5.16b
    umaxv h1, v6.8h
    uminv b2, v7.16b
    uaddlv h3, v7.16b
    ext v4.16b, v5.16b, v6.16b, #5
    xtn v5.8b, v6.8h
    xtn2 v5.16b, v1.8h
    uzp1 v6.4s, v2.4s, v3.4s
    zip1 v7.8b, v4.8b, v5.8b
    zip2 v7.16b, v4.16b, v5.16b
    ushr v2.4s, v2.4s, #3
    shl v3.2d, v3.2d, #5
    sshr v4.8h, v4.8h, #2
    cnt v5.16b, v1.16b
    movi v6.16b, #0x03
    tbl v7.16b, {v4.16b}, v6.16b
    adrp x4, vbuf
    add x4, x4, :lo12:vbuf
    st1 {v0.16b, v1.16b}, [x4], #32
    ld2 {v2.8h, v3.8h}, [x4]
    ld1r {v4.4s}, [x4]
    DUMP_AND_EXIT

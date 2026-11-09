// Integer instructions with fixed operands: carry, overflow, SP versus XZR, 32-bit zeroing.
.include "dump.inc"
.globl _start
_start:
    movz x1, #0xFFFF, lsl #48
    movk x1, #0xFFFF, lsl #32
    movk x1, #0xFFFF, lsl #16
    movk x1, #0xFFFF
    mov x2, #1
    adds x3, x1, x2             // carry out, zero
    mrs x20, nzcv
    mov x4, #0x7FFFFFFFFFFFFFFF
    adds x5, x4, x2             // signed overflow
    mrs x21, nzcv
    subs x6, x2, x1             // 1 - (-1)
    mrs x22, nzcv
    mov w7, #0x7FFFFFFF
    adds w8, w7, w2             // 32-bit overflow zero-extends
    mrs x23, nzcv
    adc x9, x2, x2
    sbc x10, x2, x2
    movn x11, #5
    orr x12, xzr, #0x00FF00FF00FF00FF
    and x13, x12, #0xF0F0F0F0F0F0F0F0
    eor w14, w12, #0xFF
    bic x15, x12, x13
    orn x16, xzr, x13
    ubfx x17, x12, #8, #16
    sbfx x18, x11, #0, #8
    lsl x19, x2, #63
    asr x24, x19, #62
    extr x25, x1, x2, #7
    ror x26, x12, #12
    madd x27, x4, x2, x2
    smulh x28, x4, x4
    umulh x29, x1, x1
    mov x0, #0
    sdiv x0, x19, x1            // INT_MIN / -1
    udiv x30, x2, xzr           // division by zero
    cmp x1, x2
    csel x3, x1, x2, hi
    csinc x4, x1, x2, lt
    csinv x5, x1, x2, lt
    csneg x6, x1, x2, lt
    ccmp x1, x2, #4, ne
    clz x7, x2
    cls x8, x1
    rbit x9, x2
    rev x10, x12
    rev16 x11, x12
    rev32 x13, x12
    mov x1, sp
    add sp, sp, #16
    sub sp, sp, #16
    mov x14, sp
    sub x1, x14, x1             // zero
    mov x14, #0                 // (the stack address itself differs between runs)
    cmn x1, #1
    mov x2, x20
    mov x3, x21
    mov x4, x22
    mov x5, x23
    DUMP_AND_EXIT

// Loads and stores in every addressing mode, pairs, exclusives and LSE atomics.
.include "dump.inc"
.bss
.balign 64
buf: .space 256
.data
.balign 16
lit: .quad 0x1122334455667788, 0x99AABBCCDDEEFF00
.text
.globl _start
_start:
    msr nzcv, xzr               // the initial flags are unspecified (qemu starts with Z set)
    adrp x20, buf
    add x20, x20, :lo12:buf
    movz x1, #0x8877
    movk x1, #0x6655, lsl #16
    movk x1, #0x4433, lsl #32
    movk x1, #0xF211, lsl #48
    str x1, [x20]
    strb w1, [x20, #17]
    strh w1, [x20, #18]
    str w1, [x20, #20]
    ldrsb x2, [x20, #7]
    ldrsh x3, [x20, #6]
    ldrsw x4, [x20, #4]
    ldrb w5, [x20, #1]
    ldur x6, [x20, #1]
    stp x1, x2, [x20, #32]
    ldp x7, x8, [x20, #32]
    ldpsw x9, x10, [x20, #32]
    add x11, x20, #64
    str x1, [x11, #8]!
    str x2, [x11], #16
    ldr x12, [x11, #-16]
    mov x13, #2
    ldr x14, [x20, x13, lsl #3]
    ldr w15, [x20, w13, uxtw #2]
    adr x16, lit
    ldr x17, lit
    ldr q0, lit
    str q0, [x20, #96]
    ldp d1, d2, [x20, #96]
    stp s1, s2, [x20, #112]
    ldr b3, [x20, #1]
    ldr h4, [x20, #2]
    // exclusives (addresses have no offset form)
    add x21, x20, #8
    add x22, x20, #32
    add x23, x20, #128
    add x24, x20, #136
    add x25, x20, #144
    ldxr x18, [x20]
    stxr w19, x2, [x20]
    stxr w26, x2, [x20]          // monitor gone: fails
    ldaxr w27, [x21]
    clrex
    stlxr w28, w27, [x21]        // monitor cleared: fails
    ldxp x7, x8, [x22]
    stxp w9, x8, x7, [x22]
    // LSE
    mov x10, #5
    ldadd x10, x11, [x23]
    swp x10, x12, [x23]
    ldumax x1, x13, [x23]
    stset x10, [x24]
    mov x3, #0
    cas x3, x10, [x25]
    ldaddal x10, x14, [x24]
    casp x2, x3, x4, x5, [x22]
    ldr x4, [x20, #128]
    ldr x5, [x20, #136]
    ldr x6, [x20, #144]
    DUMP_AND_EXIT

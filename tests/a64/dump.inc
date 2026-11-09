// Shared by the test programs: DUMP_AND_EXIT writes the registers to standard output and exits.
//
// Layout of the 8-byte words written (a64diff prints differences by these names):
//   0..=30  X0..X30      31  NZCV      32  FPSR      33..  V0 low, V0 high, V1 low, ... V7 high
// X0 is saved first through TPIDR_EL0 (a register every program may use as scratch at the end).
// SP is not dumped: its initial value differs between runs.
.bss
.balign 16
__dump: .space 8 * 49

.text
.macro DUMP_AND_EXIT
    msr tpidr_el0, x0
    adrp x0, __dump
    add x0, x0, :lo12:__dump
    stp x1, x2, [x0, #8]
    stp x3, x4, [x0, #24]
    stp x5, x6, [x0, #40]
    stp x7, x8, [x0, #56]
    stp x9, x10, [x0, #72]
    stp x11, x12, [x0, #88]
    stp x13, x14, [x0, #104]
    stp x15, x16, [x0, #120]
    stp x17, x18, [x0, #136]
    stp x19, x20, [x0, #152]
    stp x21, x22, [x0, #168]
    stp x23, x24, [x0, #184]
    stp x25, x26, [x0, #200]
    stp x27, x28, [x0, #216]
    stp x29, x30, [x0, #232]
    mrs x1, nzcv
    str x1, [x0, #248]
    mrs x1, fpsr
    and x1, x1, #3
    str x1, [x0, #256]
    add x1, x0, #264
    stp q0, q1, [x1]
    stp q2, q3, [x1, #32]
    stp q4, q5, [x1, #64]
    stp q6, q7, [x1, #96]
    mrs x1, tpidr_el0
    str x1, [x0]
    mov x1, x0
    mov x0, #1
    mov x2, #(8 * 49)
    mov x8, #64
    svc #0
    mov x0, #0
    mov x8, #93
    svc #0
.endm

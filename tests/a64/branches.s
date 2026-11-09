// Branches, link register, conditional branches on flags and registers.
.include "dump.inc"
.globl _start
_start:
    mov x0, #0
    bl f1
    add x0, x0, #1
    adr x9, f2
    blr x9
    add x0, x0, #100
    mov x1, #5
    cbz x1, skip1
    add x0, x0, #1000
skip1:
    cbnz x1, skip2
    mov x3, #20000
    add x0, x0, x3
skip2:
    tbz x1, #1, skip3
    add x0, x0, #7
skip3:
    tbnz x1, #2, skip4
    add x0, x0, #9
skip4:
    cmp x1, #5
    b.eq eq
    mov x3, #30000
    add x0, x0, x3
eq:
    b.ne ne
    mov x3, #40000
    add x0, x0, x3
ne:
    adr x9, tail
    br x9
    mov x3, #50000
    add x0, x0, x3
tail:
    mov x2, x30
    DUMP_AND_EXIT
f1:
    add x0, x0, #10
    ret
f2:
    add x0, x0, #20
    ret x30

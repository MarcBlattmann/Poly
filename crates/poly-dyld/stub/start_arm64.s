// Poly's dyld start code for arm64 (see src/start.rs), run by the arm64
// translator (ADR 0257). Assembled once with `clang -arch arm64 -c` (black
// box) by gen.sh; the bytes are embedded in start.rs. Written for Poly from
// the Arm Architecture Reference Manual and Apple's public arm64 calling
// convention; no Apple or third-party source was read.
//
// Entry: sp -> [main header address, argc, argv..., 0, envp..., 0,
// apple..., 0]. The data block is 0x1000 bytes after the start of this code
// (the same layout as the x86-64 block):
//   0  main (LC_MAIN) or the program counter of LC_UNIXTHREAD
//   8  mode: 0 = call main(argc, argv, envp, apple) and exit with its result;
//            1 = jump to the program counter with sp at argc
//  16  exit function (0: the raw exit system call)
//  24  number of initializers
//  32  the initializers, called with (argc, argv, envp, apple, the data
//      block) (ADR 0258: libdyld reads the image table after them)
//
// x19 to x26 are callee-saved (AAPCS64), so they survive every call.
    .text
    .globl _poly_dyld_start
    .p2align 2
_poly_dyld_start:
    adr x19, _poly_dyld_start       // x19 = this code
    add x19, x19, #0x1000           // x19 = the data block
    mov x20, sp                     // x20 -> main header address
    ldr x21, [x20, #8]              // argc
    add x22, x20, #16               // argv
    add x23, x22, x21, lsl #3
    add x23, x23, #8                // envp
    mov x24, x23
1:  ldr x9, [x24], #8
    cbnz x9, 1b                     // x24 = apple
    mov x25, #0                     // index of the next initializer
2:  ldr x9, [x19, #24]
    cmp x25, x9
    b.hs 3f
    add x10, x19, #32
    ldr x10, [x10, x25, lsl #3]
    add x25, x25, #1
    mov x0, x21
    mov x1, x22
    mov x2, x23
    mov x3, x24
    mov x4, x19
    blr x10
    b 2b
3:  ldr x9, [x19, #8]
    cmp x9, #1
    b.ne 4f
    add x9, x20, #8
    mov sp, x9                      // sp -> argc
    ldr x9, [x19]
    br x9
4:  mov x0, x21
    mov x1, x22
    mov x2, x23
    mov x3, x24
    ldr x9, [x19]
    blr x9
    ldr x9, [x19, #16]
    cbz x9, 5f
    blr x9
5:  mov x16, #1
    svc #0x80
    brk #1

// The thread-local variable thunk every descriptor's first word points to:
// x0 -> descriptor {thunk, key, offset}. On arm64 the key is a slot of the
// calling thread's table, which TPIDRRO_EL0 points to (slot 0 is the
// thread structure, slot k the thread's block of image k; ADR 0257
// addendum), so every thread has its own variables. Returns block +
// offset in x0; no other register changes (x1 and x2 are saved on the
// stack around their use).
    .p2align 7
    .globl _poly_tlv_get
_poly_tlv_get:
    stp x1, x2, [sp, #-16]!
    ldr x1, [x0, #8]
    mrs x2, tpidrro_el0
    ldr x2, [x2, x1, lsl #3]
    ldr x1, [x0, #16]
    add x0, x2, x1
    ldp x1, x2, [sp], #16
    ret

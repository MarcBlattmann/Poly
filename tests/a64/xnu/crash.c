/* (d) A static arm64 program that reads an unmapped address on purpose
 * (ADR 0257): the translator stops it at the fault, and the process ends
 * with SIGSEGV as a native program would. Written for Poly. */
#include "sys.h"

__attribute__((noreturn)) void poly_crash_main(void) {
    const char *s = "a64-crash: about to read address 8\n";
    poly_syscall3(SYS_write, 1, (long)s, (long)poly_strlen(s));
    volatile long *p = (volatile long *)8;
    long v = *p;
    poly_syscall3(SYS_exit, (int)v, 0, 0);
    for (;;) {
    }
}

__asm__(".globl start\nstart:\n  bl _poly_crash_main\n  brk #1\n");

/* (a) The smallest arm64 program for the translated XNU process (ADR 0257):
 * static (LC_UNIXTHREAD, no dyld), `write` and `exit` as raw system calls.
 * The kernel starts it at `start` with argc at the stack pointer, argv and
 * envp after it. It also checks the error convention: a write to a
 * descriptor that is not open fails with the carry flag set and EBADF (9)
 * in x0. Written for Poly. */
#include "sys.h"

static void say(const char *s) { poly_syscall3(SYS_write, 1, (long)s, (long)poly_strlen(s)); }

static void say_num(long v) {
    char t[24];
    int n = 0;
    do {
        t[n++] = (char)('0' + v % 10);
        v /= 10;
    } while (v);
    char out[24];
    for (int i = 0; i < n; i++) out[i] = t[n - 1 - i];
    out[n] = 0;
    say(out);
}

__attribute__((noreturn)) void poly_static_main(long *sp) {
    long argc = sp[0];
    char **argv = (char **)(sp + 1);
    char **envp = argv + argc + 1;
    int failed = 0;
    say("a64-static: hello from an arm64 program\n");
    say("a64-static: argc=");
    say_num(argc);
    say(" argv[0]=");
    say(argc > 0 && argv[0] ? argv[0] : "(none)");
    say("\n");
    if (argc < 1 || argv[argc] != 0) failed++;
    int envc = 0;
    while (envp[envc]) envc++;
    struct sysret r = poly_syscall6(SYS_write, 99, (long)"x", 1, 0, 0, 0);
    if (r.failed && r.value == 9) {
        say("a64-static: write to fd 99: carry set, EBADF\n");
    } else {
        say("a64-static: write to fd 99 did not fail with EBADF\n");
        failed++;
    }
    r = poly_syscall6(SYS_write, 1, (long)"a64-static: write returns its count\n", 36, 0, 0, 0);
    if (r.failed || r.value != 36) failed++;
    say(failed ? "a64-static: failed\n" : "a64-static: done\n");
    poly_syscall3(SYS_exit, failed, 0, 0);
    for (;;) {
    }
}

__asm__(".globl start\nstart:\n  mov x0, sp\n  bl _poly_static_main\n  brk #1\n");

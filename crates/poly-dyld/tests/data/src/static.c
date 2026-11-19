/* A static test program (LC_UNIXTHREAD, no dyld): the kernel starts it at
 * `start` with argc at the stack pointer, argv and envp after it. */
#include "sys.h"

static const char *const words[] = {"static: ", "ok\n"};

__attribute__((noreturn)) void poly_static_main(long *sp) {
    long argc = sp[0];
    char **argv = (char **)(sp + 1);
    const char *w = (argc >= 1 && argv[0] && argv[argc] == 0) ? words[1] : "bad stack\n";
    poly_syscall3(SYS_write, 1, (long)words[0], (long)poly_strlen(words[0]));
    poly_syscall3(SYS_write, 1, (long)w, (long)poly_strlen(w));
    poly_syscall3(SYS_exit, w == words[1] ? 0 : 1, 0, 0);
    for (;;) {
    }
}

#if defined(__x86_64__)
__asm__(".globl start\nstart:\n  movq %rsp, %rdi\n  andq $-16, %rsp\n  call _poly_static_main\n  ud2\n");
#else
__asm__(".globl start\nstart:\n  mov x0, sp\n  bl _poly_static_main\n  brk #1\n");
#endif

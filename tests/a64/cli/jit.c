/* A program that writes its own code (ADR 0257 addendum): MAP_JIT memory
 * flipped between writable and executable with
 * pthread_jit_write_protect_np, an ordinary page flipped with mprotect,
 * and code rewritten in a loop. The translator must notice every rewrite
 * and drop what it made from the old code. Instruction words from the Arm
 * Architecture Reference Manual: MOVZ W0, #imm16 is 0x52800000 | imm << 5,
 * RET is 0xd65f03c0. Written for Poly. */
#include <libkern/OSCacheControl.h>
#include <pthread.h>
#include <setjmp.h>
#include <signal.h>
#include <stdint.h>
#include <stdio.h>
#include <string.h>
#include <sys/mman.h>
#include <unistd.h>

static int failures;
static void check(const char *what, int ok) {
    printf("%s %s\n", ok ? "ok" : "not ok", what);
    if (!ok) failures++;
}

typedef int (*fn)(void);

static void emit(uint32_t *p, int v) {
    p[0] = 0x52800000u | ((uint32_t)v << 5);
    p[1] = 0xd65f03c0u;
}

static sigjmp_buf recover;
static volatile int fault_signo;
static void on_fault(int sig) {
    fault_signo = sig;
    siglongjmp(recover, 1);
}

int main(void) {
    printf("jit: code written at run time\n");
    size_t page = (size_t)getpagesize();

    uint32_t *j = mmap(NULL, page, PROT_READ | PROT_WRITE | PROT_EXEC, MAP_PRIVATE | MAP_ANON | MAP_JIT, -1, 0);
    check("MAP_JIT memory maps", j != MAP_FAILED && pthread_jit_write_protect_supported_np() == 1);
    pthread_jit_write_protect_np(0);
    emit(j, 1);
    pthread_jit_write_protect_np(1);
    sys_icache_invalidate(j, 8);
    int a = ((fn)j)();
    pthread_jit_write_protect_np(0);
    emit(j, 2);
    pthread_jit_write_protect_np(1);
    sys_icache_invalidate(j, 8);
    int b = ((fn)j)();
    check("MAP_JIT code runs, and runs again rewritten", a == 1 && b == 2);

    signal(SIGBUS, on_fault);
    fault_signo = 0;
    if (sigsetjmp(recover, 1) == 0) {
        j[0] = 0;
        check("MAP_JIT memory is not writable in execute mode", 0);
    }
    check("a write in execute mode is SIGBUS", fault_signo == SIGBUS);

    uint32_t *m = mmap(NULL, page, PROT_READ | PROT_WRITE, MAP_PRIVATE | MAP_ANON, -1, 0);
    emit(m, 3);
    mprotect(m, page, PROT_READ | PROT_EXEC);
    int c = ((fn)m)();
    mprotect(m, page, PROT_READ | PROT_WRITE);
    emit(m, 4);
    mprotect(m, page, PROT_READ | PROT_EXEC);
    int d = ((fn)m)();
    check("mprotect flips: code runs, and runs again rewritten", c == 3 && d == 4);

    long sum = 0;
    for (int i = 0; i < 200; i++) {
        pthread_jit_write_protect_np(0);
        emit(j, i);
        pthread_jit_write_protect_np(1);
        sys_icache_invalidate(j, 8);
        for (int k = 0; k < 10; k++) sum += ((fn)j)();
    }
    check("code rewritten 200 times, each run 10 times", sum == 10L * 199 * 200 / 2);

    printf("jit: done, %d failed\n", failures);
    return failures;
}

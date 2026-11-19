/* ulock_wait2, BSD 544 (operation, address, value, timeout in nanoseconds,
 * second value): like ulock_wait with a timeout in nanoseconds: a matching
 * word waits and fails with ETIMEDOUT, a different one returns 0 at once;
 * ULF_NO_ERRNO returns -ETIMEDOUT. */
#include "poly_mtest.h"

static __attribute__((unused)) volatile u32 nap_word;
static __attribute__((unused)) void nap_ms(int ms) { bsd4(SYS_ulock_wait, UL_COMPARE_AND_WAIT, (i64)&nap_word, 0, ms * 1000); }
static __attribute__((unused)) i64 page_size(void) {
    i64 ps = 0;
    size_t len = sizeof ps;
    bsd6(SYS_sysctlbyname, "hw.pagesize", 11, &ps, &len, 0, 0);
    return ps;
}
/* A word that a forked child and its parent both see. */
static __attribute__((unused)) volatile u32 *shared_word(void) {
    volatile u32 *w = (u32 *)bsd6(SYS_mmap, 0, page_size(), PROT_READ | PROT_WRITE, MAP_ANON | MAP_SHARED, -1, 0).ret;
    w[0] = 5;
    return w;
}
/* The child waits on the shared word (after touching its page: the kernel
 * does not fault the page in for a shared lock) and exits with the errno of
 * the wait, 0 if it was woken. */
static __attribute__((unused)) i64 start_waiter(volatile u32 *w, int op) {
    sysret f = bsd0(SYS_fork);
    if (f.ret2 == 1) {
        u32 touch = w[0];
        (void)touch;
        sysret r = bsd4(SYS_ulock_wait, op, (i64)w, 5, 3000000);
        poly_exit(r.err ? (int)r.err : 0);
    }
    return f.ret;
}
/* Wake until a waiter was there to wake: the result of that wake. */
static __attribute__((unused)) sysret wake_when_waiting(volatile u32 *w, int op) {
    sysret k = {0, 0, ENOENT};
    for (int i = 0; i < 2000 && k.err == ENOENT; i++) {
        k = bsd3(SYS_ulock_wake, op, (i64)w, 0);
        if (k.err == ENOENT) nap_ms(5);
    }
    return k;
}

static volatile u32 word = 5;

int test_main(void) {
    check_err("the word equals the value: ETIMEDOUT", bsd6(SYS_ulock_wait2, UL_COMPARE_AND_WAIT, (i64)&word, 5, 20000000, 0, 0), ETIMEDOUT);
    sysret r = bsd6(SYS_ulock_wait2, UL_COMPARE_AND_WAIT, (i64)&word, 6, 20000000, 0, 0);
    check("the word differs: 0 at once", r.err == 0 && r.ret == 0, r.err ? r.err : r.ret);
    r = bsd6(SYS_ulock_wait2, UL_COMPARE_AND_WAIT | ULF_NO_ERRNO, (i64)&word, 5, 20000000, 0, 0);
    check("ULF_NO_ERRNO: -ETIMEDOUT, no carry", r.err == 0 && r.ret == -ETIMEDOUT, r.ret);
    check_err("the shared operation: ETIMEDOUT", bsd6(SYS_ulock_wait2, UL_COMPARE_AND_WAIT_SHARED, (i64)&word, 5, 20000000, 0, 0), ETIMEDOUT);
#if defined(POLY_ARM64)
    u64 t0 = poly_mach_now();
    bsd6(SYS_ulock_wait2, UL_COMPARE_AND_WAIT, (i64)&word, 5, 20000000, 0, 0);
    u64 t1 = poly_mach_now();
    check("the wait took at least 15 ms", t1 - t0 >= 24000 * 15, (i64)(t1 - t0));
#endif
    return finish();
}

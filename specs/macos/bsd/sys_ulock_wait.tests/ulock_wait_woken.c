/* ulock_wait, BSD 515, between processes: UL_COMPARE_AND_WAIT_SHARED (3) on
 * memory two processes share; a wait that a ulock_wake ends returns without
 * an error. A forked child waits, the parent wakes it. */
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

int test_main(void) {
    volatile u32 *w = shared_word();
    i64 child = start_waiter(w, UL_COMPARE_AND_WAIT_SHARED);
    sysret k = wake_when_waiting(w, UL_COMPARE_AND_WAIT_SHARED);
    check("the wake found the waiter and returned 0", k.err == 0 && k.ret == 0, k.err);
    int st = poly_wait(child);
    check("the waiter's wait ended without an error", W_EXITED(st) && W_EXITSTATUS(st) == 0, st);
    check_err("with nobody waiting: the next wake is ENOENT", bsd3(SYS_ulock_wake, UL_COMPARE_AND_WAIT_SHARED, (i64)w, 0), ENOENT);
    return finish();
}

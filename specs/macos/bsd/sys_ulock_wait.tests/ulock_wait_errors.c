/* ulock_wait, BSD 515, errors: EINVAL for an operation that does not exist
 * (0, 99), for an address that is not 4-byte aligned and for a NULL address;
 * EFAULT for an address that is not mapped (8). Value: UL_UNFAIR_LOCK (2) with
 * a value that is not the id of a thread (EOWNERDEAD on the oracle). */
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
    check_err("operation 0: EINVAL", bsd4(SYS_ulock_wait, 0, (i64)&word, 5, 1000), EINVAL);
    check_err("operation 99: EINVAL", bsd4(SYS_ulock_wait, 99, (i64)&word, 5, 1000), EINVAL);
    check_err("an unaligned address: EINVAL", bsd4(SYS_ulock_wait, UL_COMPARE_AND_WAIT, (i64)&word + 1, 5, 1000), EINVAL);
    check_err("a NULL address: EINVAL", bsd4(SYS_ulock_wait, UL_COMPARE_AND_WAIT, 0, 5, 1000), EINVAL);
    check_err("the address 8: EFAULT", bsd4(SYS_ulock_wait, UL_COMPARE_AND_WAIT, 8, 5, 1000), EFAULT);
    value("UL_UNFAIR_LOCK with an owner that is not a thread: errno", bsd4(SYS_ulock_wait, UL_UNFAIR_LOCK, (i64)&word, 5, 1000).err);
    check_err("a good wait afterwards", bsd4(SYS_ulock_wait, UL_COMPARE_AND_WAIT, (i64)&word, 5, 1000), ETIMEDOUT);
    return finish();
}

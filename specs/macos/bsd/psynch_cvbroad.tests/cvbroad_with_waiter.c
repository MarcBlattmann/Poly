/* psynch_cvbroad, BSD 303: the broadcast does not end a wait that is not on the same word: a cvwait on another condition variable still times out.
 * The call takes eight arguments, which the test header can pass on arm64
 * only: on x86_64 this test checks nothing yet. */
#include "poly_mtest.h"

#if defined(POLY_ARM64)
static __attribute__((unused)) volatile u32 words[64];
/* psynch_cvwait(cv, cvlsgen, cvugen, mutex, mugen, flags, sec, nsec) */
static __attribute__((unused)) sysret cvwait(i64 cv, i64 mutex, i64 sec, i64 nsec) { return poly_bsd8(SYS_psynch_cvwait, cv, 0x100, 0, mutex, 0, 0, sec, nsec); }
/* psynch_cvbroad(cv, cvlsgen, cvudgen, flags, mutex, mugen, tid, 0) */
static __attribute__((unused)) sysret cvbroad(i64 cv) { return poly_bsd8(SYS_psynch_cvbroad, cv, 0x100, 0, 0, 0, 0, 0, 0); }
/* psynch_cvsignal(cv, cvlsgen, cvugen, thread port, mutex, mugen, tid, flags) */
static __attribute__((unused)) sysret cvsignal(i64 cv) { return poly_bsd8(SYS_psynch_cvsignal, cv, 0x100, 0, 0, 0, 0, 0, 0); }
/* psynch_mutexwait / psynch_mutexdrop(mutex, mgen, ugen, tid, flags) */
static __attribute__((unused)) sysret mxwait(i64 mutex) { return poly_bsd8(SYS_psynch_mutexwait, mutex, 0, 0, 0, 0, 0, 0, 0); }
static __attribute__((unused)) sysret mxdrop(i64 mutex) { return poly_bsd8(SYS_psynch_mutexdrop, mutex, 0, 0, 0, 0, 0, 0, 0); }
#endif

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

#if defined(POLY_ARM64)
int test_main(void) {
    check_ok("broadcast on one word", cvbroad((i64)words));
    sysret r = cvwait((i64)(words + 16), (i64)(words + 8), 0, 10000000);
    check("a wait on another word still times out", r.err != 0 && (r.err & 0xff) == ETIMEDOUT, r.err);
    return finish();
}
#else
int test_main(void) { return finish(); }
#endif

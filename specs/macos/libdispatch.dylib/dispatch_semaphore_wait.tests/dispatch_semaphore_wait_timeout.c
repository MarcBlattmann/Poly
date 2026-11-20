/* dispatch_semaphore_wait: 0 when a token was taken, nonzero (the library
 * says timeout) when the time ran out; the wait lasts at least the time. */
/* poly-mac: libsystem */
#include "poly_mgcd.h"

int test_main(void) {
    dispatch_semaphore_t s = dispatch_semaphore_create(0);
    long long t0 = gcd_now_ns();
    long r = dispatch_semaphore_wait(s, dispatch_time(DISPATCH_TIME_NOW, 100 * NSEC_PER_MSEC));
    long long dt = gcd_now_ns() - t0;
    check("100 ms, no token: nonzero", r != 0, r);
    check("it waited at least 100 ms", dt >= 100 * 1000000LL, dt / 1000000);
    check("and not a second more", dt < 1100 * 1000000LL, dt / 1000000);
    value("the timeout result", r);

    r = dispatch_semaphore_wait(s, DISPATCH_TIME_NOW);
    check("DISPATCH_TIME_NOW with no token returns at once, nonzero", r != 0, r);

    dispatch_semaphore_signal(s);
    t0 = gcd_now_ns();
    r = dispatch_semaphore_wait(s, dispatch_time(DISPATCH_TIME_NOW, 5 * NSEC_PER_SEC));
    check("with a token present: 0", r == 0, r);
    check("immediately", gcd_now_ns() - t0 < 1000 * 1000000LL, (gcd_now_ns() - t0) / 1000000);

    check("a timeout in the past (DISPATCH_TIME_NOW - 1 s) behaves like NOW", dispatch_semaphore_wait(s, dispatch_time(DISPATCH_TIME_NOW, -1 * (int64_t)NSEC_PER_SEC)) != 0, 0);

    dispatch_semaphore_t f = dispatch_semaphore_create(0);
    dispatch_after(dispatch_time(DISPATCH_TIME_NOW, 50 * NSEC_PER_MSEC), dispatch_get_global_queue(QOS_CLASS_DEFAULT, 0), ^{ dispatch_semaphore_signal(f); });
    check("DISPATCH_TIME_FOREVER waits until a signal arrives", dispatch_semaphore_wait(f, DISPATCH_TIME_FOREVER) == 0, 0);
    return finish();
}

/* dispatch_after with a time that has passed (DISPATCH_TIME_NOW or before)
 * runs the block as soon as the queue is free, after earlier blocks of
 * a serial queue. */
/* poly-mac: libsystem */
#include "poly_mgcd.h"

int test_main(void) {
    dispatch_queue_t q = dispatch_queue_create("com.poly.after-now", DISPATCH_QUEUE_SERIAL);
    dispatch_semaphore_t done = dispatch_semaphore_create(0), gate = dispatch_semaphore_create(0);
    static int order[4];
    __block int n = 0;
    dispatch_async(q, ^{ gcd_wait(gate); order[n++] = 1; });
    dispatch_after(DISPATCH_TIME_NOW, q, ^{ order[n++] = 2; dispatch_semaphore_signal(done); });
    dispatch_semaphore_signal(gate);
    check("a DISPATCH_TIME_NOW block ran", gcd_wait(done), 0);
    check("after the earlier async block of the serial queue", n == 2 && order[0] == 1 && order[1] == 2, n);

    long long t0 = gcd_now_ns();
    dispatch_after(dispatch_time(DISPATCH_TIME_NOW, -1 * (int64_t)NSEC_PER_SEC), q, ^{ dispatch_semaphore_signal(done); });
    check("a time one second in the past: the block runs", gcd_wait(done), 0);
    check("promptly", gcd_now_ns() - t0 < 2000 * 1000000LL, (gcd_now_ns() - t0) / 1000000);

    dispatch_after(dispatch_walltime(NULL, 0), q, ^{ dispatch_semaphore_signal(done); });
    check("dispatch_walltime(NULL, 0) (now on the wall clock): the block runs", gcd_wait(done), 0);
    return finish();
}

/* dispatch_group_wait: the result is 0 when the group emptied in time,
 * nonzero on timeout; after a timeout the group still tracks its blocks. */
/* poly-mac: libsystem */
#include "poly_mgcd.h"

int test_main(void) {
    dispatch_group_t g = dispatch_group_create();
    dispatch_semaphore_t release = dispatch_semaphore_create(0);
    dispatch_group_async(g, dispatch_get_global_queue(QOS_CLASS_DEFAULT, 0), ^{ gcd_wait(release); });
    long long t0 = gcd_now_ns();
    long r = dispatch_group_wait(g, dispatch_time(DISPATCH_TIME_NOW, 100 * NSEC_PER_MSEC));
    long long dt = gcd_now_ns() - t0;
    check("a block that waits: wait times out (nonzero)", r != 0, r);
    check("after at least the 100 ms", dt >= 100 * 1000000LL, dt / 1000000);
    check("the timeout result is the same as a semaphore's", r == dispatch_semaphore_wait(dispatch_semaphore_create(0), DISPATCH_TIME_NOW), r);
    check("DISPATCH_TIME_NOW on a busy group: nonzero at once", dispatch_group_wait(g, DISPATCH_TIME_NOW) != 0, 0);
    dispatch_semaphore_signal(release);
    check("after the block finished: 0", dispatch_group_wait(g, dispatch_time(DISPATCH_TIME_NOW, 10 * NSEC_PER_SEC)) == 0, 0);
    return finish();
}

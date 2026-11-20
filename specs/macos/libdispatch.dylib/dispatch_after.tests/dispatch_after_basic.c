/* dispatch_after: the block runs on the queue no earlier than the time
 * given, and later blocks (by time) run later. Only lower bounds are
 * asserted: the time a block really runs at is not specified. */
/* poly-mac: libsystem */
#include "poly_mgcd.h"

int test_main(void) {
    dispatch_queue_t q = dispatch_queue_create("com.poly.after", DISPATCH_QUEUE_SERIAL);
    dispatch_semaphore_t done = dispatch_semaphore_create(0);
    __block long long ran_at = 0, label_ok = 0;
    long long t0 = gcd_now_ns();
    dispatch_after(dispatch_time(DISPATCH_TIME_NOW, 100 * NSEC_PER_MSEC), q, ^{
        ran_at = gcd_now_ns();
        label_ok = strcmp(dispatch_queue_get_label(DISPATCH_CURRENT_QUEUE_LABEL), "com.poly.after") == 0;
        dispatch_semaphore_signal(done);
    });
    check("dispatch_after returns at once (the block has not run)", ran_at == 0, 0);
    check("the block ran", gcd_wait(done), 0);
    check("not before 100 ms", ran_at - t0 >= 100 * 1000000LL, (ran_at - t0) / 1000000);
    check("not seconds later", ran_at - t0 < 3000 * 1000000LL, (ran_at - t0) / 1000000);
    check("on the queue given", label_ok, 0);

    static int order[3];
    __block int n = 0;
    dispatch_after(dispatch_time(DISPATCH_TIME_NOW, 300 * NSEC_PER_MSEC), q, ^{ order[n++] = 3; dispatch_semaphore_signal(done); });
    dispatch_after(dispatch_time(DISPATCH_TIME_NOW, 100 * NSEC_PER_MSEC), q, ^{ order[n++] = 1; dispatch_semaphore_signal(done); });
    dispatch_after(dispatch_time(DISPATCH_TIME_NOW, 200 * NSEC_PER_MSEC), q, ^{ order[n++] = 2; dispatch_semaphore_signal(done); });
    for (int i = 0; i < 3; i++) gcd_wait(done);
    check("three blocks set 300, 100 and 200 ms apart ran in time order", n == 3 && order[0] == 1 && order[1] == 2 && order[2] == 3, n);
    return finish();
}

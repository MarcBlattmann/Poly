/* dispatch_set_target_queue(object, queue): changes the queue a queue runs
 * on; a queue retargeted to a serial queue is serialised with it. A NULL
 * target puts the queue back to the default global queue. */
/* poly-mac: libsystem */
#include "poly_mgcd.h"

/* 1 when a block of `a` can be running while a block of `b` runs (they are independent
 * queues); 0 when the two are serialised (a's block gives b's 300 ms to start and it cannot). */
static int independent(dispatch_queue_t a, dispatch_queue_t b) {
    dispatch_semaphore_t b_started = dispatch_semaphore_create(0), done = dispatch_semaphore_create(0);
    __block int overlapped = 0;
    dispatch_async(a, ^{
        overlapped = dispatch_semaphore_wait(b_started, dispatch_time(DISPATCH_TIME_NOW, 300 * NSEC_PER_MSEC)) == 0;
        dispatch_semaphore_signal(done);
    });
    dispatch_async(b, ^{
        dispatch_semaphore_signal(b_started);
        dispatch_semaphore_signal(done);
    });
    gcd_wait(done);
    gcd_wait(done);
    return overlapped;
}

int test_main(void) {
    dispatch_queue_t a = dispatch_queue_create("com.poly.a", DISPATCH_QUEUE_SERIAL);
    dispatch_queue_t b = dispatch_queue_create("com.poly.b", DISPATCH_QUEUE_SERIAL);
    check("two independent serial queues run blocks at the same time", independent(a, b), 0);
    dispatch_queue_t t = dispatch_queue_create("com.poly.t", DISPATCH_QUEUE_SERIAL);
    dispatch_set_target_queue(a, t);
    dispatch_set_target_queue(b, t);
    check("both retargeted to one serial queue: serialised, never at the same time", !independent(a, b), 0);
    dispatch_set_target_queue(a, NULL);
    dispatch_set_target_queue(b, NULL);
    check("with a NULL target again: independent", independent(a, b), 0);
    return finish();
}

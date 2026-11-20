/* A group's count may be raised from several threads; the notify block
 * runs once, after the last leave. */
/* poly-mac: libsystem */
#include "poly_mgcd.h"

int test_main(void) {
    dispatch_group_t g = dispatch_group_create();
    dispatch_queue_t q = dispatch_get_global_queue(QOS_CLASS_DEFAULT, 0);
    dispatch_semaphore_t done = dispatch_semaphore_create(0);
    __block int notified = 0, finished_when_notified = -1, finished = 0;
    for (int t = 0; t < 4; t++) dispatch_group_enter(g);
    dispatch_group_notify(g, q, ^{
        notified++;
        finished_when_notified = __atomic_load_n(&finished, __ATOMIC_SEQ_CST);
        dispatch_semaphore_signal(done);
    });
    for (int t = 0; t < 4; t++)
        dispatch_async(q, ^{
            gcd_pause_ms(10 * (t + 1));
            __atomic_add_fetch(&finished, 1, __ATOMIC_SEQ_CST);
            dispatch_group_leave(g);
        });
    check("the notify block ran", gcd_wait(done), 0);
    check("after all four leaves", finished_when_notified == 4, finished_when_notified);
    gcd_pause_ms(50);
    check("exactly once", notified == 1, notified);
    return finish();
}

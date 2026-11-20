/* dispatch_sync from a block into a different queue works and nests;
 * dispatch_sync to a queue from inside it deadlocks (not tested: the
 * process would be killed by the watchdog). A sync call from another
 * thread waits until the queue is free. */
/* poly-mac: libsystem */
#include "poly_mgcd.h"

int test_main(void) {
    dispatch_queue_t a = dispatch_queue_create("com.poly.a", DISPATCH_QUEUE_SERIAL);
    dispatch_queue_t b = dispatch_queue_create("com.poly.b", DISPATCH_QUEUE_SERIAL);
    static int trace[4]; __block int n = 0;
    dispatch_sync(a, ^{
        trace[n++] = 1;
        dispatch_sync(b, ^{ trace[n++] = 2; });
        trace[n++] = 3;
    });
    check("nested syncs into another queue run in call order", n == 3 && trace[0] == 1 && trace[1] == 2 && trace[2] == 3, n);

    /* A sync call from a worker thread into a busy queue waits for it. */
    dispatch_semaphore_t holding = dispatch_semaphore_create(0), release = dispatch_semaphore_create(0), done = dispatch_semaphore_create(0);
    __block int order = 0, a_done_at = 0, sync_ran_at = 0;
    dispatch_async(a, ^{
        dispatch_semaphore_signal(holding);
        gcd_wait(release);
        a_done_at = ++order;
    });
    gcd_wait(holding);
    dispatch_async(dispatch_get_global_queue(QOS_CLASS_DEFAULT, 0), ^{
        dispatch_sync(a, ^{ sync_ran_at = ++order; });
        dispatch_semaphore_signal(done);
    });
    gcd_pause_ms(100); /* gives the sync call every chance to wrongly run early */
    check("the sync block did not run while the queue was busy", sync_ran_at == 0, sync_ran_at);
    dispatch_semaphore_signal(release);
    check("it ran when the queue was free", gcd_wait(done), 0);
    check("after the busy block", a_done_at == 1 && sync_ran_at == 2, sync_ran_at);
    return finish();
}

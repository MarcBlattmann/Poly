/* dispatch_async returns at once; the block runs later on another thread;
 * the block is copied (captured values are those at submission time) and
 * blocks of one serial queue keep their order. */
/* poly-mac: libsystem */
#include "poly_mgcd.h"

int test_main(void) {
    dispatch_queue_t q = dispatch_queue_create("com.poly.async", DISPATCH_QUEUE_SERIAL);
    dispatch_semaphore_t gate = dispatch_semaphore_create(0), done = dispatch_semaphore_create(0);
    __block int ran = 0;
    __block long long tid = 0;
    dispatch_async(q, ^{
        gcd_wait(gate); /* holds the queue until the test lets go */
        ran = 1;
        tid = gcd_current_thread_id();
        dispatch_semaphore_signal(done);
    });
    check("dispatch_async returned while its block is still blocked (not run inline)", ran == 0, ran);
    dispatch_semaphore_signal(gate);
    check("the block ran", gcd_wait(done), 0);
    check("on a thread other than the caller's", ran == 1 && tid != gcd_current_thread_id(), 0);

    int value_at_submit = 7;
    dispatch_semaphore_t gate2 = dispatch_semaphore_create(0);
    __block int seen = 0;
    dispatch_async(q, ^{
        gcd_wait(gate2);
        seen = value_at_submit;
        dispatch_semaphore_signal(done);
    });
    value_at_submit = 8;
    dispatch_semaphore_signal(gate2);
    gcd_wait(done);
    check("a captured (non __block) value is the one at submission time", seen == 7, seen);

    static int order[100]; __block int n = 0;
    for (int i = 0; i < 100; i++) dispatch_async(q, ^{ order[n++] = i; if (i == 99) dispatch_semaphore_signal(done); });
    gcd_wait(done);
    int ok = n == 100;
    for (int i = 0; i < 100 && ok; i++) ok = order[i] == i;
    check("100 blocks on a serial queue ran in order", ok, n);
    return finish();
}

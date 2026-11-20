/* dispatch_semaphore_signal returns 0 when no thread was woken and nonzero
 * when it woke one; signals are counted, not merged. */
/* poly-mac: libsystem */
#include "poly_mgcd.h"

int test_main(void) {
    dispatch_semaphore_t s = dispatch_semaphore_create(0);
    long r = dispatch_semaphore_signal(s);
    check("signal with nobody waiting: 0", r == 0, r);
    dispatch_semaphore_signal(s);
    dispatch_semaphore_signal(s);
    int got = 0;
    for (int i = 0; i < 4; i++) got += dispatch_semaphore_wait(s, DISPATCH_TIME_NOW) == 0;
    check("three signals are three tokens", got == 3, got);

    dispatch_semaphore_t t = dispatch_semaphore_create(0), in = dispatch_semaphore_create(0), done = dispatch_semaphore_create(0);
    dispatch_async(dispatch_get_global_queue(QOS_CLASS_DEFAULT, 0), ^{
        dispatch_semaphore_signal(in);
        dispatch_semaphore_wait(t, DISPATCH_TIME_FOREVER);
        dispatch_semaphore_signal(done);
    });
    gcd_wait(in);
    gcd_pause_ms(100); /* the waiter is blocked in the wait by now */
    r = dispatch_semaphore_signal(t);
    check("signal that wakes a waiter: nonzero", r != 0, r);
    check("the waiter proceeded", gcd_wait(done), 0);
    value("the result of a waking signal", r);
    return finish();
}

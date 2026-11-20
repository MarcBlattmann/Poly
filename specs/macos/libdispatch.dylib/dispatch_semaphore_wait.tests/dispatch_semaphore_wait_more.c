/* The waiting thread does not burn its time: while a block waits on a
 * semaphore, other work on the same queue's threads goes on. */
/* poly-mac: libsystem */
#include "poly_mgcd.h"

int test_main(void) {
    dispatch_semaphore_t s = dispatch_semaphore_create(0), done = dispatch_semaphore_create(0);
    dispatch_queue_t g = dispatch_get_global_queue(QOS_CLASS_DEFAULT, 0);
    __block int others = 0;
    for (int i = 0; i < 16; i++)
        dispatch_async(g, ^{
            dispatch_semaphore_wait(s, DISPATCH_TIME_FOREVER);
            dispatch_semaphore_signal(done);
        });
    for (int i = 0; i < 100; i++)
        dispatch_async(g, ^{
            __atomic_add_fetch(&others, 1, __ATOMIC_SEQ_CST);
            dispatch_semaphore_signal(done);
        });
    int n = 0;
    for (int i = 0; i < 100; i++) n += gcd_wait(done);
    check("100 other blocks finished while 16 blocks sat in a semaphore wait", n == 100 && others == 100, others);
    for (int i = 0; i < 16; i++) dispatch_semaphore_signal(s);
    int m = 0;
    for (int i = 0; i < 16; i++) m += gcd_wait(done);
    check("the 16 waiters finished after 16 signals", m == 16, m);
    return finish();
}

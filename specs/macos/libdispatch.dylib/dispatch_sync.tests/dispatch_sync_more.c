/* dispatch_sync on a concurrent queue runs alongside other work; the
 * result of the block can be passed out through a __block variable. */
/* poly-mac: libsystem */
#include "poly_mgcd.h"

int test_main(void) {
    dispatch_queue_t c = dispatch_queue_create("com.poly.c", DISPATCH_QUEUE_CONCURRENT);
    dispatch_semaphore_t holding = dispatch_semaphore_create(0), release = dispatch_semaphore_create(0), done = dispatch_semaphore_create(0);
    dispatch_async(c, ^{
        dispatch_semaphore_signal(holding);
        gcd_wait(release);
        dispatch_semaphore_signal(done);
    });
    gcd_wait(holding);
    __block int ran = 0;
    dispatch_sync(c, ^{ ran = 1; });
    check("a sync block on a concurrent queue runs while an async block is still running", ran == 1, ran);
    dispatch_semaphore_signal(release);
    gcd_wait(done);

    __block int result = 0;
    for (int i = 1; i <= 10; i++) dispatch_sync(c, ^{ result += i; });
    check("ten sync calls in a row: 55", result == 55, result);
    return finish();
}

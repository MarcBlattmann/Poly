/* A concurrent queue is suspended too; a queue released while suspended
 * is kept alive by its suspension (resume it first). The global queues
 * ignore suspend. */
/* poly-mac: libsystem */
#include "poly_mgcd.h"

int test_main(void) {
    dispatch_queue_t c = dispatch_queue_create("com.poly.cs", DISPATCH_QUEUE_CONCURRENT);
    dispatch_semaphore_t done = dispatch_semaphore_create(0);
    __block int ran = 0;
    dispatch_suspend(c);
    for (int i = 0; i < 10; i++) dispatch_async(c, ^{ __atomic_add_fetch(&ran, 1, __ATOMIC_SEQ_CST); dispatch_semaphore_signal(done); });
    gcd_pause_ms(100);
    check("a suspended concurrent queue ran nothing", ran == 0, ran);
    dispatch_resume(c);
    int n = 0;
    for (int i = 0; i < 10; i++) n += gcd_wait(done);
    check("after resume all ten ran", n == 10 && ran == 10, ran);
    return finish();
}

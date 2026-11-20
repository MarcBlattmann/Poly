/* A serial queue runs its blocks one at a time, in the order they were
 * submitted, on a thread that is not the main thread. */
/* poly-mac: libsystem */
#include "poly_mgcd.h"

int test_main(void) {
    dispatch_queue_t q = dispatch_queue_create("com.poly.serial", DISPATCH_QUEUE_SERIAL);
    __block int next = 0, out_of_order = 0, overlap = 0, running = 0, on_main = 0;
    dispatch_semaphore_t done = dispatch_semaphore_create(0);
    for (int i = 0; i < 1000; i++) {
        dispatch_async(q, ^{
            if (__atomic_add_fetch(&running, 1, __ATOMIC_SEQ_CST) != 1) overlap++;
            if (i != next) out_of_order++;
            next++;
            if (gcd_on_main_thread()) on_main++;
            __atomic_sub_fetch(&running, 1, __ATOMIC_SEQ_CST);
            if (i == 999) dispatch_semaphore_signal(done);
        });
    }
    check("the last block ran", gcd_wait(done), 0);
    check("1000 blocks ran in submission order", out_of_order == 0 && next == 1000, out_of_order);
    check("never two at the same time", overlap == 0, overlap);
    check("never on the main thread (the main thread was not draining)", on_main == 0, on_main);
    return finish();
}

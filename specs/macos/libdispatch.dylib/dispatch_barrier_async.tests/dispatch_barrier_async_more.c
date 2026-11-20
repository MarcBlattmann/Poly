/* dispatch_barrier_async on a serial queue behaves as dispatch_async; on a
 * global queue it is also just an async block (no barrier). */
/* poly-mac: libsystem */
#include "poly_mgcd.h"

int test_main(void) {
    dispatch_queue_t s = dispatch_queue_create("com.poly.serial-barrier", DISPATCH_QUEUE_SERIAL);
    static int order[10]; __block int n = 0;
    dispatch_semaphore_t done = dispatch_semaphore_create(0);
    dispatch_async(s, ^{ order[n++] = 1; });
    dispatch_barrier_async(s, ^{ order[n++] = 2; });
    dispatch_async(s, ^{ order[n++] = 3; dispatch_semaphore_signal(done); });
    gcd_wait(done);
    check("on a serial queue the order is the submission order", n == 3 && order[0] == 1 && order[1] == 2 && order[2] == 3, n);

    dispatch_queue_t g = dispatch_get_global_queue(QOS_CLASS_DEFAULT, 0);
    dispatch_semaphore_t in = dispatch_semaphore_create(0), release = dispatch_semaphore_create(0);
    dispatch_barrier_async(g, ^{ dispatch_semaphore_signal(in); gcd_wait(release); dispatch_semaphore_signal(done); });
    gcd_wait(in);
    __block int ran = 0;
    dispatch_async(g, ^{ ran = 1; dispatch_semaphore_signal(done); });
    check("on a global queue a barrier block does not hold back later blocks", gcd_wait(done) && ran == 1, ran);
    dispatch_semaphore_signal(release);
    gcd_wait(done);
    return finish();
}

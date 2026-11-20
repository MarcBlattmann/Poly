/* The reader/writer pattern: many concurrent readers, writers as barriers;
 * a reader never sees a half-written pair. */
/* poly-mac: libsystem */
#include "poly_mgcd.h"
#include <sched.h>

int test_main(void) {
    dispatch_queue_t q = dispatch_queue_create("com.poly.rw", DISPATCH_QUEUE_CONCURRENT);
    static int a, b; /* the writer keeps a == b, but sets them one after the other */
    __block int torn = 0, reads = 0;
    dispatch_semaphore_t done = dispatch_semaphore_create(0);
    for (int i = 1; i <= 200; i++) {
        dispatch_barrier_async(q, ^{
            a = i;
            sched_yield();
            b = i;
        });
        for (int r = 0; r < 5; r++)
            dispatch_async(q, ^{
                int x = a, y = b;
                if (x != y) __atomic_add_fetch(&torn, 1, __ATOMIC_SEQ_CST);
                __atomic_add_fetch(&reads, 1, __ATOMIC_SEQ_CST);
                dispatch_semaphore_signal(done);
            });
    }
    for (int i = 0; i < 1000; i++) gcd_wait(done);
    check("1000 reads ran between 200 writes", reads == 1000, reads);
    check("no reader saw a half-written pair", torn == 0, torn);
    return finish();
}

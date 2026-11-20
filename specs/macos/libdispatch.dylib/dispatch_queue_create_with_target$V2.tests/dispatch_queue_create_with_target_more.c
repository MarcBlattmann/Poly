/* A concurrent queue aimed at a serial queue is serialised by it; a
 * serial queue aimed at a concurrent one is still serial. */
/* poly-mac: libsystem */
#include "poly_mgcd.h"

static int overlap_of(dispatch_queue_t q) {
    __block int running = 0, overlap = 0;
    dispatch_semaphore_t done = dispatch_semaphore_create(0);
    for (int i = 0; i < 300; i++)
        dispatch_async(q, ^{
            if (__atomic_add_fetch(&running, 1, __ATOMIC_SEQ_CST) != 1) overlap++;
            __atomic_sub_fetch(&running, 1, __ATOMIC_SEQ_CST);
            dispatch_semaphore_signal(done);
        });
    int n = 0;
    for (int i = 0; i < 300; i++) n += gcd_wait(done);
    return n == 300 ? overlap : -1;
}

int test_main(void) {
    dispatch_queue_t serial = dispatch_queue_create("com.poly.s", DISPATCH_QUEUE_SERIAL);
    dispatch_queue_t conc = dispatch_queue_create_with_target("com.poly.c", DISPATCH_QUEUE_CONCURRENT, serial);
    check("a concurrent queue aimed at a serial queue: all blocks ran, none overlapped", overlap_of(conc) == 0, overlap_of(conc));
    dispatch_queue_t wide = dispatch_queue_create("com.poly.wide", DISPATCH_QUEUE_CONCURRENT);
    dispatch_queue_t s2 = dispatch_queue_create_with_target("com.poly.s2", DISPATCH_QUEUE_SERIAL, wide);
    check("a serial queue aimed at a concurrent queue: all blocks ran, none overlapped", overlap_of(s2) == 0, overlap_of(s2));
    return finish();
}

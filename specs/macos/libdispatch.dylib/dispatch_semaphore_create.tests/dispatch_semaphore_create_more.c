/* A semaphore of 1 is a mutex between threads. */
/* poly-mac: libsystem */
#include "poly_mgcd.h"

int test_main(void) {
    dispatch_semaphore_t mutex = dispatch_semaphore_create(1);
    dispatch_semaphore_t done = dispatch_semaphore_create(0);
    __block long counter = 0;
    __block int inside = 0, collisions = 0;
    dispatch_queue_t g = dispatch_get_global_queue(QOS_CLASS_DEFAULT, 0);
    for (int t = 0; t < 8; t++)
        dispatch_async(g, ^{
            for (int i = 0; i < 5000; i++) {
                dispatch_semaphore_wait(mutex, DISPATCH_TIME_FOREVER);
                if (++inside != 1) collisions++;
                long v = counter;
                counter = v + 1;
                inside--;
                dispatch_semaphore_signal(mutex);
            }
            dispatch_semaphore_signal(done);
        });
    for (int t = 0; t < 8; t++) gcd_wait(done);
    check("8 threads x 5000 increments under a semaphore of 1: 40000", counter == 40000, counter);
    check("never two threads inside", collisions == 0, collisions);
    return finish();
}

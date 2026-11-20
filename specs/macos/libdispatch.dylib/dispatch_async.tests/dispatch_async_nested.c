/* A block may submit blocks; many threads may submit to one queue at the
 * same time and no block is lost or run twice. */
/* poly-mac: libsystem */
#include "poly_mgcd.h"

int test_main(void) {
    dispatch_queue_t q = dispatch_queue_create("com.poly.nested", DISPATCH_QUEUE_SERIAL);
    dispatch_semaphore_t done = dispatch_semaphore_create(0);
    static int trace[3]; __block int n = 0;
    dispatch_async(q, ^{
        trace[n++] = 1;
        dispatch_async(q, ^{
            trace[n++] = 3;
            dispatch_semaphore_signal(done);
        });
        trace[n++] = 2; /* the inner block cannot run before this one ends */
    });
    gcd_wait(done);
    check("a block submitted from a block of the same serial queue runs after it ends", n == 3 && trace[0] == 1 && trace[1] == 2 && trace[2] == 3, n);

    /* Several submitters. */
    dispatch_queue_t c = dispatch_get_global_queue(QOS_CLASS_DEFAULT, 0);
    __block int count = 0;
    dispatch_queue_t sink = dispatch_queue_create("com.poly.sink", DISPATCH_QUEUE_SERIAL);
    for (int t = 0; t < 8; t++)
        dispatch_async(c, ^{
            for (int i = 0; i < 250; i++) dispatch_async(sink, ^{ count++; });
            dispatch_semaphore_signal(done);
        });
    for (int t = 0; t < 8; t++) gcd_wait(done);
    dispatch_sync(sink, ^{}); /* everything submitted before is done after this */
    check("8 threads submitted 250 blocks each: 2000 ran, none lost or doubled", count == 2000, count);
    return finish();
}

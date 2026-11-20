/* A running event handler finishes after a cancel; the cancel handler
 * follows it. dispatch_source_cancel_and_wait style ordering. */
/* poly-mac: libsystem */
#include "poly_mgcd.h"

int test_main(void) {
    dispatch_queue_t q = dispatch_queue_create("com.poly.cancel-order", DISPATCH_QUEUE_SERIAL);
    dispatch_source_t s = dispatch_source_create(DISPATCH_SOURCE_TYPE_DATA_ADD, 0, 0, q);
    dispatch_semaphore_t in = dispatch_semaphore_create(0), go = dispatch_semaphore_create(0), cancelled = dispatch_semaphore_create(0);
    static int order[4];
    __block int n = 0;
    dispatch_source_set_event_handler(s, ^{
        order[n++] = 1; /* event handler starts */
        dispatch_semaphore_signal(in);
        gcd_wait(go);
        order[n++] = 2; /* and ends */
    });
    dispatch_source_set_cancel_handler(s, ^{ order[n++] = 3; dispatch_semaphore_signal(cancelled); });
    dispatch_resume(s);
    dispatch_source_merge_data(s, 1);
    gcd_wait(in);
    dispatch_source_cancel(s); /* while the event handler is running */
    dispatch_semaphore_signal(go);
    check("the cancel handler ran", gcd_wait(cancelled), 0);
    check("the order: event handler start, end, then the cancel handler", n == 3 && order[0] == 1 && order[1] == 2 && order[2] == 3, n);
    return finish();
}

/* dispatch_group_notify: the block is submitted to the given queue when
 * the group empties, not before; several notify blocks all run. */
/* poly-mac: libsystem */
#include "poly_mgcd.h"

int test_main(void) {
    dispatch_group_t g = dispatch_group_create();
    dispatch_queue_t q = dispatch_queue_create("com.poly.notify", DISPATCH_QUEUE_SERIAL);
    dispatch_semaphore_t done = dispatch_semaphore_create(0);
    __block int notified = 0, in_right_queue = 0;
    dispatch_group_enter(g);
    dispatch_group_notify(g, q, ^{
        notified = 1;
        in_right_queue = strcmp(dispatch_queue_get_label(DISPATCH_CURRENT_QUEUE_LABEL), "com.poly.notify") == 0;
        dispatch_semaphore_signal(done);
    });
    gcd_pause_ms(100);
    check("not run while the group is not empty", notified == 0, notified);
    dispatch_group_leave(g);
    check("run after the leave", gcd_wait(done) && notified == 1, notified);
    check("on the queue it was given", in_right_queue, 0);

    __block int count = 0;
    dispatch_group_enter(g);
    for (int i = 0; i < 3; i++) dispatch_group_notify(g, q, ^{ count++; dispatch_semaphore_signal(done); });
    dispatch_group_leave(g);
    for (int i = 0; i < 3; i++) gcd_wait(done);
    check("three notify blocks on one group all ran", count == 3, count);
    return finish();
}

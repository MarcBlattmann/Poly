/* dispatch_group_async: runs the block on the queue and tracks it in the
 * group; the group is empty when all its blocks have finished. */
/* poly-mac: libsystem */
#include "poly_mgcd.h"

int test_main(void) {
    dispatch_group_t g = dispatch_group_create();
    dispatch_queue_t q = dispatch_queue_create("com.poly.group", DISPATCH_QUEUE_CONCURRENT);
    __block int done_count = 0;
    for (int i = 0; i < 50; i++)
        dispatch_group_async(g, q, ^{
            gcd_pause_ms(2);
            __atomic_add_fetch(&done_count, 1, __ATOMIC_SEQ_CST);
        });
    long r = dispatch_group_wait(g, dispatch_time(DISPATCH_TIME_NOW, 10 * NSEC_PER_SEC));
    check("wait returns 0 once all blocks finished", r == 0, r);
    check("all 50 blocks had finished", done_count == 50, done_count);
    check("a second wait returns 0 at once", dispatch_group_wait(g, DISPATCH_TIME_NOW) == 0, 0);
    return finish();
}

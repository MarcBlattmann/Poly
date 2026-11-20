/* A group can be reused after it emptied. */
/* poly-mac: libsystem */
#include "poly_mgcd.h"

int test_main(void) {
    dispatch_group_t g = dispatch_group_create();
    dispatch_queue_t q = dispatch_get_global_queue(QOS_CLASS_DEFAULT, 0);
    __block int total = 0;
    for (int round = 0; round < 5; round++) {
        for (int i = 0; i < 10; i++) dispatch_group_async(g, q, ^{ __atomic_add_fetch(&total, 1, __ATOMIC_SEQ_CST); });
        long r = dispatch_group_wait(g, DISPATCH_TIME_FOREVER);
        check("each round: the group emptied", r == 0 && total == (round + 1) * 10, total);
    }
    return finish();
}

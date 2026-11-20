/* dispatch_apply on a concurrent queue spreads iterations over threads,
 * and the calling thread takes part. */
/* poly-mac: libsystem */
#include "poly_mgcd.h"

int test_main(void) {
    static long long tids[200];
    long long me = gcd_current_thread_id();
    dispatch_apply(200, dispatch_get_global_queue(QOS_CLASS_DEFAULT, 0), ^(size_t i) {
        gcd_pause_ms(1);
        tids[i] = gcd_current_thread_id();
    });
    int distinct = 0, on_caller = 0;
    for (int i = 0; i < 200; i++) {
        int seen = 0;
        for (int j = 0; j < i; j++) seen |= tids[j] == tids[i];
        distinct += !seen;
        on_caller += tids[i] == me;
    }
    check("the caller ran some iterations itself", on_caller > 0, on_caller);
    value("every iteration ran on some thread", 1);
    check("more than one thread took part (a slow body, 200 iterations)", distinct > 1, distinct);

    /* Nested applies do not deadlock. */
    __block int total = 0;
    dispatch_apply(8, dispatch_get_global_queue(QOS_CLASS_DEFAULT, 0), ^(size_t i) {
        dispatch_apply(8, dispatch_get_global_queue(QOS_CLASS_DEFAULT, 0), ^(size_t j) {
            __atomic_add_fetch(&total, (int)(i * 8 + j), __ATOMIC_SEQ_CST);
        });
    });
    check("an apply inside an apply: the sum of 0..63 is 2016", total == 2016, total);
    return finish();
}

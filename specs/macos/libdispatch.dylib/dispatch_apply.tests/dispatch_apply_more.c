/* dispatch_apply returns only when every iteration has finished; the
 * results written by the iterations are visible after it. */
/* poly-mac: libsystem */
#include "poly_mgcd.h"

int test_main(void) {
    static double out[4096];
    dispatch_apply(4096, dispatch_get_global_queue(QOS_CLASS_UTILITY, 0), ^(size_t i) { out[i] = (double)i * 2; });
    int bad = 0;
    for (int i = 0; i < 4096; i++) bad += out[i] != (double)i * 2;
    check("4096 results written by the iterations are all visible", bad == 0, bad);
    __block long long sum = 0;
    dispatch_apply(100000, dispatch_get_global_queue(QOS_CLASS_DEFAULT, 0), ^(size_t i) { __atomic_add_fetch(&sum, (long long)i, __ATOMIC_SEQ_CST); });
    check("100000 iterations: sum 4999950000", sum == 4999950000LL, sum);
    return finish();
}

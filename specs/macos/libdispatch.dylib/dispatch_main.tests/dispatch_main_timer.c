/* dispatch_after to the main queue fires while the main thread is in
 * dispatch_main, in time order. */
/* poly-mac: libsystem */
#include "poly_mgcd.h"
#include <stdlib.h>

static int order[4], n;
static long long t0;

int test_main(void) {
    t0 = gcd_now_ns();
    dispatch_after(dispatch_time(DISPATCH_TIME_NOW, 200 * NSEC_PER_MSEC), dispatch_get_main_queue(), ^{
        order[n++] = 2;
        check("the 200 ms timer: not early", gcd_now_ns() - t0 >= 200 * 1000000LL, (gcd_now_ns() - t0) / 1000000);
        check_str("on the main queue", dispatch_queue_get_label(DISPATCH_CURRENT_QUEUE_LABEL), "com.apple.main-thread");
        check("after the 50 ms one", n == 2 && order[0] == 1, n);
        exit(finish());
    });
    dispatch_after(dispatch_time(DISPATCH_TIME_NOW, 50 * NSEC_PER_MSEC), dispatch_get_main_queue(), ^{
        order[n++] = 1;
        check("the 50 ms timer: not early", gcd_now_ns() - t0 >= 50 * 1000000LL, (gcd_now_ns() - t0) / 1000000);
        value("on the original main thread (pthread_main_np)", gcd_on_main_thread());
    });
    dispatch_main();
}

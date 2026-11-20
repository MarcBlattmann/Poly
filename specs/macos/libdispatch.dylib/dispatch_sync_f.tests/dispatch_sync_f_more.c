/* dispatch_sync_f behind async work on a serial queue runs after it. */
/* poly-mac: libsystem */
#include "poly_mgcd.h"

static int log_[8], n;
static void add(void *p) { log_[n++] = (int)(long)p; }

int test_main(void) {
    dispatch_queue_t q = dispatch_queue_create("com.poly.order", DISPATCH_QUEUE_SERIAL);
    dispatch_async_f(q, (void *)1, add);
    dispatch_async_f(q, (void *)2, add);
    dispatch_sync_f(q, (void *)3, add);
    check("the sync function ran after the two async ones", n == 3 && log_[0] == 1 && log_[1] == 2 && log_[2] == 3, n);
    dispatch_async_f(q, (void *)4, add);
    dispatch_sync_f(q, (void *)5, add);
    check("and again", n == 5 && log_[3] == 4 && log_[4] == 5, n);
    return finish();
}

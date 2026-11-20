/* dispatch_async_f passes the context pointer unchanged, also NULL, and
 * the function sees the queue it was submitted to. */
/* poly-mac: libsystem */
#include "poly_mgcd.h"

static dispatch_semaphore_t done;
static void *got_ctx = (void *)1;
static char label[64];

static void probe(void *ctx) {
    got_ctx = ctx;
    strlcpy(label, dispatch_queue_get_label(DISPATCH_CURRENT_QUEUE_LABEL), sizeof label);
    dispatch_semaphore_signal(done);
}

int test_main(void) {
    done = dispatch_semaphore_create(0);
    dispatch_queue_t q = dispatch_queue_create("com.poly.f-label", DISPATCH_QUEUE_SERIAL);
    dispatch_async_f(q, NULL, probe);
    gcd_wait(done);
    check("a NULL context arrives as NULL", got_ctx == NULL, 0);
    check_str("the function runs on the queue it was submitted to", label, "com.poly.f-label");
    int x;
    dispatch_async_f(q, &x, probe);
    gcd_wait(done);
    check("a pointer arrives unchanged", got_ctx == &x, 0);
    dispatch_async_f(dispatch_get_global_queue(QOS_CLASS_UTILITY, 0), (void *)0xdeadbeef, probe);
    gcd_wait(done);
    check("any pointer value arrives unchanged (not dereferenced)", got_ctx == (void *)0xdeadbeef, 0);
    check_str("on the utility global queue", label, "com.apple.root.utility-qos");
    return finish();
}

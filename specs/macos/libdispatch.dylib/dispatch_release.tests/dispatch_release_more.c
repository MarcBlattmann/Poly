/* A finalizer set without a context does not run (the context is the
 * trigger); a source, group and semaphore are released the same way. */
/* poly-mac: libsystem */
#include "poly_mgcd.h"

static dispatch_semaphore_t finalized;
static int runs;
static void finalizer(void *ctx) { (void)ctx; runs++; dispatch_semaphore_signal(finalized); }

int test_main(void) {
    finalized = dispatch_semaphore_create(0);
    dispatch_queue_t q = dispatch_queue_create("com.poly.noctx", DISPATCH_QUEUE_SERIAL);
    dispatch_set_finalizer_f(q, finalizer);
    dispatch_release(q);
    check("a finalizer without a context (NULL) is not called", dispatch_semaphore_wait(finalized, dispatch_time(DISPATCH_TIME_NOW, 200 * NSEC_PER_MSEC)) != 0, runs);

    dispatch_group_t g = dispatch_group_create();
    dispatch_set_context(g, (void *)1);
    dispatch_set_finalizer_f(g, finalizer);
    dispatch_release(g);
    check("a group's finalizer runs when it is released", gcd_wait(finalized), 0);

    dispatch_semaphore_t s = dispatch_semaphore_create(0);
    dispatch_set_context(s, (void *)1);
    dispatch_set_finalizer_f(s, finalizer);
    dispatch_release(s);
    check("so does a semaphore's", gcd_wait(finalized), 0);
    check("three objects: one finalizer call each, two seen here", runs == 2, runs);
    return finish();
}

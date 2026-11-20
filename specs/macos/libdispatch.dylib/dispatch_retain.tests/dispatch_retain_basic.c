/* dispatch_retain / dispatch_release count references on a dispatch
 * object. The finalizer (dispatch_set_finalizer_f) runs on the object's
 * target queue when the last reference goes, with the object's context. */
/* poly-mac: libsystem */
#include "poly_mgcd.h"

static dispatch_semaphore_t finalized;
static void *finalizer_ctx;
static int finalizer_runs;
static void finalizer(void *ctx) {
    finalizer_ctx = ctx;
    finalizer_runs++;
    dispatch_semaphore_signal(finalized);
}

int test_main(void) {
    finalized = dispatch_semaphore_create(0);
    static int ctx;
    dispatch_queue_t q = dispatch_queue_create("com.poly.retain", DISPATCH_QUEUE_SERIAL);
    dispatch_set_context(q, &ctx);
    check("dispatch_get_context gives back what was set", dispatch_get_context(q) == &ctx, 0);
    dispatch_set_finalizer_f(q, finalizer);
    dispatch_retain(q);
    dispatch_retain(q);
    dispatch_release(q);
    dispatch_release(q);
    check("two retains and two releases: not finalized yet", dispatch_semaphore_wait(finalized, dispatch_time(DISPATCH_TIME_NOW, 100 * NSEC_PER_MSEC)) != 0, finalizer_runs);
    dispatch_release(q);
    check("the release of the creator's reference runs the finalizer", gcd_wait(finalized), 0);
    check("exactly once", finalizer_runs == 1, finalizer_runs);
    check("with the object's context", finalizer_ctx == &ctx, 0);

    /* A semaphore, a group and a queue are all dispatch objects. */
    dispatch_group_t g = dispatch_group_create();
    dispatch_retain(g);
    dispatch_release(g);
    dispatch_semaphore_t s = dispatch_semaphore_create(1);
    dispatch_retain(s);
    dispatch_release(s);
    check("retain/release of a group and a semaphore", 1, 0);
    dispatch_release(g);
    return finish();
}

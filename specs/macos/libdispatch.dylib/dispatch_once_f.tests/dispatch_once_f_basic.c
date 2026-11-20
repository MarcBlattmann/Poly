/* dispatch_once_f(predicate, context, function): the function runs once
 * with the context; callers wait for it. */
/* poly-mac: libsystem */
#include "poly_mgcd.h"

static dispatch_once_t once;
static int runs;
static void *seen_ctx;
static void init(void *ctx) {
    gcd_pause_ms(30);
    runs++;
    seen_ctx = ctx;
}

int test_main(void) {
    dispatch_semaphore_t done = dispatch_semaphore_create(0);
    static int token;
    dispatch_queue_t g = dispatch_get_global_queue(QOS_CLASS_DEFAULT, 0);
    __block int early = 0;
    for (int i = 0; i < 8; i++)
        dispatch_async(g, ^{
            dispatch_once_f(&once, &token, init);
            if (runs != 1) __atomic_add_fetch(&early, 1, __ATOMIC_SEQ_CST);
            dispatch_semaphore_signal(done);
        });
    for (int i = 0; i < 8; i++) gcd_wait(done);
    check("the function ran exactly once", runs == 1, runs);
    check("with the context of the first caller", seen_ctx == &token, 0);
    check("every caller returned after it finished", early == 0, early);
    dispatch_once_f(&once, NULL, init);
    check("a later call does nothing", runs == 1, runs);
    return finish();
}

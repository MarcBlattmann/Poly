/* dispatch_once: the block runs the first time only, whichever thread
 * calls; every caller returns after it has completed. The predicate is a
 * zero-initialised word. */
/* poly-mac: libsystem */
#include "poly_mgcd.h"

static dispatch_once_t once;
static int runs, finished;

int test_main(void) {
    check("the predicate starts as 0 (static storage)", once == 0, once);
    dispatch_semaphore_t done = dispatch_semaphore_create(0);
    __block int early_return = 0;
    dispatch_queue_t g = dispatch_get_global_queue(QOS_CLASS_DEFAULT, 0);
    for (int i = 0; i < 8; i++)
        dispatch_async(g, ^{
            dispatch_once(&once, ^{
                gcd_pause_ms(30); /* slow: the other callers must wait for it */
                runs++;
                finished = 1;
            });
            if (!finished) __atomic_add_fetch(&early_return, 1, __ATOMIC_SEQ_CST);
            dispatch_semaphore_signal(done);
        });
    for (int i = 0; i < 8; i++) gcd_wait(done);
    check("the block ran exactly once for 8 racing threads", runs == 1, runs);
    check("no caller returned before it had finished", early_return == 0, early_return);
    check("the predicate is nonzero afterwards", once != 0, once);
    dispatch_once(&once, ^{ runs++; });
    check("a later call runs nothing", runs == 1, runs);

    static dispatch_once_t other;
    dispatch_once(&other, ^{ runs++; });
    check("another predicate runs its block", runs == 2, runs);
    return finish();
}

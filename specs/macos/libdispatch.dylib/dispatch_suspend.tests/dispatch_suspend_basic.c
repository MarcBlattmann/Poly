/* dispatch_suspend / dispatch_resume: a suspended queue starts no new
 * block; blocks already running finish; resume lets queued blocks run in
 * order. Suspensions nest. */
/* poly-mac: libsystem */
#include "poly_mgcd.h"

int test_main(void) {
    dispatch_queue_t q = dispatch_queue_create("com.poly.suspend", DISPATCH_QUEUE_SERIAL);
    dispatch_semaphore_t done = dispatch_semaphore_create(0);
    __block int ran = 0;
    dispatch_suspend(q);
    dispatch_async(q, ^{ ran++; dispatch_semaphore_signal(done); });
    gcd_pause_ms(150);
    check("a suspended queue ran nothing", ran == 0, ran);
    dispatch_resume(q);
    check("resume lets the block run", gcd_wait(done) && ran == 1, ran);

    dispatch_suspend(q);
    dispatch_suspend(q);
    dispatch_async(q, ^{ ran++; dispatch_semaphore_signal(done); });
    dispatch_resume(q);
    gcd_pause_ms(150);
    check("two suspends need two resumes", ran == 1, ran);
    dispatch_resume(q);
    check("after the second resume the block runs", gcd_wait(done) && ran == 2, ran);

    /* A block in progress is not interrupted. */
    dispatch_semaphore_t in = dispatch_semaphore_create(0), go = dispatch_semaphore_create(0);
    __block int finished = 0;
    dispatch_async(q, ^{ dispatch_semaphore_signal(in); gcd_wait(go); finished = 1; dispatch_semaphore_signal(done); });
    gcd_wait(in);
    dispatch_suspend(q);
    dispatch_semaphore_signal(go);
    check("a running block finishes although the queue was suspended", gcd_wait(done) && finished == 1, finished);
    __block int later = 0;
    dispatch_async(q, ^{ later = 1; dispatch_semaphore_signal(done); });
    gcd_pause_ms(100);
    check("but the next one waits", later == 0, later);
    dispatch_resume(q);
    check("until resumed", gcd_wait(done) && later == 1, later);
    return finish();
}

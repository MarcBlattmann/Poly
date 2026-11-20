/* A concurrent queue starts blocks without waiting for earlier ones to
 * finish: two blocks that wait for each other both complete. */
/* poly-mac: libsystem */
#include "poly_mgcd.h"

int test_main(void) {
    dispatch_queue_t q = dispatch_queue_create("com.poly.conc", DISPATCH_QUEUE_CONCURRENT);
    dispatch_semaphore_t a_in = dispatch_semaphore_create(0), b_in = dispatch_semaphore_create(0);
    dispatch_semaphore_t done = dispatch_semaphore_create(0);
    __block int a_saw_b = 0, b_saw_a = 0;
    dispatch_async(q, ^{
        dispatch_semaphore_signal(a_in);
        a_saw_b = gcd_wait(b_in);
        dispatch_semaphore_signal(done);
    });
    dispatch_async(q, ^{
        dispatch_semaphore_signal(b_in);
        b_saw_a = gcd_wait(a_in);
        dispatch_semaphore_signal(done);
    });
    check("both blocks finished", gcd_wait(done) && gcd_wait(done), 0);
    check("each was running while the other waited for it (they ran in parallel)", a_saw_b && b_saw_a, 0);

    /* 16 blocks that all wait for each other to arrive. */
    dispatch_semaphore_t arrived = dispatch_semaphore_create(0), go = dispatch_semaphore_create(0), fin = dispatch_semaphore_create(0);
    for (int i = 0; i < 4; i++)
        dispatch_async(q, ^{
            dispatch_semaphore_signal(arrived);
            gcd_wait(go);
            dispatch_semaphore_signal(fin);
        });
    int all = 1;
    for (int i = 0; i < 4; i++) all &= gcd_wait(arrived);
    check("four blocks were running at the same time", all, 0);
    for (int i = 0; i < 4; i++) dispatch_semaphore_signal(go);
    int fins = 0;
    for (int i = 0; i < 4; i++) fins += gcd_wait(fin);
    check("and all finished", fins == 4, fins);
    return finish();
}

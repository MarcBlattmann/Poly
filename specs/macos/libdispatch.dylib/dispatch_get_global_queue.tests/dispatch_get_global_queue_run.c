/* Blocks on a global queue run on worker threads, never the main thread,
 * and a global queue runs blocks concurrently. */
/* poly-mac: libsystem */
#include "poly_mgcd.h"
#include <sys/qos.h>

int test_main(void) {
    dispatch_queue_t g = dispatch_get_global_queue(QOS_CLASS_DEFAULT, 0);
    dispatch_semaphore_t done = dispatch_semaphore_create(0);
    __block int on_main = -1;
    __block long long worker = 0;
    dispatch_async(g, ^{
        on_main = gcd_on_main_thread();
        worker = gcd_current_thread_id();
        dispatch_semaphore_signal(done);
    });
    check("the block ran", gcd_wait(done), 0);
    check("not on the main thread", on_main == 0, on_main);
    check("on a thread of its own, not the main thread's", worker != 0 && worker != gcd_current_thread_id(), 0);

    dispatch_semaphore_t in = dispatch_semaphore_create(0), release = dispatch_semaphore_create(0);
    for (int i = 0; i < 3; i++)
        dispatch_async(g, ^{
            dispatch_semaphore_signal(in);
            gcd_wait(release);
            dispatch_semaphore_signal(done);
        });
    int all = 1;
    for (int i = 0; i < 3; i++) all &= gcd_wait(in);
    check("three blocks of one global queue were running at once", all, 0);
    for (int i = 0; i < 3; i++) dispatch_semaphore_signal(release);
    int d = 0;
    for (int i = 0; i < 3; i++) d += gcd_wait(done);
    check("and all finished", d == 3, d);

    for (int i = 0; i < 4; i++) {
        dispatch_queue_t gq = dispatch_get_global_queue(i == 0 ? QOS_CLASS_BACKGROUND : i == 1 ? QOS_CLASS_UTILITY : i == 2 ? QOS_CLASS_USER_INITIATED : QOS_CLASS_USER_INTERACTIVE, 0);
        dispatch_async(gq, ^{ dispatch_semaphore_signal(done); });
    }
    int ran = 0;
    for (int i = 0; i < 4; i++) ran += gcd_wait(done);
    check("a block on each of the other four global queues ran", ran == 4, ran);
    return finish();
}

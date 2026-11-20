/* Blocks in flight keep their queue alive: the queue is released by the
 * creator while blocks are pending; the finalizer runs after they ran. */
/* poly-mac: libsystem */
#include "poly_mgcd.h"

static dispatch_semaphore_t finalized;
static int blocks_done_at_finalize = -1;
static int blocks_done;
static void finalizer(void *ctx) {
    (void)ctx;
    blocks_done_at_finalize = __atomic_load_n(&blocks_done, __ATOMIC_SEQ_CST);
    dispatch_semaphore_signal(finalized);
}

int test_main(void) {
    finalized = dispatch_semaphore_create(0);
    dispatch_queue_t q = dispatch_queue_create("com.poly.keepalive", DISPATCH_QUEUE_SERIAL);
    dispatch_set_finalizer_f(q, finalizer);
    dispatch_set_context(q, (void *)1);
    dispatch_semaphore_t gate = dispatch_semaphore_create(0);
    dispatch_async(q, ^{ gcd_wait(gate); __atomic_add_fetch(&blocks_done, 1, __ATOMIC_SEQ_CST); });
    for (int i = 0; i < 9; i++) dispatch_async(q, ^{ __atomic_add_fetch(&blocks_done, 1, __ATOMIC_SEQ_CST); });
    dispatch_release(q); /* the creator lets go with ten blocks pending */
    check("the finalizer has not run while blocks are pending", dispatch_semaphore_wait(finalized, dispatch_time(DISPATCH_TIME_NOW, 100 * NSEC_PER_MSEC)) != 0, 0);
    dispatch_semaphore_signal(gate);
    check("it runs after the pending blocks", gcd_wait(finalized), 0);
    check("all ten blocks had run first", blocks_done_at_finalize == 10, blocks_done_at_finalize);
    return finish();
}

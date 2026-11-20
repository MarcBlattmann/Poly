/* dispatch_sync from a worker thread into the main queue is run by the
 * main thread's run loop and returns after the block; the worker waits. */
/* poly-mac: libsystem */
/* poly-mac: corefoundation */
#include "poly_mgcd.h"
#include <CoreFoundation/CoreFoundation.h>

static int block_on_main = -1, worker_after = 0, seen_value = 0;

int test_main(void) {
    dispatch_async(dispatch_get_global_queue(QOS_CLASS_DEFAULT, 0), ^{
        dispatch_sync(dispatch_get_main_queue(), ^{
            block_on_main = gcd_on_main_thread();
            seen_value = 42;
        });
        worker_after = seen_value; /* only set after the sync block ran */
        dispatch_async(dispatch_get_main_queue(), ^{ CFRunLoopStop(CFRunLoopGetMain()); });
    });
    CFRunLoopRun();
    check("the block ran on the main thread", block_on_main == 1, block_on_main);
    check("the worker's dispatch_sync returned after the block (it saw 42)", worker_after == 42, worker_after);
    return finish();
}

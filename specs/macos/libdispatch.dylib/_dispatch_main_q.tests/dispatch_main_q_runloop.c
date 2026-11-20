/* The main queue (dispatch_get_main_queue, the data symbol _dispatch_main_q)
 * is drained by the main thread's run loop: blocks submitted from any
 * thread run in order on the main thread when CFRunLoopRun runs, never
 * before, and CFRunLoopStop ends the drain. */
/* poly-mac: libsystem */
/* poly-mac: corefoundation */
#include "poly_mgcd.h"
#include <CoreFoundation/CoreFoundation.h>

static int order[10], n;
static int on_main_all = 1;

static void note(int id) {
    if (!gcd_on_main_thread()) on_main_all = 0;
    order[n++] = id;
}

int test_main(void) {
    dispatch_queue_t m = dispatch_get_main_queue();
    check_str("the main queue's label", dispatch_queue_get_label(m), "com.apple.main-thread");
    check("dispatch_get_main_queue is the same on every call", m == dispatch_get_main_queue(), 0);
    check("it is not a global queue", m != dispatch_get_global_queue(QOS_CLASS_DEFAULT, 0), 0);

    dispatch_async(m, ^{ note(1); });
    dispatch_async(m, ^{ note(2); });
    gcd_pause_ms(100);
    check("blocks on the main queue wait for the run loop: none ran yet", n == 0, n);

    dispatch_async(dispatch_get_global_queue(QOS_CLASS_DEFAULT, 0), ^{
        gcd_pause_ms(50);
        dispatch_async(dispatch_get_main_queue(), ^{ note(3); });
        dispatch_async(dispatch_get_main_queue(), ^{
            note(4);
            CFRunLoopStop(CFRunLoopGetMain());
        });
    });
    CFRunLoopRun();
    check("the run loop drained the main queue: 4 blocks", n == 4, n);
    check("in submission order (blocks from two threads: 1, 2 from here, 3, 4 from the worker)", order[0] == 1 && order[1] == 2 && order[2] == 3 && order[3] == 4, n);
    check("all on the main thread", on_main_all, 0);
    return finish();
}

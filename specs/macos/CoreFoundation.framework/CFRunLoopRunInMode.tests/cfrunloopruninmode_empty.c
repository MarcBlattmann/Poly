/* poly-mac: corefoundation */
/* A mode with no sources, timers or observers returns kCFRunLoopRunFinished (1)
 * at once; the main thread's default mode is not empty and waits out the
 * timeout (kCFRunLoopRunTimedOut, 3); zero or negative timeouts return at once
 * with the same results. Constants: Finished 1, Stopped 2, TimedOut 3,
 * HandledSource 4. */
#include "poly_mcf.h"

int test_main(void) {
    CK(kCFRunLoopRunFinished == 1 && kCFRunLoopRunStopped == 2 && kCFRunLoopRunTimedOut == 3 &&
       kCFRunLoopRunHandledSource == 4);
    CFAbsoluteTime t0 = CFAbsoluteTimeGetCurrent();
    SInt32 r = CFRunLoopRunInMode(CFSTR("PolyEmptyMode"), 5.0, false);
    CKV(r == kCFRunLoopRunFinished, r);
    CK(CFAbsoluteTimeGetCurrent() - t0 < 1.0);
    r = CFRunLoopRunInMode(CFSTR("PolyEmptyMode"), 0.0, true);
    CKV(r == kCFRunLoopRunFinished, r);
    t0 = CFAbsoluteTimeGetCurrent();
    r = CFRunLoopRunInMode(kCFRunLoopDefaultMode, 0.3, false);
    CKV(r == kCFRunLoopRunTimedOut, r);
    CK(CFAbsoluteTimeGetCurrent() - t0 >= 0.25);
    CK(CFAbsoluteTimeGetCurrent() - t0 < 2.0);
    r = CFRunLoopRunInMode(kCFRunLoopDefaultMode, 0.0, false);
    CKV(r == kCFRunLoopRunTimedOut, r);
    r = CFRunLoopRunInMode(kCFRunLoopDefaultMode, -1.0, false);
    CKV(r == kCFRunLoopRunTimedOut, r);
    r = CFRunLoopRunInMode(kCFRunLoopDefaultMode, 0.05, true);
    CKV(r == kCFRunLoopRunTimedOut, r);
    return finish();
}

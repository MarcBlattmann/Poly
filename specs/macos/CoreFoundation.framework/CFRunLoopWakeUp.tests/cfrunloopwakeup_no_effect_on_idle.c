/* poly-mac: corefoundation */
/* Waking a run loop that is not waiting does nothing visible: the next run is
 * as it would have been, a wake-up does not end a run, run a source or fire
 * a timer, and it can be repeated; it works for the main run loop and
 * leaves the loop usable. */
#include "poly_mcf.h"

static int perf, fired;
static void p(void *i) { (void)i; perf++; }
static void tick(CFRunLoopTimerRef t, void *i) { (void)t; (void)i; fired++; }

int test_main(void) {
    CFRunLoopRef cur = CFRunLoopGetCurrent();
    for (int i = 0; i < 5; i++) CFRunLoopWakeUp(cur);
    CKV(CFRunLoopRunInMode(CFSTR("PolyWake"), 0.1, false) == kCFRunLoopRunFinished, 0);
    CKV(CFRunLoopRunInMode(kCFRunLoopDefaultMode, 0.1, false) == kCFRunLoopRunTimedOut, 0);
    CFRunLoopSourceContext c;
    memset(&c, 0, sizeof c);
    c.perform = p;
    CFRunLoopSourceRef s = CFRunLoopSourceCreate(kCFAllocatorDefault, 0, &c);
    CFRunLoopAddSource(cur, s, CFSTR("PolyWake"));
    CFRunLoopWakeUp(cur);
    CFRunLoopRunInMode(CFSTR("PolyWake"), 0.1, false);
    CKV(perf == 0, perf);
    CFRunLoopTimerRef t = CFRunLoopTimerCreate(kCFAllocatorDefault, CFAbsoluteTimeGetCurrent() + 100, 0, 0, 0, tick, NULL);
    CFRunLoopAddTimer(cur, t, CFSTR("PolyWake"));
    CFRunLoopWakeUp(cur);
    CFAbsoluteTime t0 = CFAbsoluteTimeGetCurrent();
    CKV(CFRunLoopRunInMode(CFSTR("PolyWake"), 0.2, false) == kCFRunLoopRunTimedOut, 0);
    CK(CFAbsoluteTimeGetCurrent() - t0 >= 0.15);
    CKV(fired == 0, fired);
    CFRunLoopSourceSignal(s);
    CFRunLoopWakeUp(cur);
    CKV(CFRunLoopRunInMode(CFSTR("PolyWake"), 0.1, true) == kCFRunLoopRunHandledSource, 0);
    CKV(perf == 1, perf);
    return finish();
}

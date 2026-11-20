/* poly-mac: corefoundation */
/* A version 0 source: schedule is called when the source is added to a run
 * loop and mode, cancel when it is removed from it or invalidated, perform per
 * signal-and-run; the order and validity are what were set; the context's
 * info reaches the callbacks; a source without callbacks can be signalled. */
#include "poly_mcf.h"

static int sched, canc, perf;
static int token = 1;
static void *seen;
static void s_sched(void *info, CFRunLoopRef rl, CFRunLoopMode m) { (void)rl; (void)m; seen = info; sched++; }
static void s_cancel(void *info, CFRunLoopRef rl, CFRunLoopMode m) { (void)info; (void)rl; (void)m; canc++; }
static void s_perform(void *info) { (void)info; perf++; }

int test_main(void) {
    CFRunLoopRef cur = CFRunLoopGetCurrent();
    CFRunLoopSourceContext c;
    memset(&c, 0, sizeof c);
    c.info = &token;
    c.schedule = s_sched;
    c.cancel = s_cancel;
    c.perform = s_perform;
    CFRunLoopSourceRef s = CFRunLoopSourceCreate(kCFAllocatorDefault, 3, &c);
    CK(s != NULL && CFGetTypeID(s) == CFRunLoopSourceGetTypeID());
    CK(CFRunLoopSourceIsValid(s) && CFRunLoopSourceGetOrder(s) == 3);
    CKV(sched == 0, sched);
    CK(!CFRunLoopContainsSource(cur, s, CFSTR("PolyS")));
    CFRunLoopAddSource(cur, s, CFSTR("PolyS"));
    CKV(sched == 1, sched);
    CK(seen == &token);
    CK(CFRunLoopContainsSource(cur, s, CFSTR("PolyS")));
    CFRunLoopSourceSignal(s);
    CFRunLoopRunInMode(CFSTR("PolyS"), 0.05, true);
    CKV(perf == 1, perf);
    CFRunLoopRemoveSource(cur, s, CFSTR("PolyS"));
    CKV(canc == 1, canc);
    CK(!CFRunLoopContainsSource(cur, s, CFSTR("PolyS")) && CFRunLoopSourceIsValid(s));
    CFRunLoopAddSource(cur, s, CFSTR("PolyS"));
    CFRunLoopSourceInvalidate(s);
    CK(!CFRunLoopSourceIsValid(s));
    CKV(canc == 2, canc);
    CFRunLoopSourceSignal(s);
    CFRunLoopRunInMode(CFSTR("PolyS"), 0.0, false);
    CKV(perf == 1, perf);
    CFRunLoopSourceContext z;
    memset(&z, 0, sizeof z);
    CFRunLoopSourceRef bare = CFRunLoopSourceCreate(kCFAllocatorDefault, 0, &z);
    CFRunLoopAddSource(cur, bare, CFSTR("PolyBare"));
    CFRunLoopSourceSignal(bare);
    CKV(CFRunLoopRunInMode(CFSTR("PolyBare"), 0.05, true) == kCFRunLoopRunHandledSource, 0);
    return finish();
}

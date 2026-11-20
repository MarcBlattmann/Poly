/* poly-mac: corefoundation */
/* A signalled source is performed during the next run, once however often it
 * was signalled; with returnAfterSourceHandled the run returns HandledSource (4)
 * after the first source handled; sources go in the order of their order
 * value; without that flag the run goes on to its timeout. */
#include "poly_mcf.h"

static char seq[16];
static int n;
static void pa(void *i) { (void)i; seq[n++] = 'a'; }
static void pb(void *i) { (void)i; seq[n++] = 'b'; }
static void pc(void *i) { (void)i; seq[n++] = 'c'; }

static CFRunLoopSourceRef mk(void (*f)(void *), CFIndex order) {
    CFRunLoopSourceContext c;
    memset(&c, 0, sizeof c);
    c.perform = f;
    return CFRunLoopSourceCreate(kCFAllocatorDefault, order, &c);
}

int test_main(void) {
    CFRunLoopRef cur = CFRunLoopGetCurrent();
    CFStringRef mode = CFSTR("PolySources");
    CFRunLoopSourceRef a = mk(pa, 5), b = mk(pb, 1), c = mk(pc, 3);
    CFRunLoopAddSource(cur, a, mode);
    CFRunLoopAddSource(cur, b, mode);
    CFRunLoopAddSource(cur, c, mode);
    CKV(CFRunLoopRunInMode(mode, 0.05, false) == kCFRunLoopRunTimedOut, 0);
    CKV(n == 0, n);
    CFRunLoopSourceSignal(a);
    CK(n == 0);
    CFRunLoopSourceSignal(a);
    CFRunLoopSourceSignal(b);
    CFRunLoopSourceSignal(c);
    SInt32 r = CFRunLoopRunInMode(mode, 1.0, true);
    CKV(r == kCFRunLoopRunHandledSource, r);
    seq[n] = 0;
    CK(seq[0] == 'b');
    r = CFRunLoopRunInMode(mode, 0.0, false);
    seq[n] = 0;
    CKV(n == 3, n);
    CK(str_eq(seq, "bca"));
    CFRunLoopSourceSignal(b);
    CFAbsoluteTime t0 = CFAbsoluteTimeGetCurrent();
    r = CFRunLoopRunInMode(mode, 0.2, false);
    CKV(r == kCFRunLoopRunTimedOut, r);
    CK(CFAbsoluteTimeGetCurrent() - t0 >= 0.15);
    CKV(n == 4, n);
    CFRunLoopSourceSignal(c);
    CFRunLoopRemoveSource(cur, c, mode);
    CFRunLoopRunInMode(mode, 0.0, false);
    CKV(n == 4, n);
    return finish();
}

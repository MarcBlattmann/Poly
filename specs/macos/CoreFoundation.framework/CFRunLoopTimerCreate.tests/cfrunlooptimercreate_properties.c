/* poly-mac: corefoundation */
/* A timer is a CFRunLoopTimer object, valid until it fires (one-shot) or is
 * invalidated, in no mode until added; its interval is what was asked, a
 * one-shot has interval 0; it can be added to several modes; a due date in
 * the past fires at once; its context info reaches the callback. */
#include "poly_mcf.h"

static int fired;
static void *seen_info;
static void cb(CFRunLoopTimerRef t, void *info) {
    (void)t;
    seen_info = info;
    fired++;
}

int test_main(void) {
    CFRunLoopRef cur = CFRunLoopGetCurrent();
    int token = 7;
    CFRunLoopTimerContext ctx = {0, &token, NULL, NULL, NULL};
    CFRunLoopTimerRef t = CFRunLoopTimerCreate(kCFAllocatorDefault, CFAbsoluteTimeGetCurrent() + 100, 0, 0, 0, cb, &ctx);
    CK(t != NULL && CFGetTypeID(t) == CFRunLoopTimerGetTypeID());
    CK(CFRunLoopTimerIsValid(t));
    CK(CFRunLoopTimerGetInterval(t) == 0);
    CK(!CFRunLoopContainsTimer(cur, t, kCFRunLoopDefaultMode));
    CFRunLoopAddTimer(cur, t, kCFRunLoopDefaultMode);
    CFRunLoopAddTimer(cur, t, CFSTR("PolyOther"));
    CK(CFRunLoopContainsTimer(cur, t, kCFRunLoopDefaultMode) && CFRunLoopContainsTimer(cur, t, CFSTR("PolyOther")));
    CK(!CFRunLoopContainsTimer(cur, t, CFSTR("PolyThird")));
    CFRunLoopRemoveTimer(cur, t, CFSTR("PolyOther"));
    CK(!CFRunLoopContainsTimer(cur, t, CFSTR("PolyOther")) && CFRunLoopTimerIsValid(t));
    CFRunLoopTimerInvalidate(t);
    CK(!CFRunLoopTimerIsValid(t));
    CFRunLoopTimerRef rep = CFRunLoopTimerCreate(kCFAllocatorDefault, CFAbsoluteTimeGetCurrent() + 100, 0.25, 0, 0, cb, &ctx);
    CK(CFRunLoopTimerGetInterval(rep) == 0.25);
    CFRunLoopTimerRef past = CFRunLoopTimerCreate(kCFAllocatorDefault, CFAbsoluteTimeGetCurrent() - 5, 0, 0, 0, cb, &ctx);
    CFRunLoopAddTimer(cur, past, CFSTR("PolyPast"));
    SInt32 r = CFRunLoopRunInMode(CFSTR("PolyPast"), 0.3, false);
    CKV(r == kCFRunLoopRunTimedOut || r == kCFRunLoopRunFinished, r);
    CKV(fired == 1, fired);
    CK(seen_info == &token);
    CFRunLoopTimerRef late = CFRunLoopTimerCreate(kCFAllocatorDefault, CFAbsoluteTimeGetCurrent() + 100, 0, 0, 0, cb, &ctx);
    CFRunLoopAddTimer(cur, late, CFSTR("PolyLate"));
    CFRunLoopTimerSetNextFireDate(late, CFAbsoluteTimeGetCurrent() + 0.02);
    CFRunLoopRunInMode(CFSTR("PolyLate"), 0.3, false);
    CKV(fired == 2, fired);
    return finish();
}

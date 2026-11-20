/* poly-mac: appkit */
/* A posted application-defined event is returned with its type (15), subtype and data; it is a
 * copy, not the object posted; events come in the order posted (first in, first out), an event
 * posted with atStart YES comes first; with dequeue NO the event is only looked at (the
 * same one is returned again), with YES it is removed; a call with a timeout returns as soon as
 * the event is there (posted from a timer at 0.1 s: returned after about 0.1 s). */
#include "poly_mak.h"

int test_main(void) {
    NSApplication *a = ak_init();
    NSEvent *ev = ak_event(7, 11);
    CK(ev.type == NSEventTypeApplicationDefined && NSEventTypeApplicationDefined == 15);
    [a postEvent:ev atStart:NO];
    NSEvent *got = [a nextEventMatchingMask:NSEventMaskAny untilDate:[NSDate dateWithTimeIntervalSinceNow:1] inMode:NSDefaultRunLoopMode dequeue:YES];
    CK(got != nil && got.type == NSEventTypeApplicationDefined && got.subtype == 7 && got.data1 == 11 && got.data2 == 0);
    CK(got != ev);
    [a postEvent:ak_event(1, 1) atStart:NO];
    [a postEvent:ak_event(2, 2) atStart:NO];
    [a postEvent:ak_event(3, 3) atStart:YES];
    NSEvent *e1 = [a nextEventMatchingMask:NSEventMaskAny untilDate:[NSDate distantPast] inMode:NSDefaultRunLoopMode dequeue:YES];
    NSEvent *e2 = [a nextEventMatchingMask:NSEventMaskAny untilDate:[NSDate distantPast] inMode:NSDefaultRunLoopMode dequeue:YES];
    NSEvent *e3 = [a nextEventMatchingMask:NSEventMaskAny untilDate:[NSDate distantPast] inMode:NSDefaultRunLoopMode dequeue:YES];
    CK(e1.subtype == 3 && e2.subtype == 1 && e3.subtype == 2);
    CK([a nextEventMatchingMask:NSEventMaskAny untilDate:[NSDate distantPast] inMode:NSDefaultRunLoopMode dequeue:YES] == nil);
    [a postEvent:ak_event(9, 9) atStart:NO];
    NSEvent *peek1 = [a nextEventMatchingMask:NSEventMaskAny untilDate:[NSDate distantPast] inMode:NSDefaultRunLoopMode dequeue:NO];
    NSEvent *peek2 = [a nextEventMatchingMask:NSEventMaskAny untilDate:[NSDate distantPast] inMode:NSDefaultRunLoopMode dequeue:NO];
    CK(peek1 != nil && peek2 != nil && peek1.subtype == 9 && peek2.subtype == 9);
    NSEvent *taken = [a nextEventMatchingMask:NSEventMaskAny untilDate:[NSDate distantPast] inMode:NSDefaultRunLoopMode dequeue:YES];
    CK(taken.subtype == 9);
    CK([a nextEventMatchingMask:NSEventMaskAny untilDate:[NSDate distantPast] inMode:NSDefaultRunLoopMode dequeue:YES] == nil);
    NSTimer *t = [NSTimer timerWithTimeInterval:0.1 repeats:NO block:^(NSTimer *x) { (void)x; [NSApp postEvent:ak_event(4, 4) atStart:NO]; }];
    [[NSRunLoop currentRunLoop] addTimer:t forMode:NSDefaultRunLoopMode];
    NSDate *t0 = [NSDate date];
    NSEvent *late = [a nextEventMatchingMask:NSEventMaskAny untilDate:[NSDate dateWithTimeIntervalSinceNow:3] inMode:NSDefaultRunLoopMode dequeue:YES];
    NSTimeInterval d = [[NSDate date] timeIntervalSinceDate:t0];
    CK(late != nil && late.subtype == 4 && d >= 0.08 && d < 1.5);
    return finish();
}

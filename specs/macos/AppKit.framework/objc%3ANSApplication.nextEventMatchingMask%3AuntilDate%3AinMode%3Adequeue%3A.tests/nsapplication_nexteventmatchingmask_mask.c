/* poly-mac: appkit */
/* The mask selects: a mask of key-down events does not return an application-defined event and
 * leaves it queued (a later call with the any-mask returns it); the application-defined mask
 * (1 << 15) returns it; a mask of zero returns nothing; the event mask constants are 1 << type
 * (NSEventMaskAny is all ones). */
#include "poly_mak.h"

int test_main(void) {
    NSApplication *a = ak_init();
    CK(NSEventMaskApplicationDefined == (1ULL << NSEventTypeApplicationDefined));
    CK(NSEventMaskKeyDown == (1ULL << NSEventTypeKeyDown));
    CK(NSEventMaskAny == ~0ULL);
    [a postEvent:ak_event(1, 1) atStart:NO];
    NSEvent *e = [a nextEventMatchingMask:NSEventMaskKeyDown untilDate:[NSDate dateWithTimeIntervalSinceNow:0.2] inMode:NSDefaultRunLoopMode dequeue:YES];
    CK(e == nil);
    e = [a nextEventMatchingMask:0 untilDate:[NSDate distantPast] inMode:NSDefaultRunLoopMode dequeue:YES];
    CK(e == nil);
    e = [a nextEventMatchingMask:NSEventMaskApplicationDefined untilDate:[NSDate distantPast] inMode:NSDefaultRunLoopMode dequeue:YES];
    CK(e != nil && e.subtype == 1);
    [a postEvent:ak_event(2, 2) atStart:NO];
    e = [a nextEventMatchingMask:NSEventMaskAny untilDate:[NSDate distantPast] inMode:NSDefaultRunLoopMode dequeue:YES];
    CK(e != nil && e.subtype == 2);
    return finish();
}

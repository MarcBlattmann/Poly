/* poly-mac: appkit */
/* stop: when the application is not running does nothing visible: no exception, isRunning stays
 * NO, and event handling still works after it (a posted event is delivered; a timeout
 * gives nil); calling it several times is the same. */
#include "poly_mak.h"

int test_main(void) {
    NSApplication *a = ak_init();
    [a stop:nil];
    [a stop:nil];
    CK(!a.isRunning);
    [a postEvent:ak_event(5, 55) atStart:NO];
    NSEvent *e = [a nextEventMatchingMask:NSEventMaskAny untilDate:[NSDate dateWithTimeIntervalSinceNow:0.5] inMode:NSDefaultRunLoopMode dequeue:YES];
    CK(e != nil && e.type == NSEventTypeApplicationDefined && e.subtype == 5 && e.data1 == 55);
    CK([a nextEventMatchingMask:NSEventMaskAny untilDate:[NSDate dateWithTimeIntervalSinceNow:0.1] inMode:NSDefaultRunLoopMode dequeue:YES] == nil);
    CK(!a.isRunning);
    return finish();
}

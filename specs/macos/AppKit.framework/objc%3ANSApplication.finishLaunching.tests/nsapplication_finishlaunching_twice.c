/* poly-mac: appkit */
/* Calling finishLaunching a second time sends applicationWillFinishLaunching: again (twice
 * in total), while applicationDidFinishLaunching: arrives once from the event loop; the
 * application is still not running. */
#include "poly_mak.h"

static int will, did;

@interface Del : NSObject <NSApplicationDelegate>
@end
@implementation Del
- (void)applicationWillFinishLaunching:(NSNotification *)x { (void)x; will++; }
- (void)applicationDidFinishLaunching:(NSNotification *)x { (void)x; did++; }
@end

int test_main(void) {
    NSApplication *a = ak_init();
    Del *d = [[Del alloc] init];
    a.delegate = d;
    [a finishLaunching];
    CK(will == 1);
    [a finishLaunching];
    CK(will == 2);
    [a nextEventMatchingMask:NSEventMaskAny untilDate:[NSDate dateWithTimeIntervalSinceNow:0.3] inMode:NSDefaultRunLoopMode dequeue:YES];
    CK(did == 1);
    CK(will == 2);
    CK(!a.isRunning);
    return finish();
}

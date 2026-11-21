/* poly-mac: appkit */
/* run can be called again after it returned: the launch callbacks are not repeated (the delegate got
 * will and did once), isRunning is YES inside the second run and NO after it; the second run
 * returns after its own stop. */
#include "poly_mak.h"

static int will, did, in_second = -1;

@interface Del : NSObject <NSApplicationDelegate>
@end
@implementation Del
- (void)applicationWillFinishLaunching:(NSNotification *)x { (void)x; will++; }
- (void)applicationDidFinishLaunching:(NSNotification *)x { (void)x; did++; [NSApp stop:nil]; [NSApp postEvent:ak_event(0, 0) atStart:YES]; }
@end

int test_main(void) {
    NSApplication *a = ak_init();
    Del *d = [[Del alloc] init];
    a.delegate = d;
    [a run];
    CK(will == 1 && did == 1 && !a.isRunning);
    NSTimer *t = [NSTimer timerWithTimeInterval:0.1 repeats:NO block:^(NSTimer *x) {
        (void)x;
        in_second = NSApp.isRunning;
        [NSApp stop:nil];
        [NSApp postEvent:ak_event(0, 0) atStart:YES];
    }];
    [[NSRunLoop currentRunLoop] addTimer:t forMode:NSRunLoopCommonModes];
    [a run];
    CK(in_second == 1);
    CK(!a.isRunning);
    CK(will == 1 && did == 1);
    return finish();
}

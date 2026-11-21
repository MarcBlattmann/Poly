/* poly-mac: appkit */
/* finishLaunching without a delegate still posts the two notifications, will before did, and
 * posts nothing more when it is called a second time while the will-launch delegate method is sent
 * again; run after finishLaunching does not repeat the did-launch part: the delegate sees
 * didFinishLaunching once in total. */
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
    __block int nw = 0, nd = 0;
    [[NSNotificationCenter defaultCenter] addObserverForName:NSApplicationWillFinishLaunchingNotification object:nil queue:nil usingBlock:^(NSNotification *x) { (void)x; nw++; }];
    [[NSNotificationCenter defaultCenter] addObserverForName:NSApplicationDidFinishLaunchingNotification object:nil queue:nil usingBlock:^(NSNotification *x) { (void)x; nd++; }];
    [a finishLaunching];
    CK(nw == 1 && nd == 0);
    [a nextEventMatchingMask:NSEventMaskAny untilDate:[NSDate dateWithTimeIntervalSinceNow:0.2] inMode:NSDefaultRunLoopMode dequeue:YES];
    CK(nw == 1 && nd == 1);
    Del *d = [[Del alloc] init];
    a.delegate = d;
    ak_stop_run_after(0.3);
    [a run];
    CK(!a.isRunning);
    CK(did == 0);
    CK(nd == 1);
    return finish();
}
